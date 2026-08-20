//! Integration tests that render the full UI through ratatui's headless
//! `TestBackend`. These catch layout/geometry panics (overflow, zero-size
//! areas, out-of-bounds writes) across a range of terminal sizes without
//! needing a real terminal.

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use toptop::app::App;
use toptop::config::Config;
use toptop::ui;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn render_at(app: &mut App, w: u16, h: u16) {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| ui::draw(f, app))
        .expect("draw must not panic");
}

#[test]
fn renders_across_many_sizes() {
    let mut app = App::new(&Config::default());
    // Includes degenerate and tiny sizes that exercise the saturating math.
    for (w, h) in [
        (1, 1),
        (2, 2),
        (10, 5),
        (40, 12),
        (80, 24),
        (120, 40),
        (200, 60),
        (250, 80),
    ] {
        render_at(&mut app, w, h);
    }
}

#[test]
fn overlays_render() {
    let mut app = App::new(&Config::default());

    app.show_help = true;
    render_at(&mut app, 100, 30);
    app.show_help = false;

    // Process detail overlay.
    app.on_key(key(KeyCode::Enter));
    assert!(app.show_detail);
    render_at(&mut app, 100, 30);
    app.on_key(key(KeyCode::Enter));
    assert!(!app.show_detail);

    // Signal menu → choose a signal → kill-confirm modal.
    app.on_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::empty()));
    assert!(app.signal_menu.is_some());
    render_at(&mut app, 100, 30);
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    assert!(app.signal_menu.is_none());
    assert!(app.pending_kill.is_some());
    render_at(&mut app, 100, 30);

    // Cancel the confirmation.
    app.on_key(key(KeyCode::Esc));
    assert!(app.pending_kill.is_none());

    // Network connections view: open, scroll, render, close.
    app.on_key(key(KeyCode::Char('n')));
    assert!(app.show_conn);
    render_at(&mut app, 120, 40);
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::PageDown));
    app.on_key(key(KeyCode::Char('G')));
    render_at(&mut app, 120, 40);
    app.on_key(key(KeyCode::Esc));
    assert!(!app.show_conn);
}

#[test]
fn demo_mode_populates_and_renders() {
    let mut app = App::new(&Config::default());
    app.demo = true;
    app.show_ai = true;
    app.on_tick();
    assert!(!app.collector.gpus.is_empty(), "demo must synthesize a GPU");
    assert!(
        !app.collector.servers.is_empty(),
        "demo must synthesize a server"
    );
    for (w, h) in [(100, 30), (60, 14)] {
        render_at(&mut app, w, h);
    }
    // Tick through enough frames to hit the alert-firing states.
    let mut saw_alert = false;
    for _ in 0..40 {
        app.on_tick();
        saw_alert |= !app.alerts.is_empty();
    }
    assert!(
        saw_alert,
        "demo should trip at least one alert within 40 ticks"
    );
    render_at(&mut app, 100, 30);
}

#[test]
fn ai_view_renders() {
    use toptop::metrics::gpu::{Gpu, GpuProc};
    let mut app = App::new(&Config::default());

    // Empty-state (no GPU) path.
    app.show_ai = true;
    render_at(&mut app, 100, 30);

    // Populated path: inject a near-full, throttling GPU and a compute process.
    app.collector.gpus = vec![Gpu {
        name: "NVIDIA GeForce RTX 4090".into(),
        util_pct: 96.0,
        has_util: true,
        mem_util: 71.0,
        has_mem_util: true,
        mem_used: 23 * 1024 * 1024 * 1024,
        mem_total: 24 * 1024 * 1024 * 1024,
        temp: 84.0,
        power: 410.0,
        power_limit: 450.0,
        throttled: true,
    }];
    app.collector.gpu_procs = vec![GpuProc {
        pid: app.collector.procs.first().map(|p| p.pid).unwrap_or(1),
        used_mem: 22 * 1024 * 1024 * 1024,
    }];
    // A discovered inference server with live tokens/sec.
    app.collector.servers = vec![toptop::metrics::ServerStats {
        runtime: "vLLM",
        pid: 4242,
        port: 8000,
        model: "meta-llama/Llama-3-8B".into(),
        gen_tps: Some(83.4),
        prompt_tps: Some(1200.0),
        running: Some(2.0),
        waiting: Some(5.0),
        kv_pct: Some(64.0),
        ttft_ms: Some(180.0),
        gpu_offload_pct: None,
    }];
    render_at(&mut app, 100, 30);
    render_at(&mut app, 60, 14); // cramped
    app.on_key(key(KeyCode::Char('a')));
    assert!(!app.show_ai);
}

#[test]
fn ai_view_renders_history_sparklines() {
    use toptop::history::History;
    use toptop::metrics::ServerHistory;

    let mut app = App::new(&Config::default());
    app.show_ai = true;
    app.collector.servers = vec![toptop::metrics::ServerStats {
        runtime: "vLLM",
        pid: 4242,
        port: 8000,
        model: "meta-llama/Llama-3-8B".into(),
        gen_tps: Some(83.4),
        kv_pct: Some(64.0),
        ..Default::default()
    }];
    let mut tps = History::new(64);
    let mut kv = History::new(64);
    for i in 0..40 {
        tps.push(40.0 + (i % 10) as f64 * 5.0);
        kv.push(30.0 + i as f64);
    }
    app.collector
        .server_history
        .insert((4242, 8000), ServerHistory { tps, kv });

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw must not panic");
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("tok/s"), "sparkline label must render");
    let has_braille = content
        .chars()
        .any(|ch| ('\u{2800}'..='\u{28FF}').contains(&ch));
    assert!(has_braille, "sparkline must draw braille cells");

    // Degenerate sizes must not panic (sparklines are skipped when cramped).
    render_at(&mut app, 40, 10);
    render_at(&mut app, 12, 4);
}

#[test]
fn layout_presets_render() {
    let mut app = App::new(&Config::default());
    // Cycle through every layout preset and render the full body each time.
    for _ in 0..4 {
        app.on_key(key(KeyCode::Char('L')));
        render_at(&mut app, 120, 40);
        render_at(&mut app, 70, 18);
    }
}

#[test]
fn connections_collect_without_panic() {
    // Exercises the real /proc parsing + inode→pid mapping on this host.
    let app = App::new(&Config::default());
    let conns = app.collector.connections();
    // Every returned row must have non-empty protocol/address strings.
    for c in conns.iter().take(50) {
        assert!(!c.proto.is_empty());
        assert!(!c.local.is_empty());
    }
}

#[test]
fn header_sort_mapping() {
    use toptop::app::{header_sort_at, SortField};
    assert_eq!(header_sort_at(0), Some(SortField::Pid));
    assert_eq!(header_sort_at(10), Some(SortField::User));
    assert_eq!(header_sort_at(20), Some(SortField::Cpu));
    assert_eq!(header_sort_at(26), Some(SortField::Mem));
    assert_eq!(header_sort_at(40), Some(SortField::Io));
    assert_eq!(header_sort_at(50), Some(SortField::Gpu));
    assert_eq!(header_sort_at(58), Some(SortField::Time));
    assert_eq!(header_sort_at(63), None);
    assert_eq!(header_sort_at(70), Some(SortField::Name));
}

#[test]
fn interaction_flow_is_stable() {
    let mut app = App::new(&Config::default());

    // Navigate, sort, toggle views, theme, filter — then render after each.
    let actions = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Char('s'),
        KeyCode::Char('i'),
        KeyCode::Char('t'),
        KeyCode::Char('e'),
        KeyCode::Char('p'),
        KeyCode::Char('G'),
        KeyCode::Char('g'),
        KeyCode::PageDown,
    ];
    for code in actions {
        app.on_key(key(code));
        app.rebuild_proc_view();
        render_at(&mut app, 120, 40);
    }

    // Filter entry typing path.
    app.on_key(key(KeyCode::Char('/')));
    for c in "kernel".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    render_at(&mut app, 120, 40);
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Esc)); // clears filter
    assert!(app.filter.is_empty());

    // Cycle through every theme and re-render.
    for _ in 0..toptop::theme::THEMES.len() + 1 {
        app.on_key(key(KeyCode::Char('p')));
        render_at(&mut app, 120, 40);
    }
}

#[test]
fn fleet_view_renders() {
    use toptop::fleet::{FleetApp, HostState, HostStatus, HostSummary, ServerLine};

    // Empty host list spawns no monitor threads — safe to construct in a test.
    let mut app = FleetApp::new(vec![], "x".into(), 0);
    app.hosts = vec![
        HostState {
            name: "gpu-node-1".into(),
            status: HostStatus::Online,
            latency_ms: Some(42),
            summary: Some(HostSummary {
                hostname: "gpu-node-1".into(),
                os: "Linux (Ubuntu 24.04)".into(),
                cpu_usage: 37.5,
                mem_used: 34_359_738_368,
                mem_total: 68_719_476_736,
                load: (4.2, 3.1, 2.5),
                uptime: 3600,
                tasks: 420,
                running: 3,
                gpu_count: 2,
                gpu_util: Some(91.0),
                vram_used: 44_023_414_784,
                vram_total: 171_798_691_840,
                gpu_power: 400.0,
                total_tps: 58.3,
                servers: vec![ServerLine {
                    runtime: "vLLM".into(),
                    model: "meta-llama/Llama-3-70B".into(),
                    gen_tps: Some(58.3),
                    kv_pct: Some(72.0),
                }],
            }),
        },
        HostState {
            name: "offline-box".into(),
            status: HostStatus::Error("Connection refused".into()),
            latency_ms: Some(6000),
            summary: None,
        },
    ];

    for (w, h) in [(1, 1), (40, 12), (100, 30), (200, 50)] {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| ui::fleet::draw(f, &app))
            .expect("fleet draw must not panic");
    }

    // Navigation + theme cycling.
    app.on_key(KeyCode::Down);
    assert_eq!(app.selected, 1);
    app.on_key(KeyCode::Char('p'));
    app.on_key(KeyCode::Char('q'));
    assert!(app.should_quit);
}

#[test]
fn quit_keys_work() {
    let mut app = App::new(&Config::default());
    app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);

    let mut app = App::new(&Config::default());
    app.on_key(key(KeyCode::Char('q')));
    assert!(app.should_quit);
}

/// The flight recorder end to end: record real ticks to a file, load it back
/// as a replay, and drive the TUI off the recording.
#[test]
fn record_then_replay_drives_the_tui() {
    use toptop::record::{Recorder, Replay};

    let path = std::env::temp_dir().join(format!("toptop-replay-{}.jsonl", std::process::id()));
    std::fs::remove_file(&path).ok();

    // Record three ticks through the app, exactly as --record does.
    {
        let mut app = App::new(&Config::default());
        app.recorder = Some(Recorder::create(&path).expect("create recording"));
        for _ in 0..3 {
            app.on_tick();
        }
        assert!(app.recorder.is_some(), "recording must not have aborted");
    }

    let replay = Replay::load(&path).expect("load the recording we just wrote");
    assert_eq!(replay.len(), 3);
    assert_eq!(replay.skipped, 0);

    // Replay it: the app must render every frame and never touch live metrics.
    let mut app = App::new(&Config::default());
    app.replay = Some(replay);
    let hostname_before = app.collector.host.hostname.clone();
    for _ in 0..5 {
        app.on_tick();
        render_at(&mut app, 110, 36);
    }
    // Playback clamps at the last frame instead of wrapping.
    assert_eq!(app.replay.as_ref().unwrap().position(), 2);
    assert_eq!(app.collector.host.hostname, hostname_before);

    // Scrubbing: ←/→ step and imply pause.
    app.on_key(key(KeyCode::Left));
    assert!(app.paused, "stepping pauses playback");
    assert_eq!(app.replay.as_ref().unwrap().position(), 1);
    render_at(&mut app, 110, 36);
    app.on_key(key(KeyCode::Right));
    assert_eq!(app.replay.as_ref().unwrap().position(), 2);
    // Paused playback stays put.
    app.on_tick();
    assert_eq!(app.replay.as_ref().unwrap().position(), 2);

    std::fs::remove_file(&path).ok();
}
