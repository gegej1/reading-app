//! Listing Markdown files, replacing them atomically and creating new ones
//! without ever overwriting an existing file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::paths;

/// One Markdown file of the knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbFile {
    /// Repo-relative POSIX path, e.g. `self/底色.md`.
    pub path: String,
    /// Last modification time in milliseconds since the Unix epoch.
    pub mtime: Option<u64>,
}

/// Directory names skipped while listing, in addition to hidden entries.
const SKIPPED_DIRS: &[&str] = &["node_modules"];

/// Lists every `*.md` file under `root` (which must be canonical).
///
/// Hidden entries (`.git/`, `.obsidian/`, `.trash/`, `.venv/`, …) and
/// `node_modules/` are skipped. Symlinked directories are never followed; a
/// symlinked Markdown file is listed only if it resolves to a readable
/// Markdown file inside the knowledge base. Names that are not valid UTF-8 are
/// skipped, and unreadable subdirectories are ignored. Sorted by path.
pub fn list_markdown(root: &Path) -> io::Result<Vec<KbFile>> {
    let mut files = Vec::new();
    // Read the root eagerly so a missing or unreadable root is an error.
    let mut pending = vec![(String::new(), fs::read_dir(root)?)];

    while let Some((prefix, entries)) = pending.pop() {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name) {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                if let Ok(children) = fs::read_dir(entry.path()) {
                    pending.push((rel, children));
                }
            } else if file_type.is_file() {
                if paths::is_markdown_name(name) {
                    let mtime = entry.metadata().ok().as_ref().and_then(mtime_ms);
                    files.push(KbFile { path: rel, mtime });
                }
            } else if file_type.is_symlink() && paths::is_markdown_name(name) {
                if let Ok(target) = paths::resolve_read(root, &rel) {
                    let mtime = fs::metadata(target).ok().as_ref().and_then(mtime_ms);
                    files.push(KbFile { path: rel, mtime });
                }
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn mtime_ms(metadata: &fs::Metadata) -> Option<u64> {
    let modified = metadata.modified().ok()?;
    let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}

/// Replaces (or creates) `path` with `content` atomically.
///
/// The content goes to a hidden temp file in the same directory, is flushed
/// to disk, takes over the permissions of the file it replaces, and is then
/// renamed over the target. Readers therefore see either the old or the new
/// file, never a half-written one. The temp file is removed on failure.
pub fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let temp = dir.join(temp_name(&name.to_string_lossy()));

    let result = write_and_rename(&temp, path, content);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;

    // Persist the rename itself; best effort, not every platform supports it.
    if let Ok(dir_handle) = File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

fn write_and_rename(temp: &Path, target: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(content.as_bytes())?;
    if let Ok(existing) = fs::metadata(target) {
        file.set_permissions(existing.permissions())?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(temp, target)
}

/// Creates `path` with `content`; fails with `AlreadyExists` when anything is
/// already there, and never overwrites.
///
/// The content goes to a hidden temp file in the same directory first and is
/// then published under the final name with a hard link, which the file
/// system refuses when the name is taken (also for a name that differs only
/// in letter case on a case-insensitive disk). Readers therefore see either
/// no file or the complete one. On file systems without hard links the file
/// is created directly with `O_EXCL` instead.
pub fn create_new(path: &Path, content: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let temp = dir.join(temp_name(&name.to_string_lossy()));

    let result = write_temp(&temp, content).and_then(|()| match fs::hard_link(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        // No hard links here (exFAT, some network shares): still exclusive,
        // just not atomic.
        Err(_) => write_exclusive(path, content),
    });
    let _ = fs::remove_file(&temp);
    result?;

    if let Ok(dir_handle) = File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

fn write_temp(temp: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

fn write_exclusive(path: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all());
    if written.is_err() {
        // The file is ours (created just now) and incomplete.
        let _ = fs::remove_file(path);
    }
    written
}

/// A unique hidden temp file name next to the target, e.g.
/// `.底色.md.dise-tmp-1234-1700000000000000000-0`.
fn temp_name(file_name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".{file_name}.dise-tmp-{}-{nanos}-{n}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{symlink, TempDir};
    use std::os::unix::fs::PermissionsExt;

    fn listed(root: &Path) -> Vec<String> {
        list_markdown(root)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect()
    }

    #[test]
    fn lists_markdown_files_sorted_with_posix_paths() {
        let kb = TempDir::new("list-basic");
        kb.write("index.md", "");
        kb.write("self/底色.md", "");
        kb.write("self/判例/P-001-示例.md", "");
        kb.write("wiki/概念/甲.md", "");
        kb.write("wiki/概念/notes.txt", "");
        kb.write("inbox/.gitkeep", "");
        kb.write("raw/对话/2026-10-11-对话(整理稿).md", "");
        kb.write("audio.mp3", "");

        let mut expected = vec![
            "index.md",
            "raw/对话/2026-10-11-对话(整理稿).md",
            "self/判例/P-001-示例.md",
            "self/底色.md",
            "wiki/概念/甲.md",
        ];
        expected.sort();
        assert_eq!(listed(kb.path()), expected);
    }

    #[test]
    fn skips_hidden_entries_and_node_modules() {
        let kb = TempDir::new("list-skip");
        kb.write("keep.md", "");
        kb.write(".git/HEAD.md", "");
        kb.write(".obsidian/workspace.md", "");
        kb.write(".trash/old.md", "");
        kb.write("tools/.venv/lib/LICENSE.md", "");
        kb.write("tools/node_modules/pkg/README.md", "");
        kb.write("node_modules/pkg/README.md", "");
        kb.write("self/.draft.md", "");
        assert_eq!(listed(kb.path()), vec!["keep.md"]);
    }

    #[test]
    fn does_not_follow_directory_symlinks() {
        let kb = TempDir::new("list-dir-link");
        let outside = TempDir::new("list-dir-link-outside");
        outside.write("secret.md", "");
        kb.write("wiki/a.md", "");
        symlink(outside.path(), &kb.path().join("outside"));
        symlink(&kb.path().join("wiki"), &kb.path().join("wiki-alias"));
        assert_eq!(listed(kb.path()), vec!["wiki/a.md"]);
    }

    #[test]
    fn lists_file_symlinks_only_when_they_stay_inside() {
        let kb = TempDir::new("list-file-link");
        let outside = TempDir::new("list-file-link-outside");
        let secret = outside.write("secret.md", "");
        let inner = kb.write("wiki/a.md", "");
        let git_file = kb.write(".git/x.md", "");
        symlink(&inner, &kb.path().join("alias.md"));
        symlink(&secret, &kb.path().join("leak.md"));
        symlink(&git_file, &kb.path().join("git.md"));
        symlink(
            &kb.path().join("missing.md"),
            &kb.path().join("dangling.md"),
        );
        assert_eq!(listed(kb.path()), vec!["alias.md", "wiki/a.md"]);
    }

    #[test]
    fn reports_modification_times_in_milliseconds() {
        let kb = TempDir::new("list-mtime");
        kb.write("a.md", "");
        let files = list_markdown(kb.path()).unwrap();
        let mtime = files[0].mtime.expect("mtime");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(
            mtime <= now + 1_000 && mtime + 600_000 > now,
            "{mtime} vs {now}"
        );
    }

    #[test]
    fn missing_root_is_an_error() {
        let kb = TempDir::new("list-missing");
        assert!(list_markdown(&kb.path().join("nope")).is_err());
    }

    #[test]
    fn serializes_as_camel_case_json() {
        let file = KbFile {
            path: "self/底色.md".into(),
            mtime: Some(5),
        };
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            r#"{"path":"self/底色.md","mtime":5}"#
        );
    }

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_temp_file() {
        let kb = TempDir::new("write-atomic");
        let path = kb.write("self/底色.md", "old content that is longer\n");
        write_atomic(&path, "### D01 新标题 ✅\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "### D01 新标题 ✅\n");

        let leftovers: Vec<_> = fs::read_dir(kb.path().join("self"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "底色.md")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn write_atomic_creates_new_files_and_keeps_permissions() {
        let kb = TempDir::new("write-atomic-perms");
        let fresh = kb.path().join("new.md");
        write_atomic(&fresh, "fresh\n").unwrap();
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "fresh\n");

        let path = kb.write("shared.md", "a\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, "b\n").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn create_new_writes_the_file_and_leaves_no_temp_file() {
        let kb = TempDir::new("create-new");
        let dir = kb.mkdir("inbox");
        let path = dir.join("2026-10-12T14-03-22-mac.md");
        create_new(&path, "---\n来源: mac\n---\n\n一条随手记\n").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "---\n来源: mac\n---\n\n一条随手记\n"
        );
        assert_eq!(names_in(&dir), vec!["2026-10-12T14-03-22-mac.md"]);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & 0o600, 0o600, "the owner can read and write");
    }

    #[test]
    fn create_new_never_overwrites() {
        let kb = TempDir::new("create-new-exists");
        let path = kb.write("inbox/note.md", "已有的内容\n");
        let error = create_new(&path, "新的内容\n").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "已有的内容\n");
        assert_eq!(names_in(&kb.path().join("inbox")), vec!["note.md"]);

        // Directories and symlinks (even dangling ones) count as taken too.
        let folder = kb.mkdir("inbox/folder.md");
        assert_eq!(
            create_new(&folder, "x").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let link = kb.path().join("inbox/link.md");
        symlink(&kb.path().join("inbox/gone.md"), &link);
        assert_eq!(
            create_new(&link, "x").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!kb.path().join("inbox/gone.md").exists());
    }

    #[test]
    fn create_new_treats_another_letter_case_as_taken_on_case_insensitive_disks() {
        let kb = TempDir::new("create-new-case");
        kb.write("inbox/note.md", "小写\n");
        if !kb.path().join("inbox/NOTE.md").exists() {
            return; // Case-sensitive file system: these are two different files.
        }
        let error = create_new(&kb.path().join("inbox/NOTE.md"), "大写\n").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(kb.path().join("inbox/note.md")).unwrap(),
            "小写\n"
        );
    }

    #[test]
    fn create_new_fails_cleanly_without_the_parent_folder() {
        let kb = TempDir::new("create-new-noparent");
        let error = create_new(&kb.path().join("missing/note.md"), "x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(names_in(kb.path()).is_empty());
    }

    #[test]
    fn write_atomic_cleans_up_when_the_rename_fails() {
        let kb = TempDir::new("write-atomic-fail");
        // Renaming a file over a non-empty directory fails.
        kb.write("blocked.md/child.md", "");
        let target = kb.path().join("blocked.md");
        assert!(write_atomic(&target, "x").is_err());
        let leftovers: Vec<_> = fs::read_dir(kb.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("dise-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
