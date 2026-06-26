//! All terminal rendering. Pure functions of `(&mut Frame, &mut App)`.

pub mod graph;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
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
    if app.show_detail {
        render_detail(f, area, app);
    }
    if let Some(idx) = app.signal_menu {
        render_signal_menu(f, area, theme, idx, app);
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
    if let Some(bat) = &c.battery {
        let t = (bat.percent / 100.0).clamp(0.0, 1.0);
        let glyph = if bat.status.eq_ignore_ascii_case("charging") {
            "⚡"
        } else {
            "🔋"
        };
        spans.push(Span::styled(
            format!("  {}{:.0}%", glyph, bat.percent),
            Style::default().fg(theme.grad(t)),
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

    // Live clock, right-aligned on the same row.
    let clock = chrono::Local::now().format("%H:%M:%S").to_string();
    let left = Paragraph::new(Line::from(spans));
    f.render_widget(left, area);
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
        spans.push(Span::styled(
            format!(" {} {:>3.0}°C", util_txt, g.temp),
            Style::default().fg(theme.grad(u / 100.0)),
        ));
        lines.push(Line::from(spans));
        if lines.len() < cap {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} · vram {} / {}",
                    truncate(&g.name, (inner.width as usize).saturating_sub(24)),
                    human_bytes(g.mem_used),
                    human_bytes(g.mem_total)
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

    let header = Row::new(vec![
        Cell::from("PID"),
        Cell::from("USER"),
        Cell::from("CPU%"),
        Cell::from("MEM%"),
        Cell::from("MEM"),
        Cell::from("DISK"),
        Cell::from("TIME"),
        Cell::from("S"),
        Cell::from("COMMAND"),
    ])
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
        let row = Row::new(vec![
            Cell::from(format!("{:>7}", p.pid)),
            Cell::from(truncate(&p.user, 9)),
            Cell::from(Span::styled(
                format!("{:>5.1}", p.cpu.min(999.0)),
                Style::default().fg(theme.grad(cpu / 100.0)),
            )),
            Cell::from(Span::styled(
                format!("{:>5.1}", p.mem_pct),
                Style::default().fg(theme.grad((p.mem_pct / 100.0).clamp(0.0, 1.0))),
            )),
            Cell::from(short_bytes(p.mem_bytes)),
            Cell::from({
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
            Cell::from(compact_duration(p.run_time)),
            Cell::from(p.status.to_string()),
            Cell::from(cmd),
        ]);
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

    let widths = [
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(1),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    f.render_widget(table, inner);
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
    let rect = centered(area, 72, 18);
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
            truncate(if p.exe.is_empty() { "—" } else { &p.exe }, wrap),
            theme.fg.color(),
        ),
        row(
            "Cwd",
            truncate(if p.cwd.is_empty() { "—" } else { &p.cwd }, wrap),
            theme.fg.color(),
        ),
        row("Command", truncate(&p.cmd, wrap), theme.dim.color()),
        Line::from(Span::styled(
            "Enter / Esc to close · K to signal",
            dim(theme),
        )),
    ];
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_signal_menu(f: &mut Frame, area: Rect, theme: &Theme, idx: usize, app: &App) {
    let target = app
        .selected_proc()
        .map(|p| format!("{} ({})", truncate(&p.name, 20), p.pid))
        .unwrap_or_else(|| "process".into());
    let rect = centered(area, 40, (crate::app::SIGNALS.len() + 3) as u16);
    f.render_widget(Clear, rect);
    let block = panel(&format!("signal · {}", target), theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, (name, num, _)) in crate::app::SIGNALS.iter().enumerate() {
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
            format!("{}{:<10} {:>2}", marker, name, num),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(
        "↑/↓ select · Enter send · Esc cancel",
        dim(theme),
    )));
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
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
        ("n", "network connections"),
        ("L", "cycle layout preset"),
        ("/", "filter processes"),
        ("K / F9", "signal menu"),
        ("Del", "terminate (SIGTERM)"),
        ("x", "kill (SIGKILL)"),
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
    let sig = match pk.signal {
        sysinfo::Signal::Kill => "SIGKILL",
        _ => "SIGTERM",
    };
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
