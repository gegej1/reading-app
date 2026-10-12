//! Finding an earlier import of the same text in the knowledge base.
//!
//! The import record (`super::ledger`) says which phone files were imported.
//! When it cannot say for sure (an entry still pending after the app
//! stopped, a record that was broken and started anew, a file imported by
//! an older version), the knowledge base itself is asked: is there a
//! capture or answer from the iPhone with the same text at one of the paths
//! this file would have become? Its git history is searched first (the file
//! may have been ingested and moved since), then the working tree (the app
//! may have stopped between creating the file and committing it).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::import::SOURCE;
use crate::kb::capture::{attempt_path, MAX_ATTEMPTS};
use crate::kb::git;
use crate::shell_env::ChildEnv;

/// An earlier import of the same text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Where it is (or was) in the knowledge base.
    pub path: String,
    /// False when the file is there but was never committed.
    pub committed: bool,
}

/// Looks for `text` from the iPhone at `bases` (capture or answer paths,
/// each with the numbered names a collision within one second gives).
pub fn find(
    root: &Path,
    env: &ChildEnv,
    bases: &[String],
    text: &str,
) -> Result<Option<Found>, String> {
    let paths: BTreeSet<String> = bases
        .iter()
        .flat_map(|base| (1..=MAX_ATTEMPTS).map(move |attempt| attempt_path(base, attempt)))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let body = normalize(text);

    // Added in a commit with this text.
    let mut log = git::plumbing(root, env);
    log.args([
        "log",
        "--format=%x01%H",
        "--name-only",
        "--diff-filter=A",
        "--no-renames",
        "--",
    ])
    .args(&paths);
    let listed = git::finish(log, git::LOCAL_TIMEOUT)?;
    if !listed.success() {
        return Err(format!("git log 失败：{}", listed.detail()));
    }
    let mut commit = "";
    for line in listed.stdout.lines() {
        if let Some(hash) = line.strip_prefix('\u{1}') {
            commit = hash.trim();
            continue;
        }
        let path = line.trim_end();
        if path.is_empty() || commit.is_empty() || !paths.contains(path) {
            continue;
        }
        let mut show = git::plumbing(root, env);
        show.args(["cat-file", "blob", &format!("{commit}:{path}")]);
        let shown = git::finish(show, git::LOCAL_TIMEOUT)?;
        if shown.success() && matches(&shown.stdout, &body) {
            return Ok(Some(Found {
                path: path.to_string(),
                committed: true,
            }));
        }
    }

    // In the working tree with this text.
    for path in &paths {
        let Ok(content) = fs::read_to_string(root.join(path)) else {
            continue;
        };
        if !matches(&content, &body) {
            continue;
        }
        let mut status = git::plumbing(root, env);
        status.args(["status", "--porcelain", "--", path]);
        let status = git::finish(status, git::LOCAL_TIMEOUT)?;
        if !status.success() {
            return Err(format!("git status 失败：{}", status.detail()));
        }
        return Ok(Some(Found {
            path: path.clone(),
            committed: status.stdout.trim().is_empty(),
        }));
    }
    Ok(None)
}

/// The text the way it is compared: LF line ends, no blank lines around it.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim_matches('\n').to_string()
}

/// True for a capture or answer from the iPhone whose text is `body`, or
/// ends with it after a blank line (an answer kept as a capture starts with
/// a line saying what it was meant for).
fn matches(content: &str, body: &str) -> bool {
    let content = content.replace("\r\n", "\n");
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);
    let Some(rest) = content.strip_prefix("---\n") else {
        return false;
    };
    let Some((front, after)) = rest.split_once("\n---\n") else {
        return false;
    };
    let source = format!("来源: {SOURCE}");
    if !front.lines().any(|line| line.trim() == source) {
        return false;
    }
    let after = after.trim_matches('\n');
    after == body || after.ends_with(&format!("\n\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::capture::{build_capture, save_capture, LocalTime};
    use crate::kb::test_support::{git as git_out, init_repo, TempDir};

    fn at() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 10,
            day: 12,
            hour: 14,
            minute: 3,
            second: 22,
            offset_minutes: 480,
        }
    }

    fn kb(tag: &str) -> TempDir {
        let kb = TempDir::new(tag);
        init_repo(kb.path());
        kb.write("index.md", "# 目录\n");
        git_out(kb.path(), &["add", "-A"]);
        git_out(kb.path(), &["commit", "-q", "-m", "init"]);
        kb
    }

    #[test]
    fn matches_the_text_of_iphone_captures_only() {
        let capture = build_capture("  第一行\r\n第二行\n", SOURCE, &at()).unwrap();
        assert!(matches(
            &capture.content,
            &normalize("  第一行\r\n第二行\n")
        ));
        assert!(!matches(&capture.content, &normalize("第二行")));
        let noted = build_capture(
            "（iPhone 上对 Q-002 的回答：它已经裁决了）\n\n好的",
            SOURCE,
            &at(),
        )
        .unwrap();
        assert!(matches(&noted.content, "好的"));
        let mac = build_capture("好的", "mac", &at()).unwrap();
        assert!(!matches(&mac.content, "好的"));
        assert!(!matches("好的", "好的"));
    }

    #[test]
    fn finds_committed_moved_and_uncommitted_imports() {
        let kb = kb("earlier");
        let env = ChildEnv::inherit();
        let capture = build_capture("记过的", SOURCE, &at()).unwrap();
        let base = vec![capture.path.clone()];
        assert_eq!(find(kb.path(), &env, &base, "记过的").unwrap(), None);

        // A second capture in the same second got a numbered name.
        kb.write(
            &capture.path,
            &build_capture("别的", SOURCE, &at()).unwrap().content,
        );
        git_out(kb.path(), &["add", "-A"]);
        git_out(kb.path(), &["commit", "-q", "-m", "other"]);
        let saved = save_capture(kb.path(), &env, &capture).unwrap();
        assert!(saved.path.ends_with("-2.md"));
        let found = find(kb.path(), &env, &base, "记过的\n").unwrap().unwrap();
        assert_eq!(
            found,
            Found {
                path: saved.path.clone(),
                committed: true
            }
        );

        // Ingested since: gone from the working tree, still in the history.
        git_out(kb.path(), &["rm", "-q", "--", &saved.path]);
        git_out(kb.path(), &["commit", "-q", "-m", "ingest"]);
        assert!(
            find(kb.path(), &env, &base, "记过的")
                .unwrap()
                .unwrap()
                .committed
        );
        assert_eq!(find(kb.path(), &env, &base, "没记过").unwrap(), None);

        // Created but never committed (the app stopped in between).
        let other = build_capture("没提交", SOURCE, &LocalTime { second: 59, ..at() }).unwrap();
        kb.write(&other.path, &other.content);
        let found = find(kb.path(), &env, std::slice::from_ref(&other.path), "没提交")
            .unwrap()
            .unwrap();
        assert_eq!(
            found,
            Found {
                path: other.path,
                committed: false
            }
        );
    }
}
