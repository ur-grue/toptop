//! Signal delivery: the reported "signalling doesn't work" symptom.

use std::process::Command;
use std::time::Duration;

use sysinfo::Signal;
use toptop::metrics::{Collector, SignalOutcome};

#[test]
fn delivers_signal_to_owned_process() {
    let mut child = Command::new("sleep").arg("30").spawn().expect("spawn sleep");
    let pid = child.id();

    let mut collector = Collector::new(64);
    // Populate the process cache so the target is resolvable.
    collector.refresh();

    let outcome = collector.signal_process(pid, Signal::Kill);
    assert_eq!(outcome, SignalOutcome::Delivered);

    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().expect("wait").is_some(),
        "child should be dead after SIGKILL"
    );
}

#[test]
fn reports_gone_for_unknown_pid() {
    let mut collector = Collector::new(64);
    collector.refresh();
    // A PID that is essentially never live.
    assert_eq!(
        collector.signal_process(0x7FFF_FFF0, Signal::Term),
        SignalOutcome::Gone
    );
}
