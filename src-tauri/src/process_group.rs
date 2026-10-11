//! Child processes that run in a process group of their own (git with its
//! transfer helpers, an AI engine with whatever it starts), and stopping
//! such a group without ever hitting a stranger.
//!
//! The id of a process group is the pid of its first process. That pid stays
//! reserved while the process is running, while it has exited but was not
//! waited for yet, and while other processes are still in the group. Once
//! the first process has been waited for and the group is empty, the system
//! hands the number out again, on macOS often within moments. So the group
//! is only ever signalled **before** its first process is waited for: its
//! end is noticed with `waitid(WNOWAIT)`, which leaves it waitable.

use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

/// How often a stopping group is looked at.
const POLL: Duration = Duration::from_millis(20);

/// Sends `signal` to the process group `pgid`.
///
/// Only call this while the group's first process has not been waited for
/// (see the module documentation).
pub fn signal(pgid: u32, signal: libc::c_int) {
    if let Ok(pid) = libc::pid_t::try_from(pgid) {
        if pid > 0 {
            // SAFETY: plain kill(2) on a process group this app created.
            unsafe {
                libc::kill(-pid, signal);
            }
        }
    }
}

/// True once `child` has ended. Unlike `Child::try_wait` this does not wait
/// for it: its pid, and with it the id of its process group, stays reserved
/// until `Child::wait` is called.
pub fn has_exited(child: &Child) -> bool {
    // SAFETY: waitid(2) with a zeroed out-parameter, on a child of this
    // process. WNOWAIT leaves the child waitable; with WNOHANG and nothing to
    // report the structure stays zeroed.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let result = libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        // An error means there is nothing to wait for any more.
        result != 0 || info.si_signo == libc::SIGCHLD
    }
}

/// Call once [`has_exited`] is true: kills what the child left behind in its
/// group (helpers must not outlive it), then waits for the child.
pub fn reap(child: &mut Child) -> Option<ExitStatus> {
    signal(child.id(), libc::SIGKILL);
    child.wait().ok()
}

/// Stops the child's whole group: SIGTERM, which lets a program clean up
/// after itself, then SIGKILL after `grace`. Waits for the child.
pub fn stop(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    signal(child.id(), libc::SIGTERM);
    let until = Instant::now() + grace;
    while Instant::now() < until && !has_exited(child) {
        thread::sleep(POLL);
    }
    // Whoever ignored SIGTERM, the child itself included.
    reap(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn spawn(script: &str) -> Child {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn sh")
    }

    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn gone_soon(pid: libc::pid_t) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pid) && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        !alive(pid)
    }

    fn wait_exited(child: &Child) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !has_exited(child) {
            assert!(Instant::now() < deadline, "the child never ended");
            thread::sleep(POLL);
        }
    }

    fn pid_in(file: &std::path::Path) -> libc::pid_t {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pid) = std::fs::read_to_string(file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "no pid in {}", file.display());
            thread::sleep(POLL);
        }
    }

    #[test]
    fn noticing_the_end_keeps_the_child_waitable() {
        let mut child = spawn("exit 7");
        wait_exited(&child);
        // Still true when asked again, and the exit status is still there.
        assert!(has_exited(&child));
        let own = libc::pid_t::try_from(child.id()).unwrap();
        assert!(alive(own), "not waited for yet: the pid is still reserved");
        assert_eq!(reap(&mut child).and_then(|status| status.code()), Some(7));
        // Waited for: nothing left to notice, and no error either.
        assert!(has_exited(&child));
    }

    #[test]
    fn a_running_child_has_not_exited() {
        let mut child = spawn("sleep 30");
        assert!(!has_exited(&child));
        let status = stop(&mut child, Duration::from_millis(500));
        assert_eq!(status.and_then(|status| status.code()), None, "ended by a signal");
    }

    #[test]
    fn reaping_kills_the_helpers_the_child_left_behind() {
        let dir = TempDir::new("process-group-helpers");
        let pid_file = dir.path().join("helper.pid");
        let mut child = spawn(&format!(
            "sleep 30 & echo $! > '{}'; exit 0",
            pid_file.display()
        ));
        let helper = pid_in(&pid_file);
        wait_exited(&child);
        assert!(alive(helper), "the helper outlives the child at first");
        assert_eq!(reap(&mut child).and_then(|status| status.code()), Some(0));
        assert!(gone_soon(helper));
    }

    #[test]
    fn stopping_ends_a_group_that_ignores_sigterm() {
        let dir = TempDir::new("process-group-stubborn");
        let pid_file = dir.path().join("helper.pid");
        let mut child = spawn(&format!(
            "trap '' TERM; (trap '' TERM; sleep 30) & echo $! > '{}'; sleep 30",
            pid_file.display()
        ));
        let helper = pid_in(&pid_file);
        let began = Instant::now();
        let status = stop(&mut child, Duration::from_millis(300));
        assert!(began.elapsed() < Duration::from_secs(5));
        assert!(status.is_some());
        assert!(gone_soon(helper));
    }

    #[test]
    fn stopping_a_cooperative_child_does_not_wait_for_the_grace_period() {
        let mut child = spawn("sleep 30");
        let began = Instant::now();
        stop(&mut child, Duration::from_secs(20));
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}
