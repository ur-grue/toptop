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
            mem_used: mem_used_mib * 1024 * 1024,
            mem_total: mem_total_mib * 1024 * 1024,
            temp,
        });
    }
    gpus
}

fn query_once() -> Option<Vec<Gpu>> {
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

/// Background poller that keeps the latest GPU snapshot in a shared slot.
pub struct GpuMonitor {
    latest: Arc<Mutex<Vec<Gpu>>>,
}

impl GpuMonitor {
    /// Probe for `nvidia-smi`; if present, spawn a polling thread. Otherwise the
    /// monitor exists but always reports an empty list.
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        if let Some(initial) = query_once() {
            *latest.lock().unwrap() = initial;
            let shared = Arc::clone(&latest);
            std::thread::Builder::new()
                .name("toptop-gpu".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(2000));
                    if let Some(gpus) = query_once() {
                        if let Ok(mut slot) = shared.lock() {
                            *slot = gpus;
                        }
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
}
