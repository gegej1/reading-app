//! Tauri desktop entry point for 底色 (Dise).
//!
//! The shell opens the main window and exposes the knowledge-base commands
//! (`kb_root`, `kb_list`, `kb_read`, `kb_read_head`, `kb_write`, `kb_create`,
//! `kb_status`, `kb_sync`, see `commands`), the ones for AI runs (see
//! `run_commands`) and a few for the desktop shell (see `desktop`).
//! Around them:
//!
//! - `shell_env`: the user's login-shell environment, captured once at
//!   startup and used for every child process (git);
//! - `kb::lock`: one knowledge-base operation at a time (writes, captures,
//!   syncs, and later long-running jobs), with a clear "busy" answer;
//! - `sync_service`: when the knowledge base is synced with its remote;
//! - `capture_window`, `shortcut`, `tray`: quick capture (记一笔) from a
//!   floating slip, a global shortcut and the menu-bar icon;
//! - `run`, `run_commands`: AI runs (入库, 落裁决): an engine edits the
//!   knowledge base, the gate checks the result, the app commits it; a
//!   notification when one ends while the main window is in the background;
//! - `bridge`, `bridge_commands`: the iPhone bridge through iCloud Drive
//!   (captures and answers from the phone come in, the 底色 layer goes out);
//! - `settings`, `app_config`: the settings screen (engine, model, budget,
//!   time limit, knowledge-base folder, capture shortcut, launch at login)
//!   and saving `config.json`;
//! - `cli`: `--capture "<text>"`, `--sync`, `--ingest` and `--land-rulings`,
//!   also handed to a running instance by the single-instance plugin;
//! - `events`: `kb-changed`, `sync-status`, `run-status` and `run-step` for
//!   the windows;
//! - `process_group`: stopping a child process together with what it started;
//! - `logging`: stdout plus a rotated file in `~/Library/Logs/com.gegej1.dise/`;
//! - `window`: closing the window hides it, the Dock icon shows it, ⌘Q quits;
//! - opener: http(s) links open in the default browser.

mod app_config;
mod bridge;
mod bridge_commands;
mod capture_window;
mod cli;
mod commands;
mod desktop;
mod events;
mod kb;
mod logging;
mod process_group;
mod run;
mod run_commands;
mod settings;
pub mod shell_env;
mod shortcut;
mod sync_service;
mod tray;
mod window;

use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};

use crate::cli::CliRequest;
use crate::events::KbChanged;
use crate::kb::capture::{self, LocalTime};
use crate::kb::lock::KbLock;
use crate::kb::root;
use crate::kb::sync::Timeouts;
use crate::run::Task;
use crate::shell_env::ShellEnvState;
use crate::sync_service::{Notice, SyncService, SyncTiming};
use crate::tray::Action;

/// Builds the desktop app from `tauri.conf.json` and runs it until exit.
pub fn run() {
    // What this launch was asked to do. A second launch hands its arguments
    // to the running instance instead (see the single-instance plugin below).
    let request = cli::parse(std::env::args().skip(1));

    let builder = tauri::Builder::default()
        // Registered first, so a second launch hands over to the running app
        // and exits before any other plugin starts.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            on_second_launch(app, args);
        }))
        .plugin(logging::plugin())
        // Links are opened explicitly by the frontend (http and https only, see
        // capabilities/default.json), so the plugin's click handler stays off.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        // Shortcuts are registered in `setup`, where a failure can be reported
        // instead of stopping the app.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Used from Rust only: a notification when a run ends in the background,
        // the folder picker of the settings, and launch at login (off until the
        // user turns it on; a Launch Agent named after the app's identifier).
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("com.gegej1.dise")
                .build(),
        )
        .on_window_event(window::on_window_event)
        .setup(move |app| {
            setup(app.handle(), &request);
            Ok(())
        });

    register(builder, KbLock::default(), ShellEnvState::default())
        .build(tauri::generate_context!())
        .expect("failed to build the Dise desktop app")
        .run(window::on_run_event);
}

/// Starts everything around the windows: the login-shell capture, syncing,
/// the shortcut, the tray icon, and what the command line asked for.
fn setup<R: Runtime>(app: &AppHandle<R>, request: &CliRequest) {
    log::info!(
        "Dise {} starting (pid {})",
        app.package_info().version,
        std::process::id()
    );
    // Capture the login-shell environment in the background now, so the
    // first git command does not have to wait for it.
    app.state::<ShellEnvState>().warm_up();

    let service = sync_service(app);
    app.manage(service.clone());
    // After the sync service (a run syncs first), before the first sync: a
    // run the app was closed in the middle of takes its job again, so its
    // changes are neither pushed nor edited before someone decided on them.
    match run_commands::service(app) {
        Ok(runs) => {
            runs.restore();
            app.manage(runs);
        }
        Err(error) => log::error!("AI runs are not available: {error}"),
    }

    let setting = commands::config_file(app)
        .and_then(|file| root::read_config_string(&file, root::SHORTCUT_KEY));
    let shortcut = shortcut::register(app, setting, capture_window::toggle);
    if let Err(error) = tray::create(app, &shortcut, on_tray_action) {
        log::error!("tray icon could not be created: {error}");
    }
    app.manage(shortcut::ShortcutHolder::new(shortcut));
    on_notification_click(app);

    service.start();
    // The iPhone bridge: sets up its folder once the login shell is ready,
    // imports what the phone wrote, and keeps watching.
    let bridge = bridge_commands::service(app);
    app.manage(bridge.clone());
    bridge.start();
    perform(app, request);
    if request.has_action() {
        log::info!("started for the command line: staying in the menu bar");
    } else {
        window::show_main(app);
    }
}

/// A click on a notification about a run shows the main window with the
/// run's result.
fn on_notification_click<R: Runtime>(app: &AppHandle<R>) {
    use tauri_plugin_notification::NotificationExt;
    if app
        .try_state::<tauri_plugin_notification::Notification<R>>()
        .is_none()
    {
        return;
    }
    let handle = app.clone();
    let registered = app.notification().on_action(move |performed| {
        if performed.action_id() == "tap" {
            log::info!("notification clicked: showing the result of the run");
            window::show_main(&handle);
            events::run_show(&handle);
        }
    });
    if let Err(error) = registered {
        log::warn!("notification clicks are not handled: {error}");
    }
}

/// The sync service, reporting to the windows and the tray.
fn sync_service<R: Runtime>(app: &AppHandle<R>) -> SyncService {
    let for_root = app.clone();
    let for_notices = app.clone();
    SyncService::new(
        app.state::<KbLock>().inner().clone(),
        app.state::<ShellEnvState>().inner().clone(),
        Timeouts::default(),
        SyncTiming::default(),
        move || commands::kb_folder(&for_root),
        move |notice| match notice {
            Notice::Status(status) => {
                tray::set_sync(&for_notices, sync_service::tray_label(&status));
                events::sync_status(&for_notices, &status);
            }
            Notice::FilesChanged => {
                events::kb_changed(&for_notices, &KbChanged::new("sync", None, None));
            }
        },
    )
}

fn on_tray_action<R: Runtime>(app: &AppHandle<R>, action: Action) {
    match action {
        Action::Capture => capture_window::show(app),
        Action::Open => window::show_main(app),
        Action::Sync => {
            if let Some(service) = app.try_state::<SyncService>() {
                service.schedule(Duration::ZERO, "menu");
            }
        }
        Action::Quit => {
            log::info!("quit from the tray menu");
            app.exit(0);
        }
    }
}

/// The app was launched again while running: do what that launch asked for,
/// or show the main window.
fn on_second_launch<R: Runtime>(app: &AppHandle<R>, args: Vec<String>) {
    let request = cli::parse(args.into_iter().skip(1));
    log::info!(
        "second launch handed over to this instance (capture={}, answer={}, sync={}, run={}, approve={})",
        request.capture.is_some(),
        request.answer.is_some(),
        request.sync,
        request.run.is_some(),
        request.approve
    );
    perform(app, &request);
    if !request.has_action() {
        window::show_main(app);
    }
}

/// Runs the command-line actions.
fn perform<R: Runtime>(app: &AppHandle<R>, request: &CliRequest) {
    for argument in &request.unknown {
        log::warn!("command line: ignored the argument {argument:?}");
    }
    let run = match &request.run {
        Some(Ok(task)) => Some(*task),
        Some(Err(error)) => {
            log::warn!("command line run: not started: {error}");
            None
        }
        None => None,
    };
    let capture = match &request.capture {
        Some(Ok(text)) => Some((text.clone(), LocalTime::now())),
        Some(Err(error)) => {
            log::warn!("command line capture: {error}");
            None
        }
        None => None,
    };
    let answer = match &request.answer {
        Some(Ok((id, text))) => Some((id.clone(), text.clone(), LocalTime::now())),
        Some(Err(error)) => {
            log::warn!("command line answer: {error}");
            None
        }
        None => None,
    };
    if capture.is_some() || answer.is_some() || run.is_some() {
        let app = app.clone();
        let engine = request.engine.clone();
        let model = request.model.clone();
        // One thread, so a capture or an answer given together with a run is
        // in the queue before the run looks at it.
        let spawned = std::thread::Builder::new()
            .name("command-line".into())
            .spawn(move || {
                if let Some((text, at)) = &capture {
                    capture_from_command_line(&app, text, at);
                }
                if let Some((id, text, at)) = &answer {
                    answer_from_command_line(&app, id, text, at);
                }
                if let Some(task) = run {
                    run_from_command_line(&app, task, engine.as_deref(), model.as_deref());
                }
            });
        if let Err(error) = spawned {
            log::error!("command line: could not start ({error})");
        }
    }
    if request.sync {
        if let Some(service) = app.try_state::<SyncService>() {
            service.schedule(Duration::ZERO, "command line");
        }
        // And whatever the iPhone left in the bridge folder.
        if let Some(bridge) = app.try_state::<bridge::BridgeService>() {
            bridge.import_now();
        }
    }
    if request.approve {
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("command-line-approve".into())
            .spawn(move || run_commands::approve_from_command_line(&app));
        if let Err(error) = spawned {
            log::error!("command line approve: could not start ({error})");
        }
    }
}

/// Starts the run `--ingest` or `--land-rulings` asked for. The run logs its
/// steps and its outcome itself.
fn run_from_command_line<R: Runtime>(
    app: &AppHandle<R>,
    task: Task,
    engine: Option<&str>,
    model: Option<&str>,
) {
    run_commands::start_from_command_line(app, task, engine, model);
}

/// Saves a capture from `--capture` exactly like the capture window does:
/// a new file in `inbox/`, committed on its own. Logs the outcome, never the text.
fn capture_from_command_line<R: Runtime>(app: &AppHandle<R>, text: &str, at: &LocalTime) {
    let started = Instant::now();
    let result = (|| {
        let built = capture::build_capture(text, capture::MAC, at)?;
        let root = commands::kb_folder(app)?;
        let env = &app.state::<ShellEnvState>().inner().get().env;
        // Also while an AI run is going on: a capture is its own commit.
        let guard = app
            .state::<KbLock>()
            .acquire_beside_job(commands::WRITE_OPERATION, commands::WRITE_WAIT)
            .map_err(|busy| busy.to_string())?;
        let saved = capture::save_capture(&root, env, &built)?;
        if let Some(commit) = &saved.outcome.commit {
            guard.note_commit(commit, &saved.path);
        }
        Ok::<_, String>(saved)
    })();
    let millis = started.elapsed().as_millis();
    match result {
        Ok(saved) => {
            let commit = saved
                .outcome
                .commit
                .as_deref()
                .map(|hash| &hash[..hash.len().min(10)])
                .unwrap_or("-");
            log::info!(
                "command line capture saved: path={:?} commit={commit} ({millis} ms)",
                saved.path
            );
            events::files_written(app, KbChanged::new("capture", Some(saved.path), None));
        }
        Err(error) => log::warn!("command line capture failed ({millis} ms): {error}"),
    }
}

/// Saves an answer from `--answer` exactly like the answer box of a ruling
/// does: a new file in `inbox/裁决/`, committed on its own (beside an ingest,
/// but not while a 落裁决 works through that folder). Logs the outcome,
/// never the text.
fn answer_from_command_line<R: Runtime>(app: &AppHandle<R>, id: &str, text: &str, at: &LocalTime) {
    let started = Instant::now();
    let result = (|| {
        let built = capture::build_answer(id, text, capture::MAC, at)?;
        let root = commands::kb_folder(app)?;
        let env = &app.state::<ShellEnvState>().inner().get().env;
        let running = app
            .try_state::<run::service::RunService>()
            .and_then(|runs| runs.active_task());
        let lock = app.state::<KbLock>();
        let guard = if commands::creates_beside_run(&built.path, running) {
            lock.acquire_beside_job(commands::WRITE_OPERATION, commands::WRITE_WAIT)
        } else {
            lock.acquire(commands::WRITE_OPERATION, commands::WRITE_WAIT)
        }
        .map_err(|busy| busy.to_string())?;
        let saved = capture::save_capture(&root, env, &built)?;
        if let Some(commit) = &saved.outcome.commit {
            guard.note_commit(commit, &saved.path);
        }
        Ok::<_, String>(saved)
    })();
    let millis = started.elapsed().as_millis();
    match result {
        Ok(saved) => {
            log::info!(
                "command line answer saved: path={:?} ({millis} ms)",
                saved.path
            );
            events::files_written(app, KbChanged::new("capture", Some(saved.path), None));
        }
        Err(error) => log::warn!("command line answer failed ({millis} ms): {error}"),
    }
}

/// Adds the commands and the state they need to a builder.
///
/// Shared by [`run`] and the IPC tests, which use Tauri's mock runtime (and
/// must not start the single-instance plugin or the login shell).
fn register<R: Runtime>(
    builder: tauri::Builder<R>,
    lock: KbLock,
    shell_env: ShellEnvState,
) -> tauri::Builder<R> {
    builder
        .manage(lock)
        .manage(shell_env)
        .manage(capture_window::PreviousApp::default())
        .manage(run_commands::EndNotices::default())
        .invoke_handler(tauri::generate_handler![
            commands::kb_root,
            commands::kb_list,
            commands::kb_read,
            commands::kb_read_head,
            commands::kb_write,
            commands::kb_create,
            commands::kb_status,
            commands::kb_sync,
            desktop::capture_show,
            desktop::capture_hide,
            desktop::tray_set_counts,
            desktop::desktop_info,
            run_commands::run_start,
            run_commands::run_cancel,
            run_commands::run_status,
            run_commands::run_steps,
            run_commands::run_gate_input,
            run_commands::run_gate_report,
            run_commands::run_discard,
            run_commands::run_keep,
            run_commands::run_approve,
            run_commands::run_retry_commit,
            run_commands::run_recheck,
            run_commands::run_history,
            run_commands::run_record,
            run_commands::run_settings,
            settings::settings_get,
            settings::settings_save_ai,
            settings::settings_check_kb,
            settings::settings_pick_kb,
            settings::settings_set_kb,
            settings::settings_set_shortcut,
            settings::settings_set_autostart,
            settings::settings_open_folder,
            settings::settings_set_bridge,
            bridge_commands::bridge_export,
        ])
}

#[cfg(test)]
mod ipc_tests {
    //! Drives the real command handlers through Tauri's IPC layer with the
    //! mock runtime and the app's real config and capabilities. This proves
    //! the commands are registered, their arguments deserialize, and the ACL
    //! gives each window exactly what its capability grants.

    use std::time::Duration;

    use serde_json::{json, Value};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

    use crate::kb::lock::KbLock;
    use crate::kb::sync::{SyncReport, SyncState, Timeouts};
    use crate::kb::test_support::{commit_count, git, init_repo, launcher, TempDir};
    use crate::run::service::{Hooks, RunDirs, RunService, RunTiming};
    use crate::run::settings::RunSettings;
    use crate::shell_env::{ChildEnv, Origin, ShellEnv, ShellEnvState};
    use crate::sync_service::{SyncService, SyncTiming};

    fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(window, request).map(|body| body.deserialize::<Value>().unwrap())
    }

    fn denied(result: Result<Value, Value>) -> String {
        let error = result.expect_err("should have been refused");
        error.as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn commands_work_through_ipc_with_each_windows_capability() {
        let kb = TempDir::new("ipc");
        init_repo(kb.path());
        kb.write("self/底色.md", "### D01 示例\n");
        kb.write("AGENTS.md", "# 规则\n");
        git(kb.path(), &["add", "--", "self/底色.md", "AGENTS.md"]);
        git(kb.path(), &["commit", "-q", "-m", "init"]);

        // The only test that touches the process environment. The app's own
        // folder (config.json, run records) is a temporary one too, so the
        // test never reads or writes the real settings.
        std::env::set_var(crate::kb::root::ENV_VAR, kb.path());
        let app_dir = TempDir::new("ipc-app-dir");
        std::env::set_var(crate::commands::APP_DIR_VAR, app_dir.path());
        app_dir.write("config.json", r#"{"kept": {"by": "hand"}, "budgetUsd": 3}"#);

        // The test process already has a working git: use its environment
        // instead of running the user's login shell.
        let shell_env = ShellEnvState::ready(ShellEnv {
            env: ChildEnv::inherit(),
            origin: Origin::LoginShell,
            took: Duration::ZERO,
        });
        let lock = KbLock::default();

        // `test = true` skips embedding Info.plist a second time, but keeps the
        // real config, capabilities and ACL.
        let app = super::register(mock_builder(), lock.clone(), shell_env.clone())
            .build(tauri::generate_context!(test = true))
            .expect("build mock app");
        let folder = kb.path().to_path_buf();
        app.manage(SyncService::new(
            lock.clone(),
            shell_env,
            Timeouts::default(),
            SyncTiming::default(),
            move || Ok(folder.clone()),
            |_| {},
        ));
        // AI runs, with a fake engine and a home folder of their own (so the
        // test never reads the real login settings).
        let scratch = TempDir::new("ipc-run");
        let scenario = scratch.write(
            "fake-claude.sh",
            concat!(
                "mkdir -p raw/inbox reports\n",
                "printf '# 收件箱\\n\\n一条\\n\\n第二条\\n' > raw/inbox/新-收件箱.md\n",
                "printf '# 入库报告\\n' > reports/新-入库报告.md\n",
                "if [ -f \"$DISE_FAKE_SECOND\" ]; then printf '\\n### D02 没标草稿的新条目\\n> 原话\\n' >> self/底色.md; fi\n",
                "printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{}\",\"total_cost_usd\":0.1,\"num_turns\":2}'\n",
            ),
        );
        let engine = launcher("fake-claude", "DISE_FAKE_CLAUDE");
        let run_env = ShellEnvState::ready(ShellEnv {
            env: ChildEnv::from_vars([
                ("PATH".to_string(), std::env::var("PATH").unwrap_or_default()),
                (
                    "HOME".to_string(),
                    scratch.path().join("home").to_string_lossy().into_owned(),
                ),
                (
                    "DISE_FAKE_CLAUDE".to_string(),
                    scenario.to_string_lossy().into_owned(),
                ),
                (
                    "DISE_FAKE_SECOND".to_string(),
                    scratch.path().join("second").to_string_lossy().into_owned(),
                ),
            ]),
            origin: Origin::LoginShell,
            took: Duration::ZERO,
        });
        let run_root = kb.path().to_path_buf();
        let engine_path = engine.to_string_lossy().into_owned();
        app.manage(RunService::new(
            lock,
            run_env,
            RunDirs {
                runs: scratch.path().join("runs"),
                temp: scratch.path().join("tmp"),
            },
            RunTiming::default(),
            Hooks {
                root: Box::new(move || Ok(run_root.clone())),
                settings: Box::new(move || {
                    (
                        RunSettings {
                            claude_path: Some(engine_path.clone()),
                            ..RunSettings::default()
                        },
                        Vec::new(),
                    )
                }),
                sync: Box::new(|| SyncReport::stopped(SyncState::NoUpstream, None)),
                notify: Box::new(|_| {}),
            },
        ));
        let main = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main window");

        let info = invoke(&main, "kb_root", json!({})).unwrap();
        assert_eq!(info["source"], "env");
        assert_eq!(info["exists"], true);
        assert_eq!(info["path"], kb.path().to_string_lossy().as_ref());

        let list = invoke(&main, "kb_list", json!({})).unwrap();
        assert_eq!(list[0]["path"], "AGENTS.md");
        assert_eq!(list[1]["path"], "self/底色.md");
        assert!(list[1]["mtime"].is_u64());

        let text = invoke(&main, "kb_read", json!({ "path": "self/底色.md" })).unwrap();
        assert_eq!(text, "### D01 示例\n");

        let outcome = invoke(
            &main,
            "kb_write",
            json!({
                "path": "self/底色.md",
                "content": "### D01 示例 ✅\n",
                "message": "底色：确认 D01",
            }),
        )
        .unwrap();
        assert_eq!(outcome["committed"], true);
        assert!(outcome["commit"].is_string());
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(
            std::fs::read_to_string(kb.path().join("self/底色.md")).unwrap(),
            "### D01 示例 ✅\n"
        );

        // Creating never overwrites.
        let created = invoke(
            &main,
            "kb_create",
            json!({
                "path": "inbox/2026-10-12T14-03-22-mac.md",
                "content": "---\n来源: mac\n---\n\n一条\n",
                "message": "收件箱：记一笔（mac）",
            }),
        )
        .unwrap();
        assert_eq!(created["committed"], true);
        assert_eq!(commit_count(kb.path()), 3);
        let exists = denied(invoke(
            &main,
            "kb_create",
            json!({ "path": "self/底色.md", "content": "x", "message": "m" }),
        ));
        assert!(exists.starts_with("文件已存在："), "{exists}");
        assert_eq!(commit_count(kb.path()), 3);

        // Unsafe paths are rejected with a readable message, before any I/O.
        let rejected = denied(invoke(&main, "kb_read", json!({ "path": "../secret.md" })));
        assert!(rejected.contains(".."), "{rejected}");
        let rejected = denied(invoke(
            &main,
            "kb_write",
            json!({ "path": ".git/config", "content": "x", "message": "坏" }),
        ));
        assert!(rejected.contains(".git"), "{rejected}");
        assert_eq!(commit_count(kb.path()), 3);

        // Missing arguments fail to deserialize instead of running.
        let missing = denied(invoke(&main, "kb_read", json!({})));
        assert!(missing.contains("missing required key path"), "{missing}");

        // Sync status and a sync: this repository has no remote.
        let status = invoke(&main, "kb_status", json!({})).unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(status["repo"]["upstream"], Value::Null);
        let synced = invoke(&main, "kb_sync", json!({})).unwrap();
        assert_eq!(synced["last"]["state"], "noUpstream");
        assert_eq!(synced["last"]["reason"], "manual");

        // Desktop helpers: no tray and no shortcut in the mock app.
        invoke(
            &main,
            "tray_set_counts",
            json!({ "inbox": 1, "rulings": 2 }),
        )
        .unwrap();
        let desktop = invoke(&main, "desktop_info", json!({})).unwrap();
        assert_eq!(desktop, json!({ "captureShortcut": null }));

        // What a file was at the last commit, whatever the working tree says.
        std::fs::write(kb.path().join("self/底色.md"), "工作区里改了，还没提交\n").unwrap();
        let head = invoke(&main, "kb_read_head", json!({ "path": "self/底色.md" })).unwrap();
        assert_eq!(head, "### D01 示例 ✅\n");
        let none = invoke(&main, "kb_read_head", json!({ "path": "self/没有.md" })).unwrap();
        assert_eq!(none, Value::Null);
        let rejected = denied(invoke(&main, "kb_read_head", json!({ "path": ".git/config" })));
        assert!(rejected.contains(".git"), "{rejected}");
        git(kb.path(), &["checkout", "-q", "--", "self/底色.md"]);

        // AI runs: nothing is going on yet.
        let idle = invoke(&main, "run_status", json!({})).unwrap();
        assert_eq!(idle, json!({ "phase": "idle", "run": null, "gateAttempt": 0 }));
        assert_eq!(invoke(&main, "run_steps", json!({})).unwrap(), json!([]));
        assert_eq!(invoke(&main, "run_history", json!({})).unwrap(), json!([]));
        let settings = invoke(&main, "run_settings", json!({})).unwrap();
        assert!(settings["engine"].is_string() && settings["budgetUsd"].is_number());
        let unknown = denied(invoke(&main, "run_start", json!({ "task": "tidy" })));
        assert!(unknown.contains("不认识的任务"), "{unknown}");
        let unknown = denied(invoke(
            &main,
            "run_start",
            json!({ "task": "ingest", "engine": "gemini" }),
        ));
        assert!(unknown.contains("不认识的引擎"), "{unknown}");
        for cmd in ["run_discard", "run_keep", "run_approve", "run_retry_commit", "run_recheck"] {
            let refused = denied(invoke(&main, cmd, json!({ "runId": "no-such-run" })));
            assert!(refused.contains("没有这次运行"), "{cmd}: {refused}");
        }
        assert!(invoke(&main, "run_cancel", json!({})).is_err());

        // A whole run through IPC, the way the main window drives it: start,
        // fetch what the gate has to check, hand in the report.
        let started = invoke(
            &main,
            "run_start",
            json!({ "task": "ingest", "engine": "claude", "model": "sonnet" }),
        )
        .unwrap();
        let run_id = started["run"]["id"].as_str().unwrap().to_string();
        assert_eq!(started["run"]["model"], "sonnet");
        let wait_for = |phase: &str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(40);
            loop {
                let status = invoke(&main, "run_status", json!({})).unwrap();
                if status["phase"] == phase {
                    return status;
                }
                assert!(std::time::Instant::now() < deadline, "{status}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let checking = wait_for("checking");
        let attempt = checking["gateAttempt"].clone();
        assert_eq!(attempt, 1);
        // A write of an existing file is refused while the run is on …
        let busy = denied(invoke(
            &main,
            "kb_write",
            json!({ "path": "self/底色.md", "content": "x", "message": "m" }),
        ));
        assert!(busy.contains("知识库正忙：正在入库"), "{busy}");
        // … and so is any other new file, but an answer goes in beside the
        // ingest as a commit of its own that the run leaves alone.
        let busy = denied(invoke(
            &main,
            "kb_create",
            json!({ "path": "self/判例/P-009-新的.md", "content": "x\n", "message": "m" }),
        ));
        assert!(busy.contains("知识库正忙：正在入库"), "{busy}");
        let answered = invoke(
            &main,
            "kb_create",
            json!({
                "path": "inbox/裁决/Q-001-2026-10-12T14-03-24-mac.md",
                "content": "---\n对象: Q-001\n时间: 2026-10-12T14:03:24+08:00\n来源: mac\n---\n\n先这样。\n",
                "message": "收件箱：回答 Q-001（mac）",
            }),
        )
        .unwrap();
        assert_eq!(answered["committed"], true);
        assert_eq!(commit_count(kb.path()), 4);
        let input = invoke(
            &main,
            "run_gate_input",
            json!({ "runId": run_id, "attempt": attempt }),
        )
        .unwrap();
        assert_eq!(input["task"], "ingest");
        assert_eq!(input["queue"].as_array().unwrap().len(), 1);
        let changed: Vec<&str> = input["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| change["path"].as_str().unwrap())
            .collect();
        assert_eq!(changed, ["raw/inbox/新-收件箱.md", "reports/新-入库报告.md"]);
        assert_eq!(input["changes"][0]["status"], "added");
        assert_eq!(input["changes"][0]["after"], "# 收件箱\n\n一条\n\n第二条\n");
        let queued = input["queue"][0]["path"].as_str().unwrap().to_string();
        invoke(
            &main,
            "run_gate_report",
            json!({
                "runId": run_id,
                "attempt": attempt,
                "report": {
                    "ok": true,
                    "violations": [],
                    "warnings": [],
                    "commitPaths": changed,
                    "deleteQueuePaths": [queued],
                    "restoreQueuePaths": [],
                    "summary": "通过检查",
                    "final": null,
                },
            }),
        )
        .unwrap();
        let done = wait_for("idle");
        assert_eq!(done["run"]["outcome"], "committed", "{done}");
        assert_eq!(commit_count(kb.path()), 5);
        assert_eq!(
            git(kb.path(), &["log", "-1", "--format=%s"]),
            "入库：1 条（Claude）"
        );
        assert!(!kb.path().join(&queued).exists());
        // The answer made during the run is still waiting for 落裁决.
        assert!(kb
            .path()
            .join("inbox/裁决/Q-001-2026-10-12T14-03-24-mac.md")
            .exists());
        assert_eq!(invoke(&main, "run_history", json!({})).unwrap()[0]["id"], run_id.as_str());
        assert!(!invoke(&main, "run_steps", json!({})).unwrap().as_array().unwrap().is_empty());
        // One run with its steps, which were also kept on disk.
        let detail = invoke(&main, "run_record", json!({ "runId": run_id })).unwrap();
        assert_eq!(detail["record"]["id"], run_id.as_str());
        let steps = detail["steps"].as_array().unwrap();
        assert!(steps.iter().any(|step| step["kind"] == "note"), "{detail}");
        assert!(steps.iter().all(|step| step["runId"] == run_id.as_str()));
        let kept = std::fs::read_to_string(scratch.path().join("runs").join(&run_id).join("steps.jsonl"))
            .unwrap();
        assert_eq!(kept.lines().count(), steps.len());
        for bad in ["../runs", "no-such-run"] {
            let refused = denied(invoke(&main, "run_record", json!({ "runId": bad })));
            assert!(refused.contains("没有这次运行的记录"), "{bad}: {refused}");
        }

        // A second run touches the 底色: it waits for the user's approval,
        // and 确认并提交 commits it as approved.
        scratch.write("second", "");
        invoke(
            &main,
            "kb_create",
            json!({
                "path": "inbox/2026-10-12T15-00-00-mac.md",
                "content": "---\n来源: mac\n时间: 2026-10-12T15:00:00+08:00\n---\n\n第二条\n",
                "message": "收件箱：记一笔（mac）",
            }),
        )
        .unwrap();
        let second = invoke(&main, "run_start", json!({ "task": "ingest" })).unwrap();
        let second_id = second["run"]["id"].as_str().unwrap().to_string();
        let checking = wait_for("checking");
        let attempt = checking["gateAttempt"].clone();
        let input = invoke(
            &main,
            "run_gate_input",
            json!({ "runId": second_id, "attempt": attempt }),
        )
        .unwrap();
        let changed: Vec<&str> = input["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| change["path"].as_str().unwrap())
            .collect();
        assert_eq!(changed, ["self/底色.md"]);
        let queued = input["queue"][0]["path"].as_str().unwrap().to_string();
        invoke(
            &main,
            "run_gate_report",
            json!({
                "runId": second_id,
                "attempt": attempt,
                "report": {
                    "ok": false,
                    "violations": [],
                    "warnings": [],
                    "approvals": [{
                        "rule": "diseDraft",
                        "path": "self/底色.md",
                        "entry": "D02",
                        "message": "新加的底色条目 D02 没有标草稿",
                        "before": null,
                        "after": "### D02 没标草稿的新条目\n> 原话",
                    }],
                    "commitPaths": changed,
                    "deleteQueuePaths": [queued],
                    "restoreQueuePaths": [],
                    "summary": "需要你确认：1 处改动",
                    "final": null,
                },
            }),
        )
        .unwrap();
        let waiting = wait_for("needsDecision");
        assert_eq!(waiting["run"]["reason"], "needsApproval", "{waiting}");
        let commits = commit_count(kb.path());
        let refused = denied(invoke(&main, "run_keep", json!({ "runId": second_id })));
        assert!(refused.contains("等你确认"), "{refused}");
        let approved = invoke(&main, "run_approve", json!({ "runId": second_id })).unwrap();
        assert_eq!(approved["phase"], "idle", "{approved}");
        assert_eq!(approved["run"]["outcome"], "committed");
        assert_eq!(approved["run"]["decision"], "approved");
        assert_eq!(commit_count(kb.path()), commits + 1);
        assert_eq!(git(kb.path(), &["log", "-1", "--format=%s"]), "入库：1 条（Claude）");
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(scratch.path().join("runs").join(&second_id).join("run.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["decision"], "approved");
        assert_eq!(saved["approved"], json!(["新加的底色条目 D02 没有标草稿"]));
        let refused = denied(invoke(&main, "run_retry_commit", json!({ "runId": second_id })));
        assert!(refused.contains("没有这次运行"), "{refused}");

        // The settings screen: what it shows, and saving keeps unknown keys.
        let shown = invoke(&main, "settings_get", json!({})).unwrap();
        assert_eq!(shown["version"], "0.1.0");
        assert_eq!(shown["kb"]["source"], "env");
        assert_eq!(shown["kb"]["check"]["ok"], true, "{shown}");
        assert_eq!(shown["kb"]["check"]["hasRules"], true);
        assert_eq!(shown["run"]["budgetUsd"], 3.0);
        assert_eq!(shown["autostart"], Value::Null);
        assert_eq!(shown["configProblem"], Value::Null);
        let saved = invoke(
            &main,
            "settings_save_ai",
            json!({ "settings": { "engine": "codex", "model": " ", "budgetUsd": 2, "runTimeoutMinutes": 5 } }),
        )
        .unwrap();
        assert_eq!(saved["run"]["engine"], "codex");
        assert_eq!(saved["run"]["model"], Value::Null);
        assert_eq!(saved["run"]["runTimeoutMinutes"], 5.0);
        let config: Value = serde_json::from_str(
            &std::fs::read_to_string(app_dir.path().join("config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            config,
            json!({ "kept": {"by": "hand"}, "budgetUsd": 2, "engine": "codex", "runTimeoutMinutes": 5 })
        );
        let refused = denied(invoke(
            &main,
            "settings_save_ai",
            json!({ "settings": { "budgetUsd": -1 } }),
        ));
        assert!(refused.contains("花费上限"), "{refused}");
        let plain = TempDir::new("ipc-not-a-repo");
        let check = invoke(&main, "settings_check_kb", json!({ "path": plain.path() })).unwrap();
        assert_eq!(check["ok"], false);
        assert!(check["problem"].as_str().unwrap().contains("不是 git 仓库"));
        let refused = denied(invoke(&main, "settings_set_kb", json!({ "path": plain.path() })));
        assert!(refused.contains("不是 git 仓库"), "{refused}");
        let moved = invoke(&main, "settings_set_kb", json!({ "path": kb.path() })).unwrap();
        assert_eq!(moved["kb"]["check"]["ok"], true);
        let config: Value = serde_json::from_str(
            &std::fs::read_to_string(app_dir.path().join("config.json")).unwrap(),
        )
        .unwrap();
        assert!(config["kbPath"].as_str().unwrap().ends_with(
            kb.path().file_name().unwrap().to_str().unwrap()
        ));
        assert_eq!(config["kept"], json!({"by": "hand"}));
        // No shortcut, no launch at login in the mock app: said in plain words.
        let refused = denied(invoke(&main, "settings_set_shortcut", json!({ "setting": "Ctrl+Alt+K" })));
        assert!(refused.contains("快捷键"), "{refused}");
        let refused = denied(invoke(&main, "settings_set_shortcut", json!({ "setting": "Cmd+Space" })));
        assert!(refused.contains("Spotlight"), "{refused}");
        let refused = denied(invoke(&main, "settings_set_autostart", json!({ "enabled": true })));
        assert!(refused.contains("开机启动"), "{refused}");
        let refused = denied(invoke(&main, "settings_open_folder", json!({ "which": "desktop" })));
        assert!(refused.contains("不认识"), "{refused}");

        // The iPhone export: registered, allowed for the main window, its
        // arguments deserialize; a bridge that is not set up writes nothing.
        app.manage(crate::bridge::BridgeService::new(
            KbLock::default(),
            ShellEnvState::default(),
            crate::bridge::BridgeTiming::default(),
            crate::bridge::Hooks {
                kb_root: Box::new(|| Err("none".into())),
                setting: Box::new(|| crate::bridge::folder::resolve(None, None, None)),
                app_dir: None,
                running: Box::new(|| None),
                imported: Box::new(|_| {}),
                ready: Box::new(|| {}),
                sign: Box::new(|_, _| Err("none".into())),
                download: Box::new(|_| {}),
            },
        ));
        let exported = invoke(
            &main,
            "bridge_export",
            json!({ "files": [{ "name": "今日.html", "content": "<p>今日</p>" }] }),
        )
        .unwrap();
        assert_eq!(exported["exported"], false);
        assert!(denied(invoke(&main, "bridge_export", json!({}))).contains("files"));
        assert_eq!(
            invoke(&main, "settings_get", json!({})).unwrap()["bridge"]["enabled"],
            false
        );

        // The capture window may only create files directly in inbox/.
        let capture = WebviewWindowBuilder::new(&app, "capture", Default::default())
            .build()
            .expect("capture window");
        let saved = invoke(
            &capture,
            "kb_create",
            json!({
                "path": "inbox/2026-10-12T14-03-23-mac.md",
                "content": "---\n来源: mac\n---\n\n第二条\n",
                "message": "收件箱：记一笔（mac）",
            }),
        )
        .unwrap();
        assert_eq!(saved["committed"], true);
        assert_eq!(commit_count(kb.path()), 8);
        for path in ["self/新的.md", "inbox/裁决/2026-10-12-Q-001.md", "index.md"] {
            let refused = denied(invoke(
                &capture,
                "kb_create",
                json!({ "path": path, "content": "x\n", "message": "m" }),
            ));
            assert!(
                refused.contains("只能在 inbox 里新建文件"),
                "{path}: {refused}"
            );
        }
        assert!(!kb.path().join("self/新的.md").exists());
        for (cmd, args) in [
            ("kb_read", json!({ "path": "self/底色.md" })),
            ("kb_list", json!({})),
            ("kb_root", json!({})),
            ("kb_status", json!({})),
            (
                "kb_write",
                json!({ "path": "self/底色.md", "content": "x", "message": "m" }),
            ),
            ("capture_show", json!({})),
            ("tray_set_counts", json!({ "inbox": 0, "rulings": 0 })),
            ("kb_read_head", json!({ "path": "self/底色.md" })),
            ("run_start", json!({ "task": "ingest" })),
            ("run_status", json!({})),
            ("run_steps", json!({})),
            ("run_cancel", json!({})),
            ("run_history", json!({})),
            ("run_settings", json!({})),
            ("run_gate_input", json!({ "runId": "x", "attempt": 1 })),
            ("run_discard", json!({ "runId": "x" })),
            ("run_keep", json!({ "runId": "x" })),
            ("run_approve", json!({ "runId": "x" })),
            ("run_retry_commit", json!({ "runId": "x" })),
            ("run_recheck", json!({ "runId": "x" })),
            ("run_record", json!({ "runId": "x" })),
            ("settings_get", json!({})),
            ("settings_save_ai", json!({ "settings": {} })),
            ("settings_check_kb", json!({ "path": "/" })),
            ("settings_pick_kb", json!({})),
            ("settings_set_kb", json!({ "path": "/" })),
            ("settings_set_shortcut", json!({ "setting": "Ctrl+Alt+K" })),
            ("settings_set_autostart", json!({ "enabled": true })),
            ("settings_open_folder", json!({ "which": "logs" })),
            ("settings_set_bridge", json!({ "enabled": false })),
            ("bridge_export", json!({ "files": [] })),
        ] {
            let refused = denied(invoke(&capture, cmd, args));
            assert!(refused.contains("not allowed"), "{cmd}: {refused}");
        }
        invoke(&capture, "capture_hide", json!({})).unwrap();
        assert_eq!(commit_count(kb.path()), 8);

        // Any other window gets nothing.
        let other = WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .expect("other window");
        let refused = denied(invoke(&other, "kb_list", json!({})));
        assert!(refused.contains("not allowed"), "{refused}");
    }
}

#[cfg(test)]
mod config_tests {
    //! Guards the security-relevant parts of the app configuration.

    use serde_json::Value;

    fn config() -> Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json")
    }

    fn capability() -> Value {
        serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("capabilities/default.json")
    }

    fn directive(csp: &Value, name: &str) -> Vec<String> {
        csp[name]
            .as_str()
            .unwrap_or_else(|| panic!("CSP directive {name} is missing"))
            .split_whitespace()
            .map(String::from)
            .collect()
    }

    #[test]
    fn the_production_csp_is_strict() {
        let config = config();
        let csp = &config["app"]["security"]["csp"];
        assert!(csp.is_object(), "csp must be set");
        assert_eq!(directive(csp, "default-src"), ["'self'"]);
        // Only the app's own scripts: no inline or remote scripts, no eval.
        assert_eq!(directive(csp, "script-src"), ["'self'"]);
        // React sets inline style attributes.
        assert_eq!(directive(csp, "style-src"), ["'self'", "'unsafe-inline'"]);
        assert_eq!(directive(csp, "img-src"), ["'self'", "data:", "https:"]);
        // Tauri 2 IPC.
        assert_eq!(
            directive(csp, "connect-src"),
            ["ipc:", "http://ipc.localhost"]
        );
        assert_eq!(directive(csp, "object-src"), ["'none'"]);
        // Nothing anywhere allows any host, plain http, eval or inline scripts.
        for (name, value) in csp.as_object().unwrap() {
            for token in value.as_str().unwrap().split_whitespace() {
                assert!(
                    ![
                        "*",
                        "http:",
                        "https:*",
                        "'unsafe-eval'",
                        "'wasm-unsafe-eval'"
                    ]
                    .contains(&token),
                    "{name}: {token}"
                );
                if token == "'unsafe-inline'" {
                    assert_eq!(name, "style-src", "'unsafe-inline' only for styles");
                }
            }
        }
    }

    #[test]
    fn the_bundle_is_an_app_only() {
        assert_eq!(config()["bundle"]["targets"], serde_json::json!(["app"]));
    }

    fn window(label: &str) -> Value {
        config()["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["label"] == label)
            .unwrap_or_else(|| panic!("window {label} is missing"))
            .clone()
    }

    #[test]
    fn the_capture_window_is_a_hidden_floating_slip() {
        let capture = window("capture");
        assert_eq!(capture["url"], "capture.html");
        assert_eq!(capture["width"], 560);
        assert_eq!(capture["height"], 300);
        for (key, expected) in [
            ("decorations", false),
            ("transparent", true),
            ("alwaysOnTop", true),
            ("skipTaskbar", true),
            ("visible", false),
            ("resizable", false),
        ] {
            assert_eq!(capture[key], expected, "{key}");
        }
        // Both windows start hidden: setup shows the main one unless the
        // command line asked for work in the background.
        assert_eq!(window("main")["visible"], false);
        assert_eq!(config()["app"]["macOSPrivateApi"], true);
    }

    #[test]
    fn the_capture_window_may_only_create_inbox_files() {
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/capture.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["capture"]));
        let permissions = capability["permissions"].as_array().unwrap();
        let names: Vec<&str> = permissions
            .iter()
            .map(|p| {
                p.as_str()
                    .unwrap_or_else(|| p["identifier"].as_str().unwrap())
            })
            .collect();
        assert_eq!(
            names,
            [
                "allow-kb-create",
                "allow-capture-hide",
                "core:event:allow-listen",
                "core:event:allow-unlisten",
                "log:default"
            ]
        );
        assert_eq!(
            permissions[0]["allow"],
            serde_json::json!([{ "folder": "inbox" }])
        );
    }

    #[test]
    fn links_may_only_open_http_and_https_urls() {
        let capability = capability();
        let permissions = capability["permissions"].as_array().unwrap();
        let opener: Vec<&Value> = permissions
            .iter()
            .filter(|p| {
                p.as_str()
                    .unwrap_or_else(|| p["identifier"].as_str().unwrap_or(""))
                    .starts_with("opener:")
            })
            .collect();
        assert_eq!(opener.len(), 1, "{opener:?}");
        assert_eq!(opener[0]["identifier"], "opener:allow-open-url");
        let mut urls: Vec<&str> = opener[0]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["url"].as_str().unwrap())
            .collect();
        urls.sort();
        assert_eq!(urls, ["http://*", "https://*"]);
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
    }
}
