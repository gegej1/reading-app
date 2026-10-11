//! Committing knowledge-base changes with the `git` command-line tool.
//!
//! Every call runs `git` directly with explicit arguments (never through a
//! shell), inside the knowledge-base root, with literal pathspecs and without
//! terminal prompts. Nothing is ever pushed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

/// Result of writing one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOutcome {
    /// `false` when the content was unchanged, so there was nothing to commit.
    pub committed: bool,
    /// Hash of the new commit, if one was made.
    pub commit: Option<String>,
}

/// A `git` invocation rooted at `root`.
fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        // Paths are file names, never pathspec magic like `:(glob)`.
        .env("GIT_LITERAL_PATHSPECS", "1")
        // Fail instead of waiting for credentials on a terminal.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    command
}

fn spawn_error(error: std::io::Error) -> String {
    format!("无法运行 git（{error}）。请确认已安装 Git（macOS 上可运行 xcode-select --install）")
}

/// Runs git with `args` and returns trimmed stdout, or a readable error.
fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git(root).args(args).output().map_err(spawn_error)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    };
    Err(format!(
        "git {} 失败：{detail}",
        args.first().unwrap_or(&"")
    ))
}

/// Requires `root` (canonical) to be the top level of a git work tree.
///
/// Being merely *inside* some other repository is not enough: commits would
/// then land in that enclosing repository.
pub fn ensure_repo(root: &Path) -> Result<(), String> {
    let toplevel = run(root, &["rev-parse", "--show-toplevel"])
        .map_err(|_| format!("知识库文件夹不是 git 仓库：{}", root.display()))?;
    let toplevel = PathBuf::from(toplevel);
    let toplevel = fs::canonicalize(&toplevel).unwrap_or(toplevel);
    if toplevel == root {
        Ok(())
    } else {
        Err(format!(
            "知识库文件夹不是 git 仓库的根目录：{}（所在仓库：{}）",
            root.display(),
            toplevel.display()
        ))
    }
}

/// A commit message must have visible text and no NUL bytes.
pub fn validate_message(message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("提交说明不能为空".to_string());
    }
    if message.contains('\0') {
        return Err("提交说明里不能有空字符".to_string());
    }
    Ok(())
}

/// Stages `rel` and commits only that path with `message`.
///
/// `git commit -- <path>` commits just that path, so anything else the user
/// has staged stays staged and uncommitted. If the file is unchanged, no
/// commit is made and `committed` is `false`.
pub fn commit_path(root: &Path, rel: &str, message: &str) -> Result<WriteOutcome, String> {
    validate_message(message)?;
    run(root, &["add", "--", rel])?;

    let status = git(root)
        .args(["diff", "--cached", "--quiet", "--no-ext-diff", "--", rel])
        .status()
        .map_err(spawn_error)?;
    match status.code() {
        Some(0) => {
            return Ok(WriteOutcome {
                committed: false,
                commit: None,
            })
        }
        Some(1) => {}
        _ => return Err(format!("git diff 失败（{status}）")),
    }

    run(root, &["commit", "--quiet", "-m", message, "--", rel])?;
    let head = run(root, &["rev-parse", "HEAD"])?;
    Ok(WriteOutcome {
        committed: true,
        commit: Some(head),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{commit_count, git as git_out, init_repo, TempDir};

    fn repo(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        init_repo(dir.path());
        dir
    }

    #[test]
    fn ensure_repo_accepts_a_repo_root() {
        let kb = repo("git-ensure");
        assert_eq!(ensure_repo(kb.path()), Ok(()));
    }

    #[test]
    fn ensure_repo_rejects_plain_folders_and_subfolders() {
        let plain = TempDir::new("git-plain");
        let err = ensure_repo(plain.path()).unwrap_err();
        assert!(err.contains("不是 git 仓库"), "{err}");

        let kb = repo("git-subfolder");
        let sub = kb.mkdir("wiki");
        let err = ensure_repo(&sub).unwrap_err();
        assert!(err.contains("根目录"), "{err}");
    }

    #[test]
    fn commits_only_the_given_path() {
        let kb = repo("git-commit");
        kb.write("self/底色.md", "### D01 标题\n");
        kb.write("other.md", "other v1\n");
        git_out(kb.path(), &["add", "--", "self/底色.md", "other.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        // The user has an unrelated change staged.
        kb.write("other.md", "other v2\n");
        git_out(kb.path(), &["add", "--", "other.md"]);

        kb.write("self/底色.md", "### D01 标题 ✅\n");
        let outcome = commit_path(kb.path(), "self/底色.md", "底色：确认 D01").unwrap();

        assert!(outcome.committed);
        let head = git_out(kb.path(), &["rev-parse", "HEAD"]);
        assert_eq!(outcome.commit.as_deref(), Some(head.as_str()));
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(
            git_out(kb.path(), &["log", "-1", "--format=%s"]),
            "底色：确认 D01"
        );
        assert_eq!(
            git_out(
                kb.path(),
                &[
                    "-c",
                    "core.quotepath=off",
                    "show",
                    "--name-only",
                    "--format=",
                    "HEAD"
                ]
            ),
            "self/底色.md"
        );
        // The unrelated staged change is still staged, not committed.
        assert_eq!(
            git_out(kb.path(), &["diff", "--cached", "--name-only"]),
            "other.md"
        );
    }

    #[test]
    fn unchanged_content_makes_no_commit() {
        let kb = repo("git-unchanged");
        kb.write("a.md", "same\n");
        git_out(kb.path(), &["add", "--", "a.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        let outcome = commit_path(kb.path(), "a.md", "不会提交").unwrap();
        assert_eq!(
            outcome,
            WriteOutcome {
                committed: false,
                commit: None
            }
        );
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn new_files_in_new_folders_are_added_and_committed() {
        let kb = repo("git-new-file");
        kb.write("index.md", "# 目录\n");
        git_out(kb.path(), &["add", "--", "index.md"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);

        kb.write("inbox/裁决/2026-10-12-回答.md", "回答\n");
        let outcome = commit_path(
            kb.path(),
            "inbox/裁决/2026-10-12-回答.md",
            "收件箱：新增回答",
        )
        .unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "");
    }

    #[test]
    fn works_in_a_repo_without_commits() {
        let kb = repo("git-empty");
        kb.write("first.md", "first\n");
        let outcome = commit_path(kb.path(), "first.md", "第一次提交").unwrap();
        assert!(outcome.committed);
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn pathspec_magic_is_taken_literally() {
        let kb = repo("git-literal");
        kb.write("keep.md", "keep\n");
        kb.write(":(glob)x.md", "odd name\n");
        let outcome = commit_path(kb.path(), ":(glob)x.md", "奇怪的文件名").unwrap();
        assert!(outcome.committed);
        assert_eq!(
            git_out(kb.path(), &["show", "--name-only", "--format=", "HEAD"]),
            ":(glob)x.md"
        );
        // keep.md was not swept into the commit.
        assert_eq!(git_out(kb.path(), &["status", "--porcelain"]), "?? keep.md");
    }

    #[test]
    fn rejects_empty_messages_and_reports_git_errors() {
        let kb = repo("git-errors");
        kb.write("a.md", "a\n");
        assert_eq!(
            commit_path(kb.path(), "a.md", "  \n"),
            Err("提交说明不能为空".to_string())
        );
        assert!(commit_path(kb.path(), "a.md", "a\0b").is_err());
        assert_eq!(commit_count(kb.path()), 0);

        let err = commit_path(kb.path(), "missing.md", "缺文件").unwrap_err();
        assert!(err.starts_with("git add 失败"), "{err}");
    }

    #[test]
    fn outcome_serializes_as_camel_case_json() {
        let outcome = WriteOutcome {
            committed: true,
            commit: Some("abc".into()),
        };
        assert_eq!(
            serde_json::to_string(&outcome).unwrap(),
            r#"{"committed":true,"commit":"abc"}"#
        );
    }
}
