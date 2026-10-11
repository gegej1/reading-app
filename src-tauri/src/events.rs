//! Events the backend sends to the windows. Features subscribe to them with
//! `listen` from `@tauri-apps/api/event`:
//!
//! | event           | payload                       | sent when                                   |
//! |-----------------|-------------------------------|---------------------------------------------|
//! | `kb-changed`    | `{ reason, path, window }`    | a file was written or created, or a sync pulled changes |
//! | `sync-status`   | `SyncStatus` (`sync_service`) | a sync started or ended, or a write moved the repository on |
//! | `capture-shown` | –                             | the capture window was shown (to that window only) |
//!
//! `reason` is `write`, `create`, `capture` (from the command line) or
//! `sync`; `window` is the label of the window that made the change, so it
//! can skip the reload it already does itself.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::sync_service::{SyncService, SyncStatus};

pub const KB_CHANGED: &str = "kb-changed";
pub const SYNC_STATUS: &str = "sync-status";

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
