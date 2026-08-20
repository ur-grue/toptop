//! OpenTelemetry (OTLP/HTTP) metrics export.
//!
//! `--serve-metrics` is a *pull* endpoint: Prometheus scrapes toptop. Teams
//! with an OTel pipeline want the opposite — toptop pushing into a collector,
//! which then fans out to whatever backend they run. This module speaks
//! OTLP/HTTP with JSON encoding, which is a documented, stable wire format that
//! needs no protobuf toolchain and no dependencies.
//!
//! Building the payload is a pure function of a snapshot plus a timestamp, so
//! the wire format is unit-tested without a collector.
//!
//! **Plain HTTP only.** OTLP collectors are near-universally reached over
//! http:// — a sidecar, a localhost agent, or an in-cluster service — and
//! toptop carries no TLS stack. An https:// endpoint is rejected at startup
//! with a message saying so, rather than failing obscurely every tick.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::metrics::Collector;

/// Where to POST, split into the pieces the request line needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// Path to POST to, defaulting to the OTLP metrics path.
    pub path: String,
}

/// Parse `http://host[:port][/path]` into an [`Endpoint`].
///
/// Defaults follow the OTLP/HTTP spec: port 4318 and `/v1/metrics`.
pub fn parse_endpoint(url: &str) -> Result<Endpoint, String> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("https://") {
        let _ = rest;
        return Err(
            "--otlp needs a plain http:// endpoint; toptop carries no TLS stack. \
             Point it at a local collector (the usual OTel deployment) and let \
             that forward over TLS."
                .to_string(),
        );
    }
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("'{url}': expected an http:// endpoint"))?;
    if rest.is_empty() {
        return Err(format!("'{url}': no host"));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("'{url}': '{p}' is not a port"))?,
        ),
        None => (authority.to_string(), 4318),
    };
    if host.is_empty() {
        return Err(format!("'{url}': no host"));
    }
    Ok(Endpoint {
        host,
        port,
        // An endpoint given without a path means the OTLP default, not "/".
        path: if path.is_empty() || path == "/" {
            "/v1/metrics".to_string()
        } else {
            path
        },
    })
}

/// One gauge data point in the payload.
struct Gauge {
    name: &'static str,
    unit: &'static str,
    description: &'static str,
    value: f64,
    /// Extra attributes as `(key, value)` string pairs.
    attrs: Vec<(String, String)>,
}

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

fn attr_json(key: &str, value: &str) -> String {
    format!(
        "{{\"key\":\"{}\",\"value\":{{\"stringValue\":\"{}\"}}}}",
        esc(key),
        esc(value)
    )
}

/// Render one snapshot as an OTLP/HTTP JSON `ExportMetricsServiceRequest`.
///
/// `now_nanos` is passed in rather than read from the clock so the output is
/// deterministic in tests.
pub fn to_otlp_json(c: &Collector, now_nanos: u128) -> String {
    let mut gauges: Vec<Gauge> = Vec::new();

    gauges.push(Gauge {
        name: "system.cpu.utilization",
        unit: "%",
        description: "Overall CPU utilization.",
        value: c.cpu.global_usage as f64,
        attrs: Vec::new(),
    });
    gauges.push(Gauge {
        name: "system.memory.usage",
        unit: "By",
        description: "Memory in use.",
        value: c.mem.used as f64,
        attrs: Vec::new(),
    });
    gauges.push(Gauge {
        name: "system.memory.limit",
        unit: "By",
        description: "Total memory.",
        value: c.mem.total as f64,
        attrs: Vec::new(),
    });

    for (i, g) in c.gpus.iter().enumerate() {
        let attrs = vec![
            ("gpu.index".to_string(), i.to_string()),
            ("gpu.name".to_string(), g.name.clone()),
        ];
        if g.has_util {
            gauges.push(Gauge {
                name: "gpu.utilization",
                unit: "%",
                description: "GPU core (SM) utilization.",
                value: g.util_pct as f64,
                attrs: attrs.clone(),
            });
        }
        if g.has_mem_util {
            gauges.push(Gauge {
                name: "gpu.memory.bandwidth.utilization",
                unit: "%",
                description: "GPU memory-bandwidth utilization.",
                value: g.mem_util as f64,
                attrs: attrs.clone(),
            });
        }
        if g.mem_total > 0 {
            gauges.push(Gauge {
                name: "gpu.memory.usage",
                unit: "By",
                description: "GPU memory in use.",
                value: g.mem_used as f64,
                attrs: attrs.clone(),
            });
            gauges.push(Gauge {
                name: "gpu.memory.limit",
                unit: "By",
                description: "Total GPU memory.",
                value: g.mem_total as f64,
                attrs: attrs.clone(),
            });
        }
        gauges.push(Gauge {
            name: "gpu.power.usage",
            unit: "W",
            description: "GPU power draw.",
            value: g.power as f64,
            attrs: attrs.clone(),
        });
        gauges.push(Gauge {
            name: "gpu.throttled",
            unit: "1",
            description: "Whether the driver reports an active throttle.",
            value: if g.throttled { 1.0 } else { 0.0 },
            attrs,
        });
    }

    for sv in &c.servers {
        let attrs = vec![
            ("inference.runtime".to_string(), sv.runtime.to_string()),
            ("inference.model".to_string(), sv.model.clone()),
            ("server.port".to_string(), sv.port.to_string()),
        ];
        let mut push =
            |name: &'static str, unit: &'static str, desc: &'static str, v: Option<f64>| {
                if let Some(value) = v {
                    gauges.push(Gauge {
                        name,
                        unit,
                        description: desc,
                        value,
                        attrs: attrs.clone(),
                    });
                }
            };
        push(
            "inference.tokens.generated_rate",
            "1/s",
            "Generation throughput.",
            sv.gen_tps,
        );
        push(
            "inference.tokens.prefill_rate",
            "1/s",
            "Prefill throughput.",
            sv.prompt_tps,
        );
        push(
            "inference.requests.running",
            "1",
            "Active requests.",
            sv.running,
        );
        push(
            "inference.requests.waiting",
            "1",
            "Queued requests.",
            sv.waiting,
        );
        push(
            "inference.kv_cache.utilization",
            "%",
            "KV-cache utilization.",
            sv.kv_pct,
        );
        push(
            "inference.ttft",
            "ms",
            "Mean time to first token.",
            sv.ttft_ms,
        );
        push(
            "inference.preemptions.rate",
            "1/s",
            "KV-cache preemptions per second.",
            sv.preempt_rate,
        );
        push(
            "inference.ttft.p95",
            "ms",
            "Time to first token, 95th percentile.",
            sv.ttft.map(|p| p.p95),
        );
        push(
            "inference.tpot.p95",
            "ms",
            "Time per output token, 95th percentile.",
            sv.tpot.map(|p| p.p95),
        );
    }

    let points: Vec<String> = gauges
        .iter()
        .map(|g| {
            let attrs: Vec<String> = g.attrs.iter().map(|(k, v)| attr_json(k, v)).collect();
            format!(
                "{{\"name\":\"{}\",\"unit\":\"{}\",\"description\":\"{}\",\
                 \"gauge\":{{\"dataPoints\":[{{\"timeUnixNano\":\"{}\",\
                 \"asDouble\":{},\"attributes\":[{}]}}]}}}}",
                g.name,
                g.unit,
                esc(g.description),
                now_nanos,
                if g.value.is_finite() { g.value } else { 0.0 },
                attrs.join(",")
            )
        })
        .collect();

    format!(
        "{{\"resourceMetrics\":[{{\"resource\":{{\"attributes\":[{},{},{}]}},\
         \"scopeMetrics\":[{{\"scope\":{{\"name\":\"toptop\",\"version\":\"{}\"}},\
         \"metrics\":[{}]}}]}}]}}",
        attr_json("host.name", &c.host.hostname),
        attr_json("os.description", &c.host.os),
        attr_json("host.arch", &c.host.arch),
        env!("CARGO_PKG_VERSION"),
        points.join(",")
    )
}

/// POST a body to the endpoint. Returns the HTTP status line on success.
fn post(endpoint: &Endpoint, body: &str, timeout: Duration) -> std::io::Result<String> {
    let addr = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{}:{} did not resolve", endpoint.host, endpoint.port),
            )
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nUser-Agent: toptop\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        body.len(),
        body
    );
    stream.write_all(req.as_bytes())?;
    let mut buf = Vec::with_capacity(1024);
    // Cap the read: a collector's error body is not worth unbounded memory.
    stream.take(64 * 1024).read_to_end(&mut buf)?;
    let resp = String::from_utf8_lossy(&buf);
    Ok(resp.lines().next().unwrap_or("").to_string())
}

/// Push metrics to an OTLP collector until the process is killed.
pub fn run(url: &str, cfg: &Config) -> anyhow::Result<()> {
    let endpoint = parse_endpoint(url).map_err(anyhow::Error::msg)?;
    let interval = Duration::from_millis(cfg.tick_ms.max(1000));
    let mut c = Collector::with_targets(256, cfg.llm_servers.clone());

    eprintln!(
        "toptop: pushing OTLP metrics to http://{}:{}{} every {:?} (Ctrl-C to stop)",
        endpoint.host, endpoint.port, endpoint.path, interval
    );

    // A collector that is down must not take the exporter down with it, but a
    // silent exporter is worse than none — so failures are reported, and
    // repeated identical failures are collapsed into a count.
    let mut last_error: Option<String> = None;
    let mut repeats: u64 = 0;
    loop {
        c.refresh();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let body = to_otlp_json(&c, now);
        match post(&endpoint, &body, Duration::from_secs(5)) {
            Ok(status) if status.contains(" 2") => {
                if repeats > 0 {
                    eprintln!("toptop: OTLP export recovered after {repeats} failure(s)");
                }
                last_error = None;
                repeats = 0;
            }
            Ok(status) => report(
                &mut last_error,
                &mut repeats,
                format!("collector said: {status}"),
            ),
            Err(e) => report(&mut last_error, &mut repeats, e.to_string()),
        }
        std::thread::sleep(interval);
    }
}

/// Print an export failure once, then count repeats instead of spamming.
fn report(last: &mut Option<String>, repeats: &mut u64, msg: String) {
    if last.as_deref() == Some(msg.as_str()) {
        *repeats += 1;
        if repeats.is_power_of_two() {
            eprintln!("toptop: OTLP export still failing ({repeats}×): {msg}");
        }
    } else {
        eprintln!("toptop: OTLP export failed: {msg}");
        *last = Some(msg);
        *repeats = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use crate::metrics::gpu::Gpu;
    use crate::metrics::ServerStats;

    #[test]
    fn endpoint_parsing() {
        assert_eq!(
            parse_endpoint("http://localhost:4318"),
            Ok(Endpoint {
                host: "localhost".into(),
                port: 4318,
                path: "/v1/metrics".into()
            })
        );
        // The OTLP defaults fill themselves in.
        assert_eq!(parse_endpoint("http://collector").unwrap().port, 4318);
        assert_eq!(
            parse_endpoint("http://collector/").unwrap().path,
            "/v1/metrics"
        );
        // An explicit path is honored.
        assert_eq!(
            parse_endpoint("http://c:9999/otlp/v1/metrics")
                .unwrap()
                .path,
            "/otlp/v1/metrics"
        );

        // https is refused with an explanation, not a cryptic failure per tick.
        let e = parse_endpoint("https://collector:4318").unwrap_err();
        assert!(e.contains("no TLS stack"), "{e}");
        assert!(e.contains("local collector"), "must say what to do instead");

        assert!(parse_endpoint("collector:4318").is_err());
        assert!(parse_endpoint("http://").is_err());
        assert!(parse_endpoint("http://host:notaport").is_err());
    }

    fn snapshot() -> Collector {
        let mut c = Collector::new(16);
        c.gpus = vec![Gpu {
            name: "RTX \"4090\"".into(), // a quote, to prove escaping
            util_pct: 31.0,
            has_util: true,
            mem_util: 94.0,
            has_mem_util: true,
            mem_used: 22_000,
            mem_total: 24_000,
            temp: 72.0,
            power: 290.0,
            power_limit: 450.0,
            throttled: true,
        }];
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            model: "Llama-3-8B".into(),
            gen_tps: Some(83.4),
            kv_pct: Some(64.0),
            preempt_rate: Some(1.2),
            ..Default::default()
        }];
        c
    }

    #[test]
    fn payload_is_valid_json_with_the_otlp_shape() {
        let body = to_otlp_json(&snapshot(), 1_700_000_000_000_000_000);
        let parsed = json::parse(&body).expect("the payload must be valid JSON");

        let rm = parsed
            .get("resourceMetrics")
            .and_then(json::Json::as_array)
            .expect("resourceMetrics array");
        assert_eq!(rm.len(), 1);
        let metrics = rm[0]
            .get("scopeMetrics")
            .and_then(json::Json::as_array)
            .expect("scopeMetrics")[0]
            .get("metrics")
            .and_then(json::Json::as_array)
            .expect("metrics array");
        assert!(!metrics.is_empty());

        // Every metric must carry a gauge with exactly one timestamped point.
        for m in metrics {
            let points = m
                .get("gauge")
                .and_then(|g| g.get("dataPoints"))
                .and_then(json::Json::as_array)
                .expect("gauge dataPoints");
            assert_eq!(points.len(), 1);
            assert_eq!(
                points[0].str("timeUnixNano"),
                Some("1700000000000000000"),
                "timestamps must be strings, per the OTLP JSON mapping"
            );
        }
    }

    #[test]
    fn the_metrics_that_matter_are_present_and_escaped() {
        let body = to_otlp_json(&snapshot(), 1);
        for name in [
            "system.cpu.utilization",
            "gpu.utilization",
            "gpu.memory.bandwidth.utilization",
            "gpu.throttled",
            "inference.tokens.generated_rate",
            "inference.kv_cache.utilization",
            "inference.preemptions.rate",
        ] {
            assert!(body.contains(name), "missing metric {name}");
        }
        // A GPU name containing a quote must not break the document.
        assert!(json::parse(&body).is_some());
        assert!(body.contains(r#"RTX \"4090\""#), "quote was not escaped");
    }

    #[test]
    fn absent_readings_are_omitted_not_zeroed() {
        let mut c = snapshot();
        c.servers[0].gen_tps = None;
        c.servers[0].preempt_rate = None;
        let body = to_otlp_json(&c, 1);
        assert!(!body.contains("inference.tokens.generated_rate"));
        assert!(!body.contains("inference.preemptions.rate"));
        // "no data" and "zero tokens per second" are different facts.
        assert!(body.contains("inference.kv_cache.utilization"));
    }

    #[test]
    fn repeated_failures_collapse_instead_of_spamming() {
        let (mut last, mut repeats) = (None, 0u64);
        report(&mut last, &mut repeats, "connection refused".into());
        assert_eq!(repeats, 1);
        for _ in 0..10 {
            report(&mut last, &mut repeats, "connection refused".into());
        }
        assert_eq!(repeats, 11);
        // A different failure resets, so a new problem is always reported.
        report(&mut last, &mut repeats, "timed out".into());
        assert_eq!(repeats, 1);
    }
}
