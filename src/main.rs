//! `toptop` — a gorgeous, feature-rich terminal system monitor.
//!
//! This binary wires together terminal lifecycle management, the event loop,
//! and the [`toptop`] library. A panic hook guarantees the terminal is restored
//! even on an unexpected crash.

use std::io::{self, Stdout, Write};
use std::path::PathBuf;
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
use toptop::fleet::FleetApp;
use toptop::theme;
use toptop::ui;

type Tui = Terminal<CrosstermBackend<Stdout>>;

const HELP: &str = "\
toptop — a gorgeous terminal system monitor

USAGE:
    toptop [OPTIONS]

OPTIONS:
    -t, --tick <MS>      Refresh interval in milliseconds (100-60000)
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix, cyberpunk, paper)
        --tree           Start in process-tree view
        --no-tree        Start in flat process view
        --ai             Open the AI / local-LLM GPU view on launch
        --remote <HOSTS> Multi-host fleet view; comma-separated SSH hosts
                         (use 'local' for this machine)
        --remote-cmd <C> Command run on each remote (default: toptop --export json)
        --config <PATH>  Use an explicit config file (default: ~/.config/toptop/config.conf)
        --no-save        Don't write the config back on exit
        --list-themes    Print available themes and exit
        --snapshot       Print a one-shot text snapshot and exit (no TUI)
        --export <FMT>   Print metrics and exit: 'json' (default), 'csv', or 'prometheus'
        --serve-metrics [ADDR]  Run a Prometheus endpoint (default 127.0.0.1:9709)
        --alert-vram <PCT>   VRAM % that triggers the spill-risk alert (default 90)
        --alert-kv <PCT>     KV-cache % considered saturated (default 95)
        --alert-queue <N>    Queued requests considered a backlog (default 8)
    -h, --help           Show this help and exit
    -V, --version        Show version and exit

KEYS (in-app, press ? for the full list):
    ↑/↓  select   s sort   t tree   / filter   K kill   p theme   space pause   q quit
";

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // `--config` must be known before the config file is loaded, so resolve
    // it up front; the parse loop below skips over it.
    let config_path: Option<PathBuf> = argv
        .iter()
        .position(|a| a == "--config")
        .map(|i| {
            argv.get(i + 1)
                .filter(|p| !p.starts_with('-'))
                .map(PathBuf::from)
                .context("--config requires a file path")
        })
        .transpose()?;

    let mut cfg = match &config_path {
        Some(path) => Config::load_path(path),
        None => Config::load(),
    };
    let mut no_save = false;
    let mut snapshot = false;
    let mut export: Option<&'static str> = None;
    let mut start_ai = false;
    let mut remote_hosts: Vec<String> = Vec::new();
    let mut remote_cmd = "toptop --export json".to_string();
    let mut serve_addr: Option<String> = None;

    let mut args = argv.iter().peekable();
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
                cfg.theme_idx = theme::index_by_name(name)
                    .with_context(|| format!("unknown theme '{name}' (try --list-themes)"))?;
            }
            "--tree" => cfg.tree = true,
            "--no-tree" => cfg.tree = false,
            "--ai" => start_ai = true,
            "--remote" => {
                let list = args
                    .next()
                    .context("--remote requires a comma-separated host list")?;
                remote_hosts = list
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "--remote-cmd" => {
                remote_cmd = args
                    .next()
                    .context("--remote-cmd requires a command")?
                    .clone();
            }
            "--config" => {
                // Already resolved in the pre-scan above; skip the value.
                args.next();
            }
            "--no-save" => no_save = true,
            "--serve-metrics" => {
                // Optional address argument; defaults to localhost:9709.
                let addr = match args.peek() {
                    Some(a) if !a.starts_with('-') => args.next().unwrap().clone(),
                    _ => "127.0.0.1:9709".to_string(),
                };
                serve_addr = Some(addr);
            }
            "--alert-vram" => {
                let v = args
                    .next()
                    .context("--alert-vram requires a percentage")?
                    .parse::<f32>()
                    .context("--alert-vram value must be a number")?;
                cfg.alerts.vram_spill_pct = v.clamp(1.0, 100.0);
            }
            "--alert-kv" => {
                let v = args
                    .next()
                    .context("--alert-kv requires a percentage")?
                    .parse::<f64>()
                    .context("--alert-kv value must be a number")?;
                cfg.alerts.kv_high_pct = v.clamp(1.0, 100.0);
            }
            "--alert-queue" => {
                let v = args
                    .next()
                    .context("--alert-queue requires a count")?
                    .parse::<f64>()
                    .context("--alert-queue value must be a number")?;
                cfg.alerts.queue_high = v.max(1.0);
            }
            "--snapshot" => snapshot = true,
            "--export" => {
                // Optional format argument: `json` (default) or `prometheus`.
                export = Some(match args.peek().map(|s| s.as_str()) {
                    Some("prometheus") | Some("prom") => {
                        args.next();
                        "prometheus"
                    }
                    Some("csv") => {
                        args.next();
                        "csv"
                    }
                    Some("json") => {
                        args.next();
                        "json"
                    }
                    _ => "json",
                });
            }
            other => {
                eprintln!("toptop: unknown argument '{other}' (try --help)");
                std::process::exit(2);
            }
        }
    }

    if let Some(format) = export {
        return run_export(&cfg, format);
    }

    if let Some(addr) = serve_addr {
        toptop::serve::run(&addr, &cfg).context("metrics server failed")?;
        return Ok(());
    }

    if snapshot {
        return run_snapshot(&cfg);
    }

    if !remote_hosts.is_empty() {
        let mut fleet = FleetApp::new(remote_hosts, remote_cmd, cfg.theme_idx);
        let mut terminal = setup_terminal().context("failed to initialize terminal")?;
        let result = run_fleet(&mut terminal, &mut fleet);
        restore_terminal(&mut terminal).ok();
        return result;
    }

    let mut app = App::new(&cfg);
    app.show_ai = start_ai;
    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal).ok();
    if !no_save {
        // The terminal is restored, so a warning lands on a usable stderr.
        let saved = match &config_path {
            Some(path) => app.config().save_path(path),
            None => app.config().save(),
        };
        if let Err(e) = saved {
            eprintln!("toptop: warning: failed to save config: {e}");
        }
    }
    result
}

/// Machine-readable JSON snapshot — the building block for multi-host
/// monitoring and external dashboards.
fn run_export(cfg: &Config, format: &str) -> Result<()> {
    let mut app = App::new(cfg);
    std::thread::sleep(Duration::from_millis(350));
    app.on_tick();
    match format {
        "prometheus" => print!(
            "{}",
            toptop::export::to_prometheus(&app.collector, &cfg.alerts)
        ),
        "csv" => print!("{}", toptop::export::to_csv(&app.collector, 20)),
        _ => println!("{}", toptop::export::to_json(&app.collector, 20)),
    }
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

fn run_fleet(terminal: &mut Tui, app: &mut FleetApp) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui::fleet::draw(f, app))?;
        let timeout = app
            .tick
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key.code);
                }
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
