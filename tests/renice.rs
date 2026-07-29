//! Renice: changing a process's scheduling priority.

use std::process::Command;

use toptop::metrics::{Collector, PriorityOutcome};

#[test]
fn applies_nice_to_owned_process() {
    let mut child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();

    let collector = Collector::new(64);
    // Raising nice (lower priority) on our own child is always permitted.
    assert_eq!(collector.set_priority(pid, 15), PriorityOutcome::Applied);

    // SAFETY: getpriority only reads the target's nice value.
    let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
    assert_eq!(nice, 15, "nice value should have been applied");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn reports_gone_for_unknown_pid() {
    let collector = Collector::new(64);
    assert_eq!(
        collector.set_priority(0x7FFF_FFF0, 0),
        PriorityOutcome::Gone
    );
}

#[test]
fn reports_permission_denied_for_foreign_process() {
    // Root can renice anything, so the guarantee doesn't hold there.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    // PID 1 (init/launchd) is root-owned; renicing it as a normal user fails
    // with EPERM before changing anything.
    let collector = Collector::new(64);
    assert_eq!(collector.set_priority(1, -5), PriorityOutcome::NotPermitted);
}
