//! Where the knowledge base lives.
//!
//! Resolution order (first match wins):
//! 1. the `DISE_KB_PATH` environment variable;
//! 2. the `kbPath` key of `config.json` in the app config dir
//!    (`~/Library/Application Support/com.gegej1.dise/` on macOS);
//! 3. the default `~/Desktop/GHJProject/阅读`.
//!
//! Everything here is pure: the caller passes in the environment value, the
//! config file location and the home directory, so it is unit-testable.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Environment variable that overrides every other setting.
pub const ENV_VAR: &str = "DISE_KB_PATH";
/// File name of the app config inside the app config dir.
pub const CONFIG_FILE_NAME: &str = "config.json";
/// Key in `config.json` that holds the knowledge-base path.
pub const CONFIG_KEY: &str = "kbPath";
/// Default location, relative to the home directory.
pub const DEFAULT_RELATIVE: &str = "Desktop/GHJProject/阅读";

/// Which setting produced the knowledge-base root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RootSource {
    Env,
    Config,
    Default,
}

impl RootSource {
    /// Short Chinese description used in user-facing messages.
    pub fn describe(self) -> &'static str {
        match self {
            RootSource::Env => "来自环境变量 DISE_KB_PATH",
            RootSource::Config => "来自 config.json 的 kbPath",
            RootSource::Default => "默认位置",
        }
    }
}

/// A resolved (but not yet canonicalized or checked) knowledge-base root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbRoot {
    pub path: PathBuf,
    pub source: RootSource,
}

/// Resolves the knowledge-base root from the three possible sources.
///
/// - `env`: value of `DISE_KB_PATH`; ignored when unset or empty.
/// - `config_file`: path of `config.json`; a missing, unreadable or malformed
///   file (or a missing/empty `kbPath`) falls through to the default.
/// - `home`: the user's home directory, used for `~` expansion and the default.
///
/// An explicitly configured path that is relative is an error rather than a
/// silent fallback, so a typo never opens a different knowledge base.
pub fn resolve_kb_root(
    env: Option<OsString>,
    config_file: Option<&Path>,
    home: Option<&Path>,
) -> Result<KbRoot, String> {
    if let Some(value) = env.filter(|v| !v.is_empty()) {
        let path = expand_home(Path::new(&value), home)
            .map_err(|e| format!("环境变量 {ENV_VAR} 无效：{e}"))?;
        return Ok(KbRoot {
            path,
            source: RootSource::Env,
        });
    }

    if let Some(value) = config_file.and_then(read_config_kb_path) {
        let path = expand_home(Path::new(&value), home)
            .map_err(|e| format!("config.json 里的 {CONFIG_KEY} 无效：{e}"))?;
        return Ok(KbRoot {
            path,
            source: RootSource::Config,
        });
    }

    let home = home.ok_or_else(|| "找不到用户主目录，无法确定知识库的默认位置".to_string())?;
    Ok(KbRoot {
        path: home.join(DEFAULT_RELATIVE),
        source: RootSource::Default,
    })
}

/// Reads the non-empty string value of `kbPath`, or `None` for anything else.
fn read_config_kb_path(config_file: &Path) -> Option<String> {
    let text = fs::read_to_string(config_file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let value = json.get(CONFIG_KEY)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Expands a leading `~` or `~/` and requires the result to be absolute.
fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let mut components = path.components();
    let expanded = match components.next() {
        Some(first) if first.as_os_str() == "~" => {
            let home = home.ok_or_else(|| "找不到用户主目录，无法展开 ~".to_string())?;
            let rest = components.as_path();
            if rest.as_os_str().is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            }
        }
        _ => path.to_path_buf(),
    };
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Err(format!("必须是绝对路径：{}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    fn home() -> PathBuf {
        PathBuf::from("/Users/someone")
    }

    #[test]
    fn env_var_wins_over_config_and_default() {
        let dir = TempDir::new("root-env");
        let config = dir.write("config.json", r#"{"kbPath": "/from/config"}"#);
        let root = resolve_kb_root(
            Some(OsString::from("/from/env")),
            Some(&config),
            Some(&home()),
        )
        .unwrap();
        assert_eq!(root.path, PathBuf::from("/from/env"));
        assert_eq!(root.source, RootSource::Env);
    }

    #[test]
    fn empty_env_var_is_ignored() {
        let root = resolve_kb_root(Some(OsString::new()), None, Some(&home())).unwrap();
        assert_eq!(root.source, RootSource::Default);
    }

    #[test]
    fn env_var_expands_tilde() {
        let root =
            resolve_kb_root(Some(OsString::from("~/notes/kb")), None, Some(&home())).unwrap();
        assert_eq!(root.path, PathBuf::from("/Users/someone/notes/kb"));

        let bare = resolve_kb_root(Some(OsString::from("~")), None, Some(&home())).unwrap();
        assert_eq!(bare.path, home());
    }

    #[test]
    fn relative_env_var_is_rejected() {
        let err =
            resolve_kb_root(Some(OsString::from("notes/kb")), None, Some(&home())).unwrap_err();
        assert!(err.contains(ENV_VAR), "{err}");
        assert!(err.contains("绝对路径"), "{err}");

        // `~user` style is not expanded, so it stays relative and is rejected.
        assert!(resolve_kb_root(Some(OsString::from("~bob/kb")), None, Some(&home())).is_err());
    }

    #[test]
    fn tilde_without_home_is_an_error() {
        assert!(resolve_kb_root(Some(OsString::from("~/kb")), None, None).is_err());
    }

    #[test]
    fn config_file_is_used_when_env_is_unset() {
        let dir = TempDir::new("root-config");
        let config = dir.write(
            "config.json",
            r#"{"kbPath": "~/Documents/知识库", "other": 1}"#,
        );
        let root = resolve_kb_root(None, Some(&config), Some(&home())).unwrap();
        assert_eq!(root.path, PathBuf::from("/Users/someone/Documents/知识库"));
        assert_eq!(root.source, RootSource::Config);
    }

    #[test]
    fn relative_config_path_is_rejected() {
        let dir = TempDir::new("root-config-rel");
        let config = dir.write("config.json", r#"{"kbPath": "Documents/kb"}"#);
        let err = resolve_kb_root(None, Some(&config), Some(&home())).unwrap_err();
        assert!(err.contains(CONFIG_KEY), "{err}");
    }

    #[test]
    fn malformed_or_missing_config_falls_through_to_default() {
        let dir = TempDir::new("root-config-bad");
        let cases = [
            dir.write("bad-json.json", "{ not json"),
            dir.write("no-key.json", r#"{"other": "/x"}"#),
            dir.write("not-string.json", r#"{"kbPath": 42}"#),
            dir.write("empty.json", r#"{"kbPath": "   "}"#),
            dir.write("array.json", r#"["/x"]"#),
            dir.path().join("does-not-exist.json"),
        ];
        for config in cases {
            let root = resolve_kb_root(None, Some(&config), Some(&home())).unwrap();
            assert_eq!(root.source, RootSource::Default, "{}", config.display());
            assert_eq!(
                root.path,
                PathBuf::from("/Users/someone/Desktop/GHJProject/阅读")
            );
        }
    }

    #[test]
    fn default_needs_a_home_directory() {
        let root = resolve_kb_root(None, None, Some(&home())).unwrap();
        assert_eq!(
            root.path,
            PathBuf::from("/Users/someone/Desktop/GHJProject/阅读")
        );
        assert_eq!(root.source, RootSource::Default);
        assert!(resolve_kb_root(None, None, None).is_err());
    }

    #[test]
    fn source_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&RootSource::Env).unwrap(), "\"env\"");
        assert_eq!(
            serde_json::to_string(&RootSource::Config).unwrap(),
            "\"config\""
        );
        assert_eq!(
            serde_json::to_string(&RootSource::Default).unwrap(),
            "\"default\""
        );
    }
}
