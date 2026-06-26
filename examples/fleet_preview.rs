//! Render the fleet dashboard to a headless buffer with sample hosts.
//!     cargo run --example fleet_preview -- 120 26

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use toptop::fleet::{FleetApp, HostState, HostStatus, HostSummary, ServerLine};
use toptop::ui;

#[allow(clippy::too_many_arguments)]
fn host(
    name: &str,
    cpu: f64,
    mem_u: u64,
    mem_t: u64,
    gpu_util: f64,
    vram_u: u64,
    vram_t: u64,
    power: f64,
    tps: f64,
    model: &str,
) -> HostState {
    HostState {
        name: name.into(),
        status: HostStatus::Online,
        latency_ms: Some(18),
        summary: Some(HostSummary {
            hostname: name.into(),
            os: "Linux (Ubuntu 24.04)".into(),
            cpu_usage: cpu,
            mem_used: mem_u,
            mem_total: mem_t,
            load: (cpu / 8.0, cpu / 10.0, cpu / 12.0),
            uptime: 86_400,
            tasks: 512,
            running: 4,
            gpu_count: 2,
            gpu_util: Some(gpu_util),
            vram_used: vram_u,
            vram_total: vram_t,
            gpu_power: power,
            total_tps: tps,
            servers: vec![ServerLine {
                runtime: "vLLM".into(),
                model: model.into(),
                gen_tps: Some(tps),
                kv_pct: Some(68.0),
            }],
        }),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let w: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);
    let h: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(26);

    let gb = 1024 * 1024 * 1024;
    let mut app = FleetApp::new(vec![], "x".into(), 0);
    app.hosts = vec![
        host(
            "gpu-node-1",
            37.0,
            34 * gb,
            64 * gb,
            91.0,
            41 * gb,
            80 * gb,
            620.0,
            58.3,
            "Llama-3-70B",
        ),
        host(
            "gpu-node-2",
            88.0,
            58 * gb,
            64 * gb,
            22.0,
            12 * gb,
            80 * gb,
            210.0,
            12.1,
            "Mixtral-8x7B",
        ),
        host(
            "inference-3",
            12.0,
            9 * gb,
            32 * gb,
            76.0,
            21 * gb,
            24 * gb,
            290.0,
            83.4,
            "Qwen2.5-7B",
        ),
        HostState {
            name: "offline-box".into(),
            status: HostStatus::Error("ssh: connect: Connection refused".into()),
            latency_ms: Some(6000),
            summary: None,
        },
    ];

    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| ui::fleet::draw(f, &app)).expect("draw");

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
