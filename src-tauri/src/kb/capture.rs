//! Building a quick capture (记一笔) on the Rust side, for captures that do
//! not come from a window: `--capture "<text>"` on the command line.
//!
//! This mirrors `src/domain/capture.ts` exactly. Both are tested against the
//! same cases in `src/domain/__fixtures__/capture-cases.json`, so a capture
//! looks the same wherever it was made:
//!
//! - path `inbox/YYYY-MM-DDTHH-mm-ss-<source>.md` in local time;
//! - YAML frontmatter with `来源` and `时间` (ISO 8601, local time with its
//!   offset), a blank line, then the text verbatim: only CRLF becomes LF and
//!   the text ends with exactly one newline;
//! - commit message `收件箱：记一笔（<source>）`.
//!
//! Answers to pending rulings (`--answer Q-004 "<text>"`) are built here too,
//! like `buildAnswerAt` in `src/domain/answers.ts`: both are tested against
//! `src/domain/__fixtures__/answer-cases.json`.

use std::path::Path;

use time::OffsetDateTime;

use super::git::WriteOutcome;
use super::ops::{self, CreateError};
use crate::shell_env::ChildEnv;

/// Where captures can come from; the desktop app itself is `mac`.
pub const SOURCES: &[&str] = &["mac", "iphone", "web"];
/// The source of everything captured in the desktop app.
pub const MAC: &str = "mac";
/// How many file names are tried when captures collide within one second.
pub const MAX_ATTEMPTS: u32 = 20;

/// A wall-clock time with its offset from UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(rename_all = "camelCase"))]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Minutes east of UTC (480 for +08:00).
    pub offset_minutes: i32,
}

impl LocalTime {
    /// The current local time (UTC when the system offset cannot be read).
    pub fn now() -> Self {
        Self::from(OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()))
    }

    /// `2026-10-12`.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `2026-10-12T14-03-22`, the time part of a capture file name.
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `2026-10-12T14:03:22+08:00`.
    pub fn iso(&self) -> String {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let offset = self.offset_minutes.unsigned_abs();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            offset / 60,
            offset % 60
        )
    }
}

impl From<OffsetDateTime> for LocalTime {
    fn from(time: OffsetDateTime) -> Self {
        LocalTime {
            year: time.year(),
            month: u8::from(time.month()),
            day: time.day(),
            hour: time.hour(),
            minute: time.minute(),
            second: time.second(),
            offset_minutes: i32::from(time.offset().whole_minutes()),
        }
    }
}

/// One capture, ready to be created in the knowledge base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub path: String,
    pub content: String,
    pub message: String,
}

/// Builds the file for one capture. Fails for empty or whitespace-only text
/// and for an unknown source.
pub fn build_capture(text: &str, source: &str, at: &LocalTime) -> Result<Capture, String> {
    if !SOURCES.contains(&source) {
        return Err(format!("不认识的来源：{source}"));
    }
    if text.chars().all(is_blank) {
        return Err("内容是空的，没有可以记下的东西".to_string());
    }
    let body = text.replace("\r\n", "\n");
    let body = body.trim_end_matches('\n');
    Ok(Capture {
        path: format!("inbox/{}-{source}.md", at.file_stamp()),
        content: format!("---\n来源: {source}\n时间: {}\n---\n\n{body}\n", at.iso()),
        message: format!("收件箱：记一笔（{source}）"),
    })
}

/// True for the id of a pending ruling: `Q-` and digits.
fn is_ruling_id(id: &str) -> bool {
    id.strip_prefix("Q-")
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// Builds the file for an answer to the pending ruling `id` (`Q-004`):
/// `inbox/裁决/Q-004-<time>-<source>.md` with `对象`, `时间` and `来源`, then
/// the text verbatim like a capture's; commit message `收件箱：回答 Q-004（mac）`.
pub fn build_answer(id: &str, text: &str, source: &str, at: &LocalTime) -> Result<Capture, String> {
    if !SOURCES.contains(&source) {
        return Err(format!("不认识的来源：{source}"));
    }
    if !is_ruling_id(id) {
        return Err(format!("不是待裁决的编号：{id}（要写成 Q-004 这样）"));
    }
    if text.chars().all(is_blank) {
        return Err("还没写字，没有可以记下的回答".to_string());
    }
    let body = text.replace("\r\n", "\n");
    let body = body.trim_end_matches('\n');
    Ok(Capture {
        path: format!("inbox/裁决/{id}-{}-{source}.md", at.file_stamp()),
        content: format!(
            "---\n对象: {id}\n时间: {}\n来源: {source}\n---\n\n{body}\n",
            at.iso()
        ),
        message: format!("收件箱：回答 {id}（{source}）"),
    })
}

/// What JavaScript's `String.prototype.trim` removes (besides U+2000 to
/// U+200A), so the Rust and the TypeScript side agree on which texts are empty.
const BLANKS: &[char] = &[
    '\t', '\n', '\u{000B}', '\u{000C}', '\r', ' ', '\u{00A0}', '\u{1680}', '\u{2028}', '\u{2029}',
    '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}',
];

fn is_blank(c: char) -> bool {
    BLANKS.contains(&c) || ('\u{2000}'..='\u{200A}').contains(&c)
}

/// True for empty or whitespace-only text: what can be neither a capture
/// nor an answer.
pub fn is_blank_text(text: &str) -> bool {
    text.chars().all(is_blank)
}

/// The path tried on the `attempt`-th try: the first one is `path` itself,
/// later ones get `-2`, `-3`, … before the extension.
pub fn attempt_path(path: &str, attempt: u32) -> String {
    if attempt <= 1 {
        return path.to_string();
    }
    match path.strip_suffix(".md") {
        Some(stem) => format!("{stem}-{attempt}.md"),
        None => format!("{path}-{attempt}"),
    }
}

/// A capture that was created and committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCapture {
    pub path: String,
    pub outcome: WriteOutcome,
}

/// Creates and commits the capture. When a file with its name already exists
/// (two captures within one second), the next free name is used; nothing is
/// ever overwritten.
pub fn save_capture(
    root: &Path,
    env: &ChildEnv,
    capture: &Capture,
) -> Result<SavedCapture, String> {
    for attempt in 1..=MAX_ATTEMPTS {
        let path = attempt_path(&capture.path, attempt);
        match ops::create_file(root, env, &path, &capture.content, &capture.message) {
            Ok(outcome) => return Ok(SavedCapture { path, outcome }),
            Err(CreateError::Exists(_)) => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(format!(
        "同一秒里已经有 {MAX_ATTEMPTS} 条随手记了，这一条没有记下：{}",
        capture.path
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::{commit_count, git, init_repo, TempDir};
    use serde::Deserialize;
    use time::macros::datetime;

    #[derive(Deserialize)]
    struct Cases {
        accept: Vec<Accept>,
        reject: Vec<String>,
        suffixes: Vec<Suffix>,
    }

    #[derive(Deserialize)]
    struct Accept {
        name: String,
        text: String,
        source: String,
        at: LocalTime,
        path: String,
        content: String,
        message: String,
    }

    #[derive(Deserialize)]
    struct Suffix {
        path: String,
        attempt: u32,
        expected: String,
    }

    /// The cases the TypeScript implementation is tested against as well.
    fn cases() -> Cases {
        serde_json::from_str(include_str!(
            "../../../src/domain/__fixtures__/capture-cases.json"
        ))
        .expect("capture-cases.json")
    }

    #[test]
    fn builds_the_same_files_as_the_typescript_side() {
        let cases = cases();
        assert!(cases.accept.len() >= 6);
        for case in cases.accept {
            let built = build_capture(&case.text, &case.source, &case.at)
                .unwrap_or_else(|error| panic!("{}: {error}", case.name));
            assert_eq!(built.path, case.path, "{}", case.name);
            assert_eq!(built.content, case.content, "{}", case.name);
            assert_eq!(built.message, case.message, "{}", case.name);
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AnswerCases {
        accept: Vec<AnswerCase>,
        reject_ids: Vec<String>,
    }

    #[derive(Deserialize)]
    struct AnswerCase {
        name: String,
        id: String,
        text: String,
        source: String,
        at: LocalTime,
        path: String,
        content: String,
        message: String,
    }

    #[test]
    fn builds_the_same_answers_as_the_typescript_side() {
        let cases: AnswerCases = serde_json::from_str(include_str!(
            "../../../src/domain/__fixtures__/answer-cases.json"
        ))
        .expect("answer-cases.json");
        for case in cases.accept {
            let built = build_answer(&case.id, &case.text, &case.source, &case.at)
                .unwrap_or_else(|error| panic!("{}: {error}", case.name));
            assert_eq!(built.path, case.path, "{}", case.name);
            assert_eq!(built.content, case.content, "{}", case.name);
            assert_eq!(built.message, case.message, "{}", case.name);
        }
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));
        for id in cases.reject_ids {
            assert!(build_answer(&id, "x", MAC, &at).is_err(), "{id:?}");
        }
        assert!(build_answer("Q-004", " \n ", MAC, &at)
            .unwrap_err()
            .contains("还没写字"));
        assert!(build_answer("Q-004", "x", "fax", &at)
            .unwrap_err()
            .contains("来源"));
    }

    #[test]
    fn refuses_empty_and_whitespace_only_text() {
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));
        let cases = cases();
        assert!(cases.reject.len() >= 8);
        for text in cases.reject {
            let error = build_capture(&text, MAC, &at).unwrap_err();
            assert!(error.contains("空"), "{text:?}: {error}");
        }
        assert!(build_capture("有内容", "fax", &at)
            .unwrap_err()
            .contains("来源"));
    }

    #[test]
    fn numbers_the_file_names_of_later_attempts() {
        for case in cases().suffixes {
            assert_eq!(attempt_path(&case.path, case.attempt), case.expected);
        }
        assert_eq!(attempt_path("inbox/a.md", 0), "inbox/a.md");
    }

    #[test]
    fn local_time_comes_from_the_clock_with_its_offset() {
        let east = LocalTime::from(datetime!(2026-10-12 14:03:22.987 +08:00));
        assert_eq!(east.file_stamp(), "2026-10-12T14-03-22");
        assert_eq!(east.date(), "2026-10-12");
        assert_eq!(east.iso(), "2026-10-12T14:03:22+08:00");
        let west = LocalTime::from(datetime!(2026-01-02 03:04:05 -03:30));
        assert_eq!(west.offset_minutes, -210);
        assert_eq!(west.iso(), "2026-01-02T03:04:05-03:30");
        let utc = LocalTime::from(datetime!(2026-01-02 03:04:05 UTC));
        assert_eq!(utc.iso(), "2026-01-02T03:04:05+00:00");

        let now = LocalTime::now();
        assert!(now.year >= 2026);
        assert!((-12 * 60..=14 * 60).contains(&now.offset_minutes));
    }

    fn kb(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        init_repo(dir.path());
        dir.write("index.md", "# 目录\n");
        git(dir.path(), &["add", "--", "index.md"]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn saves_a_capture_as_one_commit_of_one_file() {
        let kb = kb("capture-save");
        let env = ChildEnv::inherit();
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));
        let capture = build_capture("第一条\n  第二行", MAC, &at).unwrap();

        let saved = save_capture(kb.path(), &env, &capture).unwrap();
        assert_eq!(saved.path, "inbox/2026-10-12T14-03-22-mac.md");
        assert!(saved.outcome.committed);
        assert_eq!(commit_count(kb.path()), 2);
        assert_eq!(
            std::fs::read_to_string(kb.path().join(&saved.path)).unwrap(),
            "---\n来源: mac\n时间: 2026-10-12T14:03:22+08:00\n---\n\n第一条\n  第二行\n"
        );
        assert_eq!(
            git(kb.path(), &["log", "-1", "--format=%s"]),
            "收件箱：记一笔（mac）"
        );
        assert_eq!(
            git(kb.path(), &["show", "--name-only", "--format=", "HEAD"]),
            "inbox/2026-10-12T14-03-22-mac.md"
        );
        assert_eq!(git(kb.path(), &["status", "--porcelain"]), "");
    }

    #[test]
    fn a_second_capture_in_the_same_second_gets_a_numbered_name() {
        let kb = kb("capture-collide");
        let env = ChildEnv::inherit();
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));

        let first = save_capture(kb.path(), &env, &build_capture("一", MAC, &at).unwrap()).unwrap();
        let second =
            save_capture(kb.path(), &env, &build_capture("二", MAC, &at).unwrap()).unwrap();
        let third = save_capture(kb.path(), &env, &build_capture("三", MAC, &at).unwrap()).unwrap();
        assert_eq!(first.path, "inbox/2026-10-12T14-03-22-mac.md");
        assert_eq!(second.path, "inbox/2026-10-12T14-03-22-mac-2.md");
        assert_eq!(third.path, "inbox/2026-10-12T14-03-22-mac-3.md");
        assert_eq!(commit_count(kb.path()), 4);
        // The first one was not overwritten.
        assert!(std::fs::read_to_string(kb.path().join(&first.path))
            .unwrap()
            .ends_with("\n\n一\n"));
        assert!(std::fs::read_to_string(kb.path().join(&second.path))
            .unwrap()
            .ends_with("\n\n二\n"));
    }

    #[test]
    fn gives_up_when_every_name_is_taken() {
        let kb = kb("capture-full");
        let env = ChildEnv::inherit();
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));
        let capture = build_capture("挤不进去", MAC, &at).unwrap();
        for attempt in 1..=MAX_ATTEMPTS {
            kb.write(&attempt_path(&capture.path, attempt), "占位\n");
        }
        let error = save_capture(kb.path(), &env, &capture).unwrap_err();
        assert!(error.contains("没有记下"), "{error}");
        assert_eq!(commit_count(kb.path()), 1);
    }

    #[test]
    fn other_failures_are_reported_without_trying_more_names() {
        let plain = TempDir::new("capture-not-a-repo");
        let at = LocalTime::from(datetime!(2026-10-12 14:03:22 +08:00));
        let capture = build_capture("没有仓库", MAC, &at).unwrap();
        let error = save_capture(plain.path(), &ChildEnv::inherit(), &capture).unwrap_err();
        assert!(error.contains("不是 git 仓库"), "{error}");
        assert!(!plain.path().join("inbox").exists());
    }
}
