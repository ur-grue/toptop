//! All terminal rendering. Pure functions of `(&mut Frame, &mut App)`.

pub mod fleet;
pub mod graph;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;

use std::time::Instant;

use crate::alerts::{Level, TransitionState};
use crate::app::{App, ProcColumn};
use crate::metrics::ProcInfo;
use crate::theme::Theme;
use crate::util::{
    clamp_pct, compact_duration, human_bytes, human_duration, human_rate, short_bytes, truncate,
};

/// Top-level draw entry point.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = app.theme();
    if let Some(bg) = theme.bg {
        f.render_widget(
            Block::default().style(Style::default().bg(bg.color())),
            area,
        );
    }

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(f, chunks[0], app);
    render_body(f, chunks[1], app);
    render_footer(f, chunks[2], app);

    if app.show_conn {
        render_connections(f, area, app);
    }
    if app.show_ai {
        render_ai(f, area, app);
    }
    if app.show_detail {
        render_detail(f, area, app);
    }
    if app.show_alert_history {
        render_alert_history(f, area, app);
    }
    if let Some(idx) = app.signal_menu {
        render_signal_menu(f, area, theme, idx, app);
    }
    if let Some(idx) = app.renice_menu {
        render_renice_menu(f, area, theme, idx, app);
    }
    if app.show_help {
        render_help(f, area, theme);
    }
    if let Some(pk) = app.pending_kill.clone() {
        render_confirm(f, area, theme, &pk);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn panel(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border.color()))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        ))
}

fn render_lines(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn dim(theme: &Theme) -> Style {
    Style::default().fg(theme.dim.color())
}

// ── Header ───────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let c = &app.collector;
    let tasks = c.procs.len();
    let running = c.running_procs();
    let mut spans = vec![
        Span::styled(
            "▟▛ toptop",
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", dim(theme)),
        Span::styled(&c.host.hostname, Style::default().fg(theme.accent2.color())),
        Span::styled(format!("  {}", truncate(&c.host.os, 28)), dim(theme)),
        Span::styled(format!("  kernel {}", c.host.kernel), dim(theme)),
        Span::styled(
            format!("  up {}", human_duration(c.uptime)),
            Style::default().fg(theme.fg.color()),
        ),
        Span::styled(
            format!(
                "  load {:.2} {:.2} {:.2}",
                c.cpu.load_avg.0, c.cpu.load_avg.1, c.cpu.load_avg.2
            ),
            Style::default().fg(theme.fg.color()),
        ),
        Span::styled(format!("  {} tasks, {} run", tasks, running), dim(theme)),
    ];
    // Recording is a persistent, easy-to-forget state — mark it unmissably.
    if app.recorder.is_some() {
        spans.push(Span::styled(
            "  ● REC",
            Style::default()
                .fg(theme.grad(1.0))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(bat) = &c.battery {
        let t = (bat.percent / 100.0).clamp(0.0, 1.0);
        let glyph = if bat.status.eq_ignore_ascii_case("charging") {
            "⚡"
        } else {
            "🔋"
        };
        spans.push(Span::styled(
            format!("  {}{:.0}%", glyph, bat.percent),
            Style::default().fg(theme.grad(1.0 - t)),
        ));
    }
    if !app.alerts.is_empty() {
        let crit = matches!(
            crate::alerts::worst_level(&app.alerts),
            Some(crate::alerts::Level::Crit)
        );
        spans.push(Span::styled(
            format!(
                "  ⚠ {} alert{}",
                app.alerts.len(),
                if app.alerts.len() == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(theme.grad(if crit { 1.0 } else { 0.7 }))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if app.paused {
        spans.push(Span::styled(
            "  ⏸ PAUSED",
            Style::default()
                .fg(theme.grad(1.0))
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Live clock, right-aligned — but only when it won't collide with the
    // left-hand status text (otherwise the two overlap on narrow terminals).
    let clock = chrono::Local::now().format("%H:%M:%S").to_string();

    // Drop whole segments that don't fit rather than letting ratatui cut the
    // last one mid-word ("563 tasks, 301 r"). Each span is a self-contained
    // fact, so losing one entirely reads as a narrow terminal; half of one
    // reads as a bug.
    let mut budget = area.width as usize;
    let mut fitted = Vec::with_capacity(spans.len());
    for span in spans {
        let len = span.content.chars().count();
        if len > budget {
            break;
        }
        budget -= len;
        fitted.push(span);
    }
    let spans = fitted;

    let left_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let left = Paragraph::new(Line::from(spans));
    f.render_widget(left, area);
    if (area.width as usize) >= left_len + clock.len() + 2 {
        let clock_span = Span::styled(
            clock,
            Style::default()
                .fg(theme.accent2.color())
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            Paragraph::new(Line::from(clock_span)).alignment(Alignment::Right),
            area,
        );
    }
}

// ── Body layout ──────────────────────────────────────────────────────────────

fn render_body(f: &mut Frame, area: Rect, app: &mut App) {
    use crate::app::LayoutPreset;

    // The Process preset hides the whole top section.
    if app.layout == LayoutPreset::Process {
        render_procs(f, area, app);
        return;
    }

    let top_h: u16 = if area.height >= 22 {
        14
    } else if area.height >= 15 {
        9
    } else {
        0
    };

    if top_h > 0 && area.width >= 60 {
        let rows = Layout::vertical([Constraint::Length(top_h), Constraint::Min(3)]).split(area);
        match app.layout {
            LayoutPreset::Cpu => render_cpu(f, rows[0], app),
            _ => render_top(f, rows[0], app),
        }
        render_procs(f, rows[1], app);
    } else {
        render_procs(f, area, app);
    }
}

fn render_top(f: &mut Frame, area: Rect, app: &mut App) {
    let cols = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
    ])
    .split(area);

    render_cpu(f, cols[0], app);

    let mid =
        Layout::vertical([Constraint::Percentage(58), Constraint::Percentage(42)]).split(cols[1]);
    render_mem(f, mid[0], app);
    render_sensors(f, mid[1], app);

    let right =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(cols[2]);
    render_net(f, right[0], app);
    render_disk(f, right[1], app);
}

// ── CPU panel ────────────────────────────────────────────────────────────────

fn render_cpu(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = panel("cpu", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let cpu = &app.collector.cpu;
    let usage = clamp_pct(cpu.global_usage);
    let freq = if cpu.freq_mhz > 0 {
        format!("{:.2} GHz", cpu.freq_mhz as f64 / 1000.0)
    } else {
        "—".to_string()
    };
    let summary = Line::from(vec![
        Span::styled("all ", dim(theme)),
        Span::styled(
            format!("{:>5.1}%", usage),
            Style::default()
                .fg(theme.grad(usage / 100.0))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", freq), Style::default().fg(theme.fg.color())),
        Span::styled(
            format!(
                "  {}c/{}t",
                cpu.per_core.len().min(app.collector.host.physical_cores),
                cpu.per_core.len()
            ),
            dim(theme),
        ),
    ]);
    // In a limited cgroup the host core count is not what this process may
    // use, and "3% CPU" on a throttled container is actively misleading.
    let mut summary = summary;
    if let Some(cg) = &app.collector.cgroup {
        if let Some(limit) = cg.cpu_limit {
            summary.spans.push(Span::styled(
                format!("  ⧉ limit {limit:.2} cores"),
                Style::default().fg(theme.accent2.color()),
            ));
        }
        if cg.nr_throttled.is_some_and(|n| n > 0) {
            summary.spans.push(Span::styled(
                "  ⏱ throttled",
                Style::default()
                    .fg(theme.grad(1.0))
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    f.render_widget(Paragraph::new(summary), parts[0]);
    let rest = parts[1];
    if rest.height == 0 {
        return;
    }

    let ncores = cpu.per_core.len();
    let cell_w = 16usize;
    let per_row = ((inner.width as usize) / cell_w).max(1);
    let needed_rows = ncores.div_ceil(per_row);

    let (graph_h, core_rows) = if app.per_core && ncores > 0 {
        let cr = needed_rows.min(rest.height.saturating_sub(2) as usize);
        ((rest.height as usize).saturating_sub(cr), cr)
    } else {
        (rest.height as usize, 0)
    };

    if graph_h > 0 {
        let series: Vec<f64> = cpu.global_history.iter().copied().collect();
        let lines = graph::braille_graph(&series, 100.0, rest.width as usize, graph_h, theme);
        let garea = Rect {
            height: graph_h as u16,
            ..rest
        };
        render_lines(f, garea, lines);
    }

    if core_rows > 0 {
        let meter_w = (cell_w.saturating_sub(10)).max(3);
        let mut lines = Vec::with_capacity(core_rows);
        for r in 0..core_rows {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for col in 0..per_row {
                let idx = r * per_row + col;
                if idx >= ncores {
                    break;
                }
                let pct = clamp_pct(cpu.per_core[idx]);
                spans.push(Span::styled(format!("{:>2} ", idx), dim(theme)));
                spans.extend(graph::meter_spans(pct, meter_w, theme));
                spans.push(Span::styled(
                    format!(" {:>3.0} ", pct),
                    Style::default().fg(theme.grad(pct / 100.0)),
                ));
            }
            lines.push(Line::from(spans));
        }
        let carea = Rect {
            y: rest.y + graph_h as u16,
            height: core_rows as u16,
            ..rest
        };
        render_lines(f, carea, lines);
    }
}

// ── Memory panel ─────────────────────────────────────────────────────────────

fn render_mem(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = panel("memory", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mem = &app.collector.mem;
    let ram_pct = if mem.total > 0 {
        mem.used as f32 / mem.total as f32 * 100.0
    } else {
        0.0
    };
    let swap_pct = if mem.swap_total > 0 {
        mem.swap_used as f32 / mem.swap_total as f32 * 100.0
    } else {
        0.0
    };

    let mw = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("ram ", dim(theme)),
        Span::styled(
            format!("{:>5.1}%", ram_pct),
            Style::default()
                .fg(theme.grad(ram_pct / 100.0))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} / {}", human_bytes(mem.used), human_bytes(mem.total)),
            Style::default().fg(theme.fg.color()),
        ),
    ]));
    lines.push(Line::from(graph::meter_spans(ram_pct, mw, theme)));
    // The container's own limit, which is what actually OOM-kills it.
    if let Some(cg) = &app.collector.cgroup {
        if let (Some(limit), Some(used)) = (cg.mem_limit, cg.mem_used) {
            let pct = cg.mem_pct().unwrap_or(0.0) as f32;
            lines.push(Line::from(vec![
                Span::styled("⧉   ", dim(theme)),
                Span::styled(
                    format!("{pct:>5.1}%"),
                    Style::default()
                        .fg(theme.grad(pct / 100.0))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {} / {} cgroup", human_bytes(used), human_bytes(limit)),
                    Style::default().fg(theme.accent2.color()),
                ),
            ]));
            lines.push(Line::from(graph::meter_spans(pct, mw, theme)));
        }
    }
    lines.push(Line::from(vec![
        Span::styled("swp ", dim(theme)),
        Span::styled(
            format!("{:>5.1}%", swap_pct),
            Style::default().fg(theme.swap.color()),
        ),
        Span::styled(
            format!(
                "  {} / {}",
                human_bytes(mem.swap_used),
                human_bytes(mem.swap_total)
            ),
            dim(theme),
        ),
    ]));
    lines.push(Line::from(swap_meter(swap_pct, mw, theme)));

    let used = lines.len() as u16;
    let head = Rect {
        height: used.min(inner.height),
        ..inner
    };
    render_lines(f, head, lines);

    if inner.height > used + 1 {
        let garea = Rect {
            y: inner.y + used,
            height: inner.height - used,
            ..inner
        };
        let series: Vec<f64> = mem.used_history.iter().copied().collect();
        let glines = graph::braille_graph(
            &series,
            100.0,
            garea.width as usize,
            garea.height as usize,
            theme,
        );
        render_lines(f, garea, glines);
    }
}

/// Swap meter uses a flat swap color rather than the load gradient.
fn swap_meter(pct: f32, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    let pct = pct.clamp(0.0, 100.0);
    let full = ((pct / 100.0) * width as f32).round() as usize;
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        if i < full {
            spans.push(Span::styled("█", Style::default().fg(theme.swap.color())));
        } else {
            spans.push(Span::styled(
                "░",
                Style::default()
                    .fg(theme.dim.color())
                    .add_modifier(Modifier::DIM),
            ));
        }
    }
    spans
}

// ── Network panel ────────────────────────────────────────────────────────────

fn render_net(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = panel("network", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some(net) = app.collector.nets.first() else {
        render_lines(
            f,
            inner,
            vec![Line::from(Span::styled("no interfaces", dim(theme)))],
        );
        return;
    };

    let summary = Line::from(vec![
        Span::styled(
            truncate(&net.name, 10),
            Style::default().fg(theme.accent2.color()),
        ),
        Span::styled("  ↓", Style::default().fg(theme.net_down.color())),
        Span::styled(
            format!(" {:<11}", human_rate(net.down_rate)),
            Style::default().fg(theme.net_down.color()),
        ),
        Span::styled("↑", Style::default().fg(theme.net_up.color())),
        Span::styled(
            format!(" {}", human_rate(net.up_rate)),
            Style::default().fg(theme.net_up.color()),
        ),
    ]);
    let totals = Line::from(Span::styled(
        format!(
            "Σ ↓{}  ↑{}",
            human_bytes(net.total_down),
            human_bytes(net.total_up)
        ),
        dim(theme),
    ));

    let parts = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);
    render_lines(f, parts[0], vec![summary, totals]);

    let graph_area = parts[1];
    if graph_area.height > 0 {
        let down: Vec<f64> = net.down_history.iter().copied().collect();
        let up: Vec<f64> = net.up_history.iter().copied().collect();
        let max = net.down_history.max().max(net.up_history.max()).max(1024.0);
        let lines = graph::mirror_graph(
            &down,
            &up,
            max,
            graph_area.width as usize,
            graph_area.height as usize,
            theme.net_down.color(),
            theme.net_up.color(),
        );
        render_lines(f, graph_area, lines);
    }
}

// ── Disk panel ───────────────────────────────────────────────────────────────

fn render_disk(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = panel("disk", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let c = &app.collector;
    let summary = Line::from(vec![
        Span::styled("io ", dim(theme)),
        Span::styled("R ", Style::default().fg(theme.disk_read.color())),
        Span::styled(
            format!("{:<11}", human_rate(c.disk_read_rate)),
            Style::default().fg(theme.disk_read.color()),
        ),
        Span::styled("W ", Style::default().fg(theme.disk_write.color())),
        Span::styled(
            human_rate(c.disk_write_rate),
            Style::default().fg(theme.disk_write.color()),
        ),
    ]);

    // When there's room, draw a mirrored read/write I/O graph below the summary.
    let graph_h: u16 = if inner.height >= 8 { 3 } else { 0 };
    let parts = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(graph_h),
        Constraint::Min(0),
    ])
    .split(inner);
    render_lines(f, parts[0], vec![summary]);

    if graph_h > 0 {
        let read: Vec<f64> = c.disk_read_history.iter().copied().collect();
        let write: Vec<f64> = c.disk_write_history.iter().copied().collect();
        let max = c
            .disk_read_history
            .max()
            .max(c.disk_write_history.max())
            .max(1024.0);
        let lines = graph::mirror_graph(
            &read,
            &write,
            max,
            parts[1].width as usize,
            parts[1].height as usize,
            theme.disk_read.color(),
            theme.disk_write.color(),
        );
        render_lines(f, parts[1], lines);
    }

    let list = parts[2];
    if list.height == 0 {
        return;
    }
    let mw = (list.width as usize).saturating_sub(24).max(4);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for d in c.disk_list.iter().take(list.height as usize) {
        let mut spans = vec![Span::styled(
            format!("{:<8}", truncate(&d.mount, 8)),
            Style::default().fg(theme.fg.color()),
        )];
        spans.extend(graph::meter_spans(d.used_pct, mw, theme));
        spans.push(Span::styled(
            format!(" {:>3.0}% ", d.used_pct),
            Style::default().fg(theme.grad(d.used_pct / 100.0)),
        ));
        spans.push(Span::styled(short_bytes(d.total), dim(theme)));
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("no filesystems", dim(theme))));
    }
    render_lines(f, list, lines);
}

// ── Sensors panel ────────────────────────────────────────────────────────────

fn render_sensors(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let gpus = &app.collector.gpus;
    let sensors = &app.collector.sensors;
    let title = if gpus.is_empty() {
        "sensors"
    } else {
        "gpu · sensors"
    };
    let block = panel(title, theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if gpus.is_empty() && sensors.is_empty() {
        render_lines(
            f,
            inner,
            vec![Line::from(Span::styled("no sensors detected", dim(theme)))],
        );
        return;
    }

    let cap = inner.height as usize;
    let mw = (inner.width as usize).saturating_sub(22).max(3);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // GPUs first — utilization meter, temperature, and VRAM usage.
    for (i, g) in gpus.iter().enumerate() {
        if lines.len() >= cap {
            break;
        }
        let u = clamp_pct(g.util_pct);
        let mut spans = vec![Span::styled(
            format!("gpu{:<7}", i),
            Style::default().fg(theme.accent2.color()),
        )];
        spans.extend(graph::meter_spans(
            if g.has_util { u } else { 0.0 },
            mw,
            theme,
        ));
        let util_txt = if g.has_util {
            format!("{:>3.0}%", u)
        } else {
            "  --".to_string()
        };
        // A GPU that reports no temperature (Apple Silicon, most integrated
        // GPUs) must not be rendered as a suspiciously cool 0 °C.
        let temp_txt = if g.temp > 0.0 {
            format!("{:>3.0}°C", g.temp)
        } else {
            "   —".to_string()
        };
        spans.push(Span::styled(
            format!(" {util_txt} {temp_txt}"),
            Style::default().fg(theme.grad(u / 100.0)),
        ));
        lines.push(Line::from(spans));
        if lines.len() < cap {
            // Matches the AI view: unified-memory GPUs report no VRAM total,
            // and "0 B / 0 B" reads like a broken driver rather than a
            // different memory architecture.
            let mem_txt = if g.mem_total > 0 {
                format!(
                    " · vram {} / {}",
                    human_bytes(g.mem_used),
                    human_bytes(g.mem_total)
                )
            } else if g.name.contains("Apple") {
                " · unified memory".to_string()
            } else {
                String::new()
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}{mem_txt}",
                    truncate(&g.name, (inner.width as usize).saturating_sub(24)),
                ),
                dim(theme),
            )));
        }
    }

    // Then temperature sensors.
    for s in sensors {
        if lines.len() >= cap {
            break;
        }
        let scale = s.critical.or(s.high).unwrap_or(100.0).max(1.0);
        let t = (s.temp / scale).clamp(0.0, 1.0);
        let mut spans = vec![Span::styled(
            format!("{:<10}", truncate(&s.label, 10)),
            Style::default().fg(theme.fg.color()),
        )];
        spans.extend(graph::meter_spans(t * 100.0, mw, theme));
        spans.push(Span::styled(
            format!(" {:>4.0}°C", s.temp),
            Style::default().fg(theme.grad(t)),
        ));
        lines.push(Line::from(spans));
    }
    render_lines(f, inner, lines);
}

// ── Process table ────────────────────────────────────────────────────────────

fn render_procs(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = app.theme();
    let title = if app.filter.is_empty() {
        format!(
            "processes · {} ({})",
            app.sort.label(),
            if app.sort_desc { "▼" } else { "▲" }
        )
    } else {
        format!("processes · filter:\"{}\"", app.filter)
    };
    let block = panel(&title, theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height < 2 {
        app.proc_rows = 0;
        return;
    }

    let rows_cap = (inner.height - 1) as usize; // minus header
    app.proc_rows = rows_cap;

    // Keep selection visible by adjusting the scroll offset.
    let sel = app
        .selected_pid
        .and_then(|pid| app.proc_view.iter().position(|p| p.pid == pid));
    if let Some(si) = sel {
        if si < app.proc_offset {
            app.proc_offset = si;
        } else if si >= app.proc_offset + rows_cap {
            app.proc_offset = si + 1 - rows_cap;
        }
    }
    let max_offset = app.proc_view.len().saturating_sub(rows_cap);
    app.proc_offset = app.proc_offset.min(max_offset);

    app.proc_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: rows_cap as u16,
    };

    // Narrow terminals drop columns rather than squeezing all of them into
    // unreadable stubs. The result also drives click-to-sort, so the header
    // and the hit map can't disagree.
    let columns = crate::app::fit_columns(&app.columns, inner.width);
    app.visible_columns = columns.clone();
    let header = Row::new(
        columns
            .iter()
            .map(|c| Cell::from(c.header()))
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(theme.accent.color())
            .add_modifier(Modifier::BOLD),
    );

    let mut rows: Vec<Row> = Vec::with_capacity(rows_cap);
    let end = (app.proc_offset + rows_cap).min(app.proc_view.len());
    for p in &app.proc_view[app.proc_offset..end] {
        let selected = Some(p.pid) == app.selected_pid;
        let cmd = if app.tree && p.depth > 0 {
            format!("{}{}", "  ".repeat(p.depth), truncate(&p.cmd, 200))
        } else {
            p.cmd.clone()
        };
        let cpu = clamp_pct(p.cpu);
        let row = Row::new(
            columns
                .iter()
                .map(|c| proc_cell(*c, p, &cmd, cpu, theme))
                .collect::<Vec<_>>(),
        );
        let row = if selected {
            row.style(
                Style::default()
                    .bg(theme.selection.color())
                    .fg(theme.accent2.color())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            row.style(Style::default().fg(theme.fg.color()))
        };
        rows.push(row);
    }

    let widths: Vec<Constraint> = columns
        .iter()
        .map(|c| match c.width() {
            Some(w) => Constraint::Length(w),
            None => Constraint::Min(10),
        })
        .collect();
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    f.render_widget(table, inner);
}

/// Render one process-table cell. `cmd` is the (possibly tree-indented) command
/// and `cpu` the clamped CPU percentage, both computed once per row.
fn proc_cell<'a>(col: ProcColumn, p: &'a ProcInfo, cmd: &str, cpu: f32, theme: &Theme) -> Cell<'a> {
    match col {
        ProcColumn::Pid => Cell::from(format!("{:>7}", p.pid)),
        ProcColumn::User => Cell::from(truncate(&p.user, 9)),
        ProcColumn::Cpu => Cell::from(Span::styled(
            format!("{:>5.1}", p.cpu.min(999.0)),
            Style::default().fg(theme.grad(cpu / 100.0)),
        )),
        ProcColumn::MemPct => Cell::from(Span::styled(
            format!("{:>5.1}", p.mem_pct),
            Style::default().fg(theme.grad((p.mem_pct / 100.0).clamp(0.0, 1.0))),
        )),
        ProcColumn::Mem => Cell::from(short_bytes(p.mem_bytes)),
        ProcColumn::Disk => Cell::from({
            let io = p.io_read_rate + p.io_write_rate;
            if io >= 1.0 {
                Span::styled(
                    format!("{}/s", short_bytes(io as u64)),
                    Style::default().fg(theme.disk_write.color()),
                )
            } else {
                Span::styled("·", dim(theme))
            }
        }),
        ProcColumn::Vram => Cell::from(if p.gpu_mem > 0 {
            Span::styled(
                short_bytes(p.gpu_mem),
                Style::default().fg(theme.accent2.color()),
            )
        } else {
            Span::styled("·", dim(theme))
        }),
        ProcColumn::Time => Cell::from(compact_duration(p.run_time)),
        ProcColumn::State => Cell::from(p.status.to_string()),
        ProcColumn::Container => Cell::from(match &p.container {
            Some(c) => Span::styled(truncate(c, 14), Style::default().fg(theme.accent2.color())),
            // A process outside any container, which is information too.
            None => Span::styled("·", dim(theme)),
        }),
        ProcColumn::Command => Cell::from(cmd.to_string()),
    }
}

// ── Footer ───────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    if app.filter_mode {
        let line = Line::from(vec![
            Span::styled(
                "filter ",
                Style::default()
                    .fg(theme.accent.color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}_", app.filter),
                Style::default().fg(theme.fg.color()),
            ),
            Span::styled("  (Enter: apply · Esc: clear)", dim(theme)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    // Replay and recording are modes, not events — they belong in the footer
    // permanently, not in the transient status line.
    if let Some(r) = &app.replay {
        let pct = if r.len() > 1 {
            (r.position() as f64 / (r.len() - 1) as f64 * 100.0).round()
        } else {
            100.0
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if app.paused {
                        " ▮▮ REPLAY "
                    } else {
                        " ▶ REPLAY "
                    },
                    Style::default()
                        .bg(theme.accent.color())
                        .fg(theme.bg.unwrap_or(theme.selection).color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" frame {}/{} ({pct:.0}%)", r.position() + 1, r.len()),
                    Style::default().fg(theme.accent2.color()),
                ),
                Span::styled("  space: pause · ←/→: step · q: quit", dim(theme)),
            ])),
            area,
        );
        return;
    }
    if let Some((msg, _)) = &app.status {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(theme.accent2.color()),
            ))),
            area,
        );
        return;
    }
    let hint = |k: &'static str, d: &'static str, theme: &Theme| {
        vec![
            Span::styled(
                k,
                Style::default()
                    .fg(theme.accent.color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(":{} ", d), dim(theme)),
        ]
    };
    let mut spans = Vec::new();
    for (k, d) in [
        ("?", "help"),
        ("a", "ai"),
        ("C", "group"),
        ("Enter", "detail"),
        ("n", "net"),
        ("s", "sort"),
        ("t", "tree"),
        ("/", "filter"),
        ("K", "signal"),
        ("L", "layout"),
        ("q", "quit"),
    ] {
        spans.extend(hint(k, d, theme));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

// ── Overlays ─────────────────────────────────────────────────────────────────

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let Some(p) = app.selected_proc() else {
        return;
    };
    // Grow with the terminal: the base rows are fixed, the three detail
    // sections take whatever is left.
    let height = (area.height.saturating_sub(4)).clamp(18, 40);
    let width = (area.width.saturating_sub(6)).clamp(60, 96);
    let rect = centered(area, width, height);
    f.render_widget(Clear, rect);
    let block = panel(
        &format!("process · {} ({})", truncate(&p.name, 28), p.pid),
        theme,
    );
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.width == 0 {
        return;
    }
    let wrap = inner.width.saturating_sub(14) as usize;

    // Executable path and cwd are resolved on demand for just this process
    // rather than cached on every row of the table.
    let (exe, cwd) = app.collector.proc_paths(p.pid);

    let started = chrono::DateTime::from_timestamp(p.start_time as i64, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "—".into());

    let row = |label: &'static str, value: String, color| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{:<11}", label), dim(theme)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };

    let cpu = clamp_pct(p.cpu);
    let lines = vec![
        row(
            "PID / PPID",
            format!(
                "{} / {}",
                p.pid,
                p.ppid.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
            ),
            theme.fg.color(),
        ),
        row("User", p.user.clone(), theme.accent2.color()),
        row(
            "State",
            format!("{} ({})", p.status_long, p.status),
            theme.fg.color(),
        ),
        row("Threads", p.threads.to_string(), theme.fg.color()),
        row("CPU", format!("{:.1}%", p.cpu), theme.grad(cpu / 100.0)),
        row(
            "Memory",
            format!("{} ({:.1}%)", human_bytes(p.mem_bytes), p.mem_pct),
            theme.grad((p.mem_pct / 100.0).clamp(0.0, 1.0)),
        ),
        row("Virtual", human_bytes(p.virt), theme.fg.color()),
        row(
            "Disk R/W",
            format!(
                "{} / {}",
                human_bytes(p.disk_read),
                human_bytes(p.disk_written)
            ),
            theme.fg.color(),
        ),
        row(
            "Disk rate",
            format!(
                "R {}  W {}",
                human_rate(p.io_read_rate),
                human_rate(p.io_write_rate)
            ),
            theme.disk_write.color(),
        ),
        row("Started", started, theme.fg.color()),
        row("Run time", human_duration(p.run_time), theme.fg.color()),
        row(
            "Exe",
            truncate(if exe.is_empty() { "—" } else { &exe }, wrap),
            theme.fg.color(),
        ),
        row(
            "Cwd",
            truncate(if cwd.is_empty() { "—" } else { &cwd }, wrap),
            theme.fg.color(),
        ),
        row("Command", truncate(&p.cmd, wrap), theme.dim.color()),
    ];
    let mut lines = lines;

    // Open files, sockets and environment — fetched only while this overlay is
    // open. Each section is capped and reports what it elided, so a process
    // with 4000 descriptors doesn't push everything else off the panel.
    if let Some(d) = &app.detail {
        let budget = (inner.height as usize).saturating_sub(lines.len() + 2);
        let per_section = (budget / 3).max(1);

        let files: Vec<&crate::metrics::OpenFile> =
            d.open_files.iter().filter(|f| !f.is_pseudo()).collect();
        section(
            &mut lines,
            theme,
            &format!("Open files ({})", d.open_files.len()),
            files.len(),
            per_section,
            files.iter().take(per_section).map(|f| {
                Line::from(vec![
                    Span::styled(format!("  {:>4}  ", f.fd), dim(theme)),
                    Span::styled(
                        truncate(&f.target, wrap),
                        Style::default().fg(theme.fg.color()),
                    ),
                ])
            }),
        );

        section(
            &mut lines,
            theme,
            &format!("Sockets ({})", d.connections.len()),
            d.connections.len(),
            per_section,
            d.connections.iter().take(per_section).map(|c| {
                Line::from(vec![
                    Span::styled(format!("  {:<5} ", c.proto), dim(theme)),
                    Span::styled(
                        format!("{:<22}", truncate(&c.local, 22)),
                        Style::default().fg(theme.accent2.color()),
                    ),
                    Span::styled(
                        format!("{:<22}", truncate(&c.remote, 22)),
                        Style::default().fg(theme.fg.color()),
                    ),
                    Span::styled(c.state, dim(theme)),
                ])
            }),
        );

        section(
            &mut lines,
            theme,
            &format!("Environment ({})", d.env.len()),
            d.env.len(),
            per_section,
            d.env.iter().take(per_section).map(|(k, v)| {
                Line::from(vec![
                    Span::styled(
                        format!("  {k}="),
                        Style::default().fg(theme.accent2.color()),
                    ),
                    Span::styled(truncate(v, wrap.saturating_sub(k.len())), dim(theme)),
                ])
            }),
        );
    }

    lines.push(Line::from(Span::styled(
        "Enter / Esc to close · K to signal",
        dim(theme),
    )));
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Append a titled detail section, or a dimmed "none" line when it is empty.
/// `total` vs `shown` drives the "+N more" note, so an elided list never looks
/// like a complete one.
fn section<'a>(
    lines: &mut Vec<Line<'a>>,
    theme: &Theme,
    title: &str,
    total: usize,
    cap: usize,
    rows: impl Iterator<Item = Line<'a>>,
) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(theme.accent.color())
            .add_modifier(Modifier::BOLD),
    )));
    let mut shown = 0;
    for row in rows {
        lines.push(row);
        shown += 1;
    }
    if shown == 0 {
        lines.push(Line::from(Span::styled("  —", dim(theme))));
    } else if total > cap {
        lines.push(Line::from(Span::styled(
            format!("  … {} more", total - cap),
            dim(theme),
        )));
    }
}

fn render_signal_menu(f: &mut Frame, area: Rect, theme: &Theme, idx: usize, app: &App) {
    let target = app
        .selected_proc()
        .map(|p| format!("{} ({})", truncate(&p.name, 20), p.pid))
        .unwrap_or_else(|| "process".into());
    let rect = centered(area, 40, (crate::metrics::SIGNALS.len() + 3) as u16);
    f.render_widget(Clear, rect);
    let block = panel(&format!("signal · {}", target), theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, (name, num, _)) in crate::metrics::SIGNALS.iter().enumerate() {
        let selected = i == idx;
        let style = if selected {
            Style::default()
                .bg(theme.selection.color())
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg.color())
        };
        let marker = if selected { "▶ " } else { "  " };
        // `num` is 0 on platforms without signal numbers (Windows) — showing
        // it would just be noise.
        let label = if *num > 0 {
            format!("{}{:<10} {:>2}", marker, name, num)
        } else {
            format!("{}{}", marker, name)
        };
        lines.push(Line::from(Span::styled(label, style)));
    }
    lines.push(Line::from(Span::styled(
        "↑/↓ select · Enter send · Esc cancel",
        dim(theme),
    )));
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_renice_menu(f: &mut Frame, area: Rect, theme: &Theme, idx: usize, app: &App) {
    let target = app
        .selected_proc()
        .map(|p| format!("{} ({})", truncate(&p.name, 20), p.pid))
        .unwrap_or_else(|| "process".into());
    let rect = centered(area, 40, (crate::app::NICE_LEVELS.len() + 3) as u16);
    f.render_widget(Clear, rect);
    let block = panel(&format!("renice · {}", target), theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, _)) in crate::app::NICE_LEVELS.iter().enumerate() {
        let selected = i == idx;
        let style = if selected {
            Style::default()
                .bg(theme.selection.color())
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg.color())
        };
        let marker = if selected { "▶ " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{}nice {}", marker, label),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(
        "↑/↓ select · Enter apply · Esc cancel",
        dim(theme),
    )));
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// The AI / local-LLM view: the GPU metrics that actually predict inference
/// performance — core vs. memory-bandwidth utilization, VRAM headroom (spill
/// risk), power/throttle, and which processes hold GPU memory.
fn render_ai(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let c = &app.collector;
    // The panel is sized to its content, so a machine with one GPU and no
    // inference server doesn't get a box two-thirds full of empty rows. The
    // lines are therefore built against a provisional area first, and the real
    // rect is derived from how many there turned out to be.
    let width = 78.min(area.width);
    let inner = Rect {
        x: 0,
        y: 0,
        width: width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width < 4 || inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let bw = (inner.width as usize).saturating_sub(26).clamp(6, 30);

    // Firing alerts get top billing — this is the inference health banner.
    // The verdict first: every panel below reports numbers, this says what
    // they mean together. Deliberately at most a few lines — a diagnosis that
    // needs scrolling isn't one.
    let findings = crate::diagnose::diagnose(c);
    if !findings.is_empty() {
        for f in findings.iter().take(3) {
            let hot = f.severity == crate::alerts::Level::Crit;
            lines.push(Line::from(vec![
                Span::styled(
                    if hot { "▲ " } else { "◆ " },
                    Style::default().fg(theme.grad(if hot { 1.0 } else { 0.6 })),
                ),
                Span::styled(
                    f.headline,
                    Style::default()
                        .fg(theme.grad(if hot { 1.0 } else { 0.6 }))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("   {}", f.evidence), dim(theme)),
            ]));
            lines.extend(wrap_text(
                &format!("→ {}", f.advice),
                Style::default().fg(theme.fg.color()),
                inner.width as usize,
                "  ",
            ));
        }
        lines.push(Line::from(Span::raw("")));
    }

    if !app.alerts.is_empty() {
        lines.push(Line::from(Span::styled(
            "⚠ ALERTS",
            Style::default()
                .fg(theme.grad(1.0))
                .add_modifier(Modifier::BOLD),
        )));
        for a in &app.alerts {
            let hot = a.level == crate::alerts::Level::Crit;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  [{}] ", a.level.label()),
                    Style::default().fg(theme.grad(if hot { 1.0 } else { 0.7 })),
                ),
                // Truncate with an ellipsis rather than letting the panel
                // border clip mid-word — a cut-off alert reads as a rendering
                // bug on top of whatever it was warning about.
                Span::styled(
                    truncate(&a.message, (inner.width as usize).saturating_sub(9)),
                    Style::default().fg(theme.fg.color()),
                ),
            ]));
        }
        lines.push(Line::from(Span::raw("")));
    }

    if c.gpus.is_empty() {
        lines.push(Line::from(Span::styled(
            crate::metrics::gpu::no_gpu_reason(),
            dim(theme),
        )));
        lines.push(Line::from(Span::styled(
            "AI workloads and inference servers still show below.",
            dim(theme),
        )));
        // Someone who opens this view and finds it empty has no reason to
        // guess that the whole thing can be demonstrated without a GPU.
        if !app.demo {
            lines.push(Line::from(vec![
                Span::styled("Run ", dim(theme)),
                Span::styled(
                    "toptop --demo",
                    Style::default()
                        .fg(theme.accent.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to see this view with a simulated GPU.", dim(theme)),
            ]));
        }
        lines.push(Line::from(Span::raw("")));
    }

    for (i, g) in c.gpus.iter().enumerate() {
        let u = clamp_pct(g.util_pct);
        let mem_pct = clamp_pct(g.mem_pct());
        // Header line: name + power/throttle.
        let mut head = vec![Span::styled(
            format!("gpu{}  {}", i, truncate(&g.name, 26)),
            Style::default()
                .fg(theme.accent2.color())
                .add_modifier(Modifier::BOLD),
        )];
        if g.power_limit > 0.0 {
            head.push(Span::styled(
                format!("   {:.0}/{:.0}W", g.power, g.power_limit),
                dim(theme),
            ));
        }
        if g.temp > 0.0 {
            head.push(Span::styled(
                format!("  {:.0}°C", g.temp),
                Style::default().fg(theme.grad((g.temp / 95.0).clamp(0.0, 1.0))),
            ));
        }
        if g.throttled {
            head.push(Span::styled(
                "  ⚠ THROTTLING",
                Style::default()
                    .fg(theme.grad(1.0))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(head));

        // Core (compute) utilization.
        let mut core = vec![Span::styled(format!("  {:<10}", "compute"), dim(theme))];
        if g.has_util {
            core.extend(graph::meter_spans(u, bw, theme));
            core.push(Span::styled(
                format!(" {:>3.0}%", u),
                Style::default().fg(theme.grad(u / 100.0)),
            ));
        } else {
            core.push(Span::styled("   --", dim(theme)));
        }
        lines.push(Line::from(core));

        // Memory-bandwidth utilization — the LLM bottleneck nvidia-smi hides.
        let mut band = vec![Span::styled(format!("  {:<10}", "mem b/w"), dim(theme))];
        if g.has_mem_util {
            let mu = clamp_pct(g.mem_util);
            band.extend(graph::meter_spans(mu, bw, theme));
            band.push(Span::styled(
                format!(" {:>3.0}%", mu),
                Style::default().fg(theme.grad(mu / 100.0)),
            ));
        } else {
            band.push(Span::styled("   n/a (needs NVIDIA)", dim(theme)));
        }
        lines.push(Line::from(band));

        // VRAM with headroom + spill warning. Apple Silicon has no discrete
        // VRAM (unified memory) and reports mem_total == 0 — say so honestly
        // rather than draw a fake 0-byte bar.
        if g.mem_total > 0 {
            let mut vram = vec![Span::styled(format!("  {:<10}", "vram"), dim(theme))];
            vram.extend(graph::meter_spans(mem_pct, bw, theme));
            vram.push(Span::styled(
                format!(
                    " {} / {}",
                    human_bytes(g.mem_used),
                    human_bytes(g.mem_total)
                ),
                Style::default().fg(theme.grad(mem_pct / 100.0)),
            ));
            lines.push(Line::from(vram));
            if mem_pct >= 90.0 {
                lines.push(Line::from(Span::styled(
                    "             ⚠ near VRAM limit — models may spill to RAM (5–20× slower)",
                    Style::default().fg(theme.grad(1.0)),
                )));
            }
        } else if g.name.contains("Apple") {
            lines.push(Line::from(Span::styled(
                format!("  {:<10} unified memory (see the memory panel)", "vram"),
                dim(theme),
            )));
        } else {
            // Some other GPU that didn't report VRAM total — don't claim it's
            // unified; just show it's unavailable.
            lines.push(Line::from(Span::styled(
                format!("  {:<10} --", "vram"),
                dim(theme),
            )));
        }
        // Compute vs memory-bandwidth over time, mirrored around a midline.
        // Token generation is bandwidth-bound once the model is resident, so
        // the *divergence* between these two lines is the diagnosis: bandwidth
        // pinned while compute idles means you are memory-bound, and no amount
        // of a faster GPU core will help.
        if let Some(h) = c.gpu_history.get(i) {
            let graph_h = 4usize;
            // A GPU that reports no utilization at all (Apple Silicon, most
            // integrated GPUs) would otherwise get a "trend" heading over four
            // permanently blank rows.
            let has_signal = h.compute.max() > 0.0 || h.bandwidth.max() > 0.0;
            if has_signal
                && h.compute.len() >= 2
                && bw >= 12
                && lines.len() + graph_h + 1 < inner.height as usize
            {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<10}", "trend"), dim(theme)),
                    Span::styled("compute ▲", Style::default().fg(theme.accent.color())),
                    Span::styled("  /  ", dim(theme)),
                    Span::styled("▼ bandwidth", Style::default().fg(theme.accent2.color())),
                ]));
                // Only the visible window, so the graph scrolls with time
                // instead of squeezing an ever-longer history into `bw` cells.
                let (compute, bandwidth) = (h.compute.tail(bw * 2), h.bandwidth.tail(bw * 2));
                for line in graph::mirror_graph(
                    &compute,
                    &bandwidth,
                    100.0,
                    bw,
                    graph_h,
                    theme.accent.color(),
                    theme.accent2.color(),
                ) {
                    let mut spans = vec![Span::styled("            ", dim(theme))];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
            }
        }

        lines.push(Line::from(Span::raw("")));
    }

    // Multi-GPU aggregate + sharding-imbalance hint.
    let total_power: f32 = c.gpus.iter().map(|g| g.power).sum();
    if c.gpus.len() > 1 {
        let used: u64 = c.gpus.iter().map(|g| g.mem_used).sum();
        let total: u64 = c.gpus.iter().map(|g| g.mem_total).sum();
        let utils: Vec<f32> = c
            .gpus
            .iter()
            .filter(|g| g.has_util)
            .map(|g| g.util_pct)
            .collect();
        let mut spans = vec![
            Span::styled(
                format!("Σ {} GPUs  ", c.gpus.len()),
                Style::default().fg(theme.accent2.color()),
            ),
            Span::styled(
                format!("vram {} / {}", human_bytes(used), human_bytes(total)),
                dim(theme),
            ),
        ];
        if total_power > 0.0 {
            spans.push(Span::styled(format!("  {:.0} W", total_power), dim(theme)));
        }
        let spread = match (
            utils.iter().cloned().reduce(f32::min),
            utils.iter().cloned().reduce(f32::max),
        ) {
            (Some(mn), Some(mx)) => mx - mn,
            _ => 0.0,
        };
        if spread >= 40.0 {
            spans.push(Span::styled(
                "  ⚠ imbalance — uneven sharding?",
                Style::default().fg(theme.grad(0.7)),
            ));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(Span::raw("")));
    }

    // Auto-discovered inference servers — the app-level numbers (tokens/sec,
    // KV-cache pressure, queue depth) that nvidia-smi can't see.
    if !c.servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "Inference servers (auto-discovered)",
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        )));
        let cap = inner.height as usize;
        for sv in &c.servers {
            if lines.len() + 1 >= cap {
                break;
            }
            let mut head = vec![Span::styled(
                format!("  {}", sv.label()),
                Style::default()
                    .fg(theme.accent2.color())
                    .add_modifier(Modifier::BOLD),
            )];
            if !sv.model.is_empty() {
                head.push(Span::styled(
                    format!("  {}", truncate(&sv.model, 30)),
                    dim(theme),
                ));
            }
            if let Some(off) = sv.gpu_offload_pct {
                head.push(Span::styled(
                    format!("  {:.0}% on GPU", off),
                    Style::default().fg(theme.grad(off as f32 / 100.0)),
                ));
            }
            lines.push(Line::from(head));

            let mut stat = vec![Span::styled("    ", dim(theme))];
            if let Some(g) = sv.gen_tps {
                stat.push(Span::styled(
                    format!("{:.1} tok/s", g),
                    Style::default()
                        .fg(theme.grad(0.0))
                        .add_modifier(Modifier::BOLD),
                ));
                if total_power > 0.0 {
                    stat.push(Span::styled(
                        format!(" ({:.2} tok/s/W)", g as f32 / total_power),
                        dim(theme),
                    ));
                }
            }
            if let Some(p) = sv.prompt_tps {
                stat.push(Span::styled(
                    format!("  prefill {:.0}/s", p),
                    Style::default().fg(theme.accent2.color()),
                ));
            }
            if let Some(k) = sv.kv_pct {
                stat.push(Span::styled(
                    format!("  kv {:.0}%", k),
                    Style::default().fg(theme.grad((k / 100.0) as f32)),
                ));
            }
            if let Some(r) = sv.running {
                stat.push(Span::styled(
                    format!("  req {:.0}/{:.0}", r, sv.waiting.unwrap_or(0.0)),
                    dim(theme),
                ));
            }
            // Preemption is the signal nothing else surfaces: the server threw
            // away work it had already done because the KV cache ran out.
            // Throughput collapses while the GPU still looks busy.
            if let Some(rate) = sv.preempt_rate.filter(|r| *r > 0.0) {
                stat.push(Span::styled(
                    format!("  ⟲ preempt {rate:.1}/s"),
                    Style::default()
                        .fg(theme.grad(1.0))
                        .add_modifier(Modifier::BOLD),
                ));
            } else if let Some(total) = sv.preemptions.filter(|t| *t > 0.0) {
                // Not preempting now, but it has — worth knowing this server
                // has been under KV pressure at some point.
                stat.push(Span::styled(format!("  ⟲ {total:.0} total"), dim(theme)));
            }
            // Only fall back to the mean TTFT when no histogram was available;
            // the percentile line below says strictly more.
            if let (Some(t), None) = (sv.ttft_ms, sv.ttft) {
                stat.push(Span::styled(format!("  ttft {:.0}ms", t), dim(theme)));
            }
            if stat.len() > 1 {
                // The stats are a variable-length list of facts; on a narrow
                // panel they used to be clipped mid-word at the border, which
                // silently hid whichever fact came last — including preemption.
                lines.extend(wrap_spans(stat, inner.width as usize, "  "));
            }

            // Prefill vs decode as a first-class split: the two phases have
            // different bottlenecks (prefill is compute-bound, decode is
            // memory-bandwidth-bound), so the mix is what tells you which one
            // you are currently paying for.
            if let (Some(share), true) = (sv.prefill_share_pct(), lines.len() + 1 < cap) {
                let decode = 100.0 - share;
                let (phase, pct) = if share >= decode {
                    ("prefill", share)
                } else {
                    ("decode", decode)
                };
                lines.push(Line::from(vec![
                    Span::styled("    phase  ", dim(theme)),
                    Span::styled(
                        format!("prefill {share:.0}%"),
                        Style::default().fg(theme.accent2.color()),
                    ),
                    Span::styled(" · ", dim(theme)),
                    Span::styled(
                        format!("decode {decode:.0}%"),
                        Style::default().fg(theme.grad(0.0)),
                    ),
                    Span::styled(
                        format!("  — {phase}-dominated ({pct:.0}% of tokens)"),
                        dim(theme),
                    ),
                ]));
            }

            // The SLO triad: TTFT is how long until something appears, TPOT is
            // how fast it then streams. p95/p99 are what users actually feel.
            for (name, p) in [("ttft", sv.ttft), ("tpot", sv.tpot)] {
                let Some(p) = p else { continue };
                if lines.len() + 1 >= cap {
                    break;
                }
                // Scale the color by how far p95 drifts from p50: a long tail
                // is the interesting signal, not the absolute number.
                let spread = if p.p50 > 0.0 {
                    ((p.p95 / p.p50 - 1.0) / 3.0).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("    {name:<6} "), dim(theme)),
                    Span::styled(
                        format!("p50 {:.0}ms", p.p50),
                        Style::default().fg(theme.fg.color()),
                    ),
                    Span::styled(
                        format!("  p95 {:.0}ms", p.p95),
                        Style::default().fg(theme.grad(spread as f32)),
                    ),
                    Span::styled(format!("  p99 {:.0}ms", p.p99), dim(theme)),
                ]));
            }

            // Trend sparklines (nvtop-style): tokens/sec against its own peak,
            // KV% against 100. One braille row each, side by side.
            let spark_w = ((inner.width as usize).saturating_sub(16) / 2).min(20);
            if let Some(h) = c.server_history.get(&(sv.pid, sv.port)) {
                if h.tps.len() >= 2 && spark_w >= 6 && lines.len() + 1 < cap {
                    let spark = |series: &[f64], max: f64| -> Vec<Span<'static>> {
                        graph::braille_graph(series, max, spark_w, 1, theme)
                            .pop()
                            .map(|l| l.spans)
                            .unwrap_or_default()
                    };
                    let mut sp = vec![Span::styled("    tok/s ", dim(theme))];
                    sp.extend(spark(&h.tps.tail(spark_w * 2), h.tps.max()));
                    if sv.kv_pct.is_some() {
                        sp.push(Span::styled("  kv ", dim(theme)));
                        sp.extend(spark(&h.kv.tail(spark_w * 2), 100.0));
                    }
                    lines.push(Line::from(sp));
                }
            }
        }
        lines.push(Line::from(Span::raw("")));
    } else {
        lines.push(Line::from(Span::styled(
            crate::metrics::infer::no_servers_reason(),
            dim(theme),
        )));
        lines.push(Line::from(Span::raw("")));
    }

    // Detected AI workloads (serving + training), joined to CPU/RAM and — where
    // the PID matches a GPU compute process — VRAM.
    // Detected once per tick by `App::refresh_ai_workloads`, already sorted by
    // CPU — scanning every command line here would cost milliseconds a frame.
    let workloads = &app.ai_workloads;
    if !workloads.is_empty() {
        lines.push(Line::from(Span::styled(
            "AI workloads",
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "  {:<13} {:<6} {:>7} {:>5} {:>9} {:>9}",
                "RUNTIME", "TYPE", "PID", "CPU%", "RAM", "VRAM"
            ),
            dim(theme),
        )));
        let cap = inner.height as usize;
        for w in workloads {
            if lines.len() >= cap {
                break;
            }
            let (rt, vram) = (&w.runtime, &w.vram);
            let kind = match rt.kind {
                crate::metrics::ai::AiKind::Serving => "serve",
                crate::metrics::ai::AiKind::Training => "train",
            };
            // A training process pinning a CPU while its GPU sits idle is the
            // classic data-loader bottleneck — flag it.
            let dataloader_bound = rt.kind == crate::metrics::ai::AiKind::Training
                && w.cpu > 95.0
                && c.gpus.iter().any(|g| g.has_util && g.util_pct < 35.0);
            let mut spans = vec![Span::styled(
                format!(
                    "  {:<13} {:<6} {:>7} {:>5.1} {:>9} {:>9}",
                    truncate(rt.label, 13),
                    kind,
                    w.pid,
                    w.cpu,
                    human_bytes(w.mem_bytes),
                    if *vram > 0 {
                        human_bytes(*vram)
                    } else {
                        "—".to_string()
                    }
                ),
                Style::default().fg(theme.fg.color()),
            )];
            if dataloader_bound {
                spans.push(Span::styled(
                    "  ⚠ dataloader-bound?",
                    Style::default().fg(theme.grad(0.7)),
                ));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(Span::raw("")));
    }

    // Per-process VRAM, joined with our process table for names + CPU%.
    if !c.gpu_procs.is_empty() {
        lines.push(Line::from(Span::styled(
            "GPU processes (by VRAM)",
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:>7}  {:>9}  {:>5}  {}",
                "PID", "VRAM", "CPU%", "PROCESS"
            ),
            dim(theme),
        )]));
        let mut gprocs = c.gpu_procs.clone();
        gprocs.sort_by_key(|g| std::cmp::Reverse(g.used_mem));
        let cap = inner.height as usize;
        for gp in gprocs.iter() {
            if lines.len() >= cap {
                break;
            }
            let proc = c.procs.iter().find(|p| p.pid == gp.pid);
            let name = proc.map(|p| p.name.as_str()).unwrap_or("?");
            let cpu = proc.map(|p| p.cpu).unwrap_or(0.0);
            lines.push(Line::from(Span::styled(
                format!(
                    "  {:>7}  {:>9}  {:>5.1}  {}",
                    gp.pid,
                    human_bytes(gp.used_mem),
                    cpu,
                    truncate(name, 28)
                ),
                Style::default().fg(theme.fg.color()),
            )));
        }
    } else if c.gpus.iter().any(|g| g.name.contains("NVIDIA")) {
        lines.push(Line::from(Span::styled(
            "No GPU compute processes (or insufficient permissions).",
            dim(theme),
        )));
    }

    // Trailing blank lines are an artifact of the per-GPU spacer, not content.
    while lines.last().is_some_and(|l| line_is_blank(l)) {
        lines.pop();
    }
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = centered(area, width, height);
    f.render_widget(Clear, rect);
    let block = panel("AI · local-LLM GPU view · Esc/a to close", theme);
    let target = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(Text::from(lines)), target);
}

/// Wrap a single styled string across lines at word boundaries, indenting
/// continuations. For prose (diagnosis advice), where splitting between whole
/// spans isn't enough because the whole paragraph is one span.
fn wrap_text(text: &str, style: Style, width: usize, indent: &str) -> Vec<Line<'static>> {
    let usable = width.saturating_sub(indent.chars().count()).max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let extra = if current.is_empty() { 0 } else { 1 };
        if current.chars().count() + extra + word.chars().count() > usable && !current.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("{indent}{current}"),
                style,
            )));
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{indent}{current}"),
            style,
        )));
    }
    lines
}

/// Pack `spans` into lines of at most `width` cells, indenting continuations
/// with `indent`. Splits only between spans — each one is a self-contained
/// fact, and half a fact is worse than a wrapped one.
fn wrap_spans<'a>(spans: Vec<Span<'a>>, width: usize, indent: &'a str) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let mut current: Vec<Span<'a>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let len = span.content.chars().count();
        if used + len > width && !current.is_empty() {
            lines.push(Line::from(std::mem::take(&mut current)));
            current.push(Span::raw(indent));
            used = indent.chars().count();
        }
        used += len;
        current.push(span);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

/// Whether a rendered line carries no visible text.
fn line_is_blank(line: &Line<'_>) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

fn render_connections(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = app.theme();
    let rect = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    f.render_widget(Clear, rect);
    let (tcp, udp) = app.connections.iter().fold((0usize, 0usize), |(t, u), c| {
        if c.proto.starts_with("tcp") {
            (t + 1, u)
        } else {
            (t, u + 1)
        }
    });
    let block = panel(
        &format!(
            "network connections · {} ({} tcp · {} udp) · Esc/n to close",
            app.connections.len(),
            tcp,
            udp
        ),
        theme,
    );
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height < 2 {
        app.conn_rows = 0;
        return;
    }

    let rows_cap = (inner.height - 1) as usize;
    app.conn_rows = rows_cap;
    let max_offset = app.connections.len().saturating_sub(rows_cap);
    app.conn_offset = app.conn_offset.min(max_offset);

    // An empty table looks like a bug on platforms that can't enumerate at all.
    if app.connections.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                crate::metrics::netconn::no_connections_reason(),
                dim(theme),
            ))),
            inner,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from("PROTO"),
        Cell::from("LOCAL ADDRESS"),
        Cell::from("REMOTE ADDRESS"),
        Cell::from("STATE"),
        Cell::from("PID"),
        Cell::from("PROCESS"),
    ])
    .style(
        Style::default()
            .fg(theme.accent.color())
            .add_modifier(Modifier::BOLD),
    );

    let state_color = |s: &str| match s {
        "LISTEN" => theme.accent2.color(),
        "ESTABLISHED" => theme.grad(0.0),
        "SYN_SENT" | "SYN_RECV" => theme.grad(0.5),
        "—" => theme.dim.color(),
        _ => theme.dim.color(),
    };

    let end = (app.conn_offset + rows_cap).min(app.connections.len());
    let mut rows: Vec<Row> = Vec::with_capacity(rows_cap);
    for c in &app.connections[app.conn_offset..end] {
        rows.push(
            Row::new(vec![
                Cell::from(c.proto),
                Cell::from(truncate(&c.local, 30)),
                Cell::from(truncate(&c.remote, 30)),
                Cell::from(Span::styled(
                    c.state,
                    Style::default().fg(state_color(c.state)),
                )),
                Cell::from(
                    c.pid
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "—".to_string()),
                ),
                Cell::from(truncate(&c.process, 24)),
            ])
            .style(Style::default().fg(theme.fg.color())),
        );
    }
    if rows.is_empty() {
        rows.push(Row::new(vec![Cell::from(Span::styled(
            "no connections (or insufficient permissions to map sockets)",
            dim(theme),
        ))]));
    }

    let widths = [
        Constraint::Length(5),
        Constraint::Length(32),
        Constraint::Length(32),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    f.render_widget(table, inner);
}

/// Timeline of recent alert fire/resolve transitions, newest first.
///
/// The banner only ever shows what is firing *right now*; this is the "what
/// happened while I wasn't looking" view, and it is where flap suppression
/// becomes visible as a count rather than as silence.
fn render_alert_history(f: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let history = app.tracker.history();
    let suppressed = app.tracker.suppressed();

    let rect = centered(area, 78, (history.len().clamp(1, 16) + 4) as u16);
    f.render_widget(Clear, rect);
    let title = if suppressed > 0 {
        format!(
            "alert history · {} events · {suppressed} flap{} suppressed",
            history.len(),
            if suppressed == 1 { "" } else { "s" }
        )
    } else {
        format!("alert history · {} events", history.len())
    };
    let block = panel(&title, theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    if history.is_empty() {
        lines.push(Line::from(Span::styled(
            "No alerts have fired since toptop started.",
            dim(theme),
        )));
    } else {
        let now = Instant::now();
        let rows = (inner.height as usize).saturating_sub(1);
        for t in history.iter().rev().take(rows) {
            let (marker, style) = match t.state {
                TransitionState::Fired => (
                    "▲ fired   ",
                    Style::default().fg(match t.level {
                        Level::Crit => theme.grad(1.0),
                        Level::Warn => theme.grad(0.55),
                    }),
                ),
                TransitionState::Resolved => {
                    ("▼ resolved", Style::default().fg(theme.net_down.color()))
                }
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{:>5}  ", compact_age(now, t.at)), dim(theme)),
                Span::styled(marker, style),
                Span::styled(
                    format!("  {}", t.message),
                    Style::default().fg(theme.fg.color()),
                ),
            ]));
        }
    }
    lines.push(Line::from(Span::styled("Esc/A to close", dim(theme))));
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Compact relative age, e.g. `12s`, `4m`, `2h`.
fn compact_age(now: Instant, then: Instant) -> String {
    let secs = now.saturating_duration_since(then).as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn render_help(f: &mut Frame, area: Rect, theme: &Theme) {
    let rect = centered(area, 56, 26);
    f.render_widget(Clear, rect);
    let block = panel("help · toptop", theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let key = |s: &'static str| {
        Span::styled(
            format!("{:<14}", s),
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        )
    };
    let desc = |s: &'static str| Span::styled(s, Style::default().fg(theme.fg.color()));
    let entries: &[(&str, &str)] = &[
        ("↑/↓ k/j", "move selection"),
        ("PgUp/PgDn", "page up / down"),
        ("Home/End g/G", "jump to first / last"),
        ("Enter", "process details"),
        ("s", "cycle sort column"),
        ("i", "invert sort order"),
        ("click header", "sort by column"),
        ("t", "toggle process tree"),
        ("e", "toggle per-core CPU meters"),
        ("a", "AI / local-LLM GPU view"),
        ("C", "group by container / pod"),
        ("A", "alert history timeline"),
        ("n", "network connections"),
        ("L", "cycle layout preset"),
        ("/", "filter processes"),
        ("K / F9", "signal menu"),
        ("Del", "terminate (SIGTERM)"),
        ("x", "kill (SIGKILL)"),
        ("r", "renice (change priority)"),
        ("p / P", "next / prev theme"),
        ("+ / -", "faster / slower refresh"),
        ("space", "pause / resume"),
        ("? / F1", "toggle this help"),
        ("q / Ctrl-C", "quit"),
    ];
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "keybindings",
        Style::default()
            .fg(theme.accent2.color())
            .add_modifier(Modifier::BOLD),
    ))];
    for (k, d) in entries {
        lines.push(Line::from(vec![key(k), desc(d)]));
    }
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_confirm(f: &mut Frame, area: Rect, theme: &Theme, pk: &crate::app::PendingKill) {
    let rect = centered(area, 50, 5);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.grad(1.0)))
        .title(Span::styled(
            " confirm ",
            Style::default()
                .fg(theme.grad(1.0))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let sig = crate::metrics::signal_name(pk.signal);
    let lines = vec![
        Line::from(vec![
            Span::styled("Send ", Style::default().fg(theme.fg.color())),
            Span::styled(
                sig,
                Style::default()
                    .fg(theme.grad(1.0))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" to {} ({})?", truncate(&pk.name, 20), pk.pid),
                Style::default().fg(theme.fg.color()),
            ),
        ]),
        Line::from(Span::styled("[y] confirm    [n / Esc] cancel", dim(theme))),
    ];
    f.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        inner,
    );
}
