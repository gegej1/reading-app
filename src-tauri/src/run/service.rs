//! The lifecycle of an AI run. One run at a time.
//!
//! ```text
//! start ─ sync ─ snapshot ─ engine ─ collect changes ─ gate ─┬─ commit ─ sync
//!   │       │        │                      │                └─ wait for a person:
//!   │       │        └ empty queue: stop    └ nothing changed: stop   discard / keep / check again
//!   │       └ cannot sync safely: stop
//!   └ another run is going on: refused
//! ```
//!
//! - **Preconditions.** The knowledge base is a repository root on a branch,
//!   nothing tracked is modified, and the queue is not empty. With an empty
//!   queue the engine is never started (it costs money). A sync runs first, so
//!   captures made elsewhere are part of the queue.
//! - **While the engine works** the run is a *job* on the knowledge-base lock
//!   (`crate::kb::lock`): syncs and edits are refused with a "busy" answer
//!   that names the run, quick captures still go through as commits of their
//!   own, and the run later leaves those commits and files alone.
//! - **Afterwards**, however the engine ended, the changes are listed and the
//!   gate checks them (`super::gate`). Only when the engine finished normally
//!   and the gate found nothing, the app deletes the processed queue files and
//!   commits. Otherwise everything stays in the working tree, the job stays
//!   in place (so no sync or edit can pick the changes up), and a person
//!   decides: discard, keep anyway, or check again after fixing something.
//! - The gate runs in the main webview. This service hands out its input
//!   ([`RunService::gate_input`]) and waits a bounded time for the report
//!   ([`RunService::gate_report`]); without an answer nothing is committed.
//! - A run that was interrupted by the app quitting is picked up again at the
//!   next start ([`RunService::restore`]) and waits for a decision.
//!
//! Nothing here knows Tauri: what happens with a new status or step is up to
//! the `notify` hook.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::engine;
use super::gate::{self, GateInput, GateIssue, GateReport, RunFacts};
use super::process::{self, Control, Exit};
use super::prompt;
use super::record::{
    self, Decision, EngineReport, EventLog, Outcome, Phase, Reason, RunRecord, SavedSnapshot,
    DISCARDED_DIR, EVENTS_FILE, FINAL_MESSAGE_FILE, KEEP_RUNS, STDERR_FILE, STEPS_FILE,
};
use super::settings::{self, Overrides, RunSettings};
use super::stream::{Step, StepKind, StreamParser, StreamSummary};
use super::{EngineKind, Task};
use crate::kb::capture::LocalTime;
use crate::kb::changes::{self, Changes, CommitPlan, Snapshot};
use crate::kb::lock::{KbJob, KbLock, NotedCommit};
use crate::kb::sync::{SyncReport, SyncState};
use crate::shell_env::{ChildEnv, ShellEnvState};

/// The rules of the knowledge base; without them a run makes no sense.
pub const RULES_FILE: &str = "AGENTS.md";
/// Shown when Claude Code has no way to log in.
pub const NOT_LOGGED_IN: &str = "没有可用的登录凭据：Claude Code 没有登录，也没有能用的 apiKeyHelper 或 API key。请先在终端里运行一次 claude 完成登录";
/// How many steps of a run are kept for a window that opens later.
const KEEP_STEPS: usize = 400;
/// How often a check is repeated because files changed under it.
const MAX_CHECK_ROUNDS: u32 = 3;

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    pub task: Task,
    pub overrides: Overrides,
    /// What asked for it: `ui` or `command line`.
    pub trigger: &'static str,
}

/// One step of a run, as sent to the windows (and kept in `steps.jsonl`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    pub run_id: String,
    /// Counts up from 1 within a run.
    pub seq: u32,
    /// Milliseconds since the run started.
    pub elapsed_ms: u64,
    #[serde(flatten)]
    pub step: Step,
}

/// A kept run with its steps, for the history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub record: RunRecord,
    pub steps: Vec<StepEvent>,
}

/// What the windows show about runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatus {
    /// `idle` when no run is going on.
    pub phase: Phase,
    /// The run going on, else the last one that ended.
    pub run: Option<RunRecord>,
    /// While `phase` is `checking`: which check the gate is asked to answer.
    pub gate_attempt: u32,
}

/// What the service tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Status(Box<RunStatus>),
    Step(Box<StepEvent>),
    /// A run is over and syncs are possible again. `files_changed`: it
    /// committed or discarded something, so the windows should reload.
    Ended { files_changed: bool },
}

/// Where the service keeps its files.
#[derive(Debug, Clone)]
pub struct RunDirs {
    /// One folder per run (see `super::record`).
    pub runs: PathBuf,
    /// A private folder for the temporary auth file.
    pub temp: PathBuf,
}

/// The waits of the service (shortened in tests).
#[derive(Debug, Clone, Copy)]
pub struct RunTiming {
    /// How long the gate may take to answer.
    pub gate_wait: Duration,
    /// After SIGTERM, how long the engine has until SIGKILL.
    pub stop_grace: Duration,
    /// How long the run waits for a capture or a sync to release the lock.
    pub lock_wait: Duration,
}

impl Default for RunTiming {
    fn default() -> Self {
        RunTiming {
            gate_wait: Duration::from_secs(120),
            stop_grace: Duration::from_secs(3),
            lock_wait: Duration::from_secs(45),
        }
    }
}

type RootFn = dyn Fn() -> Result<PathBuf, String> + Send + Sync;
type SettingsFn = dyn Fn() -> (RunSettings, Vec<String>) + Send + Sync;
type SyncFn = dyn Fn() -> SyncReport + Send + Sync;
type NotifyFn = dyn Fn(Notice) + Send + Sync;

/// What the service needs from the app around it.
pub struct Hooks {
    /// The knowledge-base folder (canonical).
    pub root: Box<RootFn>,
    /// The engine settings, read anew for every run.
    pub settings: Box<SettingsFn>,
    /// Syncs with the remote now and says how it went.
    pub sync: Box<SyncFn>,
    pub notify: Box<NotifyFn>,
}

/// A cloneable handle to the one run service of the app.
#[derive(Clone)]
pub struct RunService(Arc<Shared>);

struct Shared {
    lock: KbLock,
    env: ShellEnvState,
    dirs: RunDirs,
    timing: RunTiming,
    hooks: Hooks,
    state: Mutex<State>,
    /// Signalled when the gate answered or a run ended.
    changed: Condvar,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    /// The last run that ended, with its steps.
    last: Option<RunRecord>,
    last_steps: Vec<StepEvent>,
    /// The app is quitting: an engine that stops now was not stopped by the user.
    quitting: bool,
}

/// The run going on.
struct Active {
    record: RunRecord,
    folder: PathBuf,
    root: PathBuf,
    started: Instant,
    cancel: Arc<AtomicBool>,
    /// The engine process exists right now.
    engine_running: bool,
    /// Held from the snapshot until the run is over.
    job: Option<KbJob>,
    snapshot: Option<Snapshot>,
    /// Captures noted in an earlier session of the app (see `restore`).
    beside: Vec<NotedCommit>,
    /// The changes the current gate input was built from.
    changes: Option<Changes>,
    gate_input: Option<Arc<GateInput>>,
    attempt: u32,
    /// The gate's report for `attempt`, until the run thread takes it.
    answer: Option<GateReport>,
    steps: VecDeque<StepEvent>,
    seq: u32,
}

impl Active {
    fn new(record: RunRecord, folder: PathBuf, root: PathBuf) -> Self {
        Active {
            record,
            folder,
            root,
            started: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            engine_running: false,
            job: None,
            snapshot: None,
            beside: Vec::new(),
            changes: None,
            gate_input: None,
            attempt: 0,
            answer: None,
            steps: VecDeque::new(),
            seq: 0,
        }
    }

    /// Why a new run cannot start while this one is around.
    fn in_the_way(&self) -> String {
        let task = self.record.task.label();
        match self.record.phase {
            Phase::NeedsDecision => {
                format!("上一次{task}的结果还在等你处理，处理完才能开始新的")
            }
            _ => format!("正在{task}，等它结束再开始新的"),
        }
    }
}

/// Why a run stops before the engine's changes are checked.
struct Stop {
    outcome: Outcome,
    message: String,
}

impl Stop {
    fn refused(message: impl Into<String>) -> Self {
        Stop {
            outcome: Outcome::Refused,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Stop {
            outcome: Outcome::Failed,
            message: message.into(),
        }
    }
}

/// What the later steps of a run need to know about it.
#[derive(Clone)]
struct Context {
    task: Task,
    engine: EngineKind,
    date: String,
    root: PathBuf,
    folder: PathBuf,
    snapshot: Snapshot,
    engine_failure: Option<String>,
    final_message: Option<String>,
}

enum Checked {
    /// The run is finished or waits for a decision.
    Settled,
    /// Files changed between the check and the commit: check again.
    Again,
}

enum CommitError {
    /// The working tree is not what was checked any more.
    Moved,
    Failed(String),
}

fn relock<T>(result: Result<T, PoisonError<T>>) -> T {
    result.unwrap_or_else(PoisonError::into_inner)
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(10)]
}

/// `入库：3 条（Claude）`, `落裁决：2 份（Codex）`.
pub fn commit_message(task: Task, engine: EngineKind, count: usize, kept: bool) -> String {
    let note = if kept {
        "，检查未通过，手动保留"
    } else {
        ""
    };
    format!(
        "{}：{count} {}（{}{note}）",
        task.label(),
        task.unit(),
        engine.label()
    )
}

/// The summary of a report that only waits for the user's approval.
fn approval_summary(count: usize) -> String {
    format!("需要你确认：{count} 处改动动到了你的底色或体系，其余都通过了检查")
}

/// Where the engines keep the user's settings: Claude Code's `settings.json`
/// and Codex's `config.toml`, as the engines themselves would look them up
/// in `env`.
fn engine_homes(env: &ChildEnv) -> (PathBuf, PathBuf) {
    let dir = |key: &str, default: &str| {
        env.get(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env.get("HOME").unwrap_or_default()).join(default)
            })
    };
    (
        dir("CLAUDE_CONFIG_DIR", ".claude").join("settings.json"),
        dir("CODEX_HOME", ".codex").join("config.toml"),
    )
}

/// Why the engine did not finish normally, or `None` when it did.
fn engine_failure(
    engine: EngineKind,
    program: &str,
    exit: &Exit,
    stream: &StreamSummary,
    settings: &RunSettings,
    quitting: bool,
) -> Option<String> {
    if let Some(error) = &exit.spawn_error {
        let key = match engine {
            EngineKind::Claude => "claudePath",
            EngineKind::Codex => "codexPath",
        };
        return Some(format!(
            "无法运行 {program}（{error}）。请确认已经安装 {}，或者在 config.json 里填写 {key}",
            engine.label()
        ));
    }
    if exit.cancelled {
        return Some(if quitting {
            "App 退出时 AI 还没做完，被停止了".to_string()
        } else {
            "已手动停止".to_string()
        });
    }
    if exit.timed_out {
        let minutes = (settings.timeout.as_secs_f64() / 60.0).round().max(1.0);
        return Some(format!("超过 {minutes} 分钟没有结束，已停止"));
    }
    let said = stream
        .error
        .as_deref()
        .or(stream.final_message.as_deref())
        .unwrap_or_default();
    if said.contains("Not logged in") || said.contains("Please run /login") {
        return Some(NOT_LOGGED_IN.to_string());
    }
    if stream.is_error {
        return Some(match stream.subtype.as_deref() {
            Some("error_max_budget_usd") => format!(
                "花费到了这次运行的上限（{} 美元），AI 停下了",
                settings.budget_usd
            ),
            Some("error_max_turns") => "对话轮数到了上限，AI 停下了".to_string(),
            _ if said.is_empty() => "AI 报告了错误，没有说明原因".to_string(),
            _ => format!("AI 报告了错误：{said}"),
        });
    }
    let code = exit
        .code
        .map_or("被信号中止".to_string(), |code| format!("退出码 {code}"));
    if !stream.finished {
        return Some(format!("AI 没有给出结果就退出了（{code}）"));
    }
    if exit.code != Some(0) {
        return Some(format!("AI 没有正常退出（{code}）"));
    }
    None
}

/// Writes every capture committed beside the run into the run's folder the
/// moment it is noted, so a restart still knows it is not the run's (a
/// discard would otherwise delete it).
fn write_down_captures(job: &KbJob, folder: &std::path::Path, id: &str) {
    let folder = folder.to_path_buf();
    let id = id.to_string();
    job.on_commit(move |noted| {
        if let Err(error) = record::append_beside(&folder, noted) {
            log::error!("run {id}: could not write down a capture made beside the run: {error}");
        }
    });
}

/// What a sync that did not end well is called in a refusal.
fn sync_problem(report: &SyncReport) -> String {
    let what = match report.state {
        SyncState::Dirty => "有还没提交的改动挡住了同步",
        SyncState::Conflict => "本机和远端改了同一处，需要手动合并",
        SyncState::Detached => "知识库没有停在分支上",
        SyncState::InProgress => "知识库里有一个没做完的 git 操作",
        SyncState::NotRepo => "知识库文件夹不是 git 仓库的根目录",
        _ => "同步出错了",
    };
    match report.paths.first() {
        Some(path) => format!("{what}：{path}"),
        None => what.to_string(),
    }
}

impl RunService {
    pub fn new(
        lock: KbLock,
        env: ShellEnvState,
        dirs: RunDirs,
        timing: RunTiming,
        hooks: Hooks,
    ) -> Self {
        RunService(Arc::new(Shared {
            lock,
            env,
            dirs,
            timing,
            hooks,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        relock(self.0.state.lock())
    }

    fn notify(&self, notice: Notice) {
        (self.0.hooks.notify)(notice);
    }

    fn env(&self) -> &ChildEnv {
        &self.0.env.get().env
    }

    /// Runs `change` on the run `id` if it is the one going on.
    fn with_active<T>(&self, id: &str, change: impl FnOnce(&mut Active) -> T) -> Option<T> {
        let mut state = self.state();
        state
            .active
            .as_mut()
            .filter(|active| active.record.id == id)
            .map(change)
    }

    /// The status as it is right now.
    pub fn status(&self) -> RunStatus {
        let state = self.state();
        match &state.active {
            Some(active) => RunStatus {
                phase: active.record.phase,
                run: Some(active.record.clone()),
                gate_attempt: active.attempt,
            },
            None => RunStatus {
                phase: Phase::Idle,
                run: state.last.clone(),
                gate_attempt: 0,
            },
        }
    }

    /// The task of the run that holds the knowledge base right now, if any.
    pub fn active_task(&self) -> Option<Task> {
        self.state().active.as_ref().map(|active| active.record.task)
    }

    /// The newest steps of the run going on, else of the last one.
    pub fn steps(&self) -> Vec<StepEvent> {
        let state = self.state();
        match &state.active {
            Some(active) => active.steps.iter().cloned().collect(),
            None => state.last_steps.clone(),
        }
    }

    /// The kept runs, newest first.
    pub fn history(&self) -> Vec<RunRecord> {
        record::history(&self.0.dirs.runs)
    }

    /// One run with its steps: the run going on or the last one from memory,
    /// an older one from its folder. Runs from before steps were kept get
    /// their steps from the engine's output (without the app's own notes).
    pub fn detail(&self, id: &str) -> Result<RunDetail, String> {
        {
            let state = self.state();
            if let Some(active) = state.active.as_ref().filter(|active| active.record.id == id) {
                return Ok(RunDetail {
                    record: active.record.clone(),
                    steps: active.steps.iter().cloned().collect(),
                });
            }
            if let Some(last) = state.last.as_ref().filter(|last| last.id == id) {
                return Ok(RunDetail {
                    record: last.clone(),
                    steps: state.last_steps.clone(),
                });
            }
        }
        let folder = record::folder_of(&self.0.dirs.runs, id)
            .ok_or_else(|| "没有这次运行的记录（可能已经被清理了）".to_string())?;
        let record = record::load(&folder).ok_or_else(|| "这次运行的记录读不出来".to_string())?;
        let steps = record::read_json_lines(&folder.join(STEPS_FILE))
            .unwrap_or_else(|| self.steps_from_events(&folder, &record));
        Ok(RunDetail { record, steps })
    }

    /// The steps of an old run, read again from its `events.jsonl`.
    fn steps_from_events(&self, folder: &std::path::Path, record: &RunRecord) -> Vec<StepEvent> {
        let Ok(events) = fs::read_to_string(folder.join(EVENTS_FILE)) else {
            return Vec::new();
        };
        let root = record::load_snapshot(folder)
            .map(|saved| saved.root)
            .or_else(|| (self.0.hooks.root)().ok())
            .unwrap_or_default();
        let mut parser = StreamParser::new(record.engine, &root);
        let mut seq = 0;
        events
            .lines()
            .flat_map(|line| parser.line(line))
            .map(|step| {
                seq += 1;
                StepEvent {
                    run_id: record.id.clone(),
                    seq,
                    elapsed_ms: 0,
                    step,
                }
            })
            .collect()
    }

    /// Tells the app the status, and writes the run's record.
    fn publish(&self) {
        let saved = {
            let state = self.state();
            state
                .active
                .as_ref()
                .map(|active| (active.folder.clone(), active.record.clone()))
        };
        if let Some((folder, record)) = saved {
            if let Err(error) = record::save(&folder, &record) {
                log::warn!("run {}: could not write run.json: {error}", record.id);
            }
        }
        self.notify(Notice::Status(Box::new(self.status())));
    }

    /// Adds a step to the run, tells the app and the log, and keeps it in the
    /// run's folder for the history.
    fn step(&self, id: &str, step: Step) {
        let event = self.with_active(id, |active| {
            active.seq += 1;
            let event = StepEvent {
                run_id: active.record.id.clone(),
                seq: active.seq,
                elapsed_ms: u64::try_from(active.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                step,
            };
            if active.steps.len() >= KEEP_STEPS {
                active.steps.pop_front();
            }
            active.steps.push_back(event.clone());
            (event, active.folder.join(STEPS_FILE))
        });
        let Some((event, file)) = event else { return };
        record::append_json_line(&file, &event);
        if let Some(line) = event.step.log_line() {
            log::info!("run {id}: {line}");
        }
        self.notify(Notice::Step(Box::new(event)));
    }

    fn note(&self, id: &str, text: impl Into<String>) {
        self.step(id, Step::note(text));
    }

    /// Starts a run in the background and returns at once. Fails when
    /// another run is going on or still waits for a decision; everything else
    /// that keeps the run from happening ends it with an outcome instead
    /// (see the status).
    pub fn start(&self, request: StartRequest) -> Result<RunStatus, String> {
        let (settings, problems) = (self.0.hooks.settings)();
        for problem in &problems {
            log::warn!("run settings: {problem}");
        }
        let (engine, model) = settings::choose(&settings, &request.overrides);
        let root = (self.0.hooks.root)()?;
        let now = LocalTime::now();

        let id = {
            let mut state = self.state();
            if let Some(active) = &state.active {
                return Err(active.in_the_way());
            }
            let id = record::create_run_folder(&self.0.dirs.runs, &now, request.task)
                .map_err(|error| format!("没能建立这次运行的记录文件夹：{error}"))?;
            let record = RunRecord::new(
                id.clone(),
                request.task,
                engine,
                model.clone(),
                request.trigger,
                now.date(),
            );
            let folder = self.0.dirs.runs.join(&id);
            state.active = Some(Active::new(record, folder, root));
            id
        };
        log::info!(
            "run {id}: {} asked for from the {} (engine={} model={})",
            request.task.slug(),
            request.trigger,
            engine.program(),
            model.as_deref().unwrap_or("default"),
        );
        self.publish();

        let service = self.clone();
        let thread_id = id.clone();
        let spawned = thread::Builder::new()
            .name("run".into())
            .spawn(move || service.run_thread(&thread_id, &settings));
        if let Err(error) = spawned {
            self.finish(
                &id,
                Outcome::Failed,
                format!("没能开始运行（{error}）"),
                None,
            );
        }
        Ok(self.status())
    }

    fn run_thread(&self, id: &str, settings: &RunSettings) {
        match self.prepare_and_run(id, settings) {
            Ok(()) => self.check(id),
            Err(stop) => self.finish(id, stop.outcome, stop.message, None),
        }
    }

    /// Everything up to the moment the engine has exited.
    fn prepare_and_run(&self, id: &str, settings: &RunSettings) -> Result<(), Stop> {
        let (task, engine, model, date, root, folder, cancel) = self
            .with_active(id, |active| {
                (
                    active.record.task,
                    active.record.engine,
                    active.record.model.clone(),
                    active.record.date.clone(),
                    active.root.clone(),
                    active.folder.clone(),
                    Arc::clone(&active.cancel),
                )
            })
            .ok_or_else(|| Stop::failed("这次运行已经不在了"))?;
        let cancelled = || cancel.load(Ordering::SeqCst);
        let stopped_early = || Stop {
            outcome: Outcome::Cancelled,
            message: "已停止，AI 还没有开始".to_string(),
        };

        // Captures made on other devices are part of the queue.
        self.note(id, "先同步一次，把别处记下的内容拿到本机");
        let synced = (self.0.hooks.sync)();
        match synced.state {
            SyncState::Synced | SyncState::RemoteMoved => {}
            SyncState::Offline => self.note(id, "连不上远端，用本机现有的内容继续"),
            SyncState::NoUpstream => self.note(id, "知识库没有连远端，只在本机处理"),
            SyncState::Busy => self.note(id, "知识库正忙，这次没有同步，用本机现有的内容继续"),
            _ => {
                return Err(Stop::refused(format!(
                    "同步没有成功（{}），先处理好再{}",
                    sync_problem(&synced),
                    task.label()
                )))
            }
        }
        if cancelled() {
            return Err(stopped_early());
        }

        // From here on syncs and edits are refused; captures still work.
        let wait = self.0.timing.lock_wait;
        let job = self
            .0
            .lock
            .start_job(task.label(), wait)
            .map_err(|busy| Stop::refused(busy.to_string()))?;
        write_down_captures(&job, &folder, id);
        let env = self.env();
        let snapshot = {
            let _guard = self
                .0
                .lock
                .acquire_beside_job(task.label(), wait)
                .map_err(|busy| Stop::refused(busy.to_string()))?;
            changes::snapshot(&root, env, task.queue_dir()).map_err(Stop::refused)?
        };
        let rules = root.join(RULES_FILE);
        if !rules.is_file() {
            return Err(Stop::refused(format!(
                "知识库里没有 {RULES_FILE}，AI 不知道规则，所以没有开始"
            )));
        }
        if snapshot.queue.is_empty() {
            return Err(Stop {
                outcome: Outcome::NothingToDo,
                message: task.empty_queue().to_string(),
            });
        }
        if cancelled() {
            return Err(stopped_early());
        }

        let queue: Vec<String> = snapshot.queue.iter().map(|file| file.path.clone()).collect();
        let saved = SavedSnapshot {
            root: root.clone(),
            snapshot: snapshot.clone(),
            beside: Vec::new(),
        };
        if let Err(error) = record::save_snapshot(&folder, &saved) {
            // Without it a restart could not tell the run's files from the user's.
            return Err(Stop::failed(format!("没能保存运行前的状态（{error}）")));
        }
        self.with_active(id, |active| {
            active.record.queue = queue.clone();
            active.record.base = Some(snapshot.base.clone());
            active.record.phase = Phase::Running;
            active.snapshot = Some(snapshot);
            active.job = Some(job);
        });
        self.publish();
        let pruned = record::prune(&self.0.dirs.runs, KEEP_RUNS);
        if pruned > 0 {
            log::info!("run {id}: removed {pruned} old run folders");
        }
        self.note(
            id,
            format!(
                "{} 里有 {} {}要处理，交给 {}",
                task.queue_dir(),
                queue.len(),
                task.unit(),
                engine.label()
            ),
        );

        // The engine.
        let prompt = prompt::engine_prompt(task, engine, &date, &queue);
        let program = settings.program(engine);
        let mut command = env.command(&program);
        command.current_dir(&root);
        let (claude_settings, codex_config) = engine_homes(env);
        let mut final_file = None;
        let (auth, stdin) = match engine {
            EngineKind::Claude => {
                let auth = engine::write_auth_file(&claude_settings, &self.0.dirs.temp, id)
                    .map_err(Stop::failed)?;
                command.args(engine::claude_args(
                    &prompt,
                    auth.path(),
                    &rules,
                    model.as_deref(),
                    settings.budget_usd,
                ));
                (Some(auth), None)
            }
            EngineKind::Codex => {
                let (servers, skipped) = engine::codex_mcp_servers(&codex_config);
                if !skipped.is_empty() {
                    log::warn!(
                        "run {id}: {} MCP servers of the Codex configuration could not be switched off (unusual names)",
                        skipped.len()
                    );
                }
                let file = folder.join(FINAL_MESSAGE_FILE);
                command.args(engine::codex_args(&root, &file, model.as_deref(), &servers));
                final_file = Some(file);
                (None, Some(prompt))
            }
        };

        let mut events = EventLog::open(&folder);
        let mut parser = StreamParser::new(engine, &root);
        let control = Control {
            cancel,
            timeout: settings.timeout,
            grace: self.0.timing.stop_grace,
        };
        let began = Instant::now();
        let exit = process::run(
            command,
            stdin,
            &control,
            &mut || {
                self.with_active(id, |active| active.engine_running = true);
            },
            &mut |line| {
                events.append(line);
                for step in parser.line(line) {
                    self.step(id, step);
                }
            },
        );
        // The login leaves the disk as soon as the engine is gone.
        drop(auth);
        drop(events);
        self.with_active(id, |active| active.engine_running = false);

        let mut stream = parser.into_summary();
        if let Some(reply) = final_file
            .and_then(|file| fs::read_to_string(file).ok())
            .filter(|reply| !reply.trim().is_empty())
        {
            stream.final_message = Some(reply);
        }
        if !exit.stderr_tail.is_empty() {
            let _ = fs::write(folder.join(STDERR_FILE), &exit.stderr_tail);
        }
        let quitting = self.state().quitting;
        let failure = engine_failure(engine, &program, &exit, &stream, settings, quitting);
        let report = EngineReport {
            ok: failure.is_none(),
            failure,
            exit,
            stream,
            duration_ms: u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        log::info!(
            "run {id}: engine ended ok={} exit={:?} cancelled={} timed_out={} model={} key={} tools={} denials={} turns={} cost={} ({} s){}",
            report.ok,
            report.exit.code,
            report.exit.cancelled,
            report.exit.timed_out,
            report.stream.model.as_deref().unwrap_or("-"),
            report.stream.key_source.as_deref().unwrap_or("-"),
            report.stream.tool_calls,
            report.stream.denials,
            report.stream.turns.map_or("-".to_string(), |turns| turns.to_string()),
            report.stream.cost_usd.map_or("-".to_string(), |cost| format!("${cost:.4}")),
            report.duration_ms / 1000,
            report.failure.as_deref().map(|why| format!(": {why}")).unwrap_or_default(),
        );
        self.with_active(id, |active| active.record.engine_report = Some(report));
        self.publish();
        Ok(())
    }

    /// What the steps after the engine need, or `None` when the run is gone.
    fn context(&self, id: &str) -> Option<Context> {
        self.with_active(id, |active| {
            let snapshot = active.snapshot.clone()?;
            let report = active.record.engine_report.as_ref();
            Some(Context {
                task: active.record.task,
                engine: active.record.engine,
                date: active.record.date.clone(),
                root: active.root.clone(),
                folder: active.folder.clone(),
                snapshot,
                engine_failure: report.and_then(|report| report.failure.clone()),
                final_message: report.and_then(|report| report.stream.final_message.clone()),
            })
        })
        .flatten()
    }

    /// The commits the app made beside the run so far (captures), from this
    /// and from earlier sessions.
    fn beside(&self, id: &str) -> Vec<NotedCommit> {
        self.with_active(id, |active| {
            let mut beside = active.beside.clone();
            if let Some(job) = &active.job {
                for noted in job.noted_commits() {
                    if !beside.contains(&noted) {
                        beside.push(noted);
                    }
                }
            }
            beside
        })
        .unwrap_or_default()
    }

    /// Lists the changes, has the gate check them, and commits or stops for
    /// a decision.
    fn check(&self, id: &str) {
        for _ in 0..MAX_CHECK_ROUNDS {
            match self.check_once(id) {
                Checked::Settled => return,
                Checked::Again => self.note(id, "检查之后文件又变了，重新检查一次"),
            }
        }
        self.needs_decision(
            id,
            None,
            "文件一直在变，没法确定检查的是不是最后的样子。等一下可以重新检查".to_string(),
        );
    }

    fn check_once(&self, id: &str) -> Checked {
        let Some(context) = self.context(id) else {
            return Checked::Settled;
        };
        let wait = self.0.timing.lock_wait;
        let collected = (|| {
            let _guard = self
                .0
                .lock
                .acquire_beside_job(context.task.label(), wait)
                .map_err(|busy| busy.to_string())?;
            let beside = self.beside(id);
            let changes = changes::collect(&context.root, self.env(), &context.snapshot, &beside)?;
            let unchanged = gate::unchanged_docs(&context.root, &changes)?;
            Ok::<_, String>((changes, unchanged, beside))
        })();
        let (changes, unchanged, beside) = match collected {
            Ok(collected) => collected,
            Err(error) => {
                self.needs_decision(id, None, format!("没能列出这次运行的改动：{error}"));
                return Checked::Settled;
            }
        };
        let saved = SavedSnapshot {
            root: context.root.clone(),
            snapshot: context.snapshot.clone(),
            beside,
        };
        if let Err(error) = record::save_snapshot(&context.folder, &saved) {
            log::warn!("run {id}: could not update snapshot.json: {error}");
        }

        if changes.is_empty() {
            let cancelled = self
                .with_active(id, |active| {
                    active
                        .record
                        .engine_report
                        .as_ref()
                        .is_some_and(|report| report.exit.cancelled)
                })
                .unwrap_or(false);
            let (outcome, message) = match &context.engine_failure {
                Some(_) if cancelled => (
                    Outcome::Cancelled,
                    "已停止。AI 没有留下任何改动，队列原样留着".to_string(),
                ),
                Some(failure) => (
                    Outcome::Failed,
                    format!("{failure}。AI 没有留下任何改动，队列原样留着"),
                ),
                None => (
                    Outcome::Failed,
                    "没有任何改动：AI 没有改动文件，队列原样留着".to_string(),
                ),
            };
            self.finish(id, outcome, message, None);
            return Checked::Settled;
        }

        let attempt = self
            .with_active(id, |active| {
                active.attempt += 1;
                let input = gate::build_input(
                    &RunFacts {
                        run_id: &active.record.id,
                        attempt: active.attempt,
                        task: context.task,
                        date: &context.date,
                        final_message: context.final_message.as_deref(),
                        engine_failure: context.engine_failure.as_deref(),
                    },
                    &context.snapshot,
                    &changes,
                    unchanged,
                );
                active.answer = None;
                active.changes = Some(changes.clone());
                active.gate_input = Some(Arc::new(input));
                active.record.phase = Phase::Checking;
                active.record.message = Some("正在检查改动".to_string());
                active.attempt
            })
            .unwrap_or(0);
        self.note(id, format!("正在检查 {} 处改动", changes.files.len()));
        self.publish();

        let Some(report) = self.wait_for_answer(id, attempt) else {
            self.needs_decision(
                id,
                None,
                "没能完成检查：检查程序没有回应。可以重新检查，或者丢弃这次的改动".to_string(),
            );
            return Checked::Settled;
        };
        self.settle(id, &context, &changes, report)
    }

    /// Waits for the gate's report on `attempt`; `None` when it does not come in time.
    fn wait_for_answer(&self, id: &str, attempt: u32) -> Option<GateReport> {
        let deadline = Instant::now() + self.0.timing.gate_wait;
        let mut state = self.state();
        loop {
            let active = state
                .active
                .as_mut()
                .filter(|active| active.record.id == id && active.attempt == attempt)?;
            if let Some(answer) = active.answer.take() {
                return Some(answer);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = relock(self.0.changed.wait_timeout(state, left)).0;
        }
    }

    /// The gate's input for the check `attempt` of the run `id`.
    pub fn gate_input(&self, id: &str, attempt: u32) -> Result<Arc<GateInput>, String> {
        self.with_active(id, |active| {
            if active.record.phase != Phase::Checking || active.attempt != attempt {
                return None;
            }
            active.gate_input.clone()
        })
        .flatten()
        .ok_or_else(|| "这次检查已经结束，或者不是当前的运行".to_string())
    }

    /// Takes the gate's report for the check `attempt` of the run `id`.
    pub fn gate_report(&self, id: &str, attempt: u32, report: GateReport) -> Result<(), String> {
        let taken = self.with_active(id, |active| {
            if active.record.phase != Phase::Checking || active.attempt != attempt {
                return false;
            }
            active.answer = Some(report);
            true
        });
        self.0.changed.notify_all();
        if taken == Some(true) {
            Ok(())
        } else {
            Err("这次检查已经结束，或者不是当前的运行".to_string())
        }
    }

    /// Decides what a report means: commit, or wait for a person.
    fn settle(
        &self,
        id: &str,
        context: &Context,
        changes: &Changes,
        mut report: GateReport,
    ) -> Checked {
        if let Err(problem) = gate::validate_report(&report, changes, &context.snapshot) {
            log::warn!("run {id}: the gate's report was not usable: {problem}");
            self.needs_decision(
                id,
                None,
                format!("检查结果不能用：{problem}。可以重新检查，或者丢弃这次的改动"),
            );
            return Checked::Settled;
        }
        // Defence in depth: the two cheapest rules again, and the engine's own end.
        for issue in gate::own_violations(changes, &context.snapshot) {
            let known = report
                .violations
                .iter()
                .any(|found| found.rule == issue.rule && found.path == issue.path);
            if !known {
                log::warn!(
                    "run {id}: the app's own check found what the gate did not: {}",
                    issue.message
                );
                report.violations.push(issue);
            }
        }
        if let Some(failure) = &context.engine_failure {
            if !report.violations.iter().any(|found| found.rule == "engine") {
                report.violations.push(GateIssue {
                    rule: "engine".into(),
                    path: None,
                    message: format!("AI 没有正常结束：{failure}"),
                });
            }
        }
        // The user's own layer: systems/ always waits for them.
        for item in gate::own_approvals(changes, &context.snapshot) {
            if !report.approvals.iter().any(|found| found.path == item.path) {
                log::warn!(
                    "run {id}: the app's own check asks what the gate did not: {}",
                    item.message
                );
                report.approvals.push(item);
            }
        }
        let ok = report.violations.is_empty() && report.approvals.is_empty();
        if report.ok && !ok {
            report.ok = false;
            report.summary = if report.violations.is_empty() {
                approval_summary(report.approvals.len())
            } else {
                format!("没通过检查：{} 个问题", report.violations.len())
            };
        }
        for warning in &report.warnings {
            log::info!("run {id}: gate warning: {}", warning.message);
        }

        if !report.ok {
            let message = report.summary.clone();
            self.needs_decision(id, Some(report), message);
            return Checked::Settled;
        }
        self.commit_checked(id, context, changes, report, None)
    }

    /// Commits a run whose report has no violations (it passed, or the user
    /// agreed to what it asked), and finishes it.
    fn commit_checked(
        &self,
        id: &str,
        context: &Context,
        changes: &Changes,
        report: GateReport,
        decision: Option<Decision>,
    ) -> Checked {
        match self.try_commit(id, context, changes, &report, decision) {
            Ok(commit) => {
                let message = match decision {
                    Some(Decision::Approved) => format!(
                        "你确认了 {} 处改动，已提交 {}",
                        report.approvals.len(),
                        short(&commit)
                    ),
                    _ => format!("{}，已提交 {}", report.summary, short(&commit)),
                };
                self.finish(id, Outcome::Committed, message, Some(commit));
                Checked::Settled
            }
            Err(CommitError::Moved) => Checked::Again,
            Err(CommitError::Failed(error)) => {
                self.commit_failed(id, &error);
                Checked::Settled
            }
        }
    }

    /// The check passed (or the user agreed), but git did not take the
    /// commit: the run waits for another try, nothing is lost.
    fn commit_failed(&self, id: &str, error: &str) {
        let message = format!("检查已经通过，但提交没有成功：{error}。改动原样留着，可以重试提交");
        let found = self.with_active(id, |active| {
            active.record.phase = Phase::NeedsDecision;
            active.record.reason = Some(Reason::CommitFailed);
            active.record.message = Some(message.clone());
            if let Some(job) = &active.job {
                job.set_operation(&format!("等你处理{}的结果", active.record.task.label()));
            }
        });
        if found.is_none() {
            return;
        }
        log::warn!("run {id}: the commit failed after the check passed: {error}");
        self.step(
            id,
            Step {
                kind: StepKind::Error,
                verb: None,
                target: None,
                text: Some(message),
            },
        );
        self.publish();
    }

    /// Makes the run's commit, after making sure the working tree is still
    /// what `checked` describes.
    fn try_commit(
        &self,
        id: &str,
        context: &Context,
        checked: &Changes,
        report: &GateReport,
        decision: Option<Decision>,
    ) -> Result<String, CommitError> {
        let kept = decision == Some(Decision::KeptDespiteViolations);
        self.with_active(id, |active| {
            active.record.phase = Phase::Committing;
            active.record.message = Some("正在提交".to_string());
            active.record.gate = Some(report.clone());
            active.record.decision = decision;
        });
        self.publish();
        let failed = CommitError::Failed;
        let _guard = self
            .0
            .lock
            .acquire_beside_job(context.task.label(), self.0.timing.lock_wait)
            .map_err(|busy| failed(busy.to_string()))?;
        let env = self.env();
        let now = changes::collect(&context.root, env, &context.snapshot, &self.beside(id))
            .map_err(failed)?;
        if now.fingerprint() != checked.fingerprint() {
            return Err(CommitError::Moved);
        }
        let message = commit_message(
            context.task,
            context.engine,
            report.delete_queue_paths.len(),
            kept,
        );
        // Written down before the commit exists: if the app goes away right
        // after it, the next start recognises the commit by this message.
        let noted = self.with_active(id, |active| {
            active.record.commit_message = Some(message.clone());
            active.record.commit = None;
            (active.folder.clone(), active.record.clone())
        });
        if let Some((folder, record)) = noted {
            record::save(&folder, &record).map_err(|error| {
                failed(format!("没能先把提交信息记进运行记录（{error}），所以没有提交"))
            })?;
        }
        let commit = changes::commit_run(
            &context.root,
            env,
            &context.snapshot,
            CommitPlan {
                paths: &report.commit_paths,
                delete_queue: &report.delete_queue_paths,
                restore_queue: &report.restore_queue_paths,
                message: &message,
            },
        )
        .map_err(failed)?;
        let noted = self.with_active(id, |active| {
            active.record.commit = Some(commit.clone());
            (active.folder.clone(), active.record.clone())
        });
        if let Some((folder, record)) = noted {
            if let Err(error) = record::save(&folder, &record) {
                log::warn!("run {id}: could not write the commit into run.json: {error}");
            }
        }
        Ok(commit)
    }

    /// Leaves the run's changes where they are and waits for a person. The
    /// job stays, so no sync or edit can pick the changes up.
    fn needs_decision(&self, id: &str, report: Option<GateReport>, message: String) {
        let reason = match &report {
            None => Reason::Incomplete,
            Some(report) if !report.violations.is_empty() => Reason::Violations,
            Some(_) => Reason::NeedsApproval,
        };
        let found = self.with_active(id, |active| {
            active.record.phase = Phase::NeedsDecision;
            active.record.reason = Some(reason);
            active.record.decision = None;
            active.record.gate = report;
            active.record.message = Some(message.clone());
            if let Some(job) = &active.job {
                let what = if reason == Reason::NeedsApproval {
                    "等你确认"
                } else {
                    "等你处理"
                };
                job.set_operation(&format!("{what}{}的结果", active.record.task.label()));
            }
            active.record.gate.clone().unwrap_or_else(|| GateReport {
                ok: false,
                violations: Vec::new(),
                warnings: Vec::new(),
                approvals: Vec::new(),
                commit_paths: Vec::new(),
                delete_queue_paths: Vec::new(),
                restore_queue_paths: Vec::new(),
                summary: String::new(),
                final_line: None,
            })
        });
        let Some(report) = found else { return };
        if reason == Reason::NeedsApproval {
            log::warn!(
                "run {id}: waiting for your confirmation: {} changes to the 底色 or systems/ (open the app: 确认并提交 or 丢弃)",
                report.approvals.len()
            );
        } else {
            log::warn!("run {id}: waiting for a decision: {message}");
        }
        for violation in &report.violations {
            log::warn!("run {id}: violation [{}] {}", violation.rule, violation.message);
        }
        for item in &report.approvals {
            log::warn!(
                "run {id}: needs confirmation [{}] {}{}",
                item.rule,
                item.path,
                item.entry.as_deref().map(|entry| format!(" {entry}")).unwrap_or_default()
            );
        }
        self.step(
            id,
            Step {
                kind: if reason == Reason::NeedsApproval {
                    StepKind::Note
                } else {
                    StepKind::Error
                },
                verb: None,
                target: None,
                text: Some(message),
            },
        );
        self.publish();
    }

    /// Ends the run: writes its record, releases the job, tells the app.
    fn finish(&self, id: &str, outcome: Outcome, message: String, commit: Option<String>) {
        // The record goes to disk first: whoever sees the run as over and
        // looks at the history finds it there.
        let Some((folder, final_record)) = self.with_active(id, |active| {
            active.record.phase = Phase::Finished;
            // Nothing waits any more; the decision (if any) says what happened.
            active.record.reason = None;
            active.record.outcome = Some(outcome);
            active.record.message = Some(message.clone());
            active.record.ended_at = Some(record::unix_millis());
            active.record.commit = commit;
            (active.folder.clone(), active.record.clone())
        }) else {
            return;
        };
        if let Err(error) = record::save(&folder, &final_record) {
            log::warn!("run {id}: could not write run.json: {error}");
        }
        let ended = {
            let mut state = self.state();
            let Some(mut active) = state.active.take_if(|active| active.record.id == id) else {
                return;
            };
            state.last = Some(active.record.clone());
            state.last_steps = active.steps.iter().cloned().collect();
            // Ends the job before anyone can see the run as over: whoever
            // reads "idle" may sync or edit right away.
            active.job = None;
            active
        };
        self.0.changed.notify_all();
        let record = &ended.record;
        let engine = record.engine_report.as_ref();
        let line = format!(
            "run {id}: finished outcome={outcome:?} commit={} cost={} turns={} engine={} s total={} s: {message}",
            record.commit.as_deref().map_or("-", short),
            engine
                .and_then(|report| report.stream.cost_usd)
                .map_or("-".to_string(), |cost| format!("${cost:.4}")),
            engine
                .and_then(|report| report.stream.turns)
                .map_or("-".to_string(), |turns| turns.to_string()),
            engine.map_or(0, |report| report.duration_ms / 1000),
            ended.started.elapsed().as_secs(),
        );
        match outcome {
            Outcome::Committed | Outcome::NothingToDo | Outcome::Discarded | Outcome::Kept => {
                log::info!("{line}")
            }
            _ => log::warn!("{line}"),
        }
        self.notify(Notice::Status(Box::new(self.status())));
        self.notify(Notice::Ended {
            files_changed: matches!(
                outcome,
                Outcome::Committed | Outcome::Kept | Outcome::Discarded
            ),
        });
    }

    /// Stops the engine of the run going on (it then goes through the usual
    /// check). Only possible before or while the engine works.
    pub fn cancel(&self) -> Result<RunStatus, String> {
        let stopped = {
            let state = self.state();
            match &state.active {
                Some(active) if matches!(active.record.phase, Phase::Starting | Phase::Running) => {
                    active.cancel.store(true, Ordering::SeqCst);
                    Some(active.record.id.clone())
                }
                _ => None,
            }
        };
        match stopped {
            Some(id) => {
                log::info!("run {id}: stop requested");
                Ok(self.status())
            }
            None => Err("现在没有可以停止的运行".to_string()),
        }
    }

    /// Moves the run `id` from "waiting for a decision" to "being worked on"
    /// and returns what the work needs.
    fn begin_decision(&self, id: &str, what: &str) -> Result<Context, String> {
        self.begin_decision_for(id, what, None)
    }

    /// Like [`Self::begin_decision`], only while the run waits for one of `reasons`.
    fn begin_decision_for(
        &self,
        id: &str,
        what: &str,
        reasons: Option<&[Reason]>,
    ) -> Result<Context, String> {
        let taken = self.with_active(id, |active| {
            if active.record.phase != Phase::NeedsDecision {
                return Err(format!("现在不能{what}：这次运行不在等待处理"));
            }
            let reason = active.record.reason.unwrap_or(Reason::Violations);
            if reasons.is_some_and(|reasons| !reasons.contains(&reason)) {
                return Err(match reason {
                    Reason::NeedsApproval => format!("现在不能{what}：这次的结果在等你确认"),
                    Reason::CommitFailed => format!("现在不能{what}：检查已经通过，只是提交没有成功，请重试提交"),
                    Reason::Violations => format!("现在不能{what}：这次的结果没通过检查"),
                    Reason::Incomplete => format!("现在不能{what}：这次的检查还没有完成，先重新检查"),
                });
            }
            active.record.phase = Phase::Committing;
            // Only a commit that is being made has a message written down.
            active.record.commit_message = None;
            Ok(())
        });
        match taken {
            Some(Ok(())) => self
                .context(id)
                .ok_or_else(|| "这次运行缺少运行前的状态，没法处理".to_string()),
            Some(Err(error)) => Err(error),
            None => Err("没有这次运行，或者它已经结束了".to_string()),
        }
    }

    /// Back to waiting, after a decision could not be carried out.
    fn back_to_decision(&self, id: &str, error: &str) {
        self.with_active(id, |active| {
            active.record.phase = Phase::NeedsDecision;
            active.record.commit_message = None;
            if active.record.reason != Some(Reason::CommitFailed) {
                active.record.decision = None;
            }
        });
        log::warn!("run {id}: {error}");
        self.publish();
    }

    /// Undoes everything the run `id` changed, after copying its files into
    /// the run's folder.
    pub fn discard(&self, id: &str) -> Result<RunStatus, String> {
        let context = self.begin_decision(id, "丢弃")?;
        self.publish();
        let backup = context.folder.join(DISCARDED_DIR);
        let result = (|| {
            let _guard = self
                .0
                .lock
                .acquire_beside_job(context.task.label(), self.0.timing.lock_wait)
                .map_err(|busy| busy.to_string())?;
            let env = self.env();
            // What is there now, not what was there at the last check.
            let changes =
                changes::collect(&context.root, env, &context.snapshot, &self.beside(id))?;
            changes::discard_run(
                &context.root,
                env,
                &context.snapshot,
                &changes,
                &backup,
                &format!("撤回：AI 自己做的提交（{}）", context.task.label()),
            )
        })();
        match result {
            Ok(done) => {
                let message = format!(
                    "已丢弃这次{}的改动：还原 {} 个文件，删除 {} 个新文件。副本留在这次运行的记录里",
                    context.task.label(),
                    done.restored,
                    done.removed
                );
                self.with_active(id, |active| {
                    active.record.discarded_to = Some(backup.to_string_lossy().into_owned());
                });
                self.finish(id, Outcome::Discarded, message, done.revert_commit);
                Ok(self.status())
            }
            Err(error) => {
                self.back_to_decision(id, &format!("discard failed: {error}"));
                Err(format!("没能丢弃：{error}"))
            }
        }
    }

    /// The gate's report and the changes it was about, for a decision.
    fn checked(&self, id: &str) -> Option<(GateReport, Changes)> {
        self.with_active(id, |active| {
            Some((active.record.gate.clone()?, active.changes.clone()?))
        })
        .flatten()
    }

    /// Commits the result of the run `id` although the check did not pass.
    /// Only ever on an explicit request; what was overruled is recorded.
    pub fn keep(&self, id: &str) -> Result<RunStatus, String> {
        let context = self.begin_decision_for(id, "保留", Some(&[Reason::Violations]))?;
        let Some((report, changes)) = self
            .checked(id)
            .filter(|(report, _)| !report.violations.is_empty())
        else {
            self.back_to_decision(id, "keep refused: no usable gate report");
            return Err("还没有可用的检查结果，先重新检查一次".to_string());
        };
        match self.try_commit(id, &context, &changes, &report, Some(Decision::KeptDespiteViolations)) {
            Ok(commit) => {
                let overruled: Vec<String> = report
                    .violations
                    .iter()
                    .map(|violation| violation.message.clone())
                    .collect();
                log::warn!(
                    "run {id}: kept on request despite {} violations",
                    overruled.len()
                );
                let message = format!(
                    "检查没通过（{} 个问题），按你的要求保留并提交了 {}",
                    overruled.len(),
                    short(&commit)
                );
                self.with_active(id, |active| {
                    active.record.kept_despite = overruled;
                    active.record.approved =
                        report.approvals.iter().map(|item| item.message.clone()).collect();
                });
                self.finish(id, Outcome::Kept, message, Some(commit));
                Ok(self.status())
            }
            Err(CommitError::Moved) => {
                self.back_to_decision(id, "keep refused: the files changed since the check");
                // Show what is there now before anything is committed.
                self.recheck(id)?;
                Err("检查之后文件又有变化，已经重新检查，请先看新的结果".to_string())
            }
            Err(CommitError::Failed(error)) => {
                self.back_to_decision(id, &format!("keep failed: {error}"));
                Err(format!("没能提交：{error}"))
            }
        }
    }

    /// Commits the result of the run `id` after the user looked at the
    /// changes to their own layer and agreed (确认并提交). Recorded as
    /// `decision: approved`, with the normal commit message.
    pub fn approve(&self, id: &str) -> Result<RunStatus, String> {
        let context = self.begin_decision_for(id, "确认提交", Some(&[Reason::NeedsApproval]))?;
        let Some((report, changes)) = self
            .checked(id)
            .filter(|(report, _)| report.violations.is_empty() && !report.approvals.is_empty())
        else {
            self.back_to_decision(id, "approve refused: no usable gate report");
            return Err("还没有可用的检查结果，先重新检查一次".to_string());
        };
        let approved: Vec<String> = report.approvals.iter().map(|item| item.message.clone()).collect();
        self.with_active(id, |active| active.record.approved = approved.clone());
        log::info!("run {id}: {} changes confirmed by the user", approved.len());
        self.decided_commit(id, &context, &changes, report, Some(Decision::Approved))
    }

    /// Tries the commit again after git refused it (the check had passed, or
    /// the user had agreed).
    pub fn retry_commit(&self, id: &str) -> Result<RunStatus, String> {
        let context = self.begin_decision_for(id, "重试提交", Some(&[Reason::CommitFailed]))?;
        let decision = self
            .with_active(id, |active| active.record.decision)
            .flatten();
        let Some((report, changes)) = self
            .checked(id)
            .filter(|(report, _)| report.violations.is_empty())
        else {
            self.back_to_decision(id, "retry refused: no usable gate report");
            return Err("还没有可用的检查结果，先重新检查一次".to_string());
        };
        if decision.is_none() && !report.approvals.is_empty() {
            self.back_to_decision(id, "retry refused: the changes were never confirmed");
            return Err("这次的改动还没有经过你确认".to_string());
        }
        self.decided_commit(id, &context, &changes, report, decision)
    }

    fn decided_commit(
        &self,
        id: &str,
        context: &Context,
        changes: &Changes,
        report: GateReport,
        decision: Option<Decision>,
    ) -> Result<RunStatus, String> {
        match self.commit_checked(id, context, changes, report, decision) {
            Checked::Settled => Ok(self.status()),
            Checked::Again => {
                self.back_to_decision(id, "commit refused: the files changed since the check");
                self.recheck(id)?;
                Err("检查之后文件又有变化，已经重新检查，请先看新的结果".to_string())
            }
        }
    }

    /// Checks the run `id` again (after something was fixed by hand, or when
    /// the gate did not answer).
    pub fn recheck(&self, id: &str) -> Result<RunStatus, String> {
        self.begin_decision(id, "重新检查")?;
        self.with_active(id, |active| {
            active.record.phase = Phase::Checking;
            active.record.message = Some("正在重新检查".to_string());
            // Until the new input is ready there is nothing to hand out.
            active.gate_input = None;
        });
        self.publish();
        let service = self.clone();
        let run = id.to_string();
        let spawned = thread::Builder::new()
            .name("run-check".into())
            .spawn(move || service.check(&run));
        if let Err(error) = spawned {
            self.back_to_decision(id, &format!("recheck could not start: {error}"));
            return Err(format!("没能开始重新检查（{error}）"));
        }
        Ok(self.status())
    }

    /// Picks up a run the app was closed in the middle of: its changes are
    /// still in the working tree. The job is taken again and the run is
    /// checked again; it then waits for a decision. Call once at startup.
    pub fn restore(&self) {
        let stale = engine::remove_stale_auth_files(&self.0.dirs.temp);
        if stale > 0 {
            log::warn!("run: removed {stale} temporary auth files left by an earlier session");
        }
        let Some((folder, mut record)) = record::unfinished(&self.0.dirs.runs) else {
            return;
        };
        let id = record.id.clone();
        let abandon = |mut record: RunRecord, message: &str| {
            record.phase = Phase::Finished;
            record.outcome = Some(Outcome::Failed);
            record.message = Some(message.to_string());
            record.ended_at = Some(record::unix_millis());
            if let Err(error) = record::save(&folder, &record) {
                log::warn!("run {}: could not write run.json: {error}", record.id);
            }
            log::warn!("run {}: not picked up again: {message}", record.id);
        };
        let Some(saved) = record::load_snapshot(&folder) else {
            abandon(record, "App 在这次运行开始之前就退出了，什么都没有改动");
            return;
        };
        let root = match (self.0.hooks.root)() {
            Ok(root) => root,
            Err(error) => {
                log::warn!("run {id}: cannot pick it up again now: {error}");
                return;
            }
        };
        if saved.root != root {
            // Its changes are in another folder: it waits until the app
            // works on that folder again, and is not in the way meanwhile.
            log::warn!("run {id}: belongs to another knowledge-base folder, left alone");
            return;
        }
        let Ok(job) = self.0.lock.start_job(
            &format!("等你处理{}的结果", record.task.label()),
            self.0.timing.lock_wait,
        ) else {
            log::warn!("run {id}: cannot pick it up again: the knowledge base is busy");
            return;
        };

        let was_committing = record.phase == Phase::Committing;
        if !record.engine_report.as_ref().is_some_and(|report| report.ok) {
            let report = record.engine_report.get_or_insert_with(|| EngineReport {
                ok: false,
                failure: None,
                exit: Exit::default(),
                stream: StreamSummary::default(),
                duration_ms: 0,
            });
            report
                .failure
                .get_or_insert_with(|| "App 在 AI 还没做完的时候退出了".to_string());
        }
        record.phase = Phase::Checking;
        record.message = Some("App 重新打开了，正在检查上次留下的改动".to_string());
        log::warn!("run {id}: picked up again after a restart");
        // Captures from the earlier session: those saved with the last check,
        // and every one written down the moment it was made.
        let mut beside = saved.beside;
        for noted in record::load_beside(&folder) {
            if !beside.contains(&noted) {
                beside.push(noted);
            }
        }
        write_down_captures(&job, &folder, &id);
        {
            let mut state = self.state();
            let mut active = Active::new(record, folder, root);
            active.snapshot = Some(saved.snapshot);
            active.beside = beside;
            active.job = Some(job);
            state.active = Some(active);
        }
        self.publish();
        // The rest runs git, which needs the login-shell environment: not
        // on the thread that starts the app.
        let service = self.clone();
        let spawned = thread::Builder::new()
            .name("run-check".into())
            .spawn(move || {
                if service.base_is_gone(&id) || (was_committing && service.was_committed(&id)) {
                    return;
                }
                service.check(&id);
            });
        if let Err(error) = spawned {
            log::error!("run: could not start the check of the restored run: {error}");
        }
    }

    /// True (and the run is finished) when the commit the run started from is
    /// not in the repository any more (history was rewritten by hand): then
    /// nothing about the run can be worked out, and it must not block the
    /// knowledge base forever.
    fn base_is_gone(&self, id: &str) -> bool {
        let Some(context) = self.context(id) else {
            return false;
        };
        let object = format!("{}^{{commit}}", context.snapshot.base);
        let present = changes::read_commit(&context.root, self.env(), &object);
        if present {
            return false;
        }
        self.finish(
            id,
            Outcome::Failed,
            "运行开始时的提交已经不在知识库里了，没法核对这次运行留下的改动。请在知识库文件夹里手动检查还没提交的文件".to_string(),
            None,
        );
        true
    }

    /// True (and the run is finished) when the app went away right after it
    /// made the run's commit, before it could write that down. The commit is
    /// recognised by the message written into run.json before it was made
    /// (or by its hash, when that was written down too).
    fn was_committed(&self, id: &str) -> bool {
        let Some(context) = self.context(id) else {
            return false;
        };
        let Some((expected, known, decision, count)) = self.with_active(id, |active| {
            (
                active.record.commit_message.clone(),
                active.record.commit.clone(),
                active.record.decision,
                active
                    .record
                    .gate
                    .as_ref()
                    .map_or(0, |report| report.delete_queue_paths.len()),
            )
        }) else {
            return false;
        };
        let Ok(changes) = changes::collect(
            &context.root,
            self.env(),
            &context.snapshot,
            &self.beside(id),
        ) else {
            return false;
        };
        // Records from before the message was written down: the subject it would have had.
        let subjects: Vec<String> = match &expected {
            Some(message) => vec![message.lines().next().unwrap_or_default().to_string()],
            None => [false, true]
                .into_iter()
                .map(|kept| commit_message(context.task, context.engine, count, kept))
                .collect(),
        };
        let found = changes.foreign_commits.iter().find(|commit| {
            known.as_deref() == Some(commit.hash.as_str())
                || (known.is_none() && subjects.contains(&commit.subject))
        });
        let Some(commit) = found else {
            return false;
        };
        let kept = decision == Some(Decision::KeptDespiteViolations)
            || (expected.is_none() && commit.subject.contains("手动保留"));
        let outcome = if kept { Outcome::Kept } else { Outcome::Committed };
        log::warn!(
            "run {id}: its commit {} was made before the app went away; written down now",
            short(&commit.hash)
        );
        self.finish(
            id,
            outcome,
            "已提交（App 在提交之后退出了，这里补记）".to_string(),
            Some(commit.hash.clone()),
        );
        true
    }

    /// The app is quitting: stop the engine and wait (a bounded time) until
    /// it is gone, so it cannot go on working on the knowledge base alone.
    /// The run's changes stay in the working tree and are picked up at the
    /// next start.
    pub fn shutdown(&self) {
        let running = {
            let mut state = self.state();
            state.quitting = true;
            state.active.as_ref().is_some_and(|active| {
                active.cancel.store(true, Ordering::SeqCst);
                active.engine_running
            })
        };
        if !running {
            return;
        }
        log::warn!("run: the app is quitting, stopping the engine");
        // The run's own thread stops it (SIGTERM, then SIGKILL after the grace period).
        let deadline = Instant::now() + self.0.timing.stop_grace + Duration::from_secs(2);
        while Instant::now() < deadline {
            let gone = !self
                .state()
                .active
                .as_ref()
                .is_some_and(|active| active.engine_running);
            if gone {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        log::error!("run: the engine was still there when the app quit");
    }
}

#[cfg(test)]
mod tests;
