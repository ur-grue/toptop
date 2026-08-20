//! Themes and the truecolor gradient engine.
//!
//! Every theme exposes a small set of semantic colors plus a *gradient* — an
//! ordered list of color stops that meters and graphs interpolate across based
//! on load (typically green → yellow → red). Colors are emitted as true RGB so
//! the gradients are smooth on any 24-bit-capable terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    rgb(184, 187, 38), // green
    rgb(250, 189, 47), // yellow
    rgb(254, 128, 25), // orange
    rgb(251, 73, 52),  // red
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
// Cyber Pixel-inspired: electric cyan → purple → neon magenta → hot pink.
static GRAD_CYBER: [Rgb; 4] = [
    rgb(0, 240, 255),  // electric cyan
    rgb(123, 97, 255), // electric purple
    rgb(255, 46, 151), // neon magenta
    rgb(255, 41, 90),  // hot pink-red
];
// Saturated-but-dark stops that stay readable on white backgrounds.
static GRAD_PAPER: [Rgb; 4] = [
    rgb(46, 125, 50), // green 800
    rgb(214, 129, 0), // amber 800
    rgb(230, 81, 0),  // orange 900
    rgb(183, 28, 28), // red 900
];

/// The compile-time themes, in cycle order. User themes loaded from
/// `themes/*.conf` are appended to these by [`init_user_themes`].
pub static BUILTINS: &[Theme] = &[
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
    Theme {
        name: "cyberpunk",
        fg: rgb(220, 235, 255),
        bg: Some(rgb(13, 6, 26)), // deep near-black violet
        dim: rgb(110, 92, 160),
        accent: rgb(255, 46, 151), // neon magenta
        accent2: rgb(0, 240, 255), // electric cyan
        border: rgb(58, 42, 93),
        border_focus: rgb(255, 46, 151),
        mem: rgb(0, 240, 255),
        swap: rgb(178, 102, 255),
        net_down: rgb(10, 255, 157), // neon green
        net_up: rgb(249, 240, 2),    // neon yellow
        disk_read: rgb(0, 240, 255),
        disk_write: rgb(255, 46, 151),
        selection: rgb(36, 23, 52),
        gradient: &GRAD_CYBER,
    },
    // For light terminal backgrounds — dark text, saturated-dark accents.
    Theme {
        name: "paper",
        fg: rgb(40, 42, 54),
        bg: None, // keep the terminal's light background
        dim: rgb(125, 128, 140),
        accent: rgb(156, 39, 143), // deep magenta
        accent2: rgb(2, 105, 160), // deep teal-blue
        border: rgb(175, 178, 190),
        border_focus: rgb(156, 39, 143),
        mem: rgb(2, 105, 160),
        swap: rgb(123, 60, 172),
        net_down: rgb(46, 125, 50),
        net_up: rgb(214, 129, 0),
        disk_read: rgb(2, 105, 160),
        disk_write: rgb(173, 38, 100),
        selection: rgb(222, 224, 233),
        gradient: &GRAD_PAPER,
    },
];

// ── Theme registry (built-ins + user themes) ─────────────────────────────────

/// The active theme list: built-ins first, then any user themes loaded from
/// disk. Populated once by [`init_user_themes`]; falls back to the built-ins
/// when nothing was loaded (tests, `--export`, library use).
static REGISTRY: OnceLock<Vec<Theme>> = OnceLock::new();

/// All themes available at runtime, in cycle order.
pub fn themes() -> &'static [Theme] {
    REGISTRY.get_or_init(|| BUILTINS.to_vec())
}

/// Look up a theme index by name (case-insensitive). Returns `None` if unknown.
pub fn index_by_name(name: &str) -> Option<usize> {
    themes()
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))
}

/// The user theme directory, honoring `XDG_CONFIG_HOME`.
pub fn user_theme_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("toptop").join("themes"))
}

/// Load `*.conf` theme files from `dir` and append them to the registry.
///
/// Call once, before the config file is read (it resolves `theme = <name>`).
/// Returns one warning line per file that could not be parsed; a broken file is
/// skipped, never fatal. Later calls are no-ops.
pub fn init_user_themes(dir: Option<&Path>) -> Vec<String> {
    let mut list = BUILTINS.to_vec();
    let mut warnings = Vec::new();
    if let Some(dir) = dir {
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "conf"))
            .collect();
        // Directory order is unspecified; sort so the cycle order is stable.
        files.sort();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(contents) = fs::read_to_string(&path) else {
                warnings.push(format!("theme {}: unreadable, ignored", path.display()));
                continue;
            };
            match parse_theme(stem, &contents) {
                Ok(theme) => {
                    // A user theme may not shadow a built-in name, or `--theme`
                    // would become ambiguous.
                    if list.iter().any(|t| t.name.eq_ignore_ascii_case(theme.name)) {
                        warnings.push(format!("theme '{stem}': name already taken, ignored"));
                    } else {
                        list.push(theme);
                    }
                }
                Err(e) => warnings.push(format!("theme '{stem}': {e}, ignored")),
            }
        }
    }
    let _ = REGISTRY.set(list);
    warnings
}

/// Build a theme from `key = #rrggbb` lines.
///
/// `base = <built-in>` picks the theme every unset key falls back to (default
/// `gruvbox`), so a file only has to state what it changes. `gradient` takes a
/// comma-separated list of stops.
pub fn parse_theme(name: &str, contents: &str) -> Result<Theme, String> {
    let mut base_name = "gruvbox".to_string();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim().to_string());
        if key == "base" {
            base_name = value;
        } else {
            pairs.push((key, value));
        }
    }

    let base = BUILTINS
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(&base_name))
        .ok_or_else(|| format!("unknown base theme '{base_name}'"))?;

    let mut theme = base.clone();
    theme.name = Box::leak(name.to_ascii_lowercase().into_boxed_str());

    for (key, value) in pairs {
        match key.as_str() {
            "gradient" => {
                let stops = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(parse_hex)
                    .collect::<Result<Vec<Rgb>, String>>()?;
                if stops.is_empty() {
                    return Err("gradient needs at least one color".to_string());
                }
                theme.gradient = Box::leak(stops.into_boxed_slice());
            }
            // `bg = none` keeps the terminal's own background.
            "bg" if value.eq_ignore_ascii_case("none") => theme.bg = None,
            "bg" => theme.bg = Some(parse_hex(&value)?),
            _ => {
                let slot = match key.as_str() {
                    "fg" => &mut theme.fg,
                    "dim" => &mut theme.dim,
                    "accent" => &mut theme.accent,
                    "accent2" => &mut theme.accent2,
                    "border" => &mut theme.border,
                    "border_focus" => &mut theme.border_focus,
                    "mem" => &mut theme.mem,
                    "swap" => &mut theme.swap,
                    "net_down" => &mut theme.net_down,
                    "net_up" => &mut theme.net_up,
                    "disk_read" => &mut theme.disk_read,
                    "disk_write" => &mut theme.disk_write,
                    "selection" => &mut theme.selection,
                    // Unknown keys keep the base value rather than failing the
                    // whole file — forward compatible with newer key sets.
                    _ => continue,
                };
                *slot = parse_hex(&value)?;
            }
        }
    }
    Ok(theme)
}

/// Parse `#rrggbb` (the leading `#` optional) into an [`Rgb`].
fn parse_hex(value: &str) -> Result<Rgb, String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("'{value}' is not a #rrggbb color"));
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    Ok(Rgb(byte(0), byte(2), byte(4)))
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
        let theme = &themes()[0];
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
    fn hex_parsing() {
        assert_eq!(parse_hex("#ff8000"), Ok(Rgb(255, 128, 0)));
        assert_eq!(parse_hex("FF8000"), Ok(Rgb(255, 128, 0)));
        assert!(parse_hex("#fff").is_err());
        assert!(parse_hex("#gggggg").is_err());
        assert!(parse_hex("").is_err());
    }

    #[test]
    fn user_theme_overrides_base_and_keeps_the_rest() {
        let t = parse_theme(
            "Midnight",
            "# my theme\nbase = nord\naccent = #ff0000\nbg = none\nunknown_key = #123456\n",
        )
        .expect("valid theme");
        assert_eq!(t.name, "midnight");
        assert_eq!(t.accent, Rgb(255, 0, 0));
        assert_eq!(t.bg, None);
        // Untouched keys keep the base theme's values.
        let nord = BUILTINS.iter().find(|b| b.name == "nord").unwrap();
        assert_eq!(t.fg, nord.fg);
        assert_eq!(t.gradient, nord.gradient);
    }

    #[test]
    fn user_theme_gradient_and_failures() {
        let t = parse_theme("g", "gradient = #000000, #ffffff\n").unwrap();
        assert_eq!(t.gradient, &[Rgb(0, 0, 0), Rgb(255, 255, 255)]);

        assert!(parse_theme("b", "base = nope\n").is_err());
        assert!(parse_theme("b", "accent = purple\n").is_err());
        assert!(parse_theme("b", "gradient =\n").is_err());
    }

    #[test]
    fn broken_theme_files_are_skipped_with_a_warning() {
        let dir = std::env::temp_dir().join(format!("toptop-themes-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.conf"), "accent = not-a-color\n").unwrap();
        fs::write(dir.join("gruvbox.conf"), "accent = #ffffff\n").unwrap();
        fs::write(dir.join("notes.txt"), "ignored\n").unwrap();
        let warnings = init_user_themes(Some(&dir));
        fs::remove_dir_all(&dir).ok();

        assert!(warnings.iter().any(|w| w.contains("broken")));
        assert!(warnings.iter().any(|w| w.contains("name already taken")));
        // A built-in must not be replaced by a same-named user file.
        assert_eq!(index_by_name("gruvbox"), Some(0));
    }

    #[test]
    fn all_themes_resolve() {
        for t in themes() {
            assert_eq!(
                index_by_name(t.name),
                Some(themes().iter().position(|x| x.name == t.name).unwrap())
            );
        }
        assert_eq!(index_by_name("nope"), None);
    }
}
