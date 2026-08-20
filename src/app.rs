//! Application state and input handling — the controller tying metrics to UI.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use sysinfo::Signal;

use crate::alerts::{self, Alert, AlertConfig, AlertTracker};
use crate::config::Config;
use crate::keys::{Action, KeyMap};
use crate::metrics::{
    signal_name, Collector, Connection, PriorityOutcome, ProcDetail, ProcInfo, SignalOutcome,
    SIGNALS,
};
use crate::notify::Notifier;
use crate::record::{Recorder, Replay};
use crate::theme::{self, Theme};

/// Process table sort columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    Cpu,
    Mem,
    Pid,
    Name,
    User,
    Time,
    Io,
    Gpu,
}

impl SortField {
    pub fn label(self) -> &'static str {
        match self {
            SortField::Cpu => "CPU%",
            SortField::Mem => "MEM%",
            SortField::Pid => "PID",
            SortField::Name => "NAME",
            SortField::User => "USER",
            SortField::Time => "TIME",
            SortField::Io => "DISK",
            SortField::Gpu => "VRAM",
        }
    }

    /// Cycle to the next sort field.
    pub fn next(self) -> Self {
        match self {
            SortField::Cpu => SortField::Mem,
            SortField::Mem => SortField::Pid,
            SortField::Pid => SortField::Name,
            SortField::Name => SortField::User,
            SortField::User => SortField::Time,
            SortField::Time => SortField::Io,
            SortField::Io => SortField::Gpu,
            SortField::Gpu => SortField::Cpu,
        }
    }
}

/// A process-table column.
///
/// The configured list drives the header, the row cells and the click-to-sort
/// mapping in `ui::render_procs` — see the `columns` config key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcColumn {
    Pid,
    User,
    Cpu,
    MemPct,
    Mem,
    Disk,
    Vram,
    Time,
    State,
    Command,
}

/// The column set shown when the config says nothing.
pub const DEFAULT_COLUMNS: &[ProcColumn] = &[
    ProcColumn::Pid,
    ProcColumn::User,
    ProcColumn::Cpu,
    ProcColumn::MemPct,
    ProcColumn::Mem,
    ProcColumn::Disk,
    ProcColumn::Vram,
    ProcColumn::Time,
    ProcColumn::State,
    ProcColumn::Command,
];

impl ProcColumn {
    /// Config name, also used when the column set is persisted.
    pub fn name(self) -> &'static str {
        match self {
            ProcColumn::Pid => "pid",
            ProcColumn::User => "user",
            ProcColumn::Cpu => "cpu",
            ProcColumn::MemPct => "mem%",
            ProcColumn::Mem => "mem",
            ProcColumn::Disk => "disk",
            ProcColumn::Vram => "vram",
            ProcColumn::Time => "time",
            ProcColumn::State => "state",
            ProcColumn::Command => "command",
        }
    }

    /// Header label.
    pub fn header(self) -> &'static str {
        match self {
            ProcColumn::Pid => "PID",
            ProcColumn::User => "USER",
            ProcColumn::Cpu => "CPU%",
            ProcColumn::MemPct => "MEM%",
            ProcColumn::Mem => "MEM",
            ProcColumn::Disk => "DISK",
            ProcColumn::Vram => "VRAM",
            ProcColumn::Time => "TIME",
            ProcColumn::State => "S",
            ProcColumn::Command => "COMMAND",
        }
    }

    /// Fixed width in cells, or `None` for the column that takes the rest.
    pub fn width(self) -> Option<u16> {
        match self {
            ProcColumn::Pid => Some(7),
            ProcColumn::User => Some(9),
            ProcColumn::Cpu | ProcColumn::MemPct => Some(5),
            ProcColumn::Mem | ProcColumn::Vram | ProcColumn::Time => Some(7),
            ProcColumn::Disk => Some(8),
            ProcColumn::State => Some(1),
            ProcColumn::Command => None,
        }
    }

    /// Sort field a click on this header selects, if the column is sortable.
    pub fn sort_field(self) -> Option<SortField> {
        match self {
            ProcColumn::Pid => Some(SortField::Pid),
            ProcColumn::User => Some(SortField::User),
            ProcColumn::Cpu => Some(SortField::Cpu),
            ProcColumn::MemPct | ProcColumn::Mem => Some(SortField::Mem),
            ProcColumn::Disk => Some(SortField::Io),
            ProcColumn::Vram => Some(SortField::Gpu),
            ProcColumn::Time => Some(SortField::Time),
            ProcColumn::Command => Some(SortField::Name),
            ProcColumn::State => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "pid" => Some(ProcColumn::Pid),
            "user" => Some(ProcColumn::User),
            "cpu" | "cpu%" => Some(ProcColumn::Cpu),
            "mem%" | "mempct" => Some(ProcColumn::MemPct),
            "mem" | "rss" => Some(ProcColumn::Mem),
            "disk" | "io" => Some(ProcColumn::Disk),
            "vram" | "gpu" => Some(ProcColumn::Vram),
            "time" => Some(ProcColumn::Time),
            "state" | "s" => Some(ProcColumn::State),
            "command" | "cmd" => Some(ProcColumn::Command),
            _ => None,
        }
    }
}

/// Sort key for the combined per-process I/O rate.
fn io_rate(p: &ProcInfo) -> f64 {
    p.io_read_rate + p.io_write_rate
}

/// Top-section layout presets, cycled with `L` and persisted to config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPreset {
    /// CPU · mem/sensors · net/disk, then the process table (the default).
    Full,
    /// A single full-width CPU panel on top, then the process table.
    Cpu,
    /// No top panels — the process table fills the whole body.
    Process,
}

impl LayoutPreset {
    pub fn label(self) -> &'static str {
        match self {
            LayoutPreset::Full => "full",
            LayoutPreset::Cpu => "cpu",
            LayoutPreset::Process => "process",
        }
    }

    pub fn next(self) -> Self {
        match self {
            LayoutPreset::Full => LayoutPreset::Cpu,
            LayoutPreset::Cpu => LayoutPreset::Process,
            LayoutPreset::Process => LayoutPreset::Full,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "full" => Some(LayoutPreset::Full),
            "cpu" => Some(LayoutPreset::Cpu),
            "process" | "proc" => Some(LayoutPreset::Process),
            _ => None,
        }
    }
}

/// Nice-value presets offered by the renice menu (label, value). Lower is
/// higher priority; negative values need privilege.
pub const NICE_LEVELS: &[(&str, i32)] = &[
    ("-20  highest", -20),
    ("-10", -10),
    ("-5", -5),
    ("0  normal", 0),
    ("5", 5),
    ("10", 10),
    ("19  lowest", 19),
];

/// A pending destructive action awaiting confirmation.
#[derive(Clone)]
pub struct PendingKill {
    pub pid: u32,
    pub name: String,
    pub signal: Signal,
}

/// The whole runtime state.
pub struct App {
    pub collector: Collector,
    pub theme_idx: usize,
    pub tick: Duration,
    pub should_quit: bool,
    pub paused: bool,
    pub show_help: bool,
    pub tree: bool,
    pub per_core: bool,
    pub layout: LayoutPreset,

    pub sort: SortField,
    pub sort_desc: bool,
    /// Process-table columns, in display order (from the config).
    pub columns: Vec<ProcColumn>,
    /// Key → action bindings (from the config).
    pub keys: KeyMap,
    pub filter: String,
    pub filter_mode: bool,

    /// The currently displayed, sorted/filtered/tree-ordered process rows.
    pub proc_view: Vec<ProcInfo>,
    pub selected_pid: Option<u32>,
    pub proc_offset: usize,
    /// Inner area of the process table, captured at render time for mouse hits.
    pub proc_area: Rect,
    /// Visible row capacity of the process table, captured at render time.
    pub proc_rows: usize,

    pub pending_kill: Option<PendingKill>,
    /// When `Some`, the signal menu is open with the given highlighted index.
    pub signal_menu: Option<usize>,
    /// When `Some`, the renice menu is open with the given highlighted index.
    pub renice_menu: Option<usize>,
    /// Whether the process detail overlay is shown for the selection.
    pub show_detail: bool,
    /// Open files, environment and sockets for the selected process, fetched
    /// only while the overlay is open.
    pub detail: Option<ProcDetail>,
    /// Whether the AI/LLM GPU view is shown.
    pub show_ai: bool,
    /// Whether the network-connections view is shown.
    pub show_conn: bool,
    /// Latest connection snapshot (refreshed while the view is open).
    pub connections: Vec<Connection>,
    /// Scroll offset into the connections list.
    pub conn_offset: usize,
    /// Visible row capacity of the connections view, captured at render time.
    pub conn_rows: usize,
    /// Currently-firing threshold alerts (recomputed each tick).
    pub alerts: Vec<Alert>,
    /// Turns each tick's alert set into debounced fire/resolve transitions.
    pub tracker: AlertTracker,
    /// Where those transitions are delivered (command and/or HTTP sinks).
    pub notify: Notifier,
    /// Whether the alert-history timeline overlay is shown.
    pub show_alert_history: bool,
    /// Flap-suppression window, kept for the config round-trip.
    flap_window_secs: u64,
    /// `--record <file>`: append one JSON snapshot per tick.
    pub recorder: Option<Recorder>,
    /// `--replay <file>`: feed recorded snapshots instead of live metrics.
    pub replay: Option<Replay>,
    /// `--demo` mode: overlay synthesized GPU + inference data each tick.
    pub demo: bool,
    demo_tick: u64,
    pub alert_cfg: AlertConfig,
    /// Manual inference-server targets, kept for the config round-trip.
    llm_servers: Vec<crate::metrics::infer::Target>,
    pub status: Option<(String, Instant)>,
}

impl App {
    pub fn new(cfg: &Config) -> Self {
        let history_len = 256;
        let collector = Collector::with_targets(history_len, cfg.llm_servers.clone());
        let mut app = Self {
            collector,
            theme_idx: cfg.theme_idx.min(theme::themes().len() - 1),
            tick: Duration::from_millis(cfg.tick_ms),
            should_quit: false,
            paused: false,
            show_help: false,
            tree: cfg.tree,
            per_core: cfg.per_core,
            layout: cfg.layout,
            sort: SortField::Cpu,
            sort_desc: true,
            columns: cfg.columns.clone(),
            keys: cfg.keys.clone(),
            filter: String::new(),
            filter_mode: false,
            proc_view: Vec::new(),
            selected_pid: None,
            proc_offset: 0,
            proc_area: Rect::default(),
            proc_rows: 0,
            pending_kill: None,
            signal_menu: None,
            renice_menu: None,
            show_detail: false,
            detail: None,
            show_ai: false,
            show_conn: false,
            connections: Vec::new(),
            conn_offset: 0,
            conn_rows: 0,
            alerts: Vec::new(),
            tracker: AlertTracker::new(Duration::from_secs(cfg.flap_window_secs)),
            notify: cfg.notify.clone(),
            show_alert_history: false,
            flap_window_secs: cfg.flap_window_secs,
            recorder: None,
            replay: None,
            demo: false,
            demo_tick: 0,
            alert_cfg: cfg.alerts.clone(),
            llm_servers: cfg.llm_servers.clone(),
            status: None,
        };
        app.rebuild_proc_view();
        if app.selected_pid.is_none() {
            app.selected_pid = app.proc_view.first().map(|p| p.pid);
        }
        app
    }

    pub fn theme(&self) -> &'static Theme {
        &theme::themes()[self.theme_idx]
    }

    /// Snapshot of the config for persistence on exit.
    pub fn config(&self) -> Config {
        Config {
            tick_ms: self.tick.as_millis() as u64,
            theme_idx: self.theme_idx,
            tree: self.tree,
            per_core: self.per_core,
            layout: self.layout,
            columns: self.columns.clone(),
            keys: self.keys.clone(),
            warnings: Vec::new(),
            alerts: self.alert_cfg.clone(),
            notify: self.notify.clone(),
            flap_window_secs: self.flap_window_secs,
            llm_servers: self.llm_servers.clone(),
        }
    }

    /// Pull fresh metrics (unless paused) and recompute the process view.
    pub fn on_tick(&mut self) {
        // Freeze the process list while a kill confirmation is pending so the
        // target can't disappear from the table between prompt and confirm.
        if let Some(replay) = &mut self.replay {
            // Replay drives the collector instead of the live system: no
            // refresh, no demo overlay, no recording.
            if !self.paused {
                replay.tick();
            }
            let replay = self.replay.as_ref().expect("just borrowed");
            replay.apply(&mut self.collector);
        } else if !self.paused && self.pending_kill.is_none() {
            self.collector.refresh();
            if self.demo {
                self.demo_tick += 1;
                crate::demo::apply(&mut self.collector, self.demo_tick);
            }
            if let Some(rec) = &mut self.recorder {
                if let Err(e) = rec.record(&self.collector) {
                    // Stop rather than silently producing a truncated
                    // recording, and say so where the user will see it.
                    self.recorder = None;
                    self.set_status(format!("Recording stopped: {e}"));
                }
            }
        }
        self.rebuild_proc_view();
        self.alerts = alerts::evaluate(&self.collector, &self.alert_cfg);
        // Fire/resolve transitions drive notifications and the timeline; the
        // alert *set* alone can't distinguish "still firing" from "just fired".
        let transitions = self.tracker.update(&self.alerts, Instant::now());
        self.notify.dispatch_all(&transitions);
        if self.show_conn {
            self.refresh_connections();
        }
        self.refresh_detail();
        self.expire_status();
    }

    /// Fetch the selected process's detail while the overlay is open, and drop
    /// it as soon as it closes — open files and environment are expensive
    /// enough that they should never be collected for a closed overlay.
    fn refresh_detail(&mut self) {
        if !self.show_detail {
            self.detail = None;
            return;
        }
        match self.selected_pid {
            Some(pid) => self.detail = Some(self.collector.proc_detail(pid)),
            None => self.detail = None,
        }
    }

    /// Re-snapshot network connections and clamp the scroll offset.
    fn refresh_connections(&mut self) {
        self.connections = self.collector.connections();
        let max = self.connections.len().saturating_sub(self.conn_rows.max(1));
        self.conn_offset = self.conn_offset.min(max);
    }

    fn scroll_conn(&mut self, delta: isize) {
        let max = self.connections.len().saturating_sub(self.conn_rows.max(1)) as isize;
        let next = (self.conn_offset as isize + delta).clamp(0, max.max(0));
        self.conn_offset = next as usize;
    }

    fn expire_status(&mut self) {
        if let Some((_, when)) = &self.status {
            if when.elapsed() > Duration::from_secs(4) {
                self.status = None;
            }
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// Rebuild [`Self::proc_view`] from the collector applying filter, tree, sort.
    pub fn rebuild_proc_view(&mut self) {
        let mut rows: Vec<ProcInfo> = self.collector.procs.clone();

        if !self.filter.is_empty() {
            let needle = self.filter.to_ascii_lowercase();
            rows.retain(|p| {
                p.name.to_ascii_lowercase().contains(&needle)
                    || p.cmd.to_ascii_lowercase().contains(&needle)
                    || p.user.to_ascii_lowercase().contains(&needle)
                    || p.pid.to_string().contains(&needle)
            });
        }

        if self.tree && self.filter.is_empty() {
            rows = self.build_tree(rows);
        } else {
            self.sort_rows(&mut rows);
        }

        self.proc_view = rows;

        // Keep selection valid.
        if let Some(pid) = self.selected_pid {
            if !self.proc_view.iter().any(|p| p.pid == pid) {
                self.selected_pid = self.proc_view.first().map(|p| p.pid);
            }
        } else {
            self.selected_pid = self.proc_view.first().map(|p| p.pid);
        }
    }

    fn sort_rows(&self, rows: &mut [ProcInfo]) {
        rows.sort_by(|a, b| {
            let ord = match self.sort {
                SortField::Cpu => a
                    .cpu
                    .partial_cmp(&b.cpu)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortField::Mem => a.mem_bytes.cmp(&b.mem_bytes),
                SortField::Pid => a.pid.cmp(&b.pid),
                SortField::Name => a
                    .name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase()),
                SortField::User => a
                    .user
                    .to_ascii_lowercase()
                    .cmp(&b.user.to_ascii_lowercase()),
                SortField::Time => a.run_time.cmp(&b.run_time),
                SortField::Io => io_rate(a)
                    .partial_cmp(&io_rate(b))
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortField::Gpu => a.gpu_mem.cmp(&b.gpu_mem),
            };
            if self.sort_desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    /// Build a depth-annotated tree ordering (parents before children).
    fn build_tree(&self, rows: Vec<ProcInfo>) -> Vec<ProcInfo> {
        use std::collections::HashMap;
        let present: std::collections::HashSet<u32> = rows.iter().map(|p| p.pid).collect();
        let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut roots: Vec<usize> = Vec::new();
        for (idx, p) in rows.iter().enumerate() {
            match p.ppid {
                Some(ppid) if present.contains(&ppid) && ppid != p.pid => {
                    children.entry(ppid).or_default().push(idx);
                }
                _ => roots.push(idx),
            }
        }

        // Sort siblings (and roots) by the active sort key.
        let sort_idx = |list: &mut Vec<usize>| {
            list.sort_by(|&a, &b| {
                let (pa, pb) = (&rows[a], &rows[b]);
                let ord = match self.sort {
                    SortField::Cpu => pa
                        .cpu
                        .partial_cmp(&pb.cpu)
                        .unwrap_or(std::cmp::Ordering::Equal),
                    SortField::Mem => pa.mem_bytes.cmp(&pb.mem_bytes),
                    SortField::Pid => pa.pid.cmp(&pb.pid),
                    SortField::Name => pa
                        .name
                        .to_ascii_lowercase()
                        .cmp(&pb.name.to_ascii_lowercase()),
                    SortField::User => pa
                        .user
                        .to_ascii_lowercase()
                        .cmp(&pb.user.to_ascii_lowercase()),
                    SortField::Time => pa.run_time.cmp(&pb.run_time),
                    SortField::Io => io_rate(pa)
                        .partial_cmp(&io_rate(pb))
                        .unwrap_or(std::cmp::Ordering::Equal),
                    SortField::Gpu => pa.gpu_mem.cmp(&pb.gpu_mem),
                };
                if self.sort_desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
        };
        sort_idx(&mut roots);
        for v in children.values_mut() {
            sort_idx(v);
        }

        let mut out = Vec::with_capacity(rows.len());
        // Iterative DFS to avoid recursion limits on deep trees.
        let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&i| (i, 0usize)).collect();
        let mut visited = std::collections::HashSet::new();
        while let Some((idx, depth)) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            let mut row = rows[idx].clone();
            row.depth = depth;
            out.push(row);
            if let Some(kids) = children.get(&rows[idx].pid) {
                for &kid in kids.iter().rev() {
                    stack.push((kid, depth + 1));
                }
            }
        }
        // Append any orphans not reached (cycles), so nothing is silently dropped.
        for (idx, p) in rows.iter().enumerate() {
            if !visited.contains(&idx) {
                out.push(p.clone());
            }
        }
        out
    }

    fn selected_index(&self) -> Option<usize> {
        let pid = self.selected_pid?;
        self.proc_view.iter().position(|p| p.pid == pid)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.proc_view.is_empty() {
            return;
        }
        let cur = self.selected_index().unwrap_or(0) as isize;
        let max = self.proc_view.len() as isize - 1;
        let next = (cur + delta).clamp(0, max) as usize;
        self.selected_pid = Some(self.proc_view[next].pid);
    }

    fn select_first(&mut self) {
        self.selected_pid = self.proc_view.first().map(|p| p.pid);
    }

    fn select_last(&mut self) {
        self.selected_pid = self.proc_view.last().map(|p| p.pid);
    }

    fn cycle_theme(&mut self, forward: bool) {
        let n = theme::themes().len();
        self.theme_idx = if forward {
            (self.theme_idx + 1) % n
        } else {
            (self.theme_idx + n - 1) % n
        };
        self.set_status(format!("Theme: {}", self.theme().name));
    }

    fn adjust_tick(&mut self, faster: bool) {
        let ms = self.tick.as_millis() as u64;
        let new = if faster {
            ms.saturating_sub(250).max(250)
        } else {
            (ms + 250).min(10_000)
        };
        self.tick = Duration::from_millis(new);
        self.set_status(format!("Refresh: {} ms", new));
    }

    /// The currently selected process row, if any.
    pub fn selected_proc(&self) -> Option<&ProcInfo> {
        self.selected_index().map(|i| &self.proc_view[i])
    }

    fn request_kill(&mut self, signal: Signal) {
        let Some(idx) = self.selected_index() else {
            return;
        };
        let p = &self.proc_view[idx];
        self.pending_kill = Some(PendingKill {
            pid: p.pid,
            name: p.name.clone(),
            signal,
        });
    }

    /// Open the interactive signal menu for the selected process.
    fn open_signal_menu(&mut self) {
        if self.selected_index().is_some() {
            self.signal_menu = Some(0);
        }
    }

    /// Confirm the highlighted signal from the menu, routing it through the
    /// kill-confirmation prompt.
    fn choose_signal(&mut self) {
        if let Some(idx) = self.signal_menu.take() {
            // idx is always in range: it starts at 0 and every move clamps to
            // SIGNALS.len() - 1, so index directly.
            let signal = SIGNALS[idx].2;
            self.request_kill(signal);
        }
    }

    /// Open the renice menu for the selected process, highlighting "0 normal".
    fn open_renice_menu(&mut self) {
        if self.selected_index().is_some() {
            self.renice_menu = Some(NICE_LEVELS.iter().position(|&(_, n)| n == 0).unwrap_or(0));
        }
    }

    /// Apply the highlighted nice value to the selected process. Renice isn't
    /// destructive, so it applies directly (no confirmation) and reports the
    /// outcome in the status line.
    fn choose_nice(&mut self) {
        let Some(idx) = self.renice_menu.take() else {
            return;
        };
        let Some((pid, name)) = self.selected_proc().map(|p| (p.pid, p.name.clone())) else {
            return;
        };
        let nice = NICE_LEVELS[idx].1;
        let msg = match self.collector.set_priority(pid, nice) {
            PriorityOutcome::Applied => format!("Reniced {} ({}) to {}", name, pid, nice),
            PriorityOutcome::NotPermitted => format!(
                "Permission denied renicing {} ({}) — lowering nice needs root",
                name, pid
            ),
            PriorityOutcome::Unsupported => {
                format!(
                    "Renicing {} ({}) isn't supported on this platform",
                    name, pid
                )
            }
            PriorityOutcome::Gone => format!("{} ({}) already exited", name, pid),
        };
        self.set_status(msg);
    }

    fn confirm_kill(&mut self) {
        if let Some(pk) = self.pending_kill.take() {
            let sig = signal_name(pk.signal);
            let msg = match self.collector.signal_process(pk.pid, pk.signal) {
                SignalOutcome::Delivered => format!("Sent {} to {} ({})", sig, pk.name, pk.pid),
                SignalOutcome::NotPermitted => format!(
                    "Permission denied signalling {} ({}) — try running as root",
                    pk.name, pk.pid
                ),
                SignalOutcome::Unsupported => {
                    format!("{} is not supported on this platform", sig)
                }
                SignalOutcome::Gone => format!("{} ({}) already exited", pk.name, pk.pid),
            };
            self.set_status(msg);
        }
    }

    // ── Input ────────────────────────────────────────────────────────────────

    /// Handle a key event. Returns immediately for modal/filter states.
    pub fn on_key(&mut self, key: KeyEvent) {
        // Confirmation modal takes precedence.
        if self.pending_kill.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.confirm_kill(),
                _ => self.pending_kill = None,
            }
            return;
        }

        // Signal menu next.
        if let Some(idx) = self.signal_menu {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.signal_menu = Some(idx.saturating_sub(1));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.signal_menu = Some((idx + 1).min(SIGNALS.len() - 1));
                }
                KeyCode::Enter => self.choose_signal(),
                KeyCode::Esc | KeyCode::Char('q') => self.signal_menu = None,
                _ => {}
            }
            return;
        }

        // Renice menu next.
        if let Some(idx) = self.renice_menu {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.renice_menu = Some(idx.saturating_sub(1));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.renice_menu = Some((idx + 1).min(NICE_LEVELS.len() - 1));
                }
                KeyCode::Enter => self.choose_nice(),
                KeyCode::Esc | KeyCode::Char('q') => self.renice_menu = None,
                _ => {}
            }
            return;
        }

        // Filter text-entry mode.
        if self.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filter_mode = false;
                    self.rebuild_proc_view();
                }
                KeyCode::Enter => {
                    self.filter_mode = false;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.rebuild_proc_view();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.rebuild_proc_view();
                }
                _ => {}
            }
            return;
        }

        // Help overlay: any key closes it.
        if self.show_help {
            self.show_help = false;
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // Connections view captures navigation while open.
        if self.show_conn {
            if key.code == KeyCode::Esc {
                self.show_conn = false;
                return;
            }
            match self.keys.action(key) {
                Some(Action::Quit) => self.should_quit = true,
                Some(Action::Connections) => self.show_conn = false,
                Some(Action::Up) => self.scroll_conn(-1),
                Some(Action::Down) => self.scroll_conn(1),
                Some(Action::PageUp) => self.scroll_conn(-(self.conn_rows.max(1) as isize)),
                Some(Action::PageDown) => self.scroll_conn(self.conn_rows.max(1) as isize),
                Some(Action::First) => self.conn_offset = 0,
                Some(Action::Last) => {
                    self.conn_offset = self.connections.len().saturating_sub(self.conn_rows.max(1))
                }
                Some(Action::ThemeNext) => self.cycle_theme(true),
                Some(Action::ThemePrev) => self.cycle_theme(false),
                _ => {}
            }
            return;
        }

        // Esc peels back overlays in order, then clears a filter, then quits.
        // Deliberately not rebindable — it is the way out of every overlay.
        if key.code == KeyCode::Esc {
            if self.show_alert_history {
                self.show_alert_history = false;
            } else if self.show_ai {
                self.show_ai = false;
            } else if self.show_detail {
                self.show_detail = false;
            } else if !self.filter.is_empty() {
                self.filter.clear();
                self.rebuild_proc_view();
            } else {
                self.should_quit = true;
            }
            return;
        }

        // Scrubbing a recording: ←/→ step, and stepping implies pausing or
        // playback would immediately undo the step. Handled before the keymap
        // because neither arrow is a bindable action.
        if self.replay.is_some() && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            let delta = if key.code == KeyCode::Right { 1 } else { -1 };
            self.paused = true;
            if let Some(r) = &mut self.replay {
                r.step(delta);
                let (pos, len) = (r.position() + 1, r.len());
                self.replay
                    .as_ref()
                    .expect("just borrowed")
                    .apply(&mut self.collector);
                self.set_status(format!("Frame {pos}/{len}"));
            }
            self.rebuild_proc_view();
            return;
        }

        match self.keys.action(key) {
            Some(Action::Quit) => self.should_quit = true,
            Some(Action::Detail) => {
                self.show_detail = !self.show_detail;
                // Fetch immediately so the overlay is never blank for a tick.
                self.refresh_detail();
            }
            Some(Action::Help) => self.show_help = true,
            Some(Action::Pause) => {
                self.paused = !self.paused;
                self.set_status(if self.paused { "Paused" } else { "Resumed" });
            }
            Some(Action::Up) => self.move_selection(-1),
            Some(Action::Down) => self.move_selection(1),
            Some(Action::PageUp) => self.move_selection(-(self.proc_rows.max(1) as isize)),
            Some(Action::PageDown) => self.move_selection(self.proc_rows.max(1) as isize),
            Some(Action::First) => self.select_first(),
            Some(Action::Last) => self.select_last(),
            Some(Action::SortNext) => {
                self.sort = self.sort.next();
                self.set_status(format!("Sort: {}", self.sort.label()));
                self.rebuild_proc_view();
            }
            Some(Action::SortInvert) => {
                self.sort_desc = !self.sort_desc;
                self.rebuild_proc_view();
            }
            Some(Action::Tree) => {
                self.tree = !self.tree;
                self.set_status(if self.tree { "Tree view" } else { "Flat view" });
                self.rebuild_proc_view();
            }
            Some(Action::PerCore) => {
                self.per_core = !self.per_core;
            }
            Some(Action::Layout) => {
                self.layout = self.layout.next();
                self.set_status(format!("Layout: {}", self.layout.label()));
            }
            Some(Action::Connections) => {
                self.show_conn = true;
                self.conn_offset = 0;
                self.refresh_connections();
            }
            Some(Action::Ai) => self.show_ai = !self.show_ai,
            Some(Action::AlertHistory) => self.show_alert_history = !self.show_alert_history,
            Some(Action::ThemeNext) => self.cycle_theme(true),
            Some(Action::ThemePrev) => self.cycle_theme(false),
            Some(Action::TickUp) => self.adjust_tick(true),
            Some(Action::TickDown) => self.adjust_tick(false),
            Some(Action::Filter) => {
                self.filter_mode = true;
                self.set_status("Filter: type to match, Enter to apply, Esc to clear");
            }
            Some(Action::Kill) => self.open_signal_menu(),
            Some(Action::Renice) => self.open_renice_menu(),
            Some(Action::SigTerm) => self.request_kill(Signal::Term),
            Some(Action::SigKill) => self.request_kill(Signal::Kill),
            None => {}
        }
    }

    /// Handle a mouse event over the process table.
    pub fn on_mouse(&mut self, ev: MouseEvent) {
        if self.show_help || self.pending_kill.is_some() || self.signal_menu.is_some() {
            return;
        }
        if self.show_conn {
            match ev.kind {
                MouseEventKind::ScrollUp => self.scroll_conn(-3),
                MouseEventKind::ScrollDown => self.scroll_conn(3),
                _ => {}
            }
            return;
        }
        match ev.kind {
            MouseEventKind::ScrollUp => self.move_selection(-3),
            MouseEventKind::ScrollDown => self.move_selection(3),
            MouseEventKind::Down(_) => {
                let area = self.proc_area;
                // Header row sits one line above the data rows: click to sort.
                if area.height > 0 && ev.row + 1 == area.y && ev.column >= area.x {
                    if let Some(field) = header_sort_at(&self.columns, ev.column - area.x) {
                        if self.sort == field {
                            self.sort_desc = !self.sort_desc;
                        } else {
                            self.sort = field;
                            self.sort_desc = true;
                        }
                        self.rebuild_proc_view();
                    }
                    return;
                }
                if ev.column >= area.x
                    && ev.column < area.x + area.width
                    && ev.row >= area.y
                    && ev.row < area.y + area.height
                {
                    let rel = (ev.row - area.y) as usize;
                    let idx = self.proc_offset + rel;
                    if idx < self.proc_view.len() {
                        self.selected_pid = Some(self.proc_view[idx].pid);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Map a column-relative x offset on the process header to its sort field.
/// Walks the configured columns, mirroring the widths (plus the one-cell
/// column spacing) that `ui::render_procs` hands to the table.
pub fn header_sort_at(columns: &[ProcColumn], rel_x: u16) -> Option<SortField> {
    let mut x = 0u16;
    for col in columns {
        match col.width() {
            // The flexible column runs to the right edge of the table.
            None => return col.sort_field(),
            Some(w) => {
                x = x.saturating_add(w).saturating_add(1); // + column spacing
                if rel_x < x {
                    return col.sort_field();
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, ppid: Option<u32>, name: &str, cpu: f32, mem: u64) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            name: name.into(),
            cmd: format!("/usr/bin/{name}"),
            user: "root".into(),
            cpu,
            mem_pct: 0.0,
            mem_bytes: mem,
            virt: 0,
            disk_read: 0,
            disk_written: 0,
            io_read_rate: 0.0,
            io_write_rate: 0.0,
            start_time: 0,
            run_time: pid as u64,
            status: 'S',
            status_long: "Sleeping",
            threads: 1,
            gpu_mem: 0,
            depth: 0,
        }
    }

    /// An App whose process table is exactly `procs` (no live refresh has run).
    fn app_with(procs: Vec<ProcInfo>) -> App {
        let mut app = App::new(&Config::default());
        app.collector.procs = procs;
        app.selected_pid = None;
        app.rebuild_proc_view();
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn pids(app: &App) -> Vec<u32> {
        app.proc_view.iter().map(|p| p.pid).collect()
    }

    #[test]
    fn sorts_by_cpu_descending_by_default_and_inverts() {
        let mut app = app_with(vec![
            proc(1, None, "low", 1.0, 0),
            proc(2, None, "high", 9.0, 0),
            proc(3, None, "mid", 5.0, 0),
        ]);
        assert_eq!(app.sort, SortField::Cpu);
        assert!(app.sort_desc);
        assert_eq!(pids(&app), vec![2, 3, 1]);

        app.on_key(key(KeyCode::Char('i')));
        assert!(!app.sort_desc);
        assert_eq!(pids(&app), vec![1, 3, 2]);
    }

    #[test]
    fn sort_fields_use_their_own_keys() {
        let mut app = app_with(vec![
            proc(30, None, "Zsh", 1.0, 100),
            proc(10, None, "bash", 2.0, 300),
            proc(20, None, "Fish", 3.0, 200),
        ]);
        app.sort = SortField::Mem;
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![10, 20, 30]);

        app.sort = SortField::Pid;
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![30, 20, 10]);

        // Name sorting is case-insensitive: bash < Fish < Zsh, descending.
        app.sort = SortField::Name;
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![30, 20, 10]);
    }

    #[test]
    fn filter_matches_name_cmd_user_and_pid_case_insensitively() {
        let mut app = app_with(vec![
            proc(100, None, "Firefox", 1.0, 0),
            proc(200, None, "vllm", 2.0, 0),
            proc(300, None, "bash", 3.0, 0),
        ]);
        app.filter = "FIRE".into();
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![100]);

        // cmdline matches too ("/usr/bin/vllm").
        app.filter = "usr/bin/vllm".into();
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![200]);

        // A numeric filter matches the PID.
        app.filter = "300".into();
        app.rebuild_proc_view();
        assert_eq!(pids(&app), vec![300]);

        app.filter = "no-such-thing".into();
        app.rebuild_proc_view();
        assert!(app.proc_view.is_empty());
    }

    #[test]
    fn filter_key_flow_narrows_and_esc_clears() {
        let mut app = app_with(vec![
            proc(1, None, "alpha", 1.0, 0),
            proc(2, None, "beta", 2.0, 0),
        ]);
        app.on_key(key(KeyCode::Char('/')));
        assert!(app.filter_mode);
        app.on_key(key(KeyCode::Char('b')));
        app.on_key(key(KeyCode::Char('e')));
        assert_eq!(pids(&app), vec![2]);

        // Enter applies the filter; Esc (outside filter mode) then clears it.
        app.on_key(key(KeyCode::Enter));
        assert!(!app.filter_mode);
        assert_eq!(app.filter, "be");
        app.on_key(key(KeyCode::Esc));
        assert!(app.filter.is_empty());
        assert_eq!(pids(&app), vec![2, 1]);
    }

    #[test]
    fn tree_orders_parents_before_children_with_depths() {
        let mut app = app_with(vec![
            proc(4, Some(2), "grandchild", 9.0, 0),
            proc(2, Some(1), "child-busy", 5.0, 0),
            proc(3, Some(1), "child-idle", 1.0, 0),
            proc(1, None, "init", 0.0, 0),
        ]);
        app.tree = true;
        app.rebuild_proc_view();
        // DFS from the root; siblings keep the CPU-descending sort.
        assert_eq!(pids(&app), vec![1, 2, 4, 3]);
        let depths: Vec<usize> = app.proc_view.iter().map(|p| p.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 1]);
    }

    #[test]
    fn tree_keeps_orphans_and_survives_cycles() {
        let mut app = app_with(vec![
            proc(1, None, "init", 1.0, 0),
            // Parent not in the table: treated as a root, not dropped.
            proc(7, Some(999), "orphan", 2.0, 0),
            // A ppid cycle: both rows must still be shown.
            proc(8, Some(9), "cyclic-a", 3.0, 0),
            proc(9, Some(8), "cyclic-b", 4.0, 0),
        ]);
        app.tree = true;
        app.rebuild_proc_view();
        let mut got = pids(&app);
        got.sort_unstable();
        assert_eq!(got, vec![1, 7, 8, 9]);
    }

    #[test]
    fn filter_takes_precedence_over_tree() {
        let mut app = app_with(vec![
            proc(1, None, "init", 1.0, 0),
            proc(2, Some(1), "match-me", 5.0, 0),
        ]);
        app.tree = true;
        app.filter = "match".into();
        app.rebuild_proc_view();
        // Flat, filtered result — no tree indentation.
        assert_eq!(pids(&app), vec![2]);
        assert_eq!(app.proc_view[0].depth, 0);
    }

    #[test]
    fn selection_follows_pid_and_falls_back_when_it_vanishes() {
        let mut app = app_with(vec![
            proc(1, None, "a", 3.0, 0),
            proc(2, None, "b", 2.0, 0),
            proc(3, None, "c", 1.0, 0),
        ]);
        // Selection defaults to the first row and moves with clamping.
        assert_eq!(app.selected_pid, Some(1));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.selected_pid, Some(2));
        app.on_key(key(KeyCode::End));
        assert_eq!(app.selected_pid, Some(3));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.selected_pid, Some(3), "must clamp at the last row");
        app.on_key(key(KeyCode::Home));
        assert_eq!(app.selected_pid, Some(1));
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.selected_pid, Some(1), "must clamp at the first row");

        // The selected PID survives a re-sort…
        app.on_key(key(KeyCode::Down));
        app.sort = SortField::Pid;
        app.rebuild_proc_view();
        assert_eq!(app.selected_pid, Some(2));

        // …and falls back to the first row when the process disappears.
        app.collector.procs.retain(|p| p.pid != 2);
        app.rebuild_proc_view();
        assert_eq!(app.selected_pid, app.proc_view.first().map(|p| p.pid));
    }

    #[test]
    fn header_click_map_matches_column_layout() {
        let cols = DEFAULT_COLUMNS;
        assert_eq!(header_sort_at(cols, 0), Some(SortField::Pid));
        assert_eq!(header_sort_at(cols, 18), Some(SortField::Cpu));
        assert_eq!(header_sort_at(cols, 40), Some(SortField::Io));
        assert_eq!(header_sort_at(cols, 63), None);
        assert_eq!(header_sort_at(cols, 120), Some(SortField::Name));
    }
}
