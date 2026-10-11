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
//! blocking thread pool so the UI never waits on it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::kb::{files, git, paths, root};

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
    let kb_root = resolve_root(&app)?;
    Ok(KbRootInfo {
        path: kb_root.path.to_string_lossy().into_owned(),
        source: kb_root.source,
        exists: kb_root.path.is_dir(),
    })
}

#[tauri::command]
pub async fn kb_list<R: Runtime>(app: AppHandle<R>) -> Result<Vec<files::KbFile>, String> {
    let kb_root = resolve_root(&app)?;
    blocking(move || {
        let root = canonical_root(&kb_root)?;
        files::list_markdown(&root).map_err(|e| format!("读取知识库目录失败：{e}"))
    })
    .await
}

#[tauri::command]
pub async fn kb_read<R: Runtime>(app: AppHandle<R>, path: String) -> Result<String, String> {
    let kb_root = resolve_root(&app)?;
    blocking(move || {
        let root = canonical_root(&kb_root)?;
        let file = paths::resolve_read(&root, &path).map_err(|e| e.to_string())?;
        std::fs::read_to_string(&file).map_err(|e| format!("读取文件失败：{path}（{e}）"))
    })
    .await
}

#[tauri::command]
pub async fn kb_write<R: Runtime>(
    app: AppHandle<R>,
    lock: State<'_, WriteLock>,
    path: String,
    content: String,
    message: String,
) -> Result<git::WriteOutcome, String> {
    let kb_root = resolve_root(&app)?;
    let lock = Arc::clone(&lock.0);
    blocking(move || {
        // A poisoned lock only means an earlier write panicked; keep going.
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = canonical_root(&kb_root)?;
        git::validate_message(&message)?;
        // Check git first so a non-repo knowledge base is never half-updated.
        git::ensure_repo(&root)?;
        let target = paths::resolve_write(&root, &path).map_err(|e| e.to_string())?;
        files::write_atomic(&target, &content)
            .map_err(|e| format!("写入文件失败：{path}（{e}）"))?;
        git::commit_path(&root, &path, &message)
            .map_err(|e| format!("文件已写入，但 git 提交失败：{e}"))
    })
    .await
}
