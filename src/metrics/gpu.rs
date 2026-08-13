//! GPU monitoring tuned for local-AI / LLM work.
//!
//! NVIDIA is queried via `nvidia-smi` and AMD/Intel via `sysfs`, both on a
//! dedicated background thread (so the variable-latency `nvidia-smi` never
//! blocks the UI). Beyond plain "GPU %", we surface the metrics that actually
//! predict local-inference performance:
//!
//! * **memory-bandwidth utilization** (`utilization.memory`) — usually the real
//!   bottleneck once a model fits in VRAM, and invisible in a bare "GPU %";
//! * **VRAM pressure** — how close you are to the hard wall before a model
//!   spills layers into system RAM (a 5–20× slowdown);
//! * **per-process VRAM** — which process is holding GPU memory (catches models
//!   "squatting" on VRAM and identifies your inference server);
//! * **power vs. limit and a throttle flag** — thermal/power throttling quietly
//!   drops tokens/sec.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A single GPU's live state.
#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// Core (SM) utilization %.
    pub util_pct: f32,
    pub has_util: bool,
    /// Memory-bandwidth utilization % (often the real LLM bottleneck).
    pub mem_util: f32,
    pub has_mem_util: bool,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp: f32,
    /// Current power draw / enforced limit, in watts (0 when unknown).
    pub power: f32,
    pub power_limit: f32,
    /// Whether the driver reports an active power/thermal throttle.
    pub throttled: bool,
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

/// A process holding GPU memory, as reported by `nvidia-smi` compute-apps.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuProc {
    pub pid: u32,
    pub used_mem: u64,
}

/// Everything the background poller publishes each cycle.
#[derive(Clone, Debug, Default)]
pub struct GpuSnapshot {
    pub gpus: Vec<Gpu>,
    pub procs: Vec<GpuProc>,
}

/// Parse one optional, possibly-`[N/A]` numeric field.
fn fopt(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('[') {
        return None;
    }
    s.parse::<f32>().ok()
}

// NVML throttle reasons we treat as a real (performance-limiting) throttle:
// SW power cap | HW slowdown | SW thermal | HW thermal | HW power brake.
const THROTTLE_MASK: u64 = 0x4 | 0x8 | 0x20 | 0x40 | 0x80;

/// Parse the legacy 5-field CSV (name, util, mem.used, mem.total, temp).
pub fn parse_nvidia_smi(output: &str) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    for line in output.lines() {
        let f: Vec<&str> = line.split(',').map(|x| x.trim()).collect();
        if f.len() < 5 || f[0].is_empty() {
            continue;
        }
        let (Some(util), Some(mu), Some(mt), Some(temp)) =
            (fopt(f[1]), fopt(f[2]), fopt(f[3]), fopt(f[4]))
        else {
            continue;
        };
        gpus.push(Gpu {
            name: f[0].to_string(),
            util_pct: util,
            has_util: true,
            mem_util: 0.0,
            has_mem_util: false,
            mem_used: mu as u64 * 1024 * 1024,
            mem_total: mt as u64 * 1024 * 1024,
            temp,
            power: 0.0,
            power_limit: 0.0,
            throttled: false,
        });
    }
    gpus
}

/// Parse the extended CSV with bandwidth, power and throttle fields:
/// name, utilization.gpu, utilization.memory, memory.used, memory.total,
/// temperature.gpu, power.draw, power.limit, clocks_throttle_reasons.active
pub fn parse_nvidia_smi_ext(output: &str) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    for line in output.lines() {
        let f: Vec<&str> = line.split(',').map(|x| x.trim()).collect();
        if f.len() < 9 || f[0].is_empty() {
            continue;
        }
        let (Some(mu), Some(mt)) = (fopt(f[3]), fopt(f[4])) else {
            continue;
        };
        let throttle = f[8]
            .trim()
            .strip_prefix("0x")
            .and_then(|h| u64::from_str_radix(h, 16).ok())
            .map(|bits| bits & THROTTLE_MASK != 0)
            .unwrap_or(false);
        gpus.push(Gpu {
            name: f[0].to_string(),
            util_pct: fopt(f[1]).unwrap_or(0.0),
            has_util: fopt(f[1]).is_some(),
            mem_util: fopt(f[2]).unwrap_or(0.0),
            has_mem_util: fopt(f[2]).is_some(),
            mem_used: mu as u64 * 1024 * 1024,
            mem_total: mt as u64 * 1024 * 1024,
            temp: fopt(f[5]).unwrap_or(0.0),
            power: fopt(f[6]).unwrap_or(0.0),
            power_limit: fopt(f[7]).unwrap_or(0.0),
            throttled: throttle,
        });
    }
    gpus
}

/// Parse `nvidia-smi --query-compute-apps=pid,used_memory` CSV (MiB).
pub fn parse_compute_apps(output: &str) -> Vec<GpuProc> {
    let mut procs = Vec::new();
    for line in output.lines() {
        let f: Vec<&str> = line.split(',').map(|x| x.trim()).collect();
        if f.len() < 2 {
            continue;
        }
        let Ok(pid) = f[0].parse::<u32>() else {
            continue;
        };
        let used = fopt(f[1]).unwrap_or(0.0) as u64 * 1024 * 1024;
        procs.push(GpuProc {
            pid,
            used_mem: used,
        });
    }
    procs
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
/// when the card exposes neither utilization nor VRAM info. `temp_milli` is in
/// millidegrees Celsius.
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
        mem_util: 0.0,
        has_mem_util: false,
        mem_used: vram_used.unwrap_or(0),
        mem_total: vram_total.unwrap_or(0),
        temp: temp_milli.map(|t| t as f32 / 1000.0).unwrap_or(0.0),
        power: 0.0,
        power_limit: 0.0,
        throttled: false,
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
        let Some(dir) = std::fs::read_dir(hw.path()).ok() else {
            continue;
        };
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

fn nvidia_smi(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("nvidia-smi")
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Query NVIDIA, preferring the extended field set and falling back to the
/// legacy one on older drivers that reject the newer query fields.
fn query_nvidia() -> Option<Vec<Gpu>> {
    if let Some(out) = nvidia_smi(&[
        "--query-gpu=name,utilization.gpu,utilization.memory,memory.used,memory.total,temperature.gpu,power.draw,power.limit,clocks_throttle_reasons.active",
        "--format=csv,noheader,nounits",
    ]) {
        let gpus = parse_nvidia_smi_ext(&out);
        if !gpus.is_empty() {
            return Some(gpus);
        }
    }
    nvidia_smi(&[
        "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
        "--format=csv,noheader,nounits",
    ])
    .map(|out| parse_nvidia_smi(&out))
}

fn query_nvidia_procs() -> Vec<GpuProc> {
    nvidia_smi(&[
        "--query-compute-apps=pid,used_memory",
        "--format=csv,noheader,nounits",
    ])
    .map(|out| parse_compute_apps(&out))
    .unwrap_or_default()
}

/// Apple Silicon GPU metrics via IOKit's `IOAccelerator` `PerformanceStatistics`
/// dictionary — the same no-root source `asitop`/`macmon` read. Raw FFI against
/// the system CoreFoundation/IOKit frameworks, so we add no crates.
#[cfg(target_os = "macos")]
mod apple {
    use std::ffi::{c_void, CString};
    use std::os::raw::{c_char, c_int, c_long};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type IoObject = u32;

    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
                                   // kCFNumberSInt64Type. CFNumberType is backed by CFIndex (c_long, 64-bit on
                                   // macOS), so this must be c_long — a c_int here is an FFI ABI mismatch.
    const SINT64: c_long = 4;
    const NULL_ALLOC: CFAllocatorRef = std::ptr::null();

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(a: CFAllocatorRef, s: *const c_char, enc: u32) -> CFStringRef;
        fn CFDictionaryGetValue(d: CFDictionaryRef, k: *const c_void) -> *const c_void;
        fn CFNumberGetValue(n: *const c_void, t: c_long, v: *mut c_void) -> bool;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const c_char) -> CFDictionaryRef;
        fn IOServiceGetMatchingService(main_port: IoObject, matching: CFDictionaryRef) -> IoObject;
        fn IORegistryEntryCreateCFProperty(
            entry: IoObject,
            key: CFStringRef,
            alloc: CFAllocatorRef,
            opts: u32,
        ) -> CFTypeRef;
        fn IOObjectRelease(obj: IoObject) -> c_int;
    }

    /// Create a CFString the caller owns (must `CFRelease`). None on failure.
    unsafe fn cfstr(s: &str) -> Option<CFStringRef> {
        let c = CString::new(s).ok()?;
        let r = CFStringCreateWithCString(NULL_ALLOC, c.as_ptr(), UTF8);
        if r.is_null() {
            None
        } else {
            Some(r)
        }
    }

    /// Read an integer from a CFDictionary by string key. The value is borrowed
    /// (Get-rule), so it is not released here.
    unsafe fn dict_i64(dict: CFDictionaryRef, key: &str) -> Option<i64> {
        let k = cfstr(key)?;
        let val = CFDictionaryGetValue(dict, k);
        CFRelease(k);
        if val.is_null() {
            return None;
        }
        let mut out: i64 = 0;
        let ok = CFNumberGetValue(val, SINT64, &mut out as *mut i64 as *mut c_void);
        ok.then_some(out)
    }

    /// GPU core utilization %, or None if IOKit doesn't report it.
    pub fn utilization() -> Option<f32> {
        // SAFETY: a standard IOKit registry read. Ownership: the matching dict
        // is consumed by IOServiceGetMatchingService; the service object and the
        // Create-rule PerformanceStatistics dict are released here; dictionary
        // values are Get-rule and not released. Port 0 is kIOMainPortDefault.
        unsafe {
            let name = CString::new("IOAccelerator").ok()?;
            let matching = IOServiceMatching(name.as_ptr());
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return None;
            }
            let Some(key) = cfstr("PerformanceStatistics") else {
                IOObjectRelease(service);
                return None;
            };
            let perf = IORegistryEntryCreateCFProperty(service, key, NULL_ALLOC, 0);
            CFRelease(key);
            IOObjectRelease(service);
            if perf.is_null() {
                return None;
            }
            let util = dict_i64(perf, "Device Utilization %");
            CFRelease(perf);
            util.map(|u| u.clamp(0, 100) as f32)
        }
    }
}

/// Apple Silicon GPU as a `Gpu` row: real utilization, no discrete VRAM (unified
/// memory representation is deferred — see ur-grue/toptop#4).
#[cfg(target_os = "macos")]
fn apple_gpus() -> Vec<Gpu> {
    match apple::utilization() {
        Some(util) => vec![Gpu {
            name: "Apple Silicon GPU".to_string(),
            util_pct: util,
            has_util: true,
            mem_util: 0.0,
            has_mem_util: false,
            mem_used: 0,
            mem_total: 0,
            temp: 0.0,
            power: 0.0,
            power_limit: 0.0,
            throttled: false,
        }],
        None => Vec::new(),
    }
}

/// Human explanation for an empty GPU list, tailored to the build target so
/// the AI view is honest instead of blank. On Apple Silicon a capable GPU
/// exists — toptop just has no metrics source for it yet — so this must NOT
/// claim "no GPU". Exactly one arm compiles per platform.
pub fn no_gpu_reason() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "Apple Silicon GPU metrics aren't wired up yet (tracked in ur-grue/toptop#4)."
    }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    {
        "No GPU metrics source on this Mac."
    }
    #[cfg(not(target_os = "macos"))]
    {
        "No GPU metrics source found (needs nvidia-smi, or /sys/class/drm for AMD/Intel)."
    }
}

/// Combine all GPU sources: NVIDIA via `nvidia-smi`, AMD/Intel via sysfs, and
/// Apple Silicon via IOKit.
fn query_all() -> GpuSnapshot {
    let mut gpus = query_nvidia().unwrap_or_default();
    let nvidia_present = !gpus.is_empty();
    gpus.extend(read_sysfs_gpus());
    #[cfg(target_os = "macos")]
    gpus.extend(apple_gpus());
    let procs = if nvidia_present {
        query_nvidia_procs()
    } else {
        Vec::new()
    };
    GpuSnapshot { gpus, procs }
}

/// Background poller that keeps the latest GPU snapshot in a shared slot.
pub struct GpuMonitor {
    latest: Arc<Mutex<GpuSnapshot>>,
}

impl GpuMonitor {
    /// Probe for any GPU source; if present, spawn a polling thread. Otherwise
    /// the monitor exists but always reports an empty snapshot.
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(GpuSnapshot::default()));
        let initial = query_all();
        if !initial.gpus.is_empty() {
            if let Ok(mut slot) = latest.lock() {
                *slot = initial;
            }
            let shared = Arc::clone(&latest);
            std::thread::Builder::new()
                .name("toptop-gpu".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(2000));
                    let snap = query_all();
                    if let Ok(mut slot) = shared.lock() {
                        *slot = snap;
                    }
                })
                .ok();
        }
        Self { latest }
    }

    /// The most recent GPU snapshot (clone of the shared slot).
    pub fn snapshot(&self) -> GpuSnapshot {
        self.latest.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, target_os = "macos"))]
mod apple_tests {
    /// The IOKit read must never panic and, when it reports a value, stay in
    /// range. Accepts None so it's not flaky on Macs/CI without an accelerator.
    #[test]
    fn utilization_reads_or_none() {
        if let Some(u) = super::apple::utilization() {
            assert!((0.0..=100.0).contains(&u), "util out of range: {u}");
        }
        // apple_gpus() must also be panic-free and produce at most one row.
        assert!(super::apple_gpus().len() <= 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gpu_reason_is_platform_honest() {
        let msg = no_gpu_reason();
        assert!(!msg.is_empty());
        // On Apple Silicon a GPU exists — the message must not deny that, and
        // must point at the tracking issue.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            assert!(msg.contains("Apple Silicon"));
            assert!(msg.contains("#4"));
            assert!(!msg.to_lowercase().contains("no gpu"));
        }
    }

    #[test]
    fn parses_legacy() {
        let g = &parse_nvidia_smi("NVIDIA T4, 10, 100, 1000, 40")[0];
        assert_eq!(g.name, "NVIDIA T4");
        assert!(g.has_util && !g.has_mem_util);
        assert_eq!(g.mem_used, 100 * 1024 * 1024);
    }

    #[test]
    fn parses_extended() {
        let out = "NVIDIA RTX 4090, 87, 63, 21000, 24564, 71, 320.5, 450.0, 0x0000000000000000";
        let g = &parse_nvidia_smi_ext(out)[0];
        assert_eq!(g.util_pct, 87.0);
        assert_eq!(g.mem_util, 63.0);
        assert!(g.has_mem_util);
        assert_eq!(g.power, 320.5);
        assert_eq!(g.power_limit, 450.0);
        assert!(!g.throttled);
    }

    #[test]
    fn detects_throttle_and_na() {
        // HW thermal slowdown (0x40) set, and some fields are [N/A].
        let out = "NVIDIA A100, [N/A], 50, 4000, 40960, 88, [N/A], 250, 0x0000000000000040";
        let g = &parse_nvidia_smi_ext(out)[0];
        assert!(g.throttled);
        assert!(!g.has_util); // [N/A] util
        assert_eq!(g.power, 0.0); // [N/A] power → 0
        assert_eq!(g.mem_util, 50.0);
    }

    #[test]
    fn compute_apps_parse() {
        let out = "1234, 2048\n5678, 512\nbroken\n";
        let p = parse_compute_apps(out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].pid, 1234);
        assert_eq!(p[0].used_mem, 2048 * 1024 * 1024);
    }

    #[test]
    fn sysfs_amd_full() {
        let g = build_sysfs_gpu(
            Some("amdgpu"),
            Some(73),
            Some(2048),
            Some(8192),
            Some(61000),
        )
        .expect("amd gpu");
        assert_eq!(g.name, "AMD GPU");
        assert!(g.has_util && !g.has_mem_util);
        assert_eq!(g.temp, 61.0);
    }

    #[test]
    fn sysfs_intel_no_util() {
        let g =
            build_sysfs_gpu(Some("i915"), None, None, Some(1024), Some(45000)).expect("intel gpu");
        assert_eq!(g.name, "Intel GPU");
        assert!(!g.has_util);
    }

    #[test]
    fn sysfs_rejects_non_gpu() {
        assert!(build_sysfs_gpu(Some("simpledrm"), None, None, None, Some(0)).is_none());
    }
}
