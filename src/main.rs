//! `toptop` — a gorgeous, feature-rich terminal system monitor.
//!
//! This binary wires together terminal lifecycle management, the event loop,
//! and the [`toptop`] library. A panic hook guarantees the terminal is restored
//! even on an unexpected crash.

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;

use toptop::app::App;
use toptop::config::Config;
use toptop::theme;
use toptop::ui;

type Tui = Terminal<CrosstermBackend<Stdout>>;

const HELP: &str = "\
toptop — a gorgeous terminal system monitor

USAGE:
    toptop [OPTIONS]

OPTIONS:
    -t, --tick <MS>      Refresh interval in milliseconds (100-60000)
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix)
        --tree           Start in process-tree view
        --no-tree        Start in flat process view
        --list-themes    Print available themes and exit
        --snapshot       Print a one-shot text snapshot and exit (no TUI)
        --export json    Print a machine-readable JSON snapshot and exit
    -h, --help           Show this help and exit
    -V, --version        Show version and exit

KEYS (in-app, press ? for the full list):
    ↑/↓  select   s sort   t tree   / filter   K kill   p theme   space pause   q quit
";

fn main() -> Result<()> {
    let mut cfg = Config::load();
    let mut snapshot = false;
    let mut export = false;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("toptop {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--list-themes" => {
                for t in theme::THEMES {
                    println!("{}", t.name);
                }
                return Ok(());
            }
            "-t" | "--tick" => {
                let v = args
                    .next()
                    .context("--tick requires a millisecond value")?
                    .parse::<u64>()
                    .context("--tick value must be an integer")?;
                cfg.tick_ms = v.clamp(100, 60_000);
            }
            "--theme" => {
                let name = args.next().context("--theme requires a name")?;
                cfg.theme_idx = theme::index_by_name(&name)
                    .with_context(|| format!("unknown theme '{name}' (try --list-themes)"))?;
            }
            "--tree" => cfg.tree = true,
            "--no-tree" => cfg.tree = false,
            "--snapshot" => snapshot = true,
            "--export" => {
                // An optional format argument may follow; only `json` is supported.
                if matches!(args.peek().map(|s| s.as_str()), Some("json")) {
                    args.next();
                }
                export = true;
            }
            other => {
                eprintln!("toptop: unknown argument '{other}' (try --help)");
                std::process::exit(2);
            }
        }
    }

    if export {
        return run_export(&cfg);
    }

    if snapshot {
        return run_snapshot(&cfg);
    }

    let mut app = App::new(&cfg);
    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal).ok();
    app.config().save();
    result
}

/// Machine-readable JSON snapshot — the building block for multi-host
/// monitoring and external dashboards.
fn run_export(cfg: &Config) -> Result<()> {
    let mut app = App::new(cfg);
    std::thread::sleep(Duration::from_millis(350));
    app.on_tick();
    println!("{}", toptop::export::to_json(&app.collector, 20));
    io::stdout().flush().ok();
    Ok(())
}

/// One-shot, non-interactive snapshot — handy for piping, scripts, or smoke
/// tests in environments without a TTY.
fn run_snapshot(cfg: &Config) -> Result<()> {
    let mut app = App::new(cfg);
    // A second sample spaced in time yields meaningful CPU% and I/O rates.
    std::thread::sleep(Duration::from_millis(350));
    app.on_tick();

    let c = &app.collector;
    println!("toptop {} — snapshot", env!("CARGO_PKG_VERSION"));
    println!(
        "host {}  ·  {}  ·  kernel {}  ·  {}",
        c.host.hostname, c.host.os, c.host.kernel, c.host.arch
    );
    println!(
        "uptime {}  ·  load {:.2} {:.2} {:.2}  ·  {} tasks, {} running",
        toptop::util::human_duration(c.uptime),
        c.cpu.load_avg.0,
        c.cpu.load_avg.1,
        c.cpu.load_avg.2,
        c.procs.len(),
        c.running_procs()
    );
    println!(
        "cpu  {:>5.1}%  ({} cores @ {} MHz)  {}",
        c.cpu.global_usage,
        c.cpu.per_core.len(),
        c.cpu.freq_mhz,
        c.host.cpu_brand
    );
    println!(
        "mem  {} / {}  ({:.1}%)  ·  swap {} / {}",
        toptop::util::human_bytes(c.mem.used),
        toptop::util::human_bytes(c.mem.total),
        if c.mem.total > 0 {
            c.mem.used as f64 / c.mem.total as f64 * 100.0
        } else {
            0.0
        },
        toptop::util::human_bytes(c.mem.swap_used),
        toptop::util::human_bytes(c.mem.swap_total),
    );
    if let Some(net) = c.nets.first() {
        println!(
            "net  {}  ↓ {}  ↑ {}",
            net.name,
            toptop::util::human_rate(net.down_rate),
            toptop::util::human_rate(net.up_rate)
        );
    }
    println!(
        "disk io  R {}  W {}",
        toptop::util::human_rate(c.disk_read_rate),
        toptop::util::human_rate(c.disk_write_rate)
    );
    for d in c.disk_list.iter().take(6) {
        println!(
            "     {:<14} {:>5.1}%  {} free of {}",
            d.mount,
            d.used_pct,
            toptop::util::human_bytes(d.available),
            toptop::util::human_bytes(d.total)
        );
    }
    println!("\ntop processes by CPU:");
    app.sort = toptop::app::SortField::Cpu;
    app.sort_desc = true;
    app.rebuild_proc_view();
    println!(
        "  {:>7}  {:<10} {:>6} {:>6}  COMMAND",
        "PID", "USER", "CPU%", "MEM%"
    );
    for p in app.proc_view.iter().take(12) {
        println!(
            "  {:>7}  {:<10} {:>6.1} {:>6.1}  {}",
            p.pid,
            toptop::util::truncate(&p.user, 10),
            p.cpu,
            p.mem_pct,
            toptop::util::truncate(&p.name, 40)
        );
    }
    io::stdout().flush().ok();
    Ok(())
}

fn run(terminal: &mut Tui, app: &mut App) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        let timeout = app
            .tick
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(m) => app.on_mouse(m),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= app.tick {
            app.on_tick();
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    // Restore the terminal even if a panic unwinds (or aborts) past us.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = io::stdout();
        let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture);
        let _ = disable_raw_mode();
        default_hook(info);
    }));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
