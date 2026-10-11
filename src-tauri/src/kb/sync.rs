//! Keeping the knowledge base in step with its git remote.
//!
//! **Data safety beats freshness.** A sync only ever does things git itself
//! considers safe, and stops with a clear state at the first doubt:
//!
//! 1. Nothing happens unless the folder is a repository root on a branch that
//!    has an upstream, with no rebase, merge, cherry-pick, revert or bisect
//!    in progress.
//! 2. `git fetch` from the upstream's remote, under a hard time limit (plus
//!    low-speed limits for HTTP). A failure means [`SyncState::Offline`].
//! 3. Count the commits ahead of and behind the upstream.
//! 4. Only behind: `git merge --ff-only`. If git refuses because local files
//!    would be overwritten, the state is [`SyncState::Dirty`].
//! 5. Diverged: with modified or staged tracked files the state is `Dirty`
//!    and nothing is tried. Otherwise HEAD is first saved under
//!    `refs/dise/backup/<unix seconds>`, then `git rebase` runs; on any
//!    failure the rebase is aborted, HEAD is checked against the backup and
//!    the state is [`SyncState::Conflict`].
//! 6. Ahead: `git push <remote> HEAD:<upstream branch>` under a time limit.
//!    If the remote got new commits in the meantime, steps 2–6 run once more.
//! 7. Only the newest [`KEEP_BACKUPS`] backup refs are kept.
//!
//! It never force-pushes, resets, stashes, cleans, checks out or restores
//! files, and never touches untracked files. The caller holds the
//! knowledge-base lock (`super::lock`) for the whole sync.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::git::{self, Finished};
use crate::shell_env::ChildEnv;

/// Where the backups of HEAD taken before a rebase live.
pub const BACKUP_PREFIX: &str = "refs/dise/backup/";
/// How many of them are kept.
pub const KEEP_BACKUPS: usize = 10;
/// Longest list of files reported for one state.
const MAX_PATHS: usize = 20;
/// Longest piece of git output kept as the detail of a state.
const MAX_DETAIL: usize = 400;
/// HTTP transfers slower than this many bytes per second for
/// [`LOW_SPEED_SECONDS`] are given up by git itself.
const LOW_SPEED_LIMIT: &str = "http.lowSpeedLimit=1000";
const LOW_SPEED_SECONDS: &str = "http.lowSpeedTime=20";

/// Time limits of one sync.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub fetch: Duration,
    pub push: Duration,
    /// For commands that only work on the local repository.
    pub local: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            fetch: Duration::from_secs(30),
            push: Duration::from_secs(30),
            local: git::LOCAL_TIMEOUT,
        }
    }
}

/// How a sync ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// Local and remote are the same.
    Synced,
    /// The remote could not be reached (network, proxy, credentials).
    Offline,
    /// Another operation holds the knowledge-base lock; nothing was tried.
    Busy,
    /// The remote kept getting new commits while pushing; the next sync retries.
    RemoteMoved,
    /// Local changes to tracked files (or files in the way) block the update.
    Dirty,
    /// Local and remote commits do not rebase cleanly; the rebase was undone.
    Conflict,
    /// The branch has no upstream branch on a remote.
    NoUpstream,
    /// The folder is not the root of a git repository.
    NotRepo,
    /// HEAD is not on a branch.
    Detached,
    /// A rebase, merge, cherry-pick, revert or bisect is in progress.
    InProgress,
    /// Something unexpected; see the detail.
    Failed,
}

/// Where the repository stands, without contacting the remote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// The current branch; `None` for a detached HEAD.
    pub branch: Option<String>,
    /// The upstream as git names it (`origin/main`); `None` without one.
    pub upstream: Option<String>,
    /// Commits the upstream does not have yet (as of the last fetch).
    pub ahead: u32,
    /// Commits of the upstream that are not here yet (as of the last fetch).
    pub behind: u32,
    /// Tracked files are modified or staged.
    pub dirty: bool,
}

/// What one sync did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub state: SyncState,
    /// More about the state: git's own words, or the operation in progress.
    pub detail: Option<String>,
    /// The files concerned: conflicting ones, or the ones in the way.
    pub paths: Vec<String>,
    /// Commits that arrived from the remote.
    pub pulled: u32,
    /// Commits that were sent to the remote.
    pub pushed: u32,
    /// The working tree changed (so the frontend reloads).
    pub changed: bool,
    /// The backup ref taken before a rebase, if there was one.
    pub backup: Option<String>,
}

impl SyncReport {
    /// A report for a sync that did nothing and ended in `state`.
    pub fn stopped(state: SyncState, detail: Option<String>) -> Self {
        SyncReport {
            state,
            detail,
            paths: Vec::new(),
            pulled: 0,
            pushed: 0,
            changed: false,
            backup: None,
        }
    }
}

/// Why a sync stopped before the end.
struct Stop {
    state: SyncState,
    detail: Option<String>,
    paths: Vec<String>,
}

impl Stop {
    fn new(state: SyncState, detail: impl Into<String>) -> Self {
        Stop {
            state,
            detail: Some(shorten(&detail.into())),
            paths: Vec::new(),
        }
    }

    fn state(state: SyncState) -> Self {
        Stop {
            state,
            detail: None,
            paths: Vec::new(),
        }
    }

    fn with_paths(mut self, mut paths: Vec<String>) -> Self {
        paths.truncate(MAX_PATHS);
        self.paths = paths;
        self
    }
}

/// Unexpected failures of git itself (it cannot be started, …).
impl From<String> for Stop {
    fn from(detail: String) -> Self {
        Stop::new(SyncState::Failed, detail)
    }
}

/// The remote branch the current branch follows.
struct Upstream {
    remote: String,
    /// The branch on the remote, as a full ref (`refs/heads/main`).
    merge_ref: String,
}

enum Push {
    Done,
    /// The remote has commits this repository has not seen yet.
    NotFastForward(String),
}

/// Runs one sync of the repository at `root` (canonical). See the module
/// documentation for the steps.
pub fn sync(root: &Path, env: &ChildEnv, timeouts: &Timeouts) -> SyncReport {
    sync_with(root, env, timeouts, &mut |_| {})
}

/// [`sync`], calling `before_push` with the round number (1 or 2) right
/// before each push. Tests use it to move the remote at exactly that moment.
pub(crate) fn sync_with(
    root: &Path,
    env: &ChildEnv,
    timeouts: &Timeouts,
    before_push: &mut dyn FnMut(u32),
) -> SyncReport {
    let repo = Repo {
        root,
        env,
        timeouts,
    };
    let mut report = SyncReport::stopped(SyncState::Synced, None);
    let outcome = match repo.preflight() {
        Ok(upstream) => {
            let outcome = repo.rounds(&upstream, &mut report, before_push);
            // Also after a failed round: an earlier sync may have left more.
            if let Err(error) = repo.prune_backups(KEEP_BACKUPS) {
                log::warn!("sync: could not prune the backup refs: {error}");
            }
            outcome
        }
        Err(stop) => Err(stop),
    };
    if let Err(stop) = outcome {
        report.state = stop.state;
        report.detail = stop.detail;
        report.paths = stop.paths;
    }
    report
}

/// Reads branch, upstream, ahead/behind and dirtiness. Local and read-only:
/// it does not contact the remote and takes no lock in the repository.
pub fn repo_status(root: &Path, env: &ChildEnv) -> Result<RepoStatus, String> {
    let timeouts = Timeouts::default();
    let repo = Repo {
        root,
        env,
        timeouts: &timeouts,
    };
    git::ensure_repo(root, env)?;
    let upstream = repo
        .read(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])
        .ok()
        .filter(|name| !name.is_empty());
    let (ahead, behind) = match upstream {
        Some(_) => repo.ahead_behind().unwrap_or((0, 0)),
        None => (0, 0),
    };
    Ok(RepoStatus {
        branch: repo.branch()?,
        upstream,
        ahead,
        behind,
        dirty: !repo.tracked_changes()?.is_empty(),
    })
}

/// Deletes all but the `keep` newest backup refs of the repository at `root`.
#[cfg(test)]
pub fn prune_backups(root: &Path, env: &ChildEnv, keep: usize) -> Result<usize, String> {
    let timeouts = Timeouts::default();
    Repo {
        root,
        env,
        timeouts: &timeouts,
    }
    .prune_backups(keep)
}

struct Repo<'a> {
    root: &'a Path,
    env: &'a ChildEnv,
    timeouts: &'a Timeouts,
}

impl Repo<'_> {
    /// Runs git with `args` under `timeout`. git's messages are requested in
    /// English, because a few of them have to be recognised; nothing may open
    /// an editor or ask for a password in a dialog.
    fn run(&self, args: &[&str], timeout: Duration) -> Result<Finished, String> {
        let mut command = git::command(self.root, self.env);
        command
            .env("LC_ALL", "C")
            .env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true")
            .env("GIT_MERGE_AUTOEDIT", "no")
            // An empty value turns the askpass helper off; credential helpers
            // (the macOS keychain) still work.
            .env("GIT_ASKPASS", "")
            // File names as they are, not as octal escapes.
            .args(["-c", "core.quotePath=false"])
            .args(args);
        git::finish(command, timeout)
    }

    /// Runs a local git command and returns its trimmed stdout.
    fn read(&self, args: &[&str]) -> Result<String, String> {
        let done = self.run(args, self.timeouts.local)?;
        if done.success() {
            Ok(done.stdout.trim().to_string())
        } else if done.timed_out {
            Err(format!("git {} 超时", args.first().unwrap_or(&"")))
        } else {
            Err(done.detail())
        }
    }

    /// True when the local git command exits with 0.
    fn succeeds(&self, args: &[&str]) -> bool {
        self.run(args, self.timeouts.local)
            .is_ok_and(|done| done.success())
    }

    /// Steps 2–6, at most twice.
    fn rounds(
        &self,
        upstream: &Upstream,
        report: &mut SyncReport,
        before_push: &mut dyn FnMut(u32),
    ) -> Result<(), Stop> {
        for round in 1..=2 {
            self.fetch(upstream)?;
            let (ahead, behind) = self.diverged_by()?;
            if behind > 0 && ahead == 0 {
                self.fast_forward()?;
            } else if behind > 0 {
                self.rebase(report)?;
            }
            if behind > 0 {
                report.changed = true;
                report.pulled += behind;
            }

            let (ahead, _) = self.diverged_by()?;
            if ahead == 0 {
                return Ok(());
            }
            before_push(round);
            match self.push(upstream)? {
                Push::Done => {
                    report.pushed += ahead;
                    return Ok(());
                }
                Push::NotFastForward(detail) if round == 2 => {
                    return Err(Stop::new(SyncState::RemoteMoved, detail));
                }
                // The remote moved between the fetch and the push: once more.
                Push::NotFastForward(_) => {}
            }
        }
        Ok(())
    }

    /// Step 1: is this a repository in a state that may be synced at all?
    fn preflight(&self) -> Result<Upstream, Stop> {
        git::ensure_repo(self.root, self.env)
            .map_err(|detail| Stop::new(SyncState::NotRepo, detail))?;
        if let Some(operation) = self.operation_in_progress()? {
            return Err(Stop::new(SyncState::InProgress, operation));
        }
        let Some(branch) = self.branch()? else {
            return Err(Stop::state(SyncState::Detached));
        };
        let setting = |key: &str| {
            self.read(&["config", "--get", &format!("branch.{branch}.{key}")])
                .ok()
                .filter(|value| !value.is_empty())
        };
        match (setting("remote"), setting("merge")) {
            // A remote of "." is another local branch: nothing to sync with.
            // A name starting with "-" would be read as an option.
            (Some(remote), Some(merge_ref))
                if remote != "."
                    && !remote.starts_with('-')
                    && merge_ref.starts_with("refs/heads/") =>
            {
                Ok(Upstream { remote, merge_ref })
            }
            _ => Err(Stop::new(SyncState::NoUpstream, branch)),
        }
    }

    /// The current branch, or `None` for a detached HEAD.
    fn branch(&self) -> Result<Option<String>, String> {
        let done = self.run(
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            self.timeouts.local,
        )?;
        Ok(done
            .success()
            .then(|| done.stdout.trim().to_string())
            .filter(|name| !name.is_empty()))
    }

    /// The name of an unfinished rebase, merge, … if there is one.
    fn operation_in_progress(&self) -> Result<Option<&'static str>, String> {
        const MARKERS: &[(&str, &str)] = &[
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
            ("BISECT_LOG", "bisect"),
        ];
        let mut args = vec!["rev-parse"];
        for (marker, _) in MARKERS {
            args.extend(["--git-path", marker]);
        }
        let printed = self.read(&args)?;
        Ok(printed
            .lines()
            .zip(MARKERS)
            .find(|(path, _)| self.resolve(path).exists())
            .map(|(_, (_, operation))| *operation))
    }

    /// git prints paths relative to the repository root.
    fn resolve(&self, printed: &str) -> PathBuf {
        let path = Path::new(printed);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// Step 2. Fetches only the upstream branch (git then also updates its
    /// remote-tracking ref); tags and other branches are left alone, and
    /// nothing is pruned.
    fn fetch(&self, upstream: &Upstream) -> Result<(), Stop> {
        let done = self.run(
            &[
                // No housekeeping while the clock is running.
                "-c",
                "maintenance.auto=false",
                "-c",
                LOW_SPEED_LIMIT,
                "-c",
                LOW_SPEED_SECONDS,
                "fetch",
                "--quiet",
                "--no-tags",
                &upstream.remote,
                &upstream.merge_ref,
            ],
            self.timeouts.fetch,
        )?;
        if done.success() {
            return Ok(());
        }
        if done.timed_out {
            return Err(Stop::new(
                SyncState::Offline,
                format!(
                    "git fetch 超过 {} 秒没有结束",
                    self.timeouts.fetch.as_secs()
                ),
            ));
        }
        let detail = done.detail();
        if detail.contains("couldn't find remote ref") {
            // The remote answered, but the branch is gone (or never existed).
            Err(Stop::new(SyncState::NoUpstream, detail))
        } else {
            Err(Stop::new(SyncState::Offline, detail))
        }
    }

    /// Step 3: (commits only here, commits only on the upstream).
    fn ahead_behind(&self) -> Result<(u32, u32), String> {
        let counts = self.read(&["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])?;
        let mut numbers = counts.split_whitespace().map(str::parse::<u32>);
        match (numbers.next(), numbers.next()) {
            (Some(Ok(ahead)), Some(Ok(behind))) => Ok((ahead, behind)),
            _ => Err(format!("git rev-list 的输出看不懂：{counts}")),
        }
    }

    fn diverged_by(&self) -> Result<(u32, u32), Stop> {
        self.ahead_behind().map_err(|detail| {
            if self.succeeds(&["rev-parse", "--quiet", "--verify", "@{upstream}"]) {
                Stop::new(SyncState::Failed, detail)
            } else {
                // The upstream is configured but the remote has no such branch.
                Stop::new(SyncState::NoUpstream, detail)
            }
        })
    }

    /// Step 4. git refuses by itself when a local file would be overwritten.
    fn fast_forward(&self) -> Result<(), Stop> {
        let done = self.run(
            &["merge", "--ff-only", "--quiet", "@{upstream}"],
            self.timeouts.local,
        )?;
        if done.success() {
            return Ok(());
        }
        let detail = done.detail();
        if detail.contains("would be overwritten") {
            Err(Stop::new(SyncState::Dirty, &detail).with_paths(listed_paths(&detail)))
        } else {
            Err(Stop::new(SyncState::Failed, detail))
        }
    }

    /// Step 5.
    fn rebase(&self, report: &mut SyncReport) -> Result<(), Stop> {
        let changes = self.tracked_changes()?;
        if !changes.is_empty() {
            return Err(Stop::state(SyncState::Dirty).with_paths(changes));
        }
        let head = self.read(&["rev-parse", "--verify", "HEAD^{commit}"])?;
        let backup = self.back_up(&head)?;
        report.backup.get_or_insert_with(|| backup.clone());

        let done = self.run(
            &[
                // Whatever the user configured: no stash, no squashing, no
                // moving of other branches.
                "-c",
                "rebase.autoStash=false",
                "-c",
                "rebase.autoSquash=false",
                "-c",
                "rebase.updateRefs=false",
                "rebase",
                "--no-autostash",
                "@{upstream}",
            ],
            self.timeouts.local,
        )?;
        if done.success() {
            return Ok(());
        }

        let detail = if done.timed_out {
            "git rebase 超时".to_string()
        } else {
            rebase_failure(&done)
        };
        let mut paths = self.unmerged_paths();
        if paths.is_empty() {
            paths = listed_paths(&detail);
        }
        let aborted = !self.rebase_in_progress()
            || self
                .run(&["rebase", "--abort"], self.timeouts.local)
                .is_ok_and(|abort| abort.success());
        let restored = self
            .read(&["rev-parse", "--verify", "HEAD^{commit}"])
            .is_ok_and(|now| now == head);
        if aborted && restored && !self.rebase_in_progress() {
            // git refuses to start when files it would have to write are in
            // the way (untracked ones, for example): that is not a conflict.
            let state = if detail.contains("would be overwritten") {
                SyncState::Dirty
            } else {
                SyncState::Conflict
            };
            Err(Stop::new(state, detail).with_paths(paths))
        } else {
            Err(Stop::new(
                SyncState::Failed,
                format!("没能把变基撤回干净。同步前的提交保存在 {backup}。git 的输出：{detail}"),
            )
            .with_paths(paths))
        }
    }

    /// Saves `head` under a new backup ref and returns the ref's name.
    ///
    /// The name is the current Unix time in seconds, or one more than the
    /// newest backup when that is not later (two rebases within a second), so
    /// names never collide and sort by age.
    fn back_up(&self, head: &str) -> Result<String, Stop> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let newest = self
            .backups()?
            .first()
            .map(|(seconds, _)| *seconds)
            .unwrap_or(0);
        let name = format!("{BACKUP_PREFIX}{}", now.max(newest + 1));
        // The empty old value means: only if the ref does not exist yet.
        let done = self.run(
            &[
                "update-ref",
                "-m",
                "dise: backup before rebase",
                &name,
                head,
                "",
            ],
            self.timeouts.local,
        )?;
        if done.success() {
            Ok(name)
        } else {
            Err(Stop::new(
                SyncState::Failed,
                format!("没能保存同步前的备份，所以没有继续：{}", done.detail()),
            ))
        }
    }

    /// The backup refs, newest first; names that are not a number count as
    /// the oldest.
    fn backups(&self) -> Result<Vec<(u64, String)>, String> {
        let listed = self.read(&[
            "for-each-ref",
            "--format=%(refname)",
            BACKUP_PREFIX.trim_end_matches('/'),
        ])?;
        let mut refs: Vec<(u64, String)> = listed
            .lines()
            .filter_map(|name| {
                let suffix = name.strip_prefix(BACKUP_PREFIX)?;
                Some((suffix.parse::<u64>().unwrap_or(0), name.to_string()))
            })
            .collect();
        refs.sort_by(|a, b| b.cmp(a));
        Ok(refs)
    }

    fn rebase_in_progress(&self) -> bool {
        matches!(self.operation_in_progress(), Ok(Some("rebase")) | Err(_))
    }

    /// Tracked files that are modified or staged (untracked files do not count).
    fn tracked_changes(&self) -> Result<Vec<String>, String> {
        let done = self.run(
            &[
                // Do not refresh the index on disk just for a look.
                "--no-optional-locks",
                "status",
                "--porcelain",
                "-z",
                "--untracked-files=no",
            ],
            self.timeouts.local,
        )?;
        if !done.success() {
            return Err(format!("git status 失败：{}", done.detail()));
        }
        // Entries are `XY <path>`, NUL-terminated; a rename is followed by its
        // old path as a separate entry.
        let mut paths = Vec::new();
        let mut entries = done.stdout.split('\0');
        while let Some(entry) = entries.next() {
            let Some((status, path)) = entry.split_at_checked(3) else {
                continue;
            };
            if status.contains(['R', 'C']) {
                entries.next();
            }
            paths.push(path.to_string());
        }
        Ok(paths)
    }

    fn unmerged_paths(&self) -> Vec<String> {
        self.run(
            &["diff", "--name-only", "--diff-filter=U", "-z"],
            self.timeouts.local,
        )
        .map(|done| {
            done.stdout
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
    }

    /// Step 6. Never forced: the remote only accepts a fast-forward.
    fn push(&self, upstream: &Upstream) -> Result<Push, Stop> {
        let refspec = format!("HEAD:{}", upstream.merge_ref);
        let done = self.run(
            &[
                "-c",
                LOW_SPEED_LIMIT,
                "-c",
                LOW_SPEED_SECONDS,
                "push",
                "--porcelain",
                &upstream.remote,
                &refspec,
            ],
            self.timeouts.push,
        )?;
        if done.success() {
            return Ok(Push::Done);
        }
        if done.timed_out {
            return Err(Stop::new(
                SyncState::Offline,
                format!("git push 超过 {} 秒没有结束", self.timeouts.push.as_secs()),
            ));
        }
        // `--porcelain` prints one line per ref: flag, refs, summary.
        let rejected = done.stdout.lines().find_map(|line| {
            let mut fields = line.split('\t');
            (fields.next() == Some("!")).then(|| fields.nth(1).unwrap_or_default().to_string())
        });
        match rejected {
            Some(summary)
                if summary.contains("non-fast-forward") || summary.contains("fetch first") =>
            {
                Ok(Push::NotFastForward(summary))
            }
            // The remote said no for a reason of its own (a hook, permissions).
            Some(summary) => Err(Stop::new(
                SyncState::Failed,
                format!("远端拒绝了推送：{summary}"),
            )),
            // No answer about the ref at all: the remote was not reached.
            None => Err(Stop::new(SyncState::Offline, done.detail())),
        }
    }

    /// Step 7.
    fn prune_backups(&self, keep: usize) -> Result<usize, String> {
        let mut removed = 0;
        for (_, name) in self.backups()?.into_iter().skip(keep) {
            self.read(&["update-ref", "-d", &name])?;
            removed += 1;
        }
        Ok(removed)
    }
}

/// What went wrong in a rebase, in git's words: the `CONFLICT` lines when
/// there are any, else the error without git's hints and progress output.
fn rebase_failure(done: &Finished) -> String {
    let lines = || {
        done.stdout
            .lines()
            .chain(done.stderr.lines())
            .flat_map(|line| line.split('\r'))
            .map(str::trim)
    };
    let conflicts: Vec<&str> = lines()
        .filter(|line| line.starts_with("CONFLICT"))
        .collect();
    if !conflicts.is_empty() {
        return conflicts.join("\n");
    }
    let errors: Vec<&str> = lines()
        .filter(|line| {
            !line.is_empty() && !line.starts_with("hint:") && !line.starts_with("Rebasing")
        })
        .collect();
    if errors.is_empty() {
        done.detail()
    } else {
        errors.join("\n")
    }
}

/// The file names git lists (one per line, after a tab) when it refuses to
/// overwrite local files.
fn listed_paths(detail: &str) -> Vec<String> {
    detail
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect()
}

/// Keeps a detail to a few lines' worth of text.
fn shorten(detail: &str) -> String {
    let detail = detail.trim();
    if detail.chars().count() <= MAX_DETAIL {
        detail.to_string()
    } else {
        let cut: String = detail.chars().take(MAX_DETAIL).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests;
