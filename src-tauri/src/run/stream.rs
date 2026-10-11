//! The engine's output, one JSON event per line, turned into steps a person
//! can follow: which model answered, every tool call as a verb and a
//! repo-relative path, short pieces of what the AI said, refusals, and the
//! result with its cost.
//!
//! Claude Code (`--output-format stream-json`) sends `system`/`init`, then
//! `assistant` and `user` messages with `tool_use` and `tool_result` blocks,
//! `system`/`permission_denied`, and a final `result`. Codex (`exec --json`)
//! sends `thread.started`, `turn.started`, `item.*` (agent messages, command
//! executions, file changes) and `turn.completed` or `turn.failed`.
//!
//! Lines that are not understood are skipped: the raw lines are kept in the
//! run's folder anyway.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::EngineKind;

/// Longest piece of the AI's own words shown as one step.
const MAX_TEXT: usize = 200;
/// Longest command or search pattern shown.
const MAX_TARGET: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    /// The engine started (which model).
    Init,
    /// A tool call: `verb` and `target`.
    Tool,
    /// Something the AI said.
    Text,
    /// A tool call that was refused.
    Denied,
    /// The engine's own result.
    Result,
    Error,
    /// A step of the app itself (syncing, checking, committing).
    Note,
}

/// One step of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub kind: StepKind,
    /// 读, 写, 改, 查找, 删, 运行, …
    pub verb: Option<String>,
    /// What the verb applies to: a repo-relative path, a search pattern or a command.
    pub target: Option<String>,
    pub text: Option<String>,
}

impl Step {
    pub fn note(text: impl Into<String>) -> Self {
        Step {
            kind: StepKind::Note,
            verb: None,
            target: None,
            text: Some(text.into()),
        }
    }

    fn tool(verb: &str, target: Option<String>) -> Self {
        Step {
            kind: StepKind::Tool,
            verb: Some(verb.to_string()),
            target,
            text: None,
        }
    }

    fn text(kind: StepKind, text: impl Into<String>) -> Self {
        Step {
            kind,
            verb: None,
            target: None,
            text: Some(text.into()),
        }
    }

    /// The step in one line for the app log: tool calls with their path,
    /// everything else by kind only. What the AI said never goes to the log.
    pub fn log_line(&self) -> Option<String> {
        match self.kind {
            StepKind::Tool | StepKind::Denied => {
                let denied = if self.kind == StepKind::Denied {
                    "被拒绝："
                } else {
                    ""
                };
                Some(format!(
                    "{denied}{} {}",
                    self.verb.as_deref().unwrap_or("工具"),
                    self.target.as_deref().unwrap_or("")
                ))
            }
            StepKind::Init | StepKind::Result | StepKind::Note => self.text.clone(),
            StepKind::Text | StepKind::Error => None,
        }
    }
}

/// What the stream told about the run as a whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSummary {
    pub model: Option<String>,
    /// Where Claude Code took its login from (`apiKeyHelper`, `none`, …): a name, never a key.
    pub key_source: Option<String>,
    /// The engine sent its closing event.
    pub finished: bool,
    /// That closing event (or a failed turn) reported an error.
    pub is_error: bool,
    pub subtype: Option<String>,
    /// The engine's last reply.
    pub final_message: Option<String>,
    /// The last error the engine reported.
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub turns: Option<u64>,
    /// As measured by the engine.
    pub duration_ms: Option<u64>,
    pub tool_calls: u32,
    pub denials: u32,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Reads the lines of one run.
pub struct StreamParser {
    engine: EngineKind,
    /// Spellings of the knowledge-base folder, to make paths repo-relative.
    roots: Vec<String>,
    summary: StreamSummary,
}

impl StreamParser {
    /// `root` is the canonical knowledge-base folder the engine works in.
    pub fn new(engine: EngineKind, root: &Path) -> Self {
        let canonical = root.to_string_lossy().trim_end_matches('/').to_string();
        let mut roots = vec![canonical.clone()];
        // macOS: /tmp and /var are links into /private.
        if let Some(short) = canonical.strip_prefix("/private") {
            if short.starts_with("/tmp") || short.starts_with("/var") {
                roots.push(short.to_string());
            }
        }
        StreamParser {
            engine,
            roots,
            summary: StreamSummary::default(),
        }
    }

    #[cfg(test)]
    pub fn summary(&self) -> &StreamSummary {
        &self.summary
    }

    pub fn into_summary(self) -> StreamSummary {
        self.summary
    }

    /// The steps one output line stands for (often none).
    pub fn line(&mut self, line: &str) -> Vec<Step> {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let steps = match self.engine {
            EngineKind::Claude => self.claude(&event),
            EngineKind::Codex => self.codex(&event),
        };
        for step in &steps {
            match step.kind {
                StepKind::Tool => self.summary.tool_calls += 1,
                StepKind::Denied => self.summary.denials += 1,
                _ => {}
            }
        }
        steps
    }

    /// `path` relative to the knowledge base when it is inside it.
    fn relative(&self, path: &str) -> String {
        for root in &self.roots {
            if path == root {
                return ".".to_string();
            }
            if let Some(rest) = path.strip_prefix(root.as_str()) {
                if let Some(rel) = rest.strip_prefix('/') {
                    return rel.to_string();
                }
            }
        }
        path.to_string()
    }

    fn claude(&mut self, event: &Value) -> Vec<Step> {
        match (str_of(event, "type"), str_of(event, "subtype")) {
            (Some("system"), Some("init")) => {
                self.summary.model = str_of(event, "model").map(String::from);
                self.summary.key_source = str_of(event, "apiKeySource").map(String::from);
                let version = str_of(event, "claude_code_version")
                    .map(|version| format!(" {version}"))
                    .unwrap_or_default();
                let model = self.summary.model.as_deref().unwrap_or("默认模型");
                vec![Step::text(
                    StepKind::Init,
                    format!("Claude Code{version} · 模型 {model}"),
                )]
            }
            (Some("system"), Some("permission_denied")) => {
                let tool = ["tool_name", "tool", "name"]
                    .iter()
                    .find_map(|key| str_of(event, key))
                    .unwrap_or("工具");
                let input = ["tool_input", "input"]
                    .iter()
                    .find_map(|key| event.get(*key))
                    .unwrap_or(&Value::Null);
                let (verb, target) = self.claude_tool(tool, input);
                vec![Step {
                    kind: StepKind::Denied,
                    verb: Some(verb),
                    target,
                    text: ["message", "reason", "decision_reason"]
                        .iter()
                        .find_map(|key| str_of(event, key))
                        .map(|reason| shorten(reason, MAX_TEXT)),
                }]
            }
            (Some("assistant"), _) => {
                let failed = event.get("error").is_some_and(|error| !error.is_null());
                let blocks = event["message"]["content"].as_array();
                blocks
                    .into_iter()
                    .flatten()
                    .filter_map(|block| match str_of(block, "type") {
                        Some("text") => str_of(block, "text")
                            .map(|text| shorten(text, MAX_TEXT))
                            .filter(|text| !text.is_empty())
                            .map(|text| {
                                let kind = if failed {
                                    StepKind::Error
                                } else {
                                    StepKind::Text
                                };
                                Step::text(kind, text)
                            }),
                        Some("tool_use") => {
                            let (verb, target) = self
                                .claude_tool(str_of(block, "name").unwrap_or("工具"), &block["input"]);
                            Some(Step::tool(&verb, target))
                        }
                        _ => None,
                    })
                    .collect()
            }
            (Some("result"), subtype) => {
                let summary = &mut self.summary;
                summary.finished = true;
                summary.is_error = event["is_error"].as_bool().unwrap_or(false)
                    || subtype.is_some_and(|subtype| subtype != "success");
                summary.subtype = subtype.map(String::from);
                summary.final_message = str_of(event, "result").map(String::from);
                summary.cost_usd = event["total_cost_usd"].as_f64();
                summary.turns = event["num_turns"].as_u64();
                summary.duration_ms = event["duration_ms"].as_u64();
                let listed = event["permission_denials"]
                    .as_array()
                    .map_or(0, |denials| u32::try_from(denials.len()).unwrap_or(u32::MAX));
                summary.denials = summary.denials.max(listed);
                if summary.is_error {
                    summary.error = summary
                        .final_message
                        .as_deref()
                        .map(|text| shorten(text, MAX_TEXT));
                }
                let cost = summary
                    .cost_usd
                    .map(|cost| format!(" · 花费 ${cost:.2}"))
                    .unwrap_or_default();
                let turns = summary
                    .turns
                    .map(|turns| format!(" · {turns} 轮"))
                    .unwrap_or_default();
                let word = if summary.is_error {
                    "AI 报告了错误"
                } else {
                    "AI 做完了"
                };
                vec![Step::text(StepKind::Result, format!("{word}{turns}{cost}"))]
            }
            _ => Vec::new(),
        }
    }

    /// The verb and target of one Claude Code tool call.
    fn claude_tool(&self, name: &str, input: &Value) -> (String, Option<String>) {
        let path = ["file_path", "path", "notebook_path"]
            .iter()
            .find_map(|key| str_of(input, key))
            .map(|path| self.relative(path));
        match name {
            "Read" => ("读".into(), path),
            "Write" => ("写".into(), path),
            "Edit" | "MultiEdit" | "NotebookEdit" => ("改".into(), path),
            "Glob" | "Grep" => {
                let pattern = str_of(input, "pattern").unwrap_or_default();
                let target = match path {
                    Some(path) if path != "." => format!("{pattern}（在 {path}）"),
                    _ => pattern.to_string(),
                };
                ("查找".into(), Some(shorten(&target, MAX_TARGET)))
            }
            other => (other.to_string(), path),
        }
    }

    fn codex(&mut self, event: &Value) -> Vec<Step> {
        let kind = str_of(event, "type").unwrap_or_default();
        match kind {
            "thread.started" => vec![Step::text(StepKind::Init, "Codex 开始工作")],
            "item.started" | "item.completed" => {
                let item = &event["item"];
                // Older versions nest the fields in `details`.
                let item = if item.get("details").is_some_and(Value::is_object) {
                    &item["details"]
                } else {
                    item
                };
                let item_type = ["type", "item_type"]
                    .iter()
                    .find_map(|key| str_of(item, key))
                    .unwrap_or_default();
                let completed = kind == "item.completed";
                match item_type {
                    "agent_message" if completed => {
                        let text = str_of(item, "text").unwrap_or_default();
                        self.summary.final_message = Some(text.to_string());
                        let text = shorten(text, MAX_TEXT);
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![Step::text(StepKind::Text, text)]
                        }
                    }
                    "command_execution" if !completed => vec![Step::tool(
                        "运行",
                        str_of(item, "command").map(|command| shorten(command, MAX_TARGET)),
                    )],
                    "file_change" if completed => item["changes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|change| {
                            let verb = match str_of(change, "kind") {
                                Some("add") => "写",
                                Some("delete") => "删",
                                _ => "改",
                            };
                            Step::tool(verb, str_of(change, "path").map(|path| self.relative(path)))
                        })
                        .collect(),
                    "mcp_tool_call" if !completed => {
                        let server = str_of(item, "server").unwrap_or_default();
                        let tool = str_of(item, "tool").unwrap_or_default();
                        vec![Step::tool("外部工具", Some(format!("{server}/{tool}")))]
                    }
                    "web_search" if completed => vec![Step::tool(
                        "搜索",
                        str_of(item, "query").map(|query| shorten(query, MAX_TARGET)),
                    )],
                    "error" if completed => {
                        let message = shorten(str_of(item, "message").unwrap_or_default(), MAX_TEXT);
                        self.summary.error = Some(message.clone());
                        vec![Step::text(StepKind::Error, message)]
                    }
                    _ => Vec::new(),
                }
            }
            "turn.completed" => {
                self.summary.finished = true;
                self.summary.turns = Some(self.summary.turns.unwrap_or(0) + 1);
                let usage = &event["usage"];
                self.summary.input_tokens = usage["input_tokens"].as_u64();
                self.summary.output_tokens = usage["output_tokens"].as_u64();
                vec![Step::text(StepKind::Result, "AI 做完了")]
            }
            "turn.failed" | "thread.failed" => {
                let message = shorten(
                    str_of(&event["error"], "message").unwrap_or("没有说明原因"),
                    MAX_TEXT,
                );
                self.summary.finished = true;
                self.summary.is_error = true;
                self.summary.error = Some(message.clone());
                vec![Step::text(StepKind::Error, message)]
            }
            // Also sent for retries ("Reconnecting…"): not the end of the run.
            "error" => {
                let message = shorten(str_of(event, "message").unwrap_or_default(), MAX_TEXT);
                self.summary.error = Some(message.clone());
                vec![Step::text(StepKind::Error, message)]
            }
            _ => Vec::new(),
        }
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// One line, at most `max` characters.
fn shorten(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        line
    } else {
        let cut: String = line.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude() -> StreamParser {
        StreamParser::new(EngineKind::Claude, Path::new("/private/tmp/dise-sandbox/kb"))
    }

    fn line(parser: &mut StreamParser, event: Value) -> Vec<Step> {
        parser.line(&event.to_string())
    }

    #[test]
    fn claude_init_names_the_model_and_where_the_login_came_from() {
        let mut parser = claude();
        let steps = line(
            &mut parser,
            json!({"type": "system", "subtype": "init", "cwd": "/private/tmp/dise-sandbox/kb",
                   "tools": ["Edit", "Glob", "Grep", "Read", "Write"], "mcp_servers": [],
                   "model": "claude-sonnet-x", "permissionMode": "acceptEdits",
                   "apiKeySource": "apiKeyHelper", "claude_code_version": "2.1.281"}),
        );
        assert_eq!(
            steps,
            [Step::text(
                StepKind::Init,
                "Claude Code 2.1.281 · 模型 claude-sonnet-x"
            )]
        );
        assert_eq!(parser.summary().model.as_deref(), Some("claude-sonnet-x"));
        assert_eq!(parser.summary().key_source.as_deref(), Some("apiKeyHelper"));
        assert!(!parser.summary().finished);
    }

    #[test]
    fn claude_tool_calls_become_a_verb_and_a_repo_relative_path() {
        let mut parser = claude();
        let steps = line(
            &mut parser,
            json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "text", "text": "先读一下规则。\n\n然后开始。"},
                {"type": "tool_use", "id": "t1", "name": "Read",
                 "input": {"file_path": "/private/tmp/dise-sandbox/kb/AGENTS.md"}},
                {"type": "tool_use", "id": "t2", "name": "Write",
                 "input": {"file_path": "/tmp/dise-sandbox/kb/raw/inbox/2026-10-12-收件箱.md", "content": "全文不出现在步骤里"}},
                {"type": "tool_use", "id": "t3", "name": "Edit",
                 "input": {"file_path": "/private/tmp/dise-sandbox/kb/index.md", "old_string": "a", "new_string": "b"}},
                {"type": "tool_use", "id": "t4", "name": "Grep",
                 "input": {"pattern": "周回顾", "path": "/private/tmp/dise-sandbox/kb/wiki"}},
                {"type": "tool_use", "id": "t5", "name": "Glob", "input": {"pattern": "self/判例/*.md"}},
                {"type": "tool_use", "id": "t6", "name": "Read", "input": {"file_path": "/etc/hosts"}},
                {"type": "thinking", "thinking": "不显示"}
            ]}}),
        );
        type Shown<'a> = (StepKind, Option<&'a str>, Option<&'a str>, Option<&'a str>);
        let shown: Vec<Shown> = steps
            .iter()
            .map(|step| {
                (
                    step.kind,
                    step.verb.as_deref(),
                    step.target.as_deref(),
                    step.text.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (StepKind::Text, None, None, Some("先读一下规则。 然后开始。")),
                (StepKind::Tool, Some("读"), Some("AGENTS.md"), None),
                (
                    StepKind::Tool,
                    Some("写"),
                    Some("raw/inbox/2026-10-12-收件箱.md"),
                    None
                ),
                (StepKind::Tool, Some("改"), Some("index.md"), None),
                (StepKind::Tool, Some("查找"), Some("周回顾（在 wiki）"), None),
                (StepKind::Tool, Some("查找"), Some("self/判例/*.md"), None),
                // Outside the knowledge base: shown as it is (and refused by the engine).
                (StepKind::Tool, Some("读"), Some("/etc/hosts"), None),
            ]
        );
        assert_eq!(parser.summary().tool_calls, 6);
        assert!(steps
            .iter()
            .all(|step| !format!("{step:?}").contains("全文不出现")));
    }

    #[test]
    fn claude_refusals_are_steps_and_counted() {
        let mut parser = claude();
        let steps = line(
            &mut parser,
            json!({"type": "system", "subtype": "permission_denied", "tool_name": "Write",
                   "tool_input": {"file_path": "/private/tmp/dise-sandbox/kb/.git/config"},
                   "message": "writes to git files need approval"}),
        );
        assert_eq!(
            steps,
            [Step {
                kind: StepKind::Denied,
                verb: Some("写".into()),
                target: Some(".git/config".into()),
                text: Some("writes to git files need approval".into()),
            }]
        );
        assert_eq!(steps[0].log_line().as_deref(), Some("被拒绝：写 .git/config"));
        assert_eq!(parser.summary().denials, 1);
    }

    #[test]
    fn claude_result_carries_cost_turns_and_the_final_reply() {
        let mut parser = claude();
        let reply = r#"{"任务": "入库", "条数": 2, "自动": 2, "判例裁决": 0, "待裁决": 0, "报告": "reports/2026-10-12-入库报告.md", "备注": ""}"#;
        let steps = line(
            &mut parser,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": reply,
                   "total_cost_usd": 0.4321, "num_turns": 37, "duration_ms": 95_000,
                   "permission_denials": [{"tool_name": "Write"}, {"tool_name": "Write"}]}),
        );
        assert_eq!(
            steps,
            [Step::text(StepKind::Result, "AI 做完了 · 37 轮 · 花费 $0.43")]
        );
        let summary = parser.into_summary();
        assert!(summary.finished);
        assert!(!summary.is_error);
        assert_eq!(summary.final_message.as_deref(), Some(reply));
        assert_eq!(summary.cost_usd, Some(0.4321));
        assert_eq!(summary.turns, Some(37));
        assert_eq!(summary.duration_ms, Some(95_000));
        assert_eq!(summary.denials, 2);
        assert_eq!(summary.error, None);
    }

    #[test]
    fn claude_reports_a_missing_login_as_an_error() {
        let mut parser = claude();
        let said = line(
            &mut parser,
            json!({"type": "assistant", "error": "authentication_failed",
                   "message": {"content": [{"type": "text", "text": "Not logged in · Please run /login"}]}}),
        );
        assert_eq!(said[0].kind, StepKind::Error);
        line(
            &mut parser,
            json!({"type": "result", "subtype": "success", "is_error": true,
                   "result": "Not logged in · Please run /login", "total_cost_usd": 0,
                   "num_turns": 1, "duration_ms": 26, "permission_denials": []}),
        );
        let summary = parser.summary();
        assert!(summary.finished && summary.is_error);
        assert_eq!(
            summary.error.as_deref(),
            Some("Not logged in · Please run /login")
        );
        assert_eq!(summary.cost_usd, Some(0.0));
    }

    #[test]
    fn claude_error_subtypes_count_as_errors() {
        let mut parser = claude();
        line(
            &mut parser,
            json!({"type": "result", "subtype": "error_max_budget_usd", "is_error": false,
                   "total_cost_usd": 1.5, "num_turns": 80}),
        );
        assert!(parser.summary().is_error);
        assert_eq!(
            parser.summary().subtype.as_deref(),
            Some("error_max_budget_usd")
        );
    }

    #[test]
    fn noise_and_unknown_events_are_skipped() {
        let mut parser = claude();
        for text in [
            "",
            "not json at all",
            "{\"type\":",
            r#"{"type": "user", "message": {"content": [{"type": "tool_result", "content": "文件全文"}]}}"#,
            r#"{"type": "system", "subtype": "something_new"}"#,
            r#"["an", "array"]"#,
        ] {
            assert!(parser.line(text).is_empty(), "{text}");
        }
        assert_eq!(parser.summary(), &StreamSummary::default());
    }

    #[test]
    fn long_text_is_cut_to_one_short_line() {
        let mut parser = claude();
        let long = "很长的一段话。".repeat(100);
        let steps = line(
            &mut parser,
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": long}]}}),
        );
        let text = steps[0].text.as_deref().unwrap();
        assert_eq!(text.chars().count(), MAX_TEXT + 1);
        assert!(text.ends_with('…'));
        // What the AI said is never a log line.
        assert_eq!(steps[0].log_line(), None);
    }

    #[test]
    fn codex_events_become_steps_too() {
        let mut parser = StreamParser::new(EngineKind::Codex, Path::new("/kb"));
        assert_eq!(
            line(&mut parser, json!({"type": "thread.started", "thread_id": "t"})),
            [Step::text(StepKind::Init, "Codex 开始工作")]
        );
        assert!(line(&mut parser, json!({"type": "turn.started"})).is_empty());
        assert_eq!(
            line(
                &mut parser,
                json!({"type": "item.started", "item": {"id": "i0", "type": "command_execution",
                       "command": "bash -lc 'ls inbox'", "status": "in_progress"}})
            ),
            [Step::tool("运行", Some("bash -lc 'ls inbox'".into()))]
        );
        assert!(line(
            &mut parser,
            json!({"type": "item.completed", "item": {"id": "i0", "type": "command_execution",
                   "command": "bash -lc 'ls inbox'", "aggregated_output": "文件列表", "exit_code": 0}})
        )
        .is_empty());
        assert_eq!(
            line(
                &mut parser,
                json!({"type": "item.completed", "item": {"id": "i1", "type": "file_change", "status": "completed",
                       "changes": [{"path": "/kb/raw/inbox/2026-10-12-收件箱.md", "kind": "add"},
                                   {"path": "/kb/index.md", "kind": "update"},
                                   {"path": "/kb/inbox/a.md", "kind": "delete"}]}})
            ),
            [
                Step::tool("写", Some("raw/inbox/2026-10-12-收件箱.md".into())),
                Step::tool("改", Some("index.md".into())),
                Step::tool("删", Some("inbox/a.md".into())),
            ]
        );
        assert!(line(
            &mut parser,
            json!({"type": "item.completed", "item": {"id": "i2", "type": "reasoning", "text": "想法"}})
        )
        .is_empty());
        let reply = r#"{"任务": "入库", "条数": 1}"#;
        assert_eq!(
            line(
                &mut parser,
                json!({"type": "item.completed", "item": {"id": "i3", "type": "agent_message", "text": reply}})
            ),
            [Step::text(StepKind::Text, reply)]
        );
        // A retry notice is not the end of the run.
        line(&mut parser, json!({"type": "error", "message": "Reconnecting... 1/5"}));
        assert!(!parser.summary().finished && !parser.summary().is_error);

        assert_eq!(
            line(
                &mut parser,
                json!({"type": "turn.completed", "usage": {"input_tokens": 1200, "cached_input_tokens": 800, "output_tokens": 340}})
            ),
            [Step::text(StepKind::Result, "AI 做完了")]
        );
        let summary = parser.into_summary();
        assert!(summary.finished && !summary.is_error);
        assert_eq!(summary.final_message.as_deref(), Some(reply));
        assert_eq!(summary.tool_calls, 4);
        assert_eq!(summary.input_tokens, Some(1200));
        assert_eq!(summary.output_tokens, Some(340));
        assert_eq!(summary.cost_usd, None);
    }

    #[test]
    fn codex_failed_turns_are_errors() {
        let mut parser = StreamParser::new(EngineKind::Codex, Path::new("/kb"));
        let steps = line(
            &mut parser,
            json!({"type": "turn.failed", "error": {"message": "unexpected status 401 Unauthorized"}}),
        );
        assert_eq!(
            steps,
            [Step::text(
                StepKind::Error,
                "unexpected status 401 Unauthorized"
            )]
        );
        assert!(parser.summary().is_error);
        assert_eq!(
            parser.summary().error.as_deref(),
            Some("unexpected status 401 Unauthorized")
        );
    }

    #[test]
    fn steps_serialize_for_the_frontend() {
        assert_eq!(
            serde_json::to_value(Step::tool("写", Some("index.md".into()))).unwrap(),
            json!({"kind": "tool", "verb": "写", "target": "index.md", "text": null})
        );
        assert_eq!(
            serde_json::to_value(Step::note("正在同步")).unwrap(),
            json!({"kind": "note", "verb": null, "target": null, "text": "正在同步"})
        );
    }
}
