//! Lightweight, dependency-free configuration with optional persistence.
//!
//! Settings are read from `$XDG_CONFIG_HOME/toptop/config.conf` (falling back to
//! `~/.config/...`) as simple `key = value` lines, and overridable from the CLI.
//! Persisting failures are non-fatal — the monitor always runs with defaults.

use std::fs;
use std::path::PathBuf;

use crate::alerts::AlertConfig;
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
    /// Alert thresholds (VRAM spill, KV-cache saturation, queue backlog).
    pub alerts: AlertConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_ms: 1500,
            theme_idx: 0,
            tree: false,
            per_core: true,
            layout: LayoutPreset::Full,
            alerts: AlertConfig::default(),
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
        let Some(path) = Self::path() else {
            return Config::default();
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return Config::default();
        };
        Self::parse(&contents)
    }

    /// Parse `key = value` lines, falling back to defaults for anything
    /// missing or malformed.
    fn parse(contents: &str) -> Self {
        let mut cfg = Config::default();
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
                "alert_vram_pct" => {
                    if let Ok(v) = value.parse::<f32>() {
                        cfg.alerts.vram_spill_pct = v.clamp(1.0, 100.0);
                    }
                }
                "alert_kv_pct" => {
                    if let Ok(v) = value.parse::<f64>() {
                        cfg.alerts.kv_high_pct = v.clamp(1.0, 100.0);
                    }
                }
                "alert_queue" => {
                    if let Ok(v) = value.parse::<f64>() {
                        cfg.alerts.queue_high = v.max(1.0);
                    }
                }
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
             layout = {}\n\
             alert_vram_pct = {}\n\
             alert_kv_pct = {}\n\
             alert_queue = {}\n",
            self.tick_ms,
            theme_name,
            self.tree,
            self.per_core,
            self.layout.label(),
            self.alerts.vram_spill_pct,
            self.alerts.kv_high_pct,
            self.alerts.queue_high
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
    fn parses_alert_thresholds() {
        let cfg = Config::parse(
            "alert_vram_pct = 85\n\
             alert_kv_pct = 90.5\n\
             alert_queue = 4\n",
        );
        assert_eq!(cfg.alerts.vram_spill_pct, 85.0);
        assert_eq!(cfg.alerts.kv_high_pct, 90.5);
        assert_eq!(cfg.alerts.queue_high, 4.0);
    }

    #[test]
    fn clamps_and_ignores_bad_alert_values() {
        let cfg = Config::parse(
            "alert_vram_pct = 250\n\
             alert_kv_pct = -5\n\
             alert_queue = lots\n",
        );
        assert_eq!(cfg.alerts.vram_spill_pct, 100.0);
        assert_eq!(cfg.alerts.kv_high_pct, 1.0);
        assert_eq!(cfg.alerts.queue_high, AlertConfig::default().queue_high);
    }

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }
}
