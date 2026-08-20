//! "Why is it slow?" — turning the metrics into a verdict.
//!
//! Every panel in toptop reports numbers. This module reads them together and
//! says what they *mean*, because the diagnostic value of local-inference
//! telemetry is almost entirely in the combinations: a GPU at 30% compute is
//! meaningless on its own, damning next to 95% memory bandwidth, and irrelevant
//! next to a server that is preempting.
//!
//! Every rule is a pure function of a [`Collector`] snapshot, so the whole
//! thing is unit-testable without a GPU. The rules are deliberately
//! conservative: each fires only on evidence toptop actually has, states that
//! evidence, and says what to change. When nothing fires, that is also an
//! answer.

use crate::alerts::Level;
use crate::metrics::Collector;

/// One diagnosis: what is happening, what the numbers say, what to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub severity: Level,
    /// Short verdict, e.g. `MEMORY-BANDWIDTH BOUND`.
    pub headline: &'static str,
    /// The numbers this verdict rests on, so it can be checked rather than
    /// believed.
    pub evidence: String,
    /// What to change. Concrete where the fix is unambiguous, hedged where it
    /// depends on the workload.
    pub advice: &'static str,
}

/// Thresholds for the rules, so they can be reasoned about in one place rather
/// than being scattered as literals.
mod t {
    /// VRAM % at which layers are about to spill (or already are).
    pub const VRAM_CRITICAL: f32 = 97.0;
    /// Memory-bandwidth % that counts as saturated.
    pub const BANDWIDTH_HIGH: f32 = 80.0;
    /// Compute % below which the SMs are clearly not the limit.
    pub const COMPUTE_IDLE: f32 = 50.0;
    /// Compute % that counts as saturated.
    pub const COMPUTE_HIGH: f32 = 85.0;
    /// Bandwidth % below which memory is clearly not the limit.
    pub const BANDWIDTH_LOW: f32 = 60.0;
    /// Compute % below which a GPU with queued work is plainly under-fed.
    pub const COMPUTE_STARVED: f32 = 40.0;
    /// CPU % at which a training process is pinning a core.
    pub const CPU_PINNED: f32 = 95.0;
}

/// Diagnose the current snapshot, most explanatory finding first.
///
/// Returns an empty vector only when there is no GPU to reason about; a healthy
/// system gets an explicit "nothing wrong" finding, because silence is
/// indistinguishable from a broken feature.
pub fn diagnose(c: &Collector) -> Vec<Finding> {
    let mut out = Vec::new();
    if c.gpus.is_empty() {
        return out;
    }

    // Aggregate across GPUs: the worst one is what limits the pipeline.
    let vram_pct = c
        .gpus
        .iter()
        .filter(|g| g.mem_total > 0)
        .map(|g| g.mem_pct())
        .fold(f32::NAN, f32::max);
    let compute = c
        .gpus
        .iter()
        .filter(|g| g.has_util)
        .map(|g| g.util_pct)
        .fold(f32::NAN, f32::max);
    let bandwidth = c
        .gpus
        .iter()
        .filter(|g| g.has_mem_util)
        .map(|g| g.mem_util)
        .fold(f32::NAN, f32::max);
    let throttled = c.gpus.iter().any(|g| g.throttled);

    // 1. VRAM exhaustion outranks everything: once layers spill to system RAM
    //    every other number is measured on a crippled pipeline.
    if vram_pct >= t::VRAM_CRITICAL {
        out.push(Finding {
            severity: Level::Crit,
            headline: "VRAM EXHAUSTED",
            evidence: format!("vram {vram_pct:.0}%"),
            advice: "Layers spill to system RAM at 5–20× the latency. Use a \
                     smaller quantization, fewer GPU layers, or a shorter \
                     context before tuning anything else.",
        });
    }

    // 2. Preemption: the server is destroying work it already did.
    if let Some((tag, rate)) = c
        .servers
        .iter()
        .filter_map(|s| s.preempt_rate.map(|r| (s.label(), r)))
        .filter(|(_, r)| *r > 0.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    {
        out.push(Finding {
            severity: Level::Crit,
            headline: "KV CACHE THRASHING",
            evidence: format!("{tag} preempting {rate:.1}/s"),
            advice: "Requests are being evicted mid-flight and recomputed. \
                     Lower the max concurrent sequences, or give the cache more \
                     room (higher gpu-memory-utilization, shorter max context).",
        });
    }

    // 3. Queued work with an idle GPU: a batching problem, not a hardware one.
    let waiting: f64 = c.servers.iter().filter_map(|s| s.waiting).sum();
    if waiting > 0.0 && compute.is_finite() && compute < t::COMPUTE_STARVED {
        out.push(Finding {
            severity: Level::Warn,
            headline: "QUEUE-BOUND, GPU UNDER-FED",
            evidence: format!("{waiting:.0} queued · compute {compute:.0}%"),
            advice: "Requests are waiting while the GPU idles — the limit is \
                     scheduling, not silicon. Raise the max concurrent \
                     sequences or batch size.",
        });
    }

    // 4/5. The bandwidth-vs-compute split, the AI view's whole reason to exist.
    if bandwidth.is_finite() && compute.is_finite() {
        if bandwidth >= t::BANDWIDTH_HIGH && compute < t::COMPUTE_IDLE {
            out.push(Finding {
                severity: Level::Warn,
                headline: "MEMORY-BANDWIDTH BOUND",
                evidence: format!("bandwidth {bandwidth:.0}% · compute {compute:.0}%"),
                advice: "Token generation is limited by memory bandwidth, not \
                         compute — a faster GPU core would not help. Quantize \
                         further, or batch more requests so each weight read \
                         serves more tokens.",
            });
        } else if compute >= t::COMPUTE_HIGH && bandwidth < t::BANDWIDTH_LOW {
            out.push(Finding {
                severity: Level::Warn,
                headline: "COMPUTE BOUND",
                evidence: format!("compute {compute:.0}% · bandwidth {bandwidth:.0}%"),
                advice: "The SMs are saturated — typical while prefilling long \
                         prompts. Shorter prompts, prefix caching, or a faster \
                         GPU are the levers here.",
            });
        }
    }

    // 6. Throttling makes every other measurement pessimistic.
    if throttled {
        out.push(Finding {
            severity: Level::Crit,
            headline: "GPU THROTTLING",
            evidence: c
                .gpus
                .iter()
                .enumerate()
                .filter(|(_, g)| g.throttled)
                .map(|(i, g)| {
                    format!(
                        "gpu{i} {:.0}°C {:.0}/{:.0}W",
                        g.temp, g.power, g.power_limit
                    )
                })
                .collect::<Vec<_>>()
                .join(" · "),
            advice: "Clocks are being cut for heat or power. Every other number \
                     here is measured on a throttled card — fix airflow or the \
                     power limit before drawing conclusions.",
        });
    }

    // 7. A training process pinning a CPU while the GPU idles: the classic
    //    data-loader bottleneck.
    if compute.is_finite() && compute < t::COMPUTE_STARVED {
        if let Some(p) = c.procs.iter().filter(|p| p.cpu > t::CPU_PINNED).find(|p| {
            crate::metrics::ai::detect_runtime(&p.name, &p.cmd)
                .is_some_and(|rt| rt.kind == crate::metrics::ai::AiKind::Training)
        }) {
            out.push(Finding {
                severity: Level::Warn,
                headline: "DATA-LOADER BOUND",
                evidence: format!("{} at {:.0}% CPU · compute {compute:.0}%", p.name, p.cpu),
                advice: "A training process is pinning a CPU core while the GPU \
                         waits. More dataloader workers, or preprocessing done \
                         ahead of time.",
            });
        }
    }

    if out.is_empty() {
        let mut evidence = Vec::new();
        if compute.is_finite() {
            evidence.push(format!("compute {compute:.0}%"));
        }
        if bandwidth.is_finite() {
            evidence.push(format!("bandwidth {bandwidth:.0}%"));
        }
        if vram_pct.is_finite() {
            evidence.push(format!("vram {vram_pct:.0}%"));
        }
        out.push(Finding {
            severity: Level::Warn,
            headline: "NOTHING OBVIOUSLY WRONG",
            evidence: evidence.join(" · "),
            advice: "No bottleneck signature in the current sample. If it still \
                     feels slow, watch the compute-vs-bandwidth trend under a \
                     sustained load rather than at idle.",
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::gpu::Gpu;
    use crate::metrics::ServerStats;

    fn gpu(util: f32, mem_util: f32, used: u64, total: u64) -> Gpu {
        Gpu {
            name: "TestGPU".into(),
            util_pct: util,
            has_util: true,
            mem_util,
            has_mem_util: true,
            mem_used: used,
            mem_total: total,
            temp: 60.0,
            power: 200.0,
            power_limit: 400.0,
            throttled: false,
        }
    }

    fn headlines(c: &Collector) -> Vec<&'static str> {
        diagnose(c).into_iter().map(|f| f.headline).collect()
    }

    #[test]
    fn no_gpu_means_no_verdict() {
        let mut c = Collector::new(8);
        c.gpus.clear();
        assert!(diagnose(&c).is_empty(), "nothing to reason about");
    }

    #[test]
    fn bandwidth_bound_is_the_defining_case() {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(31.0, 94.0, 50, 100)];
        c.servers.clear();
        let f = diagnose(&c);
        assert_eq!(f[0].headline, "MEMORY-BANDWIDTH BOUND");
        assert!(f[0].evidence.contains("bandwidth 94%"));
        assert!(f[0].evidence.contains("compute 31%"));
        assert!(
            f[0].advice.contains("faster GPU core would not help"),
            "the advice must say what NOT to buy"
        );
    }

    #[test]
    fn compute_bound_is_the_mirror_case() {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(92.0, 40.0, 50, 100)];
        c.servers.clear();
        assert_eq!(headlines(&c), vec!["COMPUTE BOUND"]);
    }

    #[test]
    fn a_balanced_gpu_is_neither() {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(70.0, 70.0, 50, 100)];
        c.servers.clear();
        // Neither signature fits, and silence would be indistinguishable from
        // a broken feature.
        assert_eq!(headlines(&c), vec!["NOTHING OBVIOUSLY WRONG"]);
    }

    #[test]
    fn vram_exhaustion_outranks_everything() {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(31.0, 94.0, 99, 100)];
        c.servers.clear();
        let h = headlines(&c);
        assert_eq!(h[0], "VRAM EXHAUSTED", "it invalidates the other numbers");
        assert!(h.contains(&"MEMORY-BANDWIDTH BOUND"));
    }

    #[test]
    fn preemption_and_queue_starvation_are_distinguished() {
        let mut c = Collector::new(8);
        c.gpus = vec![gpu(90.0, 90.0, 50, 100)];
        c.servers = vec![ServerStats {
            runtime: "vLLM",
            port: 8000,
            preempt_rate: Some(2.0),
            waiting: Some(12.0),
            ..Default::default()
        }];
        // Busy GPU + preemption: thrashing, not under-feeding.
        let h = headlines(&c);
        assert!(h.contains(&"KV CACHE THRASHING"));
        assert!(!h.contains(&"QUEUE-BOUND, GPU UNDER-FED"));

        // Idle GPU + queue, no preemption: under-fed, not thrashing.
        c.gpus = vec![gpu(10.0, 10.0, 50, 100)];
        c.servers[0].preempt_rate = None;
        let h = headlines(&c);
        assert!(h.contains(&"QUEUE-BOUND, GPU UNDER-FED"));
        assert!(!h.contains(&"KV CACHE THRASHING"));
    }

    #[test]
    fn throttling_is_always_reported() {
        let mut c = Collector::new(8);
        let mut g = gpu(70.0, 70.0, 50, 100);
        g.throttled = true;
        c.gpus = vec![g];
        c.servers.clear();
        let f = diagnose(&c);
        assert_eq!(f[0].headline, "GPU THROTTLING");
        assert!(f[0].evidence.contains("gpu0"));
        assert_eq!(f[0].severity, Level::Crit);
    }

    #[test]
    fn a_gpu_reporting_nothing_yields_no_false_verdict() {
        let mut c = Collector::new(8);
        let mut g = gpu(0.0, 0.0, 0, 0);
        g.has_util = false;
        g.has_mem_util = false;
        c.gpus = vec![g];
        c.servers.clear();
        // Apple Silicon and integrated GPUs report neither; inventing a
        // "bandwidth bound" verdict from missing data would be worse than
        // saying nothing.
        assert_eq!(headlines(&c), vec!["NOTHING OBVIOUSLY WRONG"]);
    }
}
