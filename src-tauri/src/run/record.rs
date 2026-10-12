//! What the app keeps about each run, in a folder of its own under the app's
//! data folder:
//!
//! ```text
//! runs/2026-10-12T14-30-05-ingest/
//!   run.json            what was asked, how it went, cost, the gate's report
//!   events.jsonl        the engine's output, line by line as it arrived
//!   steps.jsonl         the steps the windows were shown, one JSON object per line
//!   snapshot.json       where the run started (needed to discard or restore it)
//!   stderr.txt          the end of the engine's error output, if any
//!   final-message.txt   Codex's last reply
//!   discarded/          copies of the run's files, when it was discarded
//! ```
//!
//! These folders hold knowledge-base content and stay on this machine. The
//! newest [`KEEP_RUNS`] are kept.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::gate::GateReport;
use super::process::Exit;
use super::stream::StreamSummary;
use super::{EngineKind, Task};
use crate::kb::capture::LocalTime;
use crate::kb::changes::Snapshot;
use crate::kb::files;
use crate::kb::lock::NotedCommit;

/// How many run folders are kept.
pub const KEEP_RUNS: usize = 20;
pub const RUN_FILE: &str = "run.json";
pub const EVENTS_FILE: &str = "events.jsonl";
pub const STEPS_FILE: &str = "steps.jsonl";
pub const SNAPSHOT_FILE: &str = "snapshot.json";
pub const STDERR_FILE: &str = "stderr.txt";
pub const FINAL_MESSAGE_FILE: &str = "final-message.txt";
pub const DISCARDED_DIR: &str = "discarded";

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// No run is going on (only in a status, never in a record).
    Idle,
    /// Syncing and looking at the queue; the engine has not started.
    Starting,
    /// The engine is working.
    Running,
    /// The engine is done; the changes are being checked.
    Checking,
    /// Committing, or discarding.
    Committing,
    /// The check did not pass: the changes wait for a person.
    NeedsDecision,
    Finished,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The check passed and the result was committed.
    Committed,
    /// Committed on request although the check did not pass.
    Kept,
    /// Everything the run changed was undone on request.
    Discarded,
    /// The queue was empty: the engine never started.
    NothingToDo,
    /// A precondition was not met: the engine never started.
    Refused,
    /// The engine failed or changed nothing.
    Failed,
    /// Stopped on request before it changed anything.
    Cancelled,
}

/// How the engine did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineReport {
    /// It finished normally.
    pub ok: bool,
    /// Why not, in plain words.
    pub failure: Option<String>,
    pub exit: Exit,
    pub stream: StreamSummary,
    /// How long it ran, as measured by the app.
    pub duration_ms: u64,
}

/// `run.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    /// Also the name of the run's folder.
    pub id: String,
    pub task: Task,
    pub engine: EngineKind,
    /// The model asked for; `None` for the engine's default.
    pub model: Option<String>,
    /// What started it: `ui` or `command line`.
    pub trigger: String,
    /// Local date the run works under, `YYYY-MM-DD`.
    pub date: String,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub phase: Phase,
    pub outcome: Option<Outcome>,
    /// One line for the user: what happened, or what is waited for.
    pub message: Option<String>,
    /// The queue files this run is about.
    pub queue: Vec<String>,
    /// The commit the run started from.
    pub base: Option<String>,
    pub engine_report: Option<EngineReport>,
    /// The latest answer of the gate (with what the app itself found added).
    pub gate: Option<GateReport>,
    /// The violations at the moment a person chose to keep the result anyway.
    pub kept_despite: Vec<String>,
    /// The commit of the run.
    pub commit: Option<String>,
    /// Where the copies of a discarded run are.
    pub discarded_to: Option<String>,
}

impl RunRecord {
    /// A run that has just been asked for.
    pub fn new(
        id: String,
        task: Task,
        engine: EngineKind,
        model: Option<String>,
        trigger: &str,
        date: String,
    ) -> Self {
        RunRecord {
            id,
            task,
            engine,
            model,
            trigger: trigger.to_string(),
            date,
            started_at: unix_millis(),
            ended_at: None,
            phase: Phase::Starting,
            outcome: None,
            message: None,
            queue: Vec::new(),
            base: None,
            engine_report: None,
            gate: None,
            kept_despite: Vec::new(),
            commit: None,
            discarded_to: None,
        }
    }
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// A new folder for a run that starts at `at`, named after the time and the
/// task (`2026-10-12T14-30-05-ingest`); a second run in the same second gets
/// `-2`. Returns the id (the folder's name).
pub fn create_run_folder(runs_dir: &Path, at: &LocalTime, task: Task) -> io::Result<String> {
    fs::create_dir_all(runs_dir)?;
    let base = format!("{}-{}", at.file_stamp(), task.slug());
    for attempt in 1..=50 {
        let id = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        match fs::create_dir(runs_dir.join(&id)) {
            Ok(()) => return Ok(id),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many runs started in the same second",
    ))
}

fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    files::write_atomic(path, &text)
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn save(folder: &Path, record: &RunRecord) -> io::Result<()> {
    save_json(&folder.join(RUN_FILE), record)
}

pub fn load(folder: &Path) -> Option<RunRecord> {
    load_json(&folder.join(RUN_FILE))
}

/// Where a run started, and the commits the app made beside it so far:
/// together they are what it takes to check, commit or discard the run later,
/// also after the app was restarted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSnapshot {
    /// The knowledge-base folder the run works on (canonical). The run
    /// folders belong to the app, not to one knowledge base: a run is only
    /// ever picked up again in the folder it started in.
    pub root: PathBuf,
    pub snapshot: Snapshot,
    pub beside: Vec<NotedCommit>,
}

pub fn save_snapshot(folder: &Path, saved: &SavedSnapshot) -> io::Result<()> {
    save_json(&folder.join(SNAPSHOT_FILE), saved)
}

pub fn load_snapshot(folder: &Path) -> Option<SavedSnapshot> {
    load_json(&folder.join(SNAPSHOT_FILE))
}

/// The run folders, newest first.
fn folders(runs_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    // The names start with the time, so the order of the names is the order in time.
    folders.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    folders
}

/// The records of the kept runs, newest first.
pub fn history(runs_dir: &Path) -> Vec<RunRecord> {
    folders(runs_dir)
        .iter()
        .filter_map(|folder| load(folder))
        .collect()
}

/// The newest run that did not finish, with its folder: the one a restarted
/// app has to pick up again.
pub fn unfinished(runs_dir: &Path) -> Option<(PathBuf, RunRecord)> {
    folders(runs_dir).into_iter().find_map(|folder| {
        let record = load(&folder)?;
        (record.phase != Phase::Finished).then_some((folder, record))
    })
}

/// Deletes all but the `keep` newest run folders. A run that has not
/// finished (its result may still be waiting for a decision) is never
/// deleted. Returns how many folders were removed.
pub fn prune(runs_dir: &Path, keep: usize) -> usize {
    folders(runs_dir)
        .into_iter()
        .skip(keep)
        .filter(|folder| load(folder).is_none_or(|record| record.phase == Phase::Finished))
        .filter(|folder| fs::remove_dir_all(folder).is_ok())
        .count()
}

/// True for the name of a run folder as [`create_run_folder`] makes them
/// (`2026-10-12T14-30-05-ingest`, `…-2`): anything else (a path, `..`) is
/// never looked up.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && !id.starts_with(['-', '.'])
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The folder of the kept run `id`, if `id` is a run's name and the folder exists.
pub fn folder_of(runs_dir: &Path, id: &str) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    let folder = runs_dir.join(id);
    folder.is_dir().then_some(folder)
}

/// Appends one JSON object as a line to `file` (created when missing). A line
/// that cannot be written is lost; the run goes on.
pub fn append_json_line<T: Serialize>(file: &Path, value: &T) {
    let Ok(mut line) = serde_json::to_string(value) else {
        return;
    };
    line.push('\n');
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut out| out.write_all(line.as_bytes()));
    if let Err(error) = written {
        log::warn!("run: could not append to {}: {error}", file.display());
    }
}

/// The JSON lines of `file` that read as `T`; other lines are skipped.
pub fn read_json_lines<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<Vec<T>> {
    let text = fs::read_to_string(file).ok()?;
    Some(
        text.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect(),
    )
}

/// Appends the engine's output lines to `events.jsonl`.
pub struct EventLog(Option<BufWriter<File>>);

impl EventLog {
    /// A log that cannot be opened loses the lines but never stops the run.
    pub fn open(folder: &Path) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join(EVENTS_FILE));
        if let Err(error) = &file {
            log::warn!("run: could not open the event log: {error}");
        }
        EventLog(file.ok().map(BufWriter::new))
    }

    pub fn append(&mut self, line: &str) {
        if let Some(file) = self.0.as_mut() {
            let written = file
                .write_all(line.as_bytes())
                .and_then(|()| file.write_all(b"\n"));
            if written.is_err() {
                self.0 = None;
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(file) = self.0.as_mut() {
            let _ = file.flush();
        }
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use time::macros::datetime;

    fn at() -> LocalTime {
        LocalTime::from(datetime!(2026-10-12 14:30:05 +08:00))
    }

    fn record(id: &str, phase: Phase) -> RunRecord {
        let mut record = RunRecord::new(
            id.to_string(),
            Task::Ingest,
            EngineKind::Claude,
            Some("sonnet".into()),
            "command line",
            "2026-10-12".into(),
        );
        record.phase = phase;
        record
    }

    #[test]
    fn run_folders_are_named_after_the_time_and_the_task() {
        let dir = TempDir::new("record-folder");
        let runs = dir.path().join("runs");
        let first = create_run_folder(&runs, &at(), Task::Ingest).unwrap();
        let second = create_run_folder(&runs, &at(), Task::Ingest).unwrap();
        let rulings = create_run_folder(&runs, &at(), Task::LandRulings).unwrap();
        assert_eq!(first, "2026-10-12T14-30-05-ingest");
        assert_eq!(second, "2026-10-12T14-30-05-ingest-2");
        assert_eq!(rulings, "2026-10-12T14-30-05-land-rulings");
        assert!(runs.join(&second).is_dir());
    }

    #[test]
    fn records_round_trip_as_camel_case_json() {
        let dir = TempDir::new("record-json");
        let mut saved = record("2026-10-12T14-30-05-ingest", Phase::Finished);
        saved.outcome = Some(Outcome::Committed);
        saved.queue = vec!["inbox/a.md".into()];
        saved.commit = Some("abc".into());
        saved.engine_report = Some(EngineReport {
            ok: true,
            failure: None,
            exit: Exit {
                code: Some(0),
                ..Exit::default()
            },
            stream: StreamSummary {
                cost_usd: Some(0.42),
                turns: Some(30),
                ..StreamSummary::default()
            },
            duration_ms: 95_000,
        });
        save(dir.path(), &saved).unwrap();
        assert_eq!(load(dir.path()), Some(saved));

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(RUN_FILE)).unwrap()).unwrap();
        assert_eq!(json["task"], "ingest");
        assert_eq!(json["engine"], "claude");
        assert_eq!(json["phase"], "finished");
        assert_eq!(json["outcome"], "committed");
        assert_eq!(json["trigger"], "command line");
        assert_eq!(json["engineReport"]["stream"]["costUsd"], 0.42);
        assert_eq!(json["engineReport"]["durationMs"], 95_000);
        assert_eq!(json["keptDespite"], serde_json::json!([]));
        assert!(json["startedAt"].as_u64().unwrap() > 1_700_000_000_000);
        assert_eq!(load(&dir.path().join("missing")), None);
    }

    fn make(runs: &Path, id: &str, phase: Phase) {
        let folder = runs.join(id);
        fs::create_dir_all(&folder).unwrap();
        save(&folder, &record(id, phase)).unwrap();
    }

    #[test]
    fn keeps_the_newest_runs_and_never_deletes_an_unfinished_one() {
        let dir = TempDir::new("record-prune");
        let runs = dir.path();
        for day in 1..=9 {
            make(runs, &format!("2026-10-0{day}T10-00-00-ingest"), Phase::Finished);
        }
        // An old run whose result still waits for a decision.
        make(runs, "2026-09-30T10-00-00-ingest", Phase::NeedsDecision);
        // A folder without a record is just an old folder.
        fs::create_dir_all(runs.join("2026-09-01T10-00-00-ingest")).unwrap();

        assert_eq!(
            history(runs)
                .iter()
                .map(|record| record.id.as_str())
                .take(2)
                .collect::<Vec<_>>(),
            ["2026-10-09T10-00-00-ingest", "2026-10-08T10-00-00-ingest"]
        );
        assert_eq!(
            unfinished(runs).map(|(_, record)| record.id).as_deref(),
            Some("2026-09-30T10-00-00-ingest")
        );

        assert_eq!(prune(runs, 3), 7);
        let left: Vec<String> = folders(runs)
            .iter()
            .map(|folder| folder.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            [
                "2026-10-09T10-00-00-ingest",
                "2026-10-08T10-00-00-ingest",
                "2026-10-07T10-00-00-ingest",
                "2026-09-30T10-00-00-ingest",
            ]
        );
        assert_eq!(prune(runs, 3), 0);
        assert_eq!(prune(&runs.join("missing"), 3), 0);
    }

    #[test]
    fn the_event_log_appends_lines() {
        let dir = TempDir::new("record-events");
        {
            let mut log = EventLog::open(dir.path());
            log.append(r#"{"type":"system"}"#);
            log.append("not json, kept anyway");
        }
        {
            let mut log = EventLog::open(dir.path());
            log.append(r#"{"type":"result"}"#);
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(EVENTS_FILE)).unwrap(),
            "{\"type\":\"system\"}\nnot json, kept anyway\n{\"type\":\"result\"}\n"
        );
        // A folder that does not exist: the lines are dropped, nothing fails.
        let mut nowhere = EventLog::open(&dir.path().join("missing"));
        nowhere.append("x");
    }

    #[test]
    fn only_run_names_are_looked_up() {
        let dir = TempDir::new("record-ids");
        let runs = dir.path().join("runs");
        let id = create_run_folder(&runs, &at(), Task::Ingest).unwrap();
        assert_eq!(folder_of(&runs, &id), Some(runs.join(&id)));
        assert!(valid_id("2026-10-12T14-30-05-land-rulings-2"));
        for bad in ["", "..", "../runs", "a/b", ".hidden", "-x", "运行", "a b", &"x".repeat(81)] {
            assert!(!valid_id(bad), "{bad:?}");
            assert_eq!(folder_of(&runs, bad), None, "{bad:?}");
        }
        assert_eq!(folder_of(&runs, "2026-01-01T00-00-00-ingest"), None);
    }

    #[test]
    fn json_lines_are_appended_and_read_back() {
        let dir = TempDir::new("record-lines");
        let file = dir.path().join(STEPS_FILE);
        assert_eq!(read_json_lines::<serde_json::Value>(&file), None);
        append_json_line(&file, &serde_json::json!({ "seq": 1 }));
        fs::write(&file, format!("{}not json\n", fs::read_to_string(&file).unwrap())).unwrap();
        append_json_line(&file, &serde_json::json!({ "seq": 2 }));
        let lines: Vec<serde_json::Value> = read_json_lines(&file).unwrap();
        assert_eq!(lines, [serde_json::json!({ "seq": 1 }), serde_json::json!({ "seq": 2 })]);
        // A folder that does not exist: nothing is written, nothing fails.
        append_json_line(&dir.path().join("missing").join(STEPS_FILE), &1);
    }

    #[test]
    fn snapshots_round_trip() {
        let dir = TempDir::new("record-snapshot");
        let saved = SavedSnapshot {
            root: PathBuf::from("/tmp/kb"),
            snapshot: Snapshot {
                base: "abc".into(),
                branch: "main".into(),
                queue: Vec::new(),
                untracked: [("草稿.md".to_string(), "0f".to_string())].into(),
                dirs: ["inbox".to_string()].into(),
            },
            beside: vec![NotedCommit {
                commit: "def".into(),
                path: "inbox/b.md".into(),
            }],
        };
        save_snapshot(dir.path(), &saved).unwrap();
        assert_eq!(load_snapshot(dir.path()), Some(saved));
    }
}
