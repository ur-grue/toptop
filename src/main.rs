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
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix, cyberpunk,
                         paper, or a user theme from ~/.config/toptop/themes/<name>.conf)
        --tree           Start in process-tree view
        --no-tree        Start in flat process view
        --ai             Open the AI / local-LLM GPU view on launch
        --demo           Simulate a busy GPU + vLLM server (see the AI view, no GPU needed)
        --remote <HOSTS> Multi-host fleet view; comma-separated SSH hosts
                         (use 'local' for this machine)
        --remote-cmd <C> Command run on each remote (default: toptop --export json)
        --config <PATH>  Use an explicit config file (default: ~/.config/toptop/config.conf)
        --no-save        Don't write the config back on exit
        --list-themes    Print available themes and exit
        --snapshot       Print a one-shot text snapshot and exit (no TUI)
        --export <FMT>   Print metrics and exit: 'json' (default), 'csv', or 'prometheus'
        --serve-metrics [ADDR]  Run a Prometheus endpoint (default 127.0.0.1:9709)
        --llm-server <H:P>   Scrape an inference server at host:port (repeatable;
                             bypasses localhost auto-discovery)
        --alert-vram <PCT>   VRAM % that triggers the spill-risk alert (default 90)
        --alert-kv <PCT>     KV-cache % considered saturated (default 95)
        --alert-queue <N>    Queued requests considered a backlog (default 8)
        --alert-cmd <CMD>    Shell command run on every alert fire/resolve
                             (TOPTOP_ALERT_{STATE,SEVERITY,KEY,DETAIL,MSG} in its env)
        --alert-preempt <R>  KV-cache preemptions/sec that trigger a critical alert
                             (default 0.2 — preemption throws away completed work)
    -h, --help           Show this help and exit
    -V, --version        Show version and exit

KEYS (in-app, press ? for the full list):
    ↑/↓  select   s sort   t tree   / filter   K kill   p theme   space pause   q quit
";

/// Immediate actions that print and exit instead of starting the monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Run,
    Help,
    Version,
    ListThemes,
}

/// Everything the command line can express, applied on top of a loaded Config.
#[derive(Debug)]
struct Opts {
    cfg: Config,
    config_path: Option<PathBuf>,
    no_save: bool,
    snapshot: bool,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
    export: Option<&'static str>,
    start_ai: bool,
    demo: bool,
    remote_hosts: Vec<String>,
    remote_cmd: String,
    serve_addr: Option<String>,
    action: Action,
}

/// Pre-scan for `--config <path>`. It must be known before the config file is
/// loaded, because every other flag overrides the loaded file.
fn config_path_arg(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args.iter().position(|a| a == "--config") {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .filter(|p| !p.starts_with('-'))
            .map(|p| Some(PathBuf::from(p)))
            .ok_or_else(|| "--config requires a file path".to_string()),
    }
}

/// Parse all flags onto `cfg` (already loaded from the right file).
///
/// Pure — no printing, no exiting. An `Err` is the message to show before
/// exiting with status 2. Help/version/list-themes return immediately with the
/// matching [`Action`], mirroring the historical first-flag-wins behavior.
fn parse_args(argv: &[String], cfg: Config) -> Result<Opts, String> {
    let mut opts = Opts {
        cfg,
        config_path: None,
        no_save: false,
        snapshot: false,
        record: None,
        replay: None,
        export: None,
        start_ai: false,
        demo: false,
        remote_hosts: Vec::new(),
        remote_cmd: "toptop --export json".to_string(),
        serve_addr: None,
        action: Action::Run,
    };

    let mut args = argv.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                opts.action = Action::Help;
                return Ok(opts);
            }
            "-V" | "--version" => {
                opts.action = Action::Version;
                return Ok(opts);
            }
            "--list-themes" => {
                opts.action = Action::ListThemes;
                return Ok(opts);
            }
            "-t" | "--tick" => {
                let v = args
                    .next()
                    .ok_or("--tick requires a millisecond value")?
                    .parse::<u64>()
                    .map_err(|_| "--tick value must be an integer")?;
                opts.cfg.tick_ms = v.clamp(100, 60_000);
            }
            "--theme" => {
                let name = args.next().ok_or("--theme requires a name")?;
                opts.cfg.theme_idx = theme::index_by_name(name)
                    .ok_or_else(|| format!("unknown theme '{name}' (try --list-themes)"))?;
            }
            "--tree" => opts.cfg.tree = true,
            "--no-tree" => opts.cfg.tree = false,
            "--ai" => opts.start_ai = true,
            "--demo" => opts.demo = true,
            "--remote" => {
                let list = args
                    .next()
                    .ok_or("--remote requires a comma-separated host list")?;
                opts.remote_hosts = list
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "--remote-cmd" => {
                opts.remote_cmd = args
                    .next()
                    .ok_or("--remote-cmd requires a command")?
                    .clone();
            }
            "--config" => {
                let p = args
                    .next()
                    .filter(|p| !p.starts_with('-'))
                    .ok_or("--config requires a file path")?;
                opts.config_path = Some(PathBuf::from(p));
            }
            "--no-save" => opts.no_save = true,
            "--serve-metrics" => {
                // Optional address argument; defaults to localhost:9709.
                let addr = match args.peek() {
                    Some(a) if !a.starts_with('-') => args.next().unwrap().clone(),
                    _ => "127.0.0.1:9709".to_string(),
                };
                opts.serve_addr = Some(addr);
            }
            "--llm-server" => {
                let spec = args.next().ok_or("--llm-server requires host:port")?;
                opts.cfg
                    .llm_servers
                    .push(toptop::metrics::infer::parse_target(spec)?);
            }
            "--alert-vram" => {
                let v = args
                    .next()
                    .ok_or("--alert-vram requires a percentage")?
                    .parse::<f32>()
                    .map_err(|_| "--alert-vram value must be a number")?;
                opts.cfg.alerts.vram_spill_pct = v.clamp(1.0, 100.0);
            }
            "--alert-kv" => {
                let v = args
                    .next()
                    .ok_or("--alert-kv requires a percentage")?
                    .parse::<f64>()
                    .map_err(|_| "--alert-kv value must be a number")?;
                opts.cfg.alerts.kv_high_pct = v.clamp(1.0, 100.0);
            }
            "--alert-cmd" => {
                opts.cfg.notify.cmd = Some(
                    args.next()
                        .ok_or("--alert-cmd requires a shell command")?
                        .clone(),
                );
            }
            "--alert-preempt" => {
                let v = args
                    .next()
                    .ok_or("--alert-preempt requires a rate (preemptions/second)")?
                    .parse::<f64>()
                    .map_err(|_| "--alert-preempt value must be a number")?;
                opts.cfg.alerts.preempt_rate_high = v.max(0.0);
            }
            "--alert-queue" => {
                let v = args
                    .next()
                    .ok_or("--alert-queue requires a count")?
                    .parse::<f64>()
                    .map_err(|_| "--alert-queue value must be a number")?;
                opts.cfg.alerts.queue_high = v.max(1.0);
            }
            "--record" => {
                let p = args.next().ok_or("--record requires a file path")?;
                opts.record = Some(PathBuf::from(p));
            }
            "--replay" => {
                let p = args.next().ok_or("--replay requires a file path")?;
                opts.replay = Some(PathBuf::from(p));
            }
            "--snapshot" => opts.snapshot = true,
            "--export" => {
                // Optional format argument: `json` (default) or `prometheus`.
                opts.export = Some(match args.peek().map(|s| s.as_str()) {
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
            other => return Err(format!("unknown argument '{other}' (try --help)")),
        }
    }
    if opts.record.is_some() && opts.replay.is_some() {
        return Err("--record and --replay are mutually exclusive".to_string());
    }
    Ok(opts)
}

/// Print an argument error and exit with the documented status 2.
fn arg_error(msg: &str) -> ! {
    eprintln!("toptop: {msg}");
    std::process::exit(2);
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // User themes must be registered before the config file is read — it
    // resolves `theme = <name>` against the registry.
    for warning in theme::init_user_themes(theme::user_theme_dir().as_deref()) {
        eprintln!("toptop: {warning}");
    }

    let config_path = config_path_arg(&argv).unwrap_or_else(|e| arg_error(&e));
    let cfg = match &config_path {
        Some(path) => Config::load_path(path),
        None => Config::load(),
    };
    for warning in &cfg.warnings {
        eprintln!("toptop: {warning}");
    }
    let opts = parse_args(&argv, cfg).unwrap_or_else(|e| arg_error(&e));

    match opts.action {
        Action::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("toptop {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::ListThemes => {
            let builtin = theme::BUILTINS.len();
            for (i, t) in theme::themes().iter().enumerate() {
                if i < builtin {
                    println!("{}", t.name);
                } else {
                    println!("{}  (user)", t.name);
                }
            }
            return Ok(());
        }
        Action::Run => {}
    }

    let Opts {
        cfg,
        config_path,
        no_save,
        snapshot,
        record,
        replay,
        export,
        start_ai,
        demo,
        remote_hosts,
        remote_cmd,
        serve_addr,
        ..
    } = opts;

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

    if let Some(path) = &replay {
        let r = toptop::record::Replay::load(path)
            .with_context(|| format!("cannot replay {}", path.display()))?;
        if r.schema_version() != toptop::record::SCHEMA_VERSION {
            eprintln!(
                "toptop: {} was written by schema v{}, this build reads v{} — \
                 fields it doesn't know are ignored",
                path.display(),
                r.schema_version(),
                toptop::record::SCHEMA_VERSION
            );
        }
        if r.skipped > 0 {
            eprintln!(
                "toptop: {}: skipped {} unreadable frame(s)",
                path.display(),
                r.skipped
            );
        }
        app.replay = Some(r);
    }
    if let Some(path) = &record {
        app.recorder = Some(
            toptop::record::Recorder::create(path)
                .with_context(|| format!("cannot record to {}", path.display()))?,
        );
    }
    app.show_ai = start_ai || demo;
    app.demo = demo;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Opts, String> {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_args(&argv, Config::default())
    }

    #[test]
    fn no_args_yields_defaults() {
        let o = parse(&[]).unwrap();
        assert_eq!(o.action, Action::Run);
        assert!(!o.snapshot && !o.start_ai && !o.no_save);
        assert_eq!(o.export, None);
        assert_eq!(o.serve_addr, None);
        assert_eq!(o.config_path, None);
        assert!(o.remote_hosts.is_empty());
        assert_eq!(o.remote_cmd, "toptop --export json");
        assert_eq!(o.cfg.tick_ms, Config::default().tick_ms);
    }

    #[test]
    fn tick_parses_and_clamps() {
        assert_eq!(parse(&["--tick", "500"]).unwrap().cfg.tick_ms, 500);
        assert_eq!(parse(&["-t", "50"]).unwrap().cfg.tick_ms, 100);
        assert_eq!(parse(&["--tick", "999999"]).unwrap().cfg.tick_ms, 60_000);
    }

    #[test]
    fn tick_errors() {
        assert!(parse(&["--tick"]).unwrap_err().contains("requires"));
        assert!(parse(&["--tick", "abc"]).unwrap_err().contains("integer"));
    }

    #[test]
    fn alert_cmd_flag() {
        let o = parse(&["--alert-cmd", "notify-send $TOPTOP_ALERT_MSG"]).unwrap();
        assert_eq!(
            o.cfg.notify.cmd.as_deref(),
            Some("notify-send $TOPTOP_ALERT_MSG")
        );
        assert!(parse(&["--alert-cmd"]).unwrap_err().contains("requires"));
    }

    #[test]
    fn llm_server_targets_are_repeatable_and_validated() {
        let o = parse(&["--llm-server", "gpu-box:8000", "--llm-server", "[::1]:9000"]).unwrap();
        assert_eq!(o.cfg.llm_servers.len(), 2);
        assert_eq!(o.cfg.llm_servers[0].host, "gpu-box");
        assert_eq!(o.cfg.llm_servers[1].host, "::1");
        assert!(parse(&["--llm-server", "nope"])
            .unwrap_err()
            .contains("host:port"));
        assert!(parse(&["--llm-server"]).unwrap_err().contains("requires"));
    }

    #[test]
    fn record_and_replay_flags() {
        let o = parse(&["--record", "run.jsonl"]).unwrap();
        assert_eq!(o.record.as_deref(), Some(std::path::Path::new("run.jsonl")));
        let o = parse(&["--replay", "run.jsonl"]).unwrap();
        assert_eq!(o.replay.as_deref(), Some(std::path::Path::new("run.jsonl")));
        assert!(parse(&["--record"]).unwrap_err().contains("requires"));
        assert!(parse(&["--replay"]).unwrap_err().contains("requires"));
        // Recording a replay would be a confusing no-op.
        let e = parse(&["--record", "a", "--replay", "b"]).unwrap_err();
        assert!(e.contains("mutually exclusive"), "{e}");
    }

    #[test]
    fn theme_by_name_and_unknown() {
        let o = parse(&["--theme", "nord"]).unwrap();
        assert_eq!(Some(o.cfg.theme_idx), theme::index_by_name("nord"));
        let e = parse(&["--theme", "plaid"]).unwrap_err();
        assert!(e.contains("unknown theme 'plaid'"));
        assert!(parse(&["--theme"]).unwrap_err().contains("requires"));
    }

    #[test]
    fn tree_flags_last_one_wins() {
        assert!(parse(&["--tree"]).unwrap().cfg.tree);
        assert!(!parse(&["--tree", "--no-tree"]).unwrap().cfg.tree);
        assert!(parse(&["--ai"]).unwrap().start_ai);
    }

    #[test]
    fn remote_hosts_split_and_trimmed() {
        let o = parse(&["--remote", "a, b,,c", "--remote-cmd", "tt --export json"]).unwrap();
        assert_eq!(o.remote_hosts, vec!["a", "b", "c"]);
        assert_eq!(o.remote_cmd, "tt --export json");
        assert!(parse(&["--remote"]).unwrap_err().contains("requires"));
        assert!(parse(&["--remote-cmd"]).unwrap_err().contains("requires"));
    }

    #[test]
    fn serve_metrics_address_is_optional() {
        assert_eq!(
            parse(&["--serve-metrics"]).unwrap().serve_addr.as_deref(),
            Some("127.0.0.1:9709")
        );
        assert_eq!(
            parse(&["--serve-metrics", "0.0.0.0:9709"])
                .unwrap()
                .serve_addr
                .as_deref(),
            Some("0.0.0.0:9709")
        );
        // A following flag is not swallowed as the address.
        let o = parse(&["--serve-metrics", "--tree"]).unwrap();
        assert_eq!(o.serve_addr.as_deref(), Some("127.0.0.1:9709"));
        assert!(o.cfg.tree);
    }

    #[test]
    fn export_formats() {
        assert_eq!(parse(&["--export"]).unwrap().export, Some("json"));
        assert_eq!(parse(&["--export", "csv"]).unwrap().export, Some("csv"));
        assert_eq!(
            parse(&["--export", "prom"]).unwrap().export,
            Some("prometheus")
        );
        assert_eq!(
            parse(&["--export", "prometheus"]).unwrap().export,
            Some("prometheus")
        );
        // A following flag means default format, and the flag still applies.
        let o = parse(&["--export", "--tree"]).unwrap();
        assert_eq!(o.export, Some("json"));
        assert!(o.cfg.tree);
    }

    #[test]
    fn alert_flags_clamp_and_error() {
        let o = parse(&[
            "--alert-vram",
            "85",
            "--alert-kv",
            "90.5",
            "--alert-queue",
            "4",
        ])
        .unwrap();
        assert_eq!(o.cfg.alerts.vram_spill_pct, 85.0);
        assert_eq!(o.cfg.alerts.kv_high_pct, 90.5);
        assert_eq!(o.cfg.alerts.queue_high, 4.0);
        assert_eq!(
            parse(&["--alert-vram", "250"])
                .unwrap()
                .cfg
                .alerts
                .vram_spill_pct,
            100.0
        );
        assert_eq!(
            parse(&["--alert-queue", "0"])
                .unwrap()
                .cfg
                .alerts
                .queue_high,
            1.0
        );
        assert!(parse(&["--alert-kv", "lots"])
            .unwrap_err()
            .contains("number"));
        assert!(parse(&["--alert-vram"]).unwrap_err().contains("requires"));
    }

    #[test]
    fn config_flag_and_prescan_agree() {
        let argv: Vec<String> = ["--config", "/tmp/x.conf", "--no-save"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            config_path_arg(&argv).unwrap(),
            Some(PathBuf::from("/tmp/x.conf"))
        );
        let o = parse_args(&argv, Config::default()).unwrap();
        assert_eq!(o.config_path, Some(PathBuf::from("/tmp/x.conf")));
        assert!(o.no_save);

        for bad in [&["--config"][..], &["--config", "--tree"][..]] {
            let argv: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            assert!(config_path_arg(&argv).is_err());
            assert!(parse_args(&argv, Config::default()).is_err());
        }
        assert_eq!(config_path_arg(&[]).unwrap(), None);
    }

    #[test]
    fn action_flags_exit_early_ignoring_later_args() {
        // Historical behavior: --help wins immediately, even before bad flags.
        assert_eq!(parse(&["--help", "--bogus"]).unwrap().action, Action::Help);
        assert_eq!(parse(&["-h"]).unwrap().action, Action::Help);
        assert_eq!(parse(&["-V"]).unwrap().action, Action::Version);
        assert_eq!(parse(&["--version"]).unwrap().action, Action::Version);
        assert_eq!(
            parse(&["--list-themes"]).unwrap().action,
            Action::ListThemes
        );
    }

    #[test]
    fn unknown_flag_is_an_error() {
        let e = parse(&["--bogus"]).unwrap_err();
        assert!(e.contains("unknown argument '--bogus'"));
        assert!(e.contains("--help"));
    }
}
