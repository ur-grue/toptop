//! Inference-server auto-discovery and metrics scraping.
//!
//! toptop already knows which processes are inference runtimes and which ports
//! they listen on, so it can do something `nvidia-smi` can't: find your local
//! servers and scrape the numbers you actually watch — **tokens/sec**, KV-cache
//! pressure, queue depth, TTFT — straight from their metrics endpoints.
//!
//! Supported: llama.cpp / vLLM / TGI Prometheus `/metrics`, and Ollama's
//! `/api/ps` JSON. The HTTP client is a tiny hand-rolled localhost GET (no
//! dependencies); scraping runs on a background thread so it never blocks the
//! UI. The parsers are pure and unit-tested; discovery degrades to nothing when
//! no server is listening.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ai::{self, AiKind};
use super::netconn;

/// Live stats for one discovered inference server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerStats {
    pub runtime: &'static str,
    pub pid: u32,
    pub port: u16,
    pub model: String,
    /// Generation throughput, tokens/sec.
    pub gen_tps: Option<f64>,
    /// Prompt (prefill) throughput, tokens/sec.
    pub prompt_tps: Option<f64>,
    pub running: Option<f64>,
    pub waiting: Option<f64>,
    /// KV / GPU cache usage, 0–100%.
    pub kv_pct: Option<f64>,
    /// Mean time-to-first-token, milliseconds.
    pub ttft_ms: Option<f64>,
    /// Ollama only: share of the model resident on GPU, 0–100%.
    pub gpu_offload_pct: Option<f64>,
}

// ── Pure parsers ───────────────────────────────────────────────────────────

/// Split an HTTP response into (headers, body). Returns `None` if malformed.
pub fn split_response(resp: &str) -> Option<(&str, &str)> {
    resp.split_once("\r\n\r\n")
        .or_else(|| resp.split_once("\n\n"))
}

/// De-chunk a `Transfer-Encoding: chunked` body. Non-chunked input is returned
/// unchanged when sizes don't parse, so this is safe to call unconditionally.
pub fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else {
            // No (more) chunk header: if we've decoded nothing, it wasn't
            // chunked at all — return the body untouched.
            return if out.is_empty() {
                body.to_string()
            } else {
                out
            };
        };
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_hex, 16) else {
            return if out.is_empty() {
                body.to_string()
            } else {
                out
            };
        };
        if size == 0 {
            break;
        }
        if after.len() < size {
            return if out.is_empty() {
                body.to_string()
            } else {
                out
            };
        }
        out.push_str(&after[..size]);
        rest = after[size..].strip_prefix("\r\n").unwrap_or(&after[size..]);
    }
    out
}

/// Sum every Prometheus series whose metric name is exactly `name`
/// (ignoring labels). Returns `None` if no series matched.
pub fn prom_sum(text: &str, name: &str) -> Option<f64> {
    let mut found = false;
    let mut total = 0.0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with(name) {
            continue;
        }
        let rest = &line[name.len()..];
        let valpart = match rest.chars().next() {
            Some('{') => match rest.find('}') {
                Some(close) => &rest[close + 1..],
                None => continue,
            },
            Some(c) if c.is_whitespace() => rest,
            _ => continue, // `name` is only a prefix of a longer metric
        };
        if let Some(tok) = valpart.split_whitespace().next() {
            if let Ok(v) = tok.parse::<f64>() {
                total += v;
                found = true;
            }
        }
    }
    found.then_some(total)
}

/// Extract the first value of `label` from any series of `metric`.
pub fn prom_label(text: &str, metric: &str, label: &str) -> Option<String> {
    let needle = format!("{label}=\"");
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with(metric) {
            continue;
        }
        if let Some(p) = line.find(&needle) {
            let start = p + needle.len();
            if let Some(end) = line[start..].find('"') {
                return Some(line[start..start + end].to_string());
            }
        }
    }
    None
}

/// One model entry from Ollama's `/api/ps`.
#[derive(Clone, Debug, PartialEq)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub size_vram: u64,
}

fn num_after(hay: &str, from: usize, pat: &str, until: usize) -> Option<u64> {
    let region = &hay[from..until.min(hay.len())];
    let p = region.find(pat)? + pat.len();
    let digits: String = region[p..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Parse the subset of Ollama `/api/ps` we need: model name, size and the
/// portion resident in VRAM (for the GPU/CPU split).
pub fn parse_ollama_ps(json: &str) -> Vec<OllamaModel> {
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(rel) = json[idx..].find("\"name\":\"") {
        let nstart = idx + rel + "\"name\":\"".len();
        let Some(nend) = json[nstart..].find('"') else {
            break;
        };
        let name = json[nstart..nstart + nend].to_string();
        // Bound the numeric search to this object (before the next "name").
        let next = json[nstart..]
            .find("\"name\":\"")
            .map(|p| nstart + p)
            .unwrap_or(json.len());
        let size = num_after(json, nstart, "\"size\":", next).unwrap_or(0);
        let size_vram = num_after(json, nstart, "\"size_vram\":", next).unwrap_or(0);
        out.push(OllamaModel {
            name,
            size,
            size_vram,
        });
        idx = nstart + nend;
    }
    out
}

/// Build the gauge-derived part of `ServerStats` from a Prometheus body. Counter
/// rates (vLLM tokens/sec) are layered on by the monitor, which holds history.
pub fn stats_from_prometheus(text: &str, pid: u32, port: u16) -> Option<ServerStats> {
    let mut s = ServerStats {
        pid,
        port,
        ..Default::default()
    };
    if text.contains("llamacpp:") {
        s.runtime = "llama.cpp";
        s.gen_tps = prom_sum(text, "llamacpp:predicted_tokens_seconds");
        s.prompt_tps = prom_sum(text, "llamacpp:prompt_tokens_seconds");
        s.kv_pct = prom_sum(text, "llamacpp:kv_cache_usage_ratio").map(|v| v * 100.0);
        s.running = prom_sum(text, "llamacpp:requests_processing");
        s.waiting = prom_sum(text, "llamacpp:requests_deferred");
    } else if text.contains("vllm:") {
        s.runtime = "vLLM";
        s.kv_pct = prom_sum(text, "vllm:gpu_cache_usage_perc").map(|v| v * 100.0);
        s.running = prom_sum(text, "vllm:num_requests_running");
        s.waiting = prom_sum(text, "vllm:num_requests_waiting");
        s.model = prom_label(text, "vllm:", "model_name").unwrap_or_default();
        if let (Some(sum), Some(cnt)) = (
            prom_sum(text, "vllm:time_to_first_token_seconds_sum"),
            prom_sum(text, "vllm:time_to_first_token_seconds_count"),
        ) {
            if cnt > 0.0 {
                s.ttft_ms = Some(sum / cnt * 1000.0);
            }
        }
    } else if text.contains("tgi_") {
        s.runtime = "TGI";
        s.running = prom_sum(text, "tgi_batch_current_size");
        s.waiting = prom_sum(text, "tgi_queue_size");
    } else {
        return None;
    }
    Some(s)
}

// ── HTTP (localhost only) ──────────────────────────────────────────────────

fn http_get(port: u16, path: &str, timeout: Duration) -> Option<String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: toptop\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::with_capacity(8192);
    // Cap the read so a misbehaving endpoint can't balloon memory.
    stream.take(1 << 20).read_to_end(&mut buf).ok()?;
    let resp = String::from_utf8_lossy(&buf).into_owned();
    let (headers, body) = split_response(&resp)?;
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        Some(dechunk(body))
    } else {
        Some(body.to_string())
    }
}

fn read_proc_text(pid: u32, file: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/{file}")).ok()
}

/// Explanation shown when no inference servers were auto-discovered, tailored
/// to the build target. Localhost discovery walks `/proc`, so it only runs on
/// Linux today; elsewhere the AI view should say so rather than imply none are
/// running. Exactly one arm compiles per platform.
pub fn no_servers_reason() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "No inference servers found on localhost."
    }
    #[cfg(not(target_os = "linux"))]
    {
        "Localhost server discovery is Linux-only (other platforms tracked in ur-grue/toptop#13)."
    }
}

/// Discover serving runtimes that are listening, as `(port, pid)`.
fn discover_servers() -> Vec<(u16, u32)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (port, pid) in netconn::listening_sockets() {
        let comm = read_proc_text(pid, "comm").unwrap_or_default();
        let cmd = read_proc_text(pid, "cmdline")
            .unwrap_or_default()
            .replace('\0', " ");
        if let Some(rt) = ai::detect_runtime(comm.trim(), &cmd) {
            if rt.kind == AiKind::Serving && seen.insert((port, pid)) {
                out.push((port, pid));
            }
        }
    }
    out
}

/// Background poller that auto-discovers and scrapes local inference servers.
pub struct InferenceMonitor {
    latest: Arc<Mutex<Vec<ServerStats>>>,
}

impl InferenceMonitor {
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&latest);
        std::thread::Builder::new()
            .name("toptop-infer".into())
            .spawn(move || {
                // prev counter values for rate (tokens/sec) computation.
                let mut prev: HashMap<(u32, u16, &'static str), (f64, Instant)> = HashMap::new();
                loop {
                    let servers = scrape_once(&mut prev);
                    if let Ok(mut slot) = shared.lock() {
                        *slot = servers;
                    }
                    std::thread::sleep(Duration::from_millis(2000));
                }
            })
            .ok();
        Self { latest }
    }

    pub fn snapshot(&self) -> Vec<ServerStats> {
        self.latest.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Default for InferenceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a per-second rate from a cumulative counter, keyed by server+metric.
fn counter_rate(
    prev: &mut HashMap<(u32, u16, &'static str), (f64, Instant)>,
    key: (u32, u16, &'static str),
    cur: f64,
    now: Instant,
) -> Option<f64> {
    let rate = prev.get(&key).and_then(|(pv, pt)| {
        let dt = now.duration_since(*pt).as_secs_f64();
        (dt > 0.05 && cur >= *pv).then(|| (cur - *pv) / dt)
    });
    prev.insert(key, (cur, now));
    rate
}

fn scrape_once(prev: &mut HashMap<(u32, u16, &'static str), (f64, Instant)>) -> Vec<ServerStats> {
    let timeout = Duration::from_millis(250);
    let now = Instant::now();
    let mut out: Vec<ServerStats> = Vec::new();
    for (port, pid) in discover_servers() {
        if out.iter().any(|s| s.pid == pid) {
            continue; // one server per process
        }
        // Prometheus endpoints (llama.cpp / vLLM / TGI).
        if let Some(body) = http_get(port, "/metrics", timeout) {
            if let Some(mut s) = stats_from_prometheus(&body, pid, port) {
                if s.runtime == "vLLM" {
                    if let Some(g) = prom_sum(&body, "vllm:generation_tokens_total") {
                        s.gen_tps = counter_rate(prev, (pid, port, "gen"), g, now);
                    }
                    if let Some(p) = prom_sum(&body, "vllm:prompt_tokens_total") {
                        s.prompt_tps = counter_rate(prev, (pid, port, "prompt"), p, now);
                    }
                }
                out.push(s);
                continue;
            }
        }
        // Ollama JSON.
        if let Some(body) = http_get(port, "/api/ps", timeout) {
            if body.contains("\"models\"") {
                let models = parse_ollama_ps(&body);
                if let Some(m) = models.into_iter().max_by_key(|m| m.size) {
                    let offload = (m.size > 0).then(|| m.size_vram as f64 / m.size as f64 * 100.0);
                    out.push(ServerStats {
                        runtime: "Ollama",
                        pid,
                        port,
                        model: m.name,
                        gpu_offload_pct: offload,
                        ..Default::default()
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_servers_reason_is_platform_honest() {
        let msg = no_servers_reason();
        assert!(!msg.is_empty());
        // Off Linux, discovery doesn't run — the message must say so and point
        // at the tracking issue rather than implying nothing is running.
        #[cfg(not(target_os = "linux"))]
        {
            assert!(msg.contains("Linux-only"));
            assert!(msg.contains("#13"));
        }
    }

    #[test]
    fn splits_and_dechunks() {
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello world";
        let (h, b) = split_response(resp).unwrap();
        assert!(h.contains("200 OK"));
        assert_eq!(b, "hello world");
        // chunked: "4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"
        assert_eq!(dechunk("4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), "Wikipedia");
        // non-chunked passthrough
        assert_eq!(dechunk("plain body"), "plain body");
    }

    #[test]
    fn prometheus_sum_and_label() {
        let text = "# HELP x\nfoo 1.5\nfoo 2.5\nbar{model_name=\"llama3\"} 7\nfoobar 99\n";
        assert_eq!(prom_sum(text, "foo"), Some(4.0)); // sums series, ignores foobar
        assert_eq!(prom_sum(text, "missing"), None);
        assert_eq!(
            prom_label(text, "bar", "model_name").as_deref(),
            Some("llama3")
        );
    }

    #[test]
    fn llama_cpp_stats() {
        let text = "llamacpp:predicted_tokens_seconds 42.7\n\
                    llamacpp:prompt_tokens_seconds 530.0\n\
                    llamacpp:kv_cache_usage_ratio 0.25\n\
                    llamacpp:requests_processing 1\n\
                    llamacpp:requests_deferred 0\n";
        let s = stats_from_prometheus(text, 10, 8080).unwrap();
        assert_eq!(s.runtime, "llama.cpp");
        assert_eq!(s.gen_tps, Some(42.7));
        assert_eq!(s.kv_pct, Some(25.0));
        assert_eq!(s.running, Some(1.0));
    }

    #[test]
    fn vllm_stats_identifies() {
        let text = "vllm:num_requests_running 3\n\
                    vllm:num_requests_waiting 5\n\
                    vllm:gpu_cache_usage_perc 0.8\n\
                    vllm:generation_tokens_total{model_name=\"x\"} 1000\n";
        let s = stats_from_prometheus(text, 11, 8000).unwrap();
        assert_eq!(s.runtime, "vLLM");
        assert_eq!(s.running, Some(3.0));
        assert_eq!(s.kv_pct.map(|v| v.round()), Some(80.0));
    }

    #[test]
    fn ollama_ps_parse() {
        let json = r#"{"models":[{"name":"llama3:8b","model":"llama3","size":5400000000,"size_vram":5400000000,"details":{"parameter_size":"8.0B"}},{"name":"qwen:0.5b","size":400000000,"size_vram":200000000}]}"#;
        let m = parse_ollama_ps(json);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "llama3:8b");
        assert_eq!(m[0].size_vram, 5400000000);
        assert_eq!(m[1].size_vram, 200000000);
    }

    #[test]
    fn counter_rate_works() {
        let mut prev = HashMap::new();
        let t0 = Instant::now();
        assert_eq!(counter_rate(&mut prev, (1, 2, "gen"), 100.0, t0), None);
        let t1 = t0 + Duration::from_secs(2);
        assert_eq!(
            counter_rate(&mut prev, (1, 2, "gen"), 200.0, t1),
            Some(50.0)
        );
    }
}
