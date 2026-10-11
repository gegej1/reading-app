//! Running the engine: a child process in its own process group whose output
//! is handed over line by line while it runs, stopped on request or when its
//! time is up.
//!
//! Stopping means SIGTERM for the whole group (the engine and whatever it
//! started), then SIGKILL after a grace period (see `crate::process_group`).
//! Helpers that survive the engine are killed too, and a helper that keeps
//! the output pipe open cannot make the app wait.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::process_group;

/// After the engine exits, wait at most this long for the rest of its output.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// How much of stderr is kept (its end).
const STDERR_TAIL: usize = 16 * 1024;
/// How often cancel and the time limit are looked at.
const TICK: Duration = Duration::from_millis(50);

/// When to stop the engine.
#[derive(Debug, Clone)]
pub struct Control {
    /// Set to stop the engine as soon as possible.
    pub cancel: Arc<AtomicBool>,
    pub timeout: Duration,
    /// After SIGTERM, how long until SIGKILL.
    pub grace: Duration,
}

/// How the engine ended.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exit {
    /// Exit code; `None` when a signal ended it or it never started.
    pub code: Option<i32>,
    pub cancelled: bool,
    pub timed_out: bool,
    /// Why the program could not be started.
    pub spawn_error: Option<String>,
    /// The end of what it wrote to stderr.
    pub stderr_tail: String,
}

/// Runs `command` to its end. `stdin` is written to its standard input, which
/// is then closed. `on_started` is called right after the start; `on_line`
/// gets every line of standard output, in order, without its line break.
pub fn run(
    mut command: Command,
    stdin: Option<String>,
    control: &Control,
    on_started: &mut dyn FnMut(),
    on_line: &mut dyn FnMut(&str),
) -> Exit {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Exit {
                spawn_error: Some(error.to_string()),
                ..Exit::default()
            }
        }
    };
    on_started();

    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // On its own thread: a large prompt must not block reading the output.
        let _ = thread::Builder::new()
            .name("engine-stdin".into())
            .spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            });
    }

    let (sender, lines) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
        let _ = thread::Builder::new()
            .name("engine-stdout".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buffer = Vec::new();
                loop {
                    buffer.clear();
                    match reader.read_until(b'\n', &mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                                buffer.pop();
                            }
                            let line = String::from_utf8_lossy(&buffer).into_owned();
                            if sender.send(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
    }
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(mut stderr) = child.stderr.take() {
        let tail = Arc::clone(&tail);
        let _ = thread::Builder::new()
            .name("engine-stderr".into())
            .spawn(move || {
                let mut chunk = [0u8; 4096];
                loop {
                    match stderr.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
                            tail.extend_from_slice(&chunk[..n]);
                            if tail.len() > STDERR_TAIL {
                                let cut = tail.len() - STDERR_TAIL;
                                tail.drain(..cut);
                            }
                        }
                    }
                }
            });
    }

    let deadline = Instant::now() + control.timeout;
    let mut exit = Exit::default();
    // `Some` once the engine was stopped (and waited for); `None` while it
    // has ended by itself and still has to be waited for.
    let stopped = loop {
        match lines.recv_timeout(TICK) {
            Ok(line) => on_line(&line),
            Err(RecvTimeoutError::Timeout) => {}
            // The pipe closed: the process is about to be gone.
            Err(RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(5)),
        }
        if process_group::has_exited(&child) {
            break None;
        }
        if control.cancel.load(Ordering::SeqCst) {
            exit.cancelled = true;
            break Some(process_group::stop(&mut child, control.grace));
        }
        if Instant::now() >= deadline {
            exit.timed_out = true;
            break Some(process_group::stop(&mut child, control.grace));
        }
    };

    // What is still in the pipe, but never wait for a helper that kept it open.
    let drained = Instant::now() + DRAIN_GRACE;
    while let Some(left) = drained.checked_duration_since(Instant::now()) {
        match lines.recv_timeout(left) {
            Ok(line) => on_line(&line),
            Err(_) => break,
        }
    }
    // Nothing the engine started may outlive it.
    let status = match stopped {
        Some(status) => status,
        None => process_group::reap(&mut child),
    };

    // Stopped from outside in the very moment it ended: that is a stop.
    if control.cancel.load(Ordering::SeqCst) && !exit.timed_out {
        exit.cancelled = true;
    }
    exit.code = status.and_then(|status| status.code());
    exit.stderr_tail = String::from_utf8_lossy(&tail.lock().unwrap_or_else(PoisonError::into_inner))
        .trim()
        .to_string();
    exit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    fn control(timeout: Duration) -> Control {
        Control {
            cancel: Arc::new(AtomicBool::new(false)),
            timeout,
            grace: Duration::from_millis(300),
        }
    }

    fn collect(command: Command, stdin: Option<&str>, control: &Control) -> (Exit, Vec<String>) {
        let mut lines = Vec::new();
        let exit = run(
            command,
            stdin.map(String::from),
            control,
            &mut || {},
            &mut |line| lines.push(line.to_string()),
        );
        (exit, lines)
    }

    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_gone(pid: libc::pid_t) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        !alive(pid)
    }

    #[test]
    fn hands_over_every_line_in_order_with_the_exit_code_and_stderr() {
        let (exit, lines) = collect(
            sh("echo '{\"a\":1}'; printf 'second\\r\\n'; echo warn >&2; printf 'no newline at the end'; exit 3"),
            None,
            &control(Duration::from_secs(20)),
        );
        assert_eq!(lines, ["{\"a\":1}", "second", "no newline at the end"]);
        assert_eq!(
            exit,
            Exit {
                code: Some(3),
                cancelled: false,
                timed_out: false,
                spawn_error: None,
                stderr_tail: "warn".into(),
            }
        );
    }

    #[test]
    fn handles_long_lines_and_a_lot_of_output() {
        let (exit, lines) = collect(
            sh("i=0; while [ $i -lt 2000 ]; do printf '%0200d\\n' $i; i=$((i+1)); done; head -c 300000 /dev/zero | tr '\\0' 'x'; echo"),
            None,
            &control(Duration::from_secs(30)),
        );
        assert_eq!(exit.code, Some(0));
        assert_eq!(lines.len(), 2001);
        assert_eq!(lines[1999].trim_start_matches('0'), "1999");
        assert_eq!(lines[2000].len(), 300_000);
    }

    #[test]
    fn writes_the_prompt_to_stdin_and_closes_it() {
        let (exit, lines) = collect(
            sh("cat; echo; echo done"),
            Some("第一行\n第二行"),
            &control(Duration::from_secs(20)),
        );
        assert_eq!(exit.code, Some(0));
        assert_eq!(lines, ["第一行", "第二行", "done"]);
        // Without a prompt stdin is closed, not inherited.
        let (exit, lines) = collect(sh("cat; echo end"), None, &control(Duration::from_secs(20)));
        assert_eq!((exit.code, lines), (Some(0), vec!["end".to_string()]));
    }

    #[test]
    fn cancel_stops_the_whole_group() {
        let dir = TempDir::new("process-cancel");
        let pid_file = dir.path().join("helper.pid");
        let control = control(Duration::from_secs(60));
        let cancel = Arc::clone(&control.cancel);
        let script = format!(
            "sleep 60 & echo $! > '{}'; echo started; wait",
            pid_file.display()
        );
        let mut started = false;
        let mut lines = Vec::new();
        let begun = Instant::now();
        let exit = run(
            sh(&script),
            None,
            &control,
            &mut || started = true,
            &mut |line| {
                lines.push(line.to_string());
                // Cancel as soon as the engine has said something.
                cancel.store(true, Ordering::SeqCst);
            },
        );
        assert!(exit.cancelled && !exit.timed_out);
        assert_eq!(exit.code, None, "ended by a signal");
        assert_eq!(lines, ["started"]);
        assert!(started);
        assert!(begun.elapsed() < Duration::from_secs(10));
        let helper: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_gone(helper), "the helper survived the cancel");
    }

    #[test]
    fn an_engine_that_ends_in_the_moment_it_is_stopped_counts_as_stopped() {
        let control = control(Duration::from_secs(60));
        let cancel = Arc::clone(&control.cancel);
        let exit = run(
            sh("echo last-words"),
            None,
            &control,
            &mut || {},
            // The stop request arrives while its last line is being read.
            &mut |_| cancel.store(true, Ordering::SeqCst),
        );
        assert!(exit.cancelled);
    }

    #[test]
    fn the_time_limit_stops_an_engine_that_ignores_sigterm() {
        let begun = Instant::now();
        let (exit, lines) = collect(
            sh("trap '' TERM; echo stubborn; sleep 60"),
            None,
            &control(Duration::from_millis(400)),
        );
        assert!(exit.timed_out && !exit.cancelled);
        assert_eq!(lines, ["stubborn"]);
        assert!(begun.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_helper_that_keeps_the_pipe_open_does_not_hold_the_app() {
        let dir = TempDir::new("process-helper");
        let pid_file = dir.path().join("helper.pid");
        let begun = Instant::now();
        let (exit, lines) = collect(
            sh(&format!(
                "(sleep 30 & echo $! > '{}') ; echo done",
                pid_file.display()
            )),
            None,
            &control(Duration::from_secs(60)),
        );
        assert_eq!(exit.code, Some(0));
        assert_eq!(lines, ["done"]);
        assert!(begun.elapsed() < Duration::from_secs(8), "waited for the helper");
        let helper: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_gone(helper), "the helper outlived the engine");
    }

    #[test]
    fn a_program_that_does_not_exist_is_reported() {
        let (exit, lines) = collect(
            Command::new("/nonexistent/dise-engine"),
            None,
            &control(Duration::from_secs(5)),
        );
        assert!(exit.spawn_error.is_some());
        assert_eq!(exit.code, None);
        assert!(lines.is_empty());
    }

    #[test]
    fn only_the_end_of_stderr_is_kept() {
        let (exit, _) = collect(
            sh("i=0; while [ $i -lt 4000 ]; do echo \"error line $i\" >&2; i=$((i+1)); done; echo last-words >&2"),
            None,
            &control(Duration::from_secs(30)),
        );
        assert!(exit.stderr_tail.len() <= STDERR_TAIL);
        assert!(exit.stderr_tail.ends_with("last-words"));
        assert!(!exit.stderr_tail.contains("error line 0\n"));
    }
}
