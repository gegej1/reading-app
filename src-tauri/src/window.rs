//! Main window lifecycle on macOS: closing the window hides it (the app keeps
//! running; a menu-bar icon comes later), clicking the Dock icon or launching
//! the app again shows it, and ⌘Q (the default app menu's Quit) quits.

use tauri::{AppHandle, Manager, RunEvent, Runtime, Window, WindowEvent};

/// Label of the main window (the first window in `tauri.conf.json`).
pub const MAIN: &str = "main";

/// Whether closing the window with `label` only hides it.
pub fn close_hides(label: &str, macos: bool) -> bool {
    macos && label == MAIN
}

/// Shows, un-minimizes and focuses the main window.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    match app.get_webview_window(MAIN) {
        Some(window) => {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        None => log::warn!("main window not found"),
    }
}

/// Closing the main window hides it instead of quitting (macOS only).
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if close_hides(window.label(), cfg!(target_os = "macos")) {
            api.prevent_close();
            let _ = window.hide();
            log::info!("main window closed: hidden, the app keeps running");
        }
    }
}

/// App-level events: clicking the Dock icon brings the window back.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            // Also when it is merely minimized or behind other apps.
            show_main(app);
            if !has_visible_windows {
                log::info!("Dock icon clicked: main window shown again");
            }
        }
        RunEvent::Exit => log::info!("app exiting"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_window_on_macos_hides_on_close() {
        assert!(close_hides("main", true));
        assert!(!close_hides("main", false));
        assert!(!close_hides("capture", true));
    }
}
