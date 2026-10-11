//! The user's login-shell environment, used for every child process the app
//! starts (`git` today; other command-line tools later).
//!
//! An app launched from Finder inherits launchd's bare environment: `PATH` is
//! `/usr/bin:/bin:/usr/sbin:/sbin` and nothing exported by the shell profile is
//! there (proxy variables, Homebrew and npm-global tools, …). So once at
//! startup, off the main thread, the app runs the user's login shell
//! (`$SHELL -ilc …`, stdin closed, in its own session, killed after
//! [`CAPTURE_TIMEOUT`]) and keeps the variables that `/usr/bin/env -0` prints
//! between two unique markers. Anything the shell profile prints before or
//! after is ignored.
//!
//! When that fails, a fallback is built from the process environment, a fixed
//! `PATH` (see [`fallback_path`]) and the system proxy settings from
//! `scutil --proxy`.
//!
//! Variable values are never logged: only how many there are, whether the
//! login shell or the fallback provided them, and whether a proxy is set.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long the login shell may take before it is killed.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// Used when `$SHELL` is unset or not an absolute path (the macOS default).
const DEFAULT_SHELL: &str = "/bin/zsh";
/// More output than this means something is wrong with the shell profile.
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const BEGIN: &str = "__DISE_ENV_BEGIN_";
const END: &str = "__DISE_ENV_END_";
/// Shell bookkeeping that must not leak into child processes.
const SHELL_BOOKKEEPING: &[&str] = &["_", "SHLVL", "PWD", "OLDPWD"];
/// Variables that route network traffic through a proxy.
const PROXY_KEYS: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// The environment child processes run with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildEnv {
    vars: BTreeMap<OsString, OsString>,
}

impl ChildEnv {
    pub fn from_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        ChildEnv {
            vars: vars
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    /// The environment of the current process.
    pub fn inherit() -> Self {
        Self::from_vars(std::env::vars_os())
    }

    /// Number of variables.
    pub fn count(&self) -> usize {
        self.vars.len()
    }

    pub fn get(&self, key: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    pub fn set(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) {
        self.vars.insert(key.into(), value.into());
    }

    /// True when any proxy variable has a non-empty value.
    pub fn has_proxy(&self) -> bool {
        PROXY_KEYS
            .iter()
            .any(|key| self.get(key).is_some_and(|value| !value.is_empty()))
    }

    /// A command for `program` that runs with exactly this environment, with
    /// stdin closed and no shell in between. `program` is looked up on this
    /// environment's `PATH` (not the app's); if it is not found there, the
    /// bare name is kept and spawning reports the usual "not found" error.
    ///
    /// This is the one way the app starts external tools, so they all see the
    /// user's proxy settings and Homebrew / npm-global tools.
    pub fn command(&self, program: &str) -> Command {
        let resolved = self
            .find_program(program)
            .unwrap_or_else(|| PathBuf::from(program));
        let mut command = Command::new(resolved);
        command.env_clear().envs(&self.vars).stdin(Stdio::null());
        command
    }

    /// Where `program` would be found on this environment's `PATH`.
    pub fn find_program(&self, program: &str) -> Option<PathBuf> {
        if program.contains('/') {
            let path = PathBuf::from(program);
            return is_executable(&path).then_some(path);
        }
        let path = self.get("PATH")?;
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    }
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Where the environment came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    LoginShell,
    Fallback { reason: String },
}

/// The environment for child processes, with where it came from.
#[derive(Debug, Clone)]
pub struct ShellEnv {
    pub env: ChildEnv,
    pub origin: Origin,
    pub took: Duration,
}

impl ShellEnv {
    /// One log line about the environment, without any variable values.
    pub fn summary(&self) -> String {
        let proxy = if self.env.has_proxy() { "yes" } else { "no" };
        let git = if self.env.find_program("git").is_some() {
            "found"
        } else {
            "missing"
        };
        match &self.origin {
            Origin::LoginShell => format!(
                "shell env: captured from the login shell in {} ms (vars={}, proxy={proxy}, git={git})",
                self.took.as_millis(),
                self.env.count()
            ),
            Origin::Fallback { reason } => format!(
                "shell env: using the fallback, {reason} (vars={}, proxy={proxy}, git={git})",
                self.env.count()
            ),
        }
    }
}

/// App state: the environment, captured once and shared by every command.
#[derive(Clone, Default)]
pub struct ShellEnvState(Arc<OnceLock<ShellEnv>>);

impl ShellEnvState {
    /// A state that is already filled in (tests).
    #[cfg(test)]
    pub fn ready(env: ShellEnv) -> Self {
        let state = Self::default();
        let _ = state.0.set(env);
        state
    }

    /// Starts the capture on a background thread (call once at startup).
    pub fn warm_up(&self) {
        let state = self.clone();
        let spawned = thread::Builder::new()
            .name("shell-env".into())
            .spawn(move || {
                state.get();
            });
        if let Err(error) = spawned {
            log::warn!(
                "shell env: could not start the capture thread ({error}); capturing on first use"
            );
        }
    }

    /// The environment. The first caller captures it; callers that arrive
    /// while a capture runs wait for it (at most about [`CAPTURE_TIMEOUT`]).
    pub fn get(&self) -> &ShellEnv {
        self.0.get_or_init(load_user_env)
    }

    /// A command for an external tool (`git`, `claude`, `codex`, `node`, …)
    /// that runs with the user's environment; see [`ChildEnv::command`].
    /// Blocks while the environment is still being captured, so call it from
    /// a background thread.
    pub fn command(&self, program: &str) -> Command {
        self.get().env.command(program)
    }
}

/// Captures the login-shell environment, or builds the fallback. Logs the outcome.
fn load_user_env() -> ShellEnv {
    let started = Instant::now();
    let shell = login_shell(std::env::var_os("SHELL"));
    let shell_env = match capture_login_env(&shell, CAPTURE_TIMEOUT) {
        Ok(vars) => ShellEnv {
            env: ChildEnv::from_vars(vars),
            origin: Origin::LoginShell,
            took: started.elapsed(),
        },
        Err(error) => {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            ShellEnv {
                env: fallback_env(std::env::vars_os(), home.as_deref(), &system_proxy()),
                origin: Origin::Fallback {
                    reason: error.to_string(),
                },
                took: started.elapsed(),
            }
        }
    };
    log::info!("{}", shell_env.summary());
    shell_env
}

/// `$SHELL` when it is an absolute path, else the macOS default shell.
pub fn login_shell(shell: Option<OsString>) -> PathBuf {
    shell
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SHELL))
}

/// Why the login-shell environment could not be captured.
#[derive(Debug)]
pub enum CaptureError {
    Spawn(io::Error),
    Timeout(Duration),
    /// Output ended without both markers (the shell failed or exited early).
    NoMarkers {
        status: Option<i32>,
    },
    TooMuchOutput,
    /// The variables came through but without a `PATH`.
    NoPath,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::Spawn(error) => {
                write!(f, "the login shell could not be started ({error})")
            }
            CaptureError::Timeout(after) => {
                write!(f, "the login shell timed out after {} s", after.as_secs())
            }
            CaptureError::NoMarkers { status: Some(code) } => write!(
                f,
                "the login shell exited with status {code} without printing its environment"
            ),
            CaptureError::NoMarkers { status: None } => {
                write!(f, "the login shell ended without printing its environment")
            }
            CaptureError::TooMuchOutput => write!(f, "the login shell printed too much output"),
            CaptureError::NoPath => write!(f, "the login shell environment has no PATH"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Runs `shell -ilc` with a script that prints the environment between two
/// unique markers, and returns the variables (shell bookkeeping removed).
///
/// The shell runs in its own session (no controlling terminal, like a Finder
/// launch), with stdin and stderr closed. If the markers have not arrived
/// within `timeout`, the whole session is killed.
pub fn capture_login_env(
    shell: &Path,
    timeout: Duration,
) -> Result<Vec<(OsString, OsString)>, CaptureError> {
    let nonce = format!(
        "{}_{}__",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    // The markers are printed in two pieces, so a shell that echoes its
    // command line never prints a whole marker before the real one.
    let script =
        format!("printf '%s' '{BEGIN}' '{nonce}'; /usr/bin/env -0; printf '%s' '{END}' '{nonce}'");
    let begin = format!("{BEGIN}{nonce}");
    let end = format!("{END}{nonce}");

    let mut command = Command::new(shell);
    command
        .arg("-ilc")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe; nothing else runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(CaptureError::Spawn)?;

    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    if let Some(mut stdout) = child.stdout.take() {
        // Detached: if a background job of the profile keeps the pipe open,
        // this thread waits for it without holding anything else up.
        let _ = thread::Builder::new()
            .name("shell-env-read".into())
            .spawn(move || {
                let mut buffer = [0u8; 16 * 1024];
                loop {
                    match stdout.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sender.send(buffer[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
    }

    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            kill_session(&mut child);
            return Err(CaptureError::Timeout(timeout));
        }
        match receiver.recv_timeout(deadline - now) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                if output.len() > MAX_OUTPUT {
                    kill_session(&mut child);
                    return Err(CaptureError::TooMuchOutput);
                }
                if let Some(vars) = parse_env_block(&output, begin.as_bytes(), end.as_bytes()) {
                    // Everything needed has arrived; do not wait for whatever
                    // the profile runs afterwards.
                    reap(&mut child, Duration::from_millis(500));
                    let vars = without_bookkeeping(vars);
                    return if vars.iter().any(|(key, _)| key == "PATH") {
                        Ok(vars)
                    } else {
                        Err(CaptureError::NoPath)
                    };
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // Output ended without both markers.
                let status = reap(&mut child, Duration::from_millis(500));
                return Err(CaptureError::NoMarkers { status });
            }
        }
    }
}

/// Waits briefly for the shell to exit (it normally has) and returns its exit
/// code; a shell that is still running is killed with its whole session.
fn reap(child: &mut Child, grace: Duration) -> Option<i32> {
    let until = Instant::now() + grace;
    while Instant::now() < until {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Err(_) => return None,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    }
    kill_session(child);
    None
}

/// Kills the shell and everything it started in its session, then reaps it.
fn kill_session(child: &mut Child) {
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // The shell called setsid, so its process group id is its pid.
        // SAFETY: plain kill(2) on a process group we created.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The `KEY=VALUE` entries that `env -0` printed between `begin` and `end`,
/// or None while the end marker has not arrived yet.
pub fn parse_env_block(
    output: &[u8],
    begin: &[u8],
    end: &[u8],
) -> Option<Vec<(OsString, OsString)>> {
    let start = find(output, begin, 0)? + begin.len();
    let stop = find(output, end, start)?;
    Some(
        output[start..stop]
            .split(|&byte| byte == 0)
            .filter_map(|entry| {
                let eq = entry.iter().position(|&byte| byte == b'=')?;
                (eq > 0).then(|| {
                    (
                        OsString::from_vec(entry[..eq].to_vec()),
                        OsString::from_vec(entry[eq + 1..].to_vec()),
                    )
                })
            })
            .collect(),
    )
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + from)
}

fn without_bookkeeping(vars: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(key, _)| !SHELL_BOOKKEEPING.iter().any(|skip| key == skip))
        .collect()
}

/// The fallback `PATH`: Homebrew, /usr/local, npm-global and ~/.local tools first.
pub fn fallback_path(home: Option<&Path>) -> OsString {
    let mut dirs: Vec<PathBuf> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    if let Some(home) = home {
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    std::env::join_paths(dirs).unwrap_or_else(|_| OsString::from("/usr/bin:/bin:/usr/sbin:/sbin"))
}

/// The process environment with the fallback `PATH` and the system proxy
/// (proxy variables that are already set win).
pub fn fallback_env<I>(base: I, home: Option<&Path>, proxy: &SystemProxy) -> ChildEnv
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut env = ChildEnv::from_vars(without_bookkeeping(base.into_iter().collect()));
    env.set("PATH", fallback_path(home));
    for (key, value) in proxy.variables() {
        if env.get(key).is_none_or(|current| current.is_empty()) {
            env.set(key, value);
        }
    }
    env
}

/// HTTP(S) proxy settings of the system (System Settings › Network › Proxies).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemProxy {
    pub http: Option<String>,
    pub https: Option<String>,
    pub exceptions: Vec<String>,
}

impl SystemProxy {
    /// The variables curl, git and most command-line tools understand.
    pub fn variables(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();
        if let Some(url) = &self.http {
            vars.push(("HTTP_PROXY", url.clone()));
            vars.push(("http_proxy", url.clone()));
        }
        if let Some(url) = &self.https {
            vars.push(("HTTPS_PROXY", url.clone()));
            vars.push(("https_proxy", url.clone()));
        }
        if !vars.is_empty() && !self.exceptions.is_empty() {
            // scutil writes `*.local`; curl and git expect `.local`.
            let list = self
                .exceptions
                .iter()
                .map(|entry| entry.strip_prefix('*').unwrap_or(entry))
                .collect::<Vec<_>>()
                .join(",");
            vars.push(("NO_PROXY", list.clone()));
            vars.push(("no_proxy", list));
        }
        vars
    }
}

/// Parses the output of `scutil --proxy`.
pub fn parse_scutil_proxy(text: &str) -> SystemProxy {
    let mut fields = HashMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines() {
        let line = line.trim();
        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
            } else if let Some((_, value)) = line.split_once(" : ") {
                exceptions.push(value.trim().to_string());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(" : ") {
            let (key, value) = (key.trim(), value.trim());
            if key == "ExceptionsList" && value.starts_with("<array>") {
                in_exceptions = true;
            } else {
                fields.insert(key.to_string(), value.to_string());
            }
        }
    }
    let proxy = |enable: &str, host: &str, port: &str| -> Option<String> {
        if fields.get(enable).map(String::as_str) != Some("1") {
            return None;
        }
        let host = fields
            .get(host)
            .map(|h| h.trim())
            .filter(|h| !h.is_empty())?;
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        Some(match fields.get(port).map(String::as_str) {
            Some(port) if !port.is_empty() && port != "0" => format!("http://{host}:{port}"),
            _ => format!("http://{host}"),
        })
    };
    SystemProxy {
        http: proxy("HTTPEnable", "HTTPProxy", "HTTPPort"),
        https: proxy("HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
        exceptions,
    }
}

/// The system proxy settings, or none when `scutil` cannot tell.
fn system_proxy() -> SystemProxy {
    Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_scutil_proxy(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use std::fs;

    fn os(text: &str) -> OsString {
        OsString::from(text)
    }

    /// An executable script standing in for the user's shell.
    fn fake_shell(dir: &TempDir, body: &str) -> PathBuf {
        let path = dir.write("fake-shell", &format!("#!/bin/sh\n{body}\n"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn parses_only_the_block_between_the_markers() {
        let mut output = b"Last login: noise\nBEGIN-ish\0junk=1\0".to_vec();
        output.extend_from_slice(
            b"<B>A=1\0PATH=/x:/y\0MULTI=line one\nline two\0EQ=a=b\0=bad\0novalue\0<E>",
        );
        output.extend_from_slice(b"after=noise\0");
        let vars = parse_env_block(&output, b"<B>", b"<E>").unwrap();
        assert_eq!(
            vars,
            vec![
                (os("A"), os("1")),
                (os("PATH"), os("/x:/y")),
                (os("MULTI"), os("line one\nline two")),
                (os("EQ"), os("a=b")),
            ]
        );
        assert_eq!(parse_env_block(b"<B>A=1\0", b"<B>", b"<E>"), None);
        assert_eq!(parse_env_block(b"noise only", b"<B>", b"<E>"), None);
    }

    #[test]
    fn captures_the_environment_and_ignores_profile_noise() {
        let dir = TempDir::new("shell-env-ok");
        let shell = fake_shell(
            &dir,
            "echo 'Last login: Mon Oct 12 on ttys001'\n\
             printf 'noise with a NUL \\0 byte\\n'\n\
             export DISE_TEST_VALUE='spaces = and\nnewlines'\n\
             export PATH=\"/opt/homebrew/bin:$PATH\"\n\
             eval \"$2\"\n\
             echo 'noise after the script'",
        );
        let vars = capture_login_env(&shell, Duration::from_secs(10)).unwrap();
        let env = ChildEnv::from_vars(vars);
        assert_eq!(
            env.get("DISE_TEST_VALUE"),
            Some(OsStr::new("spaces = and\nnewlines"))
        );
        assert!(env
            .get("PATH")
            .unwrap()
            .to_string_lossy()
            .starts_with("/opt/homebrew/bin:"));
        for key in SHELL_BOOKKEEPING {
            assert_eq!(env.get(key), None, "{key}");
        }
    }

    #[test]
    fn times_out_and_kills_the_whole_session() {
        let dir = TempDir::new("shell-env-timeout");
        let pid_file = dir.path().join("grandchild.pid");
        // A background job keeps stdout open, then the shell itself hangs.
        let shell = fake_shell(
            &dir,
            &format!("sleep 30 &\necho $! > '{}'\nsleep 30", pid_file.display()),
        );
        let started = Instant::now();
        let error = capture_login_env(&shell, Duration::from_millis(1500)).unwrap_err();
        assert!(matches!(error, CaptureError::Timeout(_)), "{error}");
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(
            error
                .to_string()
                .starts_with("the login shell timed out after"),
            "{error}"
        );

        let grandchild: libc::pid_t = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: signal 0 only checks whether the process still exists.
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(grandchild, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(
            unsafe { libc::kill(grandchild, 0) },
            0,
            "background job survived"
        );
    }

    #[test]
    fn reports_a_shell_that_fails_or_cannot_start() {
        let dir = TempDir::new("shell-env-fail");
        let shell = fake_shell(&dir, "echo 'profile error' >&2\nexit 3");
        match capture_login_env(&shell, Duration::from_secs(5)) {
            Err(CaptureError::NoMarkers { status }) => assert_eq!(status, Some(3)),
            other => panic!("unexpected {other:?}"),
        }
        let missing = dir.path().join("no-such-shell");
        assert!(matches!(
            capture_login_env(&missing, Duration::from_secs(1)),
            Err(CaptureError::Spawn(_))
        ));
    }

    #[test]
    fn requires_a_path_in_the_captured_environment() {
        let dir = TempDir::new("shell-env-nopath");
        let shell = fake_shell(&dir, "unset PATH\nexport A=1\neval \"$2\"");
        // Without PATH, /usr/bin/env still runs (the script uses its full path),
        // but printf must be a builtin.
        assert!(matches!(
            capture_login_env(&shell, Duration::from_secs(5)),
            Err(CaptureError::NoPath)
        ));
    }

    #[test]
    fn picks_the_login_shell() {
        assert_eq!(
            login_shell(Some(os("/bin/bash"))),
            PathBuf::from("/bin/bash")
        );
        assert_eq!(login_shell(Some(os("bash"))), PathBuf::from(DEFAULT_SHELL));
        assert_eq!(login_shell(None), PathBuf::from(DEFAULT_SHELL));
    }

    const SCUTIL: &str = "<dictionary> {\n  ExceptionsList : <array> {\n    0 : *.local\n    1 : 169.254/16\n    2 : localhost\n  }\n  FTPPassive : 1\n  HTTPEnable : 1\n  HTTPPort : 7890\n  HTTPProxy : 127.0.0.1\n  HTTPSEnable : 1\n  HTTPSPort : 7891\n  HTTPSProxy : 127.0.0.1\n  SOCKSEnable : 0\n}\n";

    #[test]
    fn parses_scutil_proxy_settings() {
        assert_eq!(
            parse_scutil_proxy(SCUTIL),
            SystemProxy {
                http: Some("http://127.0.0.1:7890".into()),
                https: Some("http://127.0.0.1:7891".into()),
                exceptions: vec!["*.local".into(), "169.254/16".into(), "localhost".into()],
            }
        );
        let disabled = SCUTIL.replace("HTTPEnable : 1", "HTTPEnable : 0");
        assert_eq!(parse_scutil_proxy(&disabled).http, None);
        assert_eq!(
            parse_scutil_proxy("<dictionary> {\n}\n"),
            SystemProxy::default()
        );
        let ipv6 = "HTTPSEnable : 1\nHTTPSProxy : ::1\nHTTPSPort : 8080\n";
        assert_eq!(
            parse_scutil_proxy(ipv6).https.as_deref(),
            Some("http://[::1]:8080")
        );
    }

    #[test]
    fn turns_the_system_proxy_into_variables() {
        let vars = parse_scutil_proxy(SCUTIL).variables();
        let get = |key: &str| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("HTTPS_PROXY"), Some("http://127.0.0.1:7891"));
        assert_eq!(get("https_proxy"), Some("http://127.0.0.1:7891"));
        assert_eq!(get("HTTP_PROXY"), Some("http://127.0.0.1:7890"));
        assert_eq!(get("NO_PROXY"), Some(".local,169.254/16,localhost"));
        assert!(SystemProxy::default().variables().is_empty());
    }

    #[test]
    fn builds_the_fallback_environment() {
        let base = vec![
            (os("HOME"), os("/Users/someone")),
            (os("PATH"), os("/usr/bin:/bin")),
            (os("SHLVL"), os("1")),
            (os("HTTPS_PROXY"), os("http://already.set:1")),
        ];
        let env = fallback_env(
            base,
            Some(Path::new("/Users/someone")),
            &parse_scutil_proxy(SCUTIL),
        );
        assert_eq!(
            env.get("PATH"),
            Some(OsStr::new(
                "/opt/homebrew/bin:/usr/local/bin:/Users/someone/.npm-global/bin:/Users/someone/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
            ))
        );
        assert_eq!(env.get("HOME"), Some(OsStr::new("/Users/someone")));
        assert_eq!(env.get("SHLVL"), None);
        // An explicitly set proxy wins over the system setting.
        assert_eq!(
            env.get("HTTPS_PROXY"),
            Some(OsStr::new("http://already.set:1"))
        );
        assert_eq!(
            env.get("HTTP_PROXY"),
            Some(OsStr::new("http://127.0.0.1:7890"))
        );
        assert!(env.has_proxy());

        let bare = fallback_env(Vec::new(), None, &SystemProxy::default());
        assert_eq!(
            bare.get("PATH"),
            Some(OsStr::new(
                "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
            ))
        );
        assert!(!bare.has_proxy());
    }

    #[test]
    fn commands_run_with_exactly_this_environment() {
        let env = ChildEnv::from_vars([("PATH", "/usr/bin:/bin"), ("DISE_ONLY_THIS", "yes")]);
        let output = env.command("env").output().unwrap();
        let printed = String::from_utf8_lossy(&output.stdout);
        let mut lines: Vec<&str> = printed.lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["DISE_ONLY_THIS=yes", "PATH=/usr/bin:/bin"]);
    }

    #[test]
    fn finds_programs_on_its_own_path() {
        let dir = TempDir::new("shell-env-find");
        let tool = dir.write("bin/dise-tool", "#!/bin/sh\n");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        dir.write("bin/not-executable", "");
        let env = ChildEnv::from_vars([(
            os("PATH"),
            std::env::join_paths([dir.path().join("bin"), PathBuf::from("/usr/bin")]).unwrap(),
        )]);
        assert_eq!(env.find_program("dise-tool"), Some(tool));
        assert_eq!(env.find_program("not-executable"), None);
        assert_eq!(env.find_program("env"), Some(PathBuf::from("/usr/bin/env")));
        assert_eq!(ChildEnv::default().find_program("env"), None);
    }

    #[test]
    fn the_summary_never_contains_values() {
        let env = ChildEnv::from_vars([
            ("PATH", "/usr/bin:/bin"),
            ("HTTPS_PROXY", "http://secret-proxy.example:3128"),
            ("API_TOKEN", "s3cr3t-value"),
        ]);
        let captured = ShellEnv {
            env: env.clone(),
            origin: Origin::LoginShell,
            took: Duration::from_millis(812),
        };
        let line = captured.summary();
        assert_eq!(
            line,
            "shell env: captured from the login shell in 812 ms (vars=3, proxy=yes, git=found)"
        );
        let fallback = ShellEnv {
            env,
            origin: Origin::Fallback {
                reason: CaptureError::Timeout(CAPTURE_TIMEOUT).to_string(),
            },
            took: CAPTURE_TIMEOUT,
        };
        let line = fallback.summary();
        assert!(
            line.contains("using the fallback, the login shell timed out after 10 s"),
            "{line}"
        );
        for secret in ["secret-proxy", "s3cr3t", "/usr/bin"] {
            assert!(!line.contains(secret), "{line}");
            assert!(!captured.summary().contains(secret));
        }
    }

    #[test]
    fn shares_one_environment_between_callers() {
        let state = ShellEnvState::ready(ShellEnv {
            env: ChildEnv::from_vars([("PATH", "/usr/bin:/bin"), ("DISE_SHARED", "1")]),
            origin: Origin::LoginShell,
            took: Duration::ZERO,
        });
        let clone = state.clone();
        assert!(std::ptr::eq(state.get(), clone.get()));
        let output = clone.command("env").output().unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("DISE_SHARED=1\n"));
    }
}
