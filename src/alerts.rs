//! Threshold alerts for local-inference health.
//!
//! Evaluating a [`Collector`] against an [`AlertConfig`] yields the set of
//! currently-firing [`Alert`]s — VRAM spill risk, GPU throttling, KV-cache
//! pressure, and request-queue backlog. The TUI surfaces these as a red banner,
//! and the Prometheus exporter emits them as `toptop_alert{…}` gauges so
//! Alertmanager can page on them. The evaluation is pure and unit-tested.

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
