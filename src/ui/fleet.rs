//! Rendering for the multi-host fleet dashboard.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use super::graph;
use super::{dim, panel};
use crate::fleet::{FleetApp, HostStatus};
use crate::theme::Theme;
use crate::util::{human_bytes, human_duration, short_bytes};

/// Draw the entire fleet view.
pub fn draw(f: &mut Frame, app: &FleetApp) {
    let area = f.area();
    let theme = app.theme();
    if let Some(bg) = theme.bg {
        f.render_widget(
            ratatui::widgets::Block::default().style(Style::default().bg(bg.color())),
            area,
        );
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(11),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(f, rows[0], app);
    render_table(f, rows[1], app);
    render_detail(f, rows[2], app);
    render_footer(f, rows[3], theme);
}

fn render_header(f: &mut Frame, area: Rect, app: &FleetApp) {
    let theme = app.theme();
    let (online, tps, vu, vt, gpus) = app.totals();
    let mut spans = vec![
        Span::styled(
            "▟▛ toptop fleet",
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} hosts · {} online", app.hosts.len(), online),
            dim(theme),
        ),
    ];
    if gpus > 0 {
        spans.push(Span::styled(
            format!(
                "  · {} GPUs · Σ vram {}/{}",
                gpus,
                short_bytes(vu),
                short_bytes(vt)
            ),
            dim(theme),
        ));
    }
    if tps > 0.0 {
        spans.push(Span::styled(
            format!("  · Σ {:.0} tok/s", tps),
            Style::default()
                .fg(theme.grad(0.0))
                .add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let clock = chrono::Local::now().format("%H:%M:%S").to_string();
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            clock,
            Style::default()
                .fg(theme.accent2.color())
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Right),
        area,
    );
}

fn status_span(theme: &Theme, st: &HostStatus) -> Span<'static> {
    match st {
        HostStatus::Online => Span::styled("● online", Style::default().fg(theme.grad(0.0))),
        HostStatus::Connecting => Span::styled("◌ connect…", Style::default().fg(theme.grad(0.5))),
        HostStatus::Error(e) => Span::styled(
            format!("✕ {}", crate::util::truncate(e, 14)),
            Style::default().fg(theme.grad(1.0)),
        ),
    }
}

fn render_table(f: &mut Frame, area: Rect, app: &FleetApp) {
    let theme = app.theme();
    let block = panel("hosts", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 1 {
        return;
    }

    let header = Row::new(
        [
            "HOST", "STATUS", "CPU%", "MEM", "LOAD", "GPU%", "VRAM", "TOK/S", "TASKS", "UP",
        ]
        .map(Cell::from),
    )
    .style(
        Style::default()
            .fg(theme.accent.color())
            .add_modifier(Modifier::BOLD),
    );

    let pct = |v: f64| Style::default().fg(theme.grad((v / 100.0) as f32));
    let mut rows: Vec<Row> = Vec::new();
    for (i, h) in app.hosts.iter().enumerate() {
        let sel = i == app.selected;
        let s = h.summary.as_ref();
        let cell_num = |v: Option<f64>, suffix: &str| match v {
            Some(v) => Cell::from(Span::styled(format!("{:.0}{}", v, suffix), pct(v))),
            None => Cell::from(Span::styled("—", dim(theme))),
        };
        let row = Row::new(vec![
            Cell::from(crate::util::truncate(&h.name, 16)),
            Cell::from(status_span(theme, &h.status)),
            cell_num(s.map(|s| s.cpu_usage), ""),
            cell_num(s.map(|s| s.mem_pct()), ""),
            Cell::from(match s {
                Some(s) => format!("{:.1}", s.load.0),
                None => "—".into(),
            }),
            cell_num(s.and_then(|s| s.gpu_util), ""),
            Cell::from(match s {
                Some(s) if s.vram_total > 0 => Span::styled(short_bytes(s.vram_used), dim(theme)),
                _ => Span::styled("—", dim(theme)),
            }),
            Cell::from(match s {
                Some(s) if s.total_tps > 0.0 => Span::styled(
                    format!("{:.0}", s.total_tps),
                    Style::default().fg(theme.grad(0.0)),
                ),
                _ => Span::styled("—", dim(theme)),
            }),
            Cell::from(match s {
                Some(s) => s.tasks.to_string(),
                None => "—".into(),
            }),
            Cell::from(match s {
                Some(s) => human_duration(s.uptime),
                None => "—".into(),
            }),
        ]);
        rows.push(if sel {
            row.style(
                Style::default()
                    .bg(theme.selection.color())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            row.style(Style::default().fg(theme.fg.color()))
        });
    }

    let widths = [
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Min(8),
    ];
    f.render_widget(
        Table::new(rows, widths).header(header).column_spacing(1),
        inner,
    );
}

fn render_detail(f: &mut Frame, area: Rect, app: &FleetApp) {
    let theme = app.theme();
    let title = app
        .selected_host()
        .map(|h| format!("host · {}", h.name))
        .unwrap_or_else(|| "host".into());
    let block = panel(&title, theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height < 1 {
        return;
    }

    let Some(h) = app.selected_host() else {
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    if let HostStatus::Error(e) = &h.status {
        lines.push(Line::from(Span::styled(
            format!("✕ {}", e),
            Style::default().fg(theme.grad(1.0)),
        )));
        lines.push(Line::from(Span::styled(
            "retrying every 3s · check SSH access and that toptop is on the remote PATH",
            dim(theme),
        )));
    }
    if let Some(s) = &h.summary {
        let bw = (inner.width as usize).saturating_sub(26).clamp(6, 30);
        lines.push(Line::from(vec![
            Span::styled(s.os.clone(), Style::default().fg(theme.accent2.color())),
            Span::styled(
                format!(
                    "   load {:.2} {:.2} {:.2}   up {}   {} tasks ({} run)",
                    s.load.0,
                    s.load.1,
                    s.load.2,
                    human_duration(s.uptime),
                    s.tasks,
                    s.running
                ),
                dim(theme),
            ),
            match &h.latency_ms {
                Some(ms) => Span::styled(format!("   {}ms", ms), dim(theme)),
                None => Span::raw(""),
            },
        ]));
        let mut meter = |label: &str, p: f64, extra: String| {
            let mut sp = vec![Span::styled(format!("{:<6}", label), dim(theme))];
            sp.extend(graph::meter_spans(p as f32, bw, theme));
            sp.push(Span::styled(
                format!(" {}", extra),
                Style::default().fg(theme.grad((p / 100.0) as f32)),
            ));
            lines.push(Line::from(sp));
        };
        meter("cpu", s.cpu_usage, format!("{:.0}%", s.cpu_usage));
        meter(
            "mem",
            s.mem_pct(),
            format!("{} / {}", human_bytes(s.mem_used), human_bytes(s.mem_total)),
        );
        if s.vram_total > 0 {
            meter(
                "vram",
                s.vram_pct(),
                format!(
                    "{} / {}  ·  {} GPU  ·  {:.0} W",
                    human_bytes(s.vram_used),
                    human_bytes(s.vram_total),
                    s.gpu_count,
                    s.gpu_power
                ),
            );
        }
        for sv in s.servers.iter().take(3) {
            let mut sp = vec![Span::styled(
                format!("  {} ", sv.runtime),
                Style::default().fg(theme.accent2.color()),
            )];
            if !sv.model.is_empty() {
                sp.push(Span::styled(
                    format!("{} ", crate::util::truncate(&sv.model, 28)),
                    dim(theme),
                ));
            }
            if let Some(t) = sv.gen_tps {
                sp.push(Span::styled(
                    format!(" {:.1} tok/s", t),
                    Style::default()
                        .fg(theme.grad(0.0))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if let Some(k) = sv.kv_pct {
                sp.push(Span::styled(format!("  kv {:.0}%", k), dim(theme)));
            }
            lines.push(Line::from(sp));
        }
    }
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_footer(f: &mut Frame, area: Rect, theme: &Theme) {
    let mut spans = Vec::new();
    for (k, d) in [("↑/↓", "select"), ("p", "theme"), ("q", "quit")] {
        spans.push(Span::styled(
            k,
            Style::default()
                .fg(theme.accent.color())
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(":{} ", d), dim(theme)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
