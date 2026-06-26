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

    let mut app = App::new(&Config::default());
    // Take a couple of samples so meters and graphs have data.
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.on_tick();
    app.on_tick();

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
