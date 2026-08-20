//! `--demo` mode: synthesized GPU and inference-server activity.
//!
//! Everything else in the demo is real (your processes, CPU, memory, network);
//! only the GPU and inference-server data are simulated — so anyone, GPU or
//! not, can see the AI view fully alive in ten seconds: `toptop --demo`.
//!
//! The waveforms are deterministic functions of the tick counter, tuned so the
//! interesting states all occur within a minute: VRAM climbs to the spill
//! threshold and backs off, the KV cache saturates, the queue backs up, and
//! the GPU periodically reports throttling — each of which trips its alert.

use crate::metrics::gpu::{Gpu, GpuProc};
use crate::metrics::{Collector, Percentiles, ServerStats};

/// A sine oscillation between `lo` and `hi` with the given period (in ticks).
fn wave(t: u64, period: f64, lo: f64, hi: f64, phase: f64) -> f64 {
    let x = (t as f64 / period + phase) * std::f64::consts::TAU;
    lo + (hi - lo) * (0.5 + 0.5 * x.sin())
}

/// Overwrite the GPU/inference portion of `c` with demo data for tick `t`.
pub fn apply(c: &mut Collector, t: u64) {
    let gib = 1024.0 * 1024.0 * 1024.0;

    // VRAM climbs toward the 24 GiB wall and backs off, periodically crossing
    // the spill threshold so viewers see the warning fire and clear.
    let vram_used = wave(t, 24.0, 17.0, 23.4, 0.0) * gib;
    c.gpus = vec![Gpu {
        name: "NVIDIA GeForce RTX 4090 (demo)".into(),
        util_pct: wave(t, 9.0, 24.0, 46.0, 0.3) as f32,
        has_util: true,
        // Memory bandwidth runs hot while compute idles — the classic
        // "why is my model slow" signature.
        mem_util: wave(t, 7.0, 68.0, 93.0, 0.6) as f32,
        has_mem_util: true,
        mem_used: vram_used as u64,
        mem_total: (24.0 * gib) as u64,
        temp: wave(t, 30.0, 62.0, 79.0, 0.1) as f32,
        power: wave(t, 11.0, 240.0, 390.0, 0.8) as f32,
        power_limit: 450.0,
        throttled: t % 90 >= 60 && t % 90 < 72,
    }];

    // Attach the demo VRAM to the two busiest real processes so the
    // per-process VRAM table joins to names the viewer recognizes.
    let mut by_cpu: Vec<(u32, f32)> = c.procs.iter().map(|p| (p.pid, p.cpu)).collect();
    by_cpu.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    c.gpu_procs = by_cpu
        .iter()
        .take(2)
        .enumerate()
        .map(|(i, (pid, _))| GpuProc {
            pid: *pid,
            used_mem: (vram_used * if i == 0 { 0.80 } else { 0.15 }) as u64,
        })
        .collect();

    // `refresh` already sampled the *real* GPU into the history — which on a
    // demo machine reports nothing. Replace that sample rather than adding
    // one, or the trend would interleave an idle host card with the synthetic
    // 4090 and draw a comb.
    c.update_gpu_history_with(true);

    c.servers = vec![ServerStats {
        runtime: "vLLM",
        pid: by_cpu.first().map(|p| p.0).unwrap_or(1),
        port: 8000,
        model: "meta-llama/Llama-3-8B (demo)".into(),
        gen_tps: Some(wave(t, 6.0, 71.0, 92.0, 0.2)),
        prompt_tps: Some(wave(t, 8.0, 900.0, 1500.0, 0.5)),
        running: Some(wave(t, 10.0, 1.0, 4.0, 0.0).round()),
        waiting: Some(wave(t, 13.0, 0.0, 9.0, 0.7).round()),
        kv_pct: Some(wave(t, 17.0, 38.0, 97.0, 0.4)),
        ttft_ms: Some(wave(t, 9.0, 120.0, 260.0, 0.9)),
        // A realistic SLO triad: p95/p99 fan out from the median under load.
        ttft: Some(Percentiles {
            p50: wave(t, 9.0, 120.0, 260.0, 0.9),
            p95: wave(t, 9.0, 260.0, 700.0, 0.9),
            p99: wave(t, 9.0, 380.0, 1100.0, 0.9),
        }),
        tpot: Some(Percentiles {
            p50: wave(t, 11.0, 11.0, 16.0, 0.3),
            p95: wave(t, 11.0, 18.0, 34.0, 0.3),
            p99: wave(t, 11.0, 24.0, 52.0, 0.3),
        }),
        gpu_offload_pct: None,
        addr: None,
        // Once KV saturates, the demo server starts preempting — that is the
        // whole point of the story it tells.
        preemptions: Some((t as f64 / 4.0).floor()),
        preempt_rate: Some(if wave(t, 17.0, 38.0, 97.0, 0.4) > 92.0 {
            wave(t, 5.0, 0.5, 3.5, 0.1)
        } else {
            0.0
        }),
    }];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gpu_trend_reflects_the_demo_not_the_host() {
        let mut c = Collector::new(64);
        for t in 1..=10 {
            // Mimic App::on_tick: the real sample first, then the overlay.
            c.update_gpu_history_with(false);
            apply(&mut c, t);
        }
        let h = &c.gpu_history[0];
        let compute = h.compute.tail(10);
        assert_eq!(compute.len(), 10);
        // Appending instead of replacing would interleave the host's idle card
        // with the synthetic 4090 and draw a comb, which is what this guards.
        assert!(
            compute.iter().all(|v| *v > 0.0),
            "the trend picked up the host GPU: {compute:?}"
        );
        assert!(
            h.bandwidth.tail(10).iter().all(|v| *v > 50.0),
            "the demo is bandwidth-heavy by design"
        );
    }

    #[test]
    fn wave_stays_in_bounds() {
        for t in 0..200 {
            let v = wave(t, 7.0, 10.0, 20.0, 0.3);
            assert!((10.0..=20.0).contains(&v), "wave out of bounds: {v}");
        }
    }

    #[test]
    fn apply_populates_gpu_and_server() {
        let mut c = Collector::new(16);
        apply(&mut c, 6);
        assert_eq!(c.gpus.len(), 1);
        let g = &c.gpus[0];
        assert!(g.has_util && g.has_mem_util);
        assert!(g.mem_used > 0 && g.mem_used <= g.mem_total);
        assert_eq!(c.servers.len(), 1);
        let tps = c.servers[0].gen_tps.unwrap();
        assert!((71.0..=92.0).contains(&tps));
    }

    #[test]
    fn spill_and_throttle_states_occur() {
        let mut c = Collector::new(16);
        let mut spill = false;
        let mut throttle = false;
        for t in 0..120 {
            apply(&mut c, t);
            let g = &c.gpus[0];
            spill |= g.mem_pct() >= 90.0;
            throttle |= g.throttled;
        }
        assert!(spill, "demo never reaches the VRAM-spill threshold");
        assert!(throttle, "demo never throttles");
    }
}
