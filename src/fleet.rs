//! Multi-host fleet monitoring.
//!
//! Each host is polled on its own thread by running `toptop --export json`
//! there — over SSH for remote hosts, or the local binary for `local`. The
//! returned snapshot is parsed into a compact [`HostSummary`] and aggregated
//! into a cluster dashboard. Nothing here blocks the UI: the render loop only
//! ever reads the latest published snapshot.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::json::{self, Json};
use crate::theme::{self, Theme};

/// One inference server as reported by a host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerLine {
    pub runtime: String,
    pub model: String,
    pub gen_tps: Option<f64>,
    pub kv_pct: Option<f64>,
}

/// A compact, render-ready summary of one host's `--export json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostSummary {
    pub hostname: String,
    pub os: String,
    pub cpu_usage: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub load: (f64, f64, f64),
    pub uptime: u64,
    pub tasks: u64,
    pub running: u64,
    pub gpu_count: usize,
    pub gpu_util: Option<f64>,
    pub vram_used: u64,
    pub vram_total: u64,
    pub gpu_power: f64,
    pub total_tps: f64,
    pub servers: Vec<ServerLine>,
}

impl HostSummary {
    pub fn mem_pct(&self) -> f64 {
        if self.mem_total == 0 {
            0.0
        } else {
            self.mem_used as f64 / self.mem_total as f64 * 100.0
        }
    }
    pub fn vram_pct(&self) -> f64 {
        if self.vram_total == 0 {
            0.0
        } else {
            self.vram_used as f64 / self.vram_total as f64 * 100.0
        }
    }
}

/// Parse a `toptop --export json` document into a [`HostSummary`].
pub fn parse_host_json(text: &str) -> Option<HostSummary> {
    let root = json::parse(text)?;
    let mut h = HostSummary::default();

    if let Some(host) = root.get("host") {
        h.hostname = host.str("hostname").unwrap_or("?").to_string();
        h.os = host.str("os").unwrap_or_default().to_string();
    }
    h.uptime = root.u64("uptime").unwrap_or(0);
    h.tasks = root.u64("tasks").unwrap_or(0);
    h.running = root.u64("running").unwrap_or(0);

    if let Some(cpu) = root.get("cpu") {
        h.cpu_usage = cpu.num("usage").unwrap_or(0.0);
        if let Some(load) = cpu.get("load").and_then(Json::as_array) {
            let g = |i: usize| load.get(i).and_then(Json::as_f64).unwrap_or(0.0);
            h.load = (g(0), g(1), g(2));
        }
    }
    if let Some(mem) = root.get("mem") {
        h.mem_used = mem.u64("used").unwrap_or(0);
        h.mem_total = mem.u64("total").unwrap_or(0);
    }

    if let Some(gpus) = root.get("gpus").and_then(Json::as_array) {
        h.gpu_count = gpus.len();
        let mut best_util: Option<f64> = None;
        for g in gpus {
            if g.get("has_util")
                .map(|v| v == &Json::Bool(true))
                .unwrap_or(false)
            {
                let u = g.num("util").unwrap_or(0.0);
                best_util = Some(best_util.map_or(u, |b: f64| b.max(u)));
            }
            h.vram_used += g.u64("mem_used").unwrap_or(0);
            h.vram_total += g.u64("mem_total").unwrap_or(0);
            h.gpu_power += g.num("power").unwrap_or(0.0);
        }
        h.gpu_util = best_util;
    }

    if let Some(servers) = root.get("servers").and_then(Json::as_array) {
        for s in servers {
            let line = ServerLine {
                runtime: s.str("runtime").unwrap_or("?").to_string(),
                model: s.str("model").unwrap_or_default().to_string(),
                gen_tps: s.num("gen_tps"),
                kv_pct: s.num("kv_pct"),
            };
            h.total_tps += line.gen_tps.unwrap_or(0.0);
            h.servers.push(line);
        }
    }

    Some(h)
}

/// Connection state of a single fleet host.
#[derive(Clone, Debug, PartialEq)]
pub enum HostStatus {
    Connecting,
    Online,
    Error(String),
}

/// The latest known state of one host.
#[derive(Clone, Debug)]
pub struct HostState {
    pub name: String,
    pub status: HostStatus,
    pub summary: Option<HostSummary>,
    pub latency_ms: Option<u64>,
}

fn is_local(host: &str) -> bool {
    matches!(host, "local" | "localhost" | "127.0.0.1")
}

/// Run the export on a host and return its JSON (or an error string).
fn fetch(host: &str, remote_cmd: &str) -> Result<String, String> {
    let output = if is_local(host) {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Command::new(exe)
            .args(["--export", "json"])
            .output()
            .map_err(|e| e.to_string())?
    } else {
        Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=6",
                host,
                remote_cmd,
            ])
            .output()
            .map_err(|e| e.to_string())?
    };
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let msg = err.lines().next().unwrap_or("command failed").trim();
        return Err(if msg.is_empty() {
            "command failed".into()
        } else {
            msg.to_string()
        });
    }
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    if out.trim().is_empty() {
        return Err("empty response".into());
    }
    Ok(out)
}

/// Background poller: one thread per host, each republishing its latest state.
pub struct FleetMonitor {
    hosts: Vec<Arc<Mutex<HostState>>>,
}

impl FleetMonitor {
    pub fn new(specs: Vec<String>, remote_cmd: String, interval: Duration) -> Self {
        let mut hosts = Vec::new();
        for name in specs {
            let state = Arc::new(Mutex::new(HostState {
                name: name.clone(),
                status: HostStatus::Connecting,
                summary: None,
                latency_ms: None,
            }));
            hosts.push(Arc::clone(&state));
            let cmd = remote_cmd.clone();
            std::thread::Builder::new()
                .name(format!("toptop-fleet-{name}"))
                .spawn(move || loop {
                    let t0 = Instant::now();
                    let result = fetch(&name, &cmd);
                    let latency = t0.elapsed().as_millis() as u64;
                    if let Ok(mut s) = state.lock() {
                        match result {
                            Ok(json) => match parse_host_json(&json) {
                                Some(sum) => {
                                    s.status = HostStatus::Online;
                                    s.summary = Some(sum);
                                    s.latency_ms = Some(latency);
                                }
                                None => s.status = HostStatus::Error("unparseable export".into()),
                            },
                            Err(e) => {
                                s.status = HostStatus::Error(e);
                                s.latency_ms = Some(latency);
                            }
                        }
                    }
                    std::thread::sleep(interval);
                })
                .ok();
        }
        Self { hosts }
    }

    /// Snapshot the current state of every host.
    pub fn snapshot(&self) -> Vec<HostState> {
        self.hosts
            .iter()
            .filter_map(|h| h.lock().ok().map(|s| s.clone()))
            .collect()
    }
}

/// State for the fleet TUI.
pub struct FleetApp {
    monitor: FleetMonitor,
    pub hosts: Vec<HostState>,
    pub selected: usize,
    pub theme_idx: usize,
    pub tick: Duration,
    pub should_quit: bool,
}

impl FleetApp {
    pub fn new(specs: Vec<String>, remote_cmd: String, theme_idx: usize) -> Self {
        let monitor = FleetMonitor::new(specs, remote_cmd, Duration::from_secs(3));
        let hosts = monitor.snapshot();
        Self {
            monitor,
            hosts,
            selected: 0,
            theme_idx: theme_idx.min(theme::themes().len() - 1),
            tick: Duration::from_millis(1000),
            should_quit: false,
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &theme::themes()[self.theme_idx]
    }

    pub fn on_tick(&mut self) {
        self.hosts = self.monitor.snapshot();
        if self.selected >= self.hosts.len() {
            self.selected = self.hosts.len().saturating_sub(1);
        }
    }

    pub fn selected_host(&self) -> Option<&HostState> {
        self.hosts.get(self.selected)
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.hosts.is_empty() {
            return;
        }
        let max = self.hosts.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
    }

    /// Aggregate totals across all online hosts: (online, tps, vram_used, vram_total, gpus).
    pub fn totals(&self) -> (usize, f64, u64, u64, usize) {
        let mut online = 0;
        let (mut tps, mut vu, mut vt, mut gpus) = (0.0, 0, 0, 0);
        for h in &self.hosts {
            if let Some(s) = &h.summary {
                if h.status == HostStatus::Online {
                    online += 1;
                    tps += s.total_tps;
                    vu += s.vram_used;
                    vt += s.vram_total;
                    gpus += s.gpu_count;
                }
            }
        }
        (online, tps, vu, vt, gpus)
    }

    pub fn on_key(&mut self, code: ratatui::crossterm::event::KeyCode) {
        use ratatui::crossterm::event::KeyCode;
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = self.hosts.len().saturating_sub(1),
            KeyCode::Char('p') => {
                self.theme_idx = (self.theme_idx + 1) % theme::themes().len();
            }
            KeyCode::Char('P') => {
                self.theme_idx =
                    (self.theme_idx + theme::themes().len() - 1) % theme::themes().len();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "host":{"hostname":"gpu-node-1","os":"Linux (Ubuntu 24.04)","kernel":"6.8","arch":"x86_64","cpu":"Xeon","cores":32},
      "uptime":3600,"tasks":420,"running":3,
      "cpu":{"usage":37.50,"freq_mhz":3200,"load":[4.20,3.10,2.50],"per_core":[10,20]},
      "mem":{"used":34359738368,"total":68719476736,"swap_used":0,"swap_total":0},
      "net":[],"disk_io":{"read_rate":0,"write_rate":0},"disks":[],
      "gpus":[
        {"name":"NVIDIA A100","util":91.00,"has_util":true,"mem_util":80,"mem_used":42949672960,"mem_total":85899345920,"temp":71,"power":310.00,"power_limit":400,"throttled":false},
        {"name":"NVIDIA A100","util":12.00,"has_util":true,"mem_util":5,"mem_used":1073741824,"mem_total":85899345920,"temp":45,"power":90.00,"power_limit":400,"throttled":false}
      ],
      "gpu_procs":[],
      "servers":[{"runtime":"vLLM","pid":42,"port":8000,"model":"meta-llama/Llama-3-70B","gen_tps":58.30,"prompt_tps":900,"running":2,"waiting":1,"kv_pct":72.00,"ttft_ms":210,"gpu_offload_pct":null}],
      "sensors":[],"battery":null,
      "procs":[{"pid":42,"name":"vllm","user":"ml","cpu":190.0,"mem_pct":12.0,"mem_bytes":1024,"io_read":0,"io_write":0}]
    }"#;

    #[test]
    fn parses_export_summary() {
        let h = parse_host_json(SAMPLE).expect("parse");
        assert_eq!(h.hostname, "gpu-node-1");
        assert_eq!(h.cpu_usage, 37.5);
        assert_eq!(h.load.0, 4.2);
        assert_eq!(h.mem_total, 68719476736);
        assert_eq!(h.gpu_count, 2);
        assert_eq!(h.gpu_util, Some(91.0)); // max across cards
        assert_eq!(h.gpu_power, 400.0); // summed
        assert_eq!(h.servers.len(), 1);
        assert_eq!(h.total_tps, 58.3);
        assert_eq!(h.servers[0].model, "meta-llama/Llama-3-70B");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_host_json("not json").is_none());
    }

    #[test]
    fn local_detection() {
        assert!(is_local("local"));
        assert!(is_local("localhost"));
        assert!(!is_local("gpu-node-1"));
    }
}
