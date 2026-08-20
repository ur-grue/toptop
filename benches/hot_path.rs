//! Performance guard for the refresh/render hot path.
//!
//! Not a microbenchmark suite — a **regression guard**. Shared CI runners are
//! far too noisy for percent-level thresholds, so this measures the two things
//! that actually decide whether toptop feels smooth (a metrics refresh and a
//! full render frame) and fails only when one blows a deliberately generous
//! budget. That catches the regressions that matter — an accidental O(n²) over
//! the process list, a per-row syscall, an unthrottled file read — while
//! staying green on a loaded runner.
//!
//! The measured numbers are always printed, so the CI log carries a trend even
//! when nothing fails.
//!
//! Run with `cargo bench`.

use std::time::{Duration, Instant};

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use toptop::app::App;
use toptop::config::Config;
use toptop::ui;

/// One measured step: what it is, how long it took, and what it may take.
struct Measurement {
    name: &'static str,
    per_op: Duration,
    budget: Duration,
    note: &'static str,
}

impl Measurement {
    fn blown(&self) -> bool {
        self.per_op > self.budget
    }
}

/// Time `iters` calls to `f` and return the per-call duration.
fn time(iters: u32, mut f: impl FnMut()) -> Duration {
    // One untimed pass so lazily-populated caches aren't billed to the first
    // measured iteration.
    f();
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed() / iters
}

fn main() {
    let mut app = App::new(&Config::default());
    let procs = app.collector.procs.len();

    let refresh = time(10, || {
        app.collector.refresh();
    });

    let rebuild = time(50, || {
        app.rebuild_proc_view();
    });

    // A large-but-plausible terminal: everything on screen, nothing clipped.
    let mut terminal = Terminal::new(TestBackend::new(200, 60)).expect("terminal");
    let render = time(50, || {
        terminal
            .draw(|f| ui::draw(f, &mut app))
            .expect("draw must not panic");
    });

    // The AI view is the densest screen: braille sparklines per server plus
    // the GPU panels.
    app.show_ai = true;
    let render_ai = time(50, || {
        terminal
            .draw(|f| ui::draw(f, &mut app))
            .expect("draw must not panic");
    });
    app.show_ai = false;

    // Tree mode re-sorts and re-parents every process each rebuild — the most
    // algorithmically interesting path over the process list.
    app.tree = true;
    let rebuild_tree = time(50, || {
        app.rebuild_proc_view();
    });

    let results = [
        Measurement {
            name: "collector.refresh",
            per_op: refresh,
            // Dominated by the kernel and sysinfo; the guard is against adding
            // a per-process syscall or an unthrottled file read.
            budget: Duration::from_millis(600),
            note: "metrics sample",
        },
        Measurement {
            name: "rebuild_proc_view",
            per_op: rebuild,
            budget: Duration::from_millis(25),
            note: "sort + filter",
        },
        Measurement {
            name: "rebuild_proc_view (tree)",
            per_op: rebuild_tree,
            budget: Duration::from_millis(50),
            note: "sort + re-parent",
        },
        Measurement {
            name: "render 200x60",
            per_op: render,
            budget: Duration::from_millis(20),
            note: "full frame",
        },
        Measurement {
            name: "render 200x60 (ai view)",
            per_op: render_ai,
            budget: Duration::from_millis(25),
            note: "densest frame",
        },
    ];

    println!("\ntoptop hot-path guard — {procs} processes on this machine\n");
    println!(
        "{:<26} {:>10} {:>10}   what it covers",
        "step", "per op", "budget"
    );
    println!("{}", "-".repeat(72));
    for m in &results {
        println!(
            "{:<26} {:>9.2}ms {:>9.0}ms   {}{}",
            m.name,
            m.per_op.as_secs_f64() * 1000.0,
            m.budget.as_secs_f64() * 1000.0,
            m.note,
            if m.blown() { "   ← OVER BUDGET" } else { "" }
        );
    }

    let blown: Vec<&Measurement> = results.iter().filter(|m| m.blown()).collect();
    if blown.is_empty() {
        println!("\nall within budget");
        return;
    }
    eprintln!(
        "\n{} step(s) over budget. These budgets are ~10x typical, so this is \
         an algorithmic regression, not runner noise.",
        blown.len()
    );
    std::process::exit(1);
}
