//! Tauri desktop entry point for 底色 (Dise).
//!
//! The shell opens the main window and exposes the knowledge-base commands
//! (`kb_root`, `kb_list`, `kb_read`, `kb_write`). Around them:
//!
//! - `shell_env`: the user's login-shell environment, captured once at
//!   startup and used for every child process (git);
//! - `logging`: stdout plus a rotated file in `~/Library/Logs/com.gegej1.dise/`;
//! - `window`: closing the window hides it, the Dock icon shows it, ⌘Q quits;
//! - single instance: a second launch focuses the running app;
//! - opener: http(s) links open in the default browser.

mod commands;
mod kb;
mod logging;
pub mod shell_env;
mod window;

use shell_env::ShellEnvState;

/// Builds the desktop app from `tauri.conf.json` and runs it until exit.
pub fn run() {
    let builder = tauri::Builder::default()
        // Registered first, so a second launch hands over to the running app
        // and exits before any other plugin starts.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            log::info!(
                "second launch handed over to this instance (args={}); showing the main window",
                args.len()
            );
            window::show_main(app);
        }))
        .plugin(logging::plugin())
        // Links are opened explicitly by the frontend (http and https only, see
        // capabilities/default.json), so the plugin's click handler stays off.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .on_window_event(window::on_window_event)
        .setup(|app| {
            log::info!(
                "Dise {} starting (pid {})",
                app.package_info().version,
                std::process::id()
            );
            // Capture the login-shell environment in the background now, so
            // the first git command does not have to wait for it.
            tauri::Manager::state::<ShellEnvState>(app).warm_up();
            Ok(())
        });

    register(builder, ShellEnvState::default())
        .build(tauri::generate_context!())
        .expect("failed to build the Dise desktop app")
        .run(window::on_run_event);
}

/// Adds the knowledge-base commands and their state to a builder.
///
/// Shared by [`run`] and the IPC tests, which use Tauri's mock runtime (and
/// must not start the single-instance plugin or the login shell).
fn register<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    shell_env: ShellEnvState,
) -> tauri::Builder<R> {
    builder
        .manage(commands::WriteLock::default())
        .manage(shell_env)
        .invoke_handler(tauri::generate_handler![
            commands::kb_root,
            commands::kb_list,
            commands::kb_read,
            commands::kb_write,
        ])
}

#[cfg(test)]
mod ipc_tests {
    //! Drives the real command handlers through Tauri's IPC layer with the
    //! mock runtime and the app's real config and capabilities. This proves
    //! the commands are registered, their arguments deserialize, and the ACL
    //! allows them for the main window only.

    use std::time::Duration;

    use serde_json::{json, Value};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};
    use crate::shell_env::{ChildEnv, Origin, ShellEnv, ShellEnvState};

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

    #[test]
    fn commands_work_through_ipc_for_the_main_window_only() {
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

        // `test = true` skips embedding Info.plist a second time, but keeps the
        // real config, capabilities and ACL.
        let app = super::register(mock_builder(), shell_env)
            .build(tauri::generate_context!(test = true))
            .expect("build mock app");
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

        // Unsafe paths are rejected with a readable message, before any I/O.
        let rejected = invoke(&main, "kb_read", json!({ "path": "../secret.md" })).unwrap_err();
        assert!(rejected.as_str().unwrap().contains(".."), "{rejected}");
        let rejected = invoke(
            &main,
            "kb_write",
            json!({ "path": ".git/config", "content": "x", "message": "坏" }),
        )
        .unwrap_err();
        assert!(rejected.as_str().unwrap().contains(".git"), "{rejected}");
        assert_eq!(commit_count(kb.path()), 2);

        // Missing arguments fail to deserialize instead of running.
        let missing = invoke(&main, "kb_read", json!({})).unwrap_err();
        assert!(
            missing
                .as_str()
                .unwrap()
                .contains("missing required key path"),
            "{missing}"
        );

        // The capability only grants the commands to the main window.
        let other = WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .expect("other window");
        let denied = invoke(&other, "kb_list", json!({})).unwrap_err();
        assert!(denied.as_str().unwrap().contains("not allowed"), "{denied}");
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
