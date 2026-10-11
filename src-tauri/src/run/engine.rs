//! The exact command lines of the two engines, and the temporary auth-only
//! settings file Claude Code needs.
//!
//! **Claude Code** runs in restricted mode. The user's own settings may allow
//! every tool everywhere, so an unrestricted headless run could write anywhere
//! on the disk; `--restricted` confines reads and writes to the working
//! directory (the knowledge base), removes the tools that run commands, and
//! ignores user and project settings and hooks. Because it ignores the user's
//! settings, the few of them that carry the login (`apiKeyHelper` and the
//! `ANTHROPIC_*` / `CLAUDE_CODE_*` variables) are handed over in a private
//! temporary file that exists only while the engine runs. Its content is
//! never logged.
//!
//! **Codex** runs in its `workspace-write` sandbox with approvals off. It
//! keeps the user's model provider settings, but its plugins, apps, hooks,
//! memories, extra agents, browser and computer use, image generation and
//! every configured MCP server are switched off for the run.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// The only tools Claude Code gets: reading, writing and searching files.
pub const CLAUDE_TOOLS: &str = "Read,Write,Edit,Glob,Grep";
/// No MCP servers at all.
const NO_MCP_SERVERS: &str = r#"{"mcpServers":{}}"#;
/// Codex features that reach outside the knowledge base or start more
/// agents. Switched off as `-c features.<name>=false`, which (unlike
/// `--disable`) does not fail on a name a Codex version does not know.
const CODEX_DISABLED_FEATURES: &[&str] = &[
    "plugins",
    "apps",
    "hooks",
    "memories",
    "multi_agent",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "image_generation",
];
/// Auth files are named like this, so leftovers of a crash can be found.
const AUTH_PREFIX: &str = "claude-auth-";

/// The arguments of one Claude Code run. `auth` is the file from
/// [`write_auth_file`], `rules` the knowledge base's `AGENTS.md` (restricted
/// mode does not load project instructions by itself).
pub fn claude_args(
    prompt: &str,
    auth: &Path,
    rules: &Path,
    model: Option<&str>,
    budget_usd: f64,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-p".into(),
        prompt.into(),
        "--restricted".into(),
        "--settings".into(),
        auth.into(),
        "--tools".into(),
        CLAUDE_TOOLS.into(),
        "--permission-mode".into(),
        "acceptEdits".into(),
        "--permission-prompts".into(),
        "none".into(),
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        NO_MCP_SERVERS.into(),
        "--max-budget-usd".into(),
        format!("{budget_usd}").into(),
    ];
    if let Some(model) = model {
        args.push("--model".into());
        args.push(model.into());
    }
    args.extend(
        [
            "--output-format",
            "stream-json",
            "--verbose",
            "--append-system-prompt-file",
        ]
        .map(OsString::from),
    );
    args.push(rules.into());
    args
}

/// The arguments of one Codex run; the prompt goes to its stdin.
/// `final_message` is where Codex writes its last reply, `mcp_servers` the
/// servers of the user's configuration to switch off.
pub fn codex_args(
    root: &Path,
    final_message: &Path,
    model: Option<&str>,
    mcp_servers: &[String],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "exec".into(),
        "--json".into(),
        "--sandbox".into(),
        "workspace-write".into(),
        "-C".into(),
        root.into(),
        "-o".into(),
        final_message.into(),
        "-c".into(),
        r#"approval_policy="never""#.into(),
    ];
    for feature in CODEX_DISABLED_FEATURES {
        args.push("-c".into());
        args.push(format!("features.{feature}=false").into());
    }
    for server in mcp_servers {
        args.push("-c".into());
        args.push(format!("mcp_servers.{server}.enabled=false").into());
    }
    if let Some(model) = model {
        args.push("--model".into());
        args.push(model.into());
    }
    args.push("-".into());
    args
}

/// The names of the MCP servers in a Codex `config.toml`, sorted. Names that
/// cannot be written as a plain key on the command line are left out (second
/// value). No file, or one that is not TOML, has no servers.
pub fn codex_mcp_servers(config: &Path) -> (Vec<String>, Vec<String>) {
    let Some(table) = fs::read_to_string(config)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
    else {
        return (Vec::new(), Vec::new());
    };
    let Some(servers) = table.get("mcp_servers").and_then(toml::Value::as_table) else {
        return (Vec::new(), Vec::new());
    };
    let plain = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let mut names: Vec<String> = servers.keys().cloned().collect();
    names.sort();
    names.into_iter().partition(|name| plain(name))
}

/// What Claude Code needs from the user's `settings.json` to log in, and
/// nothing else: `apiKeyHelper`, and the `env` entries named `ANTHROPIC_*`,
/// `CLAUDE_CODE_*` or `API_TIMEOUT_MS`. Permissions, hooks, plugins and every
/// other setting stay behind.
pub fn auth_settings(user_settings: &Value) -> Value {
    let mut auth = Map::new();
    if let Some(helper) = user_settings.get("apiKeyHelper").and_then(Value::as_str) {
        auth.insert("apiKeyHelper".into(), json!(helper));
    }
    let env: Map<String, Value> = user_settings
        .get("env")
        .and_then(Value::as_object)
        .map(|env| {
            env.iter()
                .filter(|(name, _)| {
                    name.starts_with("ANTHROPIC_")
                        || name.starts_with("CLAUDE_CODE_")
                        || name.as_str() == "API_TIMEOUT_MS"
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    if !env.is_empty() {
        auth.insert("env".into(), Value::Object(env));
    }
    Value::Object(auth)
}

/// The auth-only settings file of one run. Deleted when dropped, however the
/// run ends.
#[derive(Debug)]
pub struct AuthFile(PathBuf);

impl AuthFile {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for AuthFile {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("run: could not delete the temporary auth file: {error}");
            }
        }
    }
}

/// Makes `dir` (and its parents) with access for the user only.
fn private_dir(dir: &Path) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("没能准备临时目录：{error}");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(failed)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(failed)
}

/// Writes the auth-only settings for one run into the private folder `dir`,
/// readable by the user only (mode 0600), taking the login from the user's
/// own Claude Code settings at `user_settings`.
///
/// Without such a file, or when it holds no login, the file is `{}`: Claude
/// Code then uses whatever other login it has (and says so if it has none).
/// Nothing of the content ever reaches an error message or the log.
pub fn write_auth_file(user_settings: &Path, dir: &Path, run_id: &str) -> Result<AuthFile, String> {
    let auth = match fs::read_to_string(user_settings) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(settings) => auth_settings(&settings),
            Err(_) => {
                log::warn!("run: the Claude Code settings file is not valid JSON; starting without its login settings");
                json!({})
            }
        },
        Err(_) => json!({}),
    };
    private_dir(dir)?;
    let path = dir.join(format!("{AUTH_PREFIX}{run_id}.json"));
    let failed = |error: std::io::Error| format!("没能写入临时的登录设置：{error}");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(failed)?;
    // From here on the file is removed again whatever happens.
    let guard = AuthFile(path);
    file.write_all(auth.to_string().as_bytes()).map_err(failed)?;
    file.sync_all().map_err(failed)?;
    Ok(guard)
}

/// Deletes auth files an earlier run left behind (the app was killed while
/// an engine ran). Returns how many there were.
pub fn remove_stale_auth_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(AUTH_PREFIX))
        })
        .filter(|entry| fs::remove_file(entry.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_runs_restricted_with_file_tools_only() {
        let args = strings(&claude_args(
            "任务提示词\n第二行",
            Path::new("/private/tmp/dise/claude-auth-1.json"),
            Path::new("/kb/AGENTS.md"),
            Some("sonnet"),
            1.5,
        ));
        assert_eq!(
            args,
            [
                "-p",
                "任务提示词\n第二行",
                "--restricted",
                "--settings",
                "/private/tmp/dise/claude-auth-1.json",
                "--tools",
                "Read,Write,Edit,Glob,Grep",
                "--permission-mode",
                "acceptEdits",
                "--permission-prompts",
                "none",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--max-budget-usd",
                "1.5",
                "--model",
                "sonnet",
                "--output-format",
                "stream-json",
                "--verbose",
                "--append-system-prompt-file",
                "/kb/AGENTS.md",
            ]
        );
    }

    #[test]
    fn claude_never_gets_a_tool_that_runs_commands_or_a_looser_mode() {
        let args = strings(&claude_args(
            "p",
            Path::new("/a.json"),
            Path::new("/kb/AGENTS.md"),
            None,
            10.0,
        ));
        assert!(!args.contains(&"--model".to_string()));
        let budget = args.iter().position(|arg| arg == "--max-budget-usd").unwrap();
        assert_eq!(args[budget + 1], "10");
        for forbidden in [
            "Bash",
            "WebFetch",
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "bypassPermissions",
            "--add-dir",
            "--allowedTools",
        ] {
            assert!(
                !args.iter().any(|arg| arg.contains(forbidden)),
                "{forbidden} in {args:?}"
            );
        }
        // Each option that takes several values is followed by another option,
        // so it can never swallow the prompt or a path.
        for variadic in ["--tools", "--mcp-config"] {
            let at = args.iter().position(|arg| arg == variadic).unwrap();
            assert!(args[at + 2].starts_with("--"), "{variadic}: {args:?}");
        }
    }

    #[test]
    fn codex_runs_sandboxed_without_plugins_or_mcp_servers() {
        let args = strings(&codex_args(
            Path::new("/kb"),
            Path::new("/runs/1/final-message.txt"),
            Some("gpt-x"),
            &["notion".to_string(), "chrome-devtools".to_string()],
        ));
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--sandbox",
                "workspace-write",
                "-C",
                "/kb",
                "-o",
                "/runs/1/final-message.txt",
                "-c",
                r#"approval_policy="never""#,
                "-c",
                "features.plugins=false",
                "-c",
                "features.apps=false",
                "-c",
                "features.hooks=false",
                "-c",
                "features.memories=false",
                "-c",
                "features.multi_agent=false",
                "-c",
                "features.browser_use=false",
                "-c",
                "features.browser_use_external=false",
                "-c",
                "features.computer_use=false",
                "-c",
                "features.in_app_browser=false",
                "-c",
                "features.image_generation=false",
                "-c",
                "mcp_servers.notion.enabled=false",
                "-c",
                "mcp_servers.chrome-devtools.enabled=false",
                "--model",
                "gpt-x",
                "-",
            ]
        );
        let plain = strings(&codex_args(Path::new("/kb"), Path::new("/f"), None, &[]));
        assert_eq!(plain.last().map(String::as_str), Some("-"));
        assert!(!plain.contains(&"--model".to_string()));
        for forbidden in ["danger-full-access", "--dangerously-bypass-approvals-and-sandbox"] {
            assert!(!plain.iter().any(|arg| arg.contains(forbidden)));
        }
    }

    #[test]
    fn finds_the_mcp_servers_of_a_codex_config() {
        let dir = TempDir::new("run-codex-config");
        let config = dir.write(
            "config.toml",
            r#"
model = "x"

[mcp_servers.notion]
url = "https://example.invalid/mcp"

[mcp_servers.chrome-devtools]
command = "npx"
args = ["-y", "chrome-devtools-mcp"]

[mcp_servers.chrome-devtools.env]
A = "1"

[mcp_servers."has.dot"]
command = "x"

[projects."/some/path"]
trust_level = "trusted"
"#,
        );
        assert_eq!(
            codex_mcp_servers(&config),
            (
                vec!["chrome-devtools".to_string(), "notion".to_string()],
                vec!["has.dot".to_string()]
            )
        );
        let none = dir.write("none.toml", "model = \"x\"\n");
        assert_eq!(codex_mcp_servers(&none), (vec![], vec![]));
        let broken = dir.write("broken.toml", "[mcp_servers.notion\n");
        assert_eq!(codex_mcp_servers(&broken), (vec![], vec![]));
        assert_eq!(
            codex_mcp_servers(&dir.path().join("missing.toml")),
            (vec![], vec![])
        );
    }

    #[test]
    fn only_the_login_leaves_the_users_claude_settings() {
        let settings = json!({
            "apiKeyHelper": "/usr/local/bin/print-key --profile work",
            "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.invalid",
                "ANTHROPIC_AUTH_TOKEN": "token-value",
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                "API_TIMEOUT_MS": "600000",
                "HTTPS_PROXY": "http://127.0.0.1:7890",
                "MY_OTHER_SECRET": "must-stay-behind",
                "XANTHROPIC_FAKE": "no"
            },
            "permissions": { "allow": ["Bash(*)", "Write(*)"], "defaultMode": "bypassPermissions" },
            "hooks": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "rm -rf /" }] }] },
            "enabledPlugins": { "x": true },
            "skipDangerousModePermissionPrompt": true
        });
        assert_eq!(
            auth_settings(&settings),
            json!({
                "apiKeyHelper": "/usr/local/bin/print-key --profile work",
                "env": {
                    "ANTHROPIC_BASE_URL": "https://gateway.example.invalid",
                    "ANTHROPIC_AUTH_TOKEN": "token-value",
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                    "API_TIMEOUT_MS": "600000"
                }
            })
        );
        assert_eq!(auth_settings(&json!({})), json!({}));
        assert_eq!(
            auth_settings(&json!({ "env": { "PATH": "/x" }, "apiKeyHelper": 42 })),
            json!({})
        );
    }

    #[test]
    fn the_auth_file_is_private_and_gone_when_dropped() {
        let dir = TempDir::new("run-auth");
        let source = dir.write(
            "settings.json",
            r#"{"apiKeyHelper": "print-key", "env": {"ANTHROPIC_BASE_URL": "https://gw.example.invalid", "OTHER": "x"}, "permissions": {"allow": ["Bash(*)"]}}"#,
        );
        let private = dir.path().join("private/tmp");

        let auth = write_auth_file(&source, &private, "2026-10-12T09-00-00-ingest").unwrap();
        let path = auth.path().to_path_buf();
        assert_eq!(
            path,
            private.join("claude-auth-2026-10-12T09-00-00-ingest.json")
        );
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&private), 0o700);
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({
                "apiKeyHelper": "print-key",
                "env": { "ANTHROPIC_BASE_URL": "https://gw.example.invalid" }
            })
        );
        // A second run with the same id must not reuse or overwrite the file.
        assert!(write_auth_file(&source, &private, "2026-10-12T09-00-00-ingest").is_err());
        assert!(path.exists(), "the failed attempt did not remove the first file");

        drop(auth);
        assert!(!path.exists());
    }

    #[test]
    fn a_missing_or_broken_settings_file_gives_an_empty_auth_file() {
        let dir = TempDir::new("run-auth-missing");
        let private = dir.path().join("tmp");
        let auth = write_auth_file(&dir.path().join("none.json"), &private, "a").unwrap();
        assert_eq!(fs::read_to_string(auth.path()).unwrap(), "{}");
        let broken = dir.write("broken.json", "{ \"apiKeyHelper\": \"secret-helper\" ");
        let auth = write_auth_file(&broken, &private, "b").unwrap();
        assert_eq!(fs::read_to_string(auth.path()).unwrap(), "{}");
    }

    #[test]
    fn leftovers_of_a_crash_are_removed() {
        let dir = TempDir::new("run-auth-stale");
        dir.write("claude-auth-old-1.json", "{}");
        dir.write("claude-auth-old-2.json", "{}");
        dir.write("something-else.json", "{}");
        assert_eq!(remove_stale_auth_files(dir.path()), 2);
        assert!(dir.path().join("something-else.json").exists());
        assert_eq!(remove_stale_auth_files(dir.path()), 0);
        assert_eq!(remove_stale_auth_files(&dir.path().join("missing")), 0);
    }
}
