//! Threshold alerts for local-inference health.
//!
//! Evaluating a [`Collector`] against an [`AlertConfig`] yields the set of
//! currently-firing [`Alert`]s — VRAM spill risk, GPU throttling, KV-cache
//! pressure, and request-queue backlog. The TUI surfaces these as a red banner,
//! and the Prometheus exporter emits them as `toptop_alert{…}` gauges so
//! Alertmanager can page on them. The evaluation is pure and unit-tested.
//!
//! [`AlertTracker`] turns each tick's alert *set* into fire/resolve
//! *transitions*, which is what notifications and the in-TUI timeline are
//! actually about. It debounces flapping — an alert that re-fires within the
//! flap window is tracked but not re-announced, and its matching resolve is
//! suppressed too, so the timeline never shows a resolve without its fire.
//! Delivering those transitions somewhere is `crate::notify`'s job; everything
//! here stays free of I/O and clocks it takes as arguments, so it is testable.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::metrics::Collector;

/// Alert severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Warn,
    Crit,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Warn => "warn",
            Level::Crit => "crit",
        }
    }
}

/// A single firing alert.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub level: Level,
    /// Stable machine key, e.g. `vram_spill` (used as a Prometheus label).
    pub key: &'static str,
    /// What the alert is about, e.g. `gpu0` or `vLLM:8000`.
    pub detail: String,
    /// Human-readable message for the TUI.
    pub message: String,
}

/// Tunable alert thresholds.
#[derive(Clone, Debug)]
pub struct AlertConfig {
    /// VRAM usage % at or above which a model risks spilling to system RAM.
    pub vram_spill_pct: f32,
    /// KV-cache usage % considered saturated.
    pub kv_high_pct: f64,
    /// Number of waiting requests considered a backlog.
    pub queue_high: f64,
    /// Preemptions per second at or above which the server is thrashing its
    /// KV cache. The default is deliberately just above zero: a preempted
    /// request throws away work already done, so a *sustained* nonzero rate is
    /// always worth knowing about.
    pub preempt_rate_high: f64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            vram_spill_pct: 90.0,
            kv_high_pct: 95.0,
            queue_high: 8.0,
            preempt_rate_high: 0.2,
        }
    }
}

/// Evaluate all alert rules against the current snapshot.
pub fn evaluate(c: &Collector, cfg: &AlertConfig) -> Vec<Alert> {
    let mut out = Vec::new();
    for (i, g) in c.gpus.iter().enumerate() {
        if g.throttled {
            out.push(Alert {
                level: Level::Crit,
                key: "gpu_throttle",
                detail: format!("gpu{i}"),
                message: format!("gpu{i} is throttling ({})", g.name),
            });
        }
        let vram = g.mem_pct();
        if g.mem_total > 0 && vram >= cfg.vram_spill_pct {
            out.push(Alert {
                level: Level::Warn,
                key: "vram_spill",
                detail: format!("gpu{i}"),
                message: format!("gpu{i} VRAM {vram:.0}% — risk of spilling to RAM"),
            });
        }
    }
    for sv in &c.servers {
        let tag = format!("{}:{}", sv.runtime, sv.port);
        if let Some(kv) = sv.kv_pct {
            if kv >= cfg.kv_high_pct {
                out.push(Alert {
                    level: Level::Warn,
                    key: "kv_high",
                    detail: tag.clone(),
                    message: format!("{tag} KV cache {kv:.0}% — near capacity"),
                });
            }
        }
        if let Some(waiting) = sv.waiting {
            if waiting >= cfg.queue_high {
                out.push(Alert {
                    level: Level::Warn,
                    key: "queue_backlog",
                    detail: tag.clone(),
                    message: format!("{tag} {waiting:.0} requests queued"),
                });
            }
        }
        if let Some(rate) = sv.preempt_rate {
            if rate >= cfg.preempt_rate_high {
                out.push(Alert {
                    // Critical: unlike a queue backlog, preemption actively
                    // destroys work that was already done.
                    level: Level::Crit,
                    key: "kv_preemption",
                    detail: tag.clone(),
                    message: format!(
                        "{tag} preempting {rate:.1}/s — KV cache thrashing, work is being recomputed"
                    ),
                });
            }
        }
    }
    out
}

/// Whether any firing alert is critical (used to color the TUI banner).
pub fn worst_level(alerts: &[Alert]) -> Option<Level> {
    alerts.iter().map(|a| a.level).reduce(|a, b| {
        if a == Level::Crit || b == Level::Crit {
            Level::Crit
        } else {
            Level::Warn
        }
    })
}

// ── Transitions, debounce and history ────────────────────────────────────────

/// Which way an alert crossed its threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionState {
    Fired,
    Resolved,
}

impl TransitionState {
    pub fn label(self) -> &'static str {
        match self {
            TransitionState::Fired => "fired",
            TransitionState::Resolved => "resolved",
        }
    }
}

/// An alert crossing its threshold in one direction, at a point in time.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub state: TransitionState,
    pub level: Level,
    pub key: &'static str,
    pub detail: String,
    pub message: String,
    /// When it happened, for the timeline's relative ages.
    pub at: Instant,
}

impl Transition {
    /// Stable identity of the underlying alert (`key:detail`).
    pub fn id(&self) -> String {
        format!("{}:{}", self.key, self.detail)
    }
}

/// Identity of an alert, stable across ticks.
fn alert_id(a: &Alert) -> String {
    format!("{}:{}", a.key, a.detail)
}

/// How many transitions the in-TUI timeline keeps.
const HISTORY_CAP: usize = 64;

/// Default flap window: an alert that re-fires this soon after its last
/// announcement is tracked silently instead of announced again.
pub const DEFAULT_FLAP_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
struct ActiveAlert {
    key: &'static str,
    detail: String,
    /// Whether this firing was announced — a flap-suppressed fire must not
    /// produce a resolve either.
    announced: bool,
}

/// Turns per-tick alert sets into debounced fire/resolve transitions and keeps
/// a bounded history of them for the TUI timeline.
#[derive(Clone, Debug)]
pub struct AlertTracker {
    active: HashMap<String, ActiveAlert>,
    /// Last announced fire per alert id, for flap suppression.
    last_fire: HashMap<String, Instant>,
    flap_window: Duration,
    history: VecDeque<Transition>,
    suppressed: u64,
}

impl Default for AlertTracker {
    fn default() -> Self {
        Self::new(DEFAULT_FLAP_WINDOW)
    }
}

impl AlertTracker {
    pub fn new(flap_window: Duration) -> Self {
        Self {
            active: HashMap::new(),
            last_fire: HashMap::new(),
            flap_window,
            history: VecDeque::new(),
            suppressed: 0,
        }
    }

    /// Feed one tick's alerts and get the transitions to announce.
    ///
    /// `now` is passed in rather than read from the clock so the debounce is
    /// testable. Announced transitions are appended to the history.
    pub fn update(&mut self, alerts: &[Alert], now: Instant) -> Vec<Transition> {
        let mut out = Vec::new();

        // Fires: alerts present now that weren't active before.
        for a in alerts {
            let id = alert_id(a);
            if self.active.contains_key(&id) {
                continue;
            }
            let announced = match self.last_fire.get(&id) {
                Some(prev) => now.duration_since(*prev) >= self.flap_window,
                None => true,
            };
            self.active.insert(
                id.clone(),
                ActiveAlert {
                    key: a.key,
                    detail: a.detail.clone(),
                    announced,
                },
            );
            if announced {
                self.last_fire.insert(id, now);
                out.push(Transition {
                    state: TransitionState::Fired,
                    level: a.level,
                    key: a.key,
                    detail: a.detail.clone(),
                    message: a.message.clone(),
                    at: now,
                });
            } else {
                self.suppressed += 1;
            }
        }

        // Resolves: alerts that were active and are gone.
        let current: Vec<String> = alerts.iter().map(alert_id).collect();
        let gone: Vec<String> = self
            .active
            .keys()
            .filter(|id| !current.contains(id))
            .cloned()
            .collect();
        for id in gone {
            let was = self.active.remove(&id).expect("just listed");
            if !was.announced {
                self.suppressed += 1;
                continue;
            }
            out.push(Transition {
                state: TransitionState::Resolved,
                // A resolve is good news; severity carries the fire's meaning,
                // so it is reported at the lower level.
                level: Level::Warn,
                key: was.key,
                message: format!("{} {} resolved", was.key, was.detail),
                detail: was.detail,
                at: now,
            });
        }

        for t in &out {
            if self.history.len() == HISTORY_CAP {
                self.history.pop_front();
            }
            self.history.push_back(t.clone());
        }
        out
    }

    /// Recent transitions, oldest first.
    pub fn history(&self) -> &VecDeque<Transition> {
        &self.history
    }

    /// How many transitions were swallowed by flap suppression.
    pub fn suppressed(&self) -> u64 {
        self.suppressed
    }
}

// ── Notification sinks ───────────────────────────────────────────────────────

/// A built-in notification target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkKind {
    /// Generic JSON webhook.
    Webhook,
    /// ntfy.sh (or a self-hosted ntfy) topic URL.
    Ntfy,
    /// Slack incoming webhook.
    Slack,
}

impl SinkKind {
    pub fn config_key(self) -> &'static str {
        match self {
            SinkKind::Webhook => "alert_webhook",
            SinkKind::Ntfy => "alert_ntfy",
            SinkKind::Slack => "alert_slack",
        }
    }
}

/// A ready-to-send HTTP request. Building it is pure, so the payload shapes are
/// unit-tested without a network; `crate::notify` does the sending.
#[derive(Clone, Debug, PartialEq)]
pub struct SinkRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Escape a string for embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
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

/// Build the request one sink should receive for a transition.
pub fn sink_request(kind: SinkKind, url: &str, t: &Transition) -> SinkRequest {
    let json = |k: &str, v: &str| format!("\"{k}\":\"{}\"", json_escape(v));
    match kind {
        SinkKind::Webhook => SinkRequest {
            url: url.to_string(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: format!(
                "{{{},{},{},{},{}}}",
                json("state", t.state.label()),
                json("severity", t.level.label()),
                json("key", t.key),
                json("detail", &t.detail),
                json("message", &t.message),
            ),
        },
        // ntfy takes a plain-text body with metadata in headers.
        SinkKind::Ntfy => SinkRequest {
            url: url.to_string(),
            headers: vec![
                (
                    "Title".into(),
                    format!("toptop: {} {}", t.key, t.state.label()),
                ),
                (
                    "Priority".into(),
                    match (t.state, t.level) {
                        (TransitionState::Resolved, _) => "low".into(),
                        (_, Level::Crit) => "urgent".into(),
                        (_, Level::Warn) => "default".into(),
                    },
                ),
                (
                    "Tags".into(),
                    match t.state {
                        TransitionState::Fired => "warning".into(),
                        TransitionState::Resolved => "white_check_mark".into(),
                    },
                ),
            ],
            body: t.message.clone(),
        },
        SinkKind::Slack => SinkRequest {
            url: url.to_string(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: format!(
                "{{{}}}",
                json(
                    "text",
                    &format!(
                        "{} *{}* — {}",
                        match t.state {
                            TransitionState::Fired => ":rotating_light:",
                            TransitionState::Resolved => ":white_check_mark:",
                        },
                        t.state.label(),
                        t.message
                    )
                )
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::gpu::Gpu;
    use crate::metrics::ServerStats;

    fn gpu(mem_used: u64, mem_total: u64, throttled: bool) -> Gpu {
        Gpu {
            name: "TestGPU".into(),
            util_pct: 50.0,
            has_util: true,
            mem_util: 50.0,
            has_mem_util: true,
            mem_used,
            mem_total,
            temp: 60.0,
            power: 200.0,
            power_limit: 400.0,
            throttled,
        }
    }

    #[test]
    fn fires_vram_spill_and_throttle() {
        let mut c = Collector::new(16);
        c.gpus = vec![gpu(95, 100, true)];
        let a = evaluate(&c, &AlertConfig::default());
        let keys: Vec<_> = a.iter().map(|x| x.key).collect();
        assert!(keys.contains(&"gpu_throttle"));
        assert!(keys.contains(&"vram_spill"));
        assert_eq!(worst_level(&a), Some(Level::Crit));
    }

    #[test]
    fn no_alerts_when_healthy() {
        let mut c = Collector::new(16);
        c.gpus = vec![gpu(40, 100, false)];
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            kv_pct: Some(30.0),
            waiting: Some(0.0),
            ..Default::default()
        }];
        assert!(evaluate(&c, &AlertConfig::default()).is_empty());
    }

    #[test]
    fn custom_thresholds_shift_the_trigger_point() {
        let mut c = Collector::new(16);
        c.gpus = vec![gpu(60, 100, false)];
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            kv_pct: Some(80.0),
            waiting: Some(3.0),
            ..Default::default()
        }];
        // Default thresholds: nothing fires at these levels.
        assert!(evaluate(&c, &AlertConfig::default()).is_empty());
        // Tightened thresholds: all three rules fire.
        let tight = AlertConfig {
            vram_spill_pct: 50.0,
            kv_high_pct: 75.0,
            queue_high: 2.0,
            ..AlertConfig::default()
        };
        let keys: Vec<_> = evaluate(&c, &tight).iter().map(|x| x.key).collect();
        assert!(keys.contains(&"vram_spill"));
        assert!(keys.contains(&"kv_high"));
        assert!(keys.contains(&"queue_backlog"));
    }

    fn alert(key: &'static str, detail: &str) -> Alert {
        Alert {
            level: Level::Warn,
            key,
            detail: detail.into(),
            message: format!("{key} {detail}"),
        }
    }

    #[test]
    fn detects_fire_and_resolve_transitions() {
        let mut t = AlertTracker::default();
        let t0 = Instant::now();
        let a = vec![alert("kv_high", "vLLM:8000")];

        let first = t.update(&a, t0);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].state, TransitionState::Fired);
        assert_eq!(first[0].id(), "kv_high:vLLM:8000");

        // Still firing on the next tick is not a new transition.
        assert!(t.update(&a, t0 + Duration::from_secs(2)).is_empty());

        let gone = t.update(&[], t0 + Duration::from_secs(4));
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].state, TransitionState::Resolved);
        assert_eq!(gone[0].key, "kv_high");
        assert_eq!(gone[0].detail, "vLLM:8000");

        // Fire and resolve are both in the timeline.
        assert_eq!(t.history().len(), 2);
    }

    #[test]
    fn flapping_is_suppressed_in_both_directions() {
        let mut t = AlertTracker::new(Duration::from_secs(60));
        let t0 = Instant::now();
        let a = vec![alert("gpu_throttle", "gpu0")];

        assert_eq!(t.update(&a, t0).len(), 1);
        assert_eq!(t.update(&[], t0 + Duration::from_secs(1)).len(), 1);

        // Re-firing inside the window is tracked but not announced …
        assert!(t.update(&a, t0 + Duration::from_secs(2)).is_empty());
        // … and its resolve must not be announced either, or the timeline
        // would show a resolve with no matching fire.
        assert!(t.update(&[], t0 + Duration::from_secs(3)).is_empty());
        assert_eq!(t.suppressed(), 2);
        assert_eq!(t.history().len(), 2, "only the first pair was announced");

        // Past the window it is news again.
        let later = t0 + Duration::from_secs(120);
        assert_eq!(t.update(&a, later).len(), 1);
    }

    #[test]
    fn independent_alerts_do_not_mask_each_other() {
        let mut t = AlertTracker::default();
        let t0 = Instant::now();
        let both = vec![alert("kv_high", "a"), alert("kv_high", "b")];
        assert_eq!(t.update(&both, t0).len(), 2);
        // Only `b` clears.
        let out = t.update(&both[..1], t0 + Duration::from_secs(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].detail, "b");
        assert_eq!(out[0].state, TransitionState::Resolved);
    }

    #[test]
    fn history_is_bounded() {
        let mut t = AlertTracker::new(Duration::ZERO);
        let t0 = Instant::now();
        for i in 0..HISTORY_CAP + 10 {
            let at = t0 + Duration::from_secs(i as u64);
            t.update(&[alert("kv_high", "x")], at);
            t.update(&[], at);
        }
        assert_eq!(t.history().len(), HISTORY_CAP);
    }

    fn transition(state: TransitionState, level: Level, msg: &str) -> Transition {
        Transition {
            state,
            level,
            key: "vram_spill",
            detail: "gpu0".into(),
            message: msg.into(),
            at: Instant::now(),
        }
    }

    #[test]
    fn webhook_payload_is_valid_json() {
        let t = transition(TransitionState::Fired, Level::Warn, "gpu0 VRAM 95%");
        let r = sink_request(SinkKind::Webhook, "http://localhost:9000/hook", &t);
        assert_eq!(r.url, "http://localhost:9000/hook");
        assert_eq!(
            r.body,
            r#"{"state":"fired","severity":"warn","key":"vram_spill","detail":"gpu0","message":"gpu0 VRAM 95%"}"#
        );
        assert!(r
            .headers
            .contains(&("Content-Type".into(), "application/json".into())));
    }

    #[test]
    fn payloads_escape_and_reflect_severity() {
        // A message with a quote and a newline must not break the JSON.
        let t = transition(TransitionState::Fired, Level::Crit, "say \"hi\"\nnow");
        let r = sink_request(SinkKind::Slack, "https://hooks.slack.com/x", &t);
        assert!(r.body.contains(r#"say \"hi\"\nnow"#), "got {}", r.body);
        assert!(r.body.starts_with(r#"{"text":":rotating_light: *fired*"#));

        let n = sink_request(SinkKind::Ntfy, "https://ntfy.sh/toptop", &t);
        assert_eq!(n.body, "say \"hi\"\nnow", "ntfy takes plain text");
        assert!(n.headers.contains(&("Priority".into(), "urgent".into())));

        // A resolve is quieter and differently tagged.
        let ok = transition(TransitionState::Resolved, Level::Warn, "cleared");
        let n = sink_request(SinkKind::Ntfy, "https://ntfy.sh/toptop", &ok);
        assert!(n.headers.contains(&("Priority".into(), "low".into())));
        assert!(n
            .headers
            .contains(&("Tags".into(), "white_check_mark".into())));
    }

    #[test]
    fn preemption_is_critical_and_thresholded() {
        let mut c = Collector::new(16);
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            preempt_rate: Some(1.5),
            ..Default::default()
        }];
        let a = evaluate(&c, &AlertConfig::default());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].key, "kv_preemption");
        assert_eq!(
            a[0].level,
            Level::Crit,
            "preemption throws away completed work — it outranks a queue backlog"
        );

        // A server that has simply never preempted must stay silent.
        c.servers[0].preempt_rate = Some(0.0);
        assert!(evaluate(&c, &AlertConfig::default()).is_empty());
        c.servers[0].preempt_rate = None;
        assert!(evaluate(&c, &AlertConfig::default()).is_empty());
    }

    #[test]
    fn fires_kv_and_queue() {
        let mut c = Collector::new(16);
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            kv_pct: Some(98.0),
            waiting: Some(12.0),
            ..Default::default()
        }];
        let keys: Vec<_> = evaluate(&c, &AlertConfig::default())
            .iter()
            .map(|x| x.key)
            .collect();
        assert!(keys.contains(&"kv_high"));
        assert!(keys.contains(&"queue_backlog"));
    }
}
