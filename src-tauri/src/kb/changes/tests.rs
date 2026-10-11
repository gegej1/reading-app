use super::*;
use crate::kb::ops;
use crate::kb::test_support::{commit_count, git, init_repo, symlink, TempDir};
use std::os::unix::fs::PermissionsExt;

const CAPTURE_A: &str = "inbox/2026-10-12T09-00-00-mac.md";
const CAPTURE_B: &str = "inbox/2026-10-12T09-05-00-mac.md";
const ANSWER: &str = "inbox/裁决/2026-10-12T10-00-00-mac.md";
const RAW_OLD: &str = "raw/inbox/2026-10-11-收件箱.md";
const RAW_NEW: &str = "raw/inbox/2026-10-12-收件箱.md";
const WIKI: &str = "wiki/概念/小实验.md";

fn env() -> ChildEnv {
    ChildEnv::inherit()
}

fn capture_text(body: &str) -> String {
    format!("---\n来源: mac\n时间: 2026-10-12T09:00:00+08:00\n---\n\n{body}\n")
}

/// A small synthetic knowledge base with two captures and one answer queued,
/// plus two files of the user that git does not know.
fn kb(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    init_repo(dir.path());
    dir.write(".gitignore", ".DS_Store\n*.pdf\n");
    dir.write("index.md", "# 总目录\n");
    dir.write("log.md", "# 操作日志\n");
    dir.write("README.md", "# 说明\n");
    dir.write("self/底色.md", "### D01 示例 ✅\n> 原话\n");
    dir.write("self/旧的.md", "# 旧的\n");
    dir.write(RAW_OLD, "# 2026-10-11 收件箱\n\n## I-01\n第一条。\n");
    dir.write(WIKI, "# 小实验\n");
    dir.write("inbox/.gitkeep", "");
    dir.write("inbox/裁决/.gitkeep", "");
    dir.write(CAPTURE_A, &capture_text("周五做一次周回顾。"));
    dir.write(CAPTURE_B, &capture_text("清单越短越好。"));
    dir.write(ANSWER, "---\n对象: Q-004\n---\n\n先不统一。\n");
    git(dir.path(), &["add", "--all"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    dir.write("草稿.md", "我自己的草稿，没有加入 git\n");
    dir.write("私人/笔记.md", "也没有加入 git\n");
    dir
}

fn status(dir: &TempDir) -> String {
    git(
        dir.path(),
        &["-c", "core.quotePath=false", "status", "--porcelain"],
    )
}

fn committed_files(dir: &TempDir) -> Vec<String> {
    let listed = git(
        dir.path(),
        &[
            "-c",
            "core.quotePath=false",
            "show",
            "--name-status",
            "--format=",
            "HEAD",
        ],
    );
    let mut files: Vec<String> = listed.lines().map(|line| line.replace('\t', " ")).collect();
    files.sort();
    files
}

fn read(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).unwrap()
}

fn paths(changes: &Changes) -> Vec<(&str, FileStatus)> {
    changes
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.status))
        .collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn snapshot_records_the_commit_the_queue_and_what_git_does_not_know() {
    let kb = kb("changes-snapshot");
    kb.write("inbox/裁决/子目录/不算.md", "不在队列里\n");
    kb.write("inbox/笔记.txt", "不是 Markdown\n");
    kb.write("inbox/手放的.md", "手动放进来的一条\n");

    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    assert_eq!(snap.base, git(kb.path(), &["rev-parse", "HEAD"]));
    assert!(!snap.branch.is_empty());
    let queue: Vec<(&str, bool)> = snap
        .queue
        .iter()
        .map(|file| (file.path.as_str(), file.tracked))
        .collect();
    assert_eq!(
        queue,
        [
            (CAPTURE_A, true),
            (CAPTURE_B, true),
            ("inbox/手放的.md", false)
        ]
    );
    assert_eq!(snap.queue[0].content, capture_text("周五做一次周回顾。"));
    // The user's own files, but not the queue file that was dropped in by hand.
    let untracked: Vec<&str> = snap.untracked.keys().map(String::as_str).collect();
    assert_eq!(
        untracked,
        [
            "inbox/笔记.txt",
            "inbox/裁决/子目录/不算.md",
            "私人/笔记.md",
            "草稿.md"
        ]
    );
    for dir in ["inbox", "inbox/裁决", "raw/inbox", "wiki/概念", "私人"] {
        assert!(snap.dirs.contains(dir), "{dir}");
    }
    assert!(!snap.dirs.iter().any(|dir| dir.starts_with(".git")));

    let answers = snapshot(kb.path(), &env(), "inbox/裁决").unwrap();
    assert_eq!(answers.queue.len(), 1);
    assert_eq!(answers.queue[0].path, ANSWER);
    // The round trip to disk and back keeps everything.
    let json = serde_json::to_string(&snap).unwrap();
    assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snap);
}

#[test]
fn snapshot_of_an_empty_or_missing_queue_is_empty() {
    let kb = kb("changes-snapshot-empty");
    git(kb.path(), &["rm", "-q", "--", CAPTURE_A, CAPTURE_B]);
    git(kb.path(), &["commit", "-q", "-m", "empty"]);
    assert!(snapshot(kb.path(), &env(), "inbox").unwrap().queue.is_empty());
    assert!(snapshot(kb.path(), &env(), "inbox/没有这个目录")
        .unwrap()
        .queue
        .is_empty());
}

#[test]
fn snapshot_refuses_a_tree_that_is_not_clean_or_not_on_a_branch() {
    let kb = kb("changes-snapshot-dirty");
    kb.write(WIKI, "# 小实验\n改了但没提交\n");
    let error = snapshot(kb.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains("还没提交的改动"), "{error}");
    assert!(error.contains(WIKI), "{error}");

    git(kb.path(), &["add", "--", WIKI]);
    let error = snapshot(kb.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains(WIKI), "staged counts too: {error}");
    git(kb.path(), &["commit", "-q", "-m", "wiki"]);
    assert!(snapshot(kb.path(), &env(), "inbox").is_ok());

    git(kb.path(), &["checkout", "-q", "--detach"]);
    let error = snapshot(kb.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains("分支"), "{error}");

    let plain = TempDir::new("changes-snapshot-plain");
    let error = snapshot(plain.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains("不是 git 仓库"), "{error}");

    let empty = TempDir::new("changes-snapshot-norev");
    init_repo(empty.path());
    let error = snapshot(empty.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains("还没有任何提交"), "{error}");
}

#[test]
fn snapshot_refuses_queue_files_it_could_not_check_verbatim() {
    let kb = kb("changes-snapshot-utf8");
    fs::write(kb.path().join("inbox/坏的.md"), [0xff, 0xfe, 0x00]).unwrap();
    let error = snapshot(kb.path(), &env(), "inbox").unwrap_err();
    assert!(error.contains("inbox/坏的.md"), "{error}");
}

#[test]
fn collect_lists_what_the_run_changed_and_nothing_of_the_users() {
    let kb = kb("changes-collect");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    assert!(collect(kb.path(), &env(), &snap, &[]).unwrap().is_empty());

    // What an engine does: edit, append, create (any extension), delete.
    kb.write(WIKI, "# 小实验\n\n新的一段。\n");
    kb.write(RAW_OLD, "# 2026-10-11 收件箱\n\n## I-01\n第一条。\n\n## I-02\n追加。\n");
    kb.write(RAW_NEW, "# 2026-10-12 收件箱\n");
    kb.write("raw/附件/说明.txt", "不是 Markdown 也算改动\n");
    fs::remove_file(kb.path().join("self/旧的.md")).unwrap();
    // Not the run's: an ignored file, and the user's files stay as they are.
    kb.write(".DS_Store", "finder");
    kb.write("raw/扫描.pdf", "ignored");

    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(
        paths(&changes),
        [
            (RAW_OLD, FileStatus::Modified),
            (RAW_NEW, FileStatus::Added),
            ("raw/附件/说明.txt", FileStatus::Added),
            ("self/旧的.md", FileStatus::Deleted),
            (WIKI, FileStatus::Modified),
        ]
    );
    assert!(changes.foreign_commits.is_empty());
    assert!(changes.untracked_changed.is_empty());

    let wiki = &changes.files[4];
    assert_eq!(wiki.before.text(), Some("# 小实验\n"));
    assert_eq!(wiki.after.text(), Some("# 小实验\n\n新的一段。\n"));
    assert_eq!(wiki.origin, Origin::Snapshot);
    let created = &changes.files[1];
    assert_eq!(created.before, Content::Absent);
    assert_eq!(created.origin, Origin::New);
    let deleted = &changes.files[3];
    assert_eq!(deleted.before.text(), Some("# 旧的\n"));
    assert_eq!(deleted.after, Content::Absent);

    // The same changes give the same fingerprint; any further edit another.
    let again = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(again.fingerprint(), changes.fingerprint());
    kb.write(RAW_NEW, "# 2026-10-12 收件箱\n又改了\n");
    let later = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_ne!(later.fingerprint(), changes.fingerprint());
}

#[test]
fn collect_reports_changes_to_files_git_never_knew() {
    let kb = kb("changes-collect-untracked");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    kb.write("草稿.md", "被改写了\n");
    fs::remove_file(kb.path().join("私人/笔记.md")).unwrap();
    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert!(changes.files.is_empty(), "{:?}", paths(&changes));
    assert_eq!(changes.untracked_changed, ["私人/笔记.md", "草稿.md"]);
    assert!(!changes.is_empty());
}

#[test]
fn collect_leaves_out_captures_the_app_committed_meanwhile() {
    let kb = kb("changes-collect-capture");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    let fresh = "inbox/2026-10-12T09-30-00-mac.md";
    let outcome = ops::create_file(
        kb.path(),
        &env(),
        fresh,
        &capture_text("运行中记下的一条。"),
        "收件箱：记一笔（mac）",
    )
    .unwrap();
    let noted = vec![NotedCommit {
        commit: outcome.commit.clone().unwrap(),
        path: fresh.to_string(),
    }];

    kb.write(RAW_NEW, "# 2026-10-12 收件箱\n");
    let changes = collect(kb.path(), &env(), &snap, &noted).unwrap();
    assert_eq!(paths(&changes), [(RAW_NEW, FileStatus::Added)]);
    assert!(changes.foreign_commits.is_empty());

    // Nobody told the run about that commit: then it is somebody else's.
    let unknown = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(unknown.foreign_commits.len(), 1);
    assert_eq!(unknown.foreign_commits[0].hash, outcome.commit.unwrap());
    assert_eq!(unknown.foreign_commits[0].subject, "收件箱：记一笔（mac）");
    assert_eq!(
        paths(&unknown),
        [(fresh, FileStatus::Added), (RAW_NEW, FileStatus::Added)]
    );

    // The engine may not touch a fresh capture either.
    kb.write(fresh, "被引擎改了\n");
    let tampered = collect(kb.path(), &env(), &snap, &noted).unwrap();
    assert_eq!(
        paths(&tampered),
        [(fresh, FileStatus::Modified), (RAW_NEW, FileStatus::Added)]
    );
    assert_eq!(tampered.files[0].origin, Origin::Capture);
    assert_eq!(
        tampered.files[0].before.text(),
        Some(capture_text("运行中记下的一条。").as_str())
    );
    fs::remove_file(kb.path().join(fresh)).unwrap();
    let removed = collect(kb.path(), &env(), &snap, &noted).unwrap();
    assert_eq!(removed.files[0].status, FileStatus::Deleted);
}

#[test]
fn collect_reports_commits_nobody_was_supposed_to_make() {
    let kb = kb("changes-collect-foreign");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    kb.write(RAW_NEW, "# 2026-10-12 收件箱\n");
    git(kb.path(), &["add", "--", RAW_NEW]);
    git(kb.path(), &["commit", "-q", "-m", "入库（引擎自己提交的）"]);
    kb.write(WIKI, "# 小实验\n之后又改了\n");

    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(changes.foreign_commits.len(), 1);
    assert_eq!(changes.foreign_commits[0].subject, "入库（引擎自己提交的）");
    // What it committed still counts as a change of the run.
    assert_eq!(
        paths(&changes),
        [(RAW_NEW, FileStatus::Added), (WIKI, FileStatus::Modified)]
    );
}

#[test]
fn collect_sees_queue_files_the_engine_removed_or_edited() {
    let kb = kb("changes-collect-queue");
    kb.write("inbox/手放的.md", "手动放进来的一条\n");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();

    fs::remove_file(kb.path().join(CAPTURE_A)).unwrap();
    fs::remove_file(kb.path().join("inbox/手放的.md")).unwrap();
    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(
        paths(&changes),
        [
            (CAPTURE_A, FileStatus::Deleted),
            ("inbox/手放的.md", FileStatus::Deleted)
        ]
    );
    assert_eq!(changes.files[0].origin, Origin::Snapshot);
    assert_eq!(changes.files[1].origin, Origin::Queue);
    assert_eq!(changes.files[1].before.text(), Some("手动放进来的一条\n"));

    kb.write("inbox/手放的.md", "被改了\n");
    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(changes.files[1].status, FileStatus::Modified);
}

#[test]
fn collect_does_not_read_through_symlinks_and_keeps_binary_files_as_bytes() {
    let kb = kb("changes-collect-kinds");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    let outside = TempDir::new("changes-collect-outside");
    let secret = outside.write("secret.txt", "不属于知识库的内容\n");
    symlink(&secret, &kb.path().join("wiki/链接.md"));
    fs::write(kb.path().join("raw/图.bin"), [0u8, 159, 146, 150]).unwrap();

    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(
        paths(&changes),
        [
            ("raw/图.bin", FileStatus::Added),
            ("wiki/链接.md", FileStatus::Added)
        ]
    );
    assert_eq!(changes.files[0].after.text(), None);
    assert!(changes.files[0].after.bytes().is_some());
    assert_eq!(changes.files[1].after, Content::Symlink);
}

#[test]
fn reads_a_file_as_of_a_commit() {
    let kb = kb("changes-read-at");
    kb.write(WIKI, "# 小实验\n工作区里改了\n");
    assert_eq!(
        read_at(kb.path(), &env(), "HEAD", WIKI).unwrap().as_deref(),
        Some("# 小实验\n".as_bytes())
    );
    assert_eq!(read_at(kb.path(), &env(), "HEAD", "wiki/没有.md").unwrap(), None);
    assert!(read_at(kb.path(), &env(), "HEAD", "../外面.md").is_err());
    assert!(read_at(kb.path(), &env(), "HEAD", ".git/config").is_err());
}

#[test]
fn accepts_any_extension_but_no_path_out_of_the_knowledge_base() {
    for good in ["raw/附件/说明.txt", "index.md", ".obsidian/app.json", "a b/c"] {
        assert!(check_rel_path(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "/etc/passwd",
        "../x.md",
        "wiki/../../x.md",
        "wiki//x.md",
        "./x.md",
        ".git/config",
        "wiki/.git/x",
        "a\\b.md",
        "a\0b",
    ] {
        assert!(check_rel_path(bad).is_err(), "{bad:?}");
    }
}

/// The run of the happy path: both captures copied into raw/, a wiki page
/// updated, and one file changed that nobody approved.
fn engine_ran(kb: &TempDir) {
    kb.write(
        RAW_NEW,
        "# 2026-10-12 收件箱\n\n## I-01\n周五做一次周回顾。\n\n## I-02\n清单越短越好。\n",
    );
    kb.write(WIKI, "# 小实验\n\n出处：[[2026-10-12-收件箱]]\n");
    kb.write("README.md", "# 说明\n\n没人批准的改动。\n");
}

#[test]
fn commit_run_makes_one_commit_of_exactly_the_plan() {
    let kb = kb("changes-commit");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    let before = commit_count(kb.path());

    let commit = commit_run(
        kb.path(),
        &env(),
        &snap,
        CommitPlan {
            paths: &strings(&[RAW_NEW, WIKI]),
            delete_queue: &strings(&[CAPTURE_A, CAPTURE_B]),
            restore_queue: &[],
            message: "入库：2 条（Claude）",
        },
    )
    .unwrap();

    assert_eq!(commit, git(kb.path(), &["rev-parse", "HEAD"]));
    assert_eq!(commit_count(kb.path()), before + 1);
    assert_eq!(
        git(kb.path(), &["log", "-1", "--format=%s"]),
        "入库：2 条（Claude）"
    );
    assert_eq!(
        committed_files(&kb),
        [
            format!("A {RAW_NEW}"),
            format!("D {CAPTURE_A}"),
            format!("D {CAPTURE_B}"),
            format!("M {WIKI}"),
        ]
    );
    assert!(!kb.path().join(CAPTURE_A).exists());
    assert!(!kb.path().join(CAPTURE_B).exists());
    // The change nobody approved and the user's files are exactly where they were.
    assert_eq!(status(&kb), "M README.md\n?? 私人/\n?? 草稿.md");
    assert_eq!(read(&kb, "草稿.md"), "我自己的草稿，没有加入 git\n");
}

#[test]
fn commit_run_puts_back_queue_files_that_were_not_copied() {
    let kb = kb("changes-commit-restore");
    kb.write("inbox/手放的.md", "手动放进来的一条\n");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    // The engine deleted everything in the queue, copied or not.
    fs::remove_file(kb.path().join(CAPTURE_A)).unwrap();
    fs::remove_file(kb.path().join(CAPTURE_B)).unwrap();
    fs::remove_file(kb.path().join("inbox/手放的.md")).unwrap();

    commit_run(
        kb.path(),
        &env(),
        &snap,
        CommitPlan {
            paths: &strings(&[RAW_NEW]),
            delete_queue: &strings(&[CAPTURE_A]),
            restore_queue: &strings(&[CAPTURE_B, "inbox/手放的.md"]),
            message: "入库：1 条（Codex）",
        },
    )
    .unwrap();

    assert_eq!(
        committed_files(&kb),
        [format!("A {RAW_NEW}"), format!("D {CAPTURE_A}")]
    );
    assert_eq!(read(&kb, CAPTURE_B), capture_text("清单越短越好。"));
    assert_eq!(read(&kb, "inbox/手放的.md"), "手动放进来的一条\n");
    // CAPTURE_B is as committed again; the hand-made file is still untracked.
    assert_eq!(
        status(&kb),
        format!("M README.md\n M {WIKI}\n?? inbox/手放的.md\n?? 私人/\n?? 草稿.md")
    );
}

#[test]
fn commit_run_removes_processed_queue_files_git_never_knew() {
    let kb = kb("changes-commit-untracked-queue");
    kb.write("inbox/手放的.md", "手动放进来的一条\n");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    kb.write(RAW_NEW, "# 2026-10-12 收件箱\n\n手动放进来的一条\n");

    commit_run(
        kb.path(),
        &env(),
        &snap,
        CommitPlan {
            paths: &strings(&[RAW_NEW]),
            delete_queue: &strings(&["inbox/手放的.md"]),
            restore_queue: &[],
            message: "入库：1 条（Claude）",
        },
    )
    .unwrap();
    assert_eq!(committed_files(&kb), [format!("A {RAW_NEW}")]);
    assert!(!kb.path().join("inbox/手放的.md").exists());
}

#[test]
fn commit_run_leaves_the_run_as_it_was_when_git_refuses() {
    let kb = kb("changes-commit-hook");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    let hook = kb.write(
        ".git/hooks/pre-commit",
        "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n",
    );
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let before = commit_count(kb.path());

    let error = commit_run(
        kb.path(),
        &env(),
        &snap,
        CommitPlan {
            paths: &strings(&[RAW_NEW, WIKI]),
            delete_queue: &strings(&[CAPTURE_A, CAPTURE_B]),
            restore_queue: &[],
            message: "入库：2 条（Claude）",
        },
    )
    .unwrap_err();
    assert!(error.contains("hook says no"), "{error}");
    assert_eq!(commit_count(kb.path()), before);
    // Nothing staged, the queue files are back, the run's edits are still there.
    assert_eq!(git(kb.path(), &["diff", "--cached", "--name-only"]), "");
    assert_eq!(read(&kb, CAPTURE_A), capture_text("周五做一次周回顾。"));
    assert_eq!(read(&kb, CAPTURE_B), capture_text("清单越短越好。"));
    assert_eq!(
        status(&kb),
        format!("M README.md\n M {WIKI}\n?? {RAW_NEW}\n?? 私人/\n?? 草稿.md")
    );

    // Once commits work again, the same plan goes through.
    fs::remove_file(&hook).unwrap();
    let again = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(again.files.len(), 3);
    commit_run(
        kb.path(),
        &env(),
        &snap,
        CommitPlan {
            paths: &strings(&[RAW_NEW, WIKI]),
            delete_queue: &strings(&[CAPTURE_A, CAPTURE_B]),
            restore_queue: &[],
            message: "入库：2 条（Claude）",
        },
    )
    .unwrap();
    assert_eq!(commit_count(kb.path()), before + 1);
}

#[test]
fn commit_run_checks_the_plan_before_touching_anything() {
    let kb = kb("changes-commit-plan");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    let before = status(&kb);
    let plan = |paths: &[&str], delete: &[&str], message: &str| {
        commit_run(
            kb.path(),
            &env(),
            &snap,
            CommitPlan {
                paths: &strings(paths),
                delete_queue: &strings(delete),
                restore_queue: &[],
                message,
            },
        )
        .unwrap_err()
    };
    assert!(plan(&["../外面.md"], &[], "m").contains("不安全"));
    assert!(plan(&[".git/config"], &[], "m").contains("不安全"));
    assert!(plan(&[RAW_NEW], &["index.md"], "m").contains("队列文件"));
    assert!(plan(&[RAW_NEW, CAPTURE_A], &[], "m").contains("队列文件"));
    assert!(plan(&[RAW_NEW], &[CAPTURE_A], "  ").contains("提交说明"));
    assert!(plan(&[], &[], "m").contains("没有可以提交的改动"));
    assert_eq!(status(&kb), before);
    assert!(kb.path().join(CAPTURE_A).exists());
}

#[test]
fn discard_run_puts_everything_back_and_keeps_a_copy() {
    let kb = kb("changes-discard");
    kb.mkdir("wiki/空目录");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    kb.write("wiki/新目录/深一层/新页.md", "# 新页\n");
    fs::remove_file(kb.path().join("self/旧的.md")).unwrap();
    fs::remove_file(kb.path().join(CAPTURE_A)).unwrap();
    // A capture arrived while the engine ran.
    let fresh = "inbox/2026-10-12T09-30-00-mac.md";
    let outcome = ops::create_file(
        kb.path(),
        &env(),
        fresh,
        &capture_text("运行中记下的一条。"),
        "收件箱：记一笔（mac）",
    )
    .unwrap();
    let noted = vec![NotedCommit {
        commit: outcome.commit.unwrap(),
        path: fresh.to_string(),
    }];
    let changes = collect(kb.path(), &env(), &snap, &noted).unwrap();
    assert_eq!(changes.files.len(), 6);
    let commits = commit_count(kb.path());

    let backup = TempDir::new("changes-discard-backup");
    let done = discard_run(
        kb.path(),
        &env(),
        &snap,
        &changes,
        backup.path(),
        "撤回",
    )
    .unwrap();
    assert_eq!(
        done,
        Discarded {
            restored: 4,
            removed: 2,
            revert_commit: None
        }
    );

    // Exactly as before the run: only the user's untracked files show up.
    assert_eq!(status(&kb), "?? 私人/\n?? 草稿.md");
    assert_eq!(commit_count(kb.path()), commits);
    assert_eq!(read(&kb, WIKI), "# 小实验\n");
    assert_eq!(read(&kb, "README.md"), "# 说明\n");
    assert_eq!(read(&kb, "self/旧的.md"), "# 旧的\n");
    assert_eq!(read(&kb, CAPTURE_A), capture_text("周五做一次周回顾。"));
    assert!(!kb.path().join(RAW_NEW).exists());
    assert!(!kb.path().join("wiki/新目录").exists(), "folders the run made are gone");
    assert!(kb.path().join("wiki/空目录").is_dir(), "older empty folders stay");
    assert_eq!(read(&kb, "草稿.md"), "我自己的草稿，没有加入 git\n");
    assert_eq!(read(&kb, "私人/笔记.md"), "也没有加入 git\n");
    assert_eq!(read(&kb, fresh), capture_text("运行中记下的一条。"));

    // What the run had written can be taken from the copy.
    let copy = |rel: &str| fs::read_to_string(backup.path().join(rel)).unwrap();
    assert!(copy(RAW_NEW).contains("周五做一次周回顾。"));
    assert_eq!(copy(WIKI), "# 小实验\n\n出处：[[2026-10-12-收件箱]]\n");
    assert_eq!(copy("README.md"), "# 说明\n\n没人批准的改动。\n");
    assert_eq!(copy("wiki/新目录/深一层/新页.md"), "# 新页\n");
    assert!(!backup.path().join("self/旧的.md").exists(), "a deleted file has no copy");
}

#[test]
fn discard_run_restores_fresh_captures_and_untracked_queue_files() {
    let kb = kb("changes-discard-queue");
    kb.write("inbox/手放的.md", "手动放进来的一条\n");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    let fresh = "inbox/2026-10-12T09-30-00-mac.md";
    let outcome = ops::create_file(
        kb.path(),
        &env(),
        fresh,
        &capture_text("运行中记下的一条。"),
        "收件箱：记一笔（mac）",
    )
    .unwrap();
    let noted = vec![NotedCommit {
        commit: outcome.commit.unwrap(),
        path: fresh.to_string(),
    }];
    kb.write(fresh, "被引擎改了\n");
    kb.write("inbox/手放的.md", "也被改了\n");
    let outside = TempDir::new("changes-discard-outside");
    let target = outside.write("target.txt", "外面的文件\n");
    symlink(&target, &kb.path().join("wiki/链接.md"));

    let changes = collect(kb.path(), &env(), &snap, &noted).unwrap();
    let backup = TempDir::new("changes-discard-queue-backup");
    discard_run(kb.path(), &env(), &snap, &changes, backup.path(), "撤回").unwrap();

    assert_eq!(read(&kb, fresh), capture_text("运行中记下的一条。"));
    assert_eq!(read(&kb, "inbox/手放的.md"), "手动放进来的一条\n");
    assert!(fs::symlink_metadata(kb.path().join("wiki/链接.md")).is_err());
    assert_eq!(read(&outside, "target.txt"), "外面的文件\n", "the link target is untouched");
    assert_eq!(
        fs::read_to_string(backup.path().join("wiki/链接.md.symlink")).unwrap(),
        target.to_string_lossy()
    );
    assert_eq!(status(&kb), "?? inbox/手放的.md\n?? 私人/\n?? 草稿.md");
}

#[test]
fn discard_run_takes_back_commits_the_engine_made_without_rewriting_history() {
    let kb = kb("changes-discard-foreign");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    git(kb.path(), &["add", "--", RAW_NEW, WIKI]);
    git(kb.path(), &["commit", "-q", "-m", "引擎自己提交的"]);
    let engine_commit = git(kb.path(), &["rev-parse", "HEAD"]);
    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    assert_eq!(changes.foreign_commits.len(), 1);

    let backup = TempDir::new("changes-discard-foreign-backup");
    let done = discard_run(
        kb.path(),
        &env(),
        &snap,
        &changes,
        backup.path(),
        "撤回：AI 自己做的提交（入库）",
    )
    .unwrap();

    let revert = done.revert_commit.expect("a commit takes the changes back");
    assert_eq!(revert, git(kb.path(), &["rev-parse", "HEAD"]));
    assert_eq!(git(kb.path(), &["rev-parse", "HEAD^"]), engine_commit);
    assert_eq!(
        git(kb.path(), &["log", "-1", "--format=%s"]),
        "撤回：AI 自己做的提交（入库）"
    );
    // The files are as they were when the run started, and nothing is left over.
    assert_eq!(
        git(kb.path(), &["diff", "--name-only", &snap.base, "HEAD"]),
        ""
    );
    assert_eq!(status(&kb), "?? 私人/\n?? 草稿.md");
    assert!(!kb.path().join(RAW_NEW).exists());
}

#[test]
fn discard_run_discards_nothing_when_the_copy_cannot_be_made() {
    let kb = kb("changes-discard-nobackup");
    let snap = snapshot(kb.path(), &env(), "inbox").unwrap();
    engine_ran(&kb);
    let changes = collect(kb.path(), &env(), &snap, &[]).unwrap();
    let blocked = TempDir::new("changes-discard-blocked");
    // A file where the backup folder should be.
    let not_a_dir = blocked.write("backup", "in the way");

    let error = discard_run(kb.path(), &env(), &snap, &changes, &not_a_dir, "撤回").unwrap_err();
    assert!(error.contains("什么都没有丢弃"), "{error}");
    assert!(read(&kb, RAW_NEW).contains("周五做一次周回顾。"));
    assert_eq!(read(&kb, WIKI), "# 小实验\n\n出处：[[2026-10-12-收件箱]]\n");
}
