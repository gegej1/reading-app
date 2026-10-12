//! The iPhone bridge in the app (see `crate::bridge`): building its service
//! and the command the main window exports through. Main window only.
//!
//! | command         | arguments | returns / does                                       |
//! |-----------------|-----------|------------------------------------------------------|
//! | `bridge_export` | `files`   | writes the exported pages into the bridge's `阅读/`   |
//!
//! `files` is `[{ name, content }]` with names like `今日.html` or
//! `待裁决/Q-001.txt` (see `crate::bridge::export` for the accepted names).
//! When the bridge is off or its folder is not ready nothing is written and
//! the answer says `exported: false`. The backend sends `bridge-ready` when
//! the folder is (again) ready, so the main window exports at once.

use std::time::Instant;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::bridge::export::ExportFile;
use crate::bridge::{self, folder, BridgeService, BridgeTiming, ExportOutcome, Hooks};
use crate::commands;
use crate::events::{self, KbChanged};
use crate::kb::lock::KbLock;
use crate::run::service::RunService;
use crate::shell_env::ShellEnvState;

/// The bridge settings as they are now.
pub fn setting<R: Runtime>(app: &AppHandle<R>) -> folder::BridgeSetting {
    folder::resolve(
        std::env::var_os(folder::ENV_VAR),
        commands::config_file(app).as_deref(),
        app.path().home_dir().ok().as_deref(),
    )
}

/// The bridge service of the app.
pub fn service<R: Runtime>(app: &AppHandle<R>) -> BridgeService {
    let for_root = app.clone();
    let for_setting = app.clone();
    let for_running = app.clone();
    let for_imported = app.clone();
    let for_ready = app.clone();
    BridgeService::new(
        app.state::<KbLock>().inner().clone(),
        app.state::<ShellEnvState>().inner().clone(),
        BridgeTiming::default(),
        Hooks {
            kb_root: Box::new(move || commands::kb_folder(&for_root)),
            setting: Box::new(move || setting(&for_setting)),
            app_dir: commands::app_dir(app),
            running: Box::new(move || {
                for_running
                    .try_state::<RunService>()
                    .and_then(|runs| runs.active_task())
            }),
            imported: Box::new(move |paths| {
                for path in paths {
                    events::files_written(
                        &for_imported,
                        KbChanged::new("bridge", Some(path), None),
                    );
                }
            }),
            ready: Box::new(move || events::bridge_ready(&for_ready)),
            sign: Box::new(bridge::shortcuts::sign_with_system),
            download: Box::new(bridge::download_with_brctl),
        },
    )
}

#[tauri::command]
pub async fn bridge_export(
    service: State<'_, BridgeService>,
    files: Vec<ExportFile>,
) -> Result<ExportOutcome, String> {
    let started = Instant::now();
    let count = files.len();
    let service = service.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || service.export(&files))
        .await
        .map_err(|error| format!("后台任务失败：{error}"))
        .and_then(|result| result);
    let millis = started.elapsed().as_millis();
    match &result {
        // Unchanged exports are the usual case: only at debug level.
        Ok(done) => log::debug!(
            "bridge_export ok files={count} exported={} written={} ({millis} ms)",
            done.exported,
            done.written.written
        ),
        Err(error) => log::warn!("bridge_export failed ({millis} ms): {error}"),
    }
    result
}
