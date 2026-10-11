//! Tauri desktop entry point for 底色 (Dise).
//!
//! The shell opens the main window and exposes the knowledge-base commands
//! (`kb_root`, `kb_list`, `kb_read`, `kb_write`). The menu-bar icon and
//! plugins are added in later milestones.

mod commands;
mod kb;

/// Builds the desktop app from `tauri.conf.json` and runs it until exit.
pub fn run() {
    register(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("failed to run the Dise desktop app");
}

/// Adds the knowledge-base commands and their state to a builder.
///
/// Shared by [`run`] and the IPC tests, which use Tauri's mock runtime.
fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .manage(commands::WriteLock::default())
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

    use serde_json::{json, Value};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};

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

        // `test = true` skips embedding Info.plist a second time, but keeps the
        // real config, capabilities and ACL.
        let app = super::register(mock_builder())
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
