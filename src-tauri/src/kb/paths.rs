//! Safety rules for knowledge-base paths received from the frontend.
//!
//! A request path is a repo-relative POSIX path such as `self/底色.md`. It
//! must not leave the knowledge base, touch `.git/` or other hidden entries,
//! or name anything other than a Markdown file. Symlinks are resolved and the
//! real location must satisfy the same rules.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Why a path was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathErrorKind {
    Empty,
    Nul,
    Backslash,
    Absolute,
    Tilde,
    /// An empty, `.` or `..` segment.
    BadSegment,
    Git,
    Hidden,
    NotMarkdown,
    NotFound,
    NotAFile,
    /// The write target itself is a symlink.
    Symlink,
    /// The real location is outside the knowledge base, or inside `.git/` or
    /// another hidden directory.
    Escape,
    /// The real location differs from the requested one (a symlinked
    /// directory, or different letter case on a case-insensitive disk).
    Mismatch,
    Io(String),
}

impl PathErrorKind {
    fn describe(&self) -> String {
        match self {
            PathErrorKind::Empty => "路径为空".into(),
            PathErrorKind::Nul => "路径里不能有空字符".into(),
            PathErrorKind::Backslash => "路径要用 / 分隔，不能有反斜杠".into(),
            PathErrorKind::Absolute => "只能用知识库内的相对路径，不能用绝对路径".into(),
            PathErrorKind::Tilde => "路径不能以 ~ 开头".into(),
            PathErrorKind::BadSegment => "路径里不能有空段、. 或 ..".into(),
            PathErrorKind::Git => "不能访问 .git 目录".into(),
            PathErrorKind::Hidden => "不能访问隐藏文件或隐藏目录".into(),
            PathErrorKind::NotMarkdown => "只能读写 .md 文件".into(),
            PathErrorKind::NotFound => "找不到文件".into(),
            PathErrorKind::NotAFile => "不是普通文件".into(),
            PathErrorKind::Symlink => "不能写入符号链接".into(),
            PathErrorKind::Escape => "路径经符号链接指向了知识库外部或隐藏目录".into(),
            PathErrorKind::Mismatch => {
                "路径和磁盘上的实际位置不一致（可能经过符号链接，或大小写不同）".into()
            }
            PathErrorKind::Io(message) => format!("访问文件失败（{message}）"),
        }
    }
}

/// A rejected path together with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError {
    pub kind: PathErrorKind,
    pub path: String,
}

impl PathError {
    fn new(kind: PathErrorKind, path: &str) -> Self {
        PathError {
            kind,
            path: path.to_string(),
        }
    }

    fn io(path: &str, error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            PathError::new(PathErrorKind::NotFound, path)
        } else {
            PathError::new(PathErrorKind::Io(error.to_string()), path)
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}：{}", self.kind.describe(), self.path)
    }
}

impl std::error::Error for PathError {}

/// Checks a request path without touching the file system.
///
/// Rejects: empty paths, NUL bytes, backslashes, absolute (or drive-letter)
/// paths, a leading `~`, empty/`.`/`..` segments, any `.git` or other hidden
/// (dot-prefixed) segment, and a final name that does not end in `.md`.
/// The same rules apply to reads and writes.
pub fn validate_rel_path(rel: &str) -> Result<(), PathError> {
    let err = |kind| Err(PathError::new(kind, rel));
    if rel.is_empty() {
        return err(PathErrorKind::Empty);
    }
    if rel.contains('\0') {
        return err(PathErrorKind::Nul);
    }
    if rel.contains('\\') {
        return err(PathErrorKind::Backslash);
    }
    if rel.starts_with('/') || has_drive_prefix(rel) {
        return err(PathErrorKind::Absolute);
    }
    if rel.starts_with('~') {
        return err(PathErrorKind::Tilde);
    }
    if let Some(kind) = segment_problem(rel.split('/')) {
        return err(kind);
    }
    if !is_markdown_name(rel.rsplit('/').next().unwrap_or_default()) {
        return err(PathErrorKind::NotMarkdown);
    }
    Ok(())
}

/// `true` for a file name like `底色.md` (a non-empty stem plus `.md`).
pub fn is_markdown_name(name: &str) -> bool {
    name.len() > ".md".len() && name.ends_with(".md")
}

/// Resolves a request path for reading.
///
/// `root` must be canonical. The path is validated, symlinks are resolved,
/// and the real location must be a Markdown file inside `root` that is not
/// under `.git/` or another hidden directory. Returns the canonical path.
pub fn resolve_read(root: &Path, rel: &str) -> Result<PathBuf, PathError> {
    validate_rel_path(rel)?;
    let canonical = fs::canonicalize(root.join(rel)).map_err(|e| PathError::io(rel, e))?;
    let inner = canonical
        .strip_prefix(root)
        .map_err(|_| PathError::new(PathErrorKind::Escape, rel))?;
    let inner = inner
        .to_str()
        .ok_or_else(|| PathError::new(PathErrorKind::Escape, rel))?;
    validate_rel_path(inner).map_err(|e| {
        let kind = match e.kind {
            PathErrorKind::NotMarkdown => PathErrorKind::NotMarkdown,
            _ => PathErrorKind::Escape,
        };
        PathError::new(kind, rel)
    })?;
    let metadata = fs::metadata(&canonical).map_err(|e| PathError::io(rel, e))?;
    if !metadata.is_file() {
        return Err(PathError::new(PathErrorKind::NotAFile, rel));
    }
    Ok(canonical)
}

/// Resolves a request path for writing and creates missing parent directories.
///
/// `root` must be canonical. The target may not exist yet, but if it does it
/// must be a regular file (not a symlink or directory). Every existing parent
/// directory must be a real directory exactly where the path says, so writes
/// can never be redirected by a symlink. Returns the path to write to.
pub fn resolve_write(root: &Path, rel: &str) -> Result<PathBuf, PathError> {
    validate_rel_path(rel)?;
    let rel_path = Path::new(rel);
    let rel_parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = rel_path
        .file_name()
        .ok_or_else(|| PathError::new(PathErrorKind::BadSegment, rel))?;

    match fs::symlink_metadata(root.join(rel)) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(PathError::new(PathErrorKind::Symlink, rel));
        }
        Ok(meta) if !meta.is_file() => {
            return Err(PathError::new(PathErrorKind::NotAFile, rel));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(PathError::io(rel, e)),
    }

    // Check the part of the parent chain that already exists before creating
    // anything, so a symlinked directory can never make us create directories
    // outside the knowledge base.
    let parent = root.join(rel_parent);
    let existing = nearest_existing(&parent);
    let existing_rel = existing.strip_prefix(root).unwrap_or(rel_parent);
    expect_location(root, &existing, existing_rel, rel)?;

    fs::create_dir_all(&parent).map_err(|e| PathError::io(rel, e))?;
    expect_location(root, &parent, rel_parent, rel)?;

    let canonical_parent = fs::canonicalize(&parent).map_err(|e| PathError::io(rel, e))?;
    let target = canonical_parent.join(file_name);
    // On a case-insensitive disk `Notes.md` would silently replace `notes.md`.
    if let Ok(existing_target) = fs::canonicalize(&target) {
        if existing_target != target {
            return Err(PathError::new(PathErrorKind::Mismatch, rel));
        }
    }
    Ok(target)
}

/// Requires the real location of `path` to be `root/expected_rel`.
fn expect_location(
    root: &Path,
    path: &Path,
    expected_rel: &Path,
    rel: &str,
) -> Result<(), PathError> {
    let canonical = fs::canonicalize(path).map_err(|e| PathError::io(rel, e))?;
    let inner = canonical
        .strip_prefix(root)
        .map_err(|_| PathError::new(PathErrorKind::Escape, rel))?;
    if inner == expected_rel {
        return Ok(());
    }
    let inner_has_problem = match inner.to_str() {
        Some("") => false,
        Some(s) => segment_problem(s.split('/')).is_some(),
        None => true,
    };
    let kind = if inner_has_problem {
        PathErrorKind::Escape
    } else {
        PathErrorKind::Mismatch
    };
    Err(PathError::new(kind, rel))
}

/// The closest ancestor of `path` (or `path` itself) that exists on disk.
/// A dangling symlink counts as existing, so it is checked rather than skipped.
fn nearest_existing(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if fs::symlink_metadata(current).is_ok() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return current.to_path_buf(),
        }
    }
}

/// First problem found in a sequence of path segments, if any.
fn segment_problem<'a>(segments: impl Iterator<Item = &'a str>) -> Option<PathErrorKind> {
    for segment in segments {
        match segment {
            "" | "." | ".." => return Some(PathErrorKind::BadSegment),
            ".git" => return Some(PathErrorKind::Git),
            s if s.starts_with('.') => return Some(PathErrorKind::Hidden),
            _ => {}
        }
    }
    None
}

/// `C:/...` style prefixes are absolute on Windows; reject them everywhere.
fn has_drive_prefix(rel: &str) -> bool {
    let bytes = rel.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{symlink, TempDir};

    fn kind(rel: &str) -> PathErrorKind {
        validate_rel_path(rel).unwrap_err().kind
    }

    #[test]
    fn accepts_plain_and_chinese_markdown_paths() {
        for rel in [
            "index.md",
            "self/底色.md",
            "self/判例/P-001-示例.md",
            "wiki/人物与公司/某人.md",
            "inbox/裁决/2026-10-12-回答.md",
            "raw/对话/2026-10-11-对话(整理稿).md",
            "a b/c d.md",
        ] {
            assert_eq!(validate_rel_path(rel), Ok(()), "{rel}");
        }
    }

    #[test]
    fn rejects_every_unsafe_shape() {
        assert_eq!(kind(""), PathErrorKind::Empty);
        assert_eq!(kind("self/a\0.md"), PathErrorKind::Nul);
        assert_eq!(kind("self\\a.md"), PathErrorKind::Backslash);
        assert_eq!(kind("/etc/passwd.md"), PathErrorKind::Absolute);
        assert_eq!(kind("C:/kb/a.md"), PathErrorKind::Absolute);
        assert_eq!(kind("~/kb/a.md"), PathErrorKind::Tilde);
        assert_eq!(kind("~a.md"), PathErrorKind::Tilde);
        assert_eq!(kind("../a.md"), PathErrorKind::BadSegment);
        assert_eq!(kind("self/../../a.md"), PathErrorKind::BadSegment);
        assert_eq!(kind("self/./a.md"), PathErrorKind::BadSegment);
        assert_eq!(kind("self//a.md"), PathErrorKind::BadSegment);
        assert_eq!(kind("self/"), PathErrorKind::BadSegment);
        assert_eq!(kind(".git/config"), PathErrorKind::Git);
        assert_eq!(kind(".git/a.md"), PathErrorKind::Git);
        assert_eq!(kind("sub/.git/a.md"), PathErrorKind::Git);
        assert_eq!(kind(".obsidian/a.md"), PathErrorKind::Hidden);
        assert_eq!(kind("self/.hidden.md"), PathErrorKind::Hidden);
        assert_eq!(kind(".GIT/a.md"), PathErrorKind::Hidden);
        assert_eq!(kind("self/底色.txt"), PathErrorKind::NotMarkdown);
        assert_eq!(kind("self/底色.MD"), PathErrorKind::NotMarkdown);
        assert_eq!(kind("self/底色"), PathErrorKind::NotMarkdown);
        assert_eq!(kind("self/底色.md.bak"), PathErrorKind::NotMarkdown);
    }

    #[test]
    fn error_messages_are_chinese_and_name_the_path() {
        let message = validate_rel_path("../a.md").unwrap_err().to_string();
        assert!(message.contains(".."), "{message}");
        assert!(message.ends_with("../a.md"), "{message}");
    }

    #[test]
    fn read_resolves_a_regular_file() {
        let kb = TempDir::new("read-ok");
        let file = kb.write("self/底色.md", "# 底色\n");
        assert_eq!(resolve_read(kb.path(), "self/底色.md"), Ok(file));
    }

    #[test]
    fn read_reports_missing_files_and_directories() {
        let kb = TempDir::new("read-missing");
        kb.mkdir("folder.md");
        assert_eq!(
            resolve_read(kb.path(), "nope.md").unwrap_err().kind,
            PathErrorKind::NotFound
        );
        assert_eq!(
            resolve_read(kb.path(), "folder.md").unwrap_err().kind,
            PathErrorKind::NotAFile
        );
    }

    #[test]
    fn read_follows_symlinks_that_stay_inside() {
        let kb = TempDir::new("read-inner-link");
        let target = kb.write("wiki/概念/甲.md", "甲\n");
        symlink(&target, &kb.path().join("alias.md"));
        assert_eq!(resolve_read(kb.path(), "alias.md"), Ok(target));
    }

    #[test]
    fn read_rejects_symlink_escapes() {
        let kb = TempDir::new("read-escape");
        let outside = TempDir::new("read-escape-outside");
        let secret = outside.write("secret.md", "secret\n");

        // A file symlink pointing outside the knowledge base.
        symlink(&secret, &kb.path().join("leak.md"));
        assert_eq!(
            resolve_read(kb.path(), "leak.md").unwrap_err().kind,
            PathErrorKind::Escape
        );

        // A directory symlink pointing outside the knowledge base.
        symlink(outside.path(), &kb.path().join("linked"));
        assert_eq!(
            resolve_read(kb.path(), "linked/secret.md")
                .unwrap_err()
                .kind,
            PathErrorKind::Escape
        );

        // A symlink into .git and into another hidden directory.
        let git_file = kb.write(".git/notes.md", "inside git\n");
        symlink(&git_file, &kb.path().join("git-notes.md"));
        assert_eq!(
            resolve_read(kb.path(), "git-notes.md").unwrap_err().kind,
            PathErrorKind::Escape
        );
        let hidden_file = kb.write(".obsidian/notes.md", "hidden\n");
        symlink(&hidden_file, &kb.path().join("hidden-notes.md"));
        assert_eq!(
            resolve_read(kb.path(), "hidden-notes.md").unwrap_err().kind,
            PathErrorKind::Escape
        );

        // A symlink to a non-Markdown file inside the knowledge base.
        let text = kb.write("notes.txt", "plain\n");
        symlink(&text, &kb.path().join("text.md"));
        assert_eq!(
            resolve_read(kb.path(), "text.md").unwrap_err().kind,
            PathErrorKind::NotMarkdown
        );

        // A dangling symlink.
        symlink(&kb.path().join("gone.md"), &kb.path().join("dangling.md"));
        assert_eq!(
            resolve_read(kb.path(), "dangling.md").unwrap_err().kind,
            PathErrorKind::NotFound
        );
    }

    #[test]
    fn write_targets_existing_and_new_files() {
        let kb = TempDir::new("write-ok");
        let existing = kb.write("self/底色.md", "old\n");
        assert_eq!(resolve_write(kb.path(), "self/底色.md"), Ok(existing));
        assert_eq!(
            resolve_write(kb.path(), "index.md"),
            Ok(kb.path().join("index.md"))
        );
    }

    #[test]
    fn write_creates_missing_parent_directories() {
        let kb = TempDir::new("write-nested");
        let target = resolve_write(kb.path(), "inbox/裁决/新的/回答.md").unwrap();
        assert_eq!(target, kb.path().join("inbox/裁决/新的/回答.md"));
        assert!(kb.path().join("inbox/裁决/新的").is_dir());
        assert!(
            !target.exists(),
            "resolving must not create the file itself"
        );
    }

    #[test]
    fn write_rejects_symlinks_and_directories() {
        let kb = TempDir::new("write-symlink");
        let real = kb.write("real.md", "real\n");
        symlink(&real, &kb.path().join("alias.md"));
        assert_eq!(
            resolve_write(kb.path(), "alias.md").unwrap_err().kind,
            PathErrorKind::Symlink
        );

        kb.mkdir("folder.md");
        assert_eq!(
            resolve_write(kb.path(), "folder.md").unwrap_err().kind,
            PathErrorKind::NotAFile
        );
    }

    #[test]
    fn write_never_follows_a_directory_symlink_outside() {
        let kb = TempDir::new("write-escape");
        let outside = TempDir::new("write-escape-outside");
        symlink(outside.path(), &kb.path().join("linked"));

        for rel in ["linked/a.md", "linked/new/deeper/a.md"] {
            assert_eq!(
                resolve_write(kb.path(), rel).unwrap_err().kind,
                PathErrorKind::Escape,
                "{rel}"
            );
        }
        assert!(
            !outside.path().join("new").exists(),
            "no directories may be created outside the knowledge base"
        );
    }

    #[test]
    fn write_rejects_directory_symlinks_into_git_or_elsewhere_inside() {
        let kb = TempDir::new("write-inner-dir-link");
        kb.mkdir(".git/objects");
        symlink(&kb.path().join(".git"), &kb.path().join("gitdir"));
        assert_eq!(
            resolve_write(kb.path(), "gitdir/a.md").unwrap_err().kind,
            PathErrorKind::Escape
        );

        kb.mkdir("wiki");
        symlink(&kb.path().join("wiki"), &kb.path().join("alias"));
        assert_eq!(
            resolve_write(kb.path(), "alias/a.md").unwrap_err().kind,
            PathErrorKind::Mismatch
        );
    }

    #[test]
    fn write_rejects_case_mismatches_on_case_insensitive_disks() {
        let kb = TempDir::new("write-case");
        kb.write("inbox/note.md", "note\n");
        let case_insensitive = kb.path().join("INBOX").exists();
        if !case_insensitive {
            return; // Nothing to check on a case-sensitive file system.
        }
        assert_eq!(
            resolve_write(kb.path(), "Inbox/new.md").unwrap_err().kind,
            PathErrorKind::Mismatch
        );
        assert_eq!(
            resolve_write(kb.path(), "inbox/NOTE.md").unwrap_err().kind,
            PathErrorKind::Mismatch
        );
    }

    #[test]
    fn write_validates_before_touching_the_disk() {
        let kb = TempDir::new("write-validate");
        assert_eq!(
            resolve_write(kb.path(), "new/../a.md").unwrap_err().kind,
            PathErrorKind::BadSegment
        );
        assert_eq!(
            resolve_write(kb.path(), "new/a.txt").unwrap_err().kind,
            PathErrorKind::NotMarkdown
        );
        assert!(!kb.path().join("new").exists());
    }
}
