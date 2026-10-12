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
//! The capture time is taken from the file itself (its creation or
//! modification time, whichever is earlier), not from its name: the
//! shortcuts' stamp follows the phone's calendar, which need not be the
//! Gregorian one. The name only keeps files apart.
//!
//! iCloud may still be writing or downloading a file: `.icloud` placeholders
//! of text files are asked to download (and left alone), and files modified
//! within the last few seconds, or whose size changes while they are read,
//! wait. Such a file is looked at again a few seconds later, but only
//! [`QUICK_RETRIES`] times; after that it waits for the normal cadence.
//!
//! The phone's file is deleted only after the commit succeeded. The import
//! record (`super::ledger`) is written before the commit and again after
//! it, so a file is never committed twice: not when its deletion failed, not
//! when the app stopped in between. When the record cannot be written, the
//! pass imports nothing. When the record has nothing certain about a file,
//! the knowledge base is searched for the same text first (`super::earlier`).
//!
//! Imports take the knowledge-base lock like any capture: captures always go
//! through (also beside an AI run); answers wait while a 落裁决 works through
//! `inbox/裁决/`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use super::earlier;
use super::folder::Layout;
use super::ledger::{self, Ledger};
use crate::commands::creates_beside_run;
use crate::kb::capture::{self, Capture, LocalTime};
use crate::kb::git;
use crate::kb::lock::{KbLock, KbLockGuard};
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
/// How often one unfinished file (a placeholder that does not download, a
/// file that keeps changing) is looked at again soon, before it waits for
/// the normal cadence like everything else.
pub const QUICK_RETRIES: u32 = 5;
/// Times before this year are not when a phone wrote something.
const EARLIEST_YEAR: i32 = 2000;

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
    /// Imported before (its deletion had failed, or the app stopped before
    /// it could be deleted): only deleted now.
    AlreadyImported { removed: bool },
    /// An empty file: there was nothing to keep, so it was deleted.
    Empty { removed: bool },
    /// Still being written or downloaded: looked at again in a few seconds.
    Settling,
    /// Not now (the knowledge base is busy, a 落裁决 is going on, the import
    /// record cannot be written): looked at again on a later pass.
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
    /// The import record could not be written: the pass stopped there.
    pub record_problem: Option<String>,
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

/// One file found in `收件/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub kind: Kind,
    pub name: String,
    pub path: PathBuf,
}

/// How often each unfinished file was looked at again soon.
#[derive(Debug, Default)]
pub struct Retries(HashMap<PathBuf, u32>);

impl Retries {
    /// Counts one more quick look at `path`; true while that is allowed.
    fn allow(&mut self, path: &Path) -> bool {
        let tries = self.0.entry(path.to_path_buf()).or_insert(0);
        *tries += 1;
        *tries <= QUICK_RETRIES
    }

    /// Forgets the files that are gone or no longer unfinished.
    fn keep(&mut self, unfinished: &HashSet<PathBuf>) {
        self.0.retain(|path, _| unfinished.contains(path));
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
    /// Kept from pass to pass.
    pub retries: &'a mut Retries,
}

/// True for the names the app imports: `.txt` and `.md`.
fn is_text_name(name: &str) -> bool {
    let extension = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    matches!(extension.as_deref(), Some("txt" | "md"))
}

/// The file a `.名字.txt.icloud` placeholder stands for, if it is one.
fn placeholder_of(name: &str) -> Option<&str> {
    name.strip_prefix('.')?.strip_suffix(".icloud")
}

/// Every file in `收件/记一笔/` and `收件/回答/`, oldest name first. Hidden
/// files are left out, except placeholders of files the app would import.
pub fn scan(layout: &Layout) -> Vec<Incoming> {
    let mut found = Vec::new();
    for (kind, dir) in [
        (Kind::Capture, layout.captures()),
        (Kind::Answer, layout.answers()),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<Incoming> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                if name.starts_with('.') && !placeholder_of(&name).is_some_and(is_text_name) {
                    // .DS_Store, temp files, placeholders of a PDF: not the phone's text.
                    return None;
                }
                Some(Incoming {
                    kind,
                    name,
                    path: entry.path(),
                })
            })
            .collect();
        names.sort_by(|a, b| a.name.cmp(&b.name));
        found.extend(names);
    }
    found
}

/// Imports what [`scan`] found. Stops when the import record cannot be
/// written: then nothing more is imported in this pass.
pub fn run_pass(files: &[Incoming], ctx: &mut Context) -> Pass {
    let mut pass = Pass::default();
    let mut unfinished = HashSet::new();
    for file in files {
        let (outcome, problem) = if placeholder_of(&file.name).is_some() {
            // iCloud has not downloaded it yet.
            (ctx.download)(&file.path);
            (Outcome::Settling, None)
        } else {
            let Ok(meta) = fs::symlink_metadata(&file.path) else {
                continue;
            };
            if meta.is_dir() {
                continue;
            }
            import_file(file, &meta, ctx)
        };
        if outcome == Outcome::Settling {
            unfinished.insert(file.path.clone());
            if ctx.retries.allow(&file.path) {
                pass.retry_soon = true;
            }
        }
        pass.items.push(Item {
            kind: file.kind,
            name: file.name.clone(),
            outcome,
        });
        if problem.is_some() {
            pass.record_problem = problem;
            break;
        }
    }
    ctx.retries.keep(&unfinished);
    pass
}

/// Imports one file. The second value is set when the import record could
/// not be written: the pass stops.
fn import_file(
    file: &Incoming,
    meta: &fs::Metadata,
    ctx: &mut Context,
) -> (Outcome, Option<String>) {
    let (kind, path, name) = (file.kind, file.path.as_path(), file.name.as_str());
    if !meta.is_file() {
        return (Outcome::Skipped("不是普通文件".to_string()), None);
    }
    if !is_text_name(name) {
        return (Outcome::Skipped("只收 .txt 和 .md 文件".to_string()), None);
    }
    if meta.len() > MAX_BYTES {
        return (
            Outcome::Skipped("文件太大（超过 1 MB），不像是手机上记的一笔".to_string()),
            None,
        );
    }
    let age = meta
        .modified()
        .ok()
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .unwrap_or(Duration::ZERO);
    if age < ctx.settle {
        return (Outcome::Settling, None);
    }
    let Ok(bytes) = fs::read(path) else {
        return (Outcome::Settling, None);
    };
    match fs::metadata(path) {
        Ok(after) if after.len() == bytes.len() as u64 => {}
        _ => return (Outcome::Settling, None),
    }
    let key = ledger::key(kind.label(), name, &bytes);
    let pending = match ctx.ledger.get(&key) {
        Some(entry) if !entry.pending => {
            return (
                Outcome::AlreadyImported {
                    removed: remove(path),
                },
                None,
            )
        }
        Some(entry) => Some(entry.path.clone()),
        None => None,
    };
    let content = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let Ok(text) = std::str::from_utf8(content) else {
        return (Outcome::Skipped("不是 UTF-8 文本".to_string()), None);
    };
    if capture::is_blank_text(text) {
        return (
            Outcome::Empty {
                removed: remove(path),
            },
            None,
        );
    }
    let at = capture_time(meta);

    let (built, note) = match kind {
        Kind::Capture => (capture::build_capture(text, SOURCE, &at), None),
        Kind::Answer => match answer_target(name, ctx.kb_root) {
            Target::Open(id) => {
                if (ctx.running)() == Some(Task::LandRulings) {
                    return (
                        Outcome::Waiting("正在落裁决，回答等它结束再收".to_string()),
                        None,
                    );
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
        Err(error) => return (Outcome::Skipped(error), None),
    };

    // Was the same text imported before (a pending entry, a record that was
    // started anew, an older version of the app that named it by its stamp)?
    let mut bases = vec![built.path.clone()];
    bases.extend(pending);
    bases.extend(paths_for(kind, name, text, &at));
    if let Some(stamp) = plausible_stamp(name) {
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        bases.extend(paths_for(
            kind,
            name,
            text,
            &LocalTime::from(stamp.assume_offset(offset)),
        ));
    }
    match earlier::find(ctx.kb_root, ctx.env, &bases, text) {
        Err(error) => {
            return (
                Outcome::Failed(format!("没能查知识库里是不是已经收过它：{error}")),
                None,
            )
        }
        Ok(Some(found)) => {
            let outcome = if found.committed {
                Outcome::AlreadyImported { removed: false }
            } else {
                // The app stopped between creating the file and committing it.
                if let Err(outcome) = commit_leftover(ctx, &found.path, &built.message) {
                    return (outcome, None);
                }
                Outcome::Imported {
                    path: found.path.clone(),
                    note,
                    removed: false,
                }
            };
            let problem = ctx.ledger.finish(&key, &at.iso(), &found.path).err();
            return (with_removed(outcome, remove(path)), problem);
        }
        Ok(None) => {}
    }

    if let Err(problem) = ctx.ledger.begin(&key, &at.iso(), &built.path) {
        return (
            Outcome::Waiting("导入记录写不进去，先不收".to_string()),
            Some(problem),
        );
    }
    match save(ctx, &built) {
        Ok(saved_path) => {
            let problem = ctx.ledger.finish(&key, &at.iso(), &saved_path).err();
            (
                Outcome::Imported {
                    path: saved_path,
                    note,
                    removed: remove(path),
                },
                problem,
            )
        }
        Err(outcome) => (outcome, None),
    }
}

fn with_removed(outcome: Outcome, gone: bool) -> Outcome {
    match outcome {
        Outcome::AlreadyImported { .. } => Outcome::AlreadyImported { removed: gone },
        Outcome::Imported { path, note, .. } => Outcome::Imported {
            path,
            note,
            removed: gone,
        },
        other => other,
    }
}

/// The paths an import of `text` from the file `name` at `at` could have
/// become: an answer may have become a capture, or the other way round.
fn paths_for(kind: Kind, name: &str, text: &str, at: &LocalTime) -> Vec<String> {
    let mut paths: Vec<String> = capture::build_capture(text, SOURCE, at)
        .map(|built| built.path)
        .into_iter()
        .collect();
    if kind == Kind::Answer {
        if let Some(id) = ruling_id_in_name(name) {
            paths.extend(capture::build_answer(&id, text, SOURCE, at).map(|built| built.path));
        }
    }
    paths
}

/// Takes the knowledge-base lock for creating `path`.
fn lock_for(ctx: &Context, path: &str) -> Result<KbLockGuard, Outcome> {
    if creates_beside_run(path, (ctx.running)()) {
        ctx.lock.acquire_beside_job(OPERATION, ctx.lock_wait)
    } else {
        ctx.lock.acquire(OPERATION, ctx.lock_wait)
    }
    .map_err(|busy| Outcome::Waiting(busy.to_string()))
}

/// Creates and commits the file under the knowledge-base lock.
fn save(ctx: &Context, built: &Capture) -> Result<String, Outcome> {
    let guard = lock_for(ctx, &built.path)?;
    let saved = capture::save_capture(ctx.kb_root, ctx.env, built).map_err(Outcome::Failed)?;
    if let Some(commit) = &saved.outcome.commit {
        guard.note_commit(commit, &saved.path);
    }
    Ok(saved.path)
}

/// Commits a file an earlier import created but did not commit.
fn commit_leftover(ctx: &Context, path: &str, message: &str) -> Result<(), Outcome> {
    let guard = lock_for(ctx, path)?;
    let done = git::commit_path(ctx.kb_root, path, message, ctx.env).map_err(Outcome::Failed)?;
    if let Some(commit) = &done.commit {
        guard.note_commit(commit, path);
    }
    Ok(())
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

/// When the phone wrote the file: the earlier of its creation and
/// modification time, leaving out impossible ones; else now.
fn capture_time(meta: &fs::Metadata) -> LocalTime {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let now = OffsetDateTime::now_utc();
    [meta.created().ok(), meta.modified().ok()]
        .into_iter()
        .flatten()
        .map(OffsetDateTime::from)
        .filter(|time| time.year() >= EARLIEST_YEAR && *time <= now + Duration::from_secs(60))
        .min()
        .map(|time| LocalTime::from(time.to_offset(offset)))
        .unwrap_or_else(LocalTime::now)
}

/// The stamp in the name, when it is a Gregorian date that can be true
/// (a phone set to another calendar writes e.g. 2569 or 0008 as the year).
pub fn plausible_stamp(name: &str) -> Option<PrimitiveDateTime> {
    let stamp = stamp_in_name(name)?;
    let latest = OffsetDateTime::now_utc().year() + 1;
    (EARLIEST_YEAR..=latest)
        .contains(&stamp.year())
        .then_some(stamp)
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
    use std::os::unix::fs::PermissionsExt;

    struct Setup {
        kb: TempDir,
        bridge: TempDir,
        app: TempDir,
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
        let app = TempDir::new(&format!("{tag}-app"));
        Setup {
            kb,
            bridge,
            app,
            layout,
        }
    }

    impl Setup {
        /// The import record in the app's folder, as a restarted app reads it.
        fn ledger(&self) -> Ledger {
            Ledger::load(Some(self.app.path().join(ledger::FILE_NAME))).unwrap()
        }
    }

    /// A local wall-clock time.
    fn local(date: (i32, u8, u8), time: (u8, u8, u8)) -> OffsetDateTime {
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        PrimitiveDateTime::new(
            Date::from_calendar_date(date.0, Month::try_from(date.1).unwrap(), date.2).unwrap(),
            Time::from_hms(time.0, time.1, time.2).unwrap(),
        )
        .assume_offset(offset)
    }

    /// Writes a file the way the phone would, modified at `at`.
    fn drop_file_at(dir: &Path, name: &str, content: &[u8], at: SystemTime) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
        path
    }

    /// Writes a file the way the phone would and makes it old enough.
    fn drop_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        drop_file_at(
            dir,
            name,
            content,
            SystemTime::now() - Duration::from_secs(10),
        )
    }

    fn pass_with(
        setup: &Setup,
        ledger: &mut Ledger,
        retries: &mut Retries,
        running: Option<Task>,
    ) -> Pass {
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
            retries,
        };
        run_pass(&scan(&setup.layout), &mut ctx)
    }

    fn pass(setup: &Setup, ledger: &mut Ledger, running: Option<Task>) -> Pass {
        pass_with(setup, ledger, &mut Retries::default(), running)
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_capture_becomes_a_verbatim_iphone_capture_and_leaves_icloud() {
        let s = setup("import-capture");
        let text = "  第一行\r\n第二行 ✅ [[链接]]\n\n";
        let written = local((2026, 10, 11), (14, 3, 22));
        let file = drop_file_at(
            &s.layout.captures(),
            "2026-10-12T14-03-22+08-00.txt",
            format!("\u{FEFF}{text}").as_bytes(),
            written.into(),
        );
        let mut ledger = s.ledger();
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
        assert_eq!(path, "inbox/2026-10-11T14-03-22-iphone.md");
        assert_eq!(note, &None);
        assert!(removed);
        assert!(!file.exists(), "deleted after the commit");
        let content = fs::read_to_string(s.kb.path().join(path)).unwrap();
        let expected = capture::build_capture(text, SOURCE, &LocalTime::from(written)).unwrap();
        assert_eq!(content, expected.content);
        assert!(content.contains("来源: iphone\n"));
        assert!(content.ends_with("\n\n  第一行\n第二行 ✅ [[链接]]\n"));
        assert_eq!(
            git(s.kb.path(), &["log", "-1", "--format=%s"]),
            "收件箱：记一笔（iphone）"
        );
        let entry = s
            .ledger()
            .get(&ledger::key(
                "记一笔",
                "2026-10-12T14-03-22+08-00.txt",
                format!("\u{FEFF}{text}").as_bytes(),
            ))
            .cloned()
            .unwrap();
        assert!(!entry.pending);
        assert_eq!(&entry.path, path);
        assert_eq!(result.imported_paths(), vec![path.clone()]);
        assert_eq!(result.record_problem, None);
    }

    #[test]
    fn the_time_comes_from_the_file_not_from_a_stamp_in_another_calendar() {
        let s = setup("import-calendar");
        let written = local((2026, 10, 11), (9, 30, 0));
        // A phone set to the Buddhist calendar (2569) and one set to the
        // Japanese calendar (令和 8 = 0008) name their files like this.
        drop_file_at(
            &s.layout.captures(),
            "2569-10-12T09-30-00.txt",
            b"buddhist",
            written.into(),
        );
        drop_file_at(
            &s.layout.answers(),
            "Q-001-0008-10-12T09-30-00.txt",
            b"japanese",
            written.into(),
        );
        let mut ledger = s.ledger();
        let result = pass(&s, &mut ledger, None);
        assert_eq!(
            result.imported_paths(),
            [
                "inbox/2026-10-11T09-30-00-iphone.md",
                "inbox/裁决/Q-001-2026-10-11T09-30-00-iphone.md"
            ]
        );
        assert!(plausible_stamp("2569-10-12T09-30-00.txt").is_none());
        assert!(plausible_stamp("Q-001-0008-10-12T09-30-00.txt").is_none());
        assert!(plausible_stamp("2026-10-12T09-30-00+08-00.txt").is_some());

        // A file time that cannot be true (in the future) is not used either.
        let future = SystemTime::now() + Duration::from_secs(400 * 24 * 3600);
        let path = drop_file_at(&s.layout.captures(), "later.txt", b"x", future);
        let meta = fs::metadata(&path).unwrap();
        let at = capture_time(&meta);
        assert!(at.year <= OffsetDateTime::now_utc().year());
    }

    #[test]
    fn an_answer_to_an_open_ruling_goes_to_the_answer_folder() {
        let s = setup("import-answer");
        drop_file_at(
            &s.layout.answers(),
            "Q-001-2026-10-12T14-05-00+08-00.txt",
            "保留，但只开三十分钟。".as_bytes(),
            local((2026, 10, 11), (14, 5, 0)).into(),
        );
        let mut ledger = s.ledger();
        let result = pass(&s, &mut ledger, None);
        let Outcome::Imported { path, note, .. } = &result.items[0].outcome else {
            panic!("{:?}", result.items);
        };
        assert_eq!(path, "inbox/裁决/Q-001-2026-10-11T14-05-00-iphone.md");
        assert_eq!(note, &None);
        let content = fs::read_to_string(s.kb.path().join(path)).unwrap();
        assert!(content.starts_with("---\n对象: Q-001\n时间: 2026-10-11T14:05:00"));
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
        for (n, name) in ["Q-002-a.txt", "Q-099-b.txt", "随便写的.txt"]
            .iter()
            .enumerate()
        {
            drop_file_at(
                &s.layout.answers(),
                name,
                "我的回答".as_bytes(),
                local((2026, 10, 11), (8, 0, n as u8)).into(),
            );
        }
        let mut ledger = s.ledger();
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
        let mut ledger = s.ledger();
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
        let mut ledger = s.ledger();
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
    fn an_undeletable_file_is_committed_once_across_passes_and_restarts() {
        let s = setup("import-undeletable");
        let file = drop_file(&s.layout.captures(), "stuck.txt", b"stuck");
        let dir = s.layout.captures();
        set_mode(&dir, 0o555);
        let mut ledger = s.ledger();
        let first = pass(&s, &mut ledger, None);
        let second = pass(&s, &mut ledger, None);
        // A restart reads the record again.
        let mut restarted = s.ledger();
        let third = pass(&s, &mut restarted, None);
        set_mode(&dir, 0o755);
        assert!(
            matches!(
                first.items[0].outcome,
                Outcome::Imported { removed: false, .. }
            ),
            "{:?}",
            first.items
        );
        assert_eq!(
            second.items[0].outcome,
            Outcome::AlreadyImported { removed: false }
        );
        assert_eq!(
            third.items[0].outcome,
            Outcome::AlreadyImported { removed: false }
        );
        assert!(file.exists());
        assert_eq!(commit_count(s.kb.path()), 2);
        let result = pass(&s, &mut restarted, None);
        assert_eq!(
            result.items[0].outcome,
            Outcome::AlreadyImported { removed: true }
        );
        assert_eq!(commit_count(s.kb.path()), 2);
    }

    #[test]
    fn nothing_is_imported_while_the_record_cannot_be_written() {
        let s = setup("import-no-record");
        let first = drop_file(&s.layout.captures(), "a.txt", b"first");
        let second = drop_file(&s.layout.captures(), "b.txt", b"second");
        let mut ledger = s.ledger();
        set_mode(s.app.path(), 0o555);
        let result = pass(&s, &mut ledger, None);
        set_mode(s.app.path(), 0o755);
        assert!(
            result
                .record_problem
                .as_deref()
                .is_some_and(|why| why.contains("导入记录写不进去")),
            "{result:?}"
        );
        assert_eq!(result.items.len(), 1, "the pass stops at the first file");
        assert!(matches!(result.items[0].outcome, Outcome::Waiting(_)));
        assert!(
            first.exists() && second.exists(),
            "the phone's files are untouched"
        );
        assert_eq!(commit_count(s.kb.path()), 1);
        assert!(result.imported_paths().is_empty());

        // Writable again: both go in, once.
        let result = pass(&s, &mut ledger, None);
        assert_eq!(result.imported_paths().len(), 2);
        assert_eq!(result.record_problem, None);
        assert_eq!(commit_count(s.kb.path()), 3);
    }

    #[test]
    fn a_crash_between_the_commit_and_the_record_does_not_commit_again() {
        let s = setup("import-crash");
        let name = "2026-10-12T14-03-22.txt";
        let written = local((2026, 10, 11), (14, 3, 22));
        let file = drop_file_at(&s.layout.captures(), name, b"crash", written.into());
        // What a stopped app leaves behind: the pending entry and the commit.
        let built = capture::build_capture("crash", SOURCE, &LocalTime::from(written)).unwrap();
        let key = ledger::key("记一笔", name, b"crash");
        let mut ledger = s.ledger();
        ledger
            .begin(&key, &LocalTime::from(written).iso(), &built.path)
            .unwrap();
        capture::save_capture(s.kb.path(), &ChildEnv::inherit(), &built).unwrap();
        assert_eq!(commit_count(s.kb.path()), 2);

        let mut restarted = s.ledger();
        assert!(restarted.get(&key).unwrap().pending);
        let result = pass(&s, &mut restarted, None);
        assert_eq!(
            result.items[0].outcome,
            Outcome::AlreadyImported { removed: true }
        );
        assert!(!file.exists());
        assert_eq!(commit_count(s.kb.path()), 2);
        let entry = s.ledger().get(&key).cloned().unwrap();
        assert!(!entry.pending);
        assert_eq!(entry.path, built.path);
    }

    #[test]
    fn a_file_created_but_not_committed_is_committed_not_duplicated() {
        let s = setup("import-leftover");
        let name = "2026-10-12T14-03-22.txt";
        let written = local((2026, 10, 11), (14, 3, 22));
        drop_file_at(&s.layout.captures(), name, b"leftover", written.into());
        let built = capture::build_capture("leftover", SOURCE, &LocalTime::from(written)).unwrap();
        s.kb.write(&built.path, &built.content);
        let mut ledger = s.ledger();
        let result = pass(&s, &mut ledger, None);
        assert_eq!(result.imported_paths(), vec![built.path.clone()]);
        assert_eq!(commit_count(s.kb.path()), 2);
        assert_eq!(git(s.kb.path(), &["status", "--porcelain"]), "");
        assert!(!s
            .kb
            .path()
            .join(capture::attempt_path(&built.path, 2))
            .exists());
    }

    #[test]
    fn a_lost_record_does_not_import_twice() {
        let s = setup("import-lost");
        let name = "2026-10-12T14-03-22+08-00.txt";
        let written: SystemTime = local((2026, 10, 11), (20, 0, 0)).into();
        drop_file_at(&s.layout.captures(), name, b"twice?", written);
        let mut ledger = s.ledger();
        pass(&s, &mut ledger, None);
        assert_eq!(commit_count(s.kb.path()), 2);
        // The record breaks; the phone's file comes back (iCloud restored it).
        fs::write(s.app.path().join(ledger::FILE_NAME), "{ cut off").unwrap();
        let mut fresh = s.ledger();
        assert_eq!(fresh.len(), 0);
        drop_file_at(&s.layout.captures(), name, b"twice?", written);
        let result = pass(&s, &mut fresh, None);
        assert_eq!(
            result.items[0].outcome,
            Outcome::AlreadyImported { removed: true }
        );
        assert_eq!(commit_count(s.kb.path()), 2);

        // Imported by an older version, which named the capture by the
        // stamp in the file name, and moved by an ingest since.
        let old = capture::build_capture(
            "older",
            SOURCE,
            &LocalTime::from(
                plausible_stamp("2026-09-01T08-00-00.txt")
                    .unwrap()
                    .assume_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)),
            ),
        )
        .unwrap();
        capture::save_capture(s.kb.path(), &ChildEnv::inherit(), &old).unwrap();
        git(s.kb.path(), &["rm", "-q", "--", &old.path]);
        git(s.kb.path(), &["commit", "-q", "-m", "ingest"]);
        let before = commit_count(s.kb.path());
        drop_file(&s.layout.captures(), "2026-09-01T08-00-00.txt", b"older");
        let result = pass(&s, &mut fresh, None);
        assert_eq!(
            result.items[0].outcome,
            Outcome::AlreadyImported { removed: true }
        );
        assert_eq!(commit_count(s.kb.path()), before);
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
        let mut ledger = s.ledger();
        let mut download = |_: &Path| {};
        let mut retries = Retries::default();
        let mut ctx = Context {
            kb_root: plain.path(),
            env: &env,
            lock: &lock,
            running: &running,
            ledger: &mut ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
            retries: &mut retries,
        };
        let result = run_pass(&scan(&s.layout), &mut ctx);
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
            retries: &mut retries,
        };
        let result = run_pass(&scan(&s.layout), &mut ctx);
        assert!(
            matches!(result.items[0].outcome, Outcome::Waiting(ref why) if why.contains("知识库正忙")),
            "{:?}",
            result.items
        );
        assert!(file.exists());
        drop(held);
        let result = run_pass(&scan(&s.layout), &mut ctx);
        assert!(matches!(result.items[0].outcome, Outcome::Imported { .. }));
        assert!(!file.exists());
        assert_eq!(commit_count(s.kb.path()), 2);
    }

    #[test]
    fn unfinished_and_foreign_files_are_left_alone() {
        let s = setup("import-unfinished");
        // Just written: waits.
        let fresh = s.layout.captures().join("fresh.txt");
        fs::write(&fresh, b"still syncing").unwrap();
        // Not text, not UTF-8, empty, hidden, placeholders, a folder.
        let pdf = drop_file(&s.layout.captures(), "scan.pdf", b"%PDF");
        let latin = drop_file(&s.layout.captures(), "latin.txt", b"caf\xe9");
        let empty = drop_file(
            &s.layout.captures(),
            "empty.txt",
            " \n\u{3000}\n".as_bytes(),
        );
        drop_file(&s.layout.captures(), ".DS_Store", b"x");
        drop_file(&s.layout.captures(), ".later.txt.icloud", b"placeholder");
        drop_file(&s.layout.captures(), ".photo.jpg.icloud", b"placeholder");
        fs::create_dir_all(s.layout.captures().join("folder")).unwrap();

        let downloads = RefCell::new(Vec::new());
        let env = ChildEnv::inherit();
        let lock = KbLock::default();
        let running = || None;
        let mut ledger = s.ledger();
        let mut download = |path: &Path| downloads.borrow_mut().push(path.to_path_buf());
        let mut retries = Retries::default();
        let mut ctx = Context {
            kb_root: s.kb.path(),
            env: &env,
            lock: &lock,
            running: &running,
            ledger: &mut ledger,
            settle: SETTLE,
            lock_wait: Duration::ZERO,
            download: &mut download,
            retries: &mut retries,
        };
        let result = run_pass(&scan(&s.layout), &mut ctx);
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
        for hidden in [".DS_Store", "folder", ".photo.jpg.icloud"] {
            assert!(!by_name.contains_key(hidden), "{hidden}");
        }
        assert_eq!(downloads.borrow().len(), 1);
        assert!(fresh.exists() && pdf.exists() && latin.exists() && !empty.exists());
        assert_eq!(commit_count(s.kb.path()), 1);
        drop(s.bridge);
    }

    #[test]
    fn a_placeholder_that_never_downloads_stops_the_quick_retries() {
        let s = setup("import-stuck-placeholder");
        drop_file(&s.layout.captures(), ".stuck.txt.icloud", b"placeholder");
        let mut ledger = s.ledger();
        let mut retries = Retries::default();
        for _ in 0..QUICK_RETRIES {
            assert!(pass_with(&s, &mut ledger, &mut retries, None).retry_soon);
        }
        for _ in 0..3 {
            let result = pass_with(&s, &mut ledger, &mut retries, None);
            assert!(!result.retry_soon, "back to the normal cadence");
            assert_eq!(result.items[0].outcome, Outcome::Settling);
        }
        // A new unfinished file still gets its quick retries.
        let fresh = s.layout.captures().join("new.txt");
        fs::write(&fresh, b"syncing").unwrap();
        assert!(pass_with(&s, &mut ledger, &mut retries, None).retry_soon);
        // Once the placeholder is gone, its count is forgotten.
        fs::remove_file(s.layout.captures().join(".stuck.txt.icloud")).unwrap();
        pass_with(&s, &mut ledger, &mut retries, None);
        assert_eq!(retries.0.len(), 1);
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
        assert!(stamp_in_name("Q-001-2026-10-12T14-03-22+08-00.txt").is_some());
    }
}
