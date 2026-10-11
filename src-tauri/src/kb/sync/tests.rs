//! Sync scenarios against a real bare "remote" in a temp folder, with two
//! clones: `kb` (the folder the app syncs) and `other` (another device that
//! pushes to the same remote). Every scenario also plants untracked and
//! ignored files in `kb` and checks that the sync left them alone.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::kb::test_support::{git, TempDir};

fn env() -> ChildEnv {
    ChildEnv::inherit()
}

fn timeouts() -> Timeouts {
    Timeouts::default()
}

/// Repo-local identity and no signing, so the user's global setup never matters.
fn configure(dir: &Path) {
    git(dir, &["config", "user.name", "Dise Test"]);
    git(dir, &["config", "user.email", "dise-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Writes, stages and commits one file; returns the new HEAD.
fn commit(dir: &Path, rel: &str, content: &str, message: &str) -> String {
    write(dir, rel, content);
    git(dir, &["add", "--", rel]);
    git(dir, &["commit", "-q", "-m", message]);
    head(dir)
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

/// `git status` including untracked files, with readable file names.
fn porcelain(dir: &Path) -> String {
    git(
        dir,
        &[
            "-c",
            "core.quotepath=off",
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    )
}

/// Runs git and returns whether it succeeded (for commands that may fail).
fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run git")
        .status
        .success()
}

fn backup_refs(dir: &Path) -> Vec<String> {
    let listed = git(
        dir,
        &["for-each-ref", "--format=%(refname)", "refs/dise/backup"],
    );
    listed.lines().map(String::from).collect()
}

/// A bare remote and two clones of it.
struct World {
    _dir: TempDir,
    remote: PathBuf,
    kb: PathBuf,
    other: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let root = dir.path().to_path_buf();
        let remote = root.join("remote.git");
        git(&root, &["init", "--bare", "-q", "-b", "main", "remote.git"]);

        let seed = root.join("seed");
        fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        configure(&seed);
        write(&seed, "index.md", "# 目录\n");
        write(&seed, "self/底色.md", "### D01 先确认能退回来\n\n第一行\n");
        write(&seed, "wiki/概念/甲.md", "# 甲\n");
        git(&seed, &["add", "--", "."]);
        git(&seed, &["commit", "-q", "-m", "init"]);
        git(&seed, &["push", "-q", remote.to_str().unwrap(), "main"]);

        let clone = |name: &str| {
            git(&root, &["clone", "-q", remote.to_str().unwrap(), name]);
            let path = root.join(name);
            configure(&path);
            path
        };
        let kb = clone("kb");
        let other = clone("other");
        World {
            _dir: dir,
            remote,
            kb,
            other,
        }
    }

    fn remote_main(&self) -> String {
        git(
            &self.kb,
            &[
                "--git-dir",
                self.remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/main",
            ],
        )
    }

    /// Another device commits one file and pushes it.
    fn other_pushes(&self, rel: &str, content: &str, message: &str) -> String {
        git(&self.other, &["pull", "-q", "--ff-only"]);
        let head = commit(&self.other, rel, content, message);
        git(&self.other, &["push", "-q", "origin", "main"]);
        head
    }

    fn sync(&self) -> SyncReport {
        sync(&self.kb, &env(), &timeouts())
    }
}

/// Untracked and ignored files a person has lying around in the folder.
struct Untracked {
    root: PathBuf,
    files: Vec<(&'static str, &'static str, SystemTime)>,
}

impl Untracked {
    fn plant(root: &Path) -> Self {
        let exclude = root.join(".git/info/exclude");
        let mut excluded = fs::read_to_string(&exclude).unwrap_or_default();
        excluded.push_str("*.tmp\n");
        fs::write(&exclude, excluded).unwrap();

        let contents = [
            ("草稿/还没想好的.md", "还没想好\n"),
            ("未跟踪.txt", "随手放的\n"),
            (".obsidian/workspace.json", "{\"open\": []}\n"),
            ("cache.tmp", "被忽略的文件\n"),
        ];
        let files = contents
            .into_iter()
            .map(|(rel, content)| {
                write(root, rel, content);
                let modified = fs::metadata(root.join(rel)).unwrap().modified().unwrap();
                (rel, content, modified)
            })
            .collect();
        Untracked {
            root: root.to_path_buf(),
            files,
        }
    }

    fn assert_intact(&self) {
        for (rel, content, modified) in &self.files {
            let path = self.root.join(rel);
            assert_eq!(
                fs::read_to_string(&path).unwrap_or_else(|_| panic!("{rel} is gone")),
                *content,
                "{rel} changed"
            );
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                *modified,
                "{rel} was rewritten"
            );
        }
    }
}

// ---------- the four basic shapes ----------

#[test]
fn ahead_only_pushes() {
    let world = World::new("sync-push");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/2026-10-12T14-03-22-mac.md",
        "---\n来源: mac\n---\n\n一条\n",
        "收件箱：记一笔（mac）",
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    assert_eq!((report.pulled, report.pushed), (0, 1));
    assert!(!report.changed);
    assert_eq!(report.backup, None);
    assert_eq!(world.remote_main(), local);
    assert_eq!(head(&world.kb), local);
    untracked.assert_intact();

    // Nothing left to do: a second sync changes nothing.
    let again = world.sync();
    assert_eq!(again.state, SyncState::Synced);
    assert_eq!((again.pulled, again.pushed, again.changed), (0, 0, false));
    untracked.assert_intact();
}

#[test]
fn behind_only_fast_forwards() {
    let world = World::new("sync-ff");
    let untracked = Untracked::plant(&world.kb);
    world.other_pushes("wiki/概念/乙.md", "# 乙\n", "知识：乙");
    let remote = world.other_pushes("wiki/概念/丙.md", "# 丙\n", "知识：丙");

    let report = world.sync();
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    assert_eq!((report.pulled, report.pushed), (2, 0));
    assert!(report.changed);
    assert_eq!(report.backup, None, "a fast-forward needs no backup");
    assert_eq!(head(&world.kb), remote);
    assert_eq!(read(&world.kb, "wiki/概念/丙.md"), "# 丙\n");
    untracked.assert_intact();
}

#[test]
fn a_fast_forward_keeps_unrelated_local_changes() {
    let world = World::new("sync-ff-unrelated");
    let untracked = Untracked::plant(&world.kb);
    write(&world.kb, "index.md", "# 目录（本机改过，还没提交）\n");
    let remote = world.other_pushes("wiki/概念/乙.md", "# 乙\n", "知识：乙");

    let report = world.sync();
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    assert_eq!(head(&world.kb), remote);
    assert_eq!(
        read(&world.kb, "index.md"),
        "# 目录（本机改过，还没提交）\n"
    );
    untracked.assert_intact();
}

#[test]
fn a_fast_forward_that_would_overwrite_local_changes_is_skipped() {
    let world = World::new("sync-ff-dirty");
    let untracked = Untracked::plant(&world.kb);
    let before = head(&world.kb);
    write(
        &world.kb,
        "self/底色.md",
        "### D01 先确认能退回来\n\n本机还没提交的改动\n",
    );
    let remote = world.other_pushes(
        "self/底色.md",
        "### D01 先确认能退回来\n\n远端的改动\n",
        "底色：改写 D01",
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Dirty, "{report:?}");
    assert_eq!(report.paths, vec!["self/底色.md"]);
    assert!(!report.changed);
    assert_eq!(head(&world.kb), before);
    assert_eq!(
        read(&world.kb, "self/底色.md"),
        "### D01 先确认能退回来\n\n本机还没提交的改动\n"
    );
    assert_eq!(world.remote_main(), remote);
    untracked.assert_intact();
}

#[test]
fn an_untracked_file_in_the_way_blocks_the_fast_forward() {
    let world = World::new("sync-ff-untracked");
    let untracked = Untracked::plant(&world.kb);
    let before = head(&world.kb);
    write(&world.kb, "inbox/同名.md", "本机这份没有加入版本管理\n");
    world.other_pushes(
        "inbox/同名.md",
        "远端的同名文件\n",
        "收件箱：记一笔（iphone）",
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Dirty, "{report:?}");
    assert_eq!(report.paths, vec!["inbox/同名.md"]);
    assert_eq!(head(&world.kb), before);
    assert_eq!(
        read(&world.kb, "inbox/同名.md"),
        "本机这份没有加入版本管理\n"
    );
    untracked.assert_intact();
}

#[test]
fn diverged_with_different_files_rebases_keeps_both_sides_and_pushes() {
    let world = World::new("sync-rebase");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/2026-10-12T14-03-22-mac.md",
        "本机记的\n",
        "收件箱：记一笔（mac）",
    );
    let remote = world.other_pushes(
        "inbox/2026-10-12T14-05-00-iphone.md",
        "手机记的\n",
        "收件箱：记一笔（iphone）",
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    assert_eq!((report.pulled, report.pushed), (1, 1));
    assert!(report.changed);

    // Both sides are there, in a straight line: the remote's commit, then ours.
    let now = head(&world.kb);
    assert_eq!(world.remote_main(), now);
    assert_eq!(git(&world.kb, &["rev-parse", "HEAD~1"]), remote);
    assert_eq!(
        git(&world.kb, &["log", "-1", "--format=%s%n%an"]),
        "收件箱：记一笔（mac）\nDise Test"
    );
    assert_eq!(
        read(&world.kb, "inbox/2026-10-12T14-03-22-mac.md"),
        "本机记的\n"
    );
    assert_eq!(
        read(&world.kb, "inbox/2026-10-12T14-05-00-iphone.md"),
        "手机记的\n"
    );

    // The commit from before the rebase is kept under a backup ref.
    let backup = report.backup.clone().expect("backup ref");
    assert!(backup.starts_with(BACKUP_PREFIX), "{backup}");
    assert_eq!(git(&world.kb, &["rev-parse", &backup]), local);
    assert_eq!(backup_refs(&world.kb), vec![backup]);
    untracked.assert_intact();
}

#[test]
fn a_real_conflict_is_aborted_and_head_restored() {
    let world = World::new("sync-conflict");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "self/底色.md",
        "### D01 先确认能退回来\n\n本机这样改\n",
        "底色：本机改 D01",
    );
    let remote = world.other_pushes(
        "self/底色.md",
        "### D01 先确认能退回来\n\n远端那样改\n",
        "底色：远端改 D01",
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Conflict, "{report:?}");
    assert_eq!(report.paths, vec!["self/底色.md"]);
    assert!(!report.changed);
    assert_eq!((report.pulled, report.pushed), (0, 0));
    assert!(
        report.detail.as_deref().unwrap_or("").contains("CONFLICT"),
        "{report:?}"
    );

    // Exactly as before: same commit, same files, no rebase left behind.
    assert_eq!(head(&world.kb), local);
    assert_eq!(
        git(&world.kb, &["symbolic-ref", "--short", "HEAD"]),
        "main",
        "back on the branch"
    );
    assert!(!world.kb.join(".git/rebase-merge").exists());
    assert!(!world.kb.join(".git/rebase-apply").exists());
    assert_eq!(
        read(&world.kb, "self/底色.md"),
        "### D01 先确认能退回来\n\n本机这样改\n"
    );
    assert!(
        porcelain(&world.kb)
            .lines()
            .all(|line| line.starts_with("??")),
        "{}",
        porcelain(&world.kb)
    );
    assert_eq!(world.remote_main(), remote);

    let backup = report.backup.clone().expect("backup ref");
    assert_eq!(git(&world.kb, &["rev-parse", &backup]), local);
    untracked.assert_intact();
}

#[test]
fn diverged_with_modified_tracked_files_is_skipped() {
    let world = World::new("sync-diverged-dirty");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    let remote = world.other_pushes("inbox/手机.md", "手机\n", "收件箱：记一笔（iphone）");
    // A tracked file is modified but not committed.
    write(&world.kb, "wiki/概念/甲.md", "# 甲（改了一半）\n");

    let report = world.sync();
    assert_eq!(report.state, SyncState::Dirty, "{report:?}");
    assert_eq!(report.paths, vec!["wiki/概念/甲.md"]);
    assert_eq!(report.backup, None, "nothing was tried");
    assert!(backup_refs(&world.kb).is_empty());
    assert_eq!(head(&world.kb), local);
    assert_eq!(read(&world.kb, "wiki/概念/甲.md"), "# 甲（改了一半）\n");
    assert_eq!(world.remote_main(), remote);
    untracked.assert_intact();

    // A staged change counts as well.
    git(&world.kb, &["add", "--", "wiki/概念/甲.md"]);
    let staged = world.sync();
    assert_eq!(staged.state, SyncState::Dirty, "{staged:?}");
    assert_eq!(head(&world.kb), local);
    assert_eq!(
        git(
            &world.kb,
            &[
                "-c",
                "core.quotepath=off",
                "diff",
                "--cached",
                "--name-only"
            ]
        ),
        "wiki/概念/甲.md"
    );
    untracked.assert_intact();
}

// ---------- the remote ----------

#[test]
fn an_unreachable_remote_is_offline_and_nothing_changes() {
    let world = World::new("sync-offline");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    let remote = world.remote_main();
    let missing = world.kb.parent().unwrap().join("no-such-remote.git");
    git(
        &world.kb,
        &["remote", "set-url", "origin", missing.to_str().unwrap()],
    );

    let report = world.sync();
    assert_eq!(report.state, SyncState::Offline, "{report:?}");
    assert!(report.detail.is_some());
    assert_eq!(
        (report.pulled, report.pushed, report.changed),
        (0, 0, false)
    );
    assert_eq!(head(&world.kb), local);
    assert_eq!(world.remote_main(), remote);
    untracked.assert_intact();
}

#[test]
fn a_remote_that_hangs_is_given_up_after_the_time_limit() {
    let world = World::new("sync-hang");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    // The remote end of the fetch never answers.
    let hang = world.kb.parent().unwrap().join("hang-upload-pack");
    fs::write(&hang, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&hang, fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &world.kb,
        &["config", "remote.origin.uploadpack", hang.to_str().unwrap()],
    );

    let limits = Timeouts {
        fetch: Duration::from_secs(1),
        ..Timeouts::default()
    };
    let started = Instant::now();
    let report = sync(&world.kb, &env(), &limits);
    assert_eq!(report.state, SyncState::Offline, "{report:?}");
    assert!(
        report.detail.as_deref().unwrap().contains("超过 1 秒"),
        "{report:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(head(&world.kb), local);
    untracked.assert_intact();
}

#[test]
fn the_remote_moving_between_fetch_and_push_is_retried_once() {
    let world = World::new("sync-moved");
    let untracked = Untracked::plant(&world.kb);
    commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );

    let mut moved = None;
    let report = sync_with(&world.kb, &env(), &timeouts(), &mut |round| {
        if round == 1 {
            moved = Some(world.other_pushes("inbox/手机.md", "手机\n", "收件箱：记一笔（iphone）"));
        }
    });
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    assert_eq!((report.pulled, report.pushed), (1, 1));
    assert!(report.changed);
    let now = head(&world.kb);
    assert_eq!(world.remote_main(), now);
    assert_eq!(git(&world.kb, &["rev-parse", "HEAD~1"]), moved.unwrap());
    assert_eq!(read(&world.kb, "inbox/本机.md"), "本机\n");
    assert_eq!(read(&world.kb, "inbox/手机.md"), "手机\n");
    untracked.assert_intact();
}

#[test]
fn a_remote_that_keeps_moving_is_left_for_the_next_sync() {
    let world = World::new("sync-moving");
    let untracked = Untracked::plant(&world.kb);
    commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );

    let mut last = String::new();
    let report = sync_with(&world.kb, &env(), &timeouts(), &mut |round| {
        last = world.other_pushes(
            &format!("inbox/手机-{round}.md"),
            "手机\n",
            "收件箱：记一笔（iphone）",
        );
    });
    assert_eq!(report.state, SyncState::RemoteMoved, "{report:?}");
    assert_eq!(report.pushed, 0);
    // Nothing lost: the local commit sits on top of the first remote commit.
    assert_eq!(read(&world.kb, "inbox/本机.md"), "本机\n");
    assert_eq!(world.remote_main(), last);
    untracked.assert_intact();

    // The next sync brings everything together.
    let next = world.sync();
    assert_eq!(next.state, SyncState::Synced, "{next:?}");
    assert_eq!(world.remote_main(), head(&world.kb));
    assert_eq!(read(&world.kb, "inbox/手机-2.md"), "手机\n");
    untracked.assert_intact();
}

#[test]
fn a_push_the_remote_refuses_for_its_own_reasons_fails_clearly() {
    let world = World::new("sync-hook");
    let untracked = Untracked::plant(&world.kb);
    let local = commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    let hook = world.remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\necho 'read-only for now' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let remote = world.remote_main();

    let report = world.sync();
    assert_eq!(report.state, SyncState::Failed, "{report:?}");
    assert!(
        report.detail.as_deref().unwrap().contains("远端拒绝"),
        "{report:?}"
    );
    assert_eq!(head(&world.kb), local);
    assert_eq!(world.remote_main(), remote);
    untracked.assert_intact();
}

// ---------- states in which a sync does not start ----------

#[test]
fn only_a_repository_root_is_synced() {
    let plain = TempDir::new("sync-plain");
    let report = sync(plain.path(), &env(), &timeouts());
    assert_eq!(report.state, SyncState::NotRepo, "{report:?}");

    let world = World::new("sync-subfolder");
    let report = sync(&world.kb.join("wiki"), &env(), &timeouts());
    assert_eq!(report.state, SyncState::NotRepo, "{report:?}");
}

#[test]
fn a_branch_without_upstream_is_not_synced() {
    let dir = TempDir::new("sync-no-upstream");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    configure(dir.path());
    commit(dir.path(), "index.md", "# 目录\n", "init");
    let untracked = Untracked::plant(dir.path());
    let report = sync(dir.path(), &env(), &timeouts());
    assert_eq!(report.state, SyncState::NoUpstream, "{report:?}");
    assert_eq!(report.detail.as_deref(), Some("main"));
    untracked.assert_intact();

    // A new local branch in a clone has no upstream either.
    let world = World::new("sync-new-branch");
    let untracked = Untracked::plant(&world.kb);
    git(&world.kb, &["switch", "-q", "-c", "draft"]);
    commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    let remote = world.remote_main();
    let report = world.sync();
    assert_eq!(report.state, SyncState::NoUpstream, "{report:?}");
    assert_eq!(world.remote_main(), remote);
    untracked.assert_intact();

    // The upstream branch was deleted on the remote.
    git(&world.kb, &["switch", "-q", "main"]);
    git(
        &world.other,
        &["push", "-q", "origin", "HEAD:refs/heads/gone"],
    );
    git(&world.kb, &["fetch", "-q", "origin"]);
    git(
        &world.kb,
        &[
            "switch",
            "-q",
            "-c",
            "follows-gone",
            "--track",
            "origin/gone",
        ],
    );
    git(&world.other, &["push", "-q", "origin", "--delete", "gone"]);
    let report = world.sync();
    assert_eq!(report.state, SyncState::NoUpstream, "{report:?}");
    untracked.assert_intact();
}

#[test]
fn a_detached_head_is_not_synced() {
    let world = World::new("sync-detached");
    let untracked = Untracked::plant(&world.kb);
    world.other_pushes("wiki/概念/乙.md", "# 乙\n", "知识：乙");
    git(&world.kb, &["switch", "-q", "--detach"]);
    let before = head(&world.kb);

    let report = world.sync();
    assert_eq!(report.state, SyncState::Detached, "{report:?}");
    assert_eq!(head(&world.kb), before);
    assert!(!world.kb.join("wiki/概念/乙.md").exists());
    untracked.assert_intact();
}

#[test]
fn an_unfinished_rebase_or_merge_is_left_alone() {
    let world = World::new("sync-in-progress");
    let untracked = Untracked::plant(&world.kb);
    commit(
        &world.kb,
        "self/底色.md",
        "### D01 先确认能退回来\n\n本机\n",
        "底色：本机改",
    );
    world.other_pushes(
        "self/底色.md",
        "### D01 先确认能退回来\n\n远端\n",
        "底色：远端改",
    );
    git(&world.kb, &["fetch", "-q", "origin"]);

    // The person started a rebase in the terminal and it stopped on a conflict.
    assert!(!git_ok(&world.kb, &["rebase", "origin/main"]));
    let during = head(&world.kb);
    let file = read(&world.kb, "self/底色.md");
    let report = world.sync();
    assert_eq!(report.state, SyncState::InProgress, "{report:?}");
    assert_eq!(report.detail.as_deref(), Some("rebase"));
    assert_eq!(head(&world.kb), during);
    assert_eq!(read(&world.kb, "self/底色.md"), file);
    assert!(
        world.kb.join(".git/rebase-merge").exists(),
        "still theirs to finish"
    );
    untracked.assert_intact();

    git(&world.kb, &["rebase", "--abort"]);
    assert!(!git_ok(&world.kb, &["merge", "origin/main"]));
    let report = world.sync();
    assert_eq!(report.state, SyncState::InProgress, "{report:?}");
    assert_eq!(report.detail.as_deref(), Some("merge"));
    assert!(world.kb.join(".git/MERGE_HEAD").exists());
    untracked.assert_intact();
}

// ---------- backups and status ----------

#[test]
fn keeps_only_the_ten_newest_backup_refs() {
    let world = World::new("sync-prune");
    let untracked = Untracked::plant(&world.kb);
    let at = head(&world.kb);
    for seconds in 1_700_000_000u64..1_700_000_013 {
        git(
            &world.kb,
            &["update-ref", &format!("{BACKUP_PREFIX}{seconds}"), &at],
        );
    }
    git(
        &world.kb,
        &["update-ref", &format!("{BACKUP_PREFIX}not-a-number"), &at],
    );
    // Other refs are never touched.
    git(&world.kb, &["update-ref", "refs/dise/other/1", &at]);

    let report = world.sync();
    assert_eq!(report.state, SyncState::Synced, "{report:?}");
    let mut kept = backup_refs(&world.kb);
    kept.sort();
    let expected: Vec<String> = (1_700_000_003u64..1_700_000_013)
        .map(|seconds| format!("{BACKUP_PREFIX}{seconds}"))
        .collect();
    assert_eq!(kept, expected);
    assert!(git_ok(
        &world.kb,
        &["rev-parse", "--verify", "refs/dise/other/1"]
    ));
    untracked.assert_intact();

    assert_eq!(prune_backups(&world.kb, &env(), 3).unwrap(), 7);
    assert_eq!(backup_refs(&world.kb).len(), 3);
    assert_eq!(prune_backups(&world.kb, &env(), 3).unwrap(), 0);
}

#[test]
fn many_rebases_never_pile_up_more_than_ten_backups() {
    let world = World::new("sync-many-backups");
    let untracked = Untracked::plant(&world.kb);
    for round in 0..12 {
        commit(
            &world.kb,
            &format!("inbox/本机-{round}.md"),
            "本机\n",
            "收件箱：记一笔（mac）",
        );
        world.other_pushes(
            &format!("inbox/手机-{round}.md"),
            "手机\n",
            "收件箱：记一笔（iphone）",
        );
        // Old backups as if they were made in earlier seconds.
        if round < 11 {
            git(
                &world.kb,
                &[
                    "update-ref",
                    &format!("{BACKUP_PREFIX}{}", 1_000 + round),
                    "HEAD",
                ],
            );
        }
        let report = world.sync();
        assert_eq!(report.state, SyncState::Synced, "round {round}: {report:?}");
        assert!(
            backup_refs(&world.kb).len() <= KEEP_BACKUPS,
            "round {round}"
        );
    }
    assert_eq!(world.remote_main(), head(&world.kb));
    untracked.assert_intact();
}

#[test]
fn status_reports_branch_upstream_counts_and_tracked_changes() {
    let world = World::new("sync-status");
    let untracked = Untracked::plant(&world.kb);
    let clean = repo_status(&world.kb, &env()).unwrap();
    assert_eq!(
        clean,
        RepoStatus {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            ahead: 0,
            behind: 0,
            dirty: false,
        },
        "untracked files do not make it dirty"
    );

    commit(
        &world.kb,
        "inbox/本机.md",
        "本机\n",
        "收件箱：记一笔（mac）",
    );
    world.other_pushes("inbox/手机.md", "手机\n", "收件箱：记一笔（iphone）");
    git(&world.kb, &["fetch", "-q", "origin"]);
    write(&world.kb, "index.md", "# 目录（改了）\n");
    let status = repo_status(&world.kb, &env()).unwrap();
    assert_eq!((status.ahead, status.behind, status.dirty), (1, 1, true));

    git(&world.kb, &["switch", "-q", "--detach"]);
    let detached = repo_status(&world.kb, &env()).unwrap();
    assert_eq!(detached.branch, None);
    assert_eq!(detached.upstream, None);

    let plain = TempDir::new("sync-status-plain");
    assert!(repo_status(plain.path(), &env()).is_err());
    untracked.assert_intact();
}

#[test]
fn reports_serialize_as_camel_case_json() {
    let report = SyncReport {
        state: SyncState::NoUpstream,
        detail: Some("main".into()),
        paths: vec!["a.md".into()],
        pulled: 1,
        pushed: 2,
        changed: true,
        backup: None,
    };
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!({
            "state": "noUpstream",
            "detail": "main",
            "paths": ["a.md"],
            "pulled": 1,
            "pushed": 2,
            "changed": true,
            "backup": null,
        })
    );
    assert_eq!(
        serde_json::to_value(RepoStatus::default()).unwrap(),
        serde_json::json!({
            "branch": null, "upstream": null, "ahead": 0, "behind": 0, "dirty": false
        })
    );
}

#[test]
fn file_lists_in_git_messages_are_read() {
    let detail = "error: Your local changes to the following files would be overwritten by merge:\n\tself/底色.md\n\twiki/概念/甲.md\nPlease commit your changes or stash them before you merge.\nAborting";
    assert_eq!(
        listed_paths(detail),
        vec!["self/底色.md", "wiki/概念/甲.md"]
    );
    assert!(listed_paths("fatal: nothing").is_empty());
    let long = "错".repeat(MAX_DETAIL + 10);
    assert!(shorten(&long).ends_with('…'));
    assert_eq!(shorten("  short \n"), "short");
}
