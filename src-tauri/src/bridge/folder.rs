//! Where the iPhone bridge folder is, whether the bridge is on, and what the
//! folder holds.
//!
//! The folder is found in this order (first match wins):
//! 1. the `DISE_BRIDGE_DIR` environment variable (for trying the app on a
//!    temporary folder);
//! 2. the `bridgeDir` key of `config.json`;
//! 3. `底色/` in the Shortcuts app's iCloud container,
//!    `~/Library/Mobile Documents/iCloud~is~workflow~my~workflows/Documents/`.
//!    iOS Shortcuts can save and read files there without asking for a
//!    folder, and the Files app shows it as iCloud Drive › 快捷指令 › 底色.
//!
//! `bridgeEnabled` in `config.json` turns the bridge off (it is on unless it
//! says `false`).
//!
//! Layout:
//!
//! ```text
//! 底色/
//!   收件/记一笔/   phone → Mac: captures (one .txt per capture)
//!   收件/回答/     phone → Mac: answers to pending rulings (Q-001-….txt)
//!   阅读/          Mac → phone: the exported 底色 layer (HTML and text)
//!   快捷指令/      the shortcut files and the setup guide
//! ```

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::kb::root::expand_home;

/// Environment variable naming the bridge folder (tests, trying things out).
pub const ENV_VAR: &str = "DISE_BRIDGE_DIR";
/// Key in `config.json` naming the bridge folder.
pub const DIR_KEY: &str = "bridgeDir";
/// Key in `config.json` that turns the bridge off when `false`.
pub const ENABLED_KEY: &str = "bridgeEnabled";
/// The Shortcuts app's iCloud container, relative to the home folder.
pub const SHORTCUTS_CONTAINER: &str =
    "Library/Mobile Documents/iCloud~is~workflow~my~workflows/Documents";
/// The bridge folder's name inside that container.
pub const FOLDER_NAME: &str = "底色";

/// Captures from the phone.
pub const CAPTURES: &str = "收件/记一笔";
/// Answers to pending rulings from the phone.
pub const ANSWERS: &str = "收件/回答";
/// What the phone reads.
pub const READING: &str = "阅读";
/// The shortcut files and the guide.
pub const SHORTCUTS: &str = "快捷指令";

/// Which setting named the bridge folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DirSource {
    Env,
    Config,
    Default,
}

/// The bridge settings as read now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSetting {
    pub enabled: bool,
    /// The folder, or why it cannot be worked out.
    pub dir: Result<PathBuf, String>,
    pub source: DirSource,
}

/// Reads the bridge settings. Pure: the caller passes the environment value,
/// the location of `config.json` and the home folder.
pub fn resolve(
    env: Option<OsString>,
    config_file: Option<&Path>,
    home: Option<&Path>,
) -> BridgeSetting {
    let config = config_file.and_then(|file| {
        let text = fs::read_to_string(file).ok()?;
        serde_json::from_str::<serde_json::Value>(&text).ok()
    });
    let enabled = config
        .as_ref()
        .and_then(|json| json.get(ENABLED_KEY))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    if let Some(value) = env.filter(|value| !value.is_empty()) {
        return BridgeSetting {
            enabled,
            dir: expand_home(Path::new(&value), home)
                .map_err(|error| format!("环境变量 {ENV_VAR} 无效：{error}")),
            source: DirSource::Env,
        };
    }
    let configured = config
        .as_ref()
        .and_then(|json| json.get(DIR_KEY))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(value) = configured {
        return BridgeSetting {
            enabled,
            dir: expand_home(Path::new(value), home)
                .map_err(|error| format!("config.json 里的 {DIR_KEY} 无效：{error}")),
            source: DirSource::Config,
        };
    }
    BridgeSetting {
        enabled,
        dir: home
            .map(|home| home.join(SHORTCUTS_CONTAINER).join(FOLDER_NAME))
            .ok_or_else(|| "找不到用户主目录，没法确定 iCloud 里的文件夹".to_string()),
        source: DirSource::Default,
    }
}

/// The bridge folder and its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn captures(&self) -> PathBuf {
        self.root.join(CAPTURES)
    }
    pub fn answers(&self) -> PathBuf {
        self.root.join(ANSWERS)
    }
    pub fn reading(&self) -> PathBuf {
        self.root.join(READING)
    }
    pub fn shortcuts(&self) -> PathBuf {
        self.root.join(SHORTCUTS)
    }
    /// The folder the phone writes into (watched for changes).
    pub fn inbox(&self) -> PathBuf {
        self.root.join("收件")
    }
}

/// Creates the bridge folder and its parts. At the default place the
/// Shortcuts container has to exist already: the app never creates an
/// iCloud container itself (it appears once iCloud Drive syncs Shortcuts).
pub fn prepare(dir: &Path, source: DirSource) -> Result<Layout, String> {
    if !dir.is_absolute() {
        return Err(format!(
            "iPhone 同步文件夹要是完整的路径：{}",
            dir.display()
        ));
    }
    if source == DirSource::Default {
        let container = dir.parent().unwrap_or(dir);
        if !container.is_dir() {
            return Err(
                "这台 Mac 上还没有快捷指令的 iCloud 文件夹。请在「系统设置 › Apple 账户 › iCloud › iCloud 云盘」里打开 iCloud 云盘，并让「快捷指令」使用 iCloud，然后重新打开底色"
                    .to_string(),
            );
        }
    }
    let layout = Layout {
        root: dir.to_path_buf(),
    };
    for part in [
        layout.captures(),
        layout.answers(),
        layout.reading(),
        layout.shortcuts(),
    ] {
        fs::create_dir_all(&part)
            .map_err(|error| format!("没能建立 iPhone 同步文件夹 {}（{error}）", part.display()))?;
    }
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    #[test]
    fn the_environment_wins_then_the_config_then_icloud() {
        let dir = TempDir::new("bridge-folder");
        let home = dir.path().join("home");
        let config = dir.write(
            "config.json",
            r#"{ "bridgeDir": "~/桥", "bridgeEnabled": false }"#,
        );

        let from_env = resolve(Some("/tmp/桥".into()), Some(&config), Some(&home));
        assert_eq!(from_env.dir, Ok(PathBuf::from("/tmp/桥")));
        assert_eq!(from_env.source, DirSource::Env);
        assert!(!from_env.enabled, "the switch still comes from the config");

        let from_config = resolve(None, Some(&config), Some(&home));
        assert_eq!(from_config.dir, Ok(home.join("桥")));
        assert_eq!(from_config.source, DirSource::Config);

        let default = resolve(Some("".into()), None, Some(&home));
        assert!(default.enabled, "on unless switched off");
        assert_eq!(default.source, DirSource::Default);
        assert_eq!(
            default.dir,
            Ok(home
                .join("Library/Mobile Documents/iCloud~is~workflow~my~workflows/Documents")
                .join("底色"))
        );

        let relative = resolve(Some("桥".into()), None, Some(&home));
        assert!(relative.dir.unwrap_err().contains(ENV_VAR));
        let broken = dir.write("broken.json", "{ not json");
        let fallback = resolve(None, Some(&broken), Some(&home));
        assert!(fallback.enabled);
        assert_eq!(fallback.source, DirSource::Default);
    }

    #[test]
    fn prepares_the_four_parts() {
        let dir = TempDir::new("bridge-prepare");
        let root = dir.path().join("底色");
        let layout = prepare(&root, DirSource::Env).unwrap();
        for part in ["收件/记一笔", "收件/回答", "阅读", "快捷指令"] {
            assert!(root.join(part).is_dir(), "{part}");
        }
        assert_eq!(layout.inbox(), root.join("收件"));
        // Preparing again changes nothing.
        prepare(&root, DirSource::Env).unwrap();
    }

    #[test]
    fn never_creates_the_icloud_container() {
        let dir = TempDir::new("bridge-no-container");
        let missing = dir
            .path()
            .join("Library/Mobile Documents/iCloud~is~workflow~my~workflows/Documents/底色");
        let error = prepare(&missing, DirSource::Default).unwrap_err();
        assert!(error.contains("iCloud"), "{error}");
        assert!(!dir.path().join("Library").exists());
        assert!(prepare(Path::new("relative"), DirSource::Env).is_err());
    }
}
