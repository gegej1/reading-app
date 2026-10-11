//! The two ways the app changes knowledge-base files: replacing one file and
//! creating a new one. Each makes exactly one commit that contains exactly
//! that file. The caller holds the knowledge-base lock (`super::lock`).

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use super::files;
use super::git::{self, WriteOutcome};
use super::paths::{self, PathErrorKind};
use crate::shell_env::ChildEnv;

/// Replaces (or creates) `rel` with `content` and commits it with `message`.
/// `root` must be canonical.
pub fn write_file(
    root: &Path,
    env: &ChildEnv,
    rel: &str,
    content: &str,
    message: &str,
) -> Result<WriteOutcome, String> {
    git::validate_message(message)?;
    // Check git first so a non-repo knowledge base is never half-updated.
    git::ensure_repo(root, env)?;
    let target = paths::resolve_write(root, rel).map_err(|e| e.to_string())?;
    files::write_atomic(&target, content).map_err(|e| format!("写入文件失败：{rel}（{e}）"))?;
    git::commit_path(root, rel, message, env)
        .map_err(|e| format!("文件已写入，但 git 提交失败：{e}"))
}

/// Why a file could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateError {
    /// Something already has this name; nothing was changed.
    Exists(String),
    /// Anything else; nothing was left behind.
    Failed(String),
}

/// How an "already exists" failure starts. The frontend recognises it by this
/// prefix (see `src/store/tauriStore.ts`) and then tries another name.
pub const EXISTS_PREFIX: &str = "文件已存在：";

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CreateError::Exists(rel) => write!(f, "{EXISTS_PREFIX}{rel}"),
            CreateError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CreateError {}

/// Creates the new file `rel` with `content` and commits it with `message`.
/// `root` must be canonical.
///
/// Never overwrites: when anything already has that name the result is
/// [`CreateError::Exists`]. It is all or nothing: if the commit fails, the
/// file that was just created is removed again, so a failed capture can
/// simply be repeated.
pub fn create_file(
    root: &Path,
    env: &ChildEnv,
    rel: &str,
    content: &str,
    message: &str,
) -> Result<WriteOutcome, CreateError> {
    git::validate_message(message).map_err(CreateError::Failed)?;
    git::ensure_repo(root, env).map_err(CreateError::Failed)?;
    let target = paths::resolve_write(root, rel).map_err(|error| match error.kind {
        // A symlink or a folder with this name: the name is taken.
        PathErrorKind::Symlink | PathErrorKind::NotAFile => CreateError::Exists(rel.to_string()),
        _ => CreateError::Failed(error.to_string()),
    })?;
    files::create_new(&target, content).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            CreateError::Exists(rel.to_string())
        } else {
            CreateError::Failed(format!("创建文件失败：{rel}（{error}）"))
        }
    })?;

    match git::commit_path(root, rel, message, env) {
        Ok(outcome) if outcome.committed => Ok(outcome),
        Ok(_) => Err(undo_create(
            root,
            env,
            rel,
            &target,
            "git 没有把新文件提交进去",
        )),
        Err(error) => Err(undo_create(root, env, rel, &target, &error)),
    }
}

/// Removes the file this call created (and takes it out of the index) after
/// its commit failed, and says what happened.
fn undo_create(root: &Path, env: &ChildEnv, rel: &str, target: &Path, reason: &str) -> CreateError {
    let _ = git::unstage_new_path(root, rel, env);
    match fs::remove_file(target) {
        Ok(()) => CreateError::Failed(format!("git 提交失败，没有留下文件：{reason}")),
        Err(error) => CreateError::Failed(format!(
            "git 提交失败（{reason}），新文件也没能撤回：{rel}（{error}）"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{commit_count, git as git_out, init_repo, symlink, TempDir};
    use std::os::unix::fs::PermissionsExt;

    fn kb(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        init_repo(dir.path());
        dir.write("self/底色.md", "### D01 示例\n");
        git_out(dir.path(), &["add", "--", "self/底色.md"]);
        git_out(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    fn env() -> ChildEnv {
        ChildEnv::inherit()
    }

    #[test]
    fn write_replaces_and_commits_one_file() {
        let kb = kb("ops-write");
        let outcome = write_file(
            kb.path(),
            &env(),
            "self/底色.md",
            "### D01 示例 ✅\n",
            "底色：确认 D01",
        )
        .unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 2);

        let same = write_file(
            kb.path(),
            &env(),
            "self/底色.md",
            "### D01 示例 ✅\n",
            "不会提交",
        )
        .unwrap();
        assert!(!same.committed);
        assert_eq!(commit_count(kb.path()), 2);
    }

    #[test]
    fn write_checks_the_repo_and_the_path_before_touching_anything() {
        let plain = TempDir::new("ops-write-plain");
        let error = write_file(plain.path(), &env(), "a.md", "x", "m").unwrap_err();
        assert!(error.contains("不是 git 仓库"), "{error}");
        assert!(!plain.path().join("a.md").exists());

        let kb = kb("ops-write-path");
        let error = write_file(kb.path(), &env(), ".git/config", "x", "m").unwrap_err();
        assert!(error.contains(".git"), "{error}");
        let error = write_file(kb.path(), &env(), "self/底色.md", "x", " ").unwrap_err();
        assert!(error.contains("提交说明"), "{error}");
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn create_adds_a_new_file_in_one_commit() {
        let kb = kb("ops-create");
        // Something unrelated is staged and something else is untracked.
        kb.write("self/底色.md", "### D01 示例（改了）\n");
        kb.write("草稿.md", "没有加入版本管理\n");

        let outcome = create_file(
            kb.path(),
            &env(),
            "inbox/2026-10-12T14-03-22-mac.md",
            "一条随手记\n",
            "收件箱：记一笔（mac）",
        )
        .unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(
            git_out(kb.path(), &["show", "--name-only", "--format=", "HEAD"]),
            "inbox/2026-10-12T14-03-22-mac.md"
        );
        // The rest of the working tree is exactly as it was.
        assert_eq!(
            git_out(
                kb.path(),
                &["-c", "core.quotepath=off", "status", "--porcelain"]
            ),
            "M self/底色.md\n?? 草稿.md"
        );
    }

    #[test]
    fn create_refuses_to_overwrite_anything() {
        let kb = kb("ops-create-exists");
        kb.write("inbox/note.md", "原来的\n");
        kb.mkdir("inbox/folder.md");
        symlink(
            &kb.path().join("inbox/note.md"),
            &kb.path().join("inbox/link.md"),
        );
        for rel in [
            "inbox/note.md",
            "inbox/folder.md",
            "inbox/link.md",
            "self/底色.md",
        ] {
            assert_eq!(
                create_file(kb.path(), &env(), rel, "新的\n", "收件箱：记一笔（mac）"),
                Err(CreateError::Exists(rel.to_string())),
                "{rel}"
            );
        }
        assert_eq!(
            fs::read_to_string(kb.path().join("inbox/note.md")).unwrap(),
            "原来的\n"
        );
        assert_eq!(
            fs::read_to_string(kb.path().join("self/底色.md")).unwrap(),
            "### D01 示例\n"
        );
        assert_eq!(commit_count(kb.path()), 1);
        assert_eq!(
            CreateError::Exists("inbox/note.md".into()).to_string(),
            "文件已存在：inbox/note.md"
        );
    }

    #[test]
    fn create_applies_the_path_rules() {
        let kb = kb("ops-create-paths");
        for rel in [
            "../逃逸.md",
            ".git/x.md",
            "inbox/.hidden.md",
            "inbox/a.txt",
            "/tmp/a.md",
        ] {
            let error = create_file(kb.path(), &env(), rel, "x\n", "m").unwrap_err();
            assert!(matches!(error, CreateError::Failed(_)), "{rel}: {error:?}");
        }
        assert_eq!(commit_count(kb.path()), 1);
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "");
    }

    #[test]
    fn create_leaves_nothing_behind_when_the_commit_fails() {
        let kb = kb("ops-create-rollback");
        // A hook that refuses every commit.
        let hook = kb.write(
            ".git/hooks/pre-commit",
            "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n",
        );
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let error = create_file(
            kb.path(),
            &env(),
            "inbox/2026-10-12T14-03-22-mac.md",
            "记不下来的一条\n",
            "收件箱：记一笔（mac）",
        )
        .unwrap_err();
        match error {
            CreateError::Failed(message) => {
                assert!(message.contains("没有留下文件"), "{message}");
                assert!(message.contains("hook says no"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!kb.path().join("inbox/2026-10-12T14-03-22-mac.md").exists());
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "");
        assert_eq!(commit_count(kb.path()), 1);

        // Once commits work again, the same capture goes through under its first name.
        fs::remove_file(&hook).unwrap();
        create_file(
            kb.path(),
            &env(),
            "inbox/2026-10-12T14-03-22-mac.md",
            "记不下来的一条\n",
            "收件箱：记一笔（mac）",
        )
        .unwrap();
        assert_eq!(commit_count(kb.path()), 2);
    }

    #[test]
    fn create_needs_a_git_repo() {
        let plain = TempDir::new("ops-create-plain");
        let error = create_file(plain.path(), &env(), "inbox/a.md", "x\n", "m").unwrap_err();
        assert!(
            matches!(&error, CreateError::Failed(m) if m.contains("不是 git 仓库")),
            "{error:?}"
        );
        assert!(!plain.path().join("inbox").exists());
    }
}
