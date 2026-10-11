//! Test helpers shared by the `kb` unit tests: throwaway directories and git repos.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique directory under the system temp dir, removed on drop.
///
/// The stored path is canonical (on macOS `/var` is a symlink to
/// `/private/var`), which matches what the commands work with.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "dise-test-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(fs::canonicalize(&dir).expect("canonicalize temp dir"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `content` to `rel` (creating parent directories) and returns the full path.
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, content).expect("write test file");
        path
    }

    /// Creates a directory (and parents) at `rel` and returns the full path.
    pub fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.0.join(rel);
        fs::create_dir_all(&path).expect("create test dir");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Creates a symlink at `link` pointing to `target`.
pub fn symlink(target: &Path, link: &Path) {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).expect("create symlink parent");
    }
    std::os::unix::fs::symlink(target, link).expect("create symlink");
}

/// Runs git in `dir` and returns trimmed stdout; panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Turns `dir` into a git repo with a repo-local identity and no signing, so
/// tests never depend on (or prompt for) the user's global git setup.
pub fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "Dise Test"]);
    git(dir, &["config", "user.email", "dise-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Number of commits reachable from HEAD (0 for an empty repo).
pub fn commit_count(dir: &Path) -> usize {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("run git");
    if !out.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// An executable shell script that runs the script named by the environment
/// variable `scenario_var` (with the same arguments), for tests that need a
/// program which behaves differently per test.
///
/// macOS assesses every new executable file the first time it is run, one at
/// a time, which takes long enough to make unrelated tests time out when
/// dozens of tests each write and run a script of their own. So there is one
/// launcher per name, kept under `target/` and left alone when it is already
/// right; what differs per test is a plain file that the launcher sources.
pub fn launcher(name: &str, scenario_var: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/dise-test-bin");
    let path = dir.join(name);
    let script = format!(
        "#!/bin/sh\n# Test helper: runs the scenario a test points {scenario_var} at.\n. \"${scenario_var}\"\n"
    );
    if fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        fs::create_dir_all(&dir).expect("create the launcher folder");
        // Written under another name first: tests run in parallel, and a
        // half-written launcher must never be run.
        let draft = dir.join(format!(".{name}-{}", std::process::id()));
        fs::write(&draft, &script).expect("write the launcher");
        fs::set_permissions(&draft, fs::Permissions::from_mode(0o755)).expect("chmod the launcher");
        fs::rename(&draft, &path).expect("publish the launcher");
    }
    path
}

