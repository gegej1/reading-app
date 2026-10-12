//! Tauri commands of the settings screen (main window only).
//!
//! | command                  | arguments  | returns / does                                       |
//! |--------------------------|------------|------------------------------------------------------|
//! | `settings_get`           | –          | everything the screen shows ([`SettingsInfo`])        |
//! | `settings_save_ai`       | `settings` | saves engine, model, budget and time limit            |
//! | `settings_check_kb`      | `path`     | whether a folder can be the knowledge base            |
//! | `settings_pick_kb`       | –          | a folder chosen in a dialog, with its check (or null) |
//! | `settings_set_kb`        | `path`     | saves `kbPath` (only a git repository root)           |
//! | `settings_set_shortcut`  | `setting`  | registers the capture shortcut live, then saves it    |
//! | `settings_set_autostart` | `enabled`  | launch at login on or off; whether it is on now       |
//! | `settings_open_folder`   | `which`    | opens the logs (`logs`), run records (`runs`) or iPhone (`bridge`) folder |
//! | `settings_set_bridge`    | `enabled`  | turns the iPhone bridge on or off                     |
//!
//! Every save goes through `crate::app_config`: atomic, and keys the app does
//! not know stay in `config.json`. Errors are user-facing Chinese strings.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::app_config::{self, Changes, RemoteTarget};
use crate::bridge::{BridgeService, BridgeStatus};
use crate::capture_window;
use crate::commands;
use crate::events::{self, KbChanged};
use crate::kb::{git, root};
use crate::run::service::RunService;
use crate::run::settings::{self as run_settings, SettingsView};
use crate::shell_env::{ChildEnv, ShellEnvState};
use crate::shortcut::{self, ShortcutHolder, ShortcutStatus};
use crate::sync_service::SyncService;
use crate::tray;

/// The folder of the kept runs inside the app's folder (see `crate::run_commands`).
const RUNS_DIR: &str = "runs";

/// The remote syncing goes to, as far as it can be shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// The remote's name (`origin`).
    pub name: String,
    #[serde(flatten)]
    pub target: RemoteTarget,
    /// The current branch follows a branch on this remote (so syncing works).
    pub tracked: bool,
}

/// Whether a folder can be the knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbFolderCheck {
    /// As given, or the canonical folder when it exists.
    pub path: String,
    pub ok: bool,
    /// Why it cannot be used, in plain words.
    pub problem: Option<String>,
    /// It has an `AGENTS.md` (入库 and 落裁决 need it).
    pub has_rules: bool,
    pub remote: Option<RemoteInfo>,
}

/// Where the knowledge base is now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbInfo {
    pub path: String,
    pub source: root::RootSource,
    pub check: KbFolderCheck,
}

/// Everything the settings screen shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub version: String,
    pub run: SettingsView,
    pub kb: Option<KbInfo>,
    /// Why the knowledge-base folder could not be worked out.
    pub kb_problem: Option<String>,
    pub capture_shortcut: Option<ShortcutStatus>,
    /// Launch at login; `None` when it cannot be told.
    pub autostart: Option<bool>,
    /// `config.json` cannot be read: saving is refused until it is fixed.
    pub config_problem: Option<String>,
    /// The iPhone bridge; `None` when the app has none (tests).
    pub bridge: Option<BridgeStatus>,
}

/// The remote the current branch syncs with (or `origin`), shown without
/// anything that could be a secret.
fn remote_of(root: &Path, env: &ChildEnv) -> Option<RemoteInfo> {
    let git = |args: &[&str]| -> Option<String> {
        let mut command = git::plumbing(root, env);
        command.args(args);
        let done = git::finish(command, git::LOCAL_TIMEOUT).ok()?;
        let out = done.stdout.trim().to_string();
        (done.success() && !out.is_empty()).then_some(out)
    };
    let branch = git(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let tracked = branch
        .as_deref()
        .and_then(|branch| git(&["config", "--get", &format!("branch.{branch}.remote")]))
        .filter(|name| name != "." && !name.starts_with('-'));
    let name = tracked.clone().or_else(|| {
        git(&["remote"]).and_then(|names| {
            let names: Vec<&str> = names.lines().collect();
            names
                .iter()
                .find(|name| **name == "origin")
                .or_else(|| names.first())
                .map(|name| name.to_string())
        })
    })?;
    let url = git(&["remote", "get-url", "--", &name])?;
    Some(RemoteInfo {
        target: app_config::remote_target(&url),
        tracked: tracked.as_deref() == Some(name.as_str()),
        name,
    })
}

/// Checks a folder: it must exist and be the root of a git repository.
pub fn check_folder(path: &Path, env: &ChildEnv) -> KbFolderCheck {
    let failed = |path: &Path, problem: String| KbFolderCheck {
        path: path.to_string_lossy().into_owned(),
        ok: false,
        problem: Some(problem),
        has_rules: false,
        remote: None,
    };
    if !path.is_absolute() {
        return failed(path, "要选一个完整的文件夹路径".to_string());
    }
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) if canonical.is_dir() => canonical,
        Ok(_) => return failed(path, "这不是一个文件夹".to_string()),
        Err(_) => return failed(path, "找不到这个文件夹".to_string()),
    };
    if let Err(problem) = git::ensure_repo(&canonical, env) {
        let problem = if problem.contains("根目录") {
            "这个文件夹在一个 git 仓库里面，但不是仓库的根目录。请选仓库最外层的那个文件夹"
                .to_string()
        } else if problem.starts_with("无法运行 git") {
            problem
        } else {
            "这个文件夹不是 git 仓库。知识库要是一个 git 仓库的根目录（里面有 .git）".to_string()
        };
        return failed(&canonical, problem);
    }
    KbFolderCheck {
        path: canonical.to_string_lossy().into_owned(),
        ok: true,
        problem: None,
        has_rules: canonical.join("AGENTS.md").is_file(),
        remote: remote_of(&canonical, env),
    }
}

fn config_file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    commands::config_file(app).ok_or_else(|| "找不到 App 的设置文件夹".to_string())
}

/// Runs blocking work off the async runtime and logs how the command went
/// (never the values: paths and settings stay out of the log).
async fn call<T, F>(command: &'static str, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("后台任务失败：{error}"))
        .and_then(|result| result);
    let millis = started.elapsed().as_millis();
    match &result {
        Ok(_) => log::info!("{command} ok ({millis} ms)"),
        Err(error) => log::warn!("{command} failed ({millis} ms): {error}"),
    }
    result
}

fn autostart<R: Runtime>(app: &AppHandle<R>) -> Option<bool> {
    use tauri_plugin_autostart::AutoLaunchManager;
    app.try_state::<AutoLaunchManager>()
        .and_then(|manager| manager.is_enabled().ok())
}

fn info<R: Runtime>(app: &AppHandle<R>, env: &ChildEnv) -> SettingsInfo {
    let config = commands::config_file(app);
    let (settings, problems) = run_settings::read(config.as_deref());
    let run = SettingsView::new(&settings, problems, |program| {
        env.find_program(program)
            .map(|path| path.to_string_lossy().into_owned())
    });
    let home = app.path().home_dir().ok();
    let (kb, kb_problem) = match root::resolve_kb_root(
        std::env::var_os(root::ENV_VAR),
        config.as_deref(),
        home.as_deref(),
    ) {
        Ok(found) => (
            Some(KbInfo {
                path: found.path.to_string_lossy().into_owned(),
                source: found.source,
                check: check_folder(&found.path, env),
            }),
            None,
        ),
        Err(problem) => (None, Some(problem)),
    };
    SettingsInfo {
        version: app.package_info().version.to_string(),
        run,
        kb,
        kb_problem,
        capture_shortcut: app.try_state::<ShortcutHolder>().map(|holder| holder.get()),
        autostart: autostart(app),
        config_problem: config
            .as_deref()
            .and_then(|file| app_config::read_object(file).err()),
        bridge: app.try_state::<BridgeService>().map(|bridge| bridge.status()),
    }
}

#[tauri::command]
pub async fn settings_get<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
) -> Result<SettingsInfo, String> {
    let shell_env = shell_env.inner().clone();
    call("settings_get", move || Ok(info(&app, &shell_env.get().env))).await
}

#[tauri::command]
pub async fn settings_save_ai<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
    settings: Value,
) -> Result<SettingsInfo, String> {
    let shell_env = shell_env.inner().clone();
    call("settings_save_ai", move || {
        let changes = app_config::ai_changes(&settings)?;
        app_config::update(&config_file(&app)?, &changes)?;
        Ok(info(&app, &shell_env.get().env))
    })
    .await
}

#[tauri::command]
pub async fn settings_check_kb(
    shell_env: State<'_, ShellEnvState>,
    path: String,
) -> Result<KbFolderCheck, String> {
    let shell_env = shell_env.inner().clone();
    call("settings_check_kb", move || {
        Ok(check_folder(Path::new(path.trim()), &shell_env.get().env))
    })
    .await
}

/// A folder chosen in the dialog, with its check.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFolder {
    pub path: String,
    pub check: KbFolderCheck,
}

#[tauri::command]
pub async fn settings_pick_kb<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
) -> Result<Option<PickedFolder>, String> {
    use tauri_plugin_dialog::DialogExt;
    let shell_env = shell_env.inner().clone();
    call("settings_pick_kb", move || {
        if app.try_state::<tauri_plugin_dialog::Dialog<R>>().is_none() {
            return Err("这里没法打开选择文件夹的窗口".to_string());
        }
        let mut dialog = app.dialog().file().set_title("选择知识库文件夹");
        if let Some(window) = app.get_webview_window(crate::window::MAIN) {
            dialog = dialog.set_parent(&window);
        }
        if let Ok(current) = commands::kb_folder(&app) {
            if let Some(parent) = current.parent() {
                dialog = dialog.set_directory(parent);
            }
        }
        let Some(picked) = dialog.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = picked
            .into_path()
            .map_err(|error| format!("没能读出选中的文件夹（{error}）"))?;
        let check = check_folder(&path, &shell_env.get().env);
        Ok(Some(PickedFolder {
            path: check.path.clone(),
            check,
        }))
    })
    .await
}

#[tauri::command]
pub async fn settings_set_kb<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
    path: String,
) -> Result<SettingsInfo, String> {
    let shell_env = shell_env.inner().clone();
    call("settings_set_kb", move || {
        if let Some(task) = app
            .try_state::<RunService>()
            .and_then(|runs| runs.active_task())
        {
            return Err(format!("正在{}，结束后再换知识库", task.label()));
        }
        let env = &shell_env.get().env;
        let check = check_folder(Path::new(path.trim()), env);
        if !check.ok {
            return Err(check
                .problem
                .unwrap_or_else(|| "这个文件夹不能用".to_string()));
        }
        app_config::update(
            &config_file(&app)?,
            &Changes {
                kb_path: Some(check.path.clone()),
                ..Changes::default()
            },
        )?;
        log::info!("knowledge-base folder changed in the settings");
        // The windows load the new folder; the next sync looks at it.
        events::kb_changed(&app, &KbChanged::new("settings", None, None));
        if let Some(service) = app.try_state::<SyncService>() {
            service.schedule(std::time::Duration::from_secs(1), "settings");
        }
        Ok(info(&app, env))
    })
    .await
}

#[tauri::command]
pub async fn settings_set_shortcut<R: Runtime>(
    app: AppHandle<R>,
    setting: String,
) -> Result<ShortcutStatus, String> {
    call("settings_set_shortcut", move || {
        let status = shortcut::replace(&app, &setting, capture_window::toggle)?;
        tray::set_capture_shortcut(&app, &status);
        app_config::update(
            &config_file(&app)?,
            &Changes {
                shortcut: Some(status.setting.clone()),
                ..Changes::default()
            },
        )
        .map_err(|error| format!("快捷键已经换成 {}，但没能保存：{error}", status.label))?;
        Ok(status)
    })
    .await
}

#[tauri::command]
pub async fn settings_set_autostart<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<bool, String> {
    use tauri_plugin_autostart::AutoLaunchManager;
    call("settings_set_autostart", move || {
        let manager = app
            .try_state::<AutoLaunchManager>()
            .ok_or_else(|| "这里没法设置开机启动".to_string())?;
        let changed = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        changed.map_err(|error| {
            format!(
                "没能{}开机启动（{error}）",
                if enabled { "打开" } else { "关掉" }
            )
        })?;
        manager
            .is_enabled()
            .map_err(|error| format!("没能读出开机启动的状态（{error}）"))
    })
    .await
}

#[tauri::command]
pub async fn settings_open_folder<R: Runtime>(
    app: AppHandle<R>,
    which: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    call("settings_open_folder", move || {
        let folder = match which.as_str() {
            "logs" => app
                .path()
                .app_log_dir()
                .map_err(|error| format!("找不到日志文件夹（{error}）"))?,
            "runs" => commands::app_dir(&app)
                .ok_or_else(|| "找不到 App 的数据文件夹".to_string())?
                .join(RUNS_DIR),
            "bridge" => {
                let folder = crate::bridge_commands::setting(&app).dir?;
                if !folder.is_dir() {
                    return Err("iPhone 同步文件夹还没有建立".to_string());
                }
                folder
            }
            other => return Err(format!("不认识的文件夹：{other}")),
        };
        std::fs::create_dir_all(&folder).map_err(|error| format!("没能建立文件夹（{error}）"))?;
        if app.try_state::<tauri_plugin_opener::Opener<R>>().is_none() {
            return Err("这里没法打开文件夹".to_string());
        }
        app.opener()
            .open_path(folder.to_string_lossy(), None::<&str>)
            .map_err(|error| format!("没能打开文件夹（{error}）"))
    })
    .await
}

/// How long turning the bridge on or off waits for it to take effect.
const BRIDGE_SWITCH_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[tauri::command]
pub async fn settings_set_bridge<R: Runtime>(
    app: AppHandle<R>,
    shell_env: State<'_, ShellEnvState>,
    enabled: bool,
) -> Result<SettingsInfo, String> {
    let shell_env = shell_env.inner().clone();
    call("settings_set_bridge", move || {
        app_config::update(
            &config_file(&app)?,
            &Changes {
                bridge_enabled: Some(enabled),
                ..Changes::default()
            },
        )?;
        log::info!(
            "iphone bridge switched {} in the settings",
            if enabled { "on" } else { "off" }
        );
        if let Some(bridge) = app.try_state::<BridgeService>() {
            bridge.reconfigure();
            let deadline = Instant::now() + BRIDGE_SWITCH_WAIT;
            while bridge.status().enabled != enabled && Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        Ok(info(&app, &shell_env.get().env))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{git as run_git, init_repo, TempDir};

    #[test]
    fn a_repository_root_with_a_remote_can_be_the_knowledge_base() {
        let kb = TempDir::new("settings-kb");
        init_repo(kb.path());
        kb.write("AGENTS.md", "# 规则\n");
        run_git(kb.path(), &["add", "--", "AGENTS.md"]);
        run_git(kb.path(), &["commit", "-q", "-m", "init"]);
        run_git(
            kb.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://someone:secret-token@github.com/someone/kb.git",
            ],
        );
        let env = ChildEnv::inherit();
        let check = check_folder(kb.path(), &env);
        assert!(check.ok, "{check:?}");
        assert!(check.has_rules);
        let remote = check.remote.clone().unwrap();
        assert_eq!(remote.name, "origin");
        assert_eq!(remote.target.host.as_deref(), Some("github.com"));
        assert!(!remote.tracked);
        assert!(!serde_json::to_string(&check).unwrap().contains("secret"));

        // A folder inside the repository is not its root.
        let inner = kb.path().join("wiki");
        std::fs::create_dir_all(&inner).unwrap();
        let check = check_folder(&inner, &env);
        assert!(!check.ok);
        assert!(check.problem.unwrap().contains("根目录"));
    }

    #[test]
    fn other_folders_are_refused_in_plain_words() {
        let dir = TempDir::new("settings-not-kb");
        let env = ChildEnv::inherit();
        let plain = check_folder(dir.path(), &env);
        assert!(!plain.ok);
        assert!(plain.problem.unwrap().contains("不是 git 仓库"));
        assert!(check_folder(&dir.path().join("missing"), &env)
            .problem
            .unwrap()
            .contains("找不到"));
        let file = dir.write("a.md", "x");
        assert!(check_folder(&file, &env)
            .problem
            .unwrap()
            .contains("不是一个文件夹"));
        assert!(check_folder(Path::new("relative/kb"), &env)
            .problem
            .unwrap()
            .contains("完整"));
        // Without any remote there is nothing to show.
        let repo = TempDir::new("settings-no-remote");
        init_repo(repo.path());
        let check = check_folder(repo.path(), &env);
        assert!(check.ok);
        assert_eq!(check.remote, None);
        assert!(!check.has_rules);
    }
}
