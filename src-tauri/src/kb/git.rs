//! Running the `git` command-line tool on the knowledge base, and committing
//! single files.
//!
//! Every call runs `git` directly with explicit arguments (never through a
//! shell), inside the knowledge-base root, with the user's login-shell
//! environment (see `crate::shell_env`: Homebrew git, proxy settings), literal
//! pathspecs and without terminal prompts. Every call is bounded: a git that
//! hangs (a credential prompt, a dead network) is stopped together with its
//! helper processes when its time is up. Pushing and pulling live in
//! `super::sync`.

use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::process_group;
use crate::shell_env::ChildEnv;

/// Time limit for git commands that only work on the local repository.
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(60);
/// After SIGTERM, git gets this long to remove its lock files before SIGKILL.
const TERMINATE_GRACE: Duration = Duration::from_secs(2);
/// After git exits, wait at most this long for the rest of its output.
const DRAIN_GRACE: Duration = Duration::from_millis(500);
/// More output than this is cut off (nothing the app runs prints that much).
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// Another git process (an editor, a terminal) may hold the index for a moment.
const INDEX_LOCK_ATTEMPTS: u32 = 6;
const INDEX_LOCK_PAUSE: Duration = Duration::from_millis(150);

/// Result of writing one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOutcome {
    /// `false` when the content was unchanged, so there was nothing to commit.
    pub committed: bool,
    /// Hash of the new commit, if one was made.
    pub commit: Option<String>,
}

/// A `git` invocation rooted at `root`, running with `env` (stdin closed).
pub(crate) fn command(root: &Path, env: &ChildEnv) -> Command {
    let mut command = env.command("git");
    command
        .current_dir(root)
        // Commits must land in the repository at `root`, whatever the shell
        // profile exports.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        // Paths are file names, never pathspec magic like `:(glob)`.
        .env("GIT_LITERAL_PATHSPECS", "1")
        // Fail instead of waiting for credentials on a terminal.
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn spawn_error(error: std::io::Error) -> String {
    format!("无法运行 git（{error}）。请确认已安装 Git（macOS 上可运行 xcode-select --install）")
}

/// A `git` invocation for the app's own reading of git's output: messages in
/// English (a few have to be recognised), file names as they are instead of
/// octal escapes, and nothing that could open an editor or ask for a password.
pub(crate) fn plumbing(root: &Path, env: &ChildEnv) -> Command {
    let mut command = command(root, env);
    command
        .env("LC_ALL", "C")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        // An empty value turns the askpass helper off; credential helpers
        // (the macOS keychain) still work.
        .env("GIT_ASKPASS", "")
        .args(["-c", "core.quotePath=false"]);
    command
}

/// What one finished git command left behind.
#[derive(Debug, Clone, Default)]
pub(crate) struct Finished {
    /// Exit code; `None` when a signal ended the process.
    pub code: Option<i32>,
    /// The time limit passed and the process group was stopped.
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Finished {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// What git said went wrong: stderr, or stdout when stderr is empty.
    pub fn detail(&self) -> String {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            self.stdout.trim().to_string()
        } else {
            stderr.to_string()
        }
    }
}

/// [`Finished`] with the output as the bytes git wrote (file contents).
#[derive(Debug, Clone, Default)]
pub(crate) struct FinishedBytes {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl FinishedBytes {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
}

/// Runs `command` until it exits, or stops it when `timeout` has passed.
///
/// The command gets its own process group, so the helpers git starts for a
/// transfer (`git-remote-https`, `ssh`) are stopped with it: first SIGTERM,
/// which lets git remove its lock files, then SIGKILL.
pub(crate) fn finish(command: Command, timeout: Duration) -> Result<Finished, String> {
    let done = finish_bytes(command, timeout, MAX_OUTPUT)?;
    Ok(Finished {
        code: done.code,
        timed_out: done.timed_out,
        stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
    })
}

/// [`finish`] without decoding the output. Each stream is cut off once it
/// holds `max_output` bytes or more, so a caller that must notice an output
/// that is too large passes its limit plus one.
pub(crate) fn finish_bytes(
    mut command: Command,
    timeout: Duration,
    max_output: usize,
) -> Result<FinishedBytes, String> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(spawn_error)?;

    let (sender, receiver) = mpsc::channel::<(bool, Vec<u8>)>();
    forward(child.stdout.take(), false, sender.clone());
    forward(child.stderr.take(), true, sender);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut keep = |(is_stderr, chunk): (bool, Vec<u8>)| {
        let buffer = if is_stderr { &mut stderr } else { &mut stdout };
        if buffer.len() < max_output {
            buffer.extend_from_slice(&chunk);
        }
    };

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        // Wakes up early for output, and at once when both pipes close
        // (which is what a finished process looks like).
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => keep(chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(2)),
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break process_group::stop(&mut child, TERMINATE_GRACE);
        }
    };

    // Read what is left in the pipes, but never wait for a background helper
    // that inherited them (an ssh control master, a credential cache).
    let drained = Instant::now() + DRAIN_GRACE;
    while let Some(left) = drained.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(chunk) => keep(chunk),
            Err(_) => break,
        }
    }

    Ok(FinishedBytes {
        code: status.and_then(|status| status.code()),
        timed_out,
        stdout,
        stderr,
    })
}

/// Copies a pipe into the channel on its own thread until the pipe closes.
fn forward<P: Read + Send + 'static>(
    pipe: Option<P>,
    is_stderr: bool,
    sender: mpsc::Sender<(bool, Vec<u8>)>,
) {
    let Some(mut pipe) = pipe else { return };
    let _ = thread::Builder::new()
        .name("git-output".into())
        .spawn(move || {
            let mut buffer = [0u8; 8 * 1024];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if sender.send((is_stderr, buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
}

/// Runs git with `args` and returns trimmed stdout, or a readable error.
pub(crate) fn run(root: &Path, env: &ChildEnv, args: &[&str]) -> Result<String, String> {
    let mut git = command(root, env);
    git.args(args);
    let finished = finish(git, LOCAL_TIMEOUT)?;
    if finished.success() {
        return Ok(finished.stdout.trim().to_string());
    }
    let name = args.first().unwrap_or(&"");
    if finished.timed_out {
        return Err(format!(
            "git {name} 超过 {} 秒没有结束，已停止",
            LOCAL_TIMEOUT.as_secs()
        ));
    }
    Err(format!("git {name} 失败：{}", finished.detail()))
}

/// Like [`run`], but tries again for a moment while another git process holds
/// the index lock.
fn run_waiting_for_index(root: &Path, env: &ChildEnv, args: &[&str]) -> Result<String, String> {
    let mut attempt = 1;
    loop {
        match run(root, env, args) {
            Err(error) if error.contains("index.lock") && attempt < INDEX_LOCK_ATTEMPTS => {
                attempt += 1;
                thread::sleep(INDEX_LOCK_PAUSE);
            }
            result => return result,
        }
    }
}

/// Requires `root` (canonical) to be the top level of a git work tree.
///
/// Being merely *inside* some other repository is not enough: commits would
/// then land in that enclosing repository.
pub fn ensure_repo(root: &Path, env: &ChildEnv) -> Result<(), String> {
    let toplevel = run(root, env, &["rev-parse", "--show-toplevel"]).map_err(|error| {
        if error.starts_with("无法运行 git") {
            error
        } else {
            format!("知识库文件夹不是 git 仓库：{}", root.display())
        }
    })?;
    let toplevel = PathBuf::from(toplevel);
    let toplevel = fs::canonicalize(&toplevel).unwrap_or(toplevel);
    if toplevel == root {
        Ok(())
    } else {
        Err(format!(
            "知识库文件夹不是 git 仓库的根目录：{}（所在仓库：{}）",
            root.display(),
            toplevel.display()
        ))
    }
}

/// A commit message must have visible text and no NUL bytes.
pub fn validate_message(message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("提交说明不能为空".to_string());
    }
    if message.contains('\0') {
        return Err("提交说明里不能有空字符".to_string());
    }
    Ok(())
}

/// Stages `rel` and commits only that path with `message`.
///
/// `git commit -- <path>` commits just that path, so anything else the user
/// has staged stays staged and uncommitted. If the file is unchanged, no
/// commit is made and `committed` is `false`.
pub fn commit_path(
    root: &Path,
    rel: &str,
    message: &str,
    env: &ChildEnv,
) -> Result<WriteOutcome, String> {
    validate_message(message)?;
    run_waiting_for_index(root, env, &["add", "--", rel])?;

    let mut diff = command(root, env);
    diff.args(["diff", "--cached", "--quiet", "--no-ext-diff", "--", rel]);
    let diff = finish(diff, LOCAL_TIMEOUT)?;
    match diff.code {
        Some(0) if !diff.timed_out => {
            return Ok(WriteOutcome {
                committed: false,
                commit: None,
            })
        }
        Some(1) if !diff.timed_out => {}
        _ => return Err(format!("git diff 失败：{}", diff.detail())),
    }

    run_waiting_for_index(root, env, &["commit", "--quiet", "-m", message, "--", rel])?;
    let head = run(root, env, &["rev-parse", "HEAD"])?;
    Ok(WriteOutcome {
        committed: true,
        commit: Some(head),
    })
}

/// Takes a path that was staged but never committed out of the index again
/// (the file itself stays where it is).
pub(crate) fn unstage_new_path(root: &Path, rel: &str, env: &ChildEnv) -> Result<(), String> {
    run_waiting_for_index(
        root,
        env,
        &["rm", "--cached", "--quiet", "--ignore-unmatch", "--", rel],
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{commit_count, git as git_out, init_repo, TempDir};
    use std::os::unix::fs::PermissionsExt;

    fn repo(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        init_repo(dir.path());
        dir
    }

    /// The test process environment, which has a working git.
    fn env() -> ChildEnv {
        ChildEnv::inherit()
    }

    #[test]
    fn ensure_repo_accepts_a_repo_root() {
        let kb = repo("git-ensure");
        assert_eq!(ensure_repo(kb.path(), &env()), Ok(()));
    }

    #[test]
    fn ensure_repo_rejects_plain_folders_and_subfolders() {
        let plain = TempDir::new("git-plain");
        let err = ensure_repo(plain.path(), &env()).unwrap_err();
        assert!(err.contains("不是 git 仓库"), "{err}");

        let kb = repo("git-subfolder");
        let sub = kb.mkdir("wiki");
        let err = ensure_repo(&sub, &env()).unwrap_err();
        assert!(err.contains("根目录"), "{err}");
    }

    #[test]
    fn git_runs_with_the_given_environment_only() {
        // A fake `git` first on PATH records the environment it was given.
        let tools = TempDir::new("git-env-tools");
        let record = tools.path().join("env.txt");
        let fake = tools.write(
            "bin/git",
            &format!(
                "#!/bin/sh\n/usr/bin/env > '{}'\necho /fake/top\n",
                record.display()
            ),
        );
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let child_env = ChildEnv::from_vars([
            (
                "PATH",
                format!("{}:/usr/bin:/bin", tools.path().join("bin").display()),
            ),
            ("HTTPS_PROXY", "http://127.0.0.1:9".to_string()),
        ]);

        let kb = TempDir::new("git-env-kb");
        let err = ensure_repo(kb.path(), &child_env).unwrap_err();
        assert!(err.contains("根目录"), "{err}");

        // The fake git was found on the child's PATH and got exactly that
        // environment plus the two git settings; nothing of the app's own
        // environment (HOME, cargo's variables) leaked through.
        let seen = std::fs::read_to_string(&record).unwrap();
        assert!(seen.contains("GIT_TERMINAL_PROMPT=0\n"), "{seen}");
        assert!(seen.contains("GIT_LITERAL_PATHSPECS=1\n"), "{seen}");
        assert!(seen.contains("HTTPS_PROXY=http://127.0.0.1:9\n"), "{seen}");
        for leaked in ["HOME=", "CARGO"] {
            assert!(!seen.contains(leaked), "{leaked} leaked into {seen}");
        }
    }

    #[test]
    fn a_missing_git_is_reported_readably() {
        let kb = repo("git-missing");
        let no_git = ChildEnv::from_vars([("PATH", "/nonexistent")]);
        kb.write("a.md", "a\n");
        let err = commit_path(kb.path(), "a.md", "提交", &no_git).unwrap_err();
        assert!(err.starts_with("无法运行 git"), "{err}");
    }

    #[test]
    fn commits_only_the_given_path() {
        let kb = repo("git-commit");
        kb.write("self/底色.md", "### D01 标题\n");
        kb.write("other.md", "other v1\n");
        git_out(kb.path(), &["add", "--", "self/底色.md", "other.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        // The user has an unrelated change staged.
        kb.write("other.md", "other v2\n");
        git_out(kb.path(), &["add", "--", "other.md"]);

        kb.write("self/底色.md", "### D01 标题 ✅\n");
        let outcome = commit_path(kb.path(), "self/底色.md", "底色：确认 D01", &env()).unwrap();

        assert!(outcome.committed);
        let head = git_out(kb.path(), &["rev-parse", "HEAD"]);
        assert_eq!(outcome.commit.as_deref(), Some(head.as_str()));
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(
            git_out(kb.path(), &["log", "-1", "--format=%s"]),
            "底色：确认 D01"
        );
        assert_eq!(
            git_out(
                kb.path(),
                &[
                    "-c",
                    "core.quotepath=off",
                    "show",
                    "--name-only",
                    "--format=",
                    "HEAD"
                ]
            ),
            "self/底色.md"
        );
        // The unrelated staged change is still staged, not committed.
        assert_eq!(
            git_out(kb.path(), &["diff", "--cached", "--name-only"]),
            "other.md"
        );
    }

    #[test]
    fn unchanged_content_makes_no_commit() {
        let kb = repo("git-unchanged");
        kb.write("a.md", "same\n");
        git_out(kb.path(), &["add", "--", "a.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        let outcome = commit_path(kb.path(), "a.md", "不会提交", &env()).unwrap();
        assert_eq!(
            outcome,
            WriteOutcome {
                committed: false,
                commit: None
            }
        );
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn new_files_in_new_folders_are_added_and_committed() {
        let kb = repo("git-new-file");
        kb.write("index.md", "# 目录\n");
        git_out(kb.path(), &["add", "--", "index.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        kb.write("inbox/裁决/2026-10-12-回答.md", "回答\n");
        let outcome = commit_path(
            kb.path(),
            "inbox/裁决/2026-10-12-回答.md",
            "收件箱：新增回答",
            &env(),
        )
        .unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "");
    }

    #[test]
    fn works_in_a_repo_without_commits() {
        let kb = repo("git-empty");
        kb.write("first.md", "first\n");
        let outcome = commit_path(kb.path(), "first.md", "第一次提交", &env()).unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn pathspec_magic_is_taken_literally() {
        let kb = repo("git-literal");
        kb.write("keep.md", "keep\n");
        kb.write(":(glob)x.md", "odd name\n");
        let outcome = commit_path(kb.path(), ":(glob)x.md", "奇怪的文件名", &env()).unwrap();
        assert!(outcome.committed);
        assert_eq!(
            git_out(kb.path(), &["show", "--name-only", "--format=", "HEAD"]),
            ":(glob)x.md"
        );
        // keep.md was not swept into the commit.
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "?? keep.md");
    }

    #[test]
    fn rejects_empty_messages_and_reports_git_errors() {
        let kb = repo("git-errors");
        kb.write("a.md", "a\n");
        assert_eq!(
            commit_path(kb.path(), "a.md", "  \n", &env()),
            Err("提交说明不能为空".to_string())
        );
        assert!(commit_path(kb.path(), "a.md", "a\0b", &env()).is_err());
        assert_eq!(commit_count(kb.path()), 0);

        let err = commit_path(kb.path(), "missing.md", "缺文件", &env()).unwrap_err();
        assert!(err.starts_with("git add 失败"), "{err}");
    }

    /// A command that runs `script` with /bin/sh, like git with a hook would.
    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script).stdin(Stdio::null());
        command
    }

    #[test]
    fn finish_collects_output_and_the_exit_code() {
        let done = finish(sh("echo out; echo err >&2; exit 3"), LOCAL_TIMEOUT).unwrap();
        assert_eq!(done.code, Some(3));
        assert!(!done.timed_out);
        assert!(!done.success());
        assert_eq!(done.stdout, "out\n");
        assert_eq!(done.stderr, "err\n");
        assert_eq!(done.detail(), "err");

        let quiet = finish(sh("echo only-stdout; exit 1"), LOCAL_TIMEOUT).unwrap();
        assert_eq!(quiet.detail(), "only-stdout");
        assert!(finish(sh("true"), LOCAL_TIMEOUT).unwrap().success());
    }

    #[test]
    fn finish_handles_more_output_than_a_pipe_holds() {
        // 300 000 lines on each stream: far more than the 64 KiB pipe buffer.
        let done = finish(
            sh("i=0; while [ $i -lt 3000 ]; do printf '%0100d\n' 7; printf '%0100d\n' 8 >&2; i=$((i+1)); done"),
            LOCAL_TIMEOUT,
        )
        .unwrap();
        assert!(done.success());
        assert_eq!(done.stdout.lines().count(), 3000);
        assert_eq!(done.stderr.lines().count(), 3000);
    }

    #[test]
    fn finish_stops_the_whole_process_group_at_the_time_limit() {
        let dir = TempDir::new("git-timeout");
        let pid_file = dir.path().join("helper.pid");
        // The helper stands in for git-remote-https or ssh.
        let started = Instant::now();
        let done = finish(
            sh(&format!(
                "sleep 30 & echo $! > '{}'; echo started; wait",
                pid_file.display()
            )),
            Duration::from_millis(400),
        )
        .unwrap();
        assert!(done.timed_out);
        assert!(!done.success());
        assert_eq!(done.stdout, "started\n");
        assert!(started.elapsed() < Duration::from_secs(10));

        let helper: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        // SAFETY: signal 0 only checks whether the process still exists.
        while unsafe { libc::kill(helper, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(helper, 0) }, 0, "the helper survived");
    }

    #[test]
    fn finish_does_not_wait_for_a_helper_that_keeps_the_pipes_open() {
        // The background job inherits stdout and outlives the command, like a
        // credential cache daemon would.
        let started = Instant::now();
        let done = finish(sh("(sleep 5 &) ; echo done"), LOCAL_TIMEOUT).unwrap();
        assert!(done.success());
        assert_eq!(done.stdout, "done\n");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "waited for the helper"
        );
    }

    #[test]
    fn a_timed_out_git_command_is_reported_readably() {
        let tools = TempDir::new("git-hang-tools");
        let fake = tools.write("bin/git", "#!/bin/sh\nexec sleep 30\n");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hanging = ChildEnv::from_vars([(
            "PATH",
            format!("{}:/usr/bin:/bin", tools.path().join("bin").display()),
        )]);
        let kb = TempDir::new("git-hang-kb");
        let mut git = command(kb.path(), &hanging);
        git.arg("status");
        let done = finish(git, Duration::from_millis(300)).unwrap();
        assert!(done.timed_out);
    }

    #[test]
    fn waits_for_another_git_process_to_release_the_index() {
        let kb = repo("git-index-lock");
        kb.write("a.md", "a\n");
        let lock = kb.path().join(".git/index.lock");
        std::fs::write(&lock, "").unwrap();
        let releaser = {
            let lock = lock.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(250));
                std::fs::remove_file(lock).unwrap();
            })
        };
        let outcome = commit_path(kb.path(), "a.md", "等一下索引锁", &env()).unwrap();
        assert!(outcome.committed);
        releaser.join().unwrap();

        // A lock that never goes away is an error, not an endless wait.
        std::fs::write(&lock, "").unwrap();
        kb.write("b.md", "b\n");
        let err = commit_path(kb.path(), "b.md", "锁一直在", &env()).unwrap_err();
        assert!(err.contains("index.lock"), "{err}");
        std::fs::remove_file(&lock).unwrap();
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn unstages_a_new_path_without_touching_the_file() {
        let kb = repo("git-unstage");
        kb.write("keep.md", "keep\n");
        git_out(kb.path(), &["add", "--", "keep.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        kb.write("inbox/new.md", "new\n");
        git_out(kb.path(), &["add", "--", "inbox/new.md"]);
        unstage_new_path(kb.path(), "inbox/new.md", &env()).unwrap();
        assert_eq!(git_out(kb.path(), &["diff", "--cached", "--name-only"]), "");
        assert!(kb.path().join("inbox/new.md").exists());
        // Nothing staged for that path: still fine.
        unstage_new_path(kb.path(), "inbox/new.md", &env()).unwrap();
        unstage_new_path(kb.path(), "inbox/never-existed.md", &env()).unwrap();
    }

    #[test]
    fn outcome_serializes_as_camel_case_json() {
        let outcome = WriteOutcome {
            committed: true,
            commit: Some("abc".into()),
        };
        assert_eq!(
            serde_json::to_string(&outcome).unwrap(),
            r#"{"committed":true,"commit":"abc"}"#
        );
    }
}
