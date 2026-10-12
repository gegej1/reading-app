//! Events the backend sends to the windows. Features subscribe to them with
//! `listen` from `@tauri-apps/api/event`:
//!
//! | event           | payload                       | sent when                                   |
//! |-----------------|-------------------------------|---------------------------------------------|
//! | `kb-changed`    | `{ reason, path, window }`    | a file was written or created, or a sync pulled changes |
//! | `sync-status`   | `SyncStatus` (`sync_service`) | a sync started or ended, or a write moved the repository on |
//! | `capture-shown` | –                             | the capture window was shown (to that window only) |
//! | `run-status`    | `RunStatus` (`run::service`)  | an AI run started, moved to its next phase or ended |
//! | `run-step`      | `StepEvent` (`run::service`)  | one step of the run going on (a tool call, a note, …) |
//! | `run-show`      | –                             | the user clicked the notification about a run (show its result) |
//!
//! `reason` is `write`, `create`, `capture` (from the command line), `sync`,
//! `run` (an AI run committed or discarded its changes) or `settings` (the
//! knowledge-base folder changed); `window` is the label of the window that
//! made the change, so it can skip the reload it already does itself.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::run::service::{RunStatus, StepEvent};
use crate::sync_service::{SyncService, SyncStatus};

pub const KB_CHANGED: &str = "kb-changed";
pub const SYNC_STATUS: &str = "sync-status";
pub const RUN_STATUS: &str = "run-status";
pub const RUN_STEP: &str = "run-step";
pub const RUN_SHOW: &str = "run-show";

/// Payload of `kb-changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbChanged {
    pub reason: &'static str,
    /// The file written or created; none for a sync.
    pub path: Option<String>,
    /// The window that made the change, if a window did.
    pub window: Option<String>,
}

impl KbChanged {
    pub fn new(reason: &'static str, path: Option<String>, window: Option<&str>) -> Self {
        KbChanged {
            reason,
            path,
            window: window.map(String::from),
        }
    }
}

/// Tells every window that the knowledge base changed.
pub fn kb_changed<R: Runtime>(app: &AppHandle<R>, change: &KbChanged) {
    if let Err(error) = app.emit(KB_CHANGED, change) {
        log::warn!("could not send {KB_CHANGED}: {error}");
    }
}

/// Tells every window how syncing stands.
pub fn sync_status<R: Runtime>(app: &AppHandle<R>, status: &SyncStatus) {
    if let Err(error) = app.emit(SYNC_STATUS, status) {
        log::warn!("could not send {SYNC_STATUS}: {error}");
    }
}

/// Tells every window where the AI run stands.
pub fn run_status<R: Runtime>(app: &AppHandle<R>, status: &RunStatus) {
    if let Err(error) = app.emit(RUN_STATUS, status) {
        log::warn!("could not send {RUN_STATUS}: {error}");
    }
}

/// Tells every window about one step of the AI run.
pub fn run_step<R: Runtime>(app: &AppHandle<R>, step: &StepEvent) {
    if let Err(error) = app.emit(RUN_STEP, step) {
        log::warn!("could not send {RUN_STEP}: {error}");
    }
}

/// Asks the main window to show the result of the last run.
pub fn run_show<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.emit(RUN_SHOW, ()) {
        log::warn!("could not send {RUN_SHOW}: {error}");
    }
}

/// After the app committed a file: tell the windows, look at the repository
/// again (one more commit to push) and sync a moment later.
pub fn files_written<R: Runtime>(app: &AppHandle<R>, change: KbChanged) {
    kb_changed(app, &change);
    if let Some(service) = app.try_state::<SyncService>() {
        service.after_write();
        let service = service.inner().clone();
        // Looking at the repository runs git: not on the calling thread.
        tauri::async_runtime::spawn_blocking(move || service.refresh());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_change_payload_is_camel_case_json() {
        let change = KbChanged::new(
            "create",
            Some("inbox/2026-10-12T14-03-22-mac.md".into()),
            Some("capture"),
        );
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            serde_json::json!({
                "reason": "create",
                "path": "inbox/2026-10-12T14-03-22-mac.md",
                "window": "capture"
            })
        );
        assert_eq!(
            serde_json::to_value(KbChanged::new("sync", None, None)).unwrap(),
            serde_json::json!({ "reason": "sync", "path": null, "window": null })
        );
    }
}
