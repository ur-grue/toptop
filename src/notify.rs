//! Delivering alert transitions to the outside world.
//!
//! Everything with a side effect lives here so [`crate::alerts`] can stay pure:
//! spawning the configured `alert_cmd`, and POSTing to the built-in
//! webhook/ntfy/Slack sinks. Delivery is fire-and-forget on a background
//! thread — a slow endpoint must never stall a refresh tick, and a failing one
//! must never take the monitor down with it.
//!
//! HTTP goes through `curl` rather than the hand-rolled client in
//! `metrics::infer`: Slack and ntfy.sh are HTTPS-only, and TLS is not something
//! to hand-roll or to pull a dependency tree in for. The binary itself stays
//! dependency-free — `curl` is only needed if you configure a sink.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::alerts::{sink_request, SinkKind, Transition};

/// Where alert transitions should go.
#[derive(Clone, Debug, Default)]
pub struct Notifier {
    /// Shell command run on every transition, with `TOPTOP_ALERT_*` in its env.
    pub cmd: Option<String>,
    /// Built-in HTTP sinks.
    pub sinks: Vec<(SinkKind, String)>,
}

impl Notifier {
    /// Whether anything is configured — lets callers skip the work entirely.
    pub fn is_empty(&self) -> bool {
        self.cmd.is_none() && self.sinks.is_empty()
    }

    /// Deliver one transition to every configured target.
    pub fn dispatch(&self, t: &Transition) {
        if let Some(cmd) = &self.cmd {
            run_command(cmd, t);
        }
        for (kind, url) in &self.sinks {
            post(*kind, url, t);
        }
    }

    /// Deliver a batch, skipping the work when nothing is configured.
    pub fn dispatch_all(&self, transitions: &[Transition]) {
        if self.is_empty() {
            return;
        }
        for t in transitions {
            self.dispatch(t);
        }
    }
}

/// Run the user's command with the alert in its environment.
fn run_command(cmd: &str, t: &Transition) {
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let child = Command::new(shell)
        .arg(flag)
        .arg(cmd)
        .env("TOPTOP_ALERT_STATE", t.state.label())
        .env("TOPTOP_ALERT_SEVERITY", t.level.label())
        .env("TOPTOP_ALERT_KEY", t.key)
        .env("TOPTOP_ALERT_DETAIL", &t.detail)
        .env("TOPTOP_ALERT_MSG", &t.message)
        // The TUI owns the terminal; anything the command prints would corrupt it.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        // Reap it off-thread so a long-running hook neither blocks the tick
        // nor leaves a zombie behind.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// POST a transition to one sink via `curl`.
fn post(kind: SinkKind, url: &str, t: &Transition) {
    let req = sink_request(kind, url, t);
    std::thread::spawn(move || {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .args(["--max-time", "10"])
            .args(["-X", "POST"]);
        for (k, v) in &req.headers {
            cmd.args(["-H", &format!("{k}: {v}")]);
        }
        // The body goes over stdin so no shell or argv escaping is involved.
        cmd.args(["--data-binary", "@-"])
            .arg(&req.url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(req.body.as_bytes());
        }
        let _ = child.wait();
    });
}
