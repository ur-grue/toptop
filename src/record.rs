//! Flight recorder: `--record` writes one JSON snapshot per tick, `--replay`
//! plays them back in the normal TUI.
//!
//! "The GPU throttled ten minutes ago — what happened?" toptop keeps 256
//! in-memory samples and forgets everything on exit. A recording turns it into
//! evidence you can scrub through, share in a bug report, and replay on a
//! machine that has no GPU at all.
//!
//! The format is JSONL: exactly the document `--export json` produces, one per
//! line, each carrying a `schema_version` so a future toptop can tell an old
//! recording from a new one instead of silently misreading it.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::json::Json;
use crate::metrics::gpu::{Gpu, GpuProc};
use crate::metrics::{Battery, Collector, ProcInfo, SensorInfo, ServerStats};

/// Version of the recording format. Bump when a change would make an older
/// toptop misread a newer file (or vice versa).
pub const SCHEMA_VERSION: u64 = 1;

/// How many process rows a recording keeps per frame. Higher than the export
/// default: a flight recorder that only kept the top 20 would lose whatever
/// process caused the incident the moment it stopped being the busiest.
pub const RECORD_PROCS: usize = 100;

/// Appends one JSON line per tick to a file.
pub struct Recorder {
    file: File,
}

impl Recorder {
    /// Open `path` for appending, creating it if needed.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file })
    }

    /// Write one frame. Errors are returned rather than swallowed — a
    /// recorder that silently stopped recording would be worse than none.
    pub fn record(&mut self, c: &Collector) -> std::io::Result<()> {
        let line = crate::export::to_json(c, RECORD_PROCS);
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

/// One recorded frame, parsed into the fields the UI renders.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub schema_version: u64,
    pub uptime: u64,
    pub cpu_usage: f32,
    pub cpu_freq_mhz: u64,
    pub load: (f64, f64, f64),
    pub per_core: Vec<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_read_rate: f64,
    pub disk_write_rate: f64,
    pub gpus: Vec<Gpu>,
    pub gpu_procs: Vec<GpuProc>,
    pub servers: Vec<ServerStats>,
    pub sensors: Vec<SensorInfo>,
    pub battery: Option<Battery>,
    pub procs: Vec<ProcInfo>,
    pub hostname: String,
    pub os: String,
}

/// Parse one recorded line. Returns `None` for anything that isn't a JSON
/// object, so a truncated final line (the common case when a recording was
/// killed mid-write) is skipped rather than fatal.
pub fn parse_frame(line: &str) -> Option<Frame> {
    let root = crate::json::parse(line)?;
    if !matches!(root, Json::Obj(_)) {
        return None;
    }
    let mut f = Frame {
        // Recordings predating the field are version 0 by definition.
        schema_version: root.u64("schema_version").unwrap_or(0),
        uptime: root.u64("uptime").unwrap_or(0),
        ..Frame::default()
    };

    if let Some(h) = root.get("host") {
        f.hostname = h.str("hostname").unwrap_or_default().to_string();
        f.os = h.str("os").unwrap_or_default().to_string();
    }
    if let Some(cpu) = root.get("cpu") {
        f.cpu_usage = cpu.num("usage").unwrap_or(0.0) as f32;
        f.cpu_freq_mhz = cpu.u64("freq_mhz").unwrap_or(0);
        if let Some(load) = cpu.get("load").and_then(Json::as_array) {
            let g = |i: usize| load.get(i).and_then(Json::as_f64).unwrap_or(0.0);
            f.load = (g(0), g(1), g(2));
        }
        if let Some(cores) = cpu.get("per_core").and_then(Json::as_array) {
            f.per_core = cores
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
        }
    }
    if let Some(m) = root.get("mem") {
        f.mem_used = m.u64("used").unwrap_or(0);
        f.mem_total = m.u64("total").unwrap_or(0);
        f.swap_used = m.u64("swap_used").unwrap_or(0);
        f.swap_total = m.u64("swap_total").unwrap_or(0);
    }
    if let Some(io) = root.get("disk_io") {
        f.disk_read_rate = io.num("read_rate").unwrap_or(0.0);
        f.disk_write_rate = io.num("write_rate").unwrap_or(0.0);
    }
    if let Some(gpus) = root.get("gpus").and_then(Json::as_array) {
        f.gpus = gpus
            .iter()
            .map(|g| Gpu {
                name: g.str("name").unwrap_or_default().to_string(),
                util_pct: g.num("util").unwrap_or(0.0) as f32,
                has_util: g.get("has_util") == Some(&Json::Bool(true)),
                mem_util: g.num("mem_util").unwrap_or(0.0) as f32,
                has_mem_util: g.get("has_mem_util") == Some(&Json::Bool(true)),
                mem_used: g.u64("mem_used").unwrap_or(0),
                mem_total: g.u64("mem_total").unwrap_or(0),
                temp: g.num("temp").unwrap_or(0.0) as f32,
                power: g.num("power").unwrap_or(0.0) as f32,
                power_limit: g.num("power_limit").unwrap_or(0.0) as f32,
                throttled: g.get("throttled") == Some(&Json::Bool(true)),
            })
            .collect();
    }
    if let Some(gp) = root.get("gpu_procs").and_then(Json::as_array) {
        f.gpu_procs = gp
            .iter()
            .map(|g| GpuProc {
                pid: g.u64("pid").unwrap_or(0) as u32,
                used_mem: g.u64("used_mem").unwrap_or(0),
            })
            .collect();
    }
    if let Some(servers) = root.get("servers").and_then(Json::as_array) {
        f.servers = servers
            .iter()
            .map(|s| ServerStats {
                // `runtime` is a &'static str on the live struct; recordings
                // carry arbitrary strings, so map the known ones and fall back
                // to a neutral label rather than leaking the allocation.
                runtime: known_runtime(s.str("runtime").unwrap_or_default()),
                pid: s.u64("pid").unwrap_or(0) as u32,
                port: s.u64("port").unwrap_or(0) as u16,
                model: s.str("model").unwrap_or_default().to_string(),
                gen_tps: s.num("gen_tps"),
                prompt_tps: s.num("prompt_tps"),
                running: s.num("running"),
                waiting: s.num("waiting"),
                kv_pct: s.num("kv_pct"),
                ttft_ms: s.num("ttft_ms"),
                gpu_offload_pct: s.num("gpu_offload_pct"),
                preemptions: s.num("preemptions"),
                preempt_rate: s.num("preempt_rate"),
                // The JSON export carries neither the latency histograms nor
                // the manual-target address, so a replayed server has neither
                // rather than a fabricated one.
                ttft: None,
                tpot: None,
                addr: None,
            })
            .collect();
    }
    if let Some(sensors) = root.get("sensors").and_then(Json::as_array) {
        f.sensors = sensors
            .iter()
            .map(|t| SensorInfo {
                label: t.str("label").unwrap_or_default().to_string(),
                temp: t.num("temp").unwrap_or(0.0) as f32,
                // The export carries no thresholds, so a replayed sensor has
                // none rather than an invented default.
                high: None,
                critical: None,
            })
            .collect();
    }
    if let Some(b) = root.get("battery") {
        if b != &Json::Null {
            f.battery = Some(Battery {
                percent: b.num("percent").unwrap_or(0.0) as f32,
                status: b.str("status").unwrap_or("Unknown").to_string(),
            });
        }
    }
    if let Some(procs) = root.get("procs").and_then(Json::as_array) {
        f.procs = procs
            .iter()
            .map(|p| ProcInfo {
                pid: p.u64("pid").unwrap_or(0) as u32,
                name: p.str("name").unwrap_or_default().to_string(),
                cmd: p.str("name").unwrap_or_default().to_string(),
                user: p.str("user").unwrap_or_default().to_string(),
                cpu: p.num("cpu").unwrap_or(0.0) as f32,
                mem_pct: p.num("mem_pct").unwrap_or(0.0) as f32,
                mem_bytes: p.u64("mem_bytes").unwrap_or(0),
                io_read_rate: p.num("io_read").unwrap_or(0.0),
                io_write_rate: p.num("io_write").unwrap_or(0.0),
                status: '?',
                status_long: "Recorded",
                ..ProcInfo::default()
            })
            .collect();
    }
    Some(f)
}

/// Map a recorded runtime name back to the `&'static str` the live code uses.
fn known_runtime(name: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        "vLLM",
        "llama.cpp",
        "TGI",
        "TensorRT-LLM",
        "SGLang",
        "Ollama",
        "LM Studio",
    ];
    KNOWN
        .iter()
        .copied()
        .find(|k| k.eq_ignore_ascii_case(name))
        .unwrap_or("recorded")
}

/// A loaded recording, positioned at one frame.
pub struct Replay {
    frames: Vec<Frame>,
    idx: usize,
    /// Whether playback is advancing. Starts running; space toggles.
    pub paused: bool,
    /// Frames that could not be parsed, reported once in the UI.
    pub skipped: usize,
}

impl Replay {
    /// Load a JSONL recording. Unparseable lines are counted and skipped — a
    /// recording killed mid-write ends in a partial line, and losing the whole
    /// file over its last 40 bytes would be absurd.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let mut frames = Vec::new();
        let mut skipped = 0;
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match parse_frame(&line) {
                Some(f) => frames.push(f),
                None => skipped += 1,
            }
        }
        if frames.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: no readable frames", path.display()),
            ));
        }
        Ok(Self {
            frames,
            idx: 0,
            paused: false,
            skipped,
        })
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Position of the current frame, 0-based.
    pub fn position(&self) -> usize {
        self.idx
    }

    /// Schema version of the loaded recording (from its first frame).
    pub fn schema_version(&self) -> u64 {
        self.frames.first().map(|f| f.schema_version).unwrap_or(0)
    }

    pub fn current(&self) -> &Frame {
        &self.frames[self.idx]
    }

    /// Advance one frame unless paused or already at the end. Playback stops
    /// at the last frame instead of looping — a flight recorder that wrapped
    /// around would make "when did it start" unanswerable.
    pub fn tick(&mut self) {
        if !self.paused {
            self.step(1);
        }
    }

    /// Step by `delta` frames, clamped to the recording. Stepping manually
    /// pauses playback, which is what you want when scrubbing.
    pub fn step(&mut self, delta: isize) {
        let next = (self.idx as isize + delta).clamp(0, self.frames.len() as isize - 1);
        self.idx = next as usize;
    }

    /// Copy the current frame into a collector so the normal UI renders it.
    pub fn apply(&self, c: &mut Collector) {
        let f = self.current();
        c.uptime = f.uptime;
        c.cpu.global_usage = f.cpu_usage;
        c.cpu.freq_mhz = f.cpu_freq_mhz;
        c.cpu.load_avg = f.load;
        if !f.per_core.is_empty() {
            c.cpu.per_core = f.per_core.clone();
        }
        c.mem.used = f.mem_used;
        c.mem.total = f.mem_total;
        c.mem.swap_used = f.swap_used;
        c.mem.swap_total = f.swap_total;
        c.disk_read_rate = f.disk_read_rate;
        c.disk_write_rate = f.disk_write_rate;
        c.gpus = f.gpus.clone();
        c.gpu_procs = f.gpu_procs.clone();
        c.servers = f.servers.clone();
        c.sensors = f.sensors.clone();
        c.battery = f.battery.clone();
        c.procs = f.procs.clone();
        if !f.hostname.is_empty() {
            c.host.hostname = f.hostname.clone();
        }
        if !f.os.is_empty() {
            c.host.os = f.os.clone();
        }
        // Feed the history so graphs redraw as you scrub rather than staying
        // flat at whatever the live machine was doing.
        c.cpu.global_history.push(f.cpu_usage as f64);
        for (i, v) in f.per_core.iter().enumerate() {
            if let Some(h) = c.cpu.core_history.get_mut(i) {
                h.push(*v as f64);
            }
        }
        if f.mem_total > 0 {
            c.mem
                .used_history
                .push(f.mem_used as f64 / f.mem_total as f64 * 100.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but realistic recorded line.
    const LINE: &str = r#"{"schema_version":1,"host":{"hostname":"gpu-box","os":"Ubuntu 24.04","kernel":"6.8","arch":"x86_64","cpu":"EPYC","cores":32},"uptime":3600,"tasks":400,"running":3,"cpu":{"usage":42.5,"freq_mhz":3200,"load":[1.5,1.2,0.9],"per_core":[10,20,30,40]},"mem":{"used":8000,"total":16000,"swap_used":0,"swap_total":1000},"net":[],"disk_io":{"read_rate":1024,"write_rate":2048},"disks":[],"gpus":[{"name":"RTX 4090","util":88,"has_util":true,"mem_util":71,"has_mem_util":true,"mem_used":21000,"mem_total":24000,"temp":72,"power":390,"power_limit":450,"throttled":true}],"gpu_procs":[{"pid":4242,"name":"python","used_mem":20000}],"servers":[{"runtime":"vLLM","pid":4242,"port":8000,"model":"Llama-3-8B","gen_tps":83.4,"prompt_tps":1200,"running":2,"waiting":5,"kv_pct":64,"ttft_ms":180,"gpu_offload_pct":null}],"sensors":[{"label":"CPU","temp":55}],"battery":null,"procs":[{"pid":4242,"name":"python","user":"ml","cpu":99.5,"mem_pct":12.5,"mem_bytes":900,"io_read":10,"io_write":20}]}"#;

    #[test]
    fn parses_a_recorded_frame() {
        let f = parse_frame(LINE).expect("valid frame");
        assert_eq!(f.schema_version, 1);
        assert_eq!(f.hostname, "gpu-box");
        assert_eq!(f.cpu_usage, 42.5);
        assert_eq!(f.per_core, vec![10.0, 20.0, 30.0, 40.0]);
        assert_eq!(f.mem_used, 8000);
        assert_eq!(f.gpus.len(), 1);
        assert!(
            f.gpus[0].throttled,
            "throttle state must survive a round trip"
        );
        assert_eq!(f.gpus[0].mem_used, 21000);
        assert_eq!(f.servers[0].runtime, "vLLM");
        assert_eq!(f.servers[0].gen_tps, Some(83.4));
        assert_eq!(f.servers[0].gpu_offload_pct, None, "null stays None");
        assert_eq!(f.procs[0].pid, 4242);
        assert_eq!(f.battery, None);
    }

    #[test]
    fn rejects_junk_and_dates_old_recordings() {
        assert!(parse_frame("").is_none());
        assert!(parse_frame("not json").is_none());
        assert!(parse_frame("[1,2,3]").is_none(), "must be an object");
        // A line truncated mid-write is skipped, not fatal.
        assert!(parse_frame(&LINE[..LINE.len() / 2]).is_none());
        // Pre-versioning recordings are version 0, not "unknown".
        assert_eq!(parse_frame(r#"{"uptime":1}"#).unwrap().schema_version, 0);
    }

    #[test]
    fn unknown_runtime_names_do_not_leak() {
        assert_eq!(known_runtime("vllm"), "vLLM");
        assert_eq!(known_runtime("Ollama"), "Ollama");
        assert_eq!(known_runtime("something-new"), "recorded");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("toptop-rec-{}-{name}.jsonl", std::process::id()))
    }

    #[test]
    fn replay_loads_steps_and_clamps() {
        let path = scratch("steps");
        let body = format!("{LINE}\n{LINE}\n\ngarbage\n{LINE}\n");
        std::fs::write(&path, body).unwrap();

        let mut r = Replay::load(&path).expect("loads");
        assert_eq!(r.len(), 3);
        assert_eq!(r.skipped, 1, "the junk line is counted, not fatal");
        assert_eq!(r.schema_version(), SCHEMA_VERSION);

        assert_eq!(r.position(), 0);
        r.tick();
        assert_eq!(r.position(), 1);
        // Playback stops at the end rather than looping.
        r.tick();
        r.tick();
        r.tick();
        assert_eq!(r.position(), 2);
        // Stepping back is clamped at zero.
        r.step(-10);
        assert_eq!(r.position(), 0);
        // Paused playback doesn't advance.
        r.paused = true;
        r.tick();
        assert_eq!(r.position(), 0);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_file_with_no_frames_is_an_error() {
        let path = scratch("empty");
        std::fs::write(&path, "garbage\nmore garbage\n").unwrap();
        assert!(Replay::load(&path).is_err());
        std::fs::remove_file(&path).ok();
        assert!(Replay::load(&scratch("missing")).is_err());
    }

    #[test]
    fn record_then_replay_round_trips() {
        let path = scratch("roundtrip");
        std::fs::remove_file(&path).ok();
        let c = Collector::new(16);
        {
            let mut rec = Recorder::create(&path).expect("create");
            rec.record(&c).expect("write");
            rec.record(&c).expect("write");
        }
        let r = Replay::load(&path).expect("load what we just wrote");
        assert_eq!(r.len(), 2);
        assert_eq!(r.schema_version(), SCHEMA_VERSION);
        assert_eq!(r.current().hostname, c.host.hostname);
        std::fs::remove_file(&path).ok();
    }
}
