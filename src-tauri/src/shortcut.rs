//! The system-wide shortcut that shows and hides the capture window.
//!
//! The default is ⌃⌥N (`Ctrl+Alt+N`): ⌘⇧N already means "new folder" in
//! Finder and "new incognito window" in Chrome. `captureShortcut` in
//! `config.json` (next to `kbPath`) picks another one, written like
//! `Ctrl+Alt+K` or `Cmd+Shift+Space`. When the shortcut cannot be registered
//! (taken by another app, or not understood) the app keeps running without
//! it, logs why, and says so in the tray menu and on the 今日 screen.

use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Used when `config.json` has no `captureShortcut`.
pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+N";

/// What became of the shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    /// As written in `config.json`, or the default.
    pub setting: String,
    /// How people write it on a Mac (`⌃⌥N`); empty when the setting is not understood.
    pub label: String,
    pub registered: bool,
    /// Why it is not registered (shown to the user).
    pub error: Option<String>,
}

/// Reads a shortcut setting. It needs ⌃, ⌥ or ⌘: anything else would take
/// away a key (or a capital letter) from typing in every app.
pub fn parse(setting: &str) -> Result<Shortcut, String> {
    let shortcut = Shortcut::from_str(setting.trim())
        .map_err(|error| format!("看不懂快捷键设置「{setting}」（{error}）"))?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(format!(
            "快捷键「{setting}」至少要带 ⌃、⌥ 或 ⌘ 中的一个，不然会挡住平常打字"
        ));
    }
    Ok(shortcut)
}

/// Keys that type something (or move the cursor) when pressed with a single
/// modifier: such a shortcut would take a key away from typing or from the
/// apps' own commands.
fn types_something(code: Code) -> bool {
    let name = code.to_string();
    let function_key = name
        .strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()));
    let media = name.starts_with("Audio") || name.starts_with("Media");
    !(function_key || media)
}

/// Why `shortcut` would get in the way of macOS or of typing, if it would.
pub fn conflict(shortcut: &Shortcut) -> Option<String> {
    let mods = shortcut.mods;
    let key = shortcut.key;
    let without_shift = mods & !Modifiers::SHIFT;
    let control = Modifiers::CONTROL;
    let option = Modifiers::ALT;
    let command = Modifiers::SUPER;
    let shift = mods.contains(Modifiers::SHIFT);
    let system = match (without_shift, key) {
        (m, Code::Space) if m == command && !shift => Some("Spotlight 搜索"),
        (m, Code::Space) if m == control && !shift => Some("切换输入法"),
        (m, Code::Space) if m == command | option => Some("Finder 搜索"),
        (m, Code::Space) if m == control | command => Some("表情与符号"),
        (m, Code::Tab) if m == command => Some("切换 App"),
        (m, Code::Backquote) if m == command => Some("切换窗口"),
        (m, Code::KeyQ) if m == control | command => Some("锁定屏幕"),
        (m, Code::KeyQ) if m == command && shift => Some("退出登录"),
        (m, Code::Escape) if m == command | option => Some("强制退出"),
        (m, Code::Digit3 | Code::Digit4 | Code::Digit5) if m == command && shift => Some("截屏"),
        (m, Code::KeyD) if m == command | option => Some("显示或隐藏程序坞"),
        _ => None,
    };
    if let Some(what) = system {
        return Some(format!("{} 是 macOS 的「{what}」，换一个吧", label(shortcut)));
    }
    if types_something(key) {
        if without_shift == option {
            return Some(format!(
                "{} 是用来打特殊字符的，再加一个 ⌃ 或 ⌘，比如 ⌃⌥N",
                label(shortcut)
            ));
        }
        if (without_shift == command || without_shift == control) && !shift {
            return Some(format!(
                "{} 只带一个修饰键，各个 App 自己在用这样的快捷键（或者用它编辑文字）。再加一个修饰键，比如 ⌃⌥N",
                label(shortcut)
            ));
        }
    }
    None
}

/// Reads a shortcut the settings screen asks for: like [`parse`], and it
/// must not get in the way of macOS or of typing (see [`conflict`]).
pub fn check_setting(setting: &str) -> Result<Shortcut, String> {
    let shortcut = parse(setting)?;
    match conflict(&shortcut) {
        Some(problem) => Err(problem),
        None => Ok(shortcut),
    }
}

/// The shortcut as Mac menus show it: modifiers in the order ⌃ ⌥ ⇧ ⌘, then the key.
pub fn label(shortcut: &Shortcut) -> String {
    let mut text = String::new();
    for (modifier, symbol) in [
        (Modifiers::CONTROL, "⌃"),
        (Modifiers::ALT, "⌥"),
        (Modifiers::SHIFT, "⇧"),
        (Modifiers::SUPER, "⌘"),
    ] {
        if shortcut.mods.contains(modifier) {
            text.push_str(symbol);
        }
    }
    text.push_str(&key_label(&shortcut.key.to_string()));
    text
}

fn key_label(code: &str) -> String {
    if let Some(letter) = code.strip_prefix("Key").filter(|rest| rest.len() == 1) {
        return letter.to_string();
    }
    if let Some(digit) = code.strip_prefix("Digit").filter(|rest| rest.len() == 1) {
        return digit.to_string();
    }
    let symbol = match code {
        "Space" => "Space",
        "Enter" => "↩",
        "Tab" => "⇥",
        "Escape" => "⎋",
        "Backspace" => "⌫",
        "Delete" => "⌦",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "Backquote" => "`",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        other => other,
    };
    symbol.to_string()
}

/// What became of the shortcut, as the app knows it now (managed state, so
/// the settings screen can change the shortcut while the app runs).
#[derive(Debug)]
pub struct ShortcutHolder(Mutex<ShortcutStatus>);

impl ShortcutHolder {
    pub fn new(status: ShortcutStatus) -> Self {
        ShortcutHolder(Mutex::new(status))
    }

    pub fn get(&self) -> ShortcutStatus {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set(&self, status: ShortcutStatus) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = status;
    }
}

/// Switches to the shortcut `setting`, live: the new one is registered first,
/// and only when that worked is the old one released, so a failure leaves
/// the old shortcut working. The caller saves the setting.
pub fn replace<R, F>(app: &AppHandle<R>, setting: &str, on_press: F) -> Result<ShortcutStatus, String>
where
    R: Runtime,
    F: Fn(&AppHandle<R>) + Send + Sync + 'static,
{
    let shortcut = check_setting(setting)?;
    let label = label(&shortcut);
    let holder = app
        .try_state::<ShortcutHolder>()
        .ok_or_else(|| "快捷键还没有设置好，稍后再试".to_string())?;
    let current = holder.get();
    let old = parse(&current.setting).ok().filter(|_| current.registered);
    let setting = setting.trim().to_string();
    if old.as_ref() == Some(&shortcut) {
        let status = ShortcutStatus {
            setting,
            label,
            registered: true,
            error: None,
        };
        holder.set(status.clone());
        return Ok(status);
    }
    let shortcuts = app.global_shortcut();
    shortcuts
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                on_press(app);
            }
        })
        .map_err(|error| {
            log::warn!("global shortcut {label} ({setting}) could not be registered: {error}");
            format!("{label} 没能设置，可能已经被别的 App 占用了。原来的快捷键照常可用，换一个试试")
        })?;
    if let Some(old) = old {
        if let Err(error) = shortcuts.unregister(old) {
            log::warn!("global shortcut: could not release the old one: {error}");
        }
    }
    log::info!("global shortcut {label} ({setting}) registered from the settings");
    let status = ShortcutStatus {
        setting,
        label,
        registered: true,
        error: None,
    };
    holder.set(status.clone());
    Ok(status)
}

/// Registers the shortcut from `setting` (or the default); `on_press` runs on
/// every press. Never fails: problems end up in the returned status.
pub fn register<R, F>(app: &AppHandle<R>, setting: Option<String>, on_press: F) -> ShortcutStatus
where
    R: Runtime,
    F: Fn(&AppHandle<R>) + Send + Sync + 'static,
{
    let setting = setting.unwrap_or_else(|| DEFAULT_SHORTCUT.to_string());
    let shortcut = match parse(&setting) {
        Ok(shortcut) => shortcut,
        Err(error) => {
            log::warn!("global shortcut: the setting {setting:?} is not usable: {error}");
            return ShortcutStatus {
                setting,
                label: String::new(),
                registered: false,
                error: Some(error),
            };
        }
    };
    let label = label(&shortcut);
    let registered = app
        .global_shortcut()
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                on_press(app);
            }
        });
    match registered {
        Ok(()) => {
            log::info!(
                "global shortcut {label} ({setting}) registered: toggles the capture window"
            );
            ShortcutStatus {
                setting,
                label,
                registered: true,
                error: None,
            }
        }
        Err(error) => {
            log::warn!("global shortcut {label} ({setting}) could not be registered: {error}");
            ShortcutStatus {
                error: Some(format!(
                    "快捷键 {label} 没能注册，可能已经被别的 App 占用了。可以在 config.json 里用 captureShortcut 换一个。"
                )),
                setting,
                label,
                registered: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Code;

    #[test]
    fn the_default_is_control_option_n() {
        let shortcut = parse(DEFAULT_SHORTCUT).unwrap();
        assert_eq!(shortcut.mods, Modifiers::CONTROL | Modifiers::ALT);
        assert_eq!(shortcut.key, Code::KeyN);
        assert_eq!(label(&shortcut), "⌃⌥N");
    }

    #[test]
    fn reads_other_spellings_and_labels_them_the_mac_way() {
        assert_eq!(label(&parse("cmd+shift+space").unwrap()), "⇧⌘Space");
        assert_eq!(label(&parse("Option+Control+K").unwrap()), "⌃⌥K");
        assert_eq!(label(&parse(" Alt+Digit1 ").unwrap()), "⌥1");
        assert_eq!(label(&parse("Ctrl+F5").unwrap()), "⌃F5");
        assert_eq!(label(&parse("Ctrl+Alt+Slash").unwrap()), "⌃⌥/");
    }

    #[test]
    fn refuses_shortcuts_that_would_get_in_the_way_of_typing() {
        for setting in ["N", "Shift+N", "", "Ctrl+Alt+", "Ctrl+Alt+N+M", "Hyper+N"] {
            assert!(parse(setting).is_err(), "{setting:?}");
        }
        assert!(parse("Shift+N").unwrap_err().contains("⌃、⌥ 或 ⌘"));
    }

    #[test]
    fn the_settings_screen_refuses_shortcuts_that_collide_with_macos_or_typing() {
        for (setting, expected) in [
            ("Cmd+Space", "Spotlight"),
            ("Ctrl+Space", "输入法"),
            ("Cmd+Tab", "切换 App"),
            ("Ctrl+Cmd+Q", "锁定屏幕"),
            ("Cmd+Shift+4", "截屏"),
            ("Cmd+Alt+Escape", "强制退出"),
            ("Alt+K", "特殊字符"),
            ("Alt+Shift+K", "特殊字符"),
            ("Cmd+K", "只带一个修饰键"),
            ("Ctrl+A", "只带一个修饰键"),
            ("Cmd+ArrowLeft", "只带一个修饰键"),
            ("Shift+K", "⌃、⌥ 或 ⌘"),
            ("Ctrl+Alt+", "看不懂"),
        ] {
            let error = check_setting(setting).unwrap_err();
            assert!(error.contains(expected), "{setting}: {error}");
        }
        for setting in [
            "Ctrl+Alt+N",
            "Cmd+Shift+Space",
            "Ctrl+Alt+Cmd+K",
            "Cmd+Shift+K",
            "Ctrl+Shift+K",
            "Ctrl+F5",
            "Alt+F2",
            "Ctrl+Alt+Digit1",
        ] {
            assert!(check_setting(setting).is_ok(), "{setting}");
        }
        assert!(check_setting("Cmd+Space").unwrap_err().starts_with("⌘Space"));
    }

    #[test]
    fn the_holder_hands_out_the_latest_status() {
        let first = ShortcutStatus {
            setting: DEFAULT_SHORTCUT.into(),
            label: "⌃⌥N".into(),
            registered: true,
            error: None,
        };
        let holder = ShortcutHolder::new(first.clone());
        assert_eq!(holder.get(), first);
        let second = ShortcutStatus {
            setting: "Ctrl+Alt+K".into(),
            label: "⌃⌥K".into(),
            ..first
        };
        holder.set(second.clone());
        assert_eq!(holder.get(), second);
    }

    #[test]
    fn the_status_serializes_for_the_frontend() {
        let status = ShortcutStatus {
            setting: DEFAULT_SHORTCUT.into(),
            label: "⌃⌥N".into(),
            registered: false,
            error: Some("占用".into()),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "setting": "Ctrl+Alt+N",
                "label": "⌃⌥N",
                "registered": false,
                "error": "占用"
            })
        );
    }
}
