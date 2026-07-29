//! System data collection built on top of `sysinfo`.
//!
//! [`Collector`] owns the long-lived `sysinfo` handles and all the time-series
//! histories. Calling [`Collector::refresh`] once per tick updates every public
//! field in place — the UI layer only ever reads from a `&Collector`.

use std::time::{Duration, Instant};

use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind,
    ProcessStatus, ProcessesToUpdate, RefreshKind, Signal, System, UpdateKind, Users,
};

pub mod ai;
pub mod gpu;
pub mod infer;
pub mod netconn;

use crate::history::History;
use gpu::{Gpu, GpuMonitor, GpuProc};
use infer::InferenceMonitor;
pub use infer::ServerStats;
pub use netconn::Connection;

/// Static-ish information about the host, captured once at startup.
#[derive(Clone, Debug, Default)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_brand: String,
    pub logical_cores: usize,
    pub physical_cores: usize,
}

/// Aggregate and per-core CPU state.
pub struct CpuData {
    pub global_usage: f32,
    pub freq_mhz: u64,
    pub load_avg: (f64, f64, f64),
    pub per_core: Vec<f32>,
    pub global_history: History,
    pub core_history: Vec<History>,
}

/// RAM and swap state. Percentages are derived in the UI from these byte counts.
#[derive(Default)]
pub struct MemData {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub used_history: History,
    pub swap_history: History,
}

/// A single network interface with smoothed up/down rate histories.
pub struct NetIf {
    pub name: String,
    pub down_rate: f64,
    pub up_rate: f64,
    pub total_down: u64,
    pub total_up: u64,
    pub down_history: History,
    pub up_history: History,
    last_total_down: u64,
    last_total_up: u64,
    seen: bool,
}

/// A mounted filesystem.
#[derive(Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub fs: String,
    pub total: u64,
    pub available: u64,
    pub used_pct: f32,
    pub removable: bool,
}

/// One process row.
#[derive(Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub cmd: String,
    pub user: String,
    pub cpu: f32,
    pub mem_pct: f32,
    pub mem_bytes: u64,
    pub virt: u64,
    pub disk_read: u64,
    pub disk_written: u64,
    pub io_read_rate: f64,
    pub io_write_rate: f64,
    pub start_time: u64,
    pub run_time: u64,
    pub status: char,
    pub status_long: &'static str,
    pub threads: usize,
    /// Indentation depth when rendered as a tree (0 when flat).
    pub depth: usize,
}

/// A temperature sensor reading.
#[derive(Clone)]
pub struct SensorInfo {
    pub label: String,
    pub temp: f32,
    pub high: Option<f32>,
    pub critical: Option<f32>,
}

/// Battery state, read from `/sys/class/power_supply` when present.
#[derive(Clone)]
pub struct Battery {
    pub percent: f32,
    pub status: String,
}

/// The single source of truth the UI reads from.
pub struct Collector {
    sys: System,
    networks: Networks,
    disks: Disks,
    components: Components,
    users: Users,
    refresh_kind: RefreshKind,
    proc_refresh: ProcessRefreshKind,
    gpu_monitor: GpuMonitor,
    infer_monitor: InferenceMonitor,
    /// Per-PID cumulative (read, written) bytes from the previous tick, used to
    /// derive per-process I/O rates.
    prev_proc_io: std::collections::HashMap<u32, (u64, u64)>,
    last_instant: Option<Instant>,
    last_battery_at: Option<Instant>,
    history_len: usize,

    pub host: HostInfo,
    pub cpu: CpuData,
    pub mem: MemData,
    pub nets: Vec<NetIf>,
    pub disk_list: Vec<DiskInfo>,
    pub disk_read_rate: f64,
    pub disk_write_rate: f64,
    pub disk_read_history: History,
    pub disk_write_history: History,
    last_disk_read: u64,
    last_disk_write: u64,
    pub procs: Vec<ProcInfo>,
    pub sensors: Vec<SensorInfo>,
    pub gpus: Vec<Gpu>,
    /// Processes holding GPU memory (NVIDIA only), for the AI/LLM view.
    pub gpu_procs: Vec<GpuProc>,
    /// Auto-discovered local inference servers (tokens/sec, KV cache, …).
    pub servers: Vec<ServerStats>,
    pub battery: Option<Battery>,
    pub uptime: u64,
}

impl Collector {
    /// Build a collector and perform an initial population pass.
    pub fn new(history_len: usize) -> Self {
        let refresh_kind = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());
        // The default process refresh skips users and full command lines, so
        // request them explicitly (resolved once, then cached).
        let proc_refresh = ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_disk_usage()
            .with_user(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_tasks();
        let mut sys = System::new_with_specifics(refresh_kind);
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_refresh);

        let logical = sys.cpus().len();
        let host = HostInfo {
            hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
            os: System::long_os_version()
                .or_else(System::os_version)
                .or_else(System::name)
                .unwrap_or_else(|| "Unknown OS".into()),
            kernel: System::kernel_version().unwrap_or_else(|| "?".into()),
            arch: System::cpu_arch(),
            cpu_brand: sys
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_string())
                .unwrap_or_default(),
            logical_cores: logical,
            physical_cores: System::physical_core_count().unwrap_or(logical),
        };

        let cpu = CpuData {
            global_usage: 0.0,
            freq_mhz: 0,
            load_avg: (0.0, 0.0, 0.0),
            per_core: vec![0.0; logical],
            global_history: History::new(history_len),
            core_history: (0..logical).map(|_| History::new(history_len)).collect(),
        };

        let mut collector = Self {
            sys,
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            components: Components::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            refresh_kind,
            proc_refresh,
            gpu_monitor: GpuMonitor::new(),
            infer_monitor: InferenceMonitor::new(),
            prev_proc_io: std::collections::HashMap::new(),
            last_instant: None,
            last_battery_at: None,
            history_len,
            host,
            cpu,
            mem: MemData {
                used_history: History::new(history_len),
                swap_history: History::new(history_len),
                ..Default::default()
            },
            nets: Vec::new(),
            disk_list: Vec::new(),
            disk_read_rate: 0.0,
            disk_write_rate: 0.0,
            disk_read_history: History::new(history_len),
            disk_write_history: History::new(history_len),
            last_disk_read: 0,
            last_disk_write: 0,
            procs: Vec::new(),
            sensors: Vec::new(),
            gpus: Vec::new(),
            gpu_procs: Vec::new(),
            servers: Vec::new(),
            battery: None,
            uptime: 0,
        };
        collector.refresh();
        collector
    }

    /// Refresh every metric. Rates are computed against elapsed wall-clock time
    /// so they stay correct regardless of the configured tick interval.
    pub fn refresh(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .last_instant
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(1.0)
            .max(1e-3);
        self.last_instant = Some(now);

        self.sys.refresh_specifics(self.refresh_kind);
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, self.proc_refresh);
        self.networks.refresh(true);
        self.disks.refresh(true);
        self.components.refresh(true);

        self.uptime = System::uptime();
        self.refresh_cpu();
        self.refresh_mem();
        self.refresh_net(elapsed);
        self.refresh_disks(elapsed);
        self.refresh_procs(elapsed);
        self.refresh_sensors();
        let gpu_snap = self.gpu_monitor.snapshot();
        self.gpus = gpu_snap.gpus;
        self.gpu_procs = gpu_snap.procs;
        self.servers = self.infer_monitor.snapshot();
        // Battery level moves on the order of minutes; poll it at most this
        // often rather than doing filesystem I/O on every (up to 4×/s) tick.
        const BATTERY_POLL: Duration = Duration::from_secs(10);
        if self
            .last_battery_at
            .map(|t| now.duration_since(t) >= BATTERY_POLL)
            .unwrap_or(true)
        {
            self.battery = read_battery();
            self.last_battery_at = Some(now);
        }
    }

    fn refresh_cpu(&mut self) {
        self.cpu.global_usage = self.sys.global_cpu_usage();
        self.cpu.global_history.push(self.cpu.global_usage as f64);

        let cpus = self.sys.cpus();
        if self.cpu.per_core.len() != cpus.len() {
            self.cpu.per_core = vec![0.0; cpus.len()];
            self.cpu.core_history = (0..cpus.len())
                .map(|_| History::new(self.history_len))
                .collect();
        }
        let mut freq = 0u64;
        for (i, cpu) in cpus.iter().enumerate() {
            let usage = cpu.cpu_usage();
            self.cpu.per_core[i] = usage;
            self.cpu.core_history[i].push(usage as f64);
            freq = freq.max(cpu.frequency());
        }
        self.cpu.freq_mhz = freq;
        let la = System::load_average();
        self.cpu.load_avg = (la.one, la.five, la.fifteen);
    }

    fn refresh_mem(&mut self) {
        self.mem.total = self.sys.total_memory();
        self.mem.used = self.sys.used_memory();
        self.mem.available = self.sys.available_memory();
        self.mem.swap_total = self.sys.total_swap();
        self.mem.swap_used = self.sys.used_swap();
        let used_pct = pct(self.mem.used, self.mem.total);
        let swap_pct = pct(self.mem.swap_used, self.mem.swap_total);
        self.mem.used_history.push(used_pct as f64);
        self.mem.swap_history.push(swap_pct as f64);
    }

    fn refresh_net(&mut self, elapsed: f64) {
        for n in self.nets.iter_mut() {
            n.seen = false;
        }
        for (name, data) in self.networks.list() {
            let total_down = data.total_received();
            let total_up = data.total_transmitted();
            let entry = match self.nets.iter_mut().find(|n| &n.name == name) {
                Some(e) => e,
                None => {
                    self.nets.push(NetIf {
                        name: name.clone(),
                        down_rate: 0.0,
                        up_rate: 0.0,
                        total_down,
                        total_up,
                        down_history: History::new(self.history_len),
                        up_history: History::new(self.history_len),
                        last_total_down: total_down,
                        last_total_up: total_up,
                        seen: true,
                    });
                    continue;
                }
            };
            let d_down = total_down.saturating_sub(entry.last_total_down) as f64 / elapsed;
            let d_up = total_up.saturating_sub(entry.last_total_up) as f64 / elapsed;
            entry.down_rate = d_down;
            entry.up_rate = d_up;
            entry.total_down = total_down;
            entry.total_up = total_up;
            entry.last_total_down = total_down;
            entry.last_total_up = total_up;
            entry.down_history.push(d_down);
            entry.up_history.push(d_up);
            entry.seen = true;
        }
        self.nets.retain(|n| n.seen);
        // Most active interfaces first.
        self.nets
            .sort_by_key(|n| std::cmp::Reverse(n.total_down + n.total_up));
    }

    fn refresh_disks(&mut self, elapsed: f64) {
        self.disk_list.clear();
        let mut total_read = 0u64;
        let mut total_write = 0u64;
        for disk in self.disks.list() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            self.disk_list.push(DiskInfo {
                name: disk.name().to_string_lossy().to_string(),
                mount: disk.mount_point().to_string_lossy().to_string(),
                fs: disk.file_system().to_string_lossy().to_string(),
                total,
                available,
                used_pct: pct(used, total),
                removable: disk.is_removable(),
            });
            let usage = disk.usage();
            total_read = total_read.saturating_add(usage.total_read_bytes);
            total_write = total_write.saturating_add(usage.total_written_bytes);
        }
        self.disk_list.sort_by(|a, b| a.mount.cmp(&b.mount));

        if self.last_disk_read == 0 && self.last_disk_write == 0 {
            self.last_disk_read = total_read;
            self.last_disk_write = total_write;
        }
        self.disk_read_rate = total_read.saturating_sub(self.last_disk_read) as f64 / elapsed;
        self.disk_write_rate = total_write.saturating_sub(self.last_disk_write) as f64 / elapsed;
        self.last_disk_read = total_read;
        self.last_disk_write = total_write;
        self.disk_read_history.push(self.disk_read_rate);
        self.disk_write_history.push(self.disk_write_rate);
    }

    fn refresh_procs(&mut self, elapsed: f64) {
        let total_mem = self.sys.total_memory().max(1);
        let mut procs = Vec::with_capacity(self.sys.processes().len());
        let mut io_now: std::collections::HashMap<u32, (u64, u64)> =
            std::collections::HashMap::with_capacity(self.sys.processes().len());
        for (pid, proc_) in self.sys.processes() {
            let name = proc_.name().to_string_lossy().to_string();
            let cmd = {
                let joined = proc_
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ");
                if joined.trim().is_empty() {
                    name.clone()
                } else {
                    joined
                }
            };
            // Resolve the owning user's name; fall back to the numeric UID when
            // the user database can't be read (common in minimal containers).
            let user = proc_
                .user_id()
                .map(|uid| {
                    self.users
                        .get_user_by_id(uid)
                        .map(|u| u.name().to_string())
                        .unwrap_or_else(|| (**uid).to_string())
                })
                .unwrap_or_else(|| "—".into());
            let du = proc_.disk_usage();
            let status = proc_.status();
            let pid_u32 = pid.as_u32();
            io_now.insert(pid_u32, (du.total_read_bytes, du.total_written_bytes));
            let (io_read_rate, io_write_rate) = self
                .prev_proc_io
                .get(&pid_u32)
                .map(|&(pr, pw)| {
                    (
                        du.total_read_bytes.saturating_sub(pr) as f64 / elapsed,
                        du.total_written_bytes.saturating_sub(pw) as f64 / elapsed,
                    )
                })
                .unwrap_or((0.0, 0.0));
            procs.push(ProcInfo {
                pid: pid_u32,
                ppid: proc_.parent().map(|p| p.as_u32()),
                name,
                cmd,
                user,
                cpu: proc_.cpu_usage(),
                mem_pct: (proc_.memory() as f64 / total_mem as f64 * 100.0) as f32,
                mem_bytes: proc_.memory(),
                virt: proc_.virtual_memory(),
                disk_read: du.total_read_bytes,
                disk_written: du.total_written_bytes,
                io_read_rate,
                io_write_rate,
                start_time: proc_.start_time(),
                run_time: proc_.run_time(),
                status: status_char(status),
                status_long: status_label(status),
                threads: proc_.tasks().map(|t| t.len()).unwrap_or(0),
                depth: 0,
            });
        }
        self.procs = procs;
        self.prev_proc_io = io_now;
    }

    fn refresh_sensors(&mut self) {
        self.sensors.clear();
        for comp in self.components.list() {
            if let Some(temp) = comp.temperature() {
                self.sensors.push(SensorInfo {
                    label: comp.label().to_string(),
                    temp,
                    high: comp.max(),
                    critical: comp.critical(),
                });
            }
        }
        self.sensors.sort_by(|a, b| {
            b.temp
                .partial_cmp(&a.temp)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Number of running (non-sleeping) processes, for the header summary.
    pub fn running_procs(&self) -> usize {
        self.procs.iter().filter(|p| p.status == 'R').count()
    }

    /// Enumerate current network connections (TCP/UDP) joined to processes.
    /// Computed on demand — only call this while the connections view is open.
    pub fn connections(&self) -> Vec<Connection> {
        netconn::collect()
    }

    /// Send a signal to a process, distinguishing the ways it can fail so the
    /// UI can explain what happened instead of a blanket "failed". Uses the
    /// raw `kill(2)` so the exact errno separates permission from liveness.
    pub fn signal_process(&self, pid: u32, signal: Signal) -> SignalOutcome {
        let Some(raw) = raw_signal(signal) else {
            return SignalOutcome::Unsupported;
        };
        // SAFETY: kill() only inspects its integer arguments and has no memory
        // effects; we read errno via last_os_error() only when it fails.
        let rc = unsafe { libc::kill(pid as libc::pid_t, raw) };
        if rc == 0 {
            return SignalOutcome::Delivered;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) => SignalOutcome::NotPermitted,
            Some(libc::ESRCH) => SignalOutcome::Gone,
            Some(libc::EINVAL) => SignalOutcome::Unsupported,
            _ => SignalOutcome::Gone,
        }
    }

    /// Change a process's nice value (scheduling priority). Mirrors
    /// `signal_process`'s outcome classification via the raw `setpriority(2)`
    /// errno — raising priority (a lower nice) or renicing another user's
    /// process needs privilege.
    pub fn set_priority(&self, pid: u32, nice: i32) -> PriorityOutcome {
        // SAFETY: setpriority() only inspects its integer arguments; we read
        // errno via last_os_error() only when it fails.
        let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) };
        if rc == 0 {
            return PriorityOutcome::Applied;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) | Some(libc::EACCES) => PriorityOutcome::NotPermitted,
            Some(libc::ESRCH) => PriorityOutcome::Gone,
            _ => PriorityOutcome::Gone,
        }
    }

    /// Resolve the executable path and working directory for a single process,
    /// fetched on demand for the detail overlay rather than cached per row.
    pub fn proc_paths(&self, pid: u32) -> (String, String) {
        match self.sys.process(Pid::from_u32(pid)) {
            Some(p) => (
                p.exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                p.cwd()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
            None => (String::new(), String::new()),
        }
    }
}

/// The signals offered by the interactive menu, in display order: label,
/// platform signal number (straight from libc), and the sysinfo `Signal`.
/// Single source of truth — the menu, status line, and delivery all derive
/// from this, so a new signal is added in exactly one place.
pub const SIGNALS: &[(&str, i32, Signal)] = &[
    ("SIGTERM", libc::SIGTERM, Signal::Term),
    ("SIGKILL", libc::SIGKILL, Signal::Kill),
    ("SIGINT", libc::SIGINT, Signal::Interrupt),
    ("SIGHUP", libc::SIGHUP, Signal::Hangup),
    ("SIGQUIT", libc::SIGQUIT, Signal::Quit),
    ("SIGSTOP", libc::SIGSTOP, Signal::Stop),
    ("SIGCONT", libc::SIGCONT, Signal::Continue),
    ("SIGUSR1", libc::SIGUSR1, Signal::User1),
    ("SIGUSR2", libc::SIGUSR2, Signal::User2),
];

/// Human label for a signal, or "signal" if it isn't one the menu offers.
pub fn signal_name(sig: Signal) -> &'static str {
    SIGNALS
        .iter()
        .find(|(_, _, s)| *s == sig)
        .map(|(name, _, _)| *name)
        .unwrap_or("signal")
}

/// Platform signal number for a signal, or `None` if we don't deliver it.
fn raw_signal(sig: Signal) -> Option<i32> {
    SIGNALS
        .iter()
        .find(|(_, _, s)| *s == sig)
        .map(|(_, num, _)| *num)
}

/// Outcome of attempting to deliver a signal to a process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalOutcome {
    /// The signal was delivered.
    Delivered,
    /// kill(2) failed while the process is still alive — typically EPERM.
    NotPermitted,
    /// The signal isn't supported on this platform.
    Unsupported,
    /// The process no longer exists.
    Gone,
}

/// Outcome of attempting to change a process's nice value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PriorityOutcome {
    /// The nice value was set.
    Applied,
    /// The caller lacks privilege (raising priority, or another user's process).
    NotPermitted,
    /// The process no longer exists.
    Gone,
}

fn pct(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64 * 100.0) as f32
    }
}

/// Read the first battery from `/sys/class/power_supply`, if any.
fn read_battery() -> Option<Battery> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("BAT") {
            continue;
        }
        let capacity = std::fs::read_to_string(path.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());
        if let Some(percent) = capacity {
            let status = std::fs::read_to_string(path.join("status"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "Unknown".to_string());
            return Some(Battery { percent, status });
        }
    }
    None
}

fn status_label(status: ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Run => "Running",
        ProcessStatus::Sleep => "Sleeping",
        ProcessStatus::Idle => "Idle",
        ProcessStatus::Stop => "Stopped",
        ProcessStatus::Zombie => "Zombie",
        ProcessStatus::Tracing => "Tracing",
        ProcessStatus::Dead => "Dead",
        ProcessStatus::Wakekill => "Wakekill",
        ProcessStatus::Waking => "Waking",
        ProcessStatus::Parked => "Parked",
        ProcessStatus::LockBlocked => "Lock-blocked",
        ProcessStatus::UninterruptibleDiskSleep => "Disk-sleep",
        ProcessStatus::Suspended => "Suspended",
        ProcessStatus::Unknown(_) => "Unknown",
    }
}

fn status_char(status: ProcessStatus) -> char {
    match status {
        ProcessStatus::Run => 'R',
        ProcessStatus::Sleep => 'S',
        ProcessStatus::Idle => 'I',
        ProcessStatus::Stop => 'T',
        ProcessStatus::Zombie => 'Z',
        ProcessStatus::Tracing => 't',
        ProcessStatus::Dead => 'X',
        ProcessStatus::Wakekill => 'K',
        ProcessStatus::Waking => 'W',
        ProcessStatus::Parked => 'P',
        ProcessStatus::LockBlocked => 'L',
        ProcessStatus::UninterruptibleDiskSleep => 'D',
        ProcessStatus::Suspended => 'U',
        ProcessStatus::Unknown(_) => '?',
    }
}
