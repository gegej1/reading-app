//! `config.json` in the app's folder, as the settings screen changes it.
//!
//! The parts that use a setting read the file again on every use (the
//! knowledge-base folder per command, the run settings per run), so a saved
//! change takes effect without a restart; only the capture shortcut has to
//! be registered again (see `crate::settings`).
//!
//! Saving keeps every key it does not know about, writes atomically (a
//! temporary file, then a rename), and refuses to replace a file it cannot
//! read as a JSON object, so nothing written by hand is ever lost.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Map, Number, Value};

use crate::kb::files;
use crate::run::EngineKind;

/// The keys the settings screen may change.
pub const ENGINE: &str = "engine";
pub const MODEL: &str = "model";
pub const BUDGET: &str = "budgetUsd";
pub const TIMEOUT: &str = "runTimeoutMinutes";
pub const KB_PATH: &str = "kbPath";
pub const SHORTCUT: &str = "captureShortcut";
pub const BRIDGE_ENABLED: &str = crate::bridge::folder::ENABLED_KEY;

/// More than this is taken for a typo (the same limits the runs use).
pub const MAX_BUDGET_USD: f64 = 1000.0;
pub const MAX_TIMEOUT_MINUTES: f64 = 24.0 * 60.0;
const MAX_MODEL_CHARS: usize = 120;

/// The settings of one save, already checked. `None` removes a key (the
/// default applies again); a missing field leaves the key as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub engine: Option<EngineKind>,
    /// `Some(None)`: no model, the engine's own default.
    pub model: Option<Option<String>>,
    pub budget_usd: Option<f64>,
    pub timeout_minutes: Option<f64>,
    pub kb_path: Option<String>,
    pub shortcut: Option<String>,
    /// The iPhone bridge on (`true`, the default: the key is removed) or off.
    pub bridge_enabled: Option<bool>,
}

/// Reads the AI settings of a save from the settings screen's JSON. Every
/// value is checked; the first one that makes no sense is reported in plain words.
pub fn ai_changes(input: &Value) -> Result<Changes, String> {
    let mut changes = Changes::default();
    let object = input
        .as_object()
        .ok_or_else(|| "设置的格式不对".to_string())?;
    for key in object.keys() {
        if ![ENGINE, MODEL, BUDGET, TIMEOUT].contains(&key.as_str()) {
            return Err(format!("不认识的设置：{key}"));
        }
    }
    if let Some(value) = object.get(ENGINE) {
        let engine = value
            .as_str()
            .and_then(EngineKind::parse)
            .ok_or_else(|| "引擎只能是 Claude 或 Codex".to_string())?;
        changes.engine = Some(engine);
    }
    if let Some(value) = object.get(MODEL) {
        let model = match value {
            Value::Null => None,
            Value::String(text) => check_model(text)?,
            _ => return Err("模型名要是一段文字".to_string()),
        };
        changes.model = Some(model);
    }
    if let Some(value) = object.get(BUDGET) {
        changes.budget_usd = Some(check_budget(value)?);
    }
    if let Some(value) = object.get(TIMEOUT) {
        changes.timeout_minutes = Some(check_timeout(value)?);
    }
    Ok(changes)
}

/// An empty model means "the engine's own default".
pub fn check_model(text: &str) -> Result<Option<String>, String> {
    let model = text.trim();
    if model.is_empty() {
        return Ok(None);
    }
    if model.chars().count() > MAX_MODEL_CHARS {
        return Err(format!("模型名太长了（最多 {MAX_MODEL_CHARS} 个字符）"));
    }
    if model.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("模型名里不能有空格或换行".to_string());
    }
    Ok(Some(model.to_string()))
}

pub fn check_budget(value: &Value) -> Result<f64, String> {
    match value.as_f64() {
        Some(budget) if budget > 0.0 && budget <= MAX_BUDGET_USD => Ok(budget),
        _ => Err(format!(
            "每次的花费上限要是大于 0、不超过 {MAX_BUDGET_USD} 的美元数"
        )),
    }
}

pub fn check_timeout(value: &Value) -> Result<f64, String> {
    match value.as_f64() {
        Some(minutes) if (1.0..=MAX_TIMEOUT_MINUTES).contains(&minutes) => Ok(minutes),
        _ => Err(format!("时限要是 1 到 {MAX_TIMEOUT_MINUTES} 之间的分钟数")),
    }
}

fn number(value: f64) -> Value {
    // Whole numbers stay whole in the file (10, not 10.0).
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Value::Number(Number::from(value as i64))
    } else {
        Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

/// Puts `changes` into the settings object.
pub fn apply(object: &mut Map<String, Value>, changes: &Changes) {
    let mut set = |key: &str, value: Option<Value>| match value {
        Some(value) => {
            object.insert(key.to_string(), value);
        }
        None => {
            object.remove(key);
        }
    };
    if let Some(engine) = changes.engine {
        set(ENGINE, Some(Value::String(engine.program().to_string())));
    }
    if let Some(model) = &changes.model {
        set(MODEL, model.clone().map(Value::String));
    }
    if let Some(budget) = changes.budget_usd {
        set(BUDGET, Some(number(budget)));
    }
    if let Some(minutes) = changes.timeout_minutes {
        set(TIMEOUT, Some(number(minutes)));
    }
    if let Some(path) = &changes.kb_path {
        set(KB_PATH, Some(Value::String(path.clone())));
    }
    if let Some(shortcut) = &changes.shortcut {
        set(SHORTCUT, Some(Value::String(shortcut.clone())));
    }
    if let Some(enabled) = changes.bridge_enabled {
        set(BRIDGE_ENABLED, (!enabled).then_some(Value::Bool(false)));
    }
}

/// The settings object in `file`: empty when there is no file (or it is
/// empty), an error when it cannot be read as a JSON object.
pub fn read_object(file: &Path) -> Result<Map<String, Value>, String> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("读不了 config.json（{error}）")),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err("config.json 里不是一组设置（不是 JSON 对象）。为了不丢掉里面的内容，这次没有保存；请先修好或删掉它".to_string()),
        Err(error) => Err(format!(
            "config.json 不是有效的 JSON（{error}）。为了不丢掉里面的内容，这次没有保存；请先修好或删掉它"
        )),
    }
}

/// Changes the settings in `file` atomically, keeping every other key.
/// Returns the object as saved.
pub fn update(file: &Path, changes: &Changes) -> Result<Map<String, Value>, String> {
    let mut object = read_object(file)?;
    apply(&mut object, changes);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|error| format!("没能建立设置文件夹（{error}）"))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(object.clone()))
        .map_err(|error| format!("没能写出设置（{error}）"))?;
    files::write_atomic(file, &format!("{text}\n"))
        .map_err(|error| format!("没能保存 config.json（{error}）"))?;
    Ok(object)
}

/// Where a remote URL points, without anything that could be a secret:
/// the host of a network remote, or the folder of a remote on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTarget {
    /// `github.com`, `gitlab.example.org`; `None` for a folder on this Mac.
    pub host: Option<String>,
    /// The folder of a local remote (`/tmp/kb.git`).
    pub local_path: Option<String>,
}

/// Reads the host (or the local folder) of a git remote URL: `https://…`,
/// `ssh://…`, `git@host:path` and plain or `file://` paths. User names,
/// passwords, tokens, ports and paths of network remotes are dropped.
pub fn remote_target(url: &str) -> RemoteTarget {
    let url = url.trim();
    let local = |path: &str| RemoteTarget {
        host: None,
        local_path: Some(path.to_string()),
    };
    if let Some(path) = url.strip_prefix("file://") {
        return local(path);
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let host_port = authority.rsplit('@').next().unwrap_or_default();
            let host = if let Some(bracketed) = host_port.strip_prefix('[') {
                bracketed.split(']').next().unwrap_or_default()
            } else {
                host_port.split(':').next().unwrap_or_default()
            };
            return RemoteTarget {
                host: (!host.is_empty()).then(|| host.to_ascii_lowercase()),
                local_path: None,
            };
        }
    }
    // scp-like `user@host:path`: a colon before any slash.
    if let Some((before, _)) = url.split_once(':') {
        if !before.contains('/') && !before.is_empty() {
            let host = before.rsplit('@').next().unwrap_or_default();
            return RemoteTarget {
                host: (!host.is_empty()).then(|| host.to_ascii_lowercase()),
                local_path: None,
            };
        }
    }
    local(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use serde_json::json;

    #[test]
    fn reads_and_checks_the_ai_settings() {
        let changes = ai_changes(&json!({
            "engine": "codex", "model": " gpt-x ", "budgetUsd": 1.5, "runTimeoutMinutes": 20
        }))
        .unwrap();
        assert_eq!(
            changes,
            Changes {
                engine: Some(EngineKind::Codex),
                model: Some(Some("gpt-x".into())),
                budget_usd: Some(1.5),
                timeout_minutes: Some(20.0),
                ..Changes::default()
            }
        );
        // An empty model (or null) means the engine's own default.
        assert_eq!(
            ai_changes(&json!({ "model": "  " })).unwrap().model,
            Some(None)
        );
        assert_eq!(
            ai_changes(&json!({ "model": null })).unwrap().model,
            Some(None)
        );
        assert_eq!(ai_changes(&json!({})).unwrap(), Changes::default());

        for (bad, expected) in [
            (json!({ "engine": "gemini" }), "Claude 或 Codex"),
            (json!({ "budgetUsd": 0 }), "花费上限"),
            (json!({ "budgetUsd": 1001 }), "花费上限"),
            (json!({ "budgetUsd": "10" }), "花费上限"),
            (json!({ "runTimeoutMinutes": 0.5 }), "时限"),
            (json!({ "runTimeoutMinutes": 2000 }), "时限"),
            (json!({ "model": "a b" }), "空格"),
            (json!({ "model": 3 }), "文字"),
            (json!({ "kbPath": "/x" }), "不认识"),
            (json!([1]), "格式"),
        ] {
            let error = ai_changes(&bad).unwrap_err();
            assert!(error.contains(expected), "{bad}: {error}");
        }
        assert!(check_model(&"m".repeat(121)).unwrap_err().contains("太长"));
    }

    #[test]
    fn saving_keeps_unknown_keys_and_removes_cleared_ones() {
        let dir = TempDir::new("app-config");
        let file = dir.write(
            "config.json",
            r#"{"kbPath": "/kb", "claudePath": "/opt/claude", "future": {"x": [1, 2]}, "model": "sonnet"}"#,
        );
        let saved = update(
            &file,
            &Changes {
                engine: Some(EngineKind::Claude),
                model: Some(None),
                budget_usd: Some(10.0),
                timeout_minutes: Some(12.5),
                ..Changes::default()
            },
        )
        .unwrap();
        let on_disk: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(Value::Object(saved), on_disk);
        assert_eq!(
            on_disk,
            json!({
                "kbPath": "/kb",
                "claudePath": "/opt/claude",
                "future": {"x": [1, 2]},
                "engine": "claude",
                "budgetUsd": 10,
                "runTimeoutMinutes": 12.5
            })
        );
        assert!(fs::read_to_string(&file).unwrap().ends_with("}\n"));
        // No temporary files are left behind.
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["config.json"]);
    }

    #[test]
    fn a_missing_file_and_folder_are_created() {
        let dir = TempDir::new("app-config-new");
        let file = dir.path().join("app").join("config.json");
        update(
            &file,
            &Changes {
                shortcut: Some("Ctrl+Alt+K".into()),
                kb_path: Some("/Users/someone/kb".into()),
                ..Changes::default()
            },
        )
        .unwrap();
        let on_disk: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            on_disk,
            json!({ "captureShortcut": "Ctrl+Alt+K", "kbPath": "/Users/someone/kb" })
        );
    }

    #[test]
    fn a_file_that_is_not_a_settings_object_is_never_replaced() {
        let dir = TempDir::new("app-config-bad");
        for (name, text) in [("bad.json", "{ not json"), ("list.json", "[1, 2]")] {
            let file = dir.write(name, text);
            let error = update(&file, &Changes::default()).unwrap_err();
            assert!(error.contains("没有保存"), "{error}");
            assert_eq!(fs::read_to_string(&file).unwrap(), text);
        }
        let empty = dir.write("empty.json", "  \n");
        assert!(update(&empty, &Changes::default()).is_ok());
    }

    #[test]
    fn remote_urls_show_only_their_host_or_their_local_folder() {
        let host = |url: &str| remote_target(url).host;
        assert_eq!(
            host("https://github.com/someone/kb.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            host("https://someone:secret-token@GitHub.com:443/someone/kb.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            host("ssh://git@gitlab.example.org:2222/kb.git").as_deref(),
            Some("gitlab.example.org")
        );
        assert_eq!(
            host("git@github.com:someone/kb.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(host("http://[::1]:8080/kb").as_deref(), Some("::1"));
        for url in [
            "/tmp/dise-sandbox/remote.git",
            "file:///tmp/remote.git",
            "../remote.git",
        ] {
            let target = remote_target(url);
            assert_eq!(target.host, None, "{url}");
            assert!(target.local_path.unwrap().ends_with("remote.git"), "{url}");
        }
        // Nothing of the secret survives in what is shown.
        let shown =
            serde_json::to_string(&remote_target("https://u:secret-token@github.com/a/b")).unwrap();
        assert!(
            !shown.contains("secret") && !shown.contains("/a/b"),
            "{shown}"
        );
    }
}
