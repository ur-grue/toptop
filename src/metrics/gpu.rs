//! Optional NVIDIA GPU monitoring via `nvidia-smi`.
//!
//! `nvidia-smi` can take tens to hundreds of milliseconds to respond, so it is
//! polled on a dedicated background thread and the latest reading is published
//! through a shared slot. The UI thread never blocks on it, and machines
//! without an NVIDIA GPU simply see no GPU panel.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A single GPU's live state.
#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub util_pct: f32,
    /// Whether `util_pct` is a real reading (some drivers don't expose it).
    pub has_util: bool,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp: f32,
}

impl Gpu {
    pub fn mem_pct(&self) -> f32 {
        if self.mem_total == 0 {
            0.0
        } else {
            (self.mem_used as f64 / self.mem_total as f64 * 100.0) as f32
        }
    }
}

/// Parse the CSV emitted by:
/// `nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu
///            --format=csv,noheader,nounits`
///
/// Memory values are reported in MiB. Malformed lines are skipped.
pub fn parse_nvidia_smi(output: &str) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(|f| f.trim()).collect();
        if fields.len() < 5 {
            continue;
        }
        let name = fields[0].to_string();
        let (Ok(util), Ok(mem_used_mib), Ok(mem_total_mib), Ok(temp)) = (
            fields[1].parse::<f32>(),
            fields[2].parse::<u64>(),
            fields[3].parse::<u64>(),
            fields[4].parse::<f32>(),
        ) else {
            continue;
        };
        gpus.push(Gpu {
            name,
            util_pct: util,
            has_util: true,
            mem_used: mem_used_mib * 1024 * 1024,
            mem_total: mem_total_mib * 1024 * 1024,
            temp,
        });
    }
    gpus
}

/// Translate a kernel DRM driver name into a friendly vendor label.
fn driver_label(driver: Option<&str>) -> String {
    match driver.unwrap_or("") {
        "amdgpu" | "radeon" => "AMD GPU".to_string(),
        "i915" | "xe" => "Intel GPU".to_string(),
        "nouveau" | "nvidia" => "NVIDIA GPU".to_string(),
        "" => "GPU".to_string(),
        other => format!("GPU ({other})"),
    }
}

/// Assemble a [`Gpu`] from raw sysfs values for one DRM card. Returns `None`
/// when the card exposes neither utilization nor VRAM info (i.e. not a GPU we
/// can meaningfully report on). `temp_milli` is in millidegrees Celsius.
pub fn build_sysfs_gpu(
    driver: Option<&str>,
    busy: Option<u64>,
    vram_used: Option<u64>,
    vram_total: Option<u64>,
    temp_milli: Option<i64>,
) -> Option<Gpu> {
    if busy.is_none() && vram_total.is_none() {
        return None;
    }
    Some(Gpu {
        name: driver_label(driver),
        util_pct: busy.unwrap_or(0) as f32,
        has_util: busy.is_some(),
        mem_used: vram_used.unwrap_or(0),
        mem_total: vram_total.unwrap_or(0),
        temp: temp_milli.map(|t| t as f32 / 1000.0).unwrap_or(0.0),
    })
}

fn read_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Read the hottest `temp*_input` from a card's first hwmon directory (millideg).
fn read_card_temp(device: &std::path::Path) -> Option<i64> {
    let hwmon_root = device.join("hwmon");
    let mut best: Option<i64> = None;
    for hw in std::fs::read_dir(hwmon_root).ok()?.flatten() {
        let dir = std::fs::read_dir(hw.path()).ok();
        let Some(dir) = dir else { continue };
        for f in dir.flatten() {
            let name = f.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("temp") && name.ends_with("_input") {
                let v = std::fs::read_to_string(f.path())
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok());
                if let Some(v) = v {
                    best = Some(best.map_or(v, |b| b.max(v)));
                }
            }
        }
    }
    best
}

/// Discover GPUs through `/sys/class/drm` (AMD `amdgpu`, Intel `i915`/`xe`).
fn read_sysfs_gpus() -> Vec<Gpu> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut cards: Vec<std::path::PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    // Real cards are "card0"; connectors are "card0-DP-1".
                    n.starts_with("card") && !n.contains('-')
                })
                .unwrap_or(false)
        })
        .collect();
    cards.sort();
    for card in cards {
        let device = card.join("device");
        let driver = std::fs::read_to_string(device.join("uevent"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("DRIVER=").map(|d| d.to_string()))
            });
        let busy = read_u64(&device.join("gpu_busy_percent"));
        let vram_used = read_u64(&device.join("mem_info_vram_used"));
        let vram_total = read_u64(&device.join("mem_info_vram_total"));
        let temp = read_card_temp(&device);
        if let Some(gpu) = build_sysfs_gpu(driver.as_deref(), busy, vram_used, vram_total, temp) {
            out.push(gpu);
        }
    }
    out
}

fn query_nvidia() -> Option<Vec<Gpu>> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout)))
}

/// Combine all GPU sources: NVIDIA via `nvidia-smi`, AMD/Intel via sysfs.
fn query_all() -> Vec<Gpu> {
    let mut gpus = query_nvidia().unwrap_or_default();
    gpus.extend(read_sysfs_gpus());
    gpus
}

/// Background poller that keeps the latest GPU snapshot in a shared slot.
pub struct GpuMonitor {
    latest: Arc<Mutex<Vec<Gpu>>>,
}

impl GpuMonitor {
    /// Probe for `nvidia-smi`; if present, spawn a polling thread. Otherwise the
    /// monitor exists but always reports an empty list.
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        let initial = query_all();
        // Only spawn the poller if at least one GPU source is actually present.
        if !initial.is_empty() {
            *latest.lock().unwrap() = initial;
            let shared = Arc::clone(&latest);
            std::thread::Builder::new()
                .name("toptop-gpu".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(2000));
                    let gpus = query_all();
                    if let Ok(mut slot) = shared.lock() {
                        *slot = gpus;
                    }
                })
                .ok();
        }
        Self { latest }
    }

    /// The most recent GPU snapshot (clone of the shared slot).
    pub fn snapshot(&self) -> Vec<Gpu> {
        self.latest.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed() {
        let out = "NVIDIA GeForce RTX 4090, 42, 2048, 24564, 56\n\
                   NVIDIA A100, 100, 40000, 40960, 71\n";
        let gpus = parse_nvidia_smi(out);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(gpus[0].util_pct, 42.0);
        assert_eq!(gpus[0].mem_used, 2048 * 1024 * 1024);
        assert_eq!(gpus[0].temp, 56.0);
        assert!((gpus[1].mem_pct() - 97.65625).abs() < 0.01);
    }

    #[test]
    fn skips_malformed() {
        let out = "broken line\n\
                   , , , ,\n\
                   NVIDIA T4, 10, 100, 1000, 40\n\
                   \n";
        let gpus = parse_nvidia_smi(out);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "NVIDIA T4");
    }

    #[test]
    fn empty_input() {
        assert!(parse_nvidia_smi("").is_empty());
    }

    #[test]
    fn nvidia_has_util_flag() {
        let g = &parse_nvidia_smi("NVIDIA T4, 10, 100, 1000, 40")[0];
        assert!(g.has_util);
    }

    #[test]
    fn sysfs_amd_full() {
        let g = build_sysfs_gpu(
            Some("amdgpu"),
            Some(73),
            Some(2 * 1024 * 1024 * 1024),
            Some(8 * 1024 * 1024 * 1024),
            Some(61000),
        )
        .expect("amd gpu");
        assert_eq!(g.name, "AMD GPU");
        assert!(g.has_util);
        assert_eq!(g.util_pct, 73.0);
        assert_eq!(g.temp, 61.0);
        assert_eq!(g.mem_total, 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn sysfs_intel_no_util() {
        // Intel i915 exposes VRAM/temp but not gpu_busy_percent.
        let g =
            build_sysfs_gpu(Some("i915"), None, None, Some(1024), Some(45000)).expect("intel gpu");
        assert_eq!(g.name, "Intel GPU");
        assert!(!g.has_util);
        assert_eq!(g.temp, 45.0);
    }

    #[test]
    fn sysfs_rejects_non_gpu() {
        assert!(build_sysfs_gpu(Some("simpledrm"), None, None, None, Some(0)).is_none());
    }
}
