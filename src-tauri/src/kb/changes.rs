//! What an AI run did to the knowledge base's working tree, and committing
//! or undoing it.
//!
//! A run starts from a [`Snapshot`]: the commit it began at, the queue files
//! with their contents, and every untracked file that already existed (those
//! belong to the user and survive everything). The engine only edits files;
//! afterwards
//!
//! - [`collect`] lists what changed against the snapshot's commit: tracked
//!   files of any kind and extension, plus untracked files created since,
//!   but not the untracked files that were there before and not the captures
//!   the app committed meanwhile;
//! - [`commit_run`] turns the approved part into exactly one commit, removing
//!   the processed queue files in the same commit;
//! - [`discard_run`] first copies every changed file into the run's folder,
//!   then restores tracked files and deletes only what the run created.
//!
//! The caller holds the knowledge-base lock (`super::lock`) for each call.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::git::{self, Finished};
use super::lock::NotedCommit;
use super::paths;
use super::sync;
use crate::shell_env::ChildEnv;

/// Files larger than this are not read for the check (it then says so).
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
/// Another git process (an editor, a terminal) may hold the index for a moment.
const INDEX_LOCK_ATTEMPTS: u32 = 6;
const INDEX_LOCK_PAUSE: Duration = Duration::from_millis(150);
/// Longest list of files named in one message.
const MAX_NAMED: usize = 5;

/// One file waiting in the queue (`inbox/` or `inbox/裁决/`) when the run started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueFile {
    /// Repo-relative path.
    pub path: String,
    pub content: String,
    /// Known to git (captures always are; a file dropped into the folder by hand may not be).
    pub tracked: bool,
}

/// The state a run started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// The commit HEAD pointed at.
    pub base: String,
    pub branch: String,
    pub queue: Vec<QueueFile>,
    /// Untracked files that already existed (queue files aside), with a
    /// fingerprint of their content. They are never committed, restored or deleted.
    pub untracked: BTreeMap<String, String>,
    /// Folders that already existed, so discarding a run only removes folders it made.
    pub dirs: BTreeSet<String>,
}

impl Snapshot {
    fn queue_file(&self, path: &str) -> Option<&QueueFile> {
        self.queue.iter().find(|file| file.path == path)
    }
}

/// What a file is on one side of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// It does not exist on this side.
    Absent,
    Bytes(Vec<u8>),
    Symlink,
    /// Larger than [`MAX_FILE_BYTES`]: not read.
    Large,
    /// A folder, a submodule or anything else that is not a file.
    Other,
}

impl Content {
    pub fn exists(&self) -> bool {
        !matches!(self, Content::Absent)
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Content::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The content as text, when it is a UTF-8 text file.
    pub fn text(&self) -> Option<&str> {
        self.bytes().and_then(|bytes| std::str::from_utf8(bytes).ok())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

/// Where the "before" of a changed file comes from, which is also where a
/// discard takes it back from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The commit the run started from.
    Snapshot,
    /// A capture the app committed during the run (it is in HEAD only).
    Capture,
    /// A queue file git does not know: the content saved in the snapshot.
    Queue,
    /// Nowhere: the run created the file.
    New,
}

/// One file the run added, changed or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
    /// As it was ([`Content::Absent`] for an added file).
    pub before: Content,
    /// In the working tree now ([`Content::Absent`] for a deleted file).
    pub after: Content,
    pub origin: Origin,
}

/// A commit, for messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub hash: String,
    pub subject: String,
}

/// Everything [`collect`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    /// Sorted by path.
    pub files: Vec<ChangedFile>,
    /// Commits since the snapshot that the app did not make.
    pub foreign_commits: Vec<CommitInfo>,
    /// Untracked files from before the run that were changed or removed.
    pub untracked_changed: Vec<String>,
}

impl Changes {
    /// Nothing happened at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.foreign_commits.is_empty() && self.untracked_changed.is_empty()
    }

    /// Identifies this exact set of changes, to notice that the working tree
    /// moved on between a check and the commit that relies on it.
    pub fn fingerprint(&self) -> String {
        let mut hash = Fnv::default();
        for file in &self.files {
            hash.text(&file.path);
            hash.text(match file.status {
                FileStatus::Added => "A",
                FileStatus::Modified => "M",
                FileStatus::Deleted => "D",
            });
            match &file.after {
                Content::Bytes(bytes) => hash.bytes(bytes),
                Content::Absent => hash.text("absent"),
                Content::Symlink => hash.text("symlink"),
                Content::Large => hash.text("large"),
                Content::Other => hash.text("other"),
            }
        }
        for commit in &self.foreign_commits {
            hash.text(&commit.hash);
        }
        for path in &self.untracked_changed {
            hash.text(path);
        }
        hash.hex()
    }
}

/// FNV-1a, 64 bit: a stable fingerprint (also across app versions, since
/// snapshots are saved to disk). Not for security.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A separator, so ("ab", "c") and ("a", "bc") differ.
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Git on the knowledge base, for this module.
struct Git<'a> {
    root: &'a Path,
    env: &'a ChildEnv,
}

impl Git<'_> {
    fn run(&self, args: &[&str]) -> Result<Finished, String> {
        let mut command = git::plumbing(self.root, self.env);
        command.args(args);
        git::finish(command, git::LOCAL_TIMEOUT)
    }

    fn name(args: &[&str]) -> String {
        args.iter()
            .find(|arg| !arg.starts_with('-') && !arg.contains('='))
            .unwrap_or(&"")
            .to_string()
    }

    /// Stdout as git printed it (for NUL-separated lists), or what went wrong.
    fn output(&self, args: &[&str]) -> Result<String, String> {
        let done = self.run(args)?;
        if done.success() {
            Ok(done.stdout)
        } else if done.timed_out {
            Err(format!("git {} 超时", Self::name(args)))
        } else {
            Err(format!("git {} 失败：{}", Self::name(args), done.detail()))
        }
    }

    fn read(&self, args: &[&str]) -> Result<String, String> {
        self.output(args).map(|out| out.trim().to_string())
    }

    fn succeeds(&self, args: &[&str]) -> bool {
        self.run(args).is_ok_and(|done| done.success())
    }

    /// For commands that write the index: tries again for a moment while
    /// another git process holds the index lock.
    fn write(&self, args: &[&str]) -> Result<(), String> {
        let mut attempt = 1;
        loop {
            match self.output(args) {
                Err(error) if error.contains("index.lock") && attempt < INDEX_LOCK_ATTEMPTS => {
                    attempt += 1;
                    thread::sleep(INDEX_LOCK_PAUSE);
                }
                result => return result.map(|_| ()),
            }
        }
    }

    /// The content of a blob (`<commit>:<path>` or an object id).
    fn blob(&self, object: &str) -> Result<Content, String> {
        let mut command = git::plumbing(self.root, self.env);
        command.args(["cat-file", "blob", object]);
        let done = git::finish_bytes(command, git::LOCAL_TIMEOUT, MAX_FILE_BYTES + 1)?;
        if !done.success() {
            return Err(format!(
                "git cat-file 失败：{}",
                String::from_utf8_lossy(&done.stderr).trim()
            ));
        }
        Ok(if done.stdout.len() > MAX_FILE_BYTES {
            Content::Large
        } else {
            Content::Bytes(done.stdout)
        })
    }

    /// NUL-separated paths from a listing command.
    fn paths(&self, args: &[&str]) -> Result<Vec<String>, String> {
        Ok(self
            .output(args)?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(String::from)
            .collect())
    }
}

/// `甲、乙 等 7 个文件` for messages.
fn name_some(paths: &[String]) -> String {
    let shown: Vec<&str> = paths.iter().take(MAX_NAMED).map(String::as_str).collect();
    if paths.len() > MAX_NAMED {
        format!("{} 等 {} 个文件", shown.join("、"), paths.len())
    } else {
        shown.join("、")
    }
}

/// A path that may be handed to git and the file system: inside the
/// knowledge base and outside `.git`. Unlike `paths::validate_rel_path` it
/// accepts any extension, because a run may touch any kind of file.
pub fn check_rel_path(rel: &str) -> Result<(), String> {
    let bad = rel.is_empty()
        || rel.contains('\0')
        || rel.contains('\\')
        || rel.starts_with('/')
        || rel
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".." | ".git"));
    if bad {
        Err(format!("路径不安全：{rel}"))
    } else {
        Ok(())
    }
}

/// What is at `rel` in the working tree right now.
fn read_worktree(root: &Path, rel: &str) -> Content {
    let path = root.join(rel);
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Content::Absent;
    };
    if meta.file_type().is_symlink() {
        Content::Symlink
    } else if !meta.is_file() {
        Content::Other
    } else if meta.len() > MAX_FILE_BYTES as u64 {
        Content::Large
    } else {
        match fs::read(&path) {
            Ok(bytes) => Content::Bytes(bytes),
            Err(_) => Content::Other,
        }
    }
}

/// A fingerprint of the file at `rel`, or `None` when nothing is there.
fn fingerprint(root: &Path, rel: &str) -> Option<String> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path).ok()?;
    let mut hash = Fnv::default();
    if meta.file_type().is_symlink() {
        hash.text("symlink");
        hash.text(&fs::read_link(&path).ok()?.to_string_lossy());
    } else if meta.is_file() {
        hash.text("file");
        hash.bytes(&fs::read(&path).ok()?);
    } else {
        hash.text("other");
    }
    Some(hash.hex())
}

/// Every folder below `root` (repo-relative), `.git` aside. Symlinked
/// folders are not followed.
fn list_dirs(root: &Path) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    let mut pending = vec![(String::new(), root.to_path_buf())];
    while let Some((prefix, dir)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if name == ".git" || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            pending.push((rel.clone(), entry.path()));
            dirs.insert(rel);
        }
    }
    dirs
}

/// The Markdown files directly in `queue_dir`, sorted by name. Symlinks,
/// hidden files and subfolders are not queue files.
fn read_queue(
    root: &Path,
    queue_dir: &str,
    tracked: &BTreeSet<String>,
) -> Result<Vec<QueueFile>, String> {
    let entries = match fs::read_dir(root.join(queue_dir)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取 {queue_dir} 失败：{error}")),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().to_str().map(String::from))
        .filter(|name| !name.starts_with('.') && paths::is_markdown_name(name))
        .collect();
    names.sort();

    let mut queue = Vec::new();
    for name in names {
        let path = format!("{queue_dir}/{name}");
        let bytes =
            fs::read(root.join(&path)).map_err(|error| format!("读取 {path} 失败：{error}"))?;
        let content = String::from_utf8(bytes)
            .map_err(|_| format!("队列文件不是 UTF-8 文本，没法逐字核对：{path}"))?;
        queue.push(QueueFile {
            tracked: tracked.contains(&path),
            path,
            content,
        });
    }
    Ok(queue)
}

/// Records where a run starts. `root` must be canonical; `queue_dir` is
/// `inbox` or `inbox/裁决`.
///
/// Fails (without changing anything) unless the folder is a repository root
/// on a branch, with at least one commit, no unfinished git operation and no
/// modified or staged tracked files.
pub fn snapshot(root: &Path, env: &ChildEnv, queue_dir: &str) -> Result<Snapshot, String> {
    git::ensure_repo(root, env)?;
    let git = Git { root, env };
    if let Some(operation) = sync::operation_in_progress(root, env)? {
        return Err(format!(
            "知识库里有一个没做完的 git {operation}，先在终端里完成或取消它"
        ));
    }
    let branch = git
        .read(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|name| !name.is_empty())
        .ok_or("知识库没有停在分支上（detached HEAD），先在终端里切回分支")?;
    let base = git
        .read(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .ok()
        .filter(|hash| !hash.is_empty())
        .ok_or("知识库还没有任何提交")?;
    let dirty = sync::tracked_changes(root, env)?;
    if !dirty.is_empty() {
        return Err(format!(
            "知识库里有还没提交的改动（{}），先提交或撤销它们",
            name_some(&dirty)
        ));
    }

    let tracked: BTreeSet<String> = git
        .paths(&["ls-files", "-z", "--", queue_dir])?
        .into_iter()
        .collect();
    let queue = read_queue(root, queue_dir, &tracked)?;
    let queued: BTreeSet<&str> = queue.iter().map(|file| file.path.as_str()).collect();
    let untracked = git
        .paths(&["ls-files", "--others", "--exclude-standard", "-z"])?
        .into_iter()
        .filter(|path| !queued.contains(path.as_str()))
        .filter_map(|path| fingerprint(root, &path).map(|print| (path, print)))
        .collect();

    Ok(Snapshot {
        base,
        branch,
        queue,
        untracked,
        dirs: list_dirs(root),
    })
}

/// True when `object` (e.g. `<hash>^{commit}`) names a commit of the
/// repository at `root`.
pub fn read_commit(root: &Path, env: &ChildEnv, object: &str) -> bool {
    Git { root, env }.succeeds(&["cat-file", "-e", object])
}

/// The content of `rel` as of `commit`, or `None` when it did not exist then.
pub fn read_at(
    root: &Path,
    env: &ChildEnv,
    commit: &str,
    rel: &str,
) -> Result<Option<Vec<u8>>, String> {
    check_rel_path(rel)?;
    let git = Git { root, env };
    let object = format!("{commit}:{rel}");
    if !git.succeeds(&["cat-file", "-e", &object]) {
        return Ok(None);
    }
    match git.blob(&object)? {
        Content::Bytes(bytes) => Ok(Some(bytes)),
        _ => Err(format!("文件太大，没有读取：{rel}")),
    }
}

/// One entry of `git diff --raw -z`: the modes and object ids on both sides.
struct RawEntry {
    old_mode: String,
    old_id: String,
    path: String,
}

fn parse_raw_diff(output: &str) -> Vec<RawEntry> {
    let mut entries = Vec::new();
    let mut fields = output.split('\0');
    while let Some(meta) = fields.next() {
        let Some(meta) = meta.strip_prefix(':') else {
            continue;
        };
        let Some(path) = fields.next() else { break };
        let mut parts = meta.split(' ');
        let old_mode = parts.next().unwrap_or_default().to_string();
        let _new_mode = parts.next();
        let old_id = parts.next().unwrap_or_default().to_string();
        entries.push(RawEntry {
            old_mode,
            old_id,
            path: path.to_string(),
        });
    }
    entries
}

/// What `mode` and `id` of a tree entry hold.
fn tree_content(git: &Git, mode: &str, id: &str) -> Result<Content, String> {
    match mode {
        "000000" => Ok(Content::Absent),
        "120000" => Ok(Content::Symlink),
        "100644" | "100755" => git.blob(id),
        _ => Ok(Content::Other),
    }
}

/// Lists what changed since `snapshot`. `beside` are the commits the app made
/// meanwhile (captures): they and their files are not changes of the run.
pub fn collect(
    root: &Path,
    env: &ChildEnv,
    snapshot: &Snapshot,
    beside: &[NotedCommit],
) -> Result<Changes, String> {
    let git = Git { root, env };
    let head = git.read(&["rev-parse", "--verify", "HEAD^{commit}"])?;
    let ours: BTreeSet<&str> = beside.iter().map(|noted| noted.commit.as_str()).collect();
    let captured: BTreeSet<&str> = beside.iter().map(|noted| noted.path.as_str()).collect();

    let mut foreign_commits = Vec::new();
    if head != snapshot.base {
        if git.succeeds(&["merge-base", "--is-ancestor", &snapshot.base, &head]) {
            let range = format!("{}..{head}", snapshot.base);
            let log = git.read(&["log", "--reverse", "--format=%H%x1f%s", &range])?;
            for line in log.lines() {
                let (hash, subject) = line.split_once('\u{1f}').unwrap_or((line, ""));
                if !ours.contains(hash) {
                    foreign_commits.push(CommitInfo {
                        hash: hash.to_string(),
                        subject: subject.to_string(),
                    });
                }
            }
        } else {
            foreign_commits.push(CommitInfo {
                hash: head.clone(),
                subject: "当前分支已经不包含运行开始时的提交".to_string(),
            });
        }
    }

    let mut files: BTreeMap<String, ChangedFile> = BTreeMap::new();

    // Tracked files against the commit the run started from.
    let raw = git.output(&[
        "--no-optional-locks",
        "diff",
        "--raw",
        "-z",
        "--no-abbrev",
        "--no-renames",
        "--no-ext-diff",
        &snapshot.base,
        "--",
    ])?;
    for entry in parse_raw_diff(&raw) {
        // A file that was untracked before the run is the user's, whatever
        // happened to it since (it is reported below when it changed).
        if snapshot.untracked.contains_key(&entry.path) || captured.contains(entry.path.as_str()) {
            continue;
        }
        let before = tree_content(&git, &entry.old_mode, &entry.old_id)?;
        let after = read_worktree(root, &entry.path);
        let (status, origin) = match (before.exists(), after.exists()) {
            (false, false) => continue,
            (false, true) => (FileStatus::Added, Origin::New),
            (true, false) => (FileStatus::Deleted, Origin::Snapshot),
            (true, true) => (FileStatus::Modified, Origin::Snapshot),
        };
        files.insert(
            entry.path.clone(),
            ChangedFile {
                path: entry.path,
                status,
                before,
                after,
                origin,
            },
        );
    }

    // A capture committed during the run that is not as committed any more.
    if !captured.is_empty() {
        let mut args = vec!["--no-optional-locks", "diff", "--name-only", "-z", "HEAD", "--"];
        args.extend(captured.iter().copied());
        for path in git.paths(&args)? {
            let before = git.blob(&format!("HEAD:{path}"))?;
            let after = read_worktree(root, &path);
            let status = if after.exists() {
                FileStatus::Modified
            } else {
                FileStatus::Deleted
            };
            files.insert(
                path.clone(),
                ChangedFile {
                    path,
                    status,
                    before,
                    after,
                    origin: Origin::Capture,
                },
            );
        }
    }

    // Untracked files: new ones are the run's, the old ones are the user's.
    let queued: BTreeMap<&str, &QueueFile> = snapshot
        .queue
        .iter()
        .filter(|file| !file.tracked)
        .map(|file| (file.path.as_str(), file))
        .collect();
    for path in git.paths(&["ls-files", "--others", "--exclude-standard", "-z"])? {
        if snapshot.untracked.contains_key(&path) || queued.contains_key(path.as_str()) {
            continue;
        }
        let after = read_worktree(root, &path);
        if after.exists() {
            files.insert(
                path.clone(),
                ChangedFile {
                    path,
                    status: FileStatus::Added,
                    before: Content::Absent,
                    after,
                    origin: Origin::New,
                },
            );
        }
    }
    for (path, file) in queued {
        let after = read_worktree(root, path);
        if after.bytes() == Some(file.content.as_bytes()) {
            continue;
        }
        let status = if after.exists() {
            FileStatus::Modified
        } else {
            FileStatus::Deleted
        };
        files.insert(
            path.to_string(),
            ChangedFile {
                path: path.to_string(),
                status,
                before: Content::Bytes(file.content.clone().into_bytes()),
                after,
                origin: Origin::Queue,
            },
        );
    }
    let untracked_changed = snapshot
        .untracked
        .iter()
        .filter(|(path, print)| fingerprint(root, path).as_ref() != Some(*print))
        .map(|(path, _)| path.clone())
        .collect();

    Ok(Changes {
        files: files.into_values().collect(),
        foreign_commits,
        untracked_changed,
    })
}

/// What [`commit_run`] should do.
#[derive(Debug, Clone, Copy)]
pub struct CommitPlan<'a> {
    /// Changed paths to commit as they are in the working tree now.
    pub paths: &'a [String],
    /// Queue files to remove in the same commit.
    pub delete_queue: &'a [String],
    /// Queue files to put back as they were (the engine removed or edited them).
    pub restore_queue: &'a [String],
    pub message: &'a str,
}

/// Writes a queue file back with the content it had when the run started.
fn restore_queue_file(root: &Path, file: &QueueFile) -> Result<(), String> {
    let path = root.join(&file.path);
    if fs::read(&path).is_ok_and(|now| now == file.content.as_bytes()) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("没能放回 {}：{error}", file.path))?;
    }
    // Remove first: whatever is there now (also a symlink) is not followed.
    let _ = fs::remove_file(&path);
    fs::write(&path, &file.content).map_err(|error| format!("没能放回 {}：{error}", file.path))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Makes the one commit of a run: the files in `plan.paths` as they are now,
/// minus the queue files in `plan.delete_queue`. Queue files in
/// `plan.restore_queue` are put back first. Nothing else is staged or
/// committed, whatever else is in the working tree or the index.
///
/// When git refuses the commit, the index is put back and the queue files
/// are restored, so the run can still be kept or discarded afterwards.
/// Returns the hash of the commit.
pub fn commit_run(
    root: &Path,
    env: &ChildEnv,
    snapshot: &Snapshot,
    plan: CommitPlan<'_>,
) -> Result<String, String> {
    git::validate_message(plan.message)?;
    for path in plan
        .paths
        .iter()
        .chain(plan.delete_queue)
        .chain(plan.restore_queue)
    {
        check_rel_path(path)?;
    }
    let queue_file = |path: &String| {
        snapshot
            .queue_file(path)
            .ok_or_else(|| format!("不是这次运行的队列文件：{path}"))
    };
    let delete: Vec<&QueueFile> = plan
        .delete_queue
        .iter()
        .map(queue_file)
        .collect::<Result<_, _>>()?;
    let restore: Vec<&QueueFile> = plan
        .restore_queue
        .iter()
        .map(queue_file)
        .collect::<Result<_, _>>()?;
    if let Some(path) = plan.paths.iter().find(|path| snapshot.queue_file(path).is_some()) {
        return Err(format!("队列文件不能当作改动提交：{path}"));
    }

    let git = Git { root, env };
    for file in &restore {
        restore_queue_file(root, file)?;
    }

    let mut committed: Vec<&str> = plan.paths.iter().map(String::as_str).collect();
    let result = (|| {
        for file in &delete {
            if file.tracked {
                git.write(&["rm", "--quiet", "--force", "--ignore-unmatch", "--", &file.path])?;
                committed.push(&file.path);
            } else {
                remove_if_present(&root.join(&file.path))
                    .map_err(|error| format!("没能删除 {}：{error}", file.path))?;
            }
        }
        if !plan.paths.is_empty() {
            let mut args = vec!["add", "--all", "--"];
            args.extend(plan.paths.iter().map(String::as_str));
            git.write(&args)?;
        }
        if committed.is_empty() {
            return Err("没有可以提交的改动".to_string());
        }
        let mut staged = vec!["diff", "--cached", "--quiet", "--no-ext-diff", "--"];
        staged.extend(committed.iter().copied());
        if git.succeeds(&staged) {
            return Err("没有可以提交的改动".to_string());
        }
        let mut args = vec!["commit", "--quiet", "-m", plan.message, "--"];
        args.extend(committed.iter().copied());
        git.write(&args)?;
        git.read(&["rev-parse", "HEAD"])
    })();

    if result.is_err() {
        // Back to "changed but not staged", with every queue file in place.
        if !committed.is_empty() {
            let mut args = vec!["reset", "--quiet", "--"];
            args.extend(committed.iter().copied());
            if let Err(error) = git.write(&args) {
                log::warn!("run commit: could not unstage after a failed commit: {error}");
            }
        }
        for file in &delete {
            if let Err(error) = restore_queue_file(root, file) {
                log::warn!("run commit: {error}");
            }
        }
    }
    result
}

/// What [`discard_run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discarded {
    /// How many files went back to what they were.
    pub restored: usize,
    /// How many files the run had created and are gone now.
    pub removed: usize,
    /// The commit that takes back commits the engine made itself, if there were any.
    pub revert_commit: Option<String>,
}

/// Copies what the run left at `rel` into `backup_dir`, keeping its path.
fn back_up(root: &Path, rel: &str, backup_dir: &Path) -> Result<(), String> {
    let source = root.join(rel);
    let failed = |error: io::Error| format!("没能备份 {rel}，所以什么都没有丢弃：{error}");
    let Ok(meta) = fs::symlink_metadata(&source) else {
        return Ok(());
    };
    let target = backup_dir.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    if meta.file_type().is_symlink() {
        // Keep where the link pointed, not what it pointed at.
        let link = fs::read_link(&source).map_err(failed)?;
        let mut name = target.into_os_string();
        name.push(".symlink");
        fs::write(PathBuf::from(name), link.to_string_lossy().as_bytes()).map_err(failed)
    } else if meta.is_file() {
        fs::copy(&source, &target).map(|_| ()).map_err(failed)
    } else {
        Ok(())
    }
}

/// Undoes a run. Every file the run changed or created is first copied into
/// `backup_dir` (keeping its path), so a discard can be taken back by hand.
/// Then tracked files go back to the snapshot's commit, queue files are put
/// back, and files the run created are deleted, together with folders that
/// became empty and did not exist before. Untracked files from before the run
/// are never touched, and neither are captures made meanwhile.
///
/// If the engine made commits of its own, history is not rewritten: one more
/// commit takes their changes back. It only names the paths git knows (in
/// HEAD or in the index): a file the engine created but never committed is
/// simply deleted.
///
/// All or nothing: when any step fails, the working tree, the index and HEAD
/// are put back to what the run left (from the copies), so the discard can
/// simply be tried again.
pub fn discard_run(
    root: &Path,
    env: &ChildEnv,
    snapshot: &Snapshot,
    changes: &Changes,
    backup_dir: &Path,
    revert_message: &str,
) -> Result<Discarded, String> {
    for file in &changes.files {
        check_rel_path(&file.path)?;
    }
    for file in &changes.files {
        back_up(root, &file.path, backup_dir)?;
    }
    let git = Git { root, env };
    let head = git.read(&["rev-parse", "--verify", "HEAD^{commit}"])?;
    match apply_discard(root, &git, snapshot, changes, revert_message) {
        Ok(done) => Ok(done),
        Err(error) => match undo_discard(root, &git, changes, backup_dir, &head) {
            Ok(()) => Err(error),
            Err(problem) => {
                log::error!("run discard: could not put the run back after a failure: {problem}");
                Err(format!(
                    "{error}。而且没能把文件完全放回原样（{problem}），改动的副本在 {}",
                    backup_dir.display()
                ))
            }
        },
    }
}

/// The paths among `paths` that git knows: in HEAD or in the index.
fn known_to_git<'a>(git: &Git, paths: &[&'a str]) -> Result<Vec<&'a str>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut known: BTreeSet<String> = git
        .paths(&["ls-files", "-z"])?
        .into_iter()
        .collect();
    known.extend(git.paths(&["ls-tree", "-r", "--name-only", "-z", "HEAD"])?);
    Ok(paths
        .iter()
        .copied()
        .filter(|path| known.contains(*path))
        .collect())
}

fn apply_discard(
    root: &Path,
    git: &Git,
    snapshot: &Snapshot,
    changes: &Changes,
    revert_message: &str,
) -> Result<Discarded, String> {
    let of = |origin: Origin| -> Vec<&str> {
        changes
            .files
            .iter()
            .filter(|file| file.origin == origin)
            .map(|file| file.path.as_str())
            .collect()
    };

    // Files git knows go back to what they were: to the commit the run
    // started from, or to HEAD for a capture that was committed meanwhile.
    let from_snapshot = of(Origin::Snapshot);
    let from_head = of(Origin::Capture);
    for (commit, paths) in [(snapshot.base.as_str(), &from_snapshot), ("HEAD", &from_head)] {
        for chunk in paths.chunks(200) {
            // A link where the file used to be would be written through.
            for path in chunk {
                let full = root.join(path);
                if fs::symlink_metadata(&full).is_ok_and(|meta| meta.file_type().is_symlink()) {
                    let _ = fs::remove_file(&full);
                }
            }
            let mut args = vec!["checkout", "--quiet", commit, "--"];
            args.extend(chunk.iter().copied());
            git.write(&args)?;
        }
    }
    let mut restored = from_snapshot.len() + from_head.len();
    for path in of(Origin::Queue) {
        if let Some(queued) = snapshot.queue_file(path) {
            restore_queue_file(root, queued)?;
            restored += 1;
        }
    }

    // Files the run created go away.
    let created = of(Origin::New);
    for chunk in created.chunks(200) {
        // Only matters when the engine added them to the index itself.
        let mut args = vec!["rm", "--quiet", "--force", "--cached", "--ignore-unmatch", "--"];
        args.extend(chunk.iter().copied());
        git.write(&args)?;
    }
    let mut removed = 0;
    for path in &created {
        let full = root.join(path);
        if fs::symlink_metadata(&full).is_ok() {
            remove_if_present(&full).map_err(|error| format!("没能删除 {path}：{error}"))?;
            removed += 1;
        }
    }
    remove_new_empty_dirs(root, snapshot, &created);

    // Commits the engine made itself: one commit takes their changes back.
    let touched: Vec<&str> = from_snapshot.iter().chain(created.iter()).copied().collect();
    let known = known_to_git(git, &touched)?;
    let staged_in = |paths: &[&str]| {
        let mut args = vec!["diff", "--cached", "--quiet", "--no-ext-diff", "--"];
        args.extend(paths.iter().copied());
        !git.succeeds(&args)
    };
    let mut revert_commit = None;
    if !changes.foreign_commits.is_empty() && !known.is_empty() && staged_in(&known) {
        let mut args = vec!["commit", "--quiet", "-m", revert_message, "--"];
        args.extend(known.iter().copied());
        git.write(&args)?;
        revert_commit = Some(git.read(&["rev-parse", "HEAD"])?);
    }
    // Nothing of the discard may stay staged without being committed.
    let all: Vec<&str> = changes.files.iter().map(|file| file.path.as_str()).collect();
    let known_all = known_to_git(git, &all)?;
    if !known_all.is_empty() && staged_in(&known_all) {
        return Err("丢弃没有做完：还有改动停在暂存区里，没有提交".to_string());
    }

    Ok(Discarded {
        restored,
        removed,
        revert_commit,
    })
}

/// Puts back what the run left after a discard failed halfway: HEAD as it
/// was, the index of the run's paths as in HEAD, and every file from its copy
/// (a file that had no copy did not exist).
fn undo_discard(
    root: &Path,
    git: &Git,
    changes: &Changes,
    backup_dir: &Path,
    head: &str,
) -> Result<(), String> {
    let mut problems = Vec::new();
    let now = git.read(&["rev-parse", "--verify", "HEAD^{commit}"])?;
    if now != head {
        if let Err(error) = git.write(&["reset", "--quiet", "--soft", head]) {
            problems.push(error);
        }
    }
    let paths: Vec<&str> = changes.files.iter().map(|file| file.path.as_str()).collect();
    for chunk in paths.chunks(200) {
        let mut args = vec!["reset", "--quiet", head, "--"];
        args.extend(chunk.iter().copied());
        if let Err(error) = git.write(&args) {
            problems.push(error);
        }
    }
    for file in &changes.files {
        let target = root.join(&file.path);
        let copy = backup_dir.join(&file.path);
        let mut link = copy.clone().into_os_string();
        link.push(".symlink");
        let link = PathBuf::from(link);
        let put_back = || -> io::Result<()> {
            if fs::symlink_metadata(&target).is_ok_and(|meta| !meta.is_dir()) {
                fs::remove_file(&target)?;
            }
            if copy.is_file() || link.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
            }
            if copy.is_file() {
                fs::copy(&copy, &target).map(|_| ())
            } else if link.is_file() {
                std::os::unix::fs::symlink(fs::read_to_string(&link)?, &target)
            } else {
                Ok(())
            }
        };
        if let Err(error) = put_back() {
            problems.push(format!("{}：{error}", file.path));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("；"))
    }
}

/// Removes the folders of `created` files that are empty now and were not
/// there when the run started, deepest first.
fn remove_new_empty_dirs(root: &Path, snapshot: &Snapshot, created: &[&str]) {
    let mut dirs = BTreeSet::new();
    for path in created {
        let mut current = *path;
        while let Some((parent, _)) = current.rsplit_once('/') {
            if !snapshot.dirs.contains(parent) {
                dirs.insert(parent.to_string());
            }
            current = parent;
        }
    }
    // Longer paths first, so a folder is emptied before its parent is tried.
    let mut dirs: Vec<String> = dirs.into_iter().collect();
    dirs.sort_by_key(|dir| std::cmp::Reverse(dir.matches('/').count()));
    for dir in dirs {
        // Fails, as it should, when the folder is not empty.
        let _ = fs::remove_dir(root.join(dir));
    }
}

#[cfg(test)]
mod tests;
