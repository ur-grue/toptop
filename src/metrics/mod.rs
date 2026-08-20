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
pub mod cgroup;
pub mod gpu;
pub mod infer;
pub mod netconn;

use crate::history::History;
use gpu::{Gpu, GpuMonitor, GpuProc};
use infer::{InferenceMonitor, Target};
pub use infer::{Percentiles, ServerStats};
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
#[derive(Clone, Debug, Default)]
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
    /// GPU memory held by this process (bytes), joined from `gpu_procs`; 0 when
    /// the process holds no VRAM or no GPU is present.
    pub gpu_mem: u64,
    /// Indentation depth when rendered as a tree (0 when flat).
    pub depth: usize,
    /// Container or Kubernetes pod this process belongs to, resolved only
    /// while the group-by-container view is on (see
    /// `Collector::resolve_containers`).
    pub container: Option<String>,
}

/// A temperature sensor reading.
#[derive(Clone, Debug)]
pub struct SensorInfo {
    pub label: String,
    pub temp: f32,
    pub high: Option<f32>,
    pub critical: Option<f32>,
}

/// Battery state, read from `/sys/class/power_supply` when present.
#[derive(Clone, Debug, PartialEq)]
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
    /// Whether to resolve each process's container. Off by default: it is a
    /// `/proc/<pid>/cgroup` read per process, which is exactly the sort of
    /// per-row syscall the hot-path guard exists to catch.
    pub resolve_containers: bool,
    /// Cache of resolved container labels. A process cannot change cgroup
    /// during its lifetime, so this is exact rather than a staleness tradeoff;
    /// entries are dropped when the PID goes away.
    container_cache: std::collections::HashMap<u32, Option<String>>,

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
    /// Per-GPU compute and memory-bandwidth history, indexed like `gpus`.
    /// Token generation is bandwidth-bound once the model is resident, so the
    /// *divergence* between these two lines over time is the single most
    /// diagnostic picture toptop can draw.
    pub gpu_history: Vec<GpuHistory>,
    /// Auto-discovered local inference servers (tokens/sec, KV cache, …).
    pub servers: Vec<ServerStats>,
    /// Per-server time series (keyed by pid+port) feeding the AI-view
    /// sparklines; pruned when a server disappears.
    pub server_history: std::collections::HashMap<(u32, u16), ServerHistory>,
    pub battery: Option<Battery>,
    /// cgroup v2 limits, when this process runs in a limited cgroup. Present
    /// means the host's CPU count and memory total do not describe what this
    /// process is allowed to use.
    pub cgroup: Option<cgroup::Cgroup>,
    pub uptime: u64,
}

/// Compute and memory-bandwidth utilization trends for one GPU.
#[derive(Clone, Debug)]
pub struct GpuHistory {
    /// SM/compute utilization %, 0–100.
    pub compute: History,
    /// Memory-bandwidth utilization %, 0–100.
    pub bandwidth: History,
    /// VRAM used %, 0–100 — the spill story over time.
    pub vram: History,
}

impl GpuHistory {
    fn new(capacity: usize) -> Self {
        Self {
            compute: History::new(capacity),
            bandwidth: History::new(capacity),
            vram: History::new(capacity),
        }
    }
}

/// Tokens/sec and KV-cache trends for one inference server.
#[derive(Clone, Debug)]
pub struct ServerHistory {
    pub tps: History,
    pub kv: History,
}

impl Collector {
    /// Build a collector and perform an initial population pass.
    pub fn new(history_len: usize) -> Self {
        Self::with_targets(history_len, Vec::new())
    }

    /// Build a collector that also scrapes manually configured inference
    /// servers (`--llm-server`) alongside auto-discovered ones.
    pub fn with_targets(history_len: usize, targets: Vec<Target>) -> Self {
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
            gpu_history: Vec::new(),
            cgroup: cgroup::current(),
            resolve_containers: false,
            container_cache: std::collections::HashMap::new(),
            infer_monitor: InferenceMonitor::with_targets(targets),
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
            server_history: std::collections::HashMap::new(),
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
        // Join per-process VRAM back onto the process list (summing across GPUs)
        // so the table can show and sort by it. Skipped entirely without a GPU.
        if !self.gpu_procs.is_empty() {
            let mut vram: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
            for gp in &self.gpu_procs {
                *vram.entry(gp.pid).or_insert(0) += gp.used_mem;
            }
            for p in &mut self.procs {
                p.gpu_mem = vram.get(&p.pid).copied().unwrap_or(0);
            }
        }
        // Limits change when a container is resized, and the throttling
        // counters move every tick — both are a handful of small reads.
        self.cgroup = cgroup::current();
        self.update_gpu_history_with(false);
        self.servers = self.infer_monitor.snapshot();
        self.update_server_history();
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

    /// Append this tick's tokens/sec and KV% to each live server's history,
    /// dropping the histories of servers that vanished.
    /// Keep one history per GPU, resizing when GPUs appear or disappear.
    /// A GPU that stops reporting utilization pushes 0 rather than nothing, so
    /// the time axis stays honest — a gap would silently compress the graph.
    /// Push this tick's per-GPU samples. `replace` overwrites the newest
    /// sample instead of appending — for the demo overlay, which supersedes a
    /// reading `refresh` already took this tick.
    pub(crate) fn update_gpu_history_with(&mut self, replace: bool) {
        if self.gpu_history.len() != self.gpus.len() {
            self.gpu_history = (0..self.gpus.len())
                .map(|_| GpuHistory::new(self.history_len))
                .collect();
        }
        for (g, h) in self.gpus.iter().zip(self.gpu_history.iter_mut()) {
            let compute = if g.has_util { g.util_pct as f64 } else { 0.0 };
            let bandwidth = if g.has_mem_util {
                g.mem_util as f64
            } else {
                0.0
            };
            let vram = g.mem_pct() as f64;
            if replace {
                h.compute.replace_last(compute);
                h.bandwidth.replace_last(bandwidth);
                h.vram.replace_last(vram);
            } else {
                h.compute.push(compute);
                h.bandwidth.push(bandwidth);
                h.vram.push(vram);
            }
        }
    }

    fn update_server_history(&mut self) {
        let live: std::collections::HashSet<(u32, u16)> =
            self.servers.iter().map(|s| (s.pid, s.port)).collect();
        self.server_history.retain(|k, _| live.contains(k));
        for sv in &self.servers {
            let h = self
                .server_history
                .entry((sv.pid, sv.port))
                .or_insert_with(|| ServerHistory {
                    tps: History::new(self.history_len),
                    kv: History::new(self.history_len),
                });
            h.tps.push(sv.gen_tps.unwrap_or(0.0));
            h.kv.push(sv.kv_pct.unwrap_or(0.0));
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
        // One physical filesystem can be mounted several times — macOS firmlinks
        // (`/` and `/System/Volumes/Data`), Linux bind mounts, btrfs
        // subvolumes. Listing it twice wastes a panel row, and *summing* its
        // I/O twice inflates the read/write rates, so both are deduplicated on
        // the device identity.
        let mut seen: std::collections::HashSet<(String, u64)> = std::collections::HashSet::new();
        for disk in self.disks.list() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            let name = disk.name().to_string_lossy().to_string();
            if !seen.insert((name.clone(), total)) {
                continue;
            }
            self.disk_list.push(DiskInfo {
                name,
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
        // Shortest mount first within a device, then alphabetically: `/` should
        // outrank `/System/Volumes/Data` when both survive dedup.
        self.disk_list.sort_by(|a, b| {
            a.mount
                .len()
                .cmp(&b.mount.len())
                .then(a.mount.cmp(&b.mount))
        });

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
                gpu_mem: 0,
                depth: 0,
                container: None,
            });
        }
        // Container labels, only when the grouping view asked for them. The
        // cache means one /proc read per process ever, not per tick.
        if self.resolve_containers {
            self.container_cache
                .retain(|pid, _| io_now.contains_key(pid));
            for p in &mut procs {
                let label = self
                    .container_cache
                    .entry(p.pid)
                    .or_insert_with(|| cgroup::container_of(p.pid));
                p.container = label.clone();
            }
        } else if !self.container_cache.is_empty() {
            self.container_cache.clear();
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
    #[cfg(unix)]
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
    #[cfg(unix)]
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

    /// Everything the detail overlay needs beyond the process row, fetched on
    /// demand for one PID rather than cached for every row of the table.
    ///
    /// Each part degrades independently: environment comes from sysinfo (all
    /// platforms), open files from `/proc/<pid>/fd` (Linux only), and sockets
    /// are the global connection scan narrowed to this PID (Linux only). An
    /// unreadable part yields an empty list, never an error — you routinely
    /// lack permission for other users' processes.
    pub fn proc_detail(&mut self, pid: u32) -> ProcDetail {
        ProcDetail {
            env: self.proc_env(pid),
            open_files: proc_open_files(pid),
            connections: self
                .connections()
                .into_iter()
                .filter(|c| c.pid == Some(pid))
                .collect(),
        }
    }

    /// Read one process's environment, refreshing just that PID for it —
    /// environments are large and change rarely, so they are deliberately not
    /// part of the per-tick refresh.
    fn proc_env(&mut self, pid: u32) -> Vec<(String, String)> {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            false,
            ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
        );
        let Some(proc) = self.sys.process(Pid::from_u32(pid)) else {
            return Vec::new();
        };
        let mut env: Vec<(String, String)> = proc
            .environ()
            .iter()
            .filter_map(|e| {
                let e = e.to_string_lossy();
                let (k, v) = e.split_once('=')?;
                Some((k.to_string(), v.to_string()))
            })
            .collect();
        env.sort_by(|a, b| a.0.cmp(&b.0));
        env
    }

    /// Windows has no signals: sysinfo maps what it can onto `TerminateProcess`
    /// and reports the rest as unsupported, which the UI already renders.
    #[cfg(windows)]
    pub fn signal_process(&self, pid: u32, signal: Signal) -> SignalOutcome {
        let Some(proc) = self.sys.process(Pid::from_u32(pid)) else {
            return SignalOutcome::Gone;
        };
        match proc.kill_with(signal) {
            Some(true) => SignalOutcome::Delivered,
            Some(false) => SignalOutcome::NotPermitted,
            None => SignalOutcome::Unsupported,
        }
    }

    /// Windows scheduling priorities are process-class based, not nice values;
    /// until that is mapped properly the renice menu reports it as unsupported
    /// rather than silently doing nothing.
    #[cfg(windows)]
    pub fn set_priority(&self, _pid: u32, _nice: i32) -> PriorityOutcome {
        PriorityOutcome::Unsupported
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
/// platform signal number (straight from libc, `0` where the platform has no
/// number), and the sysinfo `Signal`. Single source of truth — the menu,
/// status line, and delivery all derive from this, so a new signal is added in
/// exactly one place.
#[cfg(unix)]
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

/// Windows only offers process termination — everything else would be listed
/// in the menu just to fail, so it isn't listed at all.
#[cfg(windows)]
pub const SIGNALS: &[(&str, i32, Signal)] = &[("Terminate", 0, Signal::Kill)];

/// Human label for a signal, or "signal" if it isn't one the menu offers.
pub fn signal_name(sig: Signal) -> &'static str {
    SIGNALS
        .iter()
        .find(|(_, _, s)| *s == sig)
        .map(|(name, _, _)| *name)
        .unwrap_or("signal")
}

/// Platform signal number for a signal, or `None` if we don't deliver it.
#[cfg(unix)]
fn raw_signal(sig: Signal) -> Option<i32> {
    SIGNALS
        .iter()
        .find(|(_, _, s)| *s == sig)
        .map(|(_, num, _)| *num)
}

/// One entry of `/proc/<pid>/fd`: the descriptor number and what it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFile {
    pub fd: u32,
    /// Link target: a path, or a pseudo-target like `socket:[12345]`.
    pub target: String,
}

impl OpenFile {
    /// Whether this descriptor is a socket rather than a file on disk — those
    /// are listed under connections instead, with their addresses resolved.
    pub fn is_socket(&self) -> bool {
        self.target.starts_with("socket:")
    }

    /// Whether this is a pipe, event fd or similar kernel pseudo-file.
    pub fn is_pseudo(&self) -> bool {
        self.target.starts_with("pipe:")
            || self.target.starts_with("anon_inode:")
            || self.is_socket()
    }
}

/// On-demand detail for one process, backing the detail overlay.
#[derive(Clone, Debug, Default)]
pub struct ProcDetail {
    /// Environment, sorted by key.
    pub env: Vec<(String, String)>,
    /// Open file descriptors, sorted by fd.
    pub open_files: Vec<OpenFile>,
    /// This process's network connections.
    pub connections: Vec<Connection>,
}

/// Read `/proc/<pid>/fd` — Linux only; anywhere else this yields nothing.
fn proc_open_files(pid: u32) -> Vec<OpenFile> {
    let Ok(dir) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    let mut out: Vec<OpenFile> = dir
        .flatten()
        .filter_map(|entry| {
            let fd: u32 = entry.file_name().to_str()?.parse().ok()?;
            // A descriptor can close between listing and reading it; skip it
            // rather than showing a phantom entry.
            let target = std::fs::read_link(entry.path()).ok()?;
            Some(OpenFile {
                fd,
                target: target.to_string_lossy().to_string(),
            })
        })
        .collect();
    out.sort_by_key(|f| f.fd);
    out
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
    /// This platform has no nice values (Windows).
    Unsupported,
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

/// Read the first battery from `/sys/class/power_supply`, if any. Linux only;
/// other platforms simply find no directory and report no battery.
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

#[cfg(test)]
mod tests {
    use super::{OpenFile, ProcDetail};

    #[test]
    fn open_file_classification() {
        let sock = OpenFile {
            fd: 7,
            target: "socket:[12345]".into(),
        };
        assert!(sock.is_socket() && sock.is_pseudo());
        let pipe = OpenFile {
            fd: 3,
            target: "pipe:[999]".into(),
        };
        assert!(!pipe.is_socket() && pipe.is_pseudo());
        let file = OpenFile {
            fd: 4,
            target: "/var/log/app.log".into(),
        };
        assert!(!file.is_socket() && !file.is_pseudo());
    }

    #[test]
    fn proc_detail_defaults_are_empty() {
        let d = ProcDetail::default();
        assert!(d.env.is_empty() && d.open_files.is_empty() && d.connections.is_empty());
    }

    use super::*;

    fn fake_gpu(util: f32, mem_util: f32, used: u64, total: u64) -> gpu::Gpu {
        gpu::Gpu {
            name: "TestGPU".into(),
            util_pct: util,
            has_util: true,
            mem_util,
            has_mem_util: true,
            mem_used: used,
            mem_total: total,
            temp: 60.0,
            power: 200.0,
            power_limit: 400.0,
            throttled: false,
        }
    }

    #[test]
    fn gpu_history_tracks_compute_bandwidth_and_vram() {
        let mut c = Collector::new(8);
        c.gpus = vec![fake_gpu(90.0, 30.0, 50, 100)];
        c.update_gpu_history_with(false);
        c.gpus = vec![fake_gpu(20.0, 95.0, 90, 100)];
        c.update_gpu_history_with(false);

        assert_eq!(c.gpu_history.len(), 1);
        let h = &c.gpu_history[0];
        assert_eq!(h.compute.tail(2), vec![90.0, 20.0]);
        // The divergence this graph exists to show: compute fell, bandwidth
        // pinned — memory-bound, and a faster GPU core would not help.
        assert_eq!(h.bandwidth.tail(2), vec![30.0, 95.0]);
        assert_eq!(h.vram.tail(2), vec![50.0, 90.0]);
    }

    #[test]
    fn gpu_history_resizes_when_gpus_come_and_go() {
        let mut c = Collector::new(8);
        c.gpus = vec![fake_gpu(50.0, 50.0, 1, 2), fake_gpu(50.0, 50.0, 1, 2)];
        c.update_gpu_history_with(false);
        assert_eq!(c.gpu_history.len(), 2);
        // A GPU disappearing (driver reload, container restart) resizes rather
        // than indexing past the end.
        c.gpus.pop();
        c.update_gpu_history_with(false);
        assert_eq!(c.gpu_history.len(), 1);
        c.gpus.clear();
        c.update_gpu_history_with(false);
        assert!(c.gpu_history.is_empty());
    }

    #[test]
    fn gpu_without_utilization_still_advances_the_time_axis() {
        let mut c = Collector::new(8);
        let mut g = fake_gpu(0.0, 0.0, 10, 100);
        g.has_util = false;
        g.has_mem_util = false;
        c.gpus = vec![g];
        c.update_gpu_history_with(false);
        let before = c.gpu_history[0].compute.len();
        c.update_gpu_history_with(false);
        // Pushing 0 rather than nothing keeps the time axis honest — a gap
        // would silently compress the graph.
        assert_eq!(c.gpu_history[0].compute.len(), before + 1);
        assert_eq!(c.gpu_history[0].compute.last(), 0.0);
    }

    #[test]
    fn server_history_tracks_and_prunes() {
        let mut c = Collector::new(16);
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            pid: 1,
            port: 8000,
            gen_tps: Some(10.0),
            kv_pct: Some(50.0),
            ..Default::default()
        }];
        c.update_server_history();
        c.servers[0].gen_tps = Some(20.0);
        c.servers[0].kv_pct = None; // metric momentarily absent → recorded as 0
        c.update_server_history();

        let h = c.server_history.get(&(1, 8000)).expect("history exists");
        assert_eq!(h.tps.tail(2), vec![10.0, 20.0]);
        assert_eq!(h.kv.tail(2), vec![50.0, 0.0]);

        // The server disappears → its history is pruned.
        c.servers.clear();
        c.update_server_history();
        assert!(c.server_history.is_empty());
    }
}
