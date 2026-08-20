//! Lightweight, dependency-free configuration with optional persistence.
//!
//! Settings are read from `$XDG_CONFIG_HOME/toptop/config.conf` (falling back to
//! `~/.config/...`) as simple `key = value` lines, and overridable from the CLI.
//! `--config <path>` substitutes an explicit file for the default location.
//! Persisting failures are non-fatal — the monitor always runs with defaults,
//! and a failed save is reported as a one-line warning on exit.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::alerts::AlertConfig;
use crate::app::LayoutPreset;
use crate::metrics::infer::{parse_target, Target};
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
    /// Manually configured inference-server targets (`--llm-server`), scraped
    /// alongside auto-discovery.
    pub llm_servers: Vec<Target>,
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
            llm_servers: Vec::new(),
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

    /// Load config from the default location, falling back to defaults.
    pub fn load() -> Self {
        match Self::path() {
            Some(path) => Self::load_path(&path),
            None => Config::default(),
        }
    }

    /// Load config from an explicit file (`--config`), ignoring any errors
    /// and falling back to defaults.
    pub fn load_path(path: &Path) -> Self {
        let Ok(contents) = fs::read_to_string(path) else {
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
                "llm_servers" => {
                    // Malformed entries are skipped like any other bad config
                    // value; `--llm-server` reports them instead.
                    cfg.llm_servers
                        .extend(value.split(',').filter_map(|t| parse_target(t).ok()));
                }
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

    /// Persist the current config to the default location.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot resolve config path (HOME and XDG_CONFIG_HOME unset)",
            )
        })?;
        self.save_path(&path)
    }

    /// Persist the current config to an explicit file (`--config`).
    pub fn save_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
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
        fs::write(path, body)
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
    fn parses_llm_server_targets() {
        let cfg = Config::parse("llm_servers = gpu-box:8000, 10.0.0.5:11434, garbage\n");
        // Malformed entries are dropped, valid ones kept, in order.
        assert_eq!(cfg.llm_servers.len(), 2);
        assert_eq!(cfg.llm_servers[0].host, "gpu-box");
        assert_eq!(cfg.llm_servers[1].port, 11434);
    }

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    /// A scratch path unique to this test process.
    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toptop-cfg-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn save_and_load_explicit_path_round_trips() {
        let path = scratch("roundtrip").join("config.conf");
        let cfg = Config {
            tick_ms: 2500,
            tree: true,
            per_core: false,
            ..Config::default()
        };
        cfg.save_path(&path).expect("save should succeed");
        let loaded = Config::load_path(&path);
        assert_eq!(loaded.tick_ms, 2500);
        assert!(loaded.tree);
        assert!(!loaded.per_core);
        fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_missing_explicit_path_falls_back_to_defaults() {
        let cfg = Config::load_path(Path::new("/nonexistent/toptop/config.conf"));
        assert_eq!(cfg.tick_ms, Config::default().tick_ms);
    }

    #[test]
    fn save_to_unwritable_path_reports_the_error() {
        // The parent "directory" is a regular file, so the save must fail.
        let blocker = scratch("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let cfg = Config::default();
        assert!(cfg.save_path(&blocker.join("config.conf")).is_err());
        fs::remove_file(&blocker).ok();
    }
}
