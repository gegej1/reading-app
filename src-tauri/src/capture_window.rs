//! The capture window (记一笔): a small paper slip that floats above every
//! app, opened by the global shortcut, the tray menu or the 今日 screen.
//!
//! It is created hidden at startup (see `tauri.conf.json`) and only ever
//! shown and hidden. Each time it opens it is placed in the upper third of
//! the screen the pointer is on. On macOS, the app that was in front before
//! the slip opened gets the focus back when the slip closes, so a quick note
//! never pulls you out of what you were doing.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewWindow};

/// Label of the capture window in `tauri.conf.json`.
pub const CAPTURE: &str = "capture";
/// Sent to the capture window every time it is shown.
pub const SHOWN_EVENT: &str = "capture-shown";
/// Keep at least this many pixels between the slip and the top of the screen.
const TOP_MARGIN: i32 = 24;

/// The app that was in front before the slip opened (macOS process id).
#[derive(Default)]
pub struct PreviousApp(Mutex<Option<i32>>);

impl PreviousApp {
    fn set(&self, pid: Option<i32>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = pid;
    }

    fn take(&self) -> Option<i32> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Where the slip goes in a screen's usable area (`x`, `y`, `width`,
/// `height`, physical pixels): centered horizontally, with its middle at one
/// third of the height, but never closer than [`TOP_MARGIN`] to the top.
pub fn slip_position(area: (i32, i32, u32, u32), window: (u32, u32)) -> (i32, i32) {
    let (x, y, width, height) = area;
    let (window_width, window_height) = window;
    let to_i32 = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
    let left = x + (to_i32(width) - to_i32(window_width)) / 2;
    let top = y + to_i32(height) / 3 - to_i32(window_height) / 2;
    (left, top.max(y + TOP_MARGIN))
}

/// Shows the slip and focuses it (from any thread).
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || show_now(&handle));
}

/// Hides the slip; the draft stays in the page (from any thread).
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || hide_now(&handle));
}

/// The shortcut: hides the slip when it is in front, else shows it.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(CAPTURE) else {
            log::warn!("capture window not found");
            return;
        };
        let in_front = window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false);
        if in_front {
            hide_now(&handle);
        } else {
            show_now(&handle);
        }
    });
}

fn show_now<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(CAPTURE) else {
        log::warn!("capture window not found");
        return;
    };
    if let Some(pid) = focus::frontmost_other_app() {
        app.state::<PreviousApp>().set(Some(pid));
    }
    if !window.is_visible().unwrap_or(false) {
        place(app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(CAPTURE, SHOWN_EVENT, ());
    log::info!("capture window shown");
}

fn hide_now<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(CAPTURE) else {
        return;
    };
    // Only hand the focus back when the slip had it: if it closes because
    // another app was clicked, that app keeps the focus.
    let had_focus = window.is_focused().unwrap_or(false);
    let _ = window.hide();
    let previous = app.state::<PreviousApp>().take();
    if had_focus {
        if let Some(pid) = previous {
            if !focus::activate(pid) {
                log::info!("capture window hidden; the previous app could not be activated");
            }
        }
    }
}

/// Puts the slip in the upper third of the screen the pointer is on.
fn place<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|point| app.monitor_from_point(point.x, point.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        let _ = window.center();
        return;
    };
    let area = monitor.work_area();
    let (x, y) = slip_position(
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        (size.width, size.height),
    );
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Which app is in front, and bringing one back to the front (macOS).
#[cfg(target_os = "macos")]
mod focus {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

    /// The process id of the app in front, unless that is this app.
    pub fn frontmost_other_app() -> Option<i32> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        let own = i32::try_from(std::process::id()).ok()?;
        (pid > 0 && pid != own).then_some(pid)
    }

    /// Asks macOS to bring the app with this process id to the front.
    pub fn activate(pid: i32) -> bool {
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .is_some_and(|app| app.activateWithOptions(NSApplicationActivationOptions::empty()))
    }
}

#[cfg(not(target_os = "macos"))]
mod focus {
    pub fn frontmost_other_app() -> Option<i32> {
        None
    }

    pub fn activate(_pid: i32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slip_sits_in_the_upper_third() {
        // A 1512 × 945 point screen at 2×, menu bar 37 px, no Dock below.
        let area = (0, 74, 3024, 1890);
        let (x, y) = slip_position(area, (1120, 600));
        assert_eq!(x, (3024 - 1120) / 2);
        assert_eq!(y, 74 + 1890 / 3 - 300);
        // Its middle is at one third of the usable height.
        assert_eq!(y + 300 - 74, 1890 / 3);
    }

    #[test]
    fn the_slip_never_touches_the_menu_bar() {
        let (_, y) = slip_position((0, 25, 800, 400), (560, 300));
        assert_eq!(y, 25 + TOP_MARGIN);
    }

    #[test]
    fn works_on_screens_left_of_and_above_the_main_one() {
        let (x, y) = slip_position((-1920, -1080, 1920, 1080), (560, 300));
        assert_eq!(x, -1920 + (1920 - 560) / 2);
        assert_eq!(y, -1080 + 360 - 150);
    }

    #[test]
    fn remembers_the_previous_app_once() {
        let previous = PreviousApp::default();
        previous.set(Some(42));
        assert_eq!(previous.take(), Some(42));
        assert_eq!(previous.take(), None);
    }
}
