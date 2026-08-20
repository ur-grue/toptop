//! Rebindable key actions.
//!
//! Every main-view action is looked up through a [`KeyMap`] instead of matching
//! a hardcoded [`KeyCode`], so users can remap keys from the config file with
//! `bind_<action> = <key>[, <key>...]` lines. Modal input (the filter prompt,
//! the signal/renice menus, `Esc`, `Ctrl-C`) stays fixed — remapping the keys
//! that dismiss a prompt would be a way to lock yourself in.

use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A rebindable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Pause,
    Detail,
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
    SortNext,
    SortInvert,
    Tree,
    PerCore,
    Layout,
    Connections,
    Ai,
    ThemeNext,
    ThemePrev,
    TickUp,
    TickDown,
    Filter,
    Kill,
    Renice,
    SigTerm,
    SigKill,
}

impl Action {
    /// Config name, used as the `bind_<name>` suffix.
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::Help => "help",
            Action::Pause => "pause",
            Action::Detail => "detail",
            Action::Up => "up",
            Action::Down => "down",
            Action::PageUp => "page_up",
            Action::PageDown => "page_down",
            Action::First => "first",
            Action::Last => "last",
            Action::SortNext => "sort_next",
            Action::SortInvert => "sort_invert",
            Action::Tree => "tree",
            Action::PerCore => "per_core",
            Action::Layout => "layout",
            Action::Connections => "connections",
            Action::Ai => "ai",
            Action::ThemeNext => "theme_next",
            Action::ThemePrev => "theme_prev",
            Action::TickUp => "tick_up",
            Action::TickDown => "tick_down",
            Action::Filter => "filter",
            Action::Kill => "kill",
            Action::Renice => "renice",
            Action::SigTerm => "sigterm",
            Action::SigKill => "sigkill",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        ALL.iter()
            .copied()
            .find(|a| a.name() == name.trim().to_ascii_lowercase())
    }
}

/// Every action, for lookup and for the `bind_*` config keys.
pub const ALL: &[Action] = &[
    Action::Quit,
    Action::Help,
    Action::Pause,
    Action::Detail,
    Action::Up,
    Action::Down,
    Action::PageUp,
    Action::PageDown,
    Action::First,
    Action::Last,
    Action::SortNext,
    Action::SortInvert,
    Action::Tree,
    Action::PerCore,
    Action::Layout,
    Action::Connections,
    Action::Ai,
    Action::ThemeNext,
    Action::ThemePrev,
    Action::TickUp,
    Action::TickDown,
    Action::Filter,
    Action::Kill,
    Action::Renice,
    Action::SigTerm,
    Action::SigKill,
];

/// A key plus the modifiers that matter for binding (only Ctrl and Alt —
/// Shift is already carried by the character itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyPress {
    pub fn from_event(ev: KeyEvent) -> Self {
        Self {
            code: ev.code,
            ctrl: ev.modifiers.contains(KeyModifiers::CONTROL),
            alt: ev.modifiers.contains(KeyModifiers::ALT),
        }
    }

    /// Parse a spec like `q`, `space`, `pgdn`, `f9` or `ctrl+r`.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }
        let (mut ctrl, mut alt) = (false, false);
        let mut rest = spec;
        loop {
            let lower = rest.to_ascii_lowercase();
            if let Some(r) = lower.strip_prefix("ctrl+").or(lower.strip_prefix("c-")) {
                ctrl = true;
                rest = &rest[rest.len() - r.len()..];
            } else if let Some(r) = lower.strip_prefix("alt+").or(lower.strip_prefix("m-")) {
                alt = true;
                rest = &rest[rest.len() - r.len()..];
            } else {
                break;
            }
        }
        // Single characters stay case-sensitive: `P` is a distinct binding
        // from `p`. Everything else is a case-insensitive key name.
        let code = if rest.chars().count() == 1 {
            KeyCode::Char(rest.chars().next().unwrap())
        } else {
            match rest.to_ascii_lowercase().as_str() {
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "pgup" | "pageup" => KeyCode::PageUp,
                "pgdn" | "pagedown" => KeyCode::PageDown,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                "enter" | "return" => KeyCode::Enter,
                "tab" => KeyCode::Tab,
                "space" => KeyCode::Char(' '),
                "backspace" => KeyCode::Backspace,
                "delete" | "del" => KeyCode::Delete,
                "insert" | "ins" => KeyCode::Insert,
                "esc" | "escape" => KeyCode::Esc,
                other => {
                    let n = other.strip_prefix('f')?.parse::<u8>().ok()?;
                    if (1..=12).contains(&n) {
                        KeyCode::F(n)
                    } else {
                        return None;
                    }
                }
            }
        };
        Some(KeyPress { code, ctrl, alt })
    }
}

/// Key → action lookup table.
#[derive(Clone, Debug)]
pub struct KeyMap {
    map: HashMap<KeyPress, Action>,
}

/// The built-in bindings, as config specs — also the documentation of what a
/// `bind_*` line replaces.
pub const DEFAULT_BINDINGS: &[(Action, &str)] = &[
    (Action::Quit, "q"),
    (Action::Help, "?, f1"),
    (Action::Pause, "space"),
    (Action::Detail, "enter"),
    (Action::Up, "up, k"),
    (Action::Down, "down, j"),
    (Action::PageUp, "pgup"),
    (Action::PageDown, "pgdn"),
    (Action::First, "home, g"),
    (Action::Last, "end, G"),
    (Action::SortNext, "s"),
    (Action::SortInvert, "i"),
    (Action::Tree, "t"),
    (Action::PerCore, "e"),
    (Action::Layout, "L"),
    (Action::Connections, "n"),
    (Action::Ai, "a"),
    (Action::ThemeNext, "p"),
    (Action::ThemePrev, "P"),
    (Action::TickUp, "+, ="),
    (Action::TickDown, "-, _"),
    (Action::Filter, "/"),
    (Action::Kill, "K, f9"),
    (Action::Renice, "r"),
    (Action::SigTerm, "delete"),
    (Action::SigKill, "x"),
];

impl Default for KeyMap {
    fn default() -> Self {
        let mut km = KeyMap {
            map: HashMap::new(),
        };
        for (action, spec) in DEFAULT_BINDINGS {
            km.bind(*action, spec);
        }
        km
    }
}

impl KeyMap {
    /// Resolve a key event to an action.
    pub fn action(&self, ev: KeyEvent) -> Option<Action> {
        self.map.get(&KeyPress::from_event(ev)).copied()
    }

    /// Rebind `action` to a comma-separated key list, replacing whatever it was
    /// bound to before.
    ///
    /// Returns one warning per key spec that could not be parsed. A key already
    /// used by another action is re-pointed at `action` (last binding wins), so
    /// swapping two keys never leaves both stuck on the same action.
    pub fn bind(&mut self, action: Action, spec: &str) -> Vec<String> {
        let keys: Vec<KeyPress> = spec.split(',').filter_map(KeyPress::parse).collect();
        let mut warnings: Vec<String> = spec
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && KeyPress::parse(s).is_none())
            .map(|s| format!("bind_{}: '{s}' is not a known key, ignored", action.name()))
            .collect();
        if keys.is_empty() {
            warnings.push(format!(
                "bind_{}: no usable key, keeping the default",
                action.name()
            ));
            return warnings;
        }
        self.map.retain(|_, a| *a != action);
        for key in keys {
            self.map.insert(key, action);
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn defaults_cover_every_action() {
        let km = KeyMap::default();
        for action in ALL {
            assert!(
                km.map.values().any(|a| a == action),
                "{} has no default binding",
                action.name()
            );
        }
        assert_eq!(km.action(ev(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(km.action(ev(KeyCode::Char(' '))), Some(Action::Pause));
        assert_eq!(km.action(ev(KeyCode::F(9))), Some(Action::Kill));
    }

    #[test]
    fn key_spec_parsing() {
        assert_eq!(KeyPress::parse("k").unwrap().code, KeyCode::Char('k'));
        assert_eq!(KeyPress::parse("PgDn").unwrap().code, KeyCode::PageDown);
        assert_eq!(KeyPress::parse("f12").unwrap().code, KeyCode::F(12));
        assert_eq!(KeyPress::parse("space").unwrap().code, KeyCode::Char(' '));
        let c = KeyPress::parse("ctrl+r").unwrap();
        assert_eq!((c.code, c.ctrl), (KeyCode::Char('r'), true));
        // Case matters for single characters.
        assert_ne!(KeyPress::parse("p"), KeyPress::parse("P"));
        assert_eq!(KeyPress::parse("f42"), None);
        assert_eq!(KeyPress::parse("nonsense"), None);
        assert_eq!(KeyPress::parse("  "), None);
    }

    #[test]
    fn rebinding_replaces_the_old_key() {
        let mut km = KeyMap::default();
        assert!(km.bind(Action::Quit, "ctrl+x").is_empty());
        assert_eq!(km.action(ev(KeyCode::Char('q'))), None);
        assert_eq!(
            km.action(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
    }

    #[test]
    fn duplicate_binding_is_last_wins() {
        let mut km = KeyMap::default();
        // `t` already toggles the tree; pointing it at the AI view must not
        // leave it doing both.
        km.bind(Action::Ai, "t");
        assert_eq!(km.action(ev(KeyCode::Char('t'))), Some(Action::Ai));
        assert_eq!(km.action(ev(KeyCode::Char('a'))), None);
    }

    #[test]
    fn unknown_keys_warn_and_keep_the_default() {
        let mut km = KeyMap::default();
        let w = km.bind(Action::Tree, "nonsense");
        assert_eq!(w.len(), 2, "one per bad key plus the fallback note");
        assert!(w[0].contains("not a known key"));
        // The default binding survives an unusable spec.
        assert_eq!(km.action(ev(KeyCode::Char('t'))), Some(Action::Tree));

        let w = km.bind(Action::Tree, "T, bogus");
        assert_eq!(w.len(), 1);
        assert_eq!(km.action(ev(KeyCode::Char('T'))), Some(Action::Tree));
    }

    #[test]
    fn action_names_round_trip() {
        for a in ALL {
            assert_eq!(Action::from_name(a.name()), Some(*a));
        }
        assert_eq!(Action::from_name("nope"), None);
    }
}
