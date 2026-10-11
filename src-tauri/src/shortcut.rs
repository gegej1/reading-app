//! The system-wide shortcut that shows and hides the capture window.
//!
//! The default is ⌃⌥N (`Ctrl+Alt+N`): ⌘⇧N already means "new folder" in
//! Finder and "new incognito window" in Chrome. `captureShortcut` in
//! `config.json` (next to `kbPath`) picks another one, written like
//! `Ctrl+Alt+K` or `Cmd+Shift+Space`. When the shortcut cannot be registered
//! (taken by another app, or not understood) the app keeps running without
//! it, logs why, and says so in the tray menu and on the 今日 screen.

use std::str::FromStr;

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

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
