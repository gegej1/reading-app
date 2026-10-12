//! The iPhone bridge: iPhone and Mac meet in a folder in iCloud Drive.
//!
//! The phone has no VPN, cannot reach GitHub and has no Obsidian; it only
//! has Shortcuts and Files. So:
//!
//! - the phone saves captures and answers as text files into the bridge
//!   folder's `收件/` (through three shortcuts), and the Mac imports them into
//!   the knowledge base (`import`), deleting each file only after its commit;
//! - the Mac exports the 底色 layer (底色, 判例, open rulings, audits, the
//!   latest ingest report) as phone-sized HTML pages and short text files
//!   into `阅读/` (`export`; the pages are built by the main window).
//!   Nothing from `raw/`, `wiki/`, `systems/`, `inbox/` and the rest leaves
//!   the Mac this way.
//!
//! [`BridgeService`] runs it: on startup, when files appear in `收件/`
//! (FSEvents), every 30 seconds, and when the main window gains focus.
//! Where the folder is and whether the bridge is on: see `folder`.

pub mod export;
pub mod folder;
pub mod import;
pub mod ledger;
pub mod shortcuts;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use serde::Serialize;

use crate::kb::capture::LocalTime;
use crate::kb::lock::KbLock;
use crate::run::Task;
use crate::shell_env::ShellEnvState;
use folder::{BridgeSetting, DirSource, Layout};
use import::{Kind, Outcome};

/// When the service looks at `收件/`.
#[derive(Debug, Clone, Copy)]
pub struct BridgeTiming {
    /// Even without any file-system event.
    pub poll: Duration,
    /// Again this soon while a file is still being written or downloaded.
    pub settle_retry: Duration,
    /// After a file-system event (iCloud writes a file in several steps).
    pub event_delay: Duration,
    /// Focusing the main window looks again at most this often.
    pub focus_gap: Duration,
    /// Files younger than this are still being written.
    pub settle: Duration,
}

impl Default for BridgeTiming {
    fn default() -> Self {
        BridgeTiming {
            poll: Duration::from_secs(30),
            settle_retry: Duration::from_secs(4),
            event_delay: Duration::from_millis(1500),
            focus_gap: Duration::from_secs(10),
            settle: import::SETTLE,
        }
    }
}

type RootFn = dyn Fn() -> Result<PathBuf, String> + Send + Sync;
type SettingFn = dyn Fn() -> BridgeSetting + Send + Sync;
type RunningFn = dyn Fn() -> Option<Task> + Send + Sync;
type ImportedFn = dyn Fn(Vec<String>) + Send + Sync;
type ReadyFn = dyn Fn() + Send + Sync;
type SignFn = dyn Fn(&str, &Path) -> Result<(), String> + Send + Sync;
type DownloadFn = dyn Fn(&Path) + Send + Sync;

/// How the service reaches the rest of the app.
pub struct Hooks {
    /// The knowledge base (canonical).
    pub kb_root: Box<RootFn>,
    /// The bridge settings, read again on every (re)configuration.
    pub setting: Box<SettingFn>,
    /// The app's own folder (the import record, the shortcut record).
    pub app_dir: Option<PathBuf>,
    /// The AI run going on, if any.
    pub running: Box<RunningFn>,
    /// These knowledge-base files were just committed.
    pub imported: Box<ImportedFn>,
    /// The folder is ready: time to export.
    pub ready: Box<ReadyFn>,
    /// Signs one shortcut (see `shortcuts::sign_with_system`).
    pub sign: Box<SignFn>,
    /// Asks iCloud to download a placeholder.
    pub download: Box<DownloadFn>,
}

/// What the settings screen shows about the bridge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub enabled: bool,
    /// The folder, when it could be worked out.
    pub path: Option<String>,
    pub source: Option<DirSource>,
    /// The folder is there and watched.
    pub ready: bool,
    /// Why it is not ready.
    pub problem: Option<String>,
    /// The last import, e.g. `10:32 收进 1 条记一笔`.
    pub last_import: Option<String>,
    /// How the shortcuts are.
    pub shortcuts: Option<String>,
    /// Files waiting in `收件/` (busy knowledge base, 落裁决, still syncing).
    pub waiting: usize,
}

/// What an export did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    /// False when the bridge is off or its folder is not ready: nothing was written.
    pub exported: bool,
    #[serde(flatten)]
    pub written: export::Written,
}

#[derive(Clone)]
pub struct BridgeService(Arc<Shared>);

struct Shared {
    hooks: Hooks,
    lock: KbLock,
    env: ShellEnvState,
    timing: BridgeTiming,
    signal: Arc<Signal>,
    state: Mutex<State>,
    exporting: Mutex<()>,
}

/// Wakes the worker; shared with the file-system watcher.
struct Signal {
    plan: Mutex<Plan>,
    wake: Condvar,
}

struct Plan {
    due: Option<Instant>,
    reconfigure: bool,
    stopped: bool,
    last_focus: Option<Instant>,
}

#[derive(Default)]
struct State {
    status: BridgeStatus,
    layout: Option<Layout>,
    watcher: Option<notify::RecommendedWatcher>,
    shortcuts_pending: bool,
    /// What was last logged per incoming file, so a waiting file is logged
    /// once and not every 30 seconds.
    reported: HashMap<String, String>,
    /// When a placeholder was last asked to download.
    downloads: HashMap<PathBuf, Instant>,
}

fn relock<T>(result: Result<T, PoisonError<T>>) -> T {
    result.unwrap_or_else(PoisonError::into_inner)
}

impl Signal {
    fn schedule(&self, delay: Duration) {
        let at = Instant::now() + delay;
        let mut plan = relock(self.plan.lock());
        if plan.due.is_none_or(|due| at < due) {
            plan.due = Some(at);
        }
        drop(plan);
        self.wake.notify_all();
    }
}

/// `~/…` for a path in the home folder, so the log shows where without the user name.
fn home_relative(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    }
}

impl BridgeService {
    pub fn new(lock: KbLock, env: ShellEnvState, timing: BridgeTiming, hooks: Hooks) -> Self {
        BridgeService(Arc::new(Shared {
            hooks,
            lock,
            env,
            timing,
            signal: Arc::new(Signal {
                plan: Mutex::new(Plan {
                    due: None,
                    reconfigure: true,
                    stopped: false,
                    last_focus: None,
                }),
                wake: Condvar::new(),
            }),
            state: Mutex::new(State::default()),
            exporting: Mutex::new(()),
        }))
    }

    /// Starts the worker thread: set up the folder, import, keep watching.
    pub fn start(&self) {
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("iphone-bridge".into())
            .spawn(move || service.run());
        if let Err(error) = spawned {
            log::error!("iphone bridge: could not start ({error})");
        }
    }

    /// Stops the worker and the watcher.
    pub fn stop(&self) {
        relock(self.0.signal.plan.lock()).stopped = true;
        self.0.signal.wake.notify_all();
        self.state().watcher = None;
    }

    /// The settings changed: read them again.
    pub fn reconfigure(&self) {
        relock(self.0.signal.plan.lock()).reconfigure = true;
        self.0.signal.schedule(Duration::ZERO);
    }

    /// Look at `收件/` now.
    pub fn import_now(&self) {
        self.0.signal.schedule(Duration::ZERO);
    }

    /// The main window gained focus: look at `收件/` unless that happened recently.
    pub fn on_focus(&self) {
        let now = Instant::now();
        let mut plan = relock(self.0.signal.plan.lock());
        if plan
            .last_focus
            .is_some_and(|last| now.duration_since(last) < self.0.timing.focus_gap)
        {
            return;
        }
        plan.last_focus = Some(now);
        drop(plan);
        self.0.signal.schedule(Duration::ZERO);
    }

    pub fn status(&self) -> BridgeStatus {
        self.state().status.clone()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        relock(self.0.state.lock())
    }

    /// Writes the exported pages into `阅读/` (nothing when the bridge is off
    /// or its folder is not ready).
    pub fn export(&self, files: &[export::ExportFile]) -> Result<ExportOutcome, String> {
        let _one_at_a_time = relock(self.0.exporting.lock());
        let layout = {
            let state = self.state();
            if !state.status.enabled {
                None
            } else {
                state.layout.clone()
            }
        };
        let Some(layout) = layout else {
            return Ok(ExportOutcome::default());
        };
        let written = export::write_all(&layout.reading(), files)?;
        if written.written > 0 || written.removed > 0 {
            log::info!(
                "iphone export: written={} unchanged={} removed={}",
                written.written,
                written.unchanged,
                written.removed
            );
        }
        Ok(ExportOutcome {
            exported: true,
            written,
        })
    }

    fn run(&self) {
        // Imports commit with git: wait for the login-shell environment.
        self.0.env.get();
        loop {
            let reconfigure = {
                let mut plan = relock(self.0.signal.plan.lock());
                if plan.stopped {
                    return;
                }
                std::mem::take(&mut plan.reconfigure)
            };
            if reconfigure {
                self.configure();
            }
            self.import_pass();
            self.install_shortcuts();
            if !self.wait() {
                return;
            }
        }
    }

    /// Waits for the next pass; false once stopped.
    fn wait(&self) -> bool {
        let signal = &self.0.signal;
        let mut plan = relock(signal.plan.lock());
        let poll_at = Instant::now() + self.0.timing.poll;
        loop {
            if plan.stopped {
                return false;
            }
            let now = Instant::now();
            let due = plan.due.unwrap_or(poll_at).min(poll_at);
            if due <= now || plan.reconfigure {
                plan.due = None;
                return true;
            }
            plan = relock(signal.wake.wait_timeout(plan, due - now)).0;
        }
    }

    fn configure(&self) {
        let setting = (self.0.hooks.setting)();
        let mut state = self.state();
        state.watcher = None;
        state.layout = None;
        state.shortcuts_pending = false;
        state.status = BridgeStatus {
            enabled: setting.enabled,
            path: setting
                .dir
                .as_ref()
                .ok()
                .map(|dir| dir.display().to_string()),
            source: Some(setting.source),
            ..BridgeStatus::default()
        };
        if !setting.enabled {
            log::info!("iphone bridge: off (switched off in the settings)");
            return;
        }
        let dir = match &setting.dir {
            Ok(dir) => dir.clone(),
            Err(problem) => {
                log::warn!("iphone bridge: no folder: {problem}");
                state.status.problem = Some(problem.clone());
                return;
            }
        };
        let layout = match folder::prepare(&dir, setting.source) {
            Ok(layout) => layout,
            Err(problem) => {
                log::warn!("iphone bridge: folder not ready: {problem}");
                state.status.problem = Some(problem);
                return;
            }
        };
        let signal = Arc::clone(&self.0.signal);
        let delay = self.0.timing.event_delay;
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok_and(|event| !event.kind.is_access()) {
                signal.schedule(delay);
            }
        })
        .and_then(|mut watcher| {
            watcher.watch(&layout.inbox(), RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        let watching = match watcher {
            Ok(watcher) => {
                state.watcher = Some(watcher);
                true
            }
            Err(error) => {
                log::warn!("iphone bridge: file events unavailable, polling only ({error})");
                false
            }
        };
        let place = match setting.source {
            DirSource::Default => home_relative(&dir),
            DirSource::Env => "DISE_BRIDGE_DIR".to_string(),
            DirSource::Config => "config.json bridgeDir".to_string(),
        };
        log::info!(
            "iphone bridge: folder ready at {place} (source={:?}, watching={watching}, poll={}s)",
            setting.source,
            self.0.timing.poll.as_secs()
        );
        state.status.ready = true;
        state.layout = Some(layout);
        state.shortcuts_pending = true;
        drop(state);
        (self.0.hooks.ready)();
    }

    fn import_pass(&self) {
        let Some(layout) = self.state().layout.clone() else {
            return;
        };
        let root = match (self.0.hooks.kb_root)() {
            Ok(root) => root,
            Err(problem) => {
                self.report_once(
                    "kb",
                    &format!("no knowledge base: {problem}"),
                    log::Level::Warn,
                );
                return;
            }
        };
        let env = &self.0.env.get().env;
        let mut ledger = ledger::Ledger::load(
            self.0
                .hooks
                .app_dir
                .as_ref()
                .map(|dir| dir.join(ledger::FILE_NAME)),
        );
        let running = || (self.0.hooks.running)();
        let mut download = |path: &Path| {
            let mut state = self.state();
            let now = Instant::now();
            let recent = state
                .downloads
                .get(path)
                .is_some_and(|at| now.duration_since(*at) < Duration::from_secs(60));
            if !recent {
                state.downloads.insert(path.to_path_buf(), now);
                drop(state);
                (self.0.hooks.download)(path);
            }
        };
        let mut ctx = import::Context {
            kb_root: &root,
            env,
            lock: &self.0.lock,
            running: &running,
            ledger: &mut ledger,
            settle: self.0.timing.settle,
            lock_wait: import::LOCK_WAIT,
            download: &mut download,
        };
        let pass = import::run_pass(&layout, &mut ctx);

        let mut captures = 0;
        let mut answers = 0;
        let mut waiting = 0;
        let mut seen = Vec::new();
        for item in &pass.items {
            let id = format!("{}/{}", item.kind.label(), item.name);
            seen.push(id.clone());
            match &item.outcome {
                Outcome::Imported {
                    path,
                    note,
                    removed,
                } => {
                    match (item.kind, note) {
                        (Kind::Answer, None) => answers += 1,
                        _ => captures += 1,
                    }
                    log::info!(
                        "iphone import: {id} -> {path:?}{} (icloud file removed={removed})",
                        note.as_ref()
                            .map(|why| format!(" kept as a capture: {why}"))
                            .unwrap_or_default()
                    );
                    if !removed {
                        log::warn!("iphone import: {id} could not be deleted from iCloud; it will not be imported again");
                    }
                }
                Outcome::AlreadyImported { removed } => {
                    log::info!("iphone import: {id} was imported before; deleted again (removed={removed})")
                }
                Outcome::Empty { removed } => {
                    log::info!("iphone import: {id} was empty; deleted (removed={removed})")
                }
                Outcome::Settling => {
                    waiting += 1;
                    self.report_once(
                        &id,
                        &format!("{id} is still being written or downloaded"),
                        log::Level::Info,
                    );
                }
                Outcome::Waiting(why) => {
                    waiting += 1;
                    self.report_once(&id, &format!("{id} waits: {why}"), log::Level::Info);
                }
                Outcome::Skipped(why) => {
                    self.report_once(&id, &format!("{id} left alone: {why}"), log::Level::Warn)
                }
                Outcome::Failed(why) => {
                    waiting += 1;
                    self.report_once(&id, &format!("{id} not imported: {why}"), log::Level::Warn);
                }
            }
        }
        {
            let mut state = self.state();
            state
                .reported
                .retain(|id, _| id == "kb" || seen.contains(id));
            state.reported.remove("kb");
            state.status.waiting = waiting;
            if captures + answers > 0 {
                let now = LocalTime::now();
                let mut parts = Vec::new();
                if captures > 0 {
                    parts.push(format!("{captures} 条记一笔"));
                }
                if answers > 0 {
                    parts.push(format!("{answers} 份回答"));
                }
                state.status.last_import = Some(format!(
                    "{} {:02}:{:02} 收进 {}",
                    now.date(),
                    now.hour,
                    now.minute,
                    parts.join("、")
                ));
            }
        }
        let paths = pass.imported_paths();
        if !paths.is_empty() {
            (self.0.hooks.imported)(paths);
        }
        if pass.retry_soon {
            self.0.signal.schedule(self.0.timing.settle_retry);
        }
    }

    /// Logs `line` unless the same was already logged for `id`.
    fn report_once(&self, id: &str, line: &str, level: log::Level) {
        let mut state = self.state();
        if state.reported.get(id).map(String::as_str) != Some(line) {
            state.reported.insert(id.to_string(), line.to_string());
            log::log!(level, "iphone import: {line}");
        }
    }

    fn install_shortcuts(&self) {
        let layout = {
            let mut state = self.state();
            if !std::mem::take(&mut state.shortcuts_pending) {
                return;
            }
            state.layout.clone()
        };
        let Some(layout) = layout else { return };
        let record = self
            .0
            .hooks
            .app_dir
            .as_ref()
            .map(|dir| dir.join(shortcuts::STATE_FILE));
        if let Some(dir) = record.as_ref().and_then(|file| file.parent()) {
            let _ = std::fs::create_dir_all(dir);
        }
        let installed =
            shortcuts::install(&layout.shortcuts(), record.as_deref(), &*self.0.hooks.sign);
        for (name, error) in &installed.failed {
            log::warn!("iphone shortcuts: {name} could not be signed: {error}");
        }
        log::info!(
            "iphone shortcuts: signed={} kept={} failed={} guide_written={}",
            installed.signed.len(),
            installed.kept,
            installed.failed.len(),
            installed.guide_written
        );
        let total = shortcuts::SOURCES.len();
        self.state().status.shortcuts = Some(if installed.failed.is_empty() {
            format!("{total} 个快捷指令已签好，放在「快捷指令」文件夹里")
        } else {
            format!(
                "有 {} 个快捷指令没能签好（{}），可以照使用说明手动建",
                installed.failed.len(),
                installed
                    .failed
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join("、")
            )
        });
    }
}

/// Asks iCloud to download `path` (a `.icloud` placeholder) with `brctl`,
/// if the system has it. Best effort: the next pass looks again anyway.
pub fn download_with_brctl(path: &Path) {
    const BRCTL: &str = "/usr/bin/brctl";
    if !Path::new(BRCTL).exists() {
        return;
    }
    let mut command = std::process::Command::new(BRCTL);
    command.arg("download").arg(path);
    match crate::kb::git::finish(command, Duration::from_secs(15)) {
        Ok(done) if done.success() => log::info!("iphone import: asked iCloud to download a file"),
        Ok(done) => log::info!(
            "iphone import: brctl download did not work: {}",
            done.detail()
        ),
        Err(error) => log::info!("iphone import: brctl download did not run: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};
    use crate::shell_env::{ChildEnv, Origin, ShellEnv};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn imports_exports_and_switches_off() {
        let kb = TempDir::new("bridge-service-kb");
        init_repo(kb.path());
        kb.write("index.md", "# 目录\n");
        git(kb.path(), &["add", "-A"]);
        git(kb.path(), &["commit", "-q", "-m", "init"]);
        let bridge = TempDir::new("bridge-service");
        let app = TempDir::new("bridge-service-app");
        let config = app.path().join("config.json");
        let dir = bridge.path().join("底色");

        let imported = Arc::new(Mutex::new(Vec::<String>::new()));
        let ready = Arc::new(AtomicUsize::new(0));
        let root = kb.path().to_path_buf();
        let setting_dir = dir.clone();
        let config_file = config.clone();
        let imported_hook = Arc::clone(&imported);
        let ready_hook = Arc::clone(&ready);
        let service = BridgeService::new(
            KbLock::default(),
            ShellEnvState::ready(ShellEnv {
                env: ChildEnv::inherit(),
                origin: Origin::LoginShell,
                took: Duration::ZERO,
            }),
            BridgeTiming {
                poll: Duration::from_millis(200),
                settle_retry: Duration::from_millis(100),
                event_delay: Duration::from_millis(50),
                focus_gap: Duration::ZERO,
                settle: Duration::from_millis(300),
            },
            Hooks {
                kb_root: Box::new(move || Ok(root.clone())),
                setting: Box::new(move || {
                    folder::resolve(
                        Some(setting_dir.clone().into_os_string()),
                        Some(&config_file),
                        None,
                    )
                }),
                app_dir: Some(app.path().to_path_buf()),
                running: Box::new(|| None),
                imported: Box::new(move |paths| imported_hook.lock().unwrap().extend(paths)),
                ready: Box::new(move || {
                    ready_hook.fetch_add(1, Ordering::SeqCst);
                }),
                sign: Box::new(|xml, out| {
                    std::fs::write(out, format!("AEA1{}", xml.len())).map_err(|e| e.to_string())
                }),
                download: Box::new(|_| {}),
            },
        );
        service.start();
        wait_for("the folder", || service.status().ready);
        assert_eq!(ready.load(Ordering::SeqCst), 1);
        wait_for("the shortcuts", || service.status().shortcuts.is_some());
        assert!(dir.join("快捷指令/记一笔.shortcut").is_file());
        assert!(dir.join("快捷指令/使用说明.html").is_file());

        std::fs::write(
            dir.join("收件/记一笔/2026-10-12T09-00-00.txt"),
            "手机上记的",
        )
        .unwrap();
        wait_for("the import", || !imported.lock().unwrap().is_empty());
        assert_eq!(
            imported.lock().unwrap().as_slice(),
            ["inbox/2026-10-12T09-00-00-iphone.md"]
        );
        assert!(!dir.join("收件/记一笔/2026-10-12T09-00-00.txt").exists());
        assert_eq!(commit_count(kb.path()), 2);
        assert!(service.status().last_import.unwrap().contains("1 条记一笔"));
        assert!(app.path().join(ledger::FILE_NAME).is_file());

        let done = service
            .export(&[export::ExportFile {
                name: "今日.html".into(),
                content: "<p>今日</p>".into(),
            }])
            .unwrap();
        assert!(done.exported);
        assert_eq!(done.written.written, 1);
        assert!(dir.join("阅读/今日.html").is_file());

        // Switched off: nothing is imported or exported any more.
        std::fs::write(&config, r#"{ "bridgeEnabled": false }"#).unwrap();
        service.reconfigure();
        wait_for("switching off", || !service.status().enabled);
        let skipped = service.export(&[]).unwrap();
        assert!(!skipped.exported);
        assert!(
            dir.join("阅读/今日.html").is_file(),
            "an export is never removed by switching off"
        );
        std::fs::write(dir.join("收件/记一笔/later.txt"), "关掉以后").unwrap();
        std::thread::sleep(Duration::from_millis(800));
        assert!(dir.join("收件/记一笔/later.txt").exists());
        service.stop();
    }
}
