//! The menu-bar icon (a monochrome template image, drawn by
//! `scripts/build-tray-icon.mjs`) and its menu:
//!
//! ```text
//! 记一笔                  ⌃⌥N
//! 打开底色
//! ───────────────────────────
//! 收件箱 3 条 · 待裁决 2 项 · 已同步      (not clickable)
//! 立即同步
//! ───────────────────────────
//! 退出
//! ```
//!
//! The counts come from the main window after every load of the knowledge
//! base; the sync state from the sync service. The Dock icon stays.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::shortcut::ShortcutStatus;

const TRAY_ID: &str = "dise";
const ICON: Image<'static> = tauri::include_image!("icons/tray-icon.png");

/// The menu actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Capture,
    Open,
    Sync,
    Quit,
}

impl Action {
    const ALL: [Action; 4] = [Action::Capture, Action::Open, Action::Sync, Action::Quit];

    fn id(self) -> &'static str {
        match self {
            Action::Capture => "capture",
            Action::Open => "open",
            Action::Sync => "sync",
            Action::Quit => "quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.id() == id)
    }
}

/// How many captures and pending rulings the main window last counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub inbox: u32,
    pub rulings: u32,
}

/// The text of the status line.
pub fn status_line(counts: Option<Counts>, sync: &str) -> String {
    match counts {
        Some(counts) => format!(
            "收件箱 {} 条 · 待裁决 {} 项 · {sync}",
            counts.inbox, counts.rulings
        ),
        None => format!("正在读取知识库 · {sync}"),
    }
}

/// The text of the capture item, and the accelerator shown next to it.
pub fn capture_item(shortcut: &ShortcutStatus) -> (String, Option<String>) {
    if shortcut.registered {
        ("记一笔".into(), Some(shortcut.setting.clone()))
    } else if shortcut.label.is_empty() {
        ("记一笔（快捷键设置有误）".into(), None)
    } else {
        (format!("记一笔（快捷键 {} 不可用）", shortcut.label), None)
    }
}

/// The parts of the menu that change.
pub struct TrayState<R: Runtime> {
    capture: MenuItem<R>,
    status: MenuItem<R>,
    line: Mutex<(Option<Counts>, String)>,
}

impl<R: Runtime> TrayState<R> {
    fn update(&self, change: impl FnOnce(&mut (Option<Counts>, String))) {
        let mut line = self.line.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut line);
        let _ = self.status.set_text(status_line(line.0, &line.1));
    }
}

/// Creates the icon and its menu. `on_action` handles the clicks.
pub fn create<R, F>(
    app: &AppHandle<R>,
    shortcut: &ShortcutStatus,
    on_action: F,
) -> tauri::Result<()>
where
    R: Runtime,
    F: Fn(&AppHandle<R>, Action) + Send + Sync + 'static,
{
    let (capture_text, accelerator) = capture_item(shortcut);
    // The accelerator only labels the item; fall back to the plain text when
    // the menu does not understand the spelling.
    let capture = MenuItem::with_id(
        app,
        Action::Capture.id(),
        &capture_text,
        true,
        accelerator.as_deref(),
    )
    .or_else(|_| {
        MenuItem::with_id(
            app,
            Action::Capture.id(),
            format!("{capture_text}（{}）", shortcut.label),
            true,
            None::<&str>,
        )
    })?;
    let open = MenuItem::with_id(app, Action::Open.id(), "打开底色", true, None::<&str>)?;
    let sync_label = "还没同步".to_string();
    let status = MenuItem::with_id(
        app,
        "status",
        status_line(None, &sync_label),
        false,
        None::<&str>,
    )?;
    let sync = MenuItem::with_id(app, Action::Sync.id(), "立即同步", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, Action::Quit.id(), "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &capture,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &status,
            &sync,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(ICON)
        .icon_as_template(true)
        .tooltip("底色")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app: &AppHandle<R>, event: MenuEvent| {
            if let Some(action) = Action::from_id(event.id().as_ref()) {
                on_action(app, action);
            }
        })
        .build(app)?;

    app.manage(TrayState {
        capture,
        status,
        line: Mutex::new((None, sync_label)),
    });
    log::info!("tray icon created");
    Ok(())
}

/// The capture shortcut changed (in the settings): the menu shows the new one.
pub fn set_capture_shortcut<R: Runtime>(app: &AppHandle<R>, shortcut: &ShortcutStatus) {
    let (text, accelerator) = capture_item(shortcut);
    let label = shortcut.label.clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.try_state::<TrayState<R>>() else {
            return;
        };
        let _ = tray.capture.set_text(&text);
        if tray.capture.set_accelerator(accelerator.as_deref()).is_err() {
            let _ = tray.capture.set_accelerator(None::<&str>);
            let _ = tray.capture.set_text(format!("{text}（{label}）"));
        }
    });
}

/// New counts from the main window.
pub fn set_counts<R: Runtime>(app: &AppHandle<R>, counts: Counts) {
    on_main_thread(app, move |line| line.0 = Some(counts));
}

/// A new sync state (see `crate::sync_service::tray_label`).
pub fn set_sync<R: Runtime>(app: &AppHandle<R>, label: String) {
    on_main_thread(app, move |line| line.1 = label);
}

/// Changes the status line on the main thread, in the order the changes
/// were made. Changing a menu item from another thread would wait for the
/// main thread while holding the line, which can deadlock; posting never waits.
fn on_main_thread<R, F>(app: &AppHandle<R>, change: F)
where
    R: Runtime,
    F: FnOnce(&mut (Option<Counts>, String)) + Send + 'static,
{
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.try_state::<TrayState<R>>() {
            tray.update(change);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_shows_counts_and_the_sync_state() {
        assert_eq!(
            status_line(
                Some(Counts {
                    inbox: 3,
                    rulings: 2
                }),
                "已同步"
            ),
            "收件箱 3 条 · 待裁决 2 项 · 已同步"
        );
        assert_eq!(status_line(None, "还没同步"), "正在读取知识库 · 还没同步");
    }

    #[test]
    fn the_capture_item_says_when_the_shortcut_is_missing() {
        let mut shortcut = ShortcutStatus {
            setting: "Ctrl+Alt+N".into(),
            label: "⌃⌥N".into(),
            registered: true,
            error: None,
        };
        assert_eq!(
            capture_item(&shortcut),
            ("记一笔".to_string(), Some("Ctrl+Alt+N".to_string()))
        );
        shortcut.registered = false;
        assert_eq!(
            capture_item(&shortcut),
            ("记一笔（快捷键 ⌃⌥N 不可用）".to_string(), None)
        );
        shortcut.label.clear();
        assert_eq!(capture_item(&shortcut).0, "记一笔（快捷键设置有误）");
    }

    #[test]
    fn menu_ids_map_to_actions() {
        for action in Action::ALL {
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
        assert_eq!(Action::from_id("status"), None);
    }

    #[test]
    fn the_icon_is_a_small_template_image() {
        let icon = ICON;
        assert_eq!((icon.width(), icon.height()), (36, 36));
        let pixels = icon.rgba();
        // Template images are black; only the alpha channel carries the shape.
        assert!(pixels
            .chunks(4)
            .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
        let opaque = pixels.chunks(4).filter(|p| p[3] > 128).count();
        assert!(opaque > 150 && opaque < 900, "{opaque} opaque pixels");
    }
}
