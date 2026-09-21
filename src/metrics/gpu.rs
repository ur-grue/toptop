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
    s.parse::<f32>().ok().filter(|v| v.is_finite())
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
    use super::{MacGpuDevice, MacGpuStats};
    use std::ffi::{c_void, CString};
    use std::os::raw::{c_char, c_int, c_long};
    use std::sync::OnceLock;

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

    #[link(name = "Metal", kind = "framework")]
    extern "C" {
        fn MTLCreateSystemDefaultDevice() -> *const c_void;
    }

    #[link(name = "objc")]
    extern "C" {
        fn objc_msgSend(obj: *const c_void, sel: *const c_void) -> u64;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_release(obj: *const c_void);
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

    /// Send a no-argument message and get the raw register back.
    unsafe fn send(obj: *const c_void, selector: &std::ffi::CStr) -> u64 {
        let sel = sel_registerName(selector.as_ptr());
        if sel.is_null() {
            0
        } else {
            objc_msgSend(obj, sel)
        }
    }

    /// What the machine falls back to when Metal is unavailable: the build
    /// target still tells us the memory architecture.
    fn fallback_device() -> MacGpuDevice {
        MacGpuDevice {
            name: if cfg!(target_arch = "aarch64") {
                "Apple Silicon GPU".to_string()
            } else {
                "GPU".to_string()
            },
            unified: cfg!(target_arch = "aarch64"),
            max_mem: 0,
        }
    }

    /// The default Metal device: name, memory architecture and
    /// `recommendedMaxWorkingSetSize`. Cached — none of it changes for the
    /// lifetime of the process.
    pub fn metal_device() -> &'static MacGpuDevice {
        static CACHED: OnceLock<MacGpuDevice> = OnceLock::new();
        CACHED.get_or_init(|| {
            // SAFETY: MTLCreateSystemDefaultDevice returns a retained ObjC
            // object (Create rule); released via objc_release once every
            // property has been copied out. `name` returns an NSString owned
            // by the device (Get rule) — read via UTF8String before release.
            // objc_msgSend returning u64 is the integer/pointer register on
            // both x86_64 and ARM64; BOOL only defines the low byte, so
            // hasUnifiedMemory is masked.
            unsafe {
                let device = MTLCreateSystemDefaultDevice();
                if device.is_null() {
                    return fallback_device();
                }
                let max_mem = send(device, c"recommendedMaxWorkingSetSize");
                let unified = send(device, c"hasUnifiedMemory") & 0xff != 0;
                let ns_name = send(device, c"name") as *const c_void;
                let name = if ns_name.is_null() {
                    String::new()
                } else {
                    let utf8 = send(ns_name, c"UTF8String") as *const c_char;
                    if utf8.is_null() {
                        String::new()
                    } else {
                        std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()
                    }
                };
                objc_release(device);
                let name = if name.trim().is_empty() {
                    fallback_device().name
                } else {
                    name
                };
                MacGpuDevice { name, unified, max_mem }
            }
        })
    }

    /// One `PerformanceStatistics` sample from the first `IOAccelerator`
    /// service. Returns `None` when no accelerator service exists.
    pub fn read_stats() -> Option<MacGpuStats> {
        // SAFETY: standard IOKit registry read. Ownership: the matching dict
        // is consumed by IOServiceGetMatchingService; the service object and
        // Create-rule PerformanceStatistics dict are released; dict values are
        // Get-rule (not released). Port 0 is kIOMainPortDefault.
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
            let util = dict_i64(perf, "Device Utilization %")
                .map(|u| u.clamp(0, 100) as f32);
            let sys_mem_used = dict_i64(perf, "In use system memory")
                .map(|m| m.max(0) as u64)
                .unwrap_or(0);
            // Discrete cards (AMD in Intel Macs) publish VRAM residency and
            // sensors under these keys; Apple Silicon has none of them.
            let vid_mem_used = dict_i64(perf, "inUseVidMemoryBytes").map(|m| m.max(0) as u64);
            let temp = dict_i64(perf, "Temperature(C)")
                .filter(|t| *t > 0)
                .map(|t| t as f32);
            let power = dict_i64(perf, "Total Power(W)")
                .filter(|p| *p > 0)
                .map(|p| p as f32);
            CFRelease(perf);
            Some(MacGpuStats {
                util,
                sys_mem_used,
                vid_mem_used,
                temp,
                power,
            })
        }
    }
}

/// What Metal reports about the default GPU on a Mac. Platform-neutral so the
/// row builder below can be unit-tested anywhere.
#[derive(Debug, Clone, PartialEq)]
pub struct MacGpuDevice {
    /// Marketing name from `MTLDevice.name`, e.g. "Apple M2 Max" or
    /// "AMD Radeon Pro 5700 XT".
    pub name: String,
    /// `MTLDevice.hasUnifiedMemory`: true on Apple Silicon, false for the
    /// discrete AMD cards in Intel Macs.
    pub unified: bool,
    /// `recommendedMaxWorkingSetSize`: the GPU's usable share of unified
    /// memory, or the card's VRAM on a discrete GPU. 0 when Metal is absent.
    pub max_mem: u64,
}

/// One IOKit `PerformanceStatistics` sample, platform-neutral.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MacGpuStats {
    pub util: Option<f32>,
    /// "In use system memory" — what a unified-memory GPU has resident.
    pub sys_mem_used: u64,
    /// "inUseVidMemoryBytes" — what a discrete card has resident in VRAM.
    pub vid_mem_used: Option<u64>,
    pub temp: Option<f32>,
    pub power: Option<f32>,
}

/// Build the `Gpu` row for a Mac GPU. Unified-memory GPUs (Apple Silicon)
/// report their resident system memory against Metal's working-set ceiling;
/// discrete cards (AMD in Intel Macs) report VRAM in use against VRAM total.
/// When `mem_used / mem_total` nears 100 %, models start spilling layers —
/// a 5–20× slowdown the UI warns about.
pub fn mac_gpu_row(dev: &MacGpuDevice, s: &MacGpuStats) -> Gpu {
    let mem_used = if dev.unified {
        s.sys_mem_used
    } else {
        s.vid_mem_used.unwrap_or(0)
    };
    Gpu {
        name: dev.name.clone(),
        util_pct: s.util.unwrap_or(0.0),
        has_util: s.util.is_some(),
        mem_util: 0.0,
        has_mem_util: false,
        mem_used,
        mem_total: dev.max_mem,
        temp: s.temp.unwrap_or(0.0),
        power: s.power.unwrap_or(0.0),
        power_limit: 0.0,
        throttled: false,
    }
}

#[cfg(target_os = "macos")]
fn apple_gpus() -> Vec<Gpu> {
    let Some(stats) = apple::read_stats() else {
        return Vec::new();
    };
    vec![mac_gpu_row(apple::metal_device(), &stats)]
}

/// Human explanation for an empty GPU list, tailored to the build target so
/// the AI view is honest instead of blank. On Apple Silicon a capable GPU
/// exists — toptop just has no metrics source for it yet — so this must NOT
/// claim "no GPU". Exactly one arm compiles per platform.
pub fn no_gpu_reason() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "Could not read Apple Silicon GPU metrics from IOKit."
    }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    {
        "Could not read GPU metrics from IOKit (no IOAccelerator service)."
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
    #[test]
    fn reads_or_none() {
        if let Some(s) = super::apple::read_stats() {
            if let Some(u) = s.util {
                assert!((0.0..=100.0).contains(&u), "util out of range: {u}");
            }
        }
        assert!(super::apple_gpus().len() <= 1);
    }

    #[test]
    fn metal_device_is_cached_and_named() {
        let a = super::apple::metal_device();
        let b = super::apple::metal_device();
        assert!(std::ptr::eq(a, b), "OnceLock value must be stable");
        assert!(!a.name.trim().is_empty(), "device must have a name");
        if a.max_mem > 0 {
            assert!(a.max_mem >= 1024 * 1024 * 1024, "suspiciously small: {}", a.max_mem);
        }
        // When Metal answered, it must agree with the build target on the
        // memory architecture: unified on Apple Silicon, discrete on Intel.
        if a.max_mem > 0 {
            assert_eq!(a.unified, cfg!(target_arch = "aarch64"));
        }
    }

    #[test]
    fn row_uses_the_real_device_name() {
        for g in super::apple_gpus() {
            assert_eq!(g.name, super::apple::metal_device().name);
            if super::apple::metal_device().max_mem > 0 {
                assert!(g.mem_total > 0, "mem_total should come from Metal");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gpu_reason_is_platform_honest() {
        let msg = no_gpu_reason();
        assert!(!msg.is_empty());
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            assert!(msg.contains("Apple Silicon"));
            assert!(!msg.to_lowercase().contains("no gpu"));
        }
    }

    #[test]
    fn intel_mac_with_discrete_amd_is_named_and_measured_as_vram() {
        // An Intel iMac with a Radeon: Metal says "not unified", IOKit has
        // VRAM-in-use, temperature and power. The row must carry the real
        // card name (not "Apple Silicon GPU") and VRAM, not system memory.
        let dev = MacGpuDevice {
            name: "AMD Radeon Pro 5700 XT".into(),
            unified: false,
            max_mem: 16 * 1024 * 1024 * 1024,
        };
        let s = MacGpuStats {
            util: Some(35.0),
            sys_mem_used: 7_139_328,
            vid_mem_used: Some(2_450_108_416),
            temp: Some(63.0),
            power: Some(14.0),
        };
        let g = mac_gpu_row(&dev, &s);
        assert_eq!(g.name, "AMD Radeon Pro 5700 XT");
        assert_eq!(g.mem_used, 2_450_108_416);
        assert_eq!(g.mem_total, 16 * 1024 * 1024 * 1024);
        assert_eq!(g.temp, 63.0);
        assert_eq!(g.power, 14.0);
        assert!(g.has_util && g.util_pct == 35.0);
    }

    #[test]
    fn apple_silicon_reports_unified_memory_against_working_set() {
        let dev = MacGpuDevice {
            name: "Apple M2 Max".into(),
            unified: true,
            max_mem: 48 * 1024 * 1024 * 1024,
        };
        let s = MacGpuStats {
            util: Some(80.0),
            sys_mem_used: 30 * 1024 * 1024 * 1024,
            vid_mem_used: None,
            temp: None,
            power: None,
        };
        let g = mac_gpu_row(&dev, &s);
        assert_eq!(g.name, "Apple M2 Max");
        assert_eq!(g.mem_used, 30 * 1024 * 1024 * 1024);
        // No sensor → 0, which the UI renders as "—", never as 0 °C.
        assert_eq!(g.temp, 0.0);
        assert_eq!(g.power, 0.0);
    }

    #[test]
    fn mac_gpu_without_utilization_says_so() {
        let dev = MacGpuDevice {
            name: "Apple M1".into(),
            unified: true,
            max_mem: 0,
        };
        let g = mac_gpu_row(&dev, &MacGpuStats::default());
        assert!(!g.has_util);
        assert_eq!(g.mem_total, 0);
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
