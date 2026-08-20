//! High-resolution braille graphs and gradient meter bars.
//!
//! Braille cells pack a 2×4 dot matrix into a single character, giving graphs
//! four times the vertical and twice the horizontal resolution of block chars.
//! Filled areas are colored with a vertical gradient (calm at the base, hot at
//! the peaks) so load is readable at a glance.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Braille dot bit values, indexed `[column][row]` where column 0 is the left
/// sub-column and rows run top→bottom. Base code point is U+2800.
const DOTS: [[u8; 4]; 2] = [
    [0x01, 0x02, 0x04, 0x40], // left column: dots 1,2,3,7
    [0x08, 0x10, 0x20, 0x80], // right column: dots 4,5,6,8
];

/// Eighth-blocks for sub-cell precision in meter bars.
const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

fn braille_char(bits: u8) -> char {
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

/// Map a series value to a fill height in dots (0..=rows), clamped and NaN-safe.
fn fill_dots(value: f64, max: f64, rows: usize) -> usize {
    if max <= 0.0 || value <= 0.0 || !value.is_finite() {
        return 0;
    }
    ((value / max) * rows as f64)
        .round()
        .clamp(0.0, rows as f64) as usize
}

/// Pick the `width*2` most recent samples, padding the left with zeros so the
/// graph is right-aligned (newest data on the right edge) and fully sized.
fn columns(series: &[f64], dot_cols: usize) -> Vec<f64> {
    let mut out = vec![0.0; dot_cols];
    let take = series.len().min(dot_cols);
    let src = &series[series.len() - take..];
    for (i, v) in src.iter().enumerate() {
        out[dot_cols - take + i] = *v;
    }
    out
}

/// Render a single series as a braille area graph with a vertical load gradient.
///
/// Returns exactly `height` styled lines, each `width` cells wide.
pub fn braille_graph(
    series: &[f64],
    max: f64,
    width: usize,
    height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let dot_cols = width * 2;
    let dot_rows = height * 4;
    let max = max.max(f64::MIN_POSITIVE);
    let cols = columns(series, dot_cols);
    let heights: Vec<usize> = cols.iter().map(|v| fill_dots(*v, max, dot_rows)).collect();

    let mut lines = Vec::with_capacity(height);
    for cy in 0..height {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(width);
        // Vertical gradient: top rows are "hot", bottom rows are "calm".
        let center_row = cy * 4 + 2;
        let t = 1.0 - (center_row as f32 / dot_rows as f32);
        let color = theme.grad(t);
        for cx in 0..width {
            let mut bits = 0u8;
            for (sub, dotcol) in DOTS.iter().enumerate() {
                let col_idx = cx * 2 + sub;
                let h = heights[col_idx];
                for (ry, bit) in dotcol.iter().enumerate() {
                    let abs_row = cy * 4 + ry;
                    if abs_row >= dot_rows - h {
                        bits |= bit;
                    }
                }
            }
            if bits == 0 {
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled(
                    braille_char(bits).to_string(),
                    Style::default().fg(color),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Render two series in one area: `up` grows from the vertical center upward in
/// `up_color`, `down` grows downward in `down_color`. Ideal for symmetrical
/// pairs like network rx/tx or disk read/write.
pub fn mirror_graph(
    up: &[f64],
    down: &[f64],
    max: f64,
    width: usize,
    height: usize,
    up_color: ratatui::style::Color,
    down_color: ratatui::style::Color,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let dot_cols = width * 2;
    let half_rows = (height * 4) / 2;
    let max = max.max(f64::MIN_POSITIVE);
    let up_cols = columns(up, dot_cols);
    let down_cols = columns(down, dot_cols);
    let up_h: Vec<usize> = up_cols
        .iter()
        .map(|v| fill_dots(*v, max, half_rows))
        .collect();
    let down_h: Vec<usize> = down_cols
        .iter()
        .map(|v| fill_dots(*v, max, half_rows))
        .collect();

    let mid_cell = height / 2;
    let mut lines = Vec::with_capacity(height);
    for cy in 0..height {
        let upper = cy < mid_cell;
        let color = if upper { up_color } else { down_color };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(width);
        for cx in 0..width {
            let mut bits = 0u8;
            for (sub, dotcol) in DOTS.iter().enumerate() {
                let col_idx = cx * 2 + sub;
                for (ry, bit) in dotcol.iter().enumerate() {
                    let on = if upper {
                        // Distance in dot-rows above the midline (1 = just above).
                        let from_mid = (mid_cell - cy) * 4 - ry;
                        from_mid >= 1 && from_mid <= up_h[col_idx]
                    } else {
                        let from_mid = (cy - mid_cell) * 4 + ry + 1;
                        from_mid >= 1 && from_mid <= down_h[col_idx]
                    };
                    if on {
                        bits |= bit;
                    }
                }
            }
            if bits == 0 {
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled(
                    braille_char(bits).to_string(),
                    Style::default().fg(color),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Build a horizontal gradient meter bar of the given width for a 0..=100 value.
///
/// Filled cells are colored along the theme gradient (calm → hot, left → right);
/// the boundary cell uses an eighth-block for smooth sub-cell precision.
pub fn meter_spans(pct: f32, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let pct = pct.clamp(0.0, 100.0);
    let exact = (pct / 100.0) * width as f32;
    let full = exact.floor() as usize;
    let rem = exact - full as f32;
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        let t = if width <= 1 {
            1.0
        } else {
            i as f32 / (width - 1) as f32
        };
        if i < full {
            spans.push(Span::styled("█", Style::default().fg(theme.grad(t))));
        } else if i == full && rem > 0.05 {
            let idx = (rem * 8.0).round().clamp(1.0, 8.0) as usize;
            spans.push(Span::styled(
                EIGHTHS[idx].to_string(),
                Style::default().fg(theme.grad(t)),
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::BUILTINS as THEMES;

    #[test]
    fn graph_dimensions() {
        let series: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let lines = braille_graph(&series, 50.0, 20, 5, &THEMES[0]);
        assert_eq!(lines.len(), 5);
        for line in &lines {
            assert_eq!(line.width(), 20);
        }
    }

    #[test]
    fn graph_zero_size() {
        assert!(braille_graph(&[1.0], 1.0, 0, 5, &THEMES[0]).is_empty());
        assert!(braille_graph(&[1.0], 1.0, 5, 0, &THEMES[0]).is_empty());
    }

    #[test]
    fn fill_is_clamped() {
        assert_eq!(fill_dots(200.0, 100.0, 8), 8);
        assert_eq!(fill_dots(-5.0, 100.0, 8), 0);
        assert_eq!(fill_dots(f64::NAN, 100.0, 8), 0);
        assert_eq!(fill_dots(50.0, 100.0, 8), 4);
    }

    #[test]
    fn meter_widths() {
        let spans = meter_spans(50.0, 10, &THEMES[0]);
        assert_eq!(spans.len(), 10);
        let spans = meter_spans(150.0, 4, &THEMES[0]);
        assert_eq!(spans.len(), 4);
        assert!(meter_spans(50.0, 0, &THEMES[0]).is_empty());
    }

    #[test]
    fn mirror_dimensions() {
        let a: Vec<f64> = (0..40).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..40).map(|i| (40 - i) as f64).collect();
        let lines = mirror_graph(
            &a,
            &b,
            40.0,
            15,
            6,
            THEMES[0].net_down.color(),
            THEMES[0].net_up.color(),
        );
        assert_eq!(lines.len(), 6);
        for l in &lines {
            assert_eq!(l.width(), 15);
        }
    }

    #[test]
    fn columns_right_aligned() {
        let s = vec![1.0, 2.0, 3.0];
        let c = columns(&s, 6);
        assert_eq!(c, vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    }
}
