//! The engine settings in `config.json` (next to `kbPath`):
//!
//! | key                 | meaning                                         | default  |
//! |---------------------|-------------------------------------------------|----------|
//! | `engine`            | `"claude"` or `"codex"`                         | `claude` |
//! | `model`             | model name or alias for that engine             | engine's |
//! | `budgetUsd`         | spending cap of one Claude run, in US dollars   | `10`     |
//! | `runTimeoutMinutes` | the engine is stopped after this long           | `30`     |
//! | `claudePath`        | the `claude` program to run instead of the one on `PATH` | – |
//! | `codexPath`         | the same for `codex`                            | –        |
//!
//! Read at the start of every run, so a change takes effect without a
//! restart. A value that makes no sense falls back to its default and is
//! reported, never guessed at.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::EngineKind;

pub const DEFAULT_BUDGET_USD: f64 = 10.0;
pub const DEFAULT_TIMEOUT_MINUTES: u64 = 30;
/// More than this is taken for a typo.
const MAX_BUDGET_USD: f64 = 1000.0;
const MAX_TIMEOUT_MINUTES: u64 = 24 * 60;

/// How runs are started unless a request says otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSettings {
    pub engine: EngineKind,
    pub model: Option<String>,
    pub budget_usd: f64,
    pub timeout: Duration,
    pub claude_path: Option<String>,
    pub codex_path: Option<String>,
}

impl Default for RunSettings {
    fn default() -> Self {
        RunSettings {
            engine: EngineKind::Claude,
            model: None,
            budget_usd: DEFAULT_BUDGET_USD,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_MINUTES * 60),
            claude_path: None,
            codex_path: None,
        }
    }
}

impl RunSettings {
    /// The program to start for `engine`: the configured path, else its name
    /// (looked up on the user's `PATH`).
    pub fn program(&self, engine: EngineKind) -> String {
        let configured = match engine {
            EngineKind::Claude => &self.claude_path,
            EngineKind::Codex => &self.codex_path,
        };
        configured
            .clone()
            .unwrap_or_else(|| engine.program().to_string())
    }
}

/// What one run was asked to use instead of the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub engine: Option<EngineKind>,
    pub model: Option<String>,
}

/// The engine and model of one run.
///
/// The configured `model` belongs to the configured engine: when a run asks
/// for the other engine without naming a model, that engine's own default is
/// used rather than a model name it would not know.
pub fn choose(settings: &RunSettings, overrides: &Overrides) -> (EngineKind, Option<String>) {
    let engine = overrides.engine.unwrap_or(settings.engine);
    let asked = overrides
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(String::from);
    let model = asked.or_else(|| {
        (engine == settings.engine)
            .then(|| settings.model.clone())
            .flatten()
    });
    (engine, model)
}

/// Reads the settings from `config.json`. A missing or unreadable file gives
/// the defaults; the second value lists what was ignored and why.
pub fn read(config_file: Option<&Path>) -> (RunSettings, Vec<String>) {
    let json = config_file
        .and_then(|file| fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match json {
        Some(json) => from_json(&json),
        None => (RunSettings::default(), Vec::new()),
    }
}

fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

/// See [`read`].
pub fn from_json(json: &Value) -> (RunSettings, Vec<String>) {
    let mut settings = RunSettings::default();
    let mut problems = Vec::new();

    if let Some(value) = json.get("engine").filter(|value| !value.is_null()) {
        match value.as_str().and_then(EngineKind::parse) {
            Some(engine) => settings.engine = engine,
            None => problems.push(
                "config.json 里的 engine 只能是 \"claude\" 或 \"codex\"，这次按 claude 处理"
                    .to_string(),
            ),
        }
    }
    settings.model = text(json, "model");
    settings.claude_path = text(json, "claudePath");
    settings.codex_path = text(json, "codexPath");

    if let Some(value) = json.get("budgetUsd").filter(|value| !value.is_null()) {
        match value.as_f64() {
            Some(budget) if budget > 0.0 && budget <= MAX_BUDGET_USD => {
                settings.budget_usd = budget;
            }
            _ => problems.push(format!(
                "config.json 里的 budgetUsd 要是 0 到 {MAX_BUDGET_USD} 之间的数，这次按 {DEFAULT_BUDGET_USD} 美元处理"
            )),
        }
    }
    if let Some(value) = json.get("runTimeoutMinutes").filter(|value| !value.is_null()) {
        match value.as_f64() {
            Some(minutes) if minutes >= 1.0 && minutes <= MAX_TIMEOUT_MINUTES as f64 => {
                settings.timeout = Duration::from_secs_f64(minutes * 60.0);
            }
            _ => problems.push(format!(
                "config.json 里的 runTimeoutMinutes 要是 1 到 {MAX_TIMEOUT_MINUTES} 之间的数，这次按 {DEFAULT_TIMEOUT_MINUTES} 分钟处理"
            )),
        }
    }
    (settings, problems)
}

/// The settings as the settings screen shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub engine: EngineKind,
    pub model: Option<String>,
    pub budget_usd: f64,
    pub run_timeout_minutes: f64,
    pub claude_path: Option<String>,
    pub codex_path: Option<String>,
    /// Where `claude` and `codex` were found (the configured path, else on
    /// the user's `PATH`); `None` when they were not.
    pub claude_program: Option<String>,
    pub codex_program: Option<String>,
    /// Settings that were ignored, in plain words.
    pub problems: Vec<String>,
}

impl SettingsView {
    pub fn new(
        settings: &RunSettings,
        problems: Vec<String>,
        find: impl Fn(&str) -> Option<String>,
    ) -> Self {
        SettingsView {
            engine: settings.engine,
            model: settings.model.clone(),
            budget_usd: settings.budget_usd,
            run_timeout_minutes: settings.timeout.as_secs_f64() / 60.0,
            claude_path: settings.claude_path.clone(),
            codex_path: settings.codex_path.clone(),
            claude_program: find(&settings.program(EngineKind::Claude)),
            codex_program: find(&settings.program(EngineKind::Codex)),
            problems,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use serde_json::json;

    #[test]
    fn defaults_apply_without_a_config_file() {
        let (settings, problems) = read(None);
        assert_eq!(settings, RunSettings::default());
        assert_eq!(settings.engine, EngineKind::Claude);
        assert_eq!(settings.budget_usd, 10.0);
        assert_eq!(settings.timeout, Duration::from_secs(30 * 60));
        assert!(problems.is_empty());

        let dir = TempDir::new("run-settings-missing");
        for file in [
            dir.path().join("none.json"),
            dir.write("bad.json", "{ not json"),
            dir.write("other.json", r#"{"kbPath": "/kb"}"#),
        ] {
            assert_eq!(read(Some(&file)), (RunSettings::default(), Vec::new()));
        }
    }

    #[test]
    fn reads_every_setting() {
        let dir = TempDir::new("run-settings");
        let file = dir.write(
            "config.json",
            r#"{"kbPath": "/kb", "engine": "codex", "model": " gpt-x ", "budgetUsd": 1.5,
                "runTimeoutMinutes": 5, "claudePath": "/opt/claude", "codexPath": "/opt/codex"}"#,
        );
        let (settings, problems) = read(Some(&file));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            settings,
            RunSettings {
                engine: EngineKind::Codex,
                model: Some("gpt-x".into()),
                budget_usd: 1.5,
                timeout: Duration::from_secs(300),
                claude_path: Some("/opt/claude".into()),
                codex_path: Some("/opt/codex".into()),
            }
        );
        assert_eq!(settings.program(EngineKind::Claude), "/opt/claude");
        assert_eq!(settings.program(EngineKind::Codex), "/opt/codex");
        assert_eq!(RunSettings::default().program(EngineKind::Codex), "codex");
    }

    #[test]
    fn nonsense_falls_back_to_the_default_and_is_reported() {
        let (settings, problems) = from_json(&json!({
            "engine": "gpt",
            "model": "",
            "budgetUsd": -3,
            "runTimeoutMinutes": 0,
        }));
        assert_eq!(settings, RunSettings::default());
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("engine"));
        assert!(problems[1].contains("budgetUsd"));
        assert!(problems[2].contains("runTimeoutMinutes"));

        for bad in [json!("10"), json!(1001), json!(0)] {
            let (settings, problems) = from_json(&json!({ "budgetUsd": bad }));
            assert_eq!(settings.budget_usd, DEFAULT_BUDGET_USD);
            assert_eq!(problems.len(), 1);
        }
        // null means "not set".
        let (_, problems) = from_json(&json!({ "engine": null, "budgetUsd": null }));
        assert!(problems.is_empty());
    }

    #[test]
    fn a_request_overrides_engine_and_model() {
        let settings = RunSettings {
            engine: EngineKind::Claude,
            model: Some("sonnet".into()),
            ..RunSettings::default()
        };
        assert_eq!(
            choose(&settings, &Overrides::default()),
            (EngineKind::Claude, Some("sonnet".into()))
        );
        assert_eq!(
            choose(
                &settings,
                &Overrides {
                    engine: None,
                    model: Some(" opus ".into())
                }
            ),
            (EngineKind::Claude, Some("opus".into()))
        );
        // The configured model is Claude's: Codex gets its own default.
        assert_eq!(
            choose(
                &settings,
                &Overrides {
                    engine: Some(EngineKind::Codex),
                    model: None
                }
            ),
            (EngineKind::Codex, None)
        );
        assert_eq!(
            choose(
                &settings,
                &Overrides {
                    engine: Some(EngineKind::Codex),
                    model: Some("gpt-x".into())
                }
            ),
            (EngineKind::Codex, Some("gpt-x".into()))
        );
        assert_eq!(
            choose(
                &settings,
                &Overrides {
                    engine: Some(EngineKind::Claude),
                    model: Some("  ".into())
                }
            ),
            (EngineKind::Claude, Some("sonnet".into()))
        );
    }

    #[test]
    fn the_view_serializes_for_the_settings_screen() {
        let view = SettingsView::new(&RunSettings::default(), vec!["一个问题".into()], |program| {
            (program == "claude").then(|| "/usr/local/bin/claude".to_string())
        });
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            json!({
                "engine": "claude",
                "model": null,
                "budgetUsd": 10.0,
                "runTimeoutMinutes": 30.0,
                "claudePath": null,
                "codexPath": null,
                "claudeProgram": "/usr/local/bin/claude",
                "codexProgram": null,
                "problems": ["一个问题"],
            })
        );
    }
}
