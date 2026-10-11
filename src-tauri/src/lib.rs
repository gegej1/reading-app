//! Tauri desktop entry point for 底色 (Dise).
//!
//! The shell opens the main window and exposes the knowledge-base commands
//! (`kb_root`, `kb_list`, `kb_read`, `kb_write`, `kb_create`, `kb_status`,
//! `kb_sync`, see `commands`) and a few for the desktop shell (see `desktop`).
//! Around them:
//!
//! - `shell_env`: the user's login-shell environment, captured once at
//!   startup and used for every child process (git);
//! - `kb::lock`: one knowledge-base operation at a time (writes, captures,
//!   syncs, and later long-running jobs), with a clear "busy" answer;
//! - `sync_service`: when the knowledge base is synced with its remote;
//! - `capture_window`, `shortcut`, `tray`: quick capture (记一笔) from a
//!   floating slip, a global shortcut and the menu-bar icon;
//! - `cli`: `--capture "<text>"` and `--sync`, also handed to a running
//!   instance by the single-instance plugin;
//! - `events`: `kb-changed` and `sync-status` for the windows;
//! - `process_group`: stopping a child process together with what it started;
//! - `logging`: stdout plus a rotated file in `~/Library/Logs/com.gegej1.dise/`;
//! - `window`: closing the window hides it, the Dock icon shows it, ⌘Q quits;
//! - opener: http(s) links open in the default browser.

mod capture_window;
mod cli;
mod commands;
mod desktop;
mod events;
mod kb;
mod logging;
mod process_group;
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

    let setting = commands::config_file(app)
        .and_then(|file| root::read_config_string(&file, root::SHORTCUT_KEY));
    let shortcut = shortcut::register(app, setting, capture_window::toggle);
    if let Err(error) = tray::create(app, &shortcut, on_tray_action) {
        log::error!("tray icon could not be created: {error}");
    }
    app.manage(shortcut);

    service.start();
    perform(app, request);
    if request.has_action() {
        log::info!("started for the command line: staying in the menu bar");
    } else {
        window::show_main(app);
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
        "second launch handed over to this instance (capture={}, sync={})",
        request.capture.is_some(),
        request.sync
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
    match &request.capture {
        Some(Ok(text)) => {
            let app = app.clone();
            let text = text.clone();
            let at = LocalTime::now();
            let spawned = std::thread::Builder::new()
                .name("capture".into())
                .spawn(move || capture_from_command_line(&app, &text, &at));
            if let Err(error) = spawned {
                log::error!("command line capture: could not start ({error})");
            }
        }
        Some(Err(error)) => log::warn!("command line capture: {error}"),
        None => {}
    }
    if request.sync {
        if let Some(service) = app.try_state::<SyncService>() {
            service.schedule(Duration::ZERO, "command line");
        }
    }
}

/// Saves a capture from `--capture` exactly like the capture window does:
/// a new file in `inbox/`, committed on its own. Logs the outcome, never the text.
fn capture_from_command_line<R: Runtime>(app: &AppHandle<R>, text: &str, at: &LocalTime) {
    let started = Instant::now();
    let result = (|| {
        let built = capture::build_capture(text, capture::MAC, at)?;
        let root = commands::kb_folder(app)?;
        let env = &app.state::<ShellEnvState>().inner().get().env;
        // Also while a long-running job is going on: a capture is its own commit.
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
        .invoke_handler(tauri::generate_handler![
            commands::kb_root,
            commands::kb_list,
            commands::kb_read,
            commands::kb_write,
            commands::kb_create,
            commands::kb_status,
            commands::kb_sync,
            desktop::capture_show,
            desktop::capture_hide,
            desktop::tray_set_counts,
            desktop::desktop_info,
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
    use crate::kb::sync::Timeouts;
    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};
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
        git(kb.path(), &["add", "--", "self/底色.md"]);
        git(kb.path(), &["commit", "-q", "-m", "init"]);

        // The only test that touches the process environment.
        std::env::set_var(crate::kb::root::ENV_VAR, kb.path());

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
            lock,
            shell_env,
            Timeouts::default(),
            SyncTiming::default(),
            move || Ok(folder.clone()),
            |_| {},
        ));
        let main = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main window");

        let info = invoke(&main, "kb_root", json!({})).unwrap();
        assert_eq!(info["source"], "env");
        assert_eq!(info["exists"], true);
        assert_eq!(info["path"], kb.path().to_string_lossy().as_ref());

        let list = invoke(&main, "kb_list", json!({})).unwrap();
        assert_eq!(list[0]["path"], "self/底色.md");
        assert!(list[0]["mtime"].is_u64());

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
        assert_eq!(commit_count(kb.path()), 4);
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
        ] {
            let refused = denied(invoke(&capture, cmd, args));
            assert!(refused.contains("not allowed"), "{cmd}: {refused}");
        }
        invoke(&capture, "capture_hide", json!({})).unwrap();
        assert_eq!(commit_count(kb.path()), 4);

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
