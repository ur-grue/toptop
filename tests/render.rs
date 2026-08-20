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
    use toptop::app::{header_sort_at, SortField, DEFAULT_COLUMNS as C};
    assert_eq!(header_sort_at(C, 0), Some(SortField::Pid));
    assert_eq!(header_sort_at(C, 10), Some(SortField::User));
    assert_eq!(header_sort_at(C, 20), Some(SortField::Cpu));
    assert_eq!(header_sort_at(C, 26), Some(SortField::Mem));
    assert_eq!(header_sort_at(C, 40), Some(SortField::Io));
    assert_eq!(header_sort_at(C, 50), Some(SortField::Gpu));
    assert_eq!(header_sort_at(C, 58), Some(SortField::Time));
    assert_eq!(header_sort_at(C, 63), None);
    assert_eq!(header_sort_at(C, 70), Some(SortField::Name));

    // A custom column set remaps the ranges: pid(7+1) then command takes the rest.
    use toptop::app::ProcColumn;
    let custom = &[ProcColumn::Pid, ProcColumn::Command];
    assert_eq!(header_sort_at(custom, 0), Some(SortField::Pid));
    assert_eq!(header_sort_at(custom, 9), Some(SortField::Name));
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

    // Alert-history overlay: empty, then with transitions in it.
    app.on_key(key(KeyCode::Char('A')));
    assert!(app.show_alert_history);
    render_at(&mut app, 120, 40);
    {
        use std::time::Instant;
        use toptop::alerts::{Alert, Level};
        let a = vec![Alert {
            level: Level::Crit,
            key: "gpu_throttle",
            detail: "gpu0".into(),
            message: "gpu0 is throttling (TestGPU)".into(),
        }];
        let now = Instant::now();
        app.tracker.update(&a, now);
        app.tracker.update(&[], now);
    }
    render_at(&mut app, 120, 40);
    app.on_key(key(KeyCode::Esc));
    assert!(!app.show_alert_history, "Esc peels the overlay back");

    // Detail overlay: fetches open files/env/sockets for the selection and
    // renders at both a roomy and a cramped size.
    app.on_key(key(KeyCode::Enter));
    assert!(app.show_detail);
    assert!(
        app.detail.is_some(),
        "detail is fetched as the overlay opens"
    );
    render_at(&mut app, 120, 40);
    render_at(&mut app, 62, 16); // cramped: sections must not overflow
    app.on_key(key(KeyCode::Enter));
    assert!(!app.show_detail);
    assert!(
        app.detail.is_none(),
        "detail is dropped when the overlay closes"
    );

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
    for _ in 0..toptop::theme::themes().len() + 1 {
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

/// Render into a buffer and return it as lines of text, so tests can assert on
/// what a user actually sees rather than only that nothing panicked.
fn render_text(app: &mut App, w: u16, h: u16) -> Vec<String> {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| ui::draw(f, app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn unified_memory_gpus_are_not_shown_as_zero_bytes() {
    let mut app = App::new(&Config::default());
    // Apple Silicon: no discrete VRAM, and no temperature reported.
    app.collector.gpus = vec![toptop::metrics::gpu::Gpu {
        name: "Apple M3".into(),
        util_pct: 12.0,
        has_util: true,
        mem_util: 0.0,
        has_mem_util: false,
        mem_used: 0,
        mem_total: 0,
        temp: 0.0,
        power: 0.0,
        power_limit: 0.0,
        throttled: false,
    }];
    let screen = render_text(&mut app, 120, 40);

    // Scope the assertions to the GPU panel's own rows. A machine with no swap
    // legitimately renders "swp 0 B / 0 B" in the memory panel, and a host GPU
    // may well be at a real temperature — a whole-screen search would blame
    // this code for either.
    let gpu_rows: String = screen
        .iter()
        .skip_while(|l| !l.contains("gpu0"))
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !gpu_rows.is_empty(),
        "no GPU rows rendered:\n{}",
        screen.join("\n")
    );

    assert!(
        gpu_rows.contains("unified memory"),
        "a unified-memory GPU should say so:\n{gpu_rows}"
    );
    assert!(
        !gpu_rows.contains("0 B / 0 B"),
        "0 B / 0 B reads as a broken driver, not a memory architecture:\n{gpu_rows}"
    );
    assert!(
        !gpu_rows.contains("0°C"),
        "a GPU reporting no temperature must not look suspiciously cool:\n{gpu_rows}"
    );
}

#[test]
fn the_header_never_cuts_a_segment_in_half() {
    let mut app = App::new(&Config::default());
    // Widths around where segments start dropping out.
    for w in [40u16, 60, 72, 88, 100, 116, 140] {
        let header = render_text(&mut app, w, 12).remove(0);
        assert!(
            header.chars().count() <= w as usize,
            "header overflows at width {w}"
        );
        // "tasks" is the last segment before the clock; if it appears at all,
        // it must appear whole rather than as "563 tasks, 301 r".
        if let Some(rest) = header.split("tasks, ").nth(1) {
            assert!(
                rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains("run"),
                "header cut mid-segment at width {w}: {header:?}"
            );
        }
    }
}

#[test]
fn the_ai_overlay_has_no_dead_space() {
    let mut app = App::new(&Config::default());
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
    app.show_ai = true;
    let screen = render_text(&mut app, 116, 44);

    // Find the overlay's top and bottom borders by their corner glyphs.
    let top = screen
        .iter()
        .position(|l| l.contains("╭ AI ·"))
        .expect("overlay top border");
    let bottom = screen
        .iter()
        .skip(top)
        .position(|l| l.contains('╰') && l.contains('╯'))
        .expect("overlay bottom border")
        + top;

    // The row just above the bottom border must carry content — a panel sized
    // to its content has no empty rows before its own border.
    let last_row = &screen[bottom - 1];
    let inside: String = last_row
        .chars()
        .skip_while(|c| *c != '│')
        .skip(1)
        .take_while(|c| *c != '│')
        .collect();
    assert!(
        !inside.trim().is_empty(),
        "the AI overlay ends in dead space (rows {top}..{bottom}):\n{}",
        screen[top..=bottom].join("\n")
    );
}

/// One physical filesystem mounted several times (macOS firmlinks, Linux bind
/// mounts) must appear once — and, more importantly, must not have its I/O
/// counted twice into the disk rates.
#[test]
fn duplicate_mounts_are_deduplicated() {
    let c = toptop::metrics::Collector::new(16);
    let mut seen = std::collections::HashSet::new();
    for d in &c.disk_list {
        assert!(
            seen.insert((d.name.clone(), d.total)),
            "device {:?} listed twice (mounts collapse to one row): {:#?}",
            d.name,
            c.disk_list.iter().map(|d| &d.mount).collect::<Vec<_>>()
        );
    }
}

/// AI-workload detection is cached per tick, not redone per frame — the
/// difference was 7x on the AI view's render time. Guard the lifecycle so a
/// refactor can't quietly move it back into the render path.
#[test]
fn ai_workloads_are_cached_per_tick() {
    let mut app = App::new(&Config::default());
    app.on_tick();
    assert!(
        app.ai_workloads.is_empty(),
        "nothing is detected while the AI view is closed"
    );

    app.on_key(key(KeyCode::Char('a')));
    assert!(app.show_ai);
    // Opening the view populates immediately rather than one tick later.
    let detected = app.ai_workloads.len();
    app.on_tick();
    assert_eq!(app.ai_workloads.len(), detected, "stable across ticks");
    render_at(&mut app, 120, 40);

    app.on_key(key(KeyCode::Char('a')));
    assert!(!app.show_ai);
    assert!(
        app.ai_workloads.is_empty(),
        "closing the view drops the cache"
    );
}

/// The AI view must not clip facts at the panel border, and must not draw a
/// trend heading over blank rows for a GPU that reports no utilization.
#[test]
fn the_ai_view_wraps_instead_of_clipping() {
    use toptop::metrics::ServerStats;

    let mut app = App::new(&Config::default());
    // A GPU that reports nothing — Apple Silicon, most integrated GPUs.
    app.collector.gpus = vec![toptop::metrics::gpu::Gpu {
        name: "Apple M3".into(),
        util_pct: 0.0,
        has_util: false,
        mem_util: 0.0,
        has_mem_util: false,
        mem_used: 0,
        mem_total: 0,
        temp: 0.0,
        power: 0.0,
        power_limit: 0.0,
        throttled: false,
    }];
    // A server with every stat populated, so the line is at its longest.
    app.collector.servers = vec![ServerStats {
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
        preemptions: Some(12.0),
        preempt_rate: Some(1.2),
        ..Default::default()
    }];
    app.show_ai = true;
    let screen = render_text(&mut app, 118, 40);
    let joined = screen.join("\n");

    // Preemption is the most important fact on that line and used to be the
    // one clipped off the end.
    assert!(
        joined.contains("preempt 1.2/s"),
        "the preemption rate was clipped:\n{joined}"
    );
    // No "trend" heading without a trend to show.
    assert!(
        !joined.contains("compute ▲"),
        "a GPU reporting no utilization drew a trend over blank rows:\n{joined}"
    );
}

/// A narrow terminal must show fewer, readable columns rather than ten
/// unreadable stubs.
#[test]
fn the_process_table_drops_columns_when_narrow() {
    let mut app = App::new(&Config::default());

    let wide = render_text(&mut app, 120, 12);
    let wide_header = wide
        .iter()
        .find(|l| l.contains("PID"))
        .expect("header")
        .clone();
    assert!(wide_header.contains("USER"), "wide: {wide_header:?}");
    assert!(wide_header.contains("COMMAND"), "wide: {wide_header:?}");

    let narrow = render_text(&mut app, 44, 12);
    let narrow_header = narrow
        .iter()
        .find(|l| l.contains("PID"))
        .expect("header")
        .clone();
    // Fewer columns, but still the ones that identify and rank a process.
    assert!(!narrow_header.contains("USER"), "narrow: {narrow_header:?}");
    assert!(narrow_header.contains("CPU%"), "narrow: {narrow_header:?}");
    assert!(
        narrow_header.contains("COMMAND"),
        "narrow: {narrow_header:?}"
    );
    // And no truncated stubs: every retained heading is intact.
    for heading in ["PID", "CPU%", "COMMAND"] {
        assert!(
            narrow_header.contains(heading),
            "{heading} was clipped: {narrow_header:?}"
        );
    }
}
