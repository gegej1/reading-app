//! Tauri commands the frontend uses to read, write and sync the knowledge base.
//!
//! | command     | arguments                    | returns                                  |
//! |-------------|------------------------------|------------------------------------------|
//! | `kb_root`   | –                            | `{ path, source, exists }`               |
//! | `kb_list`   | –                            | `[{ path, mtime }]` (all `*.md`, sorted) |
//! | `kb_read`   | `path`                       | file content (UTF-8 string)              |
//! | `kb_read_head` | `path`                    | the content as of the last commit, or null |
//! | `kb_write`  | `path`, `content`, `message` | `{ committed, commit }`                  |
//! | `kb_create` | `path`, `content`, `message` | `{ committed, commit }`; never overwrites |
//! | `kb_status` | –                            | sync status (see `sync_service`)         |
//! | `kb_sync`   | –                            | sync status after a sync                 |
//!
//! Paths are repo-relative POSIX paths. Errors are user-facing Chinese
//! strings; a `kb_create` whose file exists fails with a message starting
//! with [`ops::EXISTS_PREFIX`]. The root is resolved on every call, so
//! changing `config.json` takes effect without restarting. File system and
//! git work runs on the blocking thread pool so the UI never waits on it, and
//! git runs with the user's login-shell environment.
//!
//! Writes, creates and syncs take turns through the knowledge-base lock
//! ([`KbLock`]): a write waits at most [`WRITE_WAIT`] for the operation in
//! progress and then fails with a "busy" message instead of hanging. While
//! an AI run is going on (see `crate::run`), writes are refused at once with
//! that message; only quick captures (new files directly in `inbox/`) still
//! go through, each as its own commit.
//!
//! Every call is logged with its name, repo-relative path, outcome and
//! duration (reads only at debug level: a load reads every file); file
//! contents and commit messages never are.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::ipc::{CommandScope, ScopeObjectMatch};
use tauri::{AppHandle, Manager, Runtime, State, Webview};

use crate::events::{self, KbChanged};
use crate::kb::lock::KbLock;
use crate::kb::{changes, files, git, ops, paths, root};
use crate::run::service::RunService;
use crate::run::Task;
use crate::shell_env::ShellEnvState;
use crate::sync_service::{SyncService, SyncStatus};

/// How long a write waits for the operation holding the lock (a sync, say).
pub const WRITE_WAIT: Duration = Duration::from_secs(45);
/// What the lock holder is called while a file is written or created.
pub const WRITE_OPERATION: &str = "写入";

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
/// given file contents or commit messages, so it cannot log them. Successes
/// are logged at `ok_level`, failures always as warnings.
pub fn call_line<T>(
    command: &str,
    path: Option<&str>,
    millis: u128,
    result: &Result<T, String>,
    ok_level: log::Level,
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
                ok_level,
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
    log_call_at(command, path, started, result, log::Level::Info, detail);
}

fn log_call_at<T>(
    command: &str,
    path: Option<&str>,
    started: Instant,
    result: &Result<T, String>,
    ok_level: log::Level,
    detail: impl FnOnce(&T) -> String,
) {
    let (level, line) = call_line(
        command,
        path,
        started.elapsed().as_millis(),
        result,
        ok_level,
        detail,
    );
    log::log!(level, "{line}");
}

/// Environment variable naming another folder for `config.json` and the run
/// records than `~/Library/Application Support/com.gegej1.dise` (for trying
/// the app on a copy of the knowledge base without touching the real settings).
pub const APP_DIR_VAR: &str = "DISE_APP_DIR";

/// The app's own folder: `config.json` and the run records live here.
pub fn app_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let chosen = std::env::var_os(APP_DIR_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute());
    chosen.or_else(|| app.path().app_config_dir().ok())
}

/// `config.json` in the app's folder, where `kbPath`, `captureShortcut` and
/// the run settings live.
pub fn config_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app_dir(app).map(|dir| dir.join(root::CONFIG_FILE_NAME))
}

/// Resolves the root from `DISE_KB_PATH`, `config.json` or the default.
fn resolve_root<R: Runtime>(app: &AppHandle<R>) -> Result<root::KbRoot, String> {
    let home = app.path().home_dir().ok();
    root::resolve_kb_root(
        std::env::var_os(root::ENV_VAR),
        config_file(app).as_deref(),
        home.as_deref(),
    )
}

/// The knowledge-base folder, canonical, for work outside a command (syncs,
/// command-line captures).
pub fn kb_folder<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    canonical_root(&resolve_root(app)?)
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
    // A load reads every file: the summary line comes from kb_list and the
    // frontend, so successful reads stay out of the log.
    log_call_at(
        "kb_read",
        Some(&logged_path),
        started,
        &result,
        log::Level::Debug,
        |_| String::new(),
    );
    result
}

/// The content of a Markdown file as of the last commit (HEAD), or `None`
/// when it is not in that commit: what a file was before an AI run (or any
/// other uncommitted change) touched it.
#[tauri::command]
pub async fn kb_read_head<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
    path: String,
) -> Result<Option<String>, String> {
    let started = Instant::now();
    let logged_path = path.clone();
    let shell_env = shell_env.inner().clone();
    let result = match resolve_root(&app) {
        Ok(kb_root) => {
            blocking(move || {
                paths::validate_rel_path(&path).map_err(|e| e.to_string())?;
                let root = canonical_root(&kb_root)?;
                let env = &shell_env.get().env;
                git::ensure_repo(&root, env)?;
                let bytes = changes::read_at(&root, env, "HEAD", &path)?;
                bytes
                    .map(|bytes| {
                        String::from_utf8(bytes).map_err(|_| format!("不是 UTF-8 文本：{path}"))
                    })
                    .transpose()
            })
            .await
        }
        Err(error) => Err(error),
    };
    log_call_at(
        "kb_read_head",
        Some(&logged_path),
        started,
        &result,
        log::Level::Debug,
        |_| String::new(),
    );
    result
}

#[tauri::command]
pub async fn kb_write<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    lock: State<'_, KbLock>,
    shell_env: State<'_, ShellEnvState>,
    path: String,
    content: String,
    message: String,
) -> Result<git::WriteOutcome, String> {
    let started = Instant::now();
    let logged_path = path.clone();
    let lock = lock.inner().clone();
    let shell_env = shell_env.inner().clone();
    let result = match resolve_root(&app) {
        Ok(kb_root) => {
            let path = path.clone();
            blocking(move || {
                let _guard = lock
                    .acquire(WRITE_OPERATION, WRITE_WAIT)
                    .map_err(|busy| busy.to_string())?;
                let root = canonical_root(&kb_root)?;
                // Waits for the login-shell environment if it is still being captured.
                let env = &shell_env.get().env;
                ops::write_file(&root, env, &path, &content, &message)
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
    if result.as_ref().is_ok_and(|outcome| outcome.committed) {
        events::files_written(
            &app,
            KbChanged::new("write", Some(path), Some(webview.label())),
        );
    }
    result
}

/// Folders a window may create files in, from its capability (for example
/// `{ "folder": "inbox" }` for the capture window). Without any, a window that
/// may call `kb_create` may create files anywhere in the knowledge base.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateScope {
    pub folder: String,
}

impl ScopeObjectMatch for CreateScope {
    type Input = str;

    /// Only files directly inside the folder, not in its subfolders.
    fn matches(&self, rel: &str) -> bool {
        rel.split_once('/')
            .is_some_and(|(folder, name)| folder == self.folder && !name.contains('/'))
    }
}

/// The folder quick captures go to (see `crate::kb::capture`).
const CAPTURE_FOLDER: &str = "inbox";
/// The folder of the answers 落裁决 works through.
const ANSWER_FOLDER: &str = "inbox/裁决";

/// True for a file directly in the capture folder: a quick capture (记一笔).
pub fn is_capture_path(rel: &str) -> bool {
    CreateScope {
        folder: CAPTURE_FOLDER.to_string(),
    }
    .matches(rel)
}

/// True for a new file directly in `inbox/裁决/`: an answer to a ruling or
/// the reason for rejecting an audit item.
pub fn is_answer_path(rel: &str) -> bool {
    rel.strip_prefix(ANSWER_FOLDER)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
}

/// Whether creating `rel` may go ahead while an AI run holds the knowledge
/// base (`running` is its task). Captures always may, each as a commit of
/// its own that the run leaves alone; so may answers, except while a
/// 落裁决 works through their folder. Everything else waits for the run.
pub fn creates_beside_run(rel: &str, running: Option<Task>) -> bool {
    is_capture_path(rel) || (is_answer_path(rel) && running != Some(Task::LandRulings))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn kb_create<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    scope: CommandScope<CreateScope>,
    lock: State<'_, KbLock>,
    shell_env: State<'_, ShellEnvState>,
    path: String,
    content: String,
    message: String,
) -> Result<git::WriteOutcome, String> {
    let started = Instant::now();
    let logged_path = path.clone();
    let result = if !scope.matches(&path) {
        let folders: Vec<&str> = scope
            .allows()
            .iter()
            .map(|allowed| allowed.folder.as_str())
            .collect();
        Err(format!(
            "这个窗口只能在 {} 里新建文件：{path}",
            folders.join("、")
        ))
    } else {
        match resolve_root(&app) {
            Ok(kb_root) => {
                let lock = lock.inner().clone();
                let shell_env = shell_env.inner().clone();
                let path = path.clone();
                let running = app
                    .try_state::<RunService>()
                    .and_then(|runs| runs.active_task());
                blocking(move || {
                    // Captures and answers keep working while an AI run is
                    // going on (see `creates_beside_run`); any other new
                    // file has to wait for the run to end.
                    let guard = if creates_beside_run(&path, running) {
                        lock.acquire_beside_job(WRITE_OPERATION, WRITE_WAIT)
                    } else {
                        lock.acquire(WRITE_OPERATION, WRITE_WAIT)
                    }
                    .map_err(|busy| busy.to_string())?;
                    let root = canonical_root(&kb_root)?;
                    let env = &shell_env.get().env;
                    let outcome = ops::create_file(&root, env, &path, &content, &message)
                        .map_err(|error| error.to_string())?;
                    if let Some(commit) = &outcome.commit {
                        guard.note_commit(commit, &path);
                    }
                    Ok(outcome)
                })
                .await
            }
            Err(error) => Err(error),
        }
    };
    log_call(
        "kb_create",
        Some(&logged_path),
        started,
        &result,
        write_detail,
    );
    if result.is_ok() {
        events::files_written(
            &app,
            KbChanged::new("create", Some(path), Some(webview.label())),
        );
    }
    result
}

#[tauri::command]
pub async fn kb_status(service: State<'_, SyncService>) -> Result<SyncStatus, String> {
    let service = service.inner().clone();
    blocking(move || Ok(service.status())).await
}

#[tauri::command]
pub async fn kb_sync(service: State<'_, SyncService>) -> Result<SyncStatus, String> {
    let service = service.inner().clone();
    blocking(move || Ok(service.sync_now("manual"))).await
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
            call_line(
                "kb_write",
                Some("self/底色.md"),
                42,
                &ok,
                log::Level::Info,
                write_detail
            ),
            (
                log::Level::Info,
                "kb_write path=\"self/底色.md\" ok committed=true commit=0123456789 (42 ms)"
                    .to_string()
            )
        );

        let listed: Result<Vec<u8>, String> = Ok(vec![0; 52]);
        assert_eq!(
            call_line("kb_list", None, 7, &listed, log::Level::Info, |list| {
                format!("files={}", list.len())
            })
            .1,
            "kb_list ok files=52 (7 ms)"
        );

        let read: Result<String, String> = Ok("文件内容绝不进日志".into());
        let (level, line) = call_line(
            "kb_read",
            Some("wiki/概念/甲.md"),
            1,
            &read,
            log::Level::Debug,
            |_| String::new(),
        );
        assert_eq!(level, log::Level::Debug);
        assert_eq!(line, "kb_read path=\"wiki/概念/甲.md\" ok (1 ms)");
        assert!(!line.contains("文件内容"));
    }

    #[test]
    fn create_scopes_allow_files_directly_in_their_folder() {
        let inbox = CreateScope {
            folder: "inbox".into(),
        };
        assert!(inbox.matches("inbox/2026-10-12T14-03-22-mac.md"));
        assert!(!inbox.matches("inbox/裁决/2026-10-12-Q-001.md"));
        assert!(!inbox.matches("self/底色.md"));
        assert!(!inbox.matches("inbox.md"));
        assert!(!inbox.matches("inboxes/a.md"));
        assert!(!inbox.matches("x/inbox/a.md"));
    }

    #[test]
    fn captures_are_new_files_directly_in_the_inbox() {
        assert!(is_capture_path("inbox/2026-10-12T14-03-22-mac.md"));
        assert!(!is_capture_path("inbox/裁决/2026-10-12T14-03-22-mac.md"));
        assert!(!is_capture_path("self/底色.md"));
        assert!(!is_capture_path("index.md"));
    }

    #[test]
    fn answers_are_new_files_directly_in_the_answer_folder() {
        assert!(is_answer_path("inbox/裁决/Q-001-2026-10-12T14-03-22-mac.md"));
        assert!(!is_answer_path("inbox/裁决/旧/Q-001.md"));
        assert!(!is_answer_path("inbox/裁决/"));
        assert!(!is_answer_path("inbox/裁决"));
        assert!(!is_answer_path("inbox/裁决x/a.md"));
        assert!(!is_answer_path("inbox/2026-10-12T14-03-22-mac.md"));
    }

    #[test]
    fn captures_always_and_answers_mostly_go_through_beside_a_run() {
        let capture = "inbox/2026-10-12T14-03-22-mac.md";
        let answer = "inbox/裁决/Q-001-2026-10-12T14-03-22-mac.md";
        for running in [None, Some(Task::Ingest), Some(Task::LandRulings)] {
            assert!(creates_beside_run(capture, running), "{running:?}");
            assert!(!creates_beside_run("self/判例/P-009.md", running), "{running:?}");
        }
        assert!(creates_beside_run(answer, None));
        assert!(creates_beside_run(answer, Some(Task::Ingest)));
        // 落裁决 works through inbox/裁决/: a new answer waits for it.
        assert!(!creates_beside_run(answer, Some(Task::LandRulings)));
    }

    #[test]
    fn logs_errors_shortened() {
        let failed: Result<(), String> = Err("文件已写入，但 git 提交失败：index.lock".into());
        assert_eq!(
            call_line(
                "kb_write",
                Some("a.md"),
                3,
                &failed,
                log::Level::Info,
                |_| String::new()
            ),
            (
                log::Level::Warn,
                "kb_write path=\"a.md\" failed (3 ms): 文件已写入，但 git 提交失败：index.lock"
                    .to_string()
            )
        );
        let long: Result<(), String> = Err("错".repeat(MAX_LOGGED_ERROR + 50));
        let (level, line) = call_line("kb_read", None, 0, &long, log::Level::Debug, |_| {
            String::new()
        });
        assert_eq!(level, log::Level::Warn, "failures are always warnings");
        assert!(line.ends_with('…'), "{line}");
        assert_eq!(line.matches('错').count(), MAX_LOGGED_ERROR);
    }
}
