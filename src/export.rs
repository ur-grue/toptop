//! Machine-readable JSON export of a metrics snapshot.
//!
//! This is the foundation for multi-host monitoring: each machine emits its
//! state with `toptop --export json` and a controller aggregates the results.
//! It's also handy on its own for scripts, dashboards, and alerting. The
//! serializer is hand-rolled to keep `toptop` dependency-free.

use crate::alerts::{self, AlertConfig};
use crate::metrics::Collector;

/// Escape a string for embedding in a JSON document.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// JSON-encode a finite `f64`, falling back to `0` for NaN/infinity (JSON has
/// no representation for those).
fn num(v: f64) -> String {
    if v.is_finite() {
        // Trim to two decimals to keep payloads compact.
        format!("{:.2}", v)
    } else {
        "0".to_string()
    }
}

/// Render a full snapshot of `c` as a single-line JSON object. `top_procs`
/// caps the number of processes included (highest CPU first) to keep the
/// payload bounded for frequent polling.
pub fn to_json(c: &Collector, top_procs: usize) -> String {
    let mut s = String::with_capacity(4096);
    s.push('{');

    // host
    s.push_str(&format!(
        "\"host\":{{\"hostname\":\"{}\",\"os\":\"{}\",\"kernel\":\"{}\",\"arch\":\"{}\",\"cpu\":\"{}\",\"cores\":{}}},",
        esc(&c.host.hostname),
        esc(&c.host.os),
        esc(&c.host.kernel),
        esc(&c.host.arch),
        esc(&c.host.cpu_brand),
        c.host.logical_cores,
    ));

    s.push_str(&format!("\"uptime\":{},", c.uptime));
    s.push_str(&format!(
        "\"tasks\":{},\"running\":{},",
        c.procs.len(),
        c.running_procs()
    ));

    // cpu
    let cores: Vec<String> = c.cpu.per_core.iter().map(|v| num(*v as f64)).collect();
    s.push_str(&format!(
        "\"cpu\":{{\"usage\":{},\"freq_mhz\":{},\"load\":[{},{},{}],\"per_core\":[{}]}},",
        num(c.cpu.global_usage as f64),
        c.cpu.freq_mhz,
        num(c.cpu.load_avg.0),
        num(c.cpu.load_avg.1),
        num(c.cpu.load_avg.2),
        cores.join(",")
    ));

    // mem
    s.push_str(&format!(
        "\"mem\":{{\"used\":{},\"total\":{},\"swap_used\":{},\"swap_total\":{}}},",
        c.mem.used, c.mem.total, c.mem.swap_used, c.mem.swap_total
    ));

    // net
    let nets: Vec<String> = c
        .nets
        .iter()
        .map(|n| {
            format!(
                "{{\"name\":\"{}\",\"down_rate\":{},\"up_rate\":{},\"total_down\":{},\"total_up\":{}}}",
                esc(&n.name),
                num(n.down_rate),
                num(n.up_rate),
                n.total_down,
                n.total_up
            )
        })
        .collect();
    s.push_str(&format!("\"net\":[{}],", nets.join(",")));

    // disk io + filesystems
    s.push_str(&format!(
        "\"disk_io\":{{\"read_rate\":{},\"write_rate\":{}}},",
        num(c.disk_read_rate),
        num(c.disk_write_rate)
    ));
    let disks: Vec<String> = c
        .disk_list
        .iter()
        .map(|d| {
            format!(
                "{{\"mount\":\"{}\",\"used_pct\":{},\"total\":{},\"available\":{}}}",
                esc(&d.mount),
                num(d.used_pct as f64),
                d.total,
                d.available
            )
        })
        .collect();
    s.push_str(&format!("\"disks\":[{}],", disks.join(",")));

    // gpus
    let gpus: Vec<String> = c
        .gpus
        .iter()
        .map(|g| {
            format!(
                "{{\"name\":\"{}\",\"util\":{},\"has_util\":{},\"mem_util\":{},\"has_mem_util\":{},\"mem_used\":{},\"mem_total\":{},\"temp\":{},\"power\":{},\"power_limit\":{},\"throttled\":{}}}",
                esc(&g.name),
                num(g.util_pct as f64),
                g.has_util,
                num(g.mem_util as f64),
                g.has_mem_util,
                g.mem_used,
                g.mem_total,
                num(g.temp as f64),
                num(g.power as f64),
                num(g.power_limit as f64),
                g.throttled
            )
        })
        .collect();
    s.push_str(&format!("\"gpus\":[{}],", gpus.join(",")));

    // GPU compute processes (NVIDIA), joined to process names where known.
    let gpu_procs: Vec<String> = c
        .gpu_procs
        .iter()
        .map(|gp| {
            let name = c
                .procs
                .iter()
                .find(|p| p.pid == gp.pid)
                .map(|p| p.name.as_str())
                .unwrap_or("");
            format!(
                "{{\"pid\":{},\"name\":\"{}\",\"used_mem\":{}}}",
                gp.pid,
                esc(name),
                gp.used_mem
            )
        })
        .collect();
    s.push_str(&format!("\"gpu_procs\":[{}],", gpu_procs.join(",")));

    // Auto-discovered inference servers (tokens/sec, KV cache, queue, …).
    let optnum = |v: Option<f64>| v.map(num).unwrap_or_else(|| "null".to_string());
    let servers: Vec<String> = c
        .servers
        .iter()
        .map(|sv| {
            format!(
                "{{\"runtime\":\"{}\",\"pid\":{},\"port\":{},\"model\":\"{}\",\"gen_tps\":{},\"prompt_tps\":{},\"running\":{},\"waiting\":{},\"kv_pct\":{},\"ttft_ms\":{},\"gpu_offload_pct\":{}}}",
                esc(sv.runtime),
                sv.pid,
                sv.port,
                esc(&sv.model),
                optnum(sv.gen_tps),
                optnum(sv.prompt_tps),
                optnum(sv.running),
                optnum(sv.waiting),
                optnum(sv.kv_pct),
                optnum(sv.ttft_ms),
                optnum(sv.gpu_offload_pct),
            )
        })
        .collect();
    s.push_str(&format!("\"servers\":[{}],", servers.join(",")));

    // sensors
    let sensors: Vec<String> = c
        .sensors
        .iter()
        .map(|t| {
            format!(
                "{{\"label\":\"{}\",\"temp\":{}}}",
                esc(&t.label),
                num(t.temp as f64)
            )
        })
        .collect();
    s.push_str(&format!("\"sensors\":[{}],", sensors.join(",")));

    // battery
    match &c.battery {
        Some(b) => s.push_str(&format!(
            "\"battery\":{{\"percent\":{},\"status\":\"{}\"}},",
            num(b.percent as f64),
            esc(&b.status)
        )),
        None => s.push_str("\"battery\":null,"),
    }

    // top processes by CPU
    let mut procs: Vec<&_> = c.procs.iter().collect();
    procs.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let rows: Vec<String> = procs
        .iter()
        .take(top_procs)
        .map(|p| {
            format!(
                "{{\"pid\":{},\"name\":\"{}\",\"user\":\"{}\",\"cpu\":{},\"mem_pct\":{},\"mem_bytes\":{},\"io_read\":{},\"io_write\":{}}}",
                p.pid,
                esc(&p.name),
                esc(&p.user),
                num(p.cpu as f64),
                num(p.mem_pct as f64),
                p.mem_bytes,
                num(p.io_read_rate),
                num(p.io_write_rate)
            )
        })
        .collect();
    s.push_str(&format!("\"procs\":[{}]", rows.join(",")));

    s.push('}');
    s
}

// ── CSV export ─────────────────────────────────────────────────────────────

/// Escape one CSV field per RFC 4180: wrap in double quotes when it contains a
/// comma, quote, or newline, doubling any embedded quotes.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Render the top processes (highest CPU first, capped at `top_procs`) as CSV
/// with a header row. CSV is the natural tabular slice for spreadsheets and
/// `awk`; the full nested snapshot stays in `--export json`.
pub fn to_csv(c: &Collector, top_procs: usize) -> String {
    let mut procs: Vec<&_> = c.procs.iter().collect();
    procs.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut s = String::with_capacity(top_procs * 64 + 80);
    s.push_str("pid,name,user,cpu_pct,mem_pct,mem_bytes,io_read_bps,io_write_bps,command\n");
    for p in procs.iter().take(top_procs) {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            p.pid,
            csv_field(&p.name),
            csv_field(&p.user),
            num(p.cpu as f64),
            num(p.mem_pct as f64),
            p.mem_bytes,
            num(p.io_read_rate),
            num(p.io_write_rate),
            csv_field(&p.cmd),
        ));
    }
    s
}

// ── Prometheus exposition format ───────────────────────────────────────────

/// Sanitize a label value for the Prometheus text format.
fn plabel(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// Render a snapshot in Prometheus exposition format so toptop can be scraped
/// into Grafana/VictoriaMetrics — the local-inference observability layer.
/// Pair with a tiny `socat`/`while` loop, or scrape `--export json` directly.
pub fn to_prometheus(c: &Collector, alert_cfg: &AlertConfig) -> String {
    let mut s = String::with_capacity(4096);
    let mut metric = |name: &str, help: &str, value: String| {
        s.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} gauge\n{value}"
        ));
    };

    metric(
        "toptop_cpu_usage_percent",
        "Global CPU utilization.",
        format!("toptop_cpu_usage_percent {:.2}\n", c.cpu.global_usage),
    );
    metric(
        "toptop_load1",
        "1-minute load average.",
        format!("toptop_load1 {:.2}\n", c.cpu.load_avg.0),
    );
    metric(
        "toptop_mem_used_bytes",
        "Used physical memory.",
        format!("toptop_mem_used_bytes {}\n", c.mem.used),
    );
    metric(
        "toptop_mem_total_bytes",
        "Total physical memory.",
        format!("toptop_mem_total_bytes {}\n", c.mem.total),
    );
    metric(
        "toptop_uptime_seconds",
        "System uptime.",
        format!("toptop_uptime_seconds {}\n", c.uptime),
    );
    metric(
        "toptop_tasks",
        "Number of running tasks.",
        format!("toptop_tasks {}\n", c.procs.len()),
    );

    // GPUs — each metric family gets its own TYPE declaration.
    if !c.gpus.is_empty() {
        let gpu_metrics: &[(&str, &str)] = &[
            ("toptop_gpu_util_percent", "GPU core utilization."),
            (
                "toptop_gpu_mem_bandwidth_percent",
                "GPU memory bandwidth utilization.",
            ),
            ("toptop_gpu_mem_used_bytes", "GPU memory used."),
            ("toptop_gpu_mem_total_bytes", "GPU memory total."),
            ("toptop_gpu_power_watts", "GPU power draw."),
            ("toptop_gpu_temp_celsius", "GPU temperature."),
            ("toptop_gpu_throttled", "GPU thermal throttle state."),
        ];
        for (name, help) in gpu_metrics {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n"));
        }
        for (i, g) in c.gpus.iter().enumerate() {
            let l = format!("gpu=\"{}\",name=\"{}\"", i, plabel(&g.name));
            s.push_str(&format!(
                "toptop_gpu_util_percent{{{l}}} {:.2}\n",
                g.util_pct
            ));
            if g.has_mem_util {
                s.push_str(&format!(
                    "toptop_gpu_mem_bandwidth_percent{{{l}}} {:.2}\n",
                    g.mem_util
                ));
            }
            s.push_str(&format!(
                "toptop_gpu_mem_used_bytes{{{l}}} {}\n",
                g.mem_used
            ));
            s.push_str(&format!(
                "toptop_gpu_mem_total_bytes{{{l}}} {}\n",
                g.mem_total
            ));
            s.push_str(&format!("toptop_gpu_power_watts{{{l}}} {:.2}\n", g.power));
            s.push_str(&format!("toptop_gpu_temp_celsius{{{l}}} {:.2}\n", g.temp));
            s.push_str(&format!(
                "toptop_gpu_throttled{{{l}}} {}\n",
                if g.throttled { 1 } else { 0 }
            ));
        }
    }

    // Inference servers — each metric family gets its own TYPE declaration.
    if !c.servers.is_empty() {
        let infer_metrics: &[(&str, &str)] = &[
            (
                "toptop_inference_tokens_per_second",
                "Generation throughput.",
            ),
            (
                "toptop_inference_prefill_tokens_per_second",
                "Prefill throughput.",
            ),
            ("toptop_inference_kv_cache_percent", "KV-cache utilization."),
            (
                "toptop_inference_requests_running",
                "Active inference requests.",
            ),
            (
                "toptop_inference_requests_waiting",
                "Queued inference requests.",
            ),
            ("toptop_inference_ttft_ms", "Time to first token."),
            (
                "toptop_inference_ttft_p50_ms",
                "Time to first token, 50th percentile.",
            ),
            (
                "toptop_inference_ttft_p95_ms",
                "Time to first token, 95th percentile.",
            ),
            (
                "toptop_inference_ttft_p99_ms",
                "Time to first token, 99th percentile.",
            ),
            (
                "toptop_inference_tpot_p50_ms",
                "Time per output token, 50th percentile.",
            ),
            (
                "toptop_inference_tpot_p95_ms",
                "Time per output token, 95th percentile.",
            ),
            (
                "toptop_inference_tpot_p99_ms",
                "Time per output token, 99th percentile.",
            ),
        ];
        for (name, help) in infer_metrics {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n"));
        }
        for sv in &c.servers {
            let l = format!(
                "runtime=\"{}\",port=\"{}\",model=\"{}\"",
                plabel(sv.runtime),
                sv.port,
                plabel(&sv.model)
            );
            let g = |name: &str, v: Option<f64>| {
                v.map(|v| format!("toptop_inference_{name}{{{l}}} {v:.2}\n"))
                    .unwrap_or_default()
            };
            s.push_str(&g("tokens_per_second", sv.gen_tps));
            s.push_str(&g("prefill_tokens_per_second", sv.prompt_tps));
            s.push_str(&g("kv_cache_percent", sv.kv_pct));
            s.push_str(&g("requests_running", sv.running));
            s.push_str(&g("requests_waiting", sv.waiting));
            s.push_str(&g("ttft_ms", sv.ttft_ms));
            s.push_str(&g("ttft_p50_ms", sv.ttft.map(|p| p.p50)));
            s.push_str(&g("ttft_p95_ms", sv.ttft.map(|p| p.p95)));
            s.push_str(&g("ttft_p99_ms", sv.ttft.map(|p| p.p99)));
            s.push_str(&g("tpot_p50_ms", sv.tpot.map(|p| p.p50)));
            s.push_str(&g("tpot_p95_ms", sv.tpot.map(|p| p.p95)));
            s.push_str(&g("tpot_p99_ms", sv.tpot.map(|p| p.p99)));
        }
    }

    // Firing alerts — one gauge per active alert so Alertmanager can page.
    let alerts = alerts::evaluate(c, alert_cfg);
    if !alerts.is_empty() {
        s.push_str("# HELP toptop_alert A firing alert (1 = active).\n# TYPE toptop_alert gauge\n");
        for a in &alerts {
            s.push_str(&format!(
                "toptop_alert{{key=\"{}\",detail=\"{}\",level=\"{}\"}} 1\n",
                a.key,
                plabel(&a.detail),
                a.level.label()
            ));
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_control_and_quotes() {
        assert_eq!(esc("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(esc("line\nbreak"), "line\\nbreak");
        assert_eq!(esc("tab\tend"), "tab\\tend");
    }

    #[test]
    fn num_is_finite_safe() {
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert_eq!(num(1.5), "1.50");
    }

    #[test]
    fn csv_field_escapes_per_rfc4180() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn csv_has_header_row() {
        let c = Collector::new(64);
        let csv = to_csv(&c, 5);
        assert_eq!(
            csv.lines().next().unwrap(),
            "pid,name,user,cpu_pct,mem_pct,mem_bytes,io_read_bps,io_write_bps,command"
        );
    }

    #[test]
    fn snapshot_is_balanced_json() {
        let c = Collector::new(64);
        let json = to_json(&c, 5);
        // Top-level object and required keys present.
        assert!(json.starts_with('{') && json.ends_with('}'));
        for key in [
            "\"host\"",
            "\"cpu\"",
            "\"mem\"",
            "\"net\"",
            "\"disks\"",
            "\"gpus\"",
            "\"procs\"",
            "\"battery\"",
            "\"servers\"",
            "\"gpu_procs\"",
        ] {
            assert!(json.contains(key), "missing {key}");
        }
        // Braces and brackets are balanced.
        let braces = json.chars().filter(|&c| c == '{').count()
            == json.chars().filter(|&c| c == '}').count();
        let brackets = json.chars().filter(|&c| c == '[').count()
            == json.chars().filter(|&c| c == ']').count();
        assert!(braces && brackets);
    }

    #[test]
    fn prometheus_has_core_metrics() {
        let c = Collector::new(64);
        let p = to_prometheus(&c, &AlertConfig::default());
        for m in [
            "# TYPE toptop_cpu_usage_percent gauge",
            "toptop_mem_total_bytes ",
            "toptop_uptime_seconds ",
        ] {
            assert!(p.contains(m), "missing {m}");
        }
        // Every metric line must be `name value` or `name{labels} value`.
        for line in p.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let val = line.rsplit(' ').next().unwrap();
            assert!(val.parse::<f64>().is_ok(), "bad metric line: {line}");
        }
    }
}
