//! Themes and the truecolor gradient engine.
//!
//! Every theme exposes a small set of semantic colors plus a *gradient* — an
//! ordered list of color stops that meters and graphs interpolate across based
//! on load (typically green → yellow → red). Colors are emitted as true RGB so
//! the gradients are smooth on any 24-bit-capable terminal.

use ratatui::style::Color;

/// An RGB triple used for interpolation before being handed to ratatui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn color(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }

    /// Linearly interpolate between two colors. `t` is clamped to `0.0..=1.0`.
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * t).round() as u8 };
        Rgb(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2))
    }
}

/// A complete color scheme.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
    /// Default foreground / body text.
    pub fg: Rgb,
    /// Panel background (used sparingly; mostly transparent).
    pub bg: Option<Rgb>,
    /// Dimmed text (labels, units, inactive items).
    pub dim: Rgb,
    /// Accent used for titles and the logo.
    pub accent: Rgb,
    /// Secondary accent (selection, highlights).
    pub accent2: Rgb,
    /// Border color for unfocused panels.
    pub border: Rgb,
    /// Border color for the focused panel.
    pub border_focus: Rgb,
    /// Color for the memory meters/graphs.
    pub mem: Rgb,
    /// Color for swap.
    pub swap: Rgb,
    /// Network download color.
    pub net_down: Rgb,
    /// Network upload color.
    pub net_up: Rgb,
    /// Disk read color.
    pub disk_read: Rgb,
    /// Disk write color.
    pub disk_write: Rgb,
    /// Selected-row background.
    pub selection: Rgb,
    /// Gradient stops for load-based coloring (low → high).
    pub gradient: &'static [Rgb],
}

impl Theme {
    /// Sample the load gradient at `t` (0.0 = idle, 1.0 = saturated).
    pub fn grad(&self, t: f32) -> Color {
        self.grad_rgb(t).color()
    }

    /// Sample the load gradient and return the raw RGB (for further blending).
    pub fn grad_rgb(&self, t: f32) -> Rgb {
        let stops = self.gradient;
        if stops.is_empty() {
            return self.fg;
        }
        if stops.len() == 1 {
            return stops[0];
        }
        let t = t.clamp(0.0, 1.0);
        let scaled = t * (stops.len() - 1) as f32;
        let idx = scaled.floor() as usize;
        if idx >= stops.len() - 1 {
            return stops[stops.len() - 1];
        }
        let frac = scaled - idx as f32;
        stops[idx].lerp(stops[idx + 1], frac)
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb(r, g, b)
}

// ── Gradient palettes ────────────────────────────────────────────────────────
static GRAD_GRUVBOX: [Rgb; 4] = [
    rgb(184, 187, 38),  // green
    rgb(250, 189, 47),  // yellow
    rgb(254, 128, 25),  // orange
    rgb(251, 73, 52),   // red
];
static GRAD_NORD: [Rgb; 4] = [
    rgb(163, 190, 140), // aurora green
    rgb(235, 203, 139), // yellow
    rgb(208, 135, 112), // orange
    rgb(191, 97, 106),  // red
];
static GRAD_DRACULA: [Rgb; 4] = [
    rgb(80, 250, 123),  // green
    rgb(241, 250, 140), // yellow
    rgb(255, 184, 108), // orange
    rgb(255, 85, 85),   // red
];
static GRAD_TOKYO: [Rgb; 4] = [
    rgb(158, 206, 106), // green
    rgb(224, 175, 104), // yellow
    rgb(255, 158, 100), // orange
    rgb(247, 118, 142), // red
];
static GRAD_MATRIX: [Rgb; 4] = [
    rgb(0, 80, 0),
    rgb(0, 160, 0),
    rgb(57, 255, 20),
    rgb(180, 255, 120),
];

/// All themes, in cycle order.
pub static THEMES: &[Theme] = &[
    Theme {
        name: "gruvbox",
        fg: rgb(235, 219, 178),
        bg: None,
        dim: rgb(146, 131, 116),
        accent: rgb(254, 128, 25),
        accent2: rgb(131, 165, 152),
        border: rgb(80, 73, 69),
        border_focus: rgb(254, 128, 25),
        mem: rgb(131, 165, 152),
        swap: rgb(211, 134, 155),
        net_down: rgb(142, 192, 124),
        net_up: rgb(250, 189, 47),
        disk_read: rgb(131, 165, 152),
        disk_write: rgb(211, 134, 155),
        selection: rgb(60, 56, 54),
        gradient: &GRAD_GRUVBOX,
    },
    Theme {
        name: "nord",
        fg: rgb(216, 222, 233),
        bg: None,
        dim: rgb(118, 128, 146),
        accent: rgb(136, 192, 208),
        accent2: rgb(143, 188, 187),
        border: rgb(59, 66, 82),
        border_focus: rgb(136, 192, 208),
        mem: rgb(129, 161, 193),
        swap: rgb(180, 142, 173),
        net_down: rgb(163, 190, 140),
        net_up: rgb(235, 203, 139),
        disk_read: rgb(136, 192, 208),
        disk_write: rgb(180, 142, 173),
        selection: rgb(59, 66, 82),
        gradient: &GRAD_NORD,
    },
    Theme {
        name: "dracula",
        fg: rgb(248, 248, 242),
        bg: None,
        dim: rgb(98, 114, 164),
        accent: rgb(189, 147, 249),
        accent2: rgb(139, 233, 253),
        border: rgb(68, 71, 90),
        border_focus: rgb(189, 147, 249),
        mem: rgb(139, 233, 253),
        swap: rgb(255, 121, 198),
        net_down: rgb(80, 250, 123),
        net_up: rgb(241, 250, 140),
        disk_read: rgb(139, 233, 253),
        disk_write: rgb(255, 121, 198),
        selection: rgb(68, 71, 90),
        gradient: &GRAD_DRACULA,
    },
    Theme {
        name: "tokyonight",
        fg: rgb(192, 202, 245),
        bg: None,
        dim: rgb(86, 95, 137),
        accent: rgb(122, 162, 247),
        accent2: rgb(125, 207, 255),
        border: rgb(41, 46, 66),
        border_focus: rgb(122, 162, 247),
        mem: rgb(125, 207, 255),
        swap: rgb(187, 154, 247),
        net_down: rgb(158, 206, 106),
        net_up: rgb(224, 175, 104),
        disk_read: rgb(125, 207, 255),
        disk_write: rgb(187, 154, 247),
        selection: rgb(41, 46, 66),
        gradient: &GRAD_TOKYO,
    },
    Theme {
        name: "matrix",
        fg: rgb(0, 200, 0),
        bg: Some(rgb(0, 0, 0)),
        dim: rgb(0, 90, 0),
        accent: rgb(57, 255, 20),
        accent2: rgb(120, 255, 120),
        border: rgb(0, 70, 0),
        border_focus: rgb(57, 255, 20),
        mem: rgb(0, 200, 0),
        swap: rgb(0, 140, 0),
        net_down: rgb(57, 255, 20),
        net_up: rgb(0, 160, 0),
        disk_read: rgb(57, 255, 20),
        disk_write: rgb(0, 160, 0),
        selection: rgb(0, 50, 0),
        gradient: &GRAD_MATRIX,
    },
];

/// Look up a theme index by name (case-insensitive). Returns `None` if unknown.
pub fn index_by_name(name: &str) -> Option<usize> {
    THEMES
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_endpoints() {
        let a = Rgb(0, 0, 0);
        let b = Rgb(100, 200, 50);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Rgb(50, 100, 25));
    }

    #[test]
    fn gradient_bounds() {
        let theme = &THEMES[0];
        assert_eq!(theme.grad_rgb(0.0), theme.gradient[0]);
        assert_eq!(
            theme.grad_rgb(1.0),
            theme.gradient[theme.gradient.len() - 1]
        );
        // Mid value lands between two stops, not outside the palette.
        let mid = theme.grad_rgb(0.5);
        assert!(mid.0 > 0 || mid.1 > 0 || mid.2 > 0);
    }

    #[test]
    fn all_themes_resolve() {
        for t in THEMES {
            assert_eq!(index_by_name(t.name), Some(THEMES.iter().position(|x| x.name == t.name).unwrap()));
        }
        assert_eq!(index_by_name("nope"), None);
    }
}
