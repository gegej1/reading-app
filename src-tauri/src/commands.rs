//! Tauri commands the frontend uses to read and write the knowledge base.
//!
//! | command    | arguments                 | returns                                   |
//! |------------|---------------------------|-------------------------------------------|
//! | `kb_root`  | –                         | `{ path, source, exists }`                |
//! | `kb_list`  | –                         | `[{ path, mtime }]` (all `*.md`, sorted)  |
//! | `kb_read`  | `path`                    | file content (UTF-8 string)               |
//! | `kb_write` | `path`, `content`, `message` | `{ committed, commit }`                |
//!
//! Paths are repo-relative POSIX paths. Errors are user-facing Chinese
//! strings. The root is resolved on every call, so changing `config.json`
//! takes effect without restarting. File system and git work runs on the
//! blocking thread pool so the UI never waits on it, and git runs with the
//! user's login-shell environment.
//!
//! Every call is logged with its name, repo-relative path, outcome and
//! duration; file contents and commit messages never are.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::kb::{files, git, paths, root};
use crate::shell_env::ShellEnvState;

/// Serializes writes, so two quick edits never race on git's index lock.
#[derive(Default)]
pub struct WriteLock(Arc<Mutex<()>>);

/// Where the knowledge base is and whether it exists.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbRootInfo {
    pub path: String,
    pub source: root::RootSource,
    pub exists: bool,
}

/// Longest error text written to the log.
const MAX_LOGGED_ERROR: usize = 300;

/// The log line for one command call: its name, the repo-relative path (if
/// any), the outcome and the duration. `detail` describes a successful result
/// (counts, flags); errors keep their user-facing message, shortened. Never
/// given file contents or commit messages, so it cannot log them.
pub fn call_line<T>(
    command: &str,
    path: Option<&str>,
    millis: u128,
    result: &Result<T, String>,
    detail: impl FnOnce(&T) -> String,
) -> (log::Level, String) {
    let target = path.map(|p| format!(" path={p:?}")).unwrap_or_default();
    match result {
        Ok(value) => {
            let detail = detail(value);
            let detail = if detail.is_empty() {
                String::new()
            } else {
                format!(" {detail}")
            };
            (
                log::Level::Info,
                format!("{command}{target} ok{detail} ({millis} ms)"),
            )
        }
        Err(error) => {
            let short: String = error.chars().take(MAX_LOGGED_ERROR).collect();
            let more = if error.chars().count() > MAX_LOGGED_ERROR {
                "…"
            } else {
                ""
            };
            (
                log::Level::Warn,
                format!("{command}{target} failed ({millis} ms): {short}{more}"),
            )
        }
    }
}

fn log_call<T>(
    command: &str,
    path: Option<&str>,
    started: Instant,
    result: &Result<T, String>,
    detail: impl FnOnce(&T) -> String,
) {
    let (level, line) = call_line(command, path, started.elapsed().as_millis(), result, detail);
    log::log!(level, "{line}");
}

/// Resolves the root from `DISE_KB_PATH`, `config.json` or the default.
fn resolve_root<R: Runtime>(app: &AppHandle<R>) -> Result<root::KbRoot, String> {
    let resolver = app.path();
    let config_file = resolver
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(root::CONFIG_FILE_NAME));
    let home = resolver.home_dir().ok();
    root::resolve_kb_root(
        std::env::var_os(root::ENV_VAR),
        config_file.as_deref(),
        home.as_deref(),
    )
}

/// The canonical root directory, or an error explaining which setting to fix.
fn canonical_root(kb_root: &root::KbRoot) -> Result<PathBuf, String> {
    match std::fs::canonicalize(&kb_root.path) {
        Ok(path) if path.is_dir() => Ok(path),
        Ok(_) => Err(format!(
            "知识库路径不是文件夹：{}（{}）",
            kb_root.path.display(),
            kb_root.source.describe()
        )),
        Err(_) => Err(format!(
            "找不到知识库文件夹：{}（{}）。可以设置环境变量 DISE_KB_PATH，或在 config.json 里填写 kbPath。",
            kb_root.path.display(),
            kb_root.source.describe()
        )),
    }
}

/// Runs blocking work off the async runtime.
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn kb_root<R: Runtime>(app: AppHandle<R>) -> Result<KbRootInfo, String> {
    let started = Instant::now();
    let result = resolve_root(&app).map(|kb_root| KbRootInfo {
        path: kb_root.path.to_string_lossy().into_owned(),
        source: kb_root.source,
        exists: kb_root.path.is_dir(),
    });
    // The folder itself is not logged: it may come from an environment variable.
    log_call("kb_root", None, started, &result, |info| {
        format!("source={:?} exists={}", info.source, info.exists)
    });
    result
}

#[tauri::command]
pub async fn kb_list<R: Runtime>(app: AppHandle<R>) -> Result<Vec<files::KbFile>, String> {
    let started = Instant::now();
    let result = match resolve_root(&app) {
        Ok(kb_root) => {
            blocking(move || {
                let root = canonical_root(&kb_root)?;
                files::list_markdown(&root).map_err(|e| format!("读取知识库目录失败：{e}"))
            })
            .await
        }
        Err(error) => Err(error),
    };
    log_call("kb_list", None, started, &result, |list| {
        format!("files={}", list.len())
    });
    result
}

#[tauri::command]
pub async fn kb_read<R: Runtime>(app: AppHandle<R>, path: String) -> Result<String, String> {
    let started = Instant::now();
    let logged_path = path.clone();
    let result = match resolve_root(&app) {
        Ok(kb_root) => {
            blocking(move || {
                let root = canonical_root(&kb_root)?;
                let file = paths::resolve_read(&root, &path).map_err(|e| e.to_string())?;
                std::fs::read_to_string(&file).map_err(|e| format!("读取文件失败：{path}（{e}）"))
            })
            .await
        }
        Err(error) => Err(error),
    };
    log_call("kb_read", Some(&logged_path), started, &result, |_| {
        String::new()
    });
    result
}

#[tauri::command]
pub async fn kb_write<R: Runtime>(
    app: AppHandle<R>,
    lock: State<'_, WriteLock>,
    shell_env: State<'_, ShellEnvState>,
    path: String,
    content: String,
    message: String,
) -> Result<git::WriteOutcome, String> {
    let started = Instant::now();
    let logged_path = path.clone();
    let lock = Arc::clone(&lock.0);
    let shell_env = shell_env.inner().clone();
    let result = match resolve_root(&app) {
        Ok(kb_root) => {
            blocking(move || {
                // A poisoned lock only means an earlier write panicked; keep going.
                let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let root = canonical_root(&kb_root)?;
                git::validate_message(&message)?;
                // Waits for the login-shell environment if it is still being captured.
                let env = &shell_env.get().env;
                // Check git first so a non-repo knowledge base is never half-updated.
                git::ensure_repo(&root, env)?;
                let target = paths::resolve_write(&root, &path).map_err(|e| e.to_string())?;
                files::write_atomic(&target, &content)
                    .map_err(|e| format!("写入文件失败：{path}（{e}）"))?;
                git::commit_path(&root, &path, &message, env)
                    .map_err(|e| format!("文件已写入，但 git 提交失败：{e}"))
            })
            .await
        }
        Err(error) => Err(error),
    };
    log_call(
        "kb_write",
        Some(&logged_path),
        started,
        &result,
        write_detail,
    );
    result
}

/// What a successful write did: whether it committed, and the short hash.
fn write_detail(outcome: &git::WriteOutcome) -> String {
    match &outcome.commit {
        Some(hash) => format!(
            "committed={} commit={}",
            outcome.committed,
            &hash[..hash.len().min(10)]
        ),
        None => format!("committed={}", outcome.committed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_name_path_outcome_and_duration() {
        let ok: Result<git::WriteOutcome, String> = Ok(git::WriteOutcome {
            committed: true,
            commit: Some("0123456789abcdef0123".into()),
        });
        assert_eq!(
            call_line("kb_write", Some("self/底色.md"), 42, &ok, write_detail),
            (
                log::Level::Info,
                "kb_write path=\"self/底色.md\" ok committed=true commit=0123456789 (42 ms)"
                    .to_string()
            )
        );

        let listed: Result<Vec<u8>, String> = Ok(vec![0; 52]);
        assert_eq!(
            call_line("kb_list", None, 7, &listed, |list| format!(
                "files={}",
                list.len()
            ))
            .1,
            "kb_list ok files=52 (7 ms)"
        );

        let read: Result<String, String> = Ok("文件内容绝不进日志".into());
        let (_, line) = call_line("kb_read", Some("wiki/概念/甲.md"), 1, &read, |_| {
            String::new()
        });
        assert_eq!(line, "kb_read path=\"wiki/概念/甲.md\" ok (1 ms)");
        assert!(!line.contains("文件内容"));
    }

    #[test]
    fn logs_errors_shortened() {
        let failed: Result<(), String> = Err("文件已写入，但 git 提交失败：index.lock".into());
        assert_eq!(
            call_line("kb_write", Some("a.md"), 3, &failed, |_| String::new()),
            (
                log::Level::Warn,
                "kb_write path=\"a.md\" failed (3 ms): 文件已写入，但 git 提交失败：index.lock"
                    .to_string()
            )
        );
        let long: Result<(), String> = Err("错".repeat(MAX_LOGGED_ERROR + 50));
        let (_, line) = call_line("kb_read", None, 0, &long, |_| String::new());
        assert!(line.ends_with('…'), "{line}");
        assert_eq!(line.matches('错').count(), MAX_LOGGED_ERROR);
    }
}
