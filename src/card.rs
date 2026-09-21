//! `toptop --diagnose`: the verdict as a paste-able card.
//!
//! The TUI answers "why is it slow?" for the person at the keyboard. This
//! module answers it for everyone else: a plain-text block that survives a
//! paste into a GitHub issue, a Reddit thread or a Slack message unchanged —
//! ASCII rules only, no line wider than [`WIDTH`], the numbers the verdict
//! rests on, and the advice. The link at the bottom is how the next person
//! finds the tool.

use crate::alerts::Level;
use crate::diagnose::{diagnose, Finding};
use crate::metrics::gpu::no_gpu_reason;
use crate::metrics::Collector;
use crate::util::human_bytes;

/// Widest line the card emits. 72 keeps it intact inside an 80-column code
/// block with room for a quote prefix.
pub const WIDTH: usize = 72;

/// Left gutter that holds the row label (`VERDICT`, `GPU`, …).
const GUTTER: usize = 9;

const REPO_URL: &str = "https://github.com/ur-grue/toptop";

/// What the card says about where and when it was taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardMeta {
    pub host: String,
    pub version: String,
    /// Already formatted, e.g. `2026-09-21 10:32 UTC`.
    pub when: String,
    /// `--demo`: the GPU and server are synthesized, and the card must say so.
    pub demo: bool,
}

/// Render the diagnosis card for one snapshot.
pub fn render(c: &Collector, meta: &CardMeta) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(header(meta));
    out.push("=".repeat(WIDTH));
    out.extend(row_wrapped("HOST", &meta.host));
    if meta.demo {
        out.push(row(
            "NOTE",
            "simulated GPU + server (--demo); system numbers are real",
        ));
    }
    push_findings(&mut out, c);
    out.push(String::new());
    push_gpus(&mut out, c);
    push_servers(&mut out, c);
    out.extend(row_wrapped("SYSTEM", &system_line(c)));
    out.push("-".repeat(WIDTH));
    out.push(footer(meta));
    out.join("\n") + "\n"
}

fn header(meta: &CardMeta) -> String {
    truncate_chars(
        &format!("toptop diagnose  ·  v{}  ·  {}", meta.version, meta.when),
        WIDTH,
    )
}

fn footer(meta: &CardMeta) -> String {
    format!(
        "toptop v{} · {} · `toptop --diagnose`",
        meta.version, REPO_URL
    )
}

fn push_findings(out: &mut Vec<String>, c: &Collector) {
    let findings = diagnose(c);
    if findings.is_empty() {
        out.push(row("VERDICT", "NO GPU METRICS"));
        out.extend(wrap_indented(no_gpu_reason()));
        out.extend(wrap_indented(
            "The system and inference-server numbers below are still real. \
             On a remote or non-Linux box, point toptop at the server with \
             --llm-server host:port.",
        ));
        return;
    }
    for (i, f) in findings.iter().enumerate() {
        if i > 0 {
            out.push(String::new());
        }
        out.push(row(finding_label(i, f), f.headline));
        out.extend(wrap_indented(&f.evidence));
        out.extend(wrap_indented(f.advice));
    }
}

/// The first finding is the verdict; the rest are graded by severity.
fn finding_label(index: usize, f: &Finding) -> &'static str {
    if index == 0 {
        "VERDICT"
    } else {
        match f.severity {
            Level::Crit => "CRIT",
            Level::Warn => "WARN",
        }
    }
}

fn push_gpus(out: &mut Vec<String>, c: &Collector) {
    for (i, g) in c.gpus.iter().enumerate() {
        out.extend(row_wrapped(if i == 0 { "GPU" } else { "" }, &g.name));
        let mut parts: Vec<String> = Vec::new();
        if g.has_util {
            parts.push(format!("compute {:.0}%", g.util_pct));
        }
        if g.has_mem_util {
            parts.push(format!("mem b/w {:.0}%", g.mem_util));
        }
        if g.mem_total > 0 {
            parts.push(format!(
                "vram {} / {} ({:.0}%)",
                human_bytes(g.mem_used),
                human_bytes(g.mem_total),
                g.mem_pct()
            ));
        }
        if g.temp > 0.0 {
            parts.push(format!("{:.0}°C", g.temp));
        }
        if g.power > 0.0 && g.power_limit > 0.0 {
            parts.push(format!("{:.0}/{:.0} W", g.power, g.power_limit));
        } else if g.power > 0.0 {
            parts.push(format!("{:.0} W", g.power));
        }
        if g.throttled {
            parts.push("THROTTLED".into());
        }
        if parts.is_empty() {
            parts.push("no utilization or memory figures from the driver".into());
        }
        out.extend(wrap_indented(&parts.join("  ·  ")));
    }
}

fn push_servers(out: &mut Vec<String>, c: &Collector) {
    for (i, s) in c.servers.iter().enumerate() {
        let title = if s.model.is_empty() {
            s.label()
        } else {
            format!("{} {}", s.label(), s.model)
        };
        out.extend(row_wrapped(if i == 0 { "SERVER" } else { "" }, &title));
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = s.gen_tps {
            parts.push(format!("{t:.1} tok/s gen"));
        }
        if let Some(t) = s.prompt_tps {
            parts.push(format!("{t:.0} tok/s prefill"));
        }
        if let Some(k) = s.kv_pct {
            parts.push(format!("kv {k:.0}%"));
        }
        if let Some(w) = s.waiting {
            parts.push(format!("queue {w:.0}"));
        }
        if let Some(p) = s.gpu_offload_pct {
            parts.push(format!("{p:.0}% on GPU"));
        }
        if let Some(t) = s.ttft {
            parts.push(format!("ttft p50 {:.0}ms p95 {:.0}ms", t.p50, t.p95));
        }
        if let Some(t) = s.tpot {
            parts.push(format!("tpot p50 {:.0}ms p95 {:.0}ms", t.p50, t.p95));
        }
        if let Some(r) = s.preempt_rate.filter(|r| *r > 0.0) {
            parts.push(format!("preempting {r:.1}/s"));
        }
        if parts.is_empty() {
            parts.push("reachable, no throughput figures yet".into());
        }
        out.extend(wrap_indented(&parts.join("  ·  ")));
    }
}

fn system_line(c: &Collector) -> String {
    let ram_pct = if c.mem.total > 0 {
        c.mem.used as f64 / c.mem.total as f64 * 100.0
    } else {
        0.0
    };
    format!(
        "{} · {} · {}c · cpu {:.0}% · ram {} / {} ({ram_pct:.0}%) · swap {}",
        c.host.os,
        c.host.arch,
        c.cpu.per_core.len(),
        c.cpu.global_usage,
        human_bytes(c.mem.used),
        human_bytes(c.mem.total),
        human_bytes(c.mem.swap_used),
    )
}

/// `LABEL    text`, label padded to the gutter, text clipped to the width.
fn row(label: &str, text: &str) -> String {
    truncate_chars(&format!("{label:<GUTTER$}{text}"), WIDTH)
}

/// Like [`row`], but long text wraps under the gutter instead of being cut.
fn row_wrapped(label: &str, text: &str) -> Vec<String> {
    let mut lines = wrap_indented(text);
    if lines.is_empty() {
        lines.push(String::new());
    }
    let first = lines[0].trim_start().to_string();
    lines[0] = format!("{label:<GUTTER$}{first}");
    lines
}

/// Word-wrap `text` under the gutter so evidence and advice stay readable
/// without ever exceeding the width.
fn wrap_indented(text: &str) -> Vec<String> {
    let body = WIDTH - GUTTER;
    let indent = " ".repeat(GUTTER);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let word = truncate_chars(word, body);
        let next_len = if cur.is_empty() {
            word.chars().count()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
        if next_len > body && !cur.is_empty() {
            lines.push(format!("{indent}{cur}"));
            cur.clear();
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() {
        lines.push(format!("{indent}{cur}"));
    }
    lines
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::gpu::Gpu;
    use crate::metrics::ServerStats;

    fn meta() -> CardMeta {
        CardMeta {
            host: "box".into(),
            version: "9.9.9".into(),
            when: "2026-09-21 10:00 UTC".into(),
            demo: false,
        }
    }

    fn gpu(util: f32, mem_util: f32, used: u64, total: u64) -> Gpu {
        Gpu {
            name: "NVIDIA GeForce RTX 4090".into(),
            util_pct: util,
            has_util: true,
            mem_util,
            has_mem_util: true,
            mem_used: used,
            mem_total: total,
            temp: 74.0,
            power: 344.0,
            power_limit: 450.0,
            throttled: false,
        }
    }

    fn bandwidth_bound() -> Collector {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(24.0, 92.0, 17 << 30, 24 << 30)];
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            model: "meta-llama/Llama-3-8B".into(),
            gen_tps: Some(73.7),
            kv_pct: Some(96.0),
            waiting: Some(0.0),
            ..Default::default()
        }];
        c
    }

    fn widest(card: &str) -> usize {
        card.lines().map(|l| l.chars().count()).max().unwrap_or(0)
    }

    #[test]
    fn leads_with_the_verdict_and_ends_with_the_link() {
        let card = render(&bandwidth_bound(), &meta());
        let verdict = card
            .lines()
            .find(|l| l.starts_with("VERDICT"))
            .expect("a verdict row");
        assert!(verdict.contains("MEMORY-BANDWIDTH BOUND"), "{verdict}");
        assert!(
            card.contains("bandwidth 92% · compute 24%"),
            "evidence stays checkable"
        );
        assert!(
            card.contains("faster GPU core would not help"),
            "advice is on the card"
        );
        assert!(card.trim_end().ends_with("`toptop --diagnose`"));
        assert!(card.contains(REPO_URL), "the link is the growth loop");
    }

    #[test]
    fn every_line_fits_the_width() {
        let card = render(&bandwidth_bound(), &meta());
        assert!(
            widest(&card) <= WIDTH,
            "widest line {} > {WIDTH}",
            widest(&card)
        );
    }

    #[test]
    fn long_rows_wrap_instead_of_losing_the_end() {
        let mut c = bandwidth_bound();
        c.host.hostname = "a-very-long-hostname-that-goes-on.internal.example.com".into();
        c.host.os = "Ubuntu 24.04.3 LTS (Noble Numbat) with a long pretty name".into();
        c.gpus[0].name = "NVIDIA RTX PRO 6000 Blackwell Server Edition 96GB".into();
        let m = CardMeta {
            host: c.host.hostname.clone(),
            ..meta()
        };
        let card = render(&c, &m);
        assert!(widest(&card) <= WIDTH);
        assert!(card.contains("HOST     a-very-long-hostname"));
        // Wrapping may break between "swap" and its value; compare on the
        // whitespace-normalised text.
        let flat = card.split_whitespace().collect::<Vec<_>>().join(" ");
        let system_end = format!("swap {}", human_bytes(c.mem.swap_used));
        assert!(
            flat.contains(&system_end),
            "the end of the system line survives"
        );
        assert!(card.contains("Blackwell"));
    }

    #[test]
    fn gpu_and_server_rows_carry_the_numbers() {
        let card = render(&bandwidth_bound(), &meta());
        assert!(card.contains("GPU      NVIDIA GeForce RTX 4090"));
        assert!(card.contains("vram 17.0 GiB / 24.0 GiB (71%)"));
        assert!(card.contains("344/450 W"));
        assert!(card.contains("SERVER   vLLM:8000 meta-llama/Llama-3-8B"));
        assert!(card.contains("73.7 tok/s gen"));
        assert!(card.contains("kv 96%"));
    }

    #[test]
    fn no_gpu_is_said_plainly_and_the_rest_still_renders() {
        let mut c = bandwidth_bound();
        c.gpus.clear();
        let card = render(&c, &meta());
        assert!(card.contains("VERDICT  NO GPU METRICS"));
        assert!(card.contains("--llm-server"), "points at the manual path");
        assert!(
            card.contains("SERVER   vLLM:8000"),
            "server data is still real"
        );
        assert!(card.contains("SYSTEM   "));
        assert!(widest(&card) <= WIDTH);
    }

    #[test]
    fn demo_cards_confess() {
        let m = CardMeta {
            demo: true,
            ..meta()
        };
        let card = render(&bandwidth_bound(), &m);
        assert!(card.contains("NOTE     simulated GPU + server (--demo)"));
        assert!(!render(&bandwidth_bound(), &meta()).contains("simulated"));
    }

    #[test]
    fn secondary_findings_are_graded_not_repeated_as_verdicts() {
        let mut c = bandwidth_bound();
        c.gpus[0].mem_used = c.gpus[0].mem_total; // VRAM exhausted on top
        let card = render(&c, &meta());
        assert_eq!(card.matches("VERDICT").count(), 1);
        assert!(card.contains("VERDICT  VRAM EXHAUSTED"));
        assert!(card.contains("WARN     MEMORY-BANDWIDTH BOUND"));
    }

    #[test]
    fn long_words_never_break_the_width() {
        let lines = wrap_indented(&"x".repeat(200));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].chars().count(), WIDTH);
    }
}
