//! Machine-readable JSON export of a metrics snapshot.
//!
//! This is the foundation for multi-host monitoring: each machine emits its
//! state with `toptop --export json` and a controller aggregates the results.
//! It's also handy on its own for scripts, dashboards, and alerting. The
//! serializer is hand-rolled to keep `toptop` dependency-free.

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
}
