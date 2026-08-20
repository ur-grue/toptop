//! The shipped Grafana dashboard must reference metrics toptop actually
//! exports.
//!
//! A dashboard is documentation that silently rots: rename a metric and the
//! panel just goes blank, for everyone, forever. This test reads
//! `assets/grafana/toptop-dashboard.json`, pulls every `toptop_*` name out of
//! it, and checks each against the exporter's own `# TYPE` declarations.

use std::collections::HashSet;

use toptop::alerts::AlertConfig;
use toptop::metrics::{gpu::Gpu, Collector, Percentiles, ServerStats};

/// Every metric name the exporter declares, with a snapshot rigged so that
/// *all* the conditional families (GPU, inference, alerts) are emitted.
fn exported_metric_names() -> HashSet<String> {
    let mut c = Collector::new(16);
    c.gpus = vec![Gpu {
        name: "TestGPU".into(),
        util_pct: 50.0,
        has_util: true,
        mem_util: 50.0,
        has_mem_util: true,
        mem_used: 99,
        mem_total: 100,
        temp: 80.0,
        power: 400.0,
        power_limit: 400.0,
        throttled: true,
    }];
    c.servers = vec![ServerStats {
        runtime: "vLLM",
        pid: 1,
        port: 8000,
        model: "test".into(),
        gen_tps: Some(1.0),
        prompt_tps: Some(1.0),
        running: Some(1.0),
        waiting: Some(99.0),
        kv_pct: Some(99.0),
        ttft_ms: Some(1.0),
        ttft: Some(Percentiles {
            p50: 1.0,
            p95: 2.0,
            p99: 3.0,
        }),
        tpot: Some(Percentiles {
            p50: 1.0,
            p95: 2.0,
            p99: 3.0,
        }),
        preemptions: Some(1.0),
        preempt_rate: Some(1.0),
        gpu_offload_pct: None,
        addr: None,
    }];

    toptop::export::to_prometheus(&c, &AlertConfig::default())
        .lines()
        .filter_map(|l| l.strip_prefix("# TYPE "))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// Pull every `toptop_*` identifier out of the dashboard JSON.
fn dashboard_metric_names(json: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = json.as_bytes();
    let mut i = 0;
    while let Some(rel) = json[i..].find("toptop_") {
        let start = i + rel;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        out.insert(json[start..end].to_string());
        i = end;
    }
    out
}

#[test]
fn the_shipped_dashboard_only_uses_metrics_we_export() {
    let json = std::fs::read_to_string("assets/grafana/toptop-dashboard.json")
        .expect("the dashboard ships with the repo");
    let used = dashboard_metric_names(&json);
    assert!(
        !used.is_empty(),
        "no metrics found — did the format change?"
    );

    let exported = exported_metric_names();
    let missing: Vec<&String> = used.difference(&exported).collect();
    assert!(
        missing.is_empty(),
        "the dashboard references metrics toptop does not export, so those \
         panels are blank for every user: {missing:?}"
    );
}

#[test]
fn the_dashboard_covers_the_metrics_that_make_toptop_worth_running() {
    let json = std::fs::read_to_string("assets/grafana/toptop-dashboard.json").unwrap();
    let used = dashboard_metric_names(&json);
    // Not every metric needs a panel, but the ones no other tool has do — a
    // dashboard without them is a dashboard for a generic system monitor.
    for name in [
        "toptop_gpu_mem_bandwidth_percent",
        "toptop_inference_tokens_per_second",
        "toptop_inference_preemptions_per_second",
        "toptop_inference_ttft_p95_ms",
        "toptop_inference_tpot_p95_ms",
        "toptop_gpu_throttled",
    ] {
        assert!(
            used.contains(name),
            "{name} has no panel — it is one of the reasons toptop exists"
        );
    }
}

#[test]
fn the_dashboard_is_valid_json_with_no_overlapping_panels() {
    let json = std::fs::read_to_string("assets/grafana/toptop-dashboard.json").unwrap();
    let parsed = toptop::json::parse(&json).expect("valid JSON");
    let panels = parsed
        .get("panels")
        .and_then(toptop::json::Json::as_array)
        .expect("panels array");
    assert!(panels.len() >= 8, "suspiciously few panels");

    // Grafana tolerates overlaps by silently reflowing, which turns a tidy
    // dashboard into a jumbled one on someone else's screen.
    let boxes: Vec<(i64, i64, i64, i64, String)> = panels
        .iter()
        .map(|p| {
            let g = p.get("gridPos").expect("gridPos");
            let n = |k: &str| g.num(k).unwrap_or(0.0) as i64;
            (
                n("x"),
                n("y"),
                n("w"),
                n("h"),
                p.str("title").unwrap_or("?").to_string(),
            )
        })
        .collect();
    for (i, a) in boxes.iter().enumerate() {
        assert!(a.0 + a.2 <= 24, "{} runs past the 24-column grid", a.4);
        for b in &boxes[i + 1..] {
            let overlap = a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
            assert!(!overlap, "panels overlap: {:?} and {:?}", a.4, b.4);
        }
    }
}
