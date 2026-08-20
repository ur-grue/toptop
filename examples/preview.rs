//! Render one frame to a headless buffer and print it as plain text.
//!
//! Useful for eyeballing the layout in CI or a non-TTY shell:
//!     cargo run --example preview -- 120 40

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use toptop::app::App;
use toptop::config::Config;
use toptop::ui;

fn main() {
    let mut args = std::env::args().skip(1);
    let w: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);
    let h: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);

    let overlay = std::env::args().nth(3).unwrap_or_default();
    // Optional 4th arg: theme name, for eyeballing a palette.
    let theme = std::env::args().nth(4).unwrap_or_default();
    let mut cfg = Config::default();
    if let Some(idx) = toptop::theme::index_by_name(&theme) {
        cfg.theme_idx = idx;
    }
    let mut app = App::new(&cfg);
    // Take a couple of samples so meters and graphs have data.
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.on_tick();
    app.on_tick();
    match overlay.as_str() {
        // `--demo` is the first thing most visitors run; render what they see.
        "demo" => {
            app.demo = true;
            app.show_ai = true;
            for _ in 0..40 {
                app.on_tick();
            }
        }
        "conn" => {
            app.show_conn = true;
            app.connections = app.collector.connections();
        }
        "ai" => {
            app.show_ai = true;
            // Inject sample data so the populated layout is visible without a GPU.
            app.collector.gpus = vec![toptop::metrics::gpu::Gpu {
                name: "NVIDIA GeForce RTX 4090".into(),
                util_pct: 31.0,
                has_util: true,
                mem_util: 78.0,
                has_mem_util: true,
                mem_used: 22 * 1024 * 1024 * 1024,
                mem_total: 24 * 1024 * 1024 * 1024,
                temp: 72.0,
                power: 290.0,
                power_limit: 450.0,
                throttled: false,
            }];
            let pid = app.collector.procs.first().map(|p| p.pid).unwrap_or(4242);
            app.collector.gpu_procs = vec![toptop::metrics::gpu::GpuProc {
                pid,
                used_mem: 22 * 1024 * 1024 * 1024,
            }];
            app.collector.servers = vec![toptop::metrics::ServerStats {
                runtime: "vLLM",
                pid: 4242,
                port: 8000,
                model: "meta-llama/Llama-3-8B".into(),
                gen_tps: Some(83.4),
                prompt_tps: Some(1240.0),
                running: Some(2.0),
                waiting: Some(5.0),
                kv_pct: Some(64.0),
                ttft_ms: Some(180.0),
                ttft: Some(toptop::metrics::Percentiles {
                    p50: 180.0,
                    p95: 520.0,
                    p99: 910.0,
                }),
                tpot: Some(toptop::metrics::Percentiles {
                    p50: 12.0,
                    p95: 28.0,
                    p99: 47.0,
                }),
                gpu_offload_pct: None,
                addr: None,
                preemptions: Some(12.0),
                preempt_rate: Some(1.2),
            }];
            app.alerts = toptop::alerts::evaluate(&app.collector, &app.alert_cfg);
        }
        "detail" => app.show_detail = true,
        "signal" => {
            use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            app.on_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::empty()));
        }
        "help" => app.show_help = true,
        _ => {}
    }

    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| ui::draw(f, &mut app)).expect("draw");

    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    print!("{out}");
}
