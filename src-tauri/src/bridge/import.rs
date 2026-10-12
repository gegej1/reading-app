//! Importing what the phone wrote into the bridge folder's `收件/`.
//!
//! - `收件/记一笔/*.txt|md`: the content, verbatim, becomes a capture (记一笔)
//!   with source `iphone`, exactly like `--capture` (same frontmatter, same
//!   commit message `收件箱：记一笔（iphone）`).
//! - `收件/回答/Q-001-<anything>.txt|md`: the content becomes an answer to
//!   the pending ruling Q-001 in `inbox/裁决/`, exactly like `--answer`
//!   (`对象: Q-001`, `来源: iphone`). An answer whose ruling is unknown or
//!   already decided, or whose name holds no ruling id, is kept anyway: as a
//!   capture whose first line says what it was meant for.
//!
//! iCloud may still be writing or downloading a file: `.icloud` placeholders
//! are asked to download (and left alone), and files modified within the
//! last few seconds, or whose size changes while they are read, wait for the
//! next pass. The phone's file is deleted only after the commit succeeded;
//! the import record (`super::ledger`) makes sure that a file whose deletion
//! failed is never imported twice.
//!
//! Imports take the knowledge-base lock like any capture: captures always go
//! through (also beside an AI run); answers wait while a 落裁决 works through
//! `inbox/裁决/`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use super::folder::Layout;
use super::ledger::{self, Entry, Ledger};
use crate::commands::creates_beside_run;
use crate::kb::capture::{self, Capture, LocalTime};
use crate::kb::lock::KbLock;
use crate::run::Task;
use crate::shell_env::ChildEnv;

/// The source written into captures and answers from the phone.
pub const SOURCE: &str = "iphone";
/// A file modified more recently than this may still be in the middle of
/// being written (or synced) and waits.
pub const SETTLE: Duration = Duration::from_secs(3);
/// How long an import waits for the knowledge-base lock before it tries
/// again on the next pass.
pub const LOCK_WAIT: Duration = Duration::from_secs(20);
/// What the lock holder is called while an import commits.
pub const OPERATION: &str = "收 iPhone 的记录";
/// More text than this is not a capture typed on a phone: left alone.
pub const MAX_BYTES: u64 = 1024 * 1024;

/// The two folders the phone writes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Capture,
    Answer,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Capture => "记一笔",
            Kind::Answer => "回答",
        }
    }
}

/// What happened to one file of the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Committed to the knowledge base as `path`. `note` says why an answer
    /// became a capture; `removed` whether the phone's file is gone.
    Imported {
        path: String,
        note: Option<String>,
        removed: bool,
    },
    /// Imported before (its deletion had failed): only deleted now.
    AlreadyImported { removed: bool },
    /// An empty file: there was nothing to keep, so it was deleted.
    Empty { removed: bool },
    /// Still being written or downloaded: looked at again in a few seconds.
    Settling,
    /// Not now (the knowledge base is busy, a 落裁决 is going on): looked at
    /// again on a later pass.
    Waiting(String),
    /// Left alone: the app does not import this file.
    Skipped(String),
    /// The knowledge base refused it; left in place and tried again later.
    Failed(String),
}

/// One file of a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub outcome: Outcome,
}

/// What one pass over `收件/` did.
#[derive(Debug, Default)]
pub struct Pass {
    pub items: Vec<Item>,
    /// Something is still being written or downloaded.
    pub retry_soon: bool,
}

impl Pass {
    /// The knowledge-base files this pass committed.
    pub fn imported_paths(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|item| match &item.outcome {
                Outcome::Imported { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

/// What an import needs.
pub struct Context<'a> {
    /// The knowledge base (canonical).
    pub kb_root: &'a Path,
    pub env: &'a ChildEnv,
    pub lock: &'a KbLock,
    /// The AI run going on, if any.
    pub running: &'a dyn Fn() -> Option<Task>,
    pub ledger: &'a mut Ledger,
    pub settle: Duration,
    /// How long to wait for the knowledge-base lock ([`LOCK_WAIT`]).
    pub lock_wait: Duration,
    /// Asks iCloud to download a file that is only a placeholder so far.
    pub download: &'a mut dyn FnMut(&Path),
}

/// Looks at every file in `收件/记一笔/` and `收件/回答/`, oldest name first.
pub fn run_pass(layout: &Layout, ctx: &mut Context) -> Pass {
    let mut pass = Pass::default();
    for (kind, dir) in [
        (Kind::Capture, layout.captures()),
        (Kind::Answer, layout.answers()),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<(String, std::path::PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                Some((name, entry.path()))
            })
            .collect();
        names.sort();
        for (name, path) in names {
            if name.starts_with('.') {
                // `.名字.txt.icloud`: iCloud has not downloaded it yet. Other
                // hidden files (.DS_Store, temp files) are not the phone's.
                if name.ends_with(".icloud") {
                    (ctx.download)(&path);
                    pass.retry_soon = true;
                    pass.items.push(Item {
                        kind,
                        name,
                        outcome: Outcome::Settling,
                    });
                }
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                continue;
            }
            let outcome = import_file(kind, &path, &name, &meta, ctx);
            if outcome == Outcome::Settling {
                pass.retry_soon = true;
            }
            pass.items.push(Item {
                kind,
                name,
                outcome,
            });
        }
    }
    pass
}

fn import_file(
    kind: Kind,
    path: &Path,
    name: &str,
    meta: &fs::Metadata,
    ctx: &mut Context,
) -> Outcome {
    if !meta.is_file() {
        return Outcome::Skipped("不是普通文件".to_string());
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("txt" | "md")) {
        return Outcome::Skipped("只收 .txt 和 .md 文件".to_string());
    }
    if meta.len() > MAX_BYTES {
        return Outcome::Skipped("文件太大（超过 1 MB），不像是手机上记的一笔".to_string());
    }
    let modified = meta.modified().ok();
    let age = modified
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .unwrap_or(Duration::ZERO);
    if age < ctx.settle {
        return Outcome::Settling;
    }
    let Ok(bytes) = fs::read(path) else {
        return Outcome::Settling;
    };
    match fs::metadata(path) {
        Ok(after) if after.len() == bytes.len() as u64 => {}
        _ => return Outcome::Settling,
    }
    let key = ledger::key(kind.label(), name, &bytes);
    if ctx.ledger.contains(&key) {
        return Outcome::AlreadyImported {
            removed: remove(path),
        };
    }
    let content = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let Ok(text) = std::str::from_utf8(content) else {
        return Outcome::Skipped("不是 UTF-8 文本".to_string());
    };
    if capture::is_blank_text(text) {
        return Outcome::Empty {
            removed: remove(path),
        };
    }
    let at = time_of(name, modified);

    let (built, note) = match kind {
        Kind::Capture => (capture::build_capture(text, SOURCE, &at), None),
        Kind::Answer => match answer_target(name, ctx.kb_root) {
            Target::Open(id) => {
                if (ctx.running)() == Some(Task::LandRulings) {
                    return Outcome::Waiting("正在落裁决，回答等它结束再收".to_string());
                }
                (capture::build_answer(&id, text, SOURCE, &at), None)
            }
            Target::Other { subject, reason } => {
                let line = format!("（iPhone 上{subject}的回答：{reason}，先作为随手记收下）");
                (
                    capture::build_capture(&format!("{line}\n\n{text}"), SOURCE, &at),
                    Some(reason),
                )
            }
        },
    };
    let built = match built {
        Ok(built) => built,
        Err(error) => return Outcome::Skipped(error),
    };
    match save(ctx, &built) {
        Ok(saved_path) => {
            if let Err(error) = ctx.ledger.record(Entry {
                key,
                at: at.iso(),
                path: saved_path.clone(),
            }) {
                log::warn!("iphone: the import record could not be saved ({error})");
            }
            Outcome::Imported {
                path: saved_path,
                note,
                removed: remove(path),
            }
        }
        Err(outcome) => outcome,
    }
}

/// Creates and commits the file under the knowledge-base lock.
fn save(ctx: &Context, built: &Capture) -> Result<String, Outcome> {
    let guard = if creates_beside_run(&built.path, (ctx.running)()) {
        ctx.lock.acquire_beside_job(OPERATION, ctx.lock_wait)
    } else {
        ctx.lock.acquire(OPERATION, ctx.lock_wait)
    }
    .map_err(|busy| Outcome::Waiting(busy.to_string()))?;
    let saved = capture::save_capture(ctx.kb_root, ctx.env, built).map_err(Outcome::Failed)?;
    if let Some(commit) = &saved.outcome.commit {
        guard.note_commit(commit, &saved.path);
    }
    Ok(saved.path)
}

/// Deletes the phone's file; true when it is gone.
fn remove(path: &Path) -> bool {
    fs::remove_file(path).is_ok() || !path.exists()
}

/// Where an answer goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// To this pending ruling.
    Open(String),
    /// Nowhere: kept as a capture. `subject` names what it was meant for.
    Other { subject: String, reason: String },
}

fn answer_target(name: &str, kb_root: &Path) -> Target {
    let Some(id) = ruling_id_in_name(name) else {
        return Target::Other {
            subject: String::new(),
            reason: "文件名里没有待裁决的编号".to_string(),
        };
    };
    let subject = format!("对 {id} ");
    match ruling_states(kb_root).get(&id) {
        Some(true) => Target::Open(id),
        Some(false) => Target::Other {
            subject,
            reason: "它已经裁决了".to_string(),
        },
        None => Target::Other {
            subject,
            reason: "知识库里没有这个待裁决".to_string(),
        },
    }
}

/// `Q-001` from `Q-001-2026-10-12T14-03-22.txt`: `Q-`, digits, then the end
/// of the stem or something that is not a letter or digit.
pub fn ruling_id_in_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("Q-")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    match rest[digits.len()..].chars().next() {
        Some(next) if next.is_alphanumeric() => None,
        _ => Some(format!("Q-{digits}")),
    }
}

/// Every ruling in `self/待裁决/` by id: true while it is pending. Mirrors
/// `parseRuling` in `src/domain/rulings.ts`: the id comes from the
/// frontmatter's `编号`, else from the file name; a ruling is pending until
/// its `状态` says 已裁决.
pub fn ruling_states(kb_root: &Path) -> HashMap<String, bool> {
    let mut states = HashMap::new();
    let Ok(entries) = fs::read_dir(kb_root.join("self/待裁决")) else {
        return states;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".md") else {
            continue;
        };
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let fields = frontmatter_fields(&text);
        let id = fields
            .get("编号")
            .filter(|id| id.starts_with("Q-"))
            .cloned()
            .or_else(|| ruling_id_in_name(stem));
        if let Some(id) = id {
            let pending = fields
                .get("状态")
                .is_none_or(|status| !status.contains("已裁决"));
            states.insert(id, pending);
        }
    }
    states
}

/// The simple `key: value` lines of a YAML frontmatter block.
fn frontmatter_fields(text: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return fields;
    }
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if !key.starts_with(' ') && !value.is_empty() {
                fields.insert(key.trim().to_string(), value.to_string());
            }
        }
    }
    fields
}

/// When the phone wrote the file: the `2026-10-12T14-03-22` stamp the
/// shortcuts put into the name, else the file's modification time, else now.
fn time_of(name: &str, modified: Option<SystemTime>) -> LocalTime {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    if let Some(stamp) = stamp_in_name(name) {
        return LocalTime::from(stamp.assume_offset(offset));
    }
    let now = OffsetDateTime::now_utc();
    match modified.map(OffsetDateTime::from) {
        Some(time) if time <= now + Duration::from_secs(60) => {
            LocalTime::from(time.to_offset(offset))
        }
        _ => LocalTime::now(),
    }
}

/// The first `YYYY-MM-DDTHH-mm-ss` in a file name.
pub fn stamp_in_name(name: &str) -> Option<PrimitiveDateTime> {
    const SHAPE: &[u8; 19] = b"0000-00-00T00-00-00";
    let bytes = name.as_bytes();
    (0..bytes.len().saturating_sub(SHAPE.len() - 1)).find_map(|start| {
        let window = &bytes[start..start + SHAPE.len()];
        let fits = window.iter().zip(SHAPE).all(|(byte, shape)| {
            if *shape == b'0' {
                byte.is_ascii_digit()
            } else {
                byte == shape
            }
        });
        if !fits {
            return None;
        }
        let number = |from: usize, to: usize| -> Option<u32> {
            std::str::from_utf8(&window[from..to]).ok()?.parse().ok()
        };
        let date = Date::from_calendar_date(
            number(0, 4)? as i32,
            Month::try_from(number(5, 7)? as u8).ok()?,
            number(8, 10)? as u8,
        )
        .ok()?;
        let time = Time::from_hms(
            number(11, 13)? as u8,
            number(14, 16)? as u8,
            number(17, 19)? as u8,
        )
        .ok()?;
        Some(PrimitiveDateTime::new(date, time))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::folder::{prepare, DirSource};
    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};
    use std::cell::RefCell;

    struct Setup {
        kb: TempDir,
        bridge: TempDir,
        layout: Layout,
    }

    fn setup(tag: &str) -> Setup {
        let kb = TempDir::new(&format!("{tag}-kb"));
        init_repo(kb.path());
        kb.write(
            "self/待裁决/Q-001-周会.md",
            "---\n类型: 待裁决\n编号: Q-001\n状态: 待裁决\n---\n\n# Q-001 周会\n",
        );
        kb.write(
            "self/待裁决/Q-002-外包.md",
            "---\n类型: 待裁决\n编号: Q-002\n状态: 已裁决（2026-10-01）\n---\n\n# Q-002 外包\n",
        );
        kb.write(
            "self/待裁决/Q-003-没有编号.md",
            "# Q-003 没有 frontmatter\n",
        );
        git(kb.path(), &["add", "-A"]);
        git(kb.path(), &["commit", "-q", "-m", "init"]);
        let bridge = TempDir::new(&format!("{tag}-bridge"));
        let layout = prepare(&bridge.path().join("底色"), DirSource::Env).unwrap();
        Setup { kb, bridge, layout }
    }

    /// Writes a file the way the phone would and makes it old enough.
    fn drop_file(dir: &Path, name: &str, content: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        let old = SystemTime::now() - Duration::from_secs(10);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        path
    }

    fn pass(setup: &Setup, ledger: &mut Ledger, running: Option<Task>) -> Pass {
        let env = ChildEnv::inherit();
        let lock = KbLock::default();
        let running_fn = move || running;
        let mut download = |_: &Path| {};
        let mut ctx = Context {
            kb_root: setup.kb.path(),
            env: &env,
            lock: &lock,
            running: &running_fn,
            ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
        };
        run_pass(&setup.layout, &mut ctx)
    }

    #[test]
    fn a_capture_becomes_a_verbatim_iphone_capture_and_leaves_icloud() {
        let s = setup("import-capture");
        let text = "  第一行\r\n第二行 ✅ [[链接]]\n\n";
        let file = drop_file(
            &s.layout.captures(),
            "2026-10-12T14-03-22.txt",
            format!("\u{FEFF}{text}").as_bytes(),
        );
        let mut ledger = Ledger::load(None);
        let result = pass(&s, &mut ledger, None);
        assert_eq!(result.items.len(), 1);
        let Outcome::Imported {
            path,
            note,
            removed,
        } = &result.items[0].outcome
        else {
            panic!("{:?}", result.items);
        };
        assert_eq!(path, "inbox/2026-10-12T14-03-22-iphone.md");
        assert_eq!(note, &None);
        assert!(removed);
        assert!(!file.exists(), "deleted after the commit");
        let content = fs::read_to_string(s.kb.path().join(path)).unwrap();
        let expected = capture::build_capture(
            text,
            SOURCE,
            &LocalTime::from(
                stamp_in_name("2026-10-12T14-03-22")
                    .unwrap()
                    .assume_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)),
            ),
        )
        .unwrap();
        assert_eq!(content, expected.content);
        assert!(content.contains("来源: iphone\n"));
        assert!(content.ends_with("\n\n  第一行\n第二行 ✅ [[链接]]\n"));
        assert_eq!(
            git(s.kb.path(), &["log", "-1", "--format=%s"]),
            "收件箱：记一笔（iphone）"
        );
        assert_eq!(ledger.len(), 1);
        assert_eq!(result.imported_paths(), vec![path.clone()]);
    }

    #[test]
    fn an_answer_to_an_open_ruling_goes_to_the_answer_folder() {
        let s = setup("import-answer");
        drop_file(
            &s.layout.answers(),
            "Q-001-2026-10-12T14-05-00.txt",
            "保留，但只开三十分钟。".as_bytes(),
        );
        let mut ledger = Ledger::load(None);
        let result = pass(&s, &mut ledger, None);
        let Outcome::Imported { path, note, .. } = &result.items[0].outcome else {
            panic!("{:?}", result.items);
        };
        assert_eq!(path, "inbox/裁决/Q-001-2026-10-12T14-05-00-iphone.md");
        assert_eq!(note, &None);
        let content = fs::read_to_string(s.kb.path().join(path)).unwrap();
        assert!(content.starts_with("---\n对象: Q-001\n时间: 2026-10-12T14:05:00"));
        assert!(content.contains("\n来源: iphone\n---\n\n保留，但只开三十分钟。\n"));
        assert_eq!(
            git(s.kb.path(), &["log", "-1", "--format=%s"]),
            "收件箱：回答 Q-001（iphone）"
        );
        assert!(fs::read_dir(s.layout.answers()).unwrap().next().is_none());
    }

    #[test]
    fn answers_without_an_open_ruling_are_kept_as_captures_with_a_note() {
        let s = setup("import-orphans");
        for name in ["Q-002-a.txt", "Q-099-b.txt", "随便写的.txt"] {
            drop_file(&s.layout.answers(), name, "我的回答".as_bytes());
        }
        let mut ledger = Ledger::load(None);
        let result = pass(&s, &mut ledger, None);
        let notes: Vec<String> = result
            .items
            .iter()
            .map(|item| match &item.outcome {
                Outcome::Imported { path, note, .. } => {
                    assert!(path.starts_with("inbox/2"), "{path}");
                    let content = fs::read_to_string(s.kb.path().join(path)).unwrap();
                    assert!(
                        content.ends_with("先作为随手记收下）\n\n我的回答\n"),
                        "{content}"
                    );
                    note.clone().unwrap()
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            notes,
            [
                "它已经裁决了",
                "知识库里没有这个待裁决",
                "文件名里没有待裁决的编号"
            ]
        );
        let first =
            fs::read_to_string(s.kb.path().join(result.imported_paths()[0].as_str())).unwrap();
        assert!(first.contains("（iPhone 上对 Q-002 的回答：它已经裁决了，先作为随手记收下）"));
        assert_eq!(commit_count(s.kb.path()), 4);
    }

    #[test]
    fn answers_wait_for_a_land_rulings_run_but_captures_do_not() {
        let s = setup("import-busy");
        let answer = drop_file(&s.layout.answers(), "Q-001-x.txt", b"answer");
        let captured = drop_file(&s.layout.captures(), "c.md", b"capture");
        let mut ledger = Ledger::load(None);
        let result = pass(&s, &mut ledger, Some(Task::LandRulings));
        assert!(matches!(result.items[0].outcome, Outcome::Imported { .. }));
        assert!(
            matches!(result.items[1].outcome, Outcome::Waiting(ref why) if why.contains("落裁决"))
        );
        assert!(answer.exists());
        assert!(!captured.exists());
        // Beside an ingest an answer goes in.
        let result = pass(&s, &mut ledger, Some(Task::Ingest));
        assert!(
            matches!(result.items[0].outcome, Outcome::Imported { .. }),
            "{:?}",
            result.items
        );
        assert!(!answer.exists());
    }

    #[test]
    fn a_failed_deletion_never_imports_twice() {
        let s = setup("import-twice");
        let mut ledger = Ledger::load(None);
        let name = "2026-10-12T14-03-22.txt";
        drop_file(&s.layout.captures(), name, b"once");
        pass(&s, &mut ledger, None);
        assert_eq!(commit_count(s.kb.path()), 2);
        // The same file shows up again (as if the deletion had not reached iCloud).
        let again = drop_file(&s.layout.captures(), name, b"once");
        let result = pass(&s, &mut ledger, None);
        assert_eq!(
            result.items[0].outcome,
            Outcome::AlreadyImported { removed: true }
        );
        assert!(!again.exists());
        assert_eq!(commit_count(s.kb.path()), 2);
    }

    #[test]
    fn nothing_is_deleted_when_the_commit_fails() {
        let s = setup("import-no-commit");
        let file = drop_file(&s.layout.captures(), "a.txt", b"keep me");
        // A knowledge base that is not a git repository refuses the capture.
        let plain = TempDir::new("import-plain");
        let env = ChildEnv::inherit();
        let lock = KbLock::default();
        let running = || None;
        let mut ledger = Ledger::load(None);
        let mut download = |_: &Path| {};
        let mut ctx = Context {
            kb_root: plain.path(),
            env: &env,
            lock: &lock,
            running: &running,
            ledger: &mut ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
        };
        let result = run_pass(&s.layout, &mut ctx);
        assert!(matches!(result.items[0].outcome, Outcome::Failed(ref e) if e.contains("git")));
        assert!(file.exists());
        assert_eq!(ledger.len(), 0);

        // A busy knowledge base: the file waits for a later pass.
        let held = lock.acquire("同步", Duration::ZERO).unwrap();
        let mut ctx = Context {
            kb_root: s.kb.path(),
            env: &env,
            lock: &lock,
            running: &running,
            ledger: &mut ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
        };
        let result = run_pass(&s.layout, &mut ctx);
        assert!(
            matches!(result.items[0].outcome, Outcome::Waiting(ref why) if why.contains("知识库正忙")),
            "{:?}",
            result.items
        );
        assert!(file.exists());
        drop(held);
        let result = run_pass(&s.layout, &mut ctx);
        assert!(matches!(result.items[0].outcome, Outcome::Imported { .. }));
        assert!(!file.exists());
    }

    #[test]
    fn unfinished_and_foreign_files_are_left_alone() {
        let s = setup("import-unfinished");
        // Just written: waits.
        let fresh = s.layout.captures().join("fresh.txt");
        fs::write(&fresh, b"still syncing").unwrap();
        // Not text, not UTF-8, empty, hidden, a placeholder, a folder.
        let pdf = drop_file(&s.layout.captures(), "scan.pdf", b"%PDF");
        let latin = drop_file(&s.layout.captures(), "latin.txt", b"caf\xe9");
        let empty = drop_file(
            &s.layout.captures(),
            "empty.txt",
            " \n\u{3000}\n".as_bytes(),
        );
        drop_file(&s.layout.captures(), ".DS_Store", b"x");
        drop_file(&s.layout.captures(), ".later.txt.icloud", b"placeholder");
        fs::create_dir_all(s.layout.captures().join("folder")).unwrap();

        let downloads = RefCell::new(Vec::new());
        let env = ChildEnv::inherit();
        let lock = KbLock::default();
        let running = || None;
        let mut ledger = Ledger::load(None);
        let mut download = |path: &Path| downloads.borrow_mut().push(path.to_path_buf());
        let mut ctx = Context {
            kb_root: s.kb.path(),
            env: &env,
            lock: &lock,
            running: &running,
            ledger: &mut ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
        };
        let result = run_pass(&s.layout, &mut ctx);
        assert!(result.retry_soon);
        let by_name: HashMap<&str, &Outcome> = result
            .items
            .iter()
            .map(|item| (item.name.as_str(), &item.outcome))
            .collect();
        assert_eq!(by_name["fresh.txt"], &Outcome::Settling);
        assert_eq!(by_name[".later.txt.icloud"], &Outcome::Settling);
        assert!(matches!(by_name["scan.pdf"], Outcome::Skipped(_)));
        assert!(matches!(by_name["latin.txt"], Outcome::Skipped(ref why) if why.contains("UTF-8")));
        assert_eq!(by_name["empty.txt"], &Outcome::Empty { removed: true });
        assert!(!by_name.contains_key(".DS_Store") && !by_name.contains_key("folder"));
        assert_eq!(downloads.borrow().len(), 1);
        assert!(fresh.exists() && pdf.exists() && latin.exists() && !empty.exists());
        assert_eq!(commit_count(s.kb.path()), 1);
        drop(s.bridge);
    }

    #[test]
    fn reads_ruling_ids_states_and_stamps() {
        assert_eq!(
            ruling_id_in_name("Q-001-2026.txt").as_deref(),
            Some("Q-001")
        );
        assert_eq!(ruling_id_in_name("Q-12.txt").as_deref(), Some("Q-12"));
        assert_eq!(ruling_id_in_name("Q-001x.txt"), None);
        assert_eq!(ruling_id_in_name("Q-.txt"), None);
        assert_eq!(ruling_id_in_name("q-001.txt"), None);

        let s = setup("import-states");
        let states = ruling_states(s.kb.path());
        assert_eq!(states.get("Q-001"), Some(&true));
        assert_eq!(states.get("Q-002"), Some(&false));
        assert_eq!(states.get("Q-003"), Some(&true), "id from the file name");
        assert_eq!(states.len(), 3);

        let stamp = stamp_in_name("Q-001-2026-10-12T14-03-22.txt").unwrap();
        assert_eq!(stamp.to_string(), "2026-10-12 14:03:22.0");
        assert!(stamp_in_name("2026-13-12T14-03-22.txt").is_none());
        assert!(stamp_in_name("短.txt").is_none());
        assert!(stamp_in_name("记一笔 2026-10-12T14-03-22").is_some());
    }
}
