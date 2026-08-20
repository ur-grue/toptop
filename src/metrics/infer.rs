//! Inference-server auto-discovery and metrics scraping.
//!
//! toptop already knows which processes are inference runtimes and which ports
//! they listen on, so it can do something `nvidia-smi` can't: find your local
//! servers and scrape the numbers you actually watch — **tokens/sec**, KV-cache
//! pressure, queue depth, TTFT — straight from their metrics endpoints.
//!
//! Supported: llama.cpp / vLLM / TGI / TensorRT-LLM Prometheus `/metrics`,
//! Ollama's `/api/ps` JSON, and LM Studio's `/api/v0/models` JSON. The HTTP
//! client is a tiny hand-rolled localhost GET (no
//! dependencies); scraping runs on a background thread so it never blocks the
//! UI. The parsers are pure and unit-tested; discovery degrades to nothing when
//! no server is listening.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
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
    /// Time-to-first-token percentiles, milliseconds (from the histogram).
    pub ttft: Option<Percentiles>,
    /// Time-per-output-token percentiles, milliseconds — the inter-token
    /// latency users actually feel while a response streams.
    pub tpot: Option<Percentiles>,
    /// Ollama only: share of the model resident on GPU, 0–100%.
    pub gpu_offload_pct: Option<f64>,
    /// Set for `--llm-server` targets: the `host:port` they were configured
    /// as. Manual targets have no local process, so this replaces the PID as
    /// the label.
    pub addr: Option<String>,
    /// Cumulative requests preempted because the KV cache ran out. When a
    /// server preempts, the work already done on that request is thrown away
    /// and recomputed later — throughput collapses while every dashboard still
    /// shows a healthy-looking GPU. This is the number that explains it.
    pub preemptions: Option<f64>,
    /// Preemptions per second, differenced from the counter. Anything above
    /// zero is worth acting on.
    pub preempt_rate: Option<f64>,
}

/// The p50/p95/p99 triad of a latency distribution, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

impl ServerStats {
    /// Label for this server in the UI: the configured address for manual
    /// targets, `runtime:port` for discovered ones.
    pub fn label(&self) -> String {
        match &self.addr {
            Some(a) => format!("{} {}", self.runtime, a),
            None => format!("{}:{}", self.runtime, self.port),
        }
    }

    /// Share of processed tokens that were prefill, 0–100%, when both phases
    /// are known. This is a workload *mix*, not a verdict on the bottleneck —
    /// see `tpot` for whether decode latency is actually hurting.
    pub fn prefill_share_pct(&self) -> Option<f64> {
        let (p, g) = (self.prompt_tps?, self.gen_tps?);
        let total = p + g;
        (total > 0.0).then(|| p / total * 100.0)
    }
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

/// Like [`prom_sum`], but only over series whose label block contains
/// `label_frag` (e.g. `method="prefill"`). Returns `None` if none matched.
pub fn prom_sum_with_label(text: &str, name: &str, label_frag: &str) -> Option<f64> {
    let mut found = false;
    let mut total = 0.0;
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with(name) {
            continue;
        }
        let rest = &line[name.len()..];
        if !rest.starts_with('{') {
            continue;
        }
        let Some(close) = rest.find('}') else {
            continue;
        };
        if !rest[..close].contains(label_frag) {
            continue;
        }
        if let Some(tok) = rest[close + 1..].split_whitespace().next() {
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

/// A parsed Prometheus histogram: cumulative bucket counts by upper bound.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    /// `(le, cumulative_count)`, sorted ascending by `le`. `+Inf` is stored as
    /// `f64::INFINITY`.
    pub buckets: Vec<(f64, f64)>,
}

/// Parse `<metric>_bucket{le="…"}` series into a [`Histogram`].
///
/// Buckets from multiple label sets (several models on one server) are summed,
/// which is what a server-wide percentile should be over.
pub fn prom_histogram(text: &str, metric: &str) -> Option<Histogram> {
    let bucket_name = format!("{metric}_bucket");
    let mut by_le: std::collections::BTreeMap<u64, (f64, f64)> = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with(&bucket_name) {
            continue;
        }
        let rest = &line[bucket_name.len()..];
        if !rest.starts_with('{') {
            continue;
        }
        let Some(close) = rest.find('}') else {
            continue;
        };
        let labels = &rest[1..close];
        let Some(pos) = labels.find("le=\"") else {
            continue;
        };
        let after = &labels[pos + 4..];
        let Some(end) = after.find('"') else { continue };
        let le_str = &after[..end];
        let le = if le_str == "+Inf" {
            f64::INFINITY
        } else {
            match le_str.parse::<f64>() {
                Ok(v) => v,
                Err(_) => continue,
            }
        };
        let Some(tok) = rest[close + 1..].split_whitespace().next() else {
            continue;
        };
        let Ok(count) = tok.parse::<f64>() else {
            continue;
        };
        // BTreeMap keyed by the bit pattern keeps ordering without needing Ord
        // on f64; the stored `le` is the real value.
        let key = le.to_bits();
        let slot = by_le.entry(key).or_insert((le, 0.0));
        slot.1 += count;
    }
    if by_le.is_empty() {
        return None;
    }
    let mut buckets: Vec<(f64, f64)> = by_le.into_values().collect();
    buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Some(Histogram { buckets })
}

impl Histogram {
    /// Total observations (the `+Inf` bucket, i.e. the last cumulative count).
    pub fn count(&self) -> f64 {
        self.buckets.last().map(|(_, c)| *c).unwrap_or(0.0)
    }

    /// Estimate a quantile, interpolating linearly inside the containing
    /// bucket — the same approach as Prometheus' `histogram_quantile`.
    ///
    /// Returns `None` for an empty histogram, or when the quantile falls in the
    /// open-ended `+Inf` bucket where no upper bound exists to interpolate to.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        let total = self.count();
        if total <= 0.0 || self.buckets.is_empty() {
            return None;
        }
        let target = q.clamp(0.0, 1.0) * total;
        let mut prev_le = 0.0;
        let mut prev_count = 0.0;
        for (le, count) in &self.buckets {
            if *count >= target {
                if le.is_infinite() {
                    // Everything above the largest finite bound: the best
                    // honest answer is that bound, not an invented number.
                    return (prev_le > 0.0).then_some(prev_le);
                }
                let span = count - prev_count;
                if span <= 0.0 {
                    return Some(*le);
                }
                let frac = (target - prev_count) / span;
                return Some(prev_le + (le - prev_le) * frac);
            }
            prev_le = if le.is_infinite() { prev_le } else { *le };
            prev_count = *count;
        }
        self.buckets
            .last()
            .map(|(le, _)| *le)
            .filter(|v| v.is_finite())
    }
}

/// The p50/p95/p99 triad of a `<metric>` histogram, converted to milliseconds.
/// Returns `None` unless at least p50 could be derived.
fn percentiles_ms(text: &str, metric: &str) -> Option<Percentiles> {
    let h = prom_histogram(text, metric)?;
    let p50 = h.quantile(0.50)?;
    Some(Percentiles {
        p50: p50 * 1000.0,
        p95: h.quantile(0.95).unwrap_or(p50) * 1000.0,
        p99: h.quantile(0.99).unwrap_or(p50) * 1000.0,
    })
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

/// Parse LM Studio's `/api/v0/models` response: the id of the first model in
/// `"state":"loaded"`, if any. Objects look like
/// `{"id":"qwen2-vl-7b-instruct", …, "state":"loaded", …}`.
pub fn parse_lmstudio_loaded_model(json: &str) -> Option<String> {
    let mut idx = 0;
    while let Some(rel) = json[idx..].find("\"id\":\"") {
        let istart = idx + rel + "\"id\":\"".len();
        let iend = json[istart..].find('"')?;
        let id = &json[istart..istart + iend];
        // Bound the state search to this object (before the next "id").
        let next = json[istart..]
            .find("\"id\":\"")
            .map(|p| istart + p)
            .unwrap_or(json.len());
        if json[istart..next].contains("\"state\":\"loaded\"") {
            return Some(id.to_string());
        }
        idx = istart + iend;
    }
    None
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
        // vLLM renamed this counter across versions; both spellings mean the
        // same thing, and a server that has never preempted omits it entirely.
        s.preemptions = prom_sum(text, "vllm:num_preemptions_total")
            .or_else(|| prom_sum(text, "vllm:num_preemptions"));
        if let (Some(sum), Some(cnt)) = (
            prom_sum(text, "vllm:time_to_first_token_seconds_sum"),
            prom_sum(text, "vllm:time_to_first_token_seconds_count"),
        ) {
            if cnt > 0.0 {
                s.ttft_ms = Some(sum / cnt * 1000.0);
            }
        }
        s.ttft = percentiles_ms(text, "vllm:time_to_first_token_seconds");
        s.tpot = percentiles_ms(text, "vllm:time_per_output_token_seconds");
    } else if text.contains("trtllm_") {
        s.runtime = "TensorRT-LLM";
        s.kv_pct = prom_sum(text, "trtllm_kv_cache_utilization").map(|v| v * 100.0);
        s.running = prom_sum(text, "trtllm_num_requests_running");
        s.waiting = prom_sum(text, "trtllm_num_requests_waiting");
        s.model = prom_label(text, "trtllm_", "model_name").unwrap_or_default();
        s.preemptions = prom_sum(text, "trtllm_num_preemptions_total");
        if let (Some(sum), Some(cnt)) = (
            prom_sum(text, "trtllm_time_to_first_token_seconds_sum"),
            prom_sum(text, "trtllm_time_to_first_token_seconds_count"),
        ) {
            if cnt > 0.0 {
                s.ttft_ms = Some(sum / cnt * 1000.0);
            }
        }
        s.ttft = percentiles_ms(text, "trtllm_time_to_first_token_seconds");
        s.tpot = percentiles_ms(text, "trtllm_time_per_output_token_seconds");
    } else if text.contains("sglang:") {
        s.runtime = "SGLang";
        // SGLang reports KV usage as `token_usage` (0–1 of the token budget).
        s.kv_pct = prom_sum(text, "sglang:token_usage").map(|v| v * 100.0);
        s.running = prom_sum(text, "sglang:num_running_reqs");
        s.waiting = prom_sum(text, "sglang:num_queue_reqs");
        s.model = prom_label(text, "sglang:", "model_name").unwrap_or_default();
        // `gen_throughput` is already a rate, so it needs no counter delta.
        s.gen_tps = prom_sum(text, "sglang:gen_throughput");
        if let (Some(sum), Some(cnt)) = (
            prom_sum(text, "sglang:time_to_first_token_seconds_sum"),
            prom_sum(text, "sglang:time_to_first_token_seconds_count"),
        ) {
            if cnt > 0.0 {
                s.ttft_ms = Some(sum / cnt * 1000.0);
            }
        }
        s.ttft = percentiles_ms(text, "sglang:time_to_first_token_seconds");
        // SGLang has renamed this metric across versions; try both spellings.
        s.tpot = percentiles_ms(text, "sglang:time_per_output_token_seconds")
            .or_else(|| percentiles_ms(text, "sglang:inter_token_latency_seconds"));
    } else if text.contains("tgi_") {
        s.runtime = "TGI";
        s.running = prom_sum(text, "tgi_batch_current_size");
        s.waiting = prom_sum(text, "tgi_queue_size");
        // TGI exposes no direct TTFT histogram, but TTFT decomposes exactly
        // into its per-request pipeline stages: validation, queue wait, then
        // the prefill forward that emits the first token.
        let mean = |metric: &str| {
            let sum = prom_sum(text, &format!("{metric}_sum"))?;
            let cnt = prom_sum(text, &format!("{metric}_count"))?;
            (cnt > 0.0).then(|| sum / cnt)
        };
        let prefill_mean = (|| {
            let sum = prom_sum_with_label(
                text,
                "tgi_batch_inference_duration_sum",
                "method=\"prefill\"",
            )?;
            let cnt = prom_sum_with_label(
                text,
                "tgi_batch_inference_duration_count",
                "method=\"prefill\"",
            )?;
            (cnt > 0.0).then(|| sum / cnt)
        })();
        if let (Some(queue), Some(prefill)) = (mean("tgi_request_queue_duration"), prefill_mean) {
            let validation = mean("tgi_request_validation_duration").unwrap_or(0.0);
            s.ttft_ms = Some((validation + queue + prefill) * 1000.0);
        }
        // TGI has no TTFT histogram to take percentiles from, but it does
        // expose per-token latency directly.
        s.tpot = percentiles_ms(text, "tgi_request_mean_time_per_token_duration");
    } else {
        return None;
    }
    Some(s)
}

/// A manually configured inference-server target (`--llm-server host:port`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
}

impl Target {
    pub fn label(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Parse a `host:port` target.
///
/// Accepts names and IPv4 literals as `host:port`, and IPv6 literals in the
/// usual bracketed form `[::1]:8000` — an unbracketed IPv6 address is
/// ambiguous with the port separator and is rejected rather than guessed at.
pub fn parse_target(spec: &str) -> Result<Target, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty target".to_string());
    }
    let (host, port_str) = if let Some(rest) = spec.strip_prefix('[') {
        let (h, r) = rest
            .split_once(']')
            .ok_or_else(|| format!("'{spec}': unclosed '[' in IPv6 address"))?;
        let p = r
            .strip_prefix(':')
            .ok_or_else(|| format!("'{spec}': missing :port after ']'"))?;
        (h.to_string(), p)
    } else {
        let (h, p) = spec
            .rsplit_once(':')
            .ok_or_else(|| format!("'{spec}': expected host:port"))?;
        if h.contains(':') {
            return Err(format!("'{spec}': bracket IPv6 addresses, e.g. [::1]:8000"));
        }
        (h.to_string(), p)
    };
    if host.is_empty() {
        return Err(format!("'{spec}': empty host"));
    }
    let port: u16 = port_str
        .parse()
        .map_err(|_| format!("'{spec}': '{port_str}' is not a port"))?;
    if port == 0 {
        return Err(format!("'{spec}': port must not be 0"));
    }
    Ok(Target { host, port })
}

// ── HTTP ───────────────────────────────────────────────────────────────────

/// Host used for auto-discovered servers, which are local by definition.
const LOCALHOST: &str = "127.0.0.1";

/// GET `path` from `host:port`. `host` is resolved through the system
/// resolver, so `--llm-server` targets can be names as well as addresses;
/// auto-discovered servers always pass `127.0.0.1`.
fn http_get(host: &str, port: u16, path: &str, timeout: Duration) -> Option<String> {
    use std::net::ToSocketAddrs;
    // Resolution can block, so it happens on the scraper thread like the rest.
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: toptop\r\nConnection: close\r\n\r\n"
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
        Self::with_targets(Vec::new())
    }

    /// Start the poller with additional manually configured targets, scraped
    /// alongside whatever auto-discovery finds.
    pub fn with_targets(targets: Vec<Target>) -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&latest);
        std::thread::Builder::new()
            .name("toptop-infer".into())
            .spawn(move || {
                // prev counter values for rate (tokens/sec) computation.
                let mut prev: HashMap<(u32, u16, &'static str), (f64, Instant)> = HashMap::new();
                loop {
                    let servers = scrape_once(&mut prev, &targets);
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

/// Layer tokens/sec onto `s` by differencing the runtime's cumulative token
/// counters against the previous scrape. Runtimes that publish a rate directly
/// (SGLang, llama.cpp) already filled these in and are left alone.
fn apply_counter_rates(
    s: &mut ServerStats,
    body: &str,
    prev: &mut HashMap<(u32, u16, &'static str), (f64, Instant)>,
    pid: u32,
    port: u16,
    now: Instant,
) {
    let (gen_metric, prompt_metric) = match s.runtime {
        "vLLM" => ("vllm:generation_tokens_total", "vllm:prompt_tokens_total"),
        "TensorRT-LLM" => (
            "trtllm_generation_tokens_total",
            "trtllm_prompt_tokens_total",
        ),
        // TGI has no tokens-total counters; its per-request histogram sums are
        // cumulative and increment as requests finish, so their rate is
        // throughput (slightly bursty).
        "TGI" => (
            "tgi_request_generated_tokens_sum",
            "tgi_request_input_length_sum",
        ),
        "SGLang" => (
            "sglang:generation_tokens_total",
            "sglang:prompt_tokens_total",
        ),
        _ => return,
    };
    if let Some(g) = prom_sum(body, gen_metric) {
        // SGLang publishes `gen_throughput` directly; only fall back to the
        // counter rate when that gauge was absent.
        if let Some(rate) = counter_rate(prev, (pid, port, "gen"), g, now) {
            if s.gen_tps.is_none() {
                s.gen_tps = Some(rate);
            }
        }
    }
    // Preemption rate: the counter is cumulative, and it is the *rate* that
    // says whether the server is thrashing right now.
    if let Some(preempt) = s.preemptions {
        s.preempt_rate = counter_rate(prev, (pid, port, "preempt"), preempt, now);
    }
    if let Some(p) = prom_sum(body, prompt_metric) {
        if let Some(rate) = counter_rate(prev, (pid, port, "prompt"), p, now) {
            if s.prompt_tps.is_none() {
                s.prompt_tps = Some(rate);
            }
        }
    }
}

fn scrape_once(
    prev: &mut HashMap<(u32, u16, &'static str), (f64, Instant)>,
    targets: &[Target],
) -> Vec<ServerStats> {
    // Manual targets may be across the network, so they get a longer budget
    // than the localhost sweep.
    let timeout = Duration::from_millis(250);
    let remote_timeout = Duration::from_millis(800);
    let now = Instant::now();
    let mut out: Vec<ServerStats> = Vec::new();

    // Manual targets first: they were asked for explicitly, and they have no
    // PID to match, so they never collide with the discovery pass below.
    for (i, t) in targets.iter().enumerate() {
        let Some(body) = http_get(&t.host, t.port, "/metrics", remote_timeout) else {
            continue;
        };
        // Manual targets have no local PID; the index keeps counter-rate keys
        // distinct between targets without pretending to be a process id.
        let key_pid = u32::MAX - i as u32;
        if let Some(mut s) = stats_from_prometheus(&body, 0, t.port) {
            s.addr = Some(t.label());
            apply_counter_rates(&mut s, &body, prev, key_pid, t.port, now);
            out.push(s);
        }
    }
    for (port, pid) in discover_servers() {
        if out.iter().any(|s| s.pid == pid) {
            continue; // one server per process
        }
        // Prometheus endpoints (llama.cpp / vLLM / TGI).
        if let Some(body) = http_get(LOCALHOST, port, "/metrics", timeout) {
            if let Some(mut s) = stats_from_prometheus(&body, pid, port) {
                apply_counter_rates(&mut s, &body, prev, pid, port, now);
                out.push(s);
                continue;
            }
        }
        // Ollama JSON.
        if let Some(body) = http_get(LOCALHOST, port, "/api/ps", timeout) {
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
                    continue;
                }
            }
        }
        // LM Studio JSON (no Prometheus endpoint; /api/v0/models lists
        // models with a "state" field marking the loaded one).
        if let Some(body) = http_get(LOCALHOST, port, "/api/v0/models", timeout) {
            if body.contains("\"state\"") {
                out.push(ServerStats {
                    runtime: "LM Studio",
                    pid,
                    port,
                    model: parse_lmstudio_loaded_model(&body).unwrap_or_default(),
                    ..Default::default()
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_quantiles_interpolate_within_buckets() {
        // 100 observations: 50 ≤ 0.1s, 90 ≤ 0.5s, 99 ≤ 1s, 100 ≤ +Inf.
        let text = "\
foo_bucket{le=\"0.1\"} 50
foo_bucket{le=\"0.5\"} 90
foo_bucket{le=\"1\"} 99
foo_bucket{le=\"+Inf\"} 100
";
        let h = prom_histogram(text, "foo").expect("parsed");
        assert_eq!(h.count(), 100.0);
        // p50 lands exactly on the 0.1 boundary.
        assert!((h.quantile(0.50).unwrap() - 0.1).abs() < 1e-9);
        // p95 sits between 0.5 and 1: 5 of the 9 observations in that bucket.
        let p95 = h.quantile(0.95).unwrap();
        assert!(p95 > 0.5 && p95 < 1.0, "p95 = {p95}");
        assert!((p95 - (0.5 + 0.5 * (95.0 - 90.0) / 9.0)).abs() < 1e-9);
    }

    #[test]
    fn histogram_edge_cases() {
        // Empty and absent histograms yield nothing rather than a bogus zero.
        assert_eq!(prom_histogram("nothing here", "foo"), None);
        let empty = prom_histogram("foo_bucket{le=\"+Inf\"} 0\n", "foo").unwrap();
        assert_eq!(empty.quantile(0.5), None);

        // A quantile falling in the open-ended +Inf bucket has no upper bound
        // to interpolate to — report the largest finite bound, never a guess.
        let long_tail = prom_histogram(
            "foo_bucket{le=\"1\"} 90\nfoo_bucket{le=\"+Inf\"} 100\n",
            "foo",
        )
        .unwrap();
        assert_eq!(long_tail.quantile(0.99), Some(1.0));

        // Buckets from several label sets are summed into one distribution.
        let multi = prom_histogram(
            "foo_bucket{model=\"a\",le=\"1\"} 5\nfoo_bucket{model=\"b\",le=\"1\"} 7\n",
            "foo",
        )
        .unwrap();
        assert_eq!(multi.buckets, vec![(1.0, 12.0)]);
    }

    #[test]
    fn vllm_slo_triad_from_histograms() {
        let text = "\
vllm:num_requests_running 2
vllm:gpu_cache_usage_perc 0.42
vllm:time_to_first_token_seconds_bucket{le=\"0.1\"} 40
vllm:time_to_first_token_seconds_bucket{le=\"0.5\"} 95
vllm:time_to_first_token_seconds_bucket{le=\"+Inf\"} 100
vllm:time_per_output_token_seconds_bucket{le=\"0.01\"} 60
vllm:time_per_output_token_seconds_bucket{le=\"0.05\"} 98
vllm:time_per_output_token_seconds_bucket{le=\"+Inf\"} 100
";
        let s = stats_from_prometheus(text, 1, 8000).expect("vLLM detected");
        assert_eq!(s.runtime, "vLLM");
        let ttft = s.ttft.expect("ttft percentiles");
        // p50 falls in the 0.1–0.5 bucket → between 100ms and 500ms.
        assert!(ttft.p50 > 100.0 && ttft.p50 < 500.0, "p50 = {}", ttft.p50);
        assert!(ttft.p95 >= ttft.p50);
        let tpot = s.tpot.expect("tpot percentiles");
        assert!(tpot.p50 > 0.0 && tpot.p50 < 50.0, "p50 = {}", tpot.p50);
        assert!(tpot.p99 >= tpot.p95);
    }

    #[test]
    fn sglang_metrics_are_parsed() {
        // NOTE: synthesized from SGLang's documented metric names, not a real
        // scrape — see the PR description.
        let text = "\
# HELP sglang:num_running_reqs The number of running requests.
sglang:num_running_reqs{model_name=\"Qwen2-7B\"} 3
sglang:num_queue_reqs{model_name=\"Qwen2-7B\"} 5
sglang:token_usage{model_name=\"Qwen2-7B\"} 0.87
sglang:gen_throughput{model_name=\"Qwen2-7B\"} 142.5
sglang:time_to_first_token_seconds_sum{model_name=\"Qwen2-7B\"} 12.0
sglang:time_to_first_token_seconds_count{model_name=\"Qwen2-7B\"} 40
sglang:time_to_first_token_seconds_bucket{model_name=\"Qwen2-7B\",le=\"0.5\"} 30
sglang:time_to_first_token_seconds_bucket{model_name=\"Qwen2-7B\",le=\"+Inf\"} 40
";
        let s = stats_from_prometheus(text, 42, 30000).expect("SGLang detected");
        assert_eq!(s.runtime, "SGLang");
        assert_eq!(s.model, "Qwen2-7B");
        assert_eq!(s.running, Some(3.0));
        assert_eq!(s.waiting, Some(5.0));
        assert_eq!(s.kv_pct, Some(87.0));
        // gen_throughput is already a rate — no counter differencing needed.
        assert_eq!(s.gen_tps, Some(142.5));
        assert_eq!(s.ttft_ms, Some(300.0));
        assert!(s.ttft.is_some());
    }

    #[test]
    fn vllm_preemption_counter_is_parsed() {
        let text = "\
vllm:num_requests_running 4
vllm:gpu_cache_usage_perc 0.99
vllm:num_preemptions_total{model_name=\"Llama-3-8B\"} 137
";
        let s = stats_from_prometheus(text, 1, 8000).expect("vLLM");
        assert_eq!(s.preemptions, Some(137.0));
        // The rate needs two scrapes; a single parse has none yet.
        assert_eq!(s.preempt_rate, None);

        // The older spelling is accepted too.
        let old = stats_from_prometheus("vllm:num_preemptions 5\n", 1, 8000).expect("vLLM");
        assert_eq!(old.preemptions, Some(5.0));

        // A server that has never preempted omits the counter entirely, which
        // must read as "no data", not as zero preemptions.
        let none = stats_from_prometheus("vllm:num_requests_running 1\n", 1, 8000).unwrap();
        assert_eq!(none.preemptions, None);
    }

    #[test]
    fn preemption_rate_is_differenced_across_scrapes() {
        let mut prev = HashMap::new();
        let t0 = Instant::now();
        let mut s = ServerStats {
            runtime: "vLLM",
            preemptions: Some(100.0),
            ..Default::default()
        };
        apply_counter_rates(&mut s, "", &mut prev, 1, 8000, t0);
        assert_eq!(s.preempt_rate, None, "first scrape has no baseline");

        let mut s2 = ServerStats {
            runtime: "vLLM",
            preemptions: Some(104.0),
            ..Default::default()
        };
        apply_counter_rates(&mut s2, "", &mut prev, 1, 8000, t0 + Duration::from_secs(2));
        assert_eq!(s2.preempt_rate, Some(2.0), "4 preemptions over 2s");

        // A counter that goes backwards (server restarted) must not produce a
        // negative rate — it produces none until a new baseline exists.
        let mut s3 = ServerStats {
            runtime: "vLLM",
            preemptions: Some(1.0),
            ..Default::default()
        };
        apply_counter_rates(&mut s3, "", &mut prev, 1, 8000, t0 + Duration::from_secs(4));
        assert_eq!(s3.preempt_rate, None, "a counter reset is not a rate");
    }

    #[test]
    fn target_parsing() {
        assert_eq!(
            parse_target("localhost:8000"),
            Ok(Target {
                host: "localhost".into(),
                port: 8000
            })
        );
        assert_eq!(
            parse_target(" 10.0.0.5:11434 "),
            Ok(Target {
                host: "10.0.0.5".into(),
                port: 11434
            })
        );
        assert_eq!(
            parse_target("[::1]:8000"),
            Ok(Target {
                host: "::1".into(),
                port: 8000
            })
        );
        // An unbracketed IPv6 address is ambiguous with the port separator.
        assert!(parse_target("::1:8000").unwrap_err().contains("bracket"));
        assert!(parse_target("nohost").unwrap_err().contains("host:port"));
        assert!(parse_target("host:99999")
            .unwrap_err()
            .contains("not a port"));
        assert!(parse_target("host:0")
            .unwrap_err()
            .contains("must not be 0"));
        assert!(parse_target(":8000").unwrap_err().contains("empty host"));
        assert!(parse_target("").unwrap_err().contains("empty"));
    }

    #[test]
    fn manual_target_labels_replace_the_port_label() {
        let s = ServerStats {
            runtime: "vLLM",
            port: 8000,
            addr: Some("gpu-box:8000".into()),
            ..Default::default()
        };
        assert_eq!(s.label(), "vLLM gpu-box:8000");
        let auto = ServerStats {
            runtime: "vLLM",
            port: 8000,
            ..Default::default()
        };
        assert_eq!(auto.label(), "vLLM:8000");
    }

    #[test]
    fn prefill_share_needs_both_phases() {
        let mut s = ServerStats {
            gen_tps: Some(75.0),
            prompt_tps: Some(25.0),
            ..Default::default()
        };
        assert_eq!(s.prefill_share_pct(), Some(25.0));
        s.prompt_tps = None;
        assert_eq!(s.prefill_share_pct(), None);
        // Idle server: no division by zero.
        let idle = ServerStats {
            gen_tps: Some(0.0),
            prompt_tps: Some(0.0),
            ..Default::default()
        };
        assert_eq!(idle.prefill_share_pct(), None);
    }

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

    /// Metric names and shapes as emitted by TGI's router (see
    /// text-generation-inference `router/src/server.rs`).
    const TGI_PAYLOAD: &str = "\
        # TYPE tgi_queue_size gauge\n\
        tgi_queue_size 2\n\
        # TYPE tgi_batch_current_size gauge\n\
        tgi_batch_current_size 1\n\
        # TYPE tgi_request_validation_duration histogram\n\
        tgi_request_validation_duration_sum 0.05\n\
        tgi_request_validation_duration_count 10\n\
        # TYPE tgi_request_queue_duration histogram\n\
        tgi_request_queue_duration_sum 1.2\n\
        tgi_request_queue_duration_count 10\n\
        # TYPE tgi_batch_inference_duration histogram\n\
        tgi_batch_inference_duration_sum{method=\"prefill\"} 0.9\n\
        tgi_batch_inference_duration_count{method=\"prefill\"} 10\n\
        tgi_batch_inference_duration_sum{method=\"decode\"} 30.0\n\
        tgi_batch_inference_duration_count{method=\"decode\"} 500\n\
        # TYPE tgi_request_generated_tokens histogram\n\
        tgi_request_generated_tokens_sum 1000\n\
        tgi_request_generated_tokens_count 10\n\
        # TYPE tgi_request_input_length histogram\n\
        tgi_request_input_length_sum 5000\n\
        tgi_request_input_length_count 10\n";

    #[test]
    fn tgi_stats_with_ttft() {
        let s = stats_from_prometheus(TGI_PAYLOAD, 12, 3000).unwrap();
        assert_eq!(s.runtime, "TGI");
        assert_eq!(s.running, Some(1.0));
        assert_eq!(s.waiting, Some(2.0));
        // TTFT = mean validation + mean queue + mean prefill
        //      = (0.05/10 + 1.2/10 + 0.9/10) s = 215 ms. Decode must not leak in.
        assert_eq!(s.ttft_ms.map(|v| v.round()), Some(215.0));
    }

    #[test]
    fn tgi_ttft_absent_without_prefill_series() {
        // Queue histogram alone isn't enough to claim a TTFT.
        let text = "tgi_queue_size 0\n\
                    tgi_request_queue_duration_sum 1.0\n\
                    tgi_request_queue_duration_count 10\n";
        let s = stats_from_prometheus(text, 12, 3000).unwrap();
        assert_eq!(s.runtime, "TGI");
        assert_eq!(s.ttft_ms, None);
    }

    #[test]
    fn prom_sum_with_label_filters_series() {
        assert_eq!(
            prom_sum_with_label(
                TGI_PAYLOAD,
                "tgi_batch_inference_duration_sum",
                "method=\"prefill\""
            ),
            Some(0.9)
        );
        // Unlabeled series never match a label filter.
        assert_eq!(
            prom_sum_with_label(TGI_PAYLOAD, "tgi_queue_size", "method=\"prefill\""),
            None
        );
    }

    #[test]
    fn tgi_token_counters_feed_the_rate_calc() {
        // The cumulative histogram sums act as counters for tokens/sec.
        let mut prev = HashMap::new();
        let t0 = Instant::now();
        let g0 = prom_sum(TGI_PAYLOAD, "tgi_request_generated_tokens_sum").unwrap();
        assert_eq!(counter_rate(&mut prev, (1, 3000, "gen"), g0, t0), None);
        // 500 more tokens completed over 2s → 250 tok/s.
        let t1 = t0 + Duration::from_secs(2);
        assert_eq!(
            counter_rate(&mut prev, (1, 3000, "gen"), g0 + 500.0, t1),
            Some(250.0)
        );
    }

    #[test]
    fn trtllm_stats() {
        // Names as emitted by TensorRT-LLM's MetricsCollector
        // (tensorrt_llm/metrics/collector.py).
        let text = "trtllm_kv_cache_utilization{model_name=\"nemotron-nano-3\"} 0.42\n\
                    trtllm_num_requests_running{model_name=\"nemotron-nano-3\"} 3\n\
                    trtllm_num_requests_waiting{model_name=\"nemotron-nano-3\"} 1\n\
                    trtllm_time_to_first_token_seconds_sum{model_name=\"nemotron-nano-3\"} 4.0\n\
                    trtllm_time_to_first_token_seconds_count{model_name=\"nemotron-nano-3\"} 20\n\
                    trtllm_generation_tokens_total{model_name=\"nemotron-nano-3\"} 12345\n";
        let s = stats_from_prometheus(text, 13, 8000).unwrap();
        assert_eq!(s.runtime, "TensorRT-LLM");
        assert_eq!(s.kv_pct.map(|v| v.round()), Some(42.0));
        assert_eq!(s.running, Some(3.0));
        assert_eq!(s.waiting, Some(1.0));
        assert_eq!(s.model, "nemotron-nano-3");
        assert_eq!(s.ttft_ms, Some(200.0));
    }

    #[test]
    fn lmstudio_models_parse() {
        // Shape from LM Studio's REST docs (GET /api/v0/models).
        let json = r#"{"object":"list","data":[
            {"id":"qwen2-vl-7b-instruct","object":"model","type":"vlm","state":"not-loaded","max_context_length":32768},
            {"id":"llama-3.2-3b-instruct","object":"model","type":"llm","state":"loaded","max_context_length":131072}]}"#;
        assert_eq!(
            parse_lmstudio_loaded_model(json).as_deref(),
            Some("llama-3.2-3b-instruct")
        );
        let none_loaded =
            r#"{"data":[{"id":"a","state":"not-loaded"},{"id":"b","state":"not-loaded"}]}"#;
        assert_eq!(parse_lmstudio_loaded_model(none_loaded), None);
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
