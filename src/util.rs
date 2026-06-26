//! Small, dependency-free formatting helpers shared across the UI.

/// Human-readable byte size using binary (IEC) units.
///
/// ```
/// assert_eq!(toptop::util::human_bytes(0), "0 B");
/// assert_eq!(toptop::util::human_bytes(1024), "1.0 KiB");
/// ```
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else if value >= 100.0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Human-readable throughput such as `12.3 MiB/s`.
pub fn human_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 0.5 {
        return "0 B/s".to_string();
    }
    format!("{}/s", human_bytes(bytes_per_sec.round() as u64))
}

/// Compact byte size that always fits in a narrow column, e.g. `1.2G`.
pub fn short_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "K", "M", "G", "T", "P", "E"];
    if bytes == 0 {
        return "0".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}", bytes)
    } else if value >= 10.0 {
        format!("{:.0}{}", value, UNITS[unit])
    } else {
        format!("{:.1}{}", value, UNITS[unit])
    }
}

/// Format a duration in seconds as `Dd HH:MM:SS` (days only when non-zero).
pub fn human_duration(total_secs: u64) -> String {
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let mins = (total_secs % 3_600) / 60;
    let secs = total_secs % 60;
    if days > 0 {
        format!("{}d {:02}:{:02}:{:02}", days, hours, mins, secs)
    } else {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    }
}

/// Compact runtime for the process table, e.g. `3d12h`, `4h07m`, `12:30`.
pub fn compact_duration(total_secs: u64) -> String {
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let mins = (total_secs % 3_600) / 60;
    let secs = total_secs % 60;
    if days > 0 {
        format!("{}d{:02}h", days, hours)
    } else if hours > 0 {
        format!("{}h{:02}m", hours, mins)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

/// Clamp a percentage into the `0.0..=100.0` range and guard against NaN.
pub fn clamp_pct(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 100.0)
    }
}

/// Truncate a string to a max display width, appending `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else if max == 1 {
        "…".to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_scale() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(human_bytes(150 * 1024 * 1024), "150 MiB");
    }

    #[test]
    fn short_scale() {
        assert_eq!(short_bytes(0), "0");
        assert_eq!(short_bytes(2048), "2.0K");
        assert_eq!(short_bytes(20 * 1024), "20K");
    }

    #[test]
    fn durations() {
        assert_eq!(human_duration(0), "00:00:00");
        assert_eq!(human_duration(3661), "01:01:01");
        assert_eq!(human_duration(90_061), "1d 01:01:01");
        assert_eq!(compact_duration(45), "00:45");
        assert_eq!(compact_duration(3700), "1h01m");
        assert_eq!(compact_duration(180_000), "2d02h");
    }

    #[test]
    fn pct_guard() {
        assert_eq!(clamp_pct(f32::NAN), 0.0);
        assert_eq!(clamp_pct(150.0), 100.0);
        assert_eq!(clamp_pct(-5.0), 0.0);
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello", 3), "he…");
        assert_eq!(truncate("hello", 1), "…");
        assert_eq!(truncate("hello", 0), "");
    }
}
