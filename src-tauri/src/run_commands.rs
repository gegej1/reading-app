//! Tauri commands for AI runs (入库, 落裁决). Main window only.
//!
//! | command           | arguments                    | returns / does                                   |
//! |-------------------|------------------------------|--------------------------------------------------|
//! | `run_start`       | `task`, `engine?`, `model?`  | status; starts a run in the background           |
//! | `run_cancel`      | –                            | status; stops the engine                         |
//! | `run_status`      | –                            | `{ phase, run, gateAttempt }`                    |
//! | `run_steps`       | –                            | the newest steps of the current (or last) run    |
//! | `run_gate_input`  | `runId`, `attempt`           | what the gate has to check                       |
//! | `run_gate_report` | `runId`, `attempt`, `report` | hands in the gate's report                       |
//! | `run_discard`     | `runId`                      | status; undoes the run (a copy is kept)          |
//! | `run_keep`        | `runId`                      | status; commits although the check did not pass  |
//! | `run_approve`     | `runId`                      | status; commits changes the user confirmed       |
//! | `run_retry_commit`| `runId`                      | status; tries a refused commit again             |
//! | `run_recheck`     | `runId`                      | status; checks the run's changes again           |
//! | `run_history`     | –                            | the kept runs, newest first                      |
//! | `run_record`      | `runId`                      | one kept run with its steps                      |
//! | `run_settings`    | –                            | the engine settings in effect                    |
//!
//! `task` is `ingest` or `landRulings`; `engine` is `claude` or `codex`.
//! While a run goes on the backend sends `run-status` and `run-step` (see
//! `crate::events`). When a run ends (or waits for a decision) while the main
//! window is not in front, a notification says so; clicking it shows the
//! result. Errors are user-facing Chinese strings.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime, State};

use crate::commands;
use crate::events::{self, KbChanged};
use crate::kb::lock::KbLock;
use crate::kb::sync::{SyncReport, SyncState};
use crate::run::gate::{GateInput, GateReport};
use crate::run::record::{Decision, Outcome, Phase, Reason, RunRecord};
use crate::run::service::{
    Hooks, Notice, RunDetail, RunDirs, RunService, RunStatus, RunTiming, StartRequest, StepEvent,
};
use crate::window;
use crate::run::settings::{self, Overrides, SettingsView};
use crate::run::{EngineKind, Task};
use crate::shell_env::ShellEnvState;
use crate::sync_service::SyncService;

/// The folder of the kept runs inside the app's data folder.
const RUNS_DIR: &str = "runs";

/// The run service of the app: its files live in the app's data folder, its
/// temporary auth file in a private folder of its own below the user's temp
/// folder, and what it has to say goes to the windows as events.
pub fn service<R: Runtime>(app: &AppHandle<R>) -> Result<RunService, String> {
    let data =
        commands::app_dir(app).ok_or_else(|| "找不到 App 的数据文件夹".to_string())?;
    let dirs = RunDirs {
        runs: data.join(RUNS_DIR),
        // The app's temp folder is shared with the webview's caches.
        temp: std::env::temp_dir()
            .join(&app.config().identifier)
            .join("auth"),
    };
    let for_root = app.clone();
    let for_settings = app.clone();
    let for_sync = app.clone();
    let for_notices = app.clone();
    Ok(RunService::new(
        app.state::<KbLock>().inner().clone(),
        app.state::<ShellEnvState>().inner().clone(),
        dirs,
        RunTiming::default(),
        Hooks {
            root: Box::new(move || commands::kb_folder(&for_root)),
            settings: Box::new(move || {
                settings::read(commands::config_file(&for_settings).as_deref())
            }),
            sync: Box::new(move || sync_before_run(&for_sync)),
            notify: Box::new(move |notice| on_notice(&for_notices, notice)),
        },
    ))
}

/// The sync at the start of a run, so captures made elsewhere are in the queue.
fn sync_before_run<R: Runtime>(app: &AppHandle<R>) -> SyncReport {
    app.try_state::<SyncService>()
        .and_then(|service| service.sync_now("run").last)
        .map(|last| last.report)
        .unwrap_or_else(|| SyncReport::stopped(SyncState::NoUpstream, None))
}

/// The end of a run a status announces: the run finished, or it waits for a
/// person to decide about its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ending {
    pub run_id: String,
    /// `Finished` or `NeedsDecision`.
    pub phase: Phase,
    pub title: String,
    pub body: String,
}

/// Longest text of a notification.
const MAX_NOTICE: usize = 140;

/// What a notification says about `status`, if it announces an ending.
pub fn ending(status: &RunStatus) -> Option<Ending> {
    let run: &RunRecord = status.run.as_ref()?;
    let phase = match (status.phase, run.phase) {
        (Phase::NeedsDecision, _) => Phase::NeedsDecision,
        (Phase::Idle, Phase::Finished) => Phase::Finished,
        _ => return None,
    };
    let task = run.task.label();
    let title = match (phase, run.outcome) {
        (Phase::NeedsDecision, _) => match run.reason {
            Some(Reason::NeedsApproval) => format!("{task}有改动需要你确认"),
            Some(Reason::CommitFailed) => format!("{task}检查通过了，但没能提交"),
            _ => format!("{task}没通过检查，等你处理"),
        },
        (_, Some(Outcome::Committed)) if run.decision == Some(Decision::Approved) => {
            format!("{task}已确认提交")
        }
        (_, Some(Outcome::Committed)) => format!("{task}完成了"),
        (_, Some(Outcome::Kept)) => format!("{task}已按你的要求提交"),
        (_, Some(Outcome::Discarded)) => format!("{task}的改动已经丢弃"),
        (_, Some(Outcome::NothingToDo)) => format!("{task}：队列是空的"),
        (_, Some(Outcome::Refused)) => format!("{task}没有开始"),
        (_, Some(Outcome::Cancelled)) => format!("{task}已停止"),
        _ => format!("{task}没有完成"),
    };
    let message = run.message.clone().unwrap_or_default();
    let mut body: String = message.chars().take(MAX_NOTICE).collect();
    if message.chars().count() > MAX_NOTICE {
        body.push('…');
    }
    Some(Ending {
        run_id: run.id.clone(),
        phase,
        title,
        body,
    })
}

/// The last ending a notification was considered for, so each is told once.
#[derive(Default)]
pub struct EndNotices(Mutex<Option<(String, Phase)>>);

impl EndNotices {
    /// True the first time `ending` is seen.
    pub fn first_time(&self, ending: &Ending) -> bool {
        let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (ending.run_id.clone(), ending.phase);
        if last.as_ref() == Some(&key) {
            return false;
        }
        *last = Some(key);
        true
    }
}

/// Tells the user with a notification that a run ended, unless they are
/// looking at the main window anyway.
fn notify_ending<R: Runtime>(app: &AppHandle<R>, status: &RunStatus) {
    use tauri_plugin_notification::NotificationExt;
    let Some(ending) = ending(status) else { return };
    let first = app
        .try_state::<EndNotices>()
        .is_some_and(|seen| seen.first_time(&ending));
    if !first {
        return;
    }
    let in_front = app.get_webview_window(window::MAIN).is_some_and(|main| {
        main.is_visible().unwrap_or(false) && main.is_focused().unwrap_or(false)
    });
    if in_front || app.try_state::<tauri_plugin_notification::Notification<R>>().is_none() {
        return;
    }
    match app
        .notification()
        .builder()
        .title(&ending.title)
        .body(&ending.body)
        .show()
    {
        Ok(()) => log::info!("run {}: notification sent", ending.run_id),
        Err(error) => log::warn!("run {}: notification failed: {error}", ending.run_id),
    }
}

fn on_notice<R: Runtime>(app: &AppHandle<R>, notice: Notice) {
    match notice {
        Notice::Status(status) => {
            events::run_status(app, &status);
            notify_ending(app, &status);
        }
        Notice::Step(step) => events::run_step(app, &step),
        // A commit or a discard: the windows reload and a sync follows.
        Notice::Ended {
            files_changed: true,
        } => events::files_written(app, KbChanged::new("run", None, None)),
        // Nothing changed, but captures made meanwhile still wait to be pushed.
        Notice::Ended {
            files_changed: false,
        } => {
            if let Some(service) = app.try_state::<SyncService>() {
                service.schedule(Duration::from_secs(1), "run");
            }
        }
    }
}

/// What a run was asked to use, from the texts of a command or the command line.
pub fn overrides(engine: Option<&str>, model: Option<&str>) -> Result<Overrides, String> {
    let engine = match engine.map(str::trim).filter(|engine| !engine.is_empty()) {
        Some(name) => Some(
            EngineKind::parse(name)
                .ok_or_else(|| format!("不认识的引擎：{name}（只能是 claude 或 codex）"))?,
        ),
        None => None,
    };
    Ok(Overrides {
        engine,
        model: model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(String::from),
    })
}

/// Starts a run for the command line and logs whether it began; the run
/// itself logs its steps and its outcome.
pub fn start_from_command_line<R: Runtime>(
    app: &AppHandle<R>,
    task: Task,
    engine: Option<&str>,
    model: Option<&str>,
) {
    let result = overrides(engine, model).and_then(|overrides| {
        let service = app
            .try_state::<RunService>()
            .ok_or("运行服务没有启动".to_string())?;
        service.start(StartRequest {
            task,
            overrides,
            trigger: "command line",
        })
    });
    match result {
        Ok(status) => log::info!(
            "command line {}: started as run {}",
            task.slug(),
            status.run.map(|run| run.id).unwrap_or_default()
        ),
        Err(error) => log::warn!("command line {}: not started: {error}", task.slug()),
    }
}

/// `--approve`: commits the run result that waits for the user's
/// confirmation, like 确认并提交 in the app. Logs what happened.
pub fn approve_from_command_line<R: Runtime>(app: &AppHandle<R>) {
    let Some(service) = app.try_state::<RunService>() else {
        log::warn!("command line approve: the run service is not running");
        return;
    };
    let status = service.status();
    let waiting = status.run.filter(|run| {
        status.phase == Phase::NeedsDecision && run.reason == Some(Reason::NeedsApproval)
    });
    let Some(run) = waiting else {
        log::warn!("command line approve: no run result waits for confirmation");
        return;
    };
    match service.approve(&run.id) {
        Ok(after) => log::info!(
            "command line approve: run {} confirmed: phase={:?} outcome={:?} commit={}",
            run.id,
            after.phase,
            after.run.as_ref().and_then(|run| run.outcome),
            after
                .run
                .as_ref()
                .and_then(|run| run.commit.as_deref())
                .unwrap_or("-"),
        ),
        Err(error) => log::warn!("command line approve: run {}: {error}", run.id),
    }
}

/// Runs blocking work off the async runtime and logs how the command went.
async fn call<T, F>(command: &'static str, run: Option<String>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("后台任务失败：{error}"))
        .and_then(|result| result);
    let run = run.map(|id| format!(" run={id}")).unwrap_or_default();
    let millis = started.elapsed().as_millis();
    match &result {
        Ok(_) => log::info!("{command}{run} ok ({millis} ms)"),
        Err(error) => log::warn!("{command}{run} failed ({millis} ms): {error}"),
    }
    result
}

#[tauri::command]
pub async fn run_start(
    service: State<'_, RunService>,
    task: String,
    engine: Option<String>,
    model: Option<String>,
) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_start", None, move || {
        let task = Task::parse(&task).ok_or_else(|| format!("不认识的任务：{task}"))?;
        service.start(StartRequest {
            task,
            overrides: overrides(engine.as_deref(), model.as_deref())?,
            trigger: "ui",
        })
    })
    .await
}

#[tauri::command]
pub async fn run_cancel(service: State<'_, RunService>) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_cancel", None, move || service.cancel()).await
}

#[tauri::command]
pub fn run_status(service: State<'_, RunService>) -> RunStatus {
    service.status()
}

#[tauri::command]
pub fn run_steps(service: State<'_, RunService>) -> Vec<StepEvent> {
    service.steps()
}

#[tauri::command]
pub async fn run_gate_input(
    service: State<'_, RunService>,
    run_id: String,
    attempt: u32,
) -> Result<GateInput, String> {
    let service = service.inner().clone();
    call("run_gate_input", Some(run_id.clone()), move || {
        service
            .gate_input(&run_id, attempt)
            .map(|input| GateInput::clone(&input))
    })
    .await
}

#[tauri::command]
pub async fn run_gate_report(
    service: State<'_, RunService>,
    run_id: String,
    attempt: u32,
    report: GateReport,
) -> Result<(), String> {
    let service = service.inner().clone();
    call("run_gate_report", Some(run_id.clone()), move || {
        service.gate_report(&run_id, attempt, report)
    })
    .await
}

#[tauri::command]
pub async fn run_discard(
    service: State<'_, RunService>,
    run_id: String,
) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_discard", Some(run_id.clone()), move || {
        service.discard(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_keep(service: State<'_, RunService>, run_id: String) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_keep", Some(run_id.clone()), move || {
        service.keep(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_approve(
    service: State<'_, RunService>,
    run_id: String,
) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_approve", Some(run_id.clone()), move || {
        service.approve(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_retry_commit(
    service: State<'_, RunService>,
    run_id: String,
) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_retry_commit", Some(run_id.clone()), move || {
        service.retry_commit(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_recheck(
    service: State<'_, RunService>,
    run_id: String,
) -> Result<RunStatus, String> {
    let service = service.inner().clone();
    call("run_recheck", Some(run_id.clone()), move || {
        service.recheck(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_history(service: State<'_, RunService>) -> Result<Vec<RunRecord>, String> {
    let service = service.inner().clone();
    call("run_history", None, move || Ok(service.history())).await
}

#[tauri::command]
pub async fn run_record(service: State<'_, RunService>, run_id: String) -> Result<RunDetail, String> {
    let service = service.inner().clone();
    call("run_record", Some(run_id.clone()), move || {
        service.detail(&run_id)
    })
    .await
}

#[tauri::command]
pub async fn run_settings<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
) -> Result<SettingsView, String> {
    let config: Option<PathBuf> = commands::config_file(&app);
    let shell_env = shell_env.inner().clone();
    call("run_settings", None, move || {
        let (settings, problems) = settings::read(config.as_deref());
        // Waits for the login-shell environment if it is still being captured.
        let env = &shell_env.get().env;
        Ok(SettingsView::new(&settings, problems, |program| {
            env.find_program(program)
                .map(|path| path.to_string_lossy().into_owned())
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{EngineKind, Task};

    fn status(phase: Phase, run_phase: Phase, outcome: Option<Outcome>) -> RunStatus {
        let mut run = RunRecord::new(
            "2026-10-12T14-30-05-ingest".into(),
            Task::Ingest,
            EngineKind::Claude,
            None,
            "ui",
            "2026-10-12".into(),
        );
        run.phase = run_phase;
        run.outcome = outcome;
        run.message = Some("通过检查：3 条随手记都逐字进了原料层，提交 5 个文件，已提交 abc1234567".into());
        RunStatus {
            phase,
            run: Some(run),
            gate_attempt: 0,
        }
    }

    #[test]
    fn a_notification_announces_each_ending_once() {
        assert_eq!(ending(&status(Phase::Running, Phase::Running, None)), None);
        assert_eq!(ending(&status(Phase::Checking, Phase::Checking, None)), None);
        let done = ending(&status(Phase::Idle, Phase::Finished, Some(Outcome::Committed))).unwrap();
        assert_eq!(done.title, "入库完成了");
        assert!(done.body.starts_with("通过检查"));
        assert_eq!(done.phase, Phase::Finished);
        let waiting = ending(&status(Phase::NeedsDecision, Phase::NeedsDecision, None)).unwrap();
        assert_eq!(waiting.title, "入库没通过检查，等你处理");
        let mut asking = status(Phase::NeedsDecision, Phase::NeedsDecision, None);
        asking.run.as_mut().unwrap().reason = Some(Reason::NeedsApproval);
        assert_eq!(ending(&asking).unwrap().title, "入库有改动需要你确认");
        let mut refused = status(Phase::NeedsDecision, Phase::NeedsDecision, None);
        refused.run.as_mut().unwrap().reason = Some(Reason::CommitFailed);
        assert_eq!(ending(&refused).unwrap().title, "入库检查通过了，但没能提交");
        let mut approved = status(Phase::Idle, Phase::Finished, Some(Outcome::Committed));
        approved.run.as_mut().unwrap().decision = Some(Decision::Approved);
        assert_eq!(ending(&approved).unwrap().title, "入库已确认提交");
        let failed = ending(&status(Phase::Idle, Phase::Finished, Some(Outcome::Failed))).unwrap();
        assert_eq!(failed.title, "入库没有完成");
        // Idle without a run that ended: nothing to tell.
        assert_eq!(
            ending(&RunStatus {
                phase: Phase::Idle,
                run: None,
                gate_attempt: 0
            }),
            None
        );

        let seen = EndNotices::default();
        assert!(seen.first_time(&waiting));
        assert!(!seen.first_time(&waiting));
        assert!(seen.first_time(&done));
        assert!(!seen.first_time(&done));

        let mut long = status(Phase::Idle, Phase::Finished, Some(Outcome::Failed));
        long.run.as_mut().unwrap().message = Some("很长".repeat(100));
        let body = ending(&long).unwrap().body;
        assert_eq!(body.chars().count(), MAX_NOTICE + 1);
        assert!(body.ends_with('…'));
    }

    #[test]
    fn reads_the_engine_and_model_a_run_was_asked_for() {
        assert_eq!(overrides(None, None), Ok(Overrides::default()));
        assert_eq!(
            overrides(Some(" Codex "), Some(" gpt-x ")),
            Ok(Overrides {
                engine: Some(EngineKind::Codex),
                model: Some("gpt-x".into())
            })
        );
        assert_eq!(overrides(Some(""), Some("  ")), Ok(Overrides::default()));
        let error = overrides(Some("gemini"), None).unwrap_err();
        assert!(error.contains("gemini") && error.contains("claude"), "{error}");
    }
}
