//! Commands for the desktop shell around the knowledge base: the capture
//! window, the counts in the tray menu, and what the 今日 screen needs to
//! know about the shortcut.
//!
//! | command           | window  | arguments          | does                               |
//! |-------------------|---------|--------------------|------------------------------------|
//! | `capture_show`    | main    | –                  | shows the capture window           |
//! | `capture_hide`    | capture | –                  | hides it (the draft stays)         |
//! | `tray_set_counts` | main    | `inbox`, `rulings` | updates the tray menu's status line |
//! | `desktop_info`    | main    | –                  | `{ captureShortcut }`              |

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::capture_window;
use crate::shortcut::ShortcutStatus;
use crate::tray::{self, Counts};

#[tauri::command]
pub fn capture_show<R: Runtime>(app: AppHandle<R>) {
    capture_window::show(&app);
}

#[tauri::command]
pub fn capture_hide<R: Runtime>(app: AppHandle<R>) {
    capture_window::hide(&app);
}

#[tauri::command]
pub fn tray_set_counts<R: Runtime>(app: AppHandle<R>, inbox: u32, rulings: u32) {
    tray::set_counts(&app, Counts { inbox, rulings });
}

/// What the main window shows about the desktop app.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopInfo {
    /// The capture shortcut and whether it works; `None` until it was registered.
    pub capture_shortcut: Option<ShortcutStatus>,
}

#[tauri::command]
pub fn desktop_info<R: Runtime>(app: AppHandle<R>) -> DesktopInfo {
    DesktopInfo {
        capture_shortcut: app
            .try_state::<ShortcutStatus>()
            .map(|status| status.inner().clone()),
    }
}
