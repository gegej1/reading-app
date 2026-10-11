//! AI runs: 入库 (ingest) and 落裁决 (land rulings).
//!
//! An engine (Claude Code or Codex) works on the knowledge base as a child
//! process and only edits files there. The app decides what becomes of the
//! result: it lists the changes, has the gate check them (TypeScript,
//! `src/domain/gate.ts`, run by the main webview), and only then deletes the
//! processed queue files, commits and syncs. A run that fails the check stays
//! in the working tree until a person discards or keeps it.
//!
//! - `settings`: `engine`, `model`, `budgetUsd`, … in `config.json`;
//! - `prompt`: the task prompts (`runners/*.md`, embedded);
//! - `engine`: the exact command lines, and the temporary auth-only settings
//!   file for Claude Code;
//! - `process`: running the engine with cancel and a time limit;
//! - `stream`: the engine's JSON lines turned into progress;
//! - `gate`: what the gate gets and answers, and the two rules that are
//!   checked again here before a commit;
//! - `record`: the per-run folder (`runs/<id>/events.jsonl`, `run.json`);
//! - `service`: the lifecycle of a run.
//!
//! Like `crate::kb`, nothing in here knows Tauri: `crate::run_commands`
//! exposes it to the windows and the command line.

pub mod engine;
pub mod gate;
pub mod process;
pub mod prompt;
pub mod record;
pub mod service;
pub mod settings;
pub mod stream;

use serde::{Deserialize, Serialize};

/// What a run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Task {
    /// 入库: work through the captures in `inbox/`.
    Ingest,
    /// 落裁决: work through the answers in `inbox/裁决/`.
    LandRulings,
}

impl Task {
    /// What the user calls it.
    pub fn label(self) -> &'static str {
        match self {
            Task::Ingest => "入库",
            Task::LandRulings => "落裁决",
        }
    }

    /// The folder whose Markdown files are this task's queue.
    pub fn queue_dir(self) -> &'static str {
        match self {
            Task::Ingest => "inbox",
            Task::LandRulings => "inbox/裁决",
        }
    }

    /// For folder names.
    pub fn slug(self) -> &'static str {
        match self {
            Task::Ingest => "ingest",
            Task::LandRulings => "land-rulings",
        }
    }

    /// `3 条` captures, `2 份` answers.
    pub fn unit(self) -> &'static str {
        match self {
            Task::Ingest => "条",
            Task::LandRulings => "份",
        }
    }

    /// What an empty queue is called.
    pub fn empty_queue(self) -> &'static str {
        match self {
            Task::Ingest => "收件箱是空的，没有要入库的随手记",
            Task::LandRulings => "没有待落的裁决（inbox/裁决/ 是空的）",
        }
    }

    pub fn parse(text: &str) -> Option<Task> {
        match text {
            "ingest" => Some(Task::Ingest),
            "landRulings" | "land-rulings" => Some(Task::LandRulings),
            _ => None,
        }
    }
}

/// Which command-line tool does the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    Claude,
    Codex,
}

impl EngineKind {
    /// As it appears in commit messages: `入库：3 条（Claude）`.
    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Claude => "Claude",
            EngineKind::Codex => "Codex",
        }
    }

    /// The program looked up on the user's `PATH`.
    pub fn program(self) -> &'static str {
        match self {
            EngineKind::Claude => "claude",
            EngineKind::Codex => "codex",
        }
    }

    pub fn parse(text: &str) -> Option<EngineKind> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(EngineKind::Claude),
            "codex" => Some(EngineKind::Codex),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_and_engines_have_their_names() {
        assert_eq!(Task::Ingest.queue_dir(), "inbox");
        assert_eq!(Task::LandRulings.queue_dir(), "inbox/裁决");
        assert_eq!(Task::parse("ingest"), Some(Task::Ingest));
        assert_eq!(Task::parse("landRulings"), Some(Task::LandRulings));
        assert_eq!(Task::parse("land-rulings"), Some(Task::LandRulings));
        assert_eq!(Task::parse("sync"), None);
        assert_eq!(EngineKind::parse(" Codex "), Some(EngineKind::Codex));
        assert_eq!(EngineKind::parse("claude"), Some(EngineKind::Claude));
        assert_eq!(EngineKind::parse("gpt"), None);
        // The names the frontend sees.
        assert_eq!(
            serde_json::to_string(&Task::LandRulings).unwrap(),
            "\"landRulings\""
        );
        assert_eq!(
            serde_json::to_string(&EngineKind::Claude).unwrap(),
            "\"claude\""
        );
    }
}
