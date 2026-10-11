//! When the knowledge base is synced, and what the app knows about the last
//! sync.
//!
//! Syncs run one at a time; every trigger ends up in [`SyncService::sync_now`]:
//!
//! - once after startup, as soon as the login-shell environment is ready;
//! - about [`SyncTiming::after_write`] after a write or a capture (debounced:
//!   a burst of writes leads to one sync);
//! - every [`SyncTiming::period`];
//! - when the main window gains focus, at most once per [`SyncTiming::focus_gap`];
//! - on request: the tray menu, the sync indicator, `--sync` on the command line.
//!
//! A sync holds the knowledge-base lock ([`KbLock`]) for its whole duration.
//! When another operation holds it for longer than [`SyncTiming::lock_wait`],
//! the sync ends as [`SyncState::Busy`] and is tried again later.
//!
//! The service knows no Tauri types: what happens with a new status (events,
//! the tray) is up to the `notify` callback.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::kb::lock::KbLock;
use crate::kb::sync::{self, RepoStatus, SyncReport, SyncState, Timeouts};
use crate::shell_env::ShellEnvState;

/// What the lock holder is called while a sync runs.
pub const SYNC_OPERATION: &str = "同步";

/// The delays of the service (shortened in tests).
#[derive(Debug, Clone, Copy)]
pub struct SyncTiming {
    pub after_write: Duration,
    pub period: Duration,
    pub focus_gap: Duration,
    /// How long a sync waits for another operation to release the lock.
    pub lock_wait: Duration,
    /// When to try again after the lock was busy.
    pub busy_retry: Duration,
}

impl Default for SyncTiming {
    fn default() -> Self {
        SyncTiming {
            after_write: Duration::from_secs(3),
            period: Duration::from_secs(10 * 60),
            focus_gap: Duration::from_secs(60),
            lock_wait: Duration::from_secs(20),
            busy_retry: Duration::from_secs(60),
        }
    }
}

/// The last sync that ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    /// When it ended, in milliseconds since the Unix epoch.
    pub at: u64,
    /// What started it.
    pub reason: String,
    #[serde(flatten)]
    pub report: SyncReport,
}

/// Everything the frontend and the tray show about syncing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// A sync is running right now.
    pub running: bool,
    /// Branch, upstream, ahead/behind and dirtiness (as of the last look).
    pub repo: Option<RepoStatus>,
    pub last: Option<LastSync>,
    /// Another operation holds the knowledge-base lock (what it is doing).
    pub busy: Option<String>,
}

/// What the service tells the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The status changed (a sync started or ended, or the repository moved on).
    Status(Box<SyncStatus>),
    /// A sync changed the files in the working tree.
    FilesChanged,
}

type RootFn = dyn Fn() -> Result<PathBuf, String> + Send + Sync;
type NotifyFn = dyn Fn(Notice) + Send + Sync;

/// A cloneable handle to the one sync service of the app.
#[derive(Clone)]
pub struct SyncService(Arc<Shared>);

struct Shared {
    lock: KbLock,
    env: ShellEnvState,
    timeouts: Timeouts,
    timing: SyncTiming,
    root: Box<RootFn>,
    notify: Box<NotifyFn>,
    state: Mutex<State>,
    finished: Condvar,
    plan: Mutex<Plan>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    running: bool,
    repo: Option<RepoStatus>,
    last: Option<LastSync>,
}

struct Plan {
    /// The next requested sync and why.
    due: Option<(Instant, &'static str)>,
    next_periodic: Instant,
    last_focus: Instant,
    stopped: bool,
}

fn relock<T>(result: Result<T, PoisonError<T>>) -> T {
    result.unwrap_or_else(PoisonError::into_inner)
}

impl SyncService {
    /// `root` resolves the knowledge-base folder (canonical) on every sync;
    /// `notify` is called with every change of the status.
    pub fn new(
        lock: KbLock,
        env: ShellEnvState,
        timeouts: Timeouts,
        timing: SyncTiming,
        root: impl Fn() -> Result<PathBuf, String> + Send + Sync + 'static,
        notify: impl Fn(Notice) + Send + Sync + 'static,
    ) -> Self {
        let now = Instant::now();
        SyncService(Arc::new(Shared {
            lock,
            env,
            timeouts,
            timing,
            root: Box::new(root),
            notify: Box::new(notify),
            state: Mutex::new(State::default()),
            finished: Condvar::new(),
            plan: Mutex::new(Plan {
                due: None,
                next_periodic: now + timing.period,
                // The startup sync counts as the first focus sync.
                last_focus: now,
                stopped: false,
            }),
            wake: Condvar::new(),
        }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        relock(self.0.state.lock())
    }

    fn plan(&self) -> MutexGuard<'_, Plan> {
        relock(self.0.plan.lock())
    }

    /// Starts the background thread that runs the startup, scheduled and
    /// periodic syncs.
    pub fn start(&self) {
        let service = self.clone();
        let spawned = thread::Builder::new()
            .name("kb-sync".into())
            .spawn(move || service.run_schedule());
        if let Err(error) = spawned {
            log::error!("sync: could not start the sync thread ({error}); syncing only on request");
        }
    }

    /// Ends the background thread after the sync it may be running.
    pub fn stop(&self) {
        self.plan().stopped = true;
        self.0.wake.notify_all();
    }

    /// Asks for a sync `delay` from now, unless one is already due sooner.
    pub fn schedule(&self, delay: Duration, reason: &'static str) {
        let at = Instant::now() + delay;
        let mut plan = self.plan();
        if plan.due.is_none_or(|(due, _)| at < due) {
            plan.due = Some((at, reason));
        }
        drop(plan);
        self.0.wake.notify_all();
    }

    /// A write or a capture just committed: sync a moment later. Debounced:
    /// every write moves the sync to shortly after itself, so a burst of
    /// writes leads to a single sync.
    pub fn after_write(&self) {
        let at = Instant::now() + self.0.timing.after_write;
        self.plan().due = Some((at, "write"));
        self.0.wake.notify_all();
    }

    /// The main window gained focus: sync unless that happened recently.
    /// Returns whether a sync was scheduled.
    pub fn on_focus(&self) -> bool {
        let now = Instant::now();
        let mut plan = self.plan();
        if now.duration_since(plan.last_focus) < self.0.timing.focus_gap {
            return false;
        }
        plan.last_focus = now;
        drop(plan);
        self.schedule(Duration::ZERO, "focus");
        true
    }

    fn run_schedule(&self) {
        // The first sync needs git and the proxy settings of the login shell.
        self.0.env.get();
        if self.plan().stopped {
            return;
        }
        self.sync_now("startup");
        while let Some(reason) = self.next_due() {
            self.sync_now(reason);
        }
    }

    /// Waits until the next sync is due and returns why; `None` once stopped.
    fn next_due(&self) -> Option<&'static str> {
        let mut plan = self.plan();
        loop {
            if plan.stopped {
                return None;
            }
            let now = Instant::now();
            if let Some((at, reason)) = plan.due {
                if at <= now {
                    plan.due = None;
                    plan.next_periodic = now + self.0.timing.period;
                    return Some(reason);
                }
            }
            if plan.next_periodic <= now {
                plan.next_periodic = now + self.0.timing.period;
                return Some("periodic");
            }
            let wake_at = match plan.due {
                Some((at, _)) => at.min(plan.next_periodic),
                None => plan.next_periodic,
            };
            plan = relock(self.0.wake.wait_timeout(plan, wake_at - now)).0;
        }
    }

    /// Runs one sync and returns the status afterwards. When another sync is
    /// running, waits for it first: every call gets a sync that started
    /// after the call.
    pub fn sync_now(&self, reason: &str) -> SyncStatus {
        {
            let mut state = self.state();
            while state.running {
                state = relock(self.0.finished.wait(state));
            }
            state.running = true;
        }
        // Clears `running` even if the sync panics, so later syncs never wait forever.
        let running = Running(self);
        self.publish();

        let started = Instant::now();
        let (report, repo) = self.perform();
        let line = format!(
            "sync ({reason}): {:?} pulled={} pushed={} changed={}{} ({} ms)",
            report.state,
            report.pulled,
            report.pushed,
            report.changed,
            report
                .detail
                .as_deref()
                .map(|detail| format!(" detail={:?}", detail.lines().next().unwrap_or("")))
                .unwrap_or_default(),
            started.elapsed().as_millis()
        );
        match report.state {
            SyncState::Synced | SyncState::Offline | SyncState::Busy | SyncState::RemoteMoved => {
                log::info!("{line}")
            }
            _ => log::warn!("{line}"),
        }

        let changed = report.changed;
        let busy = report.state == SyncState::Busy;
        {
            let mut state = self.state();
            state.last = Some(LastSync {
                at: unix_millis(),
                reason: reason.to_string(),
                report,
            });
            if repo.is_some() {
                state.repo = repo;
            }
        }
        drop(running);

        if changed {
            (self.0.notify)(Notice::FilesChanged);
        }
        self.publish();
        if busy {
            self.schedule(self.0.timing.busy_retry, "retry");
        }
        self.snapshot()
    }

    /// The sync itself, under the knowledge-base lock, plus a fresh look at
    /// the repository while the lock is still held.
    fn perform(&self) -> (SyncReport, Option<RepoStatus>) {
        let root = match (self.0.root)() {
            Ok(root) => root,
            Err(error) => return (SyncReport::stopped(SyncState::NotRepo, Some(error)), None),
        };
        let _guard = match self.0.lock.acquire(SYNC_OPERATION, self.0.timing.lock_wait) {
            Ok(guard) => guard,
            Err(busy) => {
                return (
                    SyncReport::stopped(SyncState::Busy, Some(busy.operation)),
                    None,
                )
            }
        };
        let env = &self.0.env.get().env;
        let report = sync::sync(&root, env, &self.0.timeouts);
        let repo = sync::repo_status(&root, env).ok();
        (report, repo)
    }

    /// The current status, with a fresh look at the repository unless a sync
    /// is running (it reports when it is done).
    pub fn status(&self) -> SyncStatus {
        if !self.state().running {
            if let Some(repo) = self.look() {
                let mut state = self.state();
                if !state.running {
                    state.repo = Some(repo);
                }
            }
        }
        self.snapshot()
    }

    /// Looks at the repository again and tells the app (after a write).
    pub fn refresh(&self) {
        if let Some(repo) = self.look() {
            self.state().repo = Some(repo);
        }
        self.publish();
    }

    fn look(&self) -> Option<RepoStatus> {
        let root = (self.0.root)().ok()?;
        sync::repo_status(&root, &self.0.env.get().env).ok()
    }

    /// The status as it is known right now, without running git.
    pub fn snapshot(&self) -> SyncStatus {
        let state = self.state();
        SyncStatus {
            running: state.running,
            repo: state.repo.clone(),
            last: state.last.clone(),
            busy: self
                .0
                .lock
                .held_by()
                .map(|busy| busy.operation)
                .filter(|operation| operation != SYNC_OPERATION),
        }
    }

    fn publish(&self) {
        (self.0.notify)(Notice::Status(Box::new(self.snapshot())));
    }
}

/// Marks the end of a sync when dropped.
struct Running<'a>(&'a SyncService);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.state().running = false;
        self.0 .0.finished.notify_all();
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// The short sync state shown in the tray menu, e.g. `已同步` or `↑2 待推送`.
pub fn tray_label(status: &SyncStatus) -> String {
    if status.running {
        return "正在同步…".into();
    }
    let attention = status.last.as_ref().is_some_and(|last| {
        !matches!(
            last.report.state,
            SyncState::Synced | SyncState::Offline | SyncState::Busy | SyncState::RemoteMoved
        )
    });
    if attention {
        return "同步需要处理".into();
    }
    if status
        .last
        .as_ref()
        .is_some_and(|last| last.report.state == SyncState::Busy)
    {
        return "稍后同步".into();
    }
    let (ahead, behind) = status
        .repo
        .as_ref()
        .map_or((0, 0), |repo| (repo.ahead, repo.behind));
    let offline = status
        .last
        .as_ref()
        .is_some_and(|last| last.report.state == SyncState::Offline);
    match (offline, ahead, behind) {
        (true, 0, _) => "离线".into(),
        (true, ahead, _) => format!("离线 · ↑{ahead} 待推送"),
        (false, ahead, _) if ahead > 0 => format!("↑{ahead} 待推送"),
        (false, _, behind) if behind > 0 => format!("↓{behind} 待拉取"),
        _ if status.last.is_none() => "还没同步".into(),
        _ => "已同步".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{git, TempDir};
    use crate::shell_env::{ChildEnv, Origin, ShellEnv};
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc;

    fn ready_env() -> ShellEnvState {
        ShellEnvState::ready(ShellEnv {
            env: ChildEnv::inherit(),
            origin: Origin::LoginShell,
            took: Duration::ZERO,
        })
    }

    fn quick() -> SyncTiming {
        SyncTiming {
            after_write: Duration::from_millis(50),
            period: Duration::from_secs(3600),
            focus_gap: Duration::from_millis(300),
            lock_wait: Duration::from_millis(100),
            busy_retry: Duration::from_secs(3600),
        }
    }

    fn configure(dir: &Path) {
        git(dir, &["config", "user.name", "Dise Test"]);
        git(dir, &["config", "user.email", "dise-test@example.invalid"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    /// A clone of a bare remote with one commit, plus a second clone.
    struct World {
        _dir: TempDir,
        kb: PathBuf,
        other: PathBuf,
        remote: PathBuf,
    }

    fn world(tag: &str) -> World {
        let dir = TempDir::new(tag);
        let root = dir.path().to_path_buf();
        git(&root, &["init", "--bare", "-q", "-b", "main", "remote.git"]);
        let seed = root.join("seed");
        fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        configure(&seed);
        fs::write(seed.join("index.md"), "# 目录\n").unwrap();
        git(&seed, &["add", "--", "index.md"]);
        git(&seed, &["commit", "-q", "-m", "init"]);
        git(&seed, &["push", "-q", "../remote.git", "main"]);
        for name in ["kb", "other"] {
            git(&root, &["clone", "-q", "remote.git", name]);
            configure(&root.join(name));
        }
        World {
            kb: root.join("kb"),
            other: root.join("other"),
            remote: root.join("remote.git"),
            _dir: dir,
        }
    }

    fn commit(dir: &Path, rel: &str) {
        fs::write(dir.join(rel), format!("{rel}\n")).unwrap();
        git(dir, &["add", "--", rel]);
        git(dir, &["commit", "-q", "-m", rel]);
    }

    fn remote_main(world: &World) -> String {
        git(
            &world.kb,
            &[
                "--git-dir",
                world.remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/main",
            ],
        )
    }

    fn service(world: &World, lock: KbLock) -> (SyncService, mpsc::Receiver<Notice>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let kb = world.kb.clone();
        let service = SyncService::new(
            lock,
            ready_env(),
            Timeouts::default(),
            quick(),
            move || Ok(kb.clone()),
            move |notice| {
                let _ = relock(sender.lock()).send(notice);
            },
        );
        (service, receiver)
    }

    /// Notices until a status that is not running (the end of a sync).
    fn until_finished(notices: &mpsc::Receiver<Notice>) -> (SyncStatus, bool) {
        let mut files_changed = false;
        loop {
            match notices
                .recv_timeout(Duration::from_secs(20))
                .expect("a notice")
            {
                Notice::FilesChanged => files_changed = true,
                Notice::Status(status) if !status.running && status.last.is_some() => {
                    return (*status, files_changed)
                }
                Notice::Status(_) => {}
            }
        }
    }

    #[test]
    fn sync_now_syncs_and_reports_start_and_end() {
        let world = world("service-now");
        commit(&world.kb, "a.md");
        let (service, notices) = service(&world, KbLock::default());

        let status = service.sync_now("manual");
        assert!(!status.running);
        let last = status.last.clone().unwrap();
        assert_eq!(last.report.state, SyncState::Synced);
        assert_eq!(last.report.pushed, 1);
        assert_eq!(last.reason, "manual");
        assert!(last.at > 1_700_000_000_000);
        assert_eq!(
            status.repo.as_ref().map(|repo| (repo.ahead, repo.behind)),
            Some((0, 0))
        );
        assert_eq!(remote_main(&world), git(&world.kb, &["rev-parse", "HEAD"]));

        let first = notices.recv().unwrap();
        assert!(
            matches!(&first, Notice::Status(s) if s.running),
            "{first:?}"
        );
        let (end, files_changed) = until_finished(&notices);
        assert_eq!(end, status);
        assert!(!files_changed, "a push does not change the files");
    }

    #[test]
    fn pulled_changes_are_announced() {
        let world = world("service-pull");
        commit(&world.other, "b.md");
        git(&world.other, &["push", "-q", "origin", "main"]);
        let (service, notices) = service(&world, KbLock::default());
        service.sync_now("manual");
        let (status, files_changed) = until_finished(&notices);
        assert!(files_changed);
        assert_eq!(status.last.unwrap().report.pulled, 1);
        assert!(world.kb.join("b.md").exists());
    }

    #[test]
    fn a_busy_lock_is_reported_and_nothing_is_tried() {
        let world = world("service-busy");
        commit(&world.kb, "a.md");
        let lock = KbLock::default();
        let (service, _notices) = service(&world, lock.clone());
        let before = remote_main(&world);

        let job = lock.try_acquire("入库").unwrap();
        let status = service.sync_now("manual");
        let last = status.last.clone().unwrap();
        assert_eq!(last.report.state, SyncState::Busy);
        assert_eq!(last.report.detail.as_deref(), Some("入库"));
        assert_eq!(status.busy.as_deref(), Some("入库"));
        assert_eq!(remote_main(&world), before);
        assert_eq!(tray_label(&status), "稍后同步");

        drop(job);
        let status = service.sync_now("retry");
        assert_eq!(status.last.unwrap().report.state, SyncState::Synced);
        assert_eq!(status.busy, None);
    }

    #[test]
    fn writes_schedule_one_sync_shortly_after_the_last_one() {
        let world = world("service-debounce");
        let (service, notices) = service(&world, KbLock::default());
        service.start();
        let (startup, _) = until_finished(&notices);
        assert_eq!(startup.last.unwrap().reason, "startup");

        // Three quick writes, one sync. The commits come first: git can take
        // longer than the 50 ms debounce on a busy machine, and the burst is
        // about the calls, not about git.
        for name in ["c1.md", "c2.md", "c3.md"] {
            commit(&world.kb, name);
        }
        for _ in 0..3 {
            service.after_write();
            thread::sleep(Duration::from_millis(10));
        }
        let (after, _) = until_finished(&notices);
        let last = after.last.unwrap();
        assert_eq!(last.reason, "write");
        assert_eq!(last.report.pushed, 3);
        assert!(
            notices.recv_timeout(Duration::from_millis(300)).is_err(),
            "only one sync for the burst"
        );
        service.stop();
    }

    #[test]
    fn requests_keep_the_earliest_time_and_writes_debounce() {
        let world = world("service-plan");
        let (service, _notices) = service(&world, KbLock::default());
        let due = |service: &SyncService| service.plan().due;

        service.schedule(Duration::from_secs(60), "retry");
        let (retry_at, _) = due(&service).unwrap();
        service.schedule(Duration::ZERO, "menu");
        let (menu_at, reason) = due(&service).unwrap();
        assert!(menu_at < retry_at);
        assert_eq!(reason, "menu");
        // A later request does not push an earlier one back.
        service.schedule(Duration::from_secs(60), "retry");
        assert_eq!(due(&service).unwrap().1, "menu");
        // A write does: the sync waits for the writes to settle.
        service.after_write();
        let (write_at, reason) = due(&service).unwrap();
        assert_eq!(reason, "write");
        assert!(write_at > menu_at);
    }

    #[test]
    fn focus_syncs_at_most_once_per_gap() {
        let world = world("service-focus");
        let (service, notices) = service(&world, KbLock::default());
        // Right after start the startup sync covers it.
        assert!(!service.on_focus());
        service.start();
        until_finished(&notices);
        thread::sleep(Duration::from_millis(320));
        assert!(service.on_focus());
        assert!(!service.on_focus(), "too soon");
        let (status, _) = until_finished(&notices);
        assert_eq!(status.last.unwrap().reason, "focus");
        service.stop();
    }

    #[test]
    fn overlapping_requests_run_one_after_the_other() {
        let world = world("service-overlap");
        commit(&world.kb, "a.md");
        let (service, _notices) = service(&world, KbLock::default());
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let service = service.clone();
                thread::spawn(move || service.sync_now("manual"))
            })
            .collect();
        for thread in threads {
            let status = thread.join().unwrap();
            assert_eq!(status.last.unwrap().report.state, SyncState::Synced);
        }
        assert_eq!(remote_main(&world), git(&world.kb, &["rev-parse", "HEAD"]));
    }

    #[test]
    fn status_looks_at_the_repository_again() {
        let world = world("service-status");
        let (service, _notices) = service(&world, KbLock::default());
        assert_eq!(service.snapshot().repo, None);
        commit(&world.kb, "a.md");
        let status = service.status();
        assert_eq!(status.repo.as_ref().unwrap().ahead, 1);
        assert_eq!(status.last, None);
        assert_eq!(tray_label(&status), "↑1 待推送");
    }

    #[test]
    fn a_missing_folder_is_reported_without_running_git() {
        let (sender, _receiver) = mpsc::channel::<Notice>();
        let sender = Mutex::new(sender);
        let service = SyncService::new(
            KbLock::default(),
            ready_env(),
            Timeouts::default(),
            quick(),
            || Err("找不到知识库文件夹".to_string()),
            move |notice| {
                let _ = relock(sender.lock()).send(notice);
            },
        );
        let last = service.sync_now("manual").last.unwrap();
        assert_eq!(last.report.state, SyncState::NotRepo);
        assert_eq!(last.report.detail.as_deref(), Some("找不到知识库文件夹"));
    }

    #[test]
    fn the_tray_label_says_what_matters_most() {
        let report = |state| LastSync {
            at: 1,
            reason: "x".into(),
            report: SyncReport::stopped(state, None),
        };
        let repo = |ahead, behind| RepoStatus {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            ahead,
            behind,
            dirty: false,
        };
        let status = |running, repo: Option<RepoStatus>, last: Option<LastSync>| SyncStatus {
            running,
            repo,
            last,
            busy: None,
        };
        assert_eq!(tray_label(&status(false, None, None)), "还没同步");
        assert_eq!(
            tray_label(&status(true, None, Some(report(SyncState::Conflict)))),
            "正在同步…"
        );
        for state in [
            SyncState::Dirty,
            SyncState::Conflict,
            SyncState::NoUpstream,
            SyncState::NotRepo,
            SyncState::Detached,
            SyncState::InProgress,
            SyncState::Failed,
        ] {
            assert_eq!(
                tray_label(&status(false, Some(repo(1, 0)), Some(report(state)))),
                "同步需要处理",
                "{state:?}"
            );
        }
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(0, 0)),
                Some(report(SyncState::Offline))
            )),
            "离线"
        );
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(2, 0)),
                Some(report(SyncState::Offline))
            )),
            "离线 · ↑2 待推送"
        );
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(3, 1)),
                Some(report(SyncState::Synced))
            )),
            "↑3 待推送"
        );
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(0, 4)),
                Some(report(SyncState::RemoteMoved))
            )),
            "↓4 待拉取"
        );
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(1, 0)),
                Some(report(SyncState::Busy))
            )),
            "稍后同步"
        );
        assert_eq!(
            tray_label(&status(
                false,
                Some(repo(0, 0)),
                Some(report(SyncState::Synced))
            )),
            "已同步"
        );
    }

    #[test]
    fn statuses_serialize_for_the_frontend() {
        let status = SyncStatus {
            running: false,
            repo: None,
            last: Some(LastSync {
                at: 5,
                reason: "startup".into(),
                report: SyncReport::stopped(SyncState::Offline, Some("no network".into())),
            }),
            busy: Some("写入".into()),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "running": false,
                "repo": null,
                "last": {
                    "at": 5,
                    "reason": "startup",
                    "state": "offline",
                    "detail": "no network",
                    "paths": [],
                    "pulled": 0,
                    "pushed": 0,
                    "changed": false,
                    "backup": null
                },
                "busy": "写入"
            })
        );
    }
}
