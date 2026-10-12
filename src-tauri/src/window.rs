//! Window lifecycle on macOS: closing the main window hides it (the app keeps
//! running in the menu bar), clicking the Dock icon or launching the app
//! again shows it, and ⌘Q (or 退出 in the tray menu) quits. The capture
//! window is never closed, only hidden. Focusing the main window syncs the
//! knowledge base (at most once a minute) and looks for files from the iPhone.

use tauri::{AppHandle, Manager, RunEvent, Runtime, Window, WindowEvent};
use tauri_plugin_global_shortcut::GlobalShortcut;

use crate::bridge::BridgeService;
use crate::capture_window::CAPTURE;
use crate::run::service::RunService;
use crate::sync_service::SyncService;

/// Label of the main window (the first window in `tauri.conf.json`).
pub const MAIN: &str = "main";

/// Whether closing the window with `label` only hides it.
pub fn close_hides(label: &str, macos: bool) -> bool {
    label == CAPTURE || (macos && label == MAIN)
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

/// Closing hides instead of quitting; focusing the main window syncs.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            if close_hides(window.label(), cfg!(target_os = "macos")) {
                api.prevent_close();
                let _ = window.hide();
                if window.label() == MAIN {
                    log::info!("main window closed: hidden, the app keeps running");
                }
            }
        }
        WindowEvent::Focused(true) if window.label() == MAIN => {
            if let Some(service) = window.app_handle().try_state::<SyncService>() {
                service.on_focus();
            }
            // Whatever the phone wrote in the meantime comes in now.
            if let Some(bridge) = window.app_handle().try_state::<BridgeService>() {
                bridge.on_focus();
            }
        }
        _ => {}
    }
}

/// App-level events: clicking the Dock icon brings the window back; on exit
/// a running engine is stopped, and the sync thread and the global shortcut
/// are released.
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
        RunEvent::Exit => {
            // An engine must not keep working on the knowledge base alone.
            if let Some(runs) = app.try_state::<RunService>() {
                runs.shutdown();
            }
            if let Some(service) = app.try_state::<SyncService>() {
                service.stop();
            }
            if let Some(bridge) = app.try_state::<BridgeService>() {
                bridge.stop();
            }
            if let Some(shortcuts) = app.try_state::<GlobalShortcut<R>>() {
                if let Err(error) = shortcuts.unregister_all() {
                    log::warn!("global shortcut: could not unregister on exit: {error}");
                }
            }
            log::info!("app exiting");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_window_hides_on_macos_and_the_capture_window_always() {
        assert!(close_hides("main", true));
        assert!(!close_hides("main", false));
        assert!(close_hides("capture", true));
        assert!(close_hides("capture", false));
        assert!(!close_hides("other", true));
    }
}
