//! Network connection enumeration by parsing `/proc/net/{tcp,tcp6,udp,udp6}`
//! and mapping socket inodes back to owning processes via `/proc/<pid>/fd`.
//!
//! The hex address/state parsers are pure and unit-tested; the `/proc` scan is
//! done on demand (only while the connections view is open) since walking every
//! process's file descriptors is relatively expensive.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

/// A single network connection joined to its owning process when known.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub proto: &'static str,
    pub local: String,
    pub remote: String,
    pub state: &'static str,
    pub pid: Option<u32>,
    pub process: String,
}

/// Decode a little-endian IPv4 hex address as printed by the kernel
/// (e.g. `0100007F:1F90` → `127.0.0.1:8080`).
pub fn parse_ipv4_hex(s: &str) -> Option<(String, u16)> {
    let (addr, port) = s.split_once(':')?;
    if addr.len() != 8 {
        return None;
    }
    let raw = u32::from_str_radix(addr, 16).ok()?;
    let b = raw.to_le_bytes(); // kernel stores the address in host (LE) order
    let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
    let port = u16::from_str_radix(port, 16).ok()?;
    Some((ip.to_string(), port))
}

/// Decode a 32-hex-char IPv6 address as printed by the kernel, where each
/// 32-bit word is in host byte order.
pub fn parse_ipv6_hex(s: &str) -> Option<(String, u16)> {
    let (addr, port) = s.split_once(':')?;
    if addr.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (i, chunk) in addr.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        bytes[i] = u8::from_str_radix(pair, 16).ok()?;
    }
    // Reverse each 4-byte word to undo the kernel's per-word byte swap.
    for word in bytes.chunks_mut(4) {
        word.reverse();
    }
    let mut segs = [0u16; 8];
    for (i, seg) in segs.iter_mut().enumerate() {
        *seg = u16::from_be_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
    }
    let ip = Ipv6Addr::new(
        segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7],
    );
    let port = u16::from_str_radix(port, 16).ok()?;
    Some((ip.to_string(), port))
}

/// Map a TCP state hex code (`/proc/net/tcp` column `st`) to its name.
pub fn tcp_state(hex: &str) -> &'static str {
    match hex {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        "0C" => "NEW_SYN_RECV",
        _ => "UNKNOWN",
    }
}

/// A parsed `/proc/net/*` row before it is joined to a process.
#[derive(Debug, PartialEq)]
pub struct RawConn {
    pub local: String,
    pub remote: String,
    pub state_hex: String,
    pub inode: u64,
}

/// Parse one data line of `/proc/net/{tcp,tcp6,udp,udp6}`.
pub fn parse_net_line(line: &str, v6: bool) -> Option<RawConn> {
    let f: Vec<&str> = line.split_whitespace().collect();
    // sl local rem st tx:rx tr:tm retrnsmt uid timeout inode ...
    if f.len() < 10 {
        return None;
    }
    let fmt = |hex: &str| -> Option<String> {
        let (ip, port) = if v6 {
            parse_ipv6_hex(hex)?
        } else {
            parse_ipv4_hex(hex)?
        };
        Some(if ip.contains(':') {
            format!("[{ip}]:{port}")
        } else {
            format!("{ip}:{port}")
        })
    };
    Some(RawConn {
        local: fmt(f[1])?,
        remote: fmt(f[2])?,
        state_hex: f[3].to_string(),
        inode: f[9].parse().ok()?,
    })
}

fn parse_table(contents: &str, v6: bool) -> Vec<RawConn> {
    contents
        .lines()
        .skip(1) // header row
        .filter_map(|l| parse_net_line(l, v6))
        .collect()
}

/// Build a `socket inode -> (pid, process name)` map by scanning `/proc/*/fd`.
fn inode_to_pid() -> HashMap<u64, (u32, String)> {
    let mut map = HashMap::new();
    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return map;
    };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(&fd_dir) else {
            continue;
        };
        let mut comm: Option<String> = None;
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let t = target.to_string_lossy();
            if let Some(rest) = t.strip_prefix("socket:[") {
                if let Some(inode) = rest.strip_suffix(']').and_then(|n| n.parse::<u64>().ok()) {
                    let pname = comm.get_or_insert_with(|| {
                        std::fs::read_to_string(entry.path().join("comm"))
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default()
                    });
                    map.entry(inode).or_insert((pid, pname.clone()));
                }
            }
        }
    }
    map
}

/// Local TCP port of a `/proc/net/tcp{,6}` line, if it is in LISTEN state.
fn listen_port(line: &str) -> Option<u16> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 4 || f[3] != "0A" {
        return None;
    }
    let (_, port) = f[1].split_once(':')?;
    u16::from_str_radix(port, 16).ok()
}

/// Enumerate listening TCP sockets joined to their owning PID, as `(port, pid)`.
/// Used to auto-discover local inference servers to scrape.
pub fn listening_sockets() -> Vec<(u16, u32)> {
    let inodes = inode_to_pid();
    let mut out = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines().skip(1) {
            let Some(port) = listen_port(line) else {
                continue;
            };
            // The inode is column 9 (same layout as parse_net_line).
            let f: Vec<&str> = line.split_whitespace().collect();
            let Some(inode) = f.get(9).and_then(|s| s.parse::<u64>().ok()) else {
                continue;
            };
            if let Some((pid, _)) = inodes.get(&inode) {
                out.push((port, *pid));
            }
        }
    }
    out
}

/// Enumerate all current TCP/UDP connections, joined to owning processes.
pub fn collect() -> Vec<Connection> {
    let sources: &[(&'static str, &str, bool, bool)] = &[
        ("tcp", "/proc/net/tcp", false, true),
        ("tcp6", "/proc/net/tcp6", true, true),
        ("udp", "/proc/net/udp", false, false),
        ("udp6", "/proc/net/udp6", true, false),
    ];
    let inodes = inode_to_pid();
    let mut out = Vec::new();
    for (proto, path, v6, is_tcp) in sources {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for raw in parse_table(&contents, *v6) {
            let (pid, process) = match inodes.get(&raw.inode) {
                Some((pid, name)) => (Some(*pid), name.clone()),
                None => (None, String::new()),
            };
            out.push(Connection {
                proto,
                local: raw.local,
                remote: raw.remote,
                state: if *is_tcp {
                    tcp_state(&raw.state_hex)
                } else {
                    "—"
                },
                pid,
                process,
            });
        }
    }
    // Listening sockets first, then established, then the rest; stable within.
    out.sort_by_key(|c| match c.state {
        "LISTEN" => 0,
        "ESTABLISHED" => 1,
        _ => 2,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_loopback() {
        assert_eq!(
            parse_ipv4_hex("0100007F:1F90"),
            Some(("127.0.0.1".to_string(), 8080))
        );
    }

    #[test]
    fn ipv4_any() {
        assert_eq!(
            parse_ipv4_hex("00000000:0050"),
            Some(("0.0.0.0".to_string(), 80))
        );
    }

    #[test]
    fn ipv6_loopback() {
        let (ip, port) = parse_ipv6_hex("00000000000000000000000001000000:0050").unwrap();
        assert_eq!(ip, "::1");
        assert_eq!(port, 80);
    }

    #[test]
    fn states() {
        assert_eq!(tcp_state("0A"), "LISTEN");
        assert_eq!(tcp_state("01"), "ESTABLISHED");
        assert_eq!(tcp_state("zz"), "UNKNOWN");
    }

    #[test]
    fn parse_line() {
        let line = "  0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 54321 1 ...";
        let rc = parse_net_line(line, false).unwrap();
        assert_eq!(rc.local, "127.0.0.1:8080");
        assert_eq!(rc.remote, "0.0.0.0:0");
        assert_eq!(rc.state_hex, "0A");
        assert_eq!(rc.inode, 54321);
    }
}
