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
    terminal.draw(|f| ui::draw(f, app)).expect("draw must not panic");
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
    app.on_key(key(KeyCode::Char('n')));
    assert!(app.pending_kill.is_none());
}

#[test]
fn header_sort_mapping() {
    use toptop::app::{header_sort_at, SortField};
    assert_eq!(header_sort_at(0), Some(SortField::Pid));
    assert_eq!(header_sort_at(10), Some(SortField::User));
    assert_eq!(header_sort_at(20), Some(SortField::Cpu));
    assert_eq!(header_sort_at(26), Some(SortField::Mem));
    assert_eq!(header_sort_at(40), Some(SortField::Time));
    assert_eq!(header_sort_at(46), None);
    assert_eq!(header_sort_at(60), Some(SortField::Name));
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
fn quit_keys_work() {
    let mut app = App::new(&Config::default());
    app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);

    let mut app = App::new(&Config::default());
    app.on_key(key(KeyCode::Char('q')));
    assert!(app.should_quit);
}
