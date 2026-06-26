//! Lightweight, dependency-free configuration with optional persistence.
//!
//! Settings are read from `$XDG_CONFIG_HOME/toptop/config.conf` (falling back to
//! `~/.config/...`) as simple `key = value` lines, and overridable from the CLI.
//! Persisting failures are non-fatal — the monitor always runs with defaults.

use std::fs;
use std::path::PathBuf;

use crate::app::LayoutPreset;
use crate::theme;

/// User-tunable settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Refresh interval in milliseconds.
    pub tick_ms: u64,
    /// Index into [`theme::THEMES`].
    pub theme_idx: usize,
    /// Show the process tree by default.
    pub tree: bool,
    /// Show per-core CPU meters by default.
    pub per_core: bool,
    /// Body layout preset.
    pub layout: LayoutPreset,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_ms: 1500,
            theme_idx: 0,
            tree: false,
            per_core: true,
            layout: LayoutPreset::Full,
        }
    }
}

impl Config {
    /// Resolve the config file path, honoring `XDG_CONFIG_HOME`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("toptop").join("config.conf"))
    }

    /// Load config from disk, ignoring any errors and falling back to defaults.
    pub fn load() -> Self {
        let mut cfg = Config::default();
        let Some(path) = Self::path() else {
            return cfg;
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return cfg;
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "tick_ms" => {
                    if let Ok(v) = value.parse::<u64>() {
                        cfg.tick_ms = v.clamp(100, 60_000);
                    }
                }
                "theme" => {
                    if let Some(idx) = theme::index_by_name(value) {
                        cfg.theme_idx = idx;
                    }
                }
                "tree" => cfg.tree = parse_bool(value).unwrap_or(cfg.tree),
                "per_core" => cfg.per_core = parse_bool(value).unwrap_or(cfg.per_core),
                "layout" => cfg.layout = LayoutPreset::from_name(value).unwrap_or(cfg.layout),
                _ => {}
            }
        }
        cfg
    }

    /// Persist the current config. Errors are intentionally swallowed.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let theme_name = theme::THEMES
            .get(self.theme_idx)
            .map(|t| t.name)
            .unwrap_or("gruvbox");
        let body = format!(
            "# toptop configuration\n\
             tick_ms = {}\n\
             theme = {}\n\
             tree = {}\n\
             per_core = {}\n\
             layout = {}\n",
            self.tick_ms,
            theme_name,
            self.tree,
            self.per_core,
            self.layout.label()
        );
        let _ = fs::write(path, body);
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert!(c.tick_ms >= 100);
        assert!(c.theme_idx < theme::THEMES.len());
    }

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }
}
