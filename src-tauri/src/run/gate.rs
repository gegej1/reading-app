//! What the gate is given and what it answers, and the two rules the app
//! checks again itself before it commits.
//!
//! The gate is TypeScript (`src/domain/gate.ts`): it needs the Markdown,
//! 底色 and wikilink parsers, which exist only there. The main webview runs
//! it; this module builds its input from the run's changes, reads its
//! report, makes sure the report is about exactly these changes, and repeats
//! the two cheapest rules (allowed paths, `raw/` only grows) as defence in
//! depth, so a broken or misled frontend can never get a commit past them.
//!
//! The field names are shared with `src/domain/gate.ts`; both sides are
//! tested against `src/domain/__fixtures__/gate-example.json`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Task;
use crate::kb::changes::{
    ChangedFile, Changes, CommitInfo, Content, FileStatus, Snapshot, MAX_FILE_BYTES,
};
use crate::kb::files;

/// The folders an AI run may change (AGENTS.md section 九).
pub const ALLOWED_FOLDERS: [&str; 5] = ["raw/", "wiki/", "systems/", "self/", "reports/"];
/// The two pages at the top it may change as well.
pub const ALLOWED_FILES: [&str; 2] = ["index.md", "log.md"];
const RAW: &str = "raw/";
const SYSTEMS: &str = "systems/";

/// Why a changed file cannot be compared as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Opaque {
    Binary,
    Symlink,
    Large,
}

/// One changed file as the gate sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateFile {
    pub path: String,
    pub status: FileStatus,
    /// Text before the run; `None` when it did not exist or is opaque.
    pub before: Option<String>,
    /// Text after the run; `None` when it is gone or opaque.
    pub after: Option<String>,
    pub opaque: Option<Opaque>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateDoc {
    pub path: String,
    pub content: String,
}

/// Everything the gate needs to check one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateInput {
    pub run_id: String,
    /// Which check of this run it is (a run can be checked again).
    pub attempt: u32,
    pub task: Task,
    /// Local date of the run, `YYYY-MM-DD`.
    pub date: String,
    pub changes: Vec<GateFile>,
    /// Every other Markdown file of the knowledge base, as it is now.
    pub unchanged: Vec<GateDoc>,
    /// The queue files as they were when the run started.
    pub queue: Vec<GateDoc>,
    pub final_message: Option<String>,
    /// Why the engine did not finish normally; `None` when it did.
    pub engine_failure: Option<String>,
    pub foreign_commits: Vec<CommitInfo>,
    pub untracked_changed: Vec<String>,
}

/// One problem the gate found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateIssue {
    pub rule: String,
    pub path: Option<String>,
    pub message: String,
}

/// A change that breaks no rule but touches the user's own layer (the 底色
/// or `systems/`): it is only committed once the user agreed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateApproval {
    /// `diseEntry`, `diseDraft`, `diseFrame` or `systems`.
    pub rule: String,
    pub path: String,
    /// The 底色 entry (`D09`), when it is about one.
    pub entry: Option<String>,
    pub message: String,
    /// The part that changed, before and after.
    pub before: Option<String>,
    pub after: Option<String>,
}

/// The gate's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateReport {
    /// No violations and nothing to approve.
    pub ok: bool,
    pub violations: Vec<GateIssue>,
    pub warnings: Vec<GateIssue>,
    /// What waits for the user's approval (older reports have none).
    #[serde(default)]
    pub approvals: Vec<GateApproval>,
    pub commit_paths: Vec<String>,
    pub delete_queue_paths: Vec<String>,
    pub restore_queue_paths: Vec<String>,
    pub summary: String,
    /// The engine's final JSON line, when it gave one.
    #[serde(rename = "final")]
    pub final_line: Option<Value>,
}

/// The text of one side of a change, or why it is not text.
fn side(content: &Content) -> (Option<String>, Option<Opaque>) {
    match content {
        Content::Absent => (None, None),
        Content::Bytes(_) => match content.text() {
            Some(text) => (Some(text.to_string()), None),
            None => (None, Some(Opaque::Binary)),
        },
        Content::Symlink => (None, Some(Opaque::Symlink)),
        Content::Large => (None, Some(Opaque::Large)),
        Content::Other => (None, Some(Opaque::Binary)),
    }
}

impl From<&ChangedFile> for GateFile {
    fn from(file: &ChangedFile) -> Self {
        let (before, before_opaque) = side(&file.before);
        let (after, after_opaque) = side(&file.after);
        // A link wins: it is the reason nothing was read.
        let opaque = [after_opaque, before_opaque]
            .into_iter()
            .flatten()
            .max_by_key(|opaque| *opaque == Opaque::Symlink);
        GateFile {
            path: file.path.clone(),
            status: file.status,
            before: if opaque.is_some() { None } else { before },
            after: if opaque.is_some() { None } else { after },
            opaque,
        }
    }
}

/// Every Markdown file of the knowledge base that is not among `changes`.
/// Files that cannot be read as text are left out.
pub fn unchanged_docs(root: &Path, changes: &Changes) -> Result<Vec<GateDoc>, String> {
    let changed: BTreeSet<&str> = changes.files.iter().map(|file| file.path.as_str()).collect();
    let listed =
        files::list_markdown(root).map_err(|error| format!("读取知识库目录失败：{error}"))?;
    Ok(listed
        .into_iter()
        .filter(|file| !changed.contains(file.path.as_str()))
        .filter_map(|file| {
            let path = root.join(&file.path);
            let small = fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_FILE_BYTES as u64);
            let content = small.then(|| fs::read_to_string(&path).ok()).flatten()?;
            Some(GateDoc {
                path: file.path,
                content,
            })
        })
        .collect())
}

/// What a check is about, besides the files.
pub struct RunFacts<'a> {
    pub run_id: &'a str,
    pub attempt: u32,
    pub task: Task,
    pub date: &'a str,
    pub final_message: Option<&'a str>,
    pub engine_failure: Option<&'a str>,
}

/// Puts the gate's input together.
pub fn build_input(
    facts: &RunFacts<'_>,
    snapshot: &Snapshot,
    changes: &Changes,
    unchanged: Vec<GateDoc>,
) -> GateInput {
    GateInput {
        run_id: facts.run_id.to_string(),
        attempt: facts.attempt,
        task: facts.task,
        date: facts.date.to_string(),
        changes: changes.files.iter().map(GateFile::from).collect(),
        unchanged,
        queue: snapshot
            .queue
            .iter()
            .map(|file| GateDoc {
                path: file.path.clone(),
                content: file.content.clone(),
            })
            .collect(),
        final_message: facts.final_message.map(String::from),
        engine_failure: facts.engine_failure.map(String::from),
        foreign_commits: changes.foreign_commits.clone(),
        untracked_changed: changes.untracked_changed.clone(),
    }
}

/// True for the paths an AI run may change.
pub fn allowed_path(path: &str) -> bool {
    ALLOWED_FILES.contains(&path)
        || ALLOWED_FOLDERS
            .iter()
            .any(|folder| path.starts_with(folder) && path.len() > folder.len())
}

/// How `file` breaks "raw/ only grows at the end", if it does.
pub fn raw_problem(file: &ChangedFile) -> Option<String> {
    if !file.path.starts_with(RAW) || file.status == FileStatus::Added {
        return None;
    }
    if file.status == FileStatus::Deleted {
        return Some(format!(
            "删除了原料 {}（原料只增不改，也不能改名）",
            file.path
        ));
    }
    match (file.before.bytes(), file.after.bytes()) {
        (Some(before), Some(after)) if after.starts_with(before) => None,
        (Some(_), Some(_)) => Some(format!(
            "原料 {} 的原有内容被改动了（只能在末尾追加）",
            file.path
        )),
        _ => Some(format!("没法核对原料 {} 是不是只在末尾追加", file.path)),
    }
}

/// The two rules the app checks itself, whatever the gate said: only the
/// allowed paths changed, and `raw/` files only grew at the end.
pub fn own_violations(changes: &Changes, snapshot: &Snapshot) -> Vec<GateIssue> {
    let queued: BTreeSet<&str> = snapshot
        .queue
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let mut issues = Vec::new();
    for file in &changes.files {
        if queued.contains(file.path.as_str()) {
            continue;
        }
        if !allowed_path(&file.path) {
            issues.push(GateIssue {
                rule: "paths".into(),
                path: Some(file.path.clone()),
                message: format!(
                    "改动了不允许改的文件：{}（只能改 raw/、wiki/、systems/、self/、reports/ 和 index.md、log.md）",
                    file.path
                ),
            });
        }
        if let Some(message) = raw_problem(file) {
            issues.push(GateIssue {
                rule: "rawAppendOnly".into(),
                path: Some(file.path.clone()),
                message,
            });
        }
    }
    issues
}

/// The changes the app itself always asks the user about, whatever the gate
/// said: everything under `systems/` (AGENTS.md section 九).
pub fn own_approvals(changes: &Changes, snapshot: &Snapshot) -> Vec<GateApproval> {
    let queued: BTreeSet<&str> = snapshot
        .queue
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    changes
        .files
        .iter()
        .filter(|file| file.path.starts_with(SYSTEMS) && !queued.contains(file.path.as_str()))
        .map(|file| {
            let what = match file.status {
                FileStatus::Added => "新建了",
                FileStatus::Deleted => "删除了",
                FileStatus::Modified => "改了",
            };
            GateApproval {
                rule: "systems".into(),
                path: file.path.clone(),
                entry: None,
                message: format!(
                    "{what}体系文件 {}：体系和机制以你为准，改动要你确认",
                    file.path
                ),
                before: None,
                after: None,
            }
        })
        .collect()
}

/// Makes sure a report is about exactly these changes: it may only name
/// changed files and this run's queue files, and a report that says "ok"
/// must account for every change. A report that fails this is not used.
pub fn validate_report(
    report: &GateReport,
    changes: &Changes,
    snapshot: &Snapshot,
) -> Result<(), String> {
    let queued: BTreeSet<&str> = snapshot
        .queue
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let changed: BTreeSet<&str> = changes.files.iter().map(|file| file.path.as_str()).collect();
    let commit: BTreeSet<&str> = report.commit_paths.iter().map(String::as_str).collect();
    let delete: BTreeSet<&str> = report
        .delete_queue_paths
        .iter()
        .map(String::as_str)
        .collect();
    let restore: BTreeSet<&str> = report
        .restore_queue_paths
        .iter()
        .map(String::as_str)
        .collect();

    if report.ok != (report.violations.is_empty() && report.approvals.is_empty()) {
        return Err("检查结果自相矛盾（通过与否和问题清单对不上）".into());
    }
    if let Some(item) = report
        .approvals
        .iter()
        .find(|item| !changed.contains(item.path.as_str()) || queued.contains(item.path.as_str()))
    {
        return Err(format!("检查结果要你确认一个不在改动里的文件：{}", item.path));
    }
    if let Some(path) = commit
        .iter()
        .find(|path| !changed.contains(*path) || queued.contains(*path))
    {
        return Err(format!("检查结果要提交一个不在改动里的文件：{path}"));
    }
    if let Some(path) = delete
        .iter()
        .chain(&restore)
        .find(|path| !queued.contains(*path))
    {
        return Err(format!("检查结果提到了不属于这次队列的文件：{path}"));
    }
    if let Some(path) = delete.intersection(&restore).next() {
        return Err(format!("检查结果对同一个队列文件既要删除又要放回：{path}"));
    }
    // Without violations the report is what gets committed, now or once the
    // user agreed: it has to account for every change.
    if report.violations.is_empty() {
        if let Some(path) = changed
            .iter()
            .find(|path| !queued.contains(*path) && !commit.contains(*path))
        {
            return Err(format!("检查结果说通过，却漏了一个改动：{path}"));
        }
        if let Some(path) = queued.iter().find(|path| !delete.contains(*path)) {
            return Err(format!(
                "检查结果说通过，却没有确认这个队列文件已经进了 raw/：{path}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::changes::{Origin, QueueFile};
    use std::collections::{BTreeMap, BTreeSet as Set};

    fn file(path: &str, before: Option<&str>, after: Option<&str>) -> ChangedFile {
        let content = |text: Option<&str>| match text {
            Some(text) => Content::Bytes(text.as_bytes().to_vec()),
            None => Content::Absent,
        };
        ChangedFile {
            path: path.to_string(),
            status: match (before, after) {
                (None, _) => FileStatus::Added,
                (_, None) => FileStatus::Deleted,
                _ => FileStatus::Modified,
            },
            before: content(before),
            after: content(after),
            origin: if before.is_some() {
                Origin::Snapshot
            } else {
                Origin::New
            },
        }
    }

    fn snapshot(queue: &[&str]) -> Snapshot {
        Snapshot {
            base: "0".repeat(40),
            branch: "main".into(),
            queue: queue
                .iter()
                .map(|path| QueueFile {
                    path: path.to_string(),
                    content: "---\n来源: mac\n---\n\n一条\n".into(),
                    tracked: true,
                })
                .collect(),
            untracked: BTreeMap::new(),
            dirs: Set::new(),
        }
    }

    fn changes(files: Vec<ChangedFile>) -> Changes {
        Changes {
            files,
            ..Changes::default()
        }
    }

    fn report(ok: bool, commit: &[&str], delete: &[&str], restore: &[&str]) -> GateReport {
        let list = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
        GateReport {
            ok,
            violations: if ok {
                Vec::new()
            } else {
                vec![GateIssue {
                    rule: "queueVerbatim".into(),
                    path: None,
                    message: "有问题".into(),
                }]
            },
            warnings: Vec::new(),
            approvals: Vec::new(),
            commit_paths: list(commit),
            delete_queue_paths: list(delete),
            restore_queue_paths: list(restore),
            summary: String::new(),
            final_line: None,
        }
    }

    const QUEUED: &str = "inbox/2026-10-12T09-00-00-mac.md";

    #[test]
    fn allows_the_same_paths_as_the_typescript_gate() {
        for path in [
            "raw/inbox/2026-10-12-收件箱.md",
            "wiki/概念/新概念.md",
            "systems/周节奏.md",
            "self/判例/P-009-新判例.md",
            "reports/2026-10-12-入库报告.md",
            "index.md",
            "log.md",
        ] {
            assert!(allowed_path(path), "{path}");
        }
        for path in [
            "AGENTS.md",
            "CLAUDE.md",
            "README.md",
            ".gitignore",
            ".git/config",
            "inbox/2026-10-12T09-00-00-mac.md",
            "inbox/裁决/a.md",
            "rawx/a.md",
            "raw",
            "raw/",
            "wiki",
            "notes/index.md",
            "self",
        ] {
            assert!(!allowed_path(path), "{path}");
        }
    }

    #[test]
    fn raw_files_may_only_grow_at_the_end() {
        let raw = "raw/inbox/2026-10-11-收件箱.md";
        assert_eq!(raw_problem(&file(raw, None, Some("新文件\n"))), None);
        assert_eq!(
            raw_problem(&file(raw, Some("第一条\n"), Some("第一条\n第二条\n"))),
            None
        );
        assert_eq!(raw_problem(&file(raw, Some("一\n"), Some("一\n"))), None);
        let rewritten = raw_problem(&file(raw, Some("第一条\n"), Some("改过的第一条\n第二条\n")));
        assert!(rewritten.unwrap().contains("末尾追加"));
        let shorter = raw_problem(&file(raw, Some("第一条\n第二条\n"), Some("第一条\n")));
        assert!(shorter.is_some());
        let deleted = raw_problem(&file(raw, Some("第一条\n"), None));
        assert!(deleted.unwrap().contains("删除了原料"));
        // What cannot be compared is not waved through.
        let mut link = file(raw, Some("第一条\n"), Some("x"));
        link.after = Content::Symlink;
        assert!(raw_problem(&link).is_some());
        // Other folders are none of this rule's business.
        assert_eq!(
            raw_problem(&file("wiki/a.md", Some("旧\n"), Some("全新\n"))),
            None
        );
    }

    #[test]
    fn the_app_finds_forbidden_paths_and_rewritten_raw_files_itself() {
        let found = own_violations(
            &changes(vec![
                file("README.md", Some("a\n"), Some("b\n")),
                file("raw/a.md", Some("旧\n"), Some("新\n")),
                file("wiki/概念/甲.md", None, Some("# 甲\n")),
                // A queue file is the gate's business (it may be deleted).
                file(QUEUED, Some("x"), None),
            ]),
            &snapshot(&[QUEUED]),
        );
        let rules: Vec<(&str, Option<&str>)> = found
            .iter()
            .map(|issue| (issue.rule.as_str(), issue.path.as_deref()))
            .collect();
        assert_eq!(
            rules,
            [
                ("paths", Some("README.md")),
                ("rawAppendOnly", Some("raw/a.md"))
            ]
        );
        assert!(found[0].message.contains("README.md"));
    }

    #[test]
    fn a_report_must_be_about_exactly_these_changes() {
        let snap = snapshot(&[QUEUED]);
        let run = changes(vec![
            file("raw/a.md", None, Some("一条\n")),
            file("index.md", Some("a\n"), Some("b\n")),
        ]);
        let ok = report(true, &["raw/a.md", "index.md"], &[QUEUED], &[]);
        assert_eq!(validate_report(&ok, &run, &snap), Ok(()));
        let failed = report(false, &["raw/a.md", "index.md"], &[], &[]);
        assert_eq!(validate_report(&failed, &run, &snap), Ok(()));

        let cases = [
            (report(true, &["raw/a.md"], &[QUEUED], &[]), "漏了一个改动"),
            (
                report(true, &["raw/a.md", "index.md"], &[], &[]),
                "没有确认这个队列文件",
            ),
            (
                report(true, &["raw/a.md", "index.md", "self/底色.md"], &[QUEUED], &[]),
                "不在改动里",
            ),
            (
                report(true, &["raw/a.md", "index.md", QUEUED], &[QUEUED], &[]),
                "不在改动里",
            ),
            (
                report(false, &["raw/a.md"], &["index.md"], &[]),
                "不属于这次队列",
            ),
            (
                report(false, &["raw/a.md"], &[QUEUED], &[QUEUED]),
                "既要删除又要放回",
            ),
        ];
        for (bad, expected) in cases {
            let error = validate_report(&bad, &run, &snap).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let mut contradiction = report(true, &["raw/a.md", "index.md"], &[QUEUED], &[]);
        contradiction.violations.push(GateIssue {
            rule: "paths".into(),
            path: None,
            message: "x".into(),
        });
        assert!(validate_report(&contradiction, &run, &snap)
            .unwrap_err()
            .contains("自相矛盾"));
    }

    fn approval(path: &str) -> GateApproval {
        GateApproval {
            rule: "systems".into(),
            path: path.into(),
            entry: None,
            message: "要你确认".into(),
            before: None,
            after: None,
        }
    }

    #[test]
    fn a_report_that_asks_for_approval_is_complete_and_not_ok() {
        let snap = snapshot(&[QUEUED]);
        let run = changes(vec![
            file("raw/a.md", None, Some("一条\n")),
            file("systems/周节奏.md", Some("a\n"), Some("b\n")),
        ]);
        let mut asks = report(true, &["raw/a.md", "systems/周节奏.md"], &[QUEUED], &[]);
        asks.ok = false;
        asks.approvals.push(approval("systems/周节奏.md"));
        assert_eq!(validate_report(&asks, &run, &snap), Ok(()));

        // "ok" with something to approve is a contradiction.
        let mut contradiction = asks.clone();
        contradiction.ok = true;
        assert!(validate_report(&contradiction, &run, &snap)
            .unwrap_err()
            .contains("自相矛盾"));
        // What waits for approval gets committed later: nothing may be missing.
        let mut partial = asks.clone();
        partial.commit_paths = vec!["systems/周节奏.md".into()];
        assert!(validate_report(&partial, &run, &snap)
            .unwrap_err()
            .contains("漏了一个改动"));
        // It can only ask about a changed file.
        let mut stranger = asks;
        stranger.approvals.push(approval("systems/别的.md"));
        assert!(validate_report(&stranger, &run, &snap)
            .unwrap_err()
            .contains("不在改动里"));
    }

    #[test]
    fn the_app_itself_asks_about_every_change_under_systems() {
        let found = own_approvals(
            &changes(vec![
                file("systems/新机制.md", None, Some("# 新\n")),
                file("systems/周节奏.md", Some("a\n"), Some("b\n")),
                file("systems/旧.md", Some("a\n"), None),
                file("wiki/a.md", Some("a\n"), Some("b\n")),
                file("systemsx/a.md", None, Some("x\n")),
            ]),
            &snapshot(&[]),
        );
        let paths: Vec<&str> = found.iter().map(|item| item.path.as_str()).collect();
        assert_eq!(paths, ["systems/新机制.md", "systems/周节奏.md", "systems/旧.md"]);
        assert!(found[0].message.starts_with("新建了"));
        assert!(found[2].message.starts_with("删除了"));
        assert!(found.iter().all(|item| item.rule == "systems"));
    }

    #[test]
    fn changed_files_reach_the_gate_as_text_or_as_opaque() {
        let text = GateFile::from(&file("wiki/a.md", Some("旧\n"), Some("新\n")));
        assert_eq!(
            text,
            GateFile {
                path: "wiki/a.md".into(),
                status: FileStatus::Modified,
                before: Some("旧\n".into()),
                after: Some("新\n".into()),
                opaque: None,
            }
        );
        let mut binary = file("raw/图.bin", None, Some(""));
        binary.after = Content::Bytes(vec![0, 159, 146, 150]);
        assert_eq!(GateFile::from(&binary).opaque, Some(Opaque::Binary));
        assert_eq!(GateFile::from(&binary).after, None);

        let mut link = file("wiki/a.md", Some("旧\n"), Some(""));
        link.after = Content::Symlink;
        let link = GateFile::from(&link);
        assert_eq!((link.opaque, link.before, link.after), (Some(Opaque::Symlink), None, None));

        let mut large = file("raw/大.md", Some("旧\n"), Some(""));
        large.after = Content::Large;
        assert_eq!(GateFile::from(&large).opaque, Some(Opaque::Large));
    }

    /// The example both this module and `src/domain/gate.test.ts` are tested against.
    const EXAMPLE: &str = include_str!("../../../src/domain/__fixtures__/gate-example.json");

    #[test]
    fn speaks_the_same_json_as_the_typescript_gate() {
        let example: Value = serde_json::from_str(EXAMPLE).expect("gate-example.json");
        // The input the frontend receives has exactly these fields …
        let input: GateInput = serde_json::from_value(example["input"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), example["input"]);
        assert_eq!(input.task, Task::Ingest);
        assert_eq!(input.changes[0].status, FileStatus::Modified);
        assert!(input.changes.iter().any(|file| file.opaque == Some(Opaque::Symlink)));
        // … and the report it sends back is read completely.
        let report: GateReport = serde_json::from_value(example["report"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&report).unwrap(), example["report"]);
        assert!(!report.ok);
        assert!(!report.violations.is_empty());
        assert!(report.final_line.is_some());
        assert_eq!(report.approvals.len(), 1);
        assert_eq!(report.approvals[0].path, "systems/周节奏.md");
        // A report from before approvals existed still reads.
        let mut old = example["report"].clone();
        old.as_object_mut().unwrap().remove("approvals");
        let old: GateReport = serde_json::from_value(old).unwrap();
        assert!(old.approvals.is_empty());
    }
}
