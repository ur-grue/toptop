//! cgroup v2 awareness: what the *container* is allowed, not what the host has.
//!
//! Inside a container, htop, btop and toptop-before-this-module all report the
//! host's CPU count and memory. A pod limited to 2 cores on a 64-core node
//! shows "3% CPU" while it is in fact pinned and being throttled. Anyone
//! running inference in Docker or Kubernetes is reading numbers that do not
//! describe their process.
//!
//! Everything here parses `/sys/fs/cgroup` (v2 unified hierarchy) and is a pure
//! function of file contents, so the parsers are unit-tested against real
//! fixture text. Outside a container — or on any non-Linux host — the files are
//! absent and every function returns `None`, which callers treat as "use the
//! host numbers".

use std::path::Path;

/// The limits and usage of the cgroup this process belongs to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cgroup {
    /// CPU quota in whole cores, e.g. `2.5`. `None` means unlimited (`max`).
    pub cpu_limit: Option<f64>,
    /// Memory limit in bytes. `None` means unlimited.
    pub mem_limit: Option<u64>,
    /// Current memory usage in bytes, as the kernel accounts it.
    pub mem_used: Option<u64>,
    /// Cumulative microseconds this cgroup was throttled off-CPU, from
    /// `cpu.stat`. A rising value is the smoking gun for "my container is
    /// slow and the host looks idle".
    pub throttled_usec: Option<u64>,
    /// Number of throttling periods, for the same reason.
    pub nr_throttled: Option<u64>,
}

impl Cgroup {
    /// Whether any limit at all is in force — i.e. whether reporting cgroup
    /// numbers instead of host numbers is meaningful.
    pub fn is_limited(&self) -> bool {
        self.cpu_limit.is_some() || self.mem_limit.is_some()
    }

    /// Memory usage as a percentage of the limit, when both are known.
    pub fn mem_pct(&self) -> Option<f64> {
        let (used, limit) = (self.mem_used?, self.mem_limit?);
        (limit > 0).then(|| used as f64 / limit as f64 * 100.0)
    }
}

/// Parse `cpu.max`: `"<quota|max> <period>"`, quota and period in microseconds.
/// Returns the limit in whole cores, or `None` for `max` (unlimited).
pub fn parse_cpu_max(text: &str) -> Option<f64> {
    let mut parts = text.split_whitespace();
    let quota = parts.next()?;
    if quota == "max" {
        return None;
    }
    let quota: f64 = quota.parse().ok()?;
    let period: f64 = parts.next().unwrap_or("100000").parse().ok()?;
    (period > 0.0).then(|| quota / period)
}

/// Parse a cgroup byte file (`memory.max`, `memory.current`). `max` is
/// unlimited.
pub fn parse_bytes(text: &str) -> Option<u64> {
    let text = text.trim();
    if text == "max" {
        return None;
    }
    text.parse().ok()
}

/// Pull `throttled_usec` and `nr_throttled` out of `cpu.stat`, which is a
/// `key value` line list.
pub fn parse_cpu_stat(text: &str) -> (Option<u64>, Option<u64>) {
    let find = |key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == key).then(|| v.trim().parse::<u64>().ok())?
        })
    };
    (find("throttled_usec"), find("nr_throttled"))
}

/// Read the cgroup v2 limits for this process, if it is in a limited cgroup.
///
/// Reads the *unified* mount at `/sys/fs/cgroup`, which inside a container is
/// already namespaced to that container's own cgroup — so the plain paths are
/// the right ones, without resolving `/proc/self/cgroup`.
pub fn current() -> Option<Cgroup> {
    read_from(Path::new("/sys/fs/cgroup"))
}

/// The path-taking half of [`current`], so tests can point it at a fixture.
pub fn read_from(root: &Path) -> Option<Cgroup> {
    let read = |name: &str| std::fs::read_to_string(root.join(name)).ok();
    // Without cpu.max or memory.max there is no v2 hierarchy here worth
    // reporting — cgroup v1, a host, or a non-Linux system.
    let cpu_max = read("cpu.max");
    let mem_max = read("memory.max");
    if cpu_max.is_none() && mem_max.is_none() {
        return None;
    }
    let (throttled_usec, nr_throttled) = read("cpu.stat")
        .map(|s| parse_cpu_stat(&s))
        .unwrap_or((None, None));

    let cg = Cgroup {
        cpu_limit: cpu_max.as_deref().and_then(parse_cpu_max),
        mem_limit: mem_max.as_deref().and_then(parse_bytes),
        mem_used: read("memory.current").as_deref().and_then(parse_bytes),
        throttled_usec,
        nr_throttled,
    };
    // An unlimited cgroup is the same as no cgroup for display purposes.
    cg.is_limited().then_some(cg)
}

/// Extract a container or pod identity from a `/proc/<pid>/cgroup` line.
///
/// Handles the shapes that actually occur: Docker (`/docker/<64-hex>`),
/// containerd/CRI (`cri-containerd-<hex>.scope`), and systemd-managed
/// Kubernetes slices (`kubepods-besteffort-pod<uid>.slice`). Returns a short
/// label, not the raw path — a 64-character hash is not a useful column.
pub fn container_label(cgroup_line: &str) -> Option<String> {
    let path = cgroup_line.rsplit(':').next()?.trim();
    if path.is_empty() || path == "/" {
        return None;
    }
    let last = path.rsplit('/').find(|s| !s.is_empty())?;

    // Kubernetes pod slices carry the pod UID, which is what identifies the
    // pod across its containers.
    if let Some(rest) = last.strip_prefix("kubepods-") {
        if let Some(uid) = rest.split("pod").nth(1) {
            let uid = uid.trim_end_matches(".slice");
            return Some(format!("pod/{}", short_id(uid)));
        }
    }
    // containerd/CRI scopes: cri-containerd-<id>.scope, docker-<id>.scope.
    for prefix in ["cri-containerd-", "docker-", "crio-"] {
        if let Some(rest) = last.strip_prefix(prefix) {
            return Some(short_id(rest.trim_end_matches(".scope")));
        }
    }
    // Plain Docker: /docker/<id>.
    if path.starts_with("/docker/") || path.starts_with("/system.slice/docker") {
        return Some(short_id(last));
    }
    // A bare 64-hex id anywhere else is a container id too.
    if last.len() >= 32 && last.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(short_id(last));
    }
    None
}

/// Shorten an id to the 12 characters everyone actually reads, leaving
/// human-readable names alone.
fn short_id(id: &str) -> String {
    let id = id.trim_matches('-');
    // Kubernetes pod UIDs use underscores in slice names, Docker ids are bare
    // hex — both are opaque, and both are recognizable from their first 12.
    if id.len() > 12
        && id
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_')
    {
        id.chars().take(12).collect()
    } else {
        id.to_string()
    }
}

/// Read the container label for one PID, or `None` outside a container.
pub fn container_of(pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    text.lines().find_map(container_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_max_parsing() {
        assert_eq!(parse_cpu_max("200000 100000"), Some(2.0));
        assert_eq!(parse_cpu_max("50000 100000"), Some(0.5));
        // A quota with no period defaults to the kernel's 100ms.
        assert_eq!(parse_cpu_max("250000"), Some(2.5));
        assert_eq!(parse_cpu_max("max 100000"), None, "unlimited");
        assert_eq!(parse_cpu_max(""), None);
        assert_eq!(parse_cpu_max("nonsense 100000"), None);
        assert_eq!(parse_cpu_max("100000 0"), None, "no division by zero");
    }

    #[test]
    fn byte_file_parsing() {
        assert_eq!(parse_bytes("8589934592\n"), Some(8_589_934_592));
        assert_eq!(parse_bytes("max\n"), None);
        assert_eq!(parse_bytes("garbage"), None);
    }

    #[test]
    fn cpu_stat_parsing() {
        let text = "usage_usec 12345\nuser_usec 6000\nsystem_usec 6345\n\
                    nr_periods 900\nnr_throttled 42\nthrottled_usec 987654\n";
        assert_eq!(parse_cpu_stat(text), (Some(987_654), Some(42)));
        // A kernel without CPU accounting enabled omits both.
        assert_eq!(parse_cpu_stat("usage_usec 1\n"), (None, None));
    }

    #[test]
    fn container_labels_from_real_cgroup_lines() {
        // Docker
        assert_eq!(
            container_label(
                "0::/docker/3f7a9c1b2e4d5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b"
            ),
            Some("3f7a9c1b2e4d".to_string())
        );
        // containerd / CRI scope
        assert_eq!(
            container_label(
                "0::/system.slice/cri-containerd-abcdef0123456789abcdef0123456789.scope"
            ),
            Some("abcdef012345".to_string())
        );
        // Kubernetes pod slice — the pod UID identifies the pod, not the
        // container, which is what you want to group by.
        let k8s = "0::/kubepods.slice/kubepods-besteffort.slice/\
                   kubepods-besteffort-pod7d4f1e2a_3b4c_5d6e_7f80_91a2b3c4d5e6.slice";
        assert_eq!(container_label(k8s), Some("pod/7d4f1e2a_3b4".to_string()));
        // Not in a container.
        assert_eq!(container_label("0::/"), None);
        assert_eq!(container_label("0::/user.slice/user-1000.slice"), None);
        assert_eq!(container_label(""), None);
    }

    #[test]
    fn reading_a_fixture_hierarchy() {
        let dir = std::env::temp_dir().join(format!("toptop-cg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cpu.max"), "200000 100000\n").unwrap();
        std::fs::write(dir.join("memory.max"), "8589934592\n").unwrap();
        std::fs::write(dir.join("memory.current"), "4294967296\n").unwrap();
        std::fs::write(
            dir.join("cpu.stat"),
            "nr_throttled 7\nthrottled_usec 1234\n",
        )
        .unwrap();

        let cg = read_from(&dir).expect("a limited cgroup");
        assert_eq!(cg.cpu_limit, Some(2.0));
        assert_eq!(cg.mem_limit, Some(8_589_934_592));
        assert_eq!(cg.mem_used, Some(4_294_967_296));
        assert_eq!(cg.mem_pct(), Some(50.0));
        assert_eq!(cg.nr_throttled, Some(7));
        assert!(cg.is_limited());

        // An unlimited cgroup is indistinguishable from no cgroup for display.
        std::fs::write(dir.join("cpu.max"), "max 100000\n").unwrap();
        std::fs::write(dir.join("memory.max"), "max\n").unwrap();
        assert_eq!(read_from(&dir), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_hierarchy_means_no_cgroup() {
        assert_eq!(read_from(Path::new("/nonexistent/cgroup")), None);
    }
}
