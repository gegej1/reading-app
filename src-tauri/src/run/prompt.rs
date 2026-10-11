//! The task prompts handed to the engine.
//!
//! The text lives in `runners/ingest.md` and `runners/land-rulings.md` at the
//! top of the repository and is embedded at build time. The prompts only say
//! which task to do and which files to process; how to do it is in the
//! knowledge base's own `AGENTS.md`.

use super::{EngineKind, Task};

const INGEST: &str = include_str!("../../../runners/ingest.md");
const LAND_RULINGS: &str = include_str!("../../../runners/land-rulings.md");
/// Appended for Codex: AGENTS.md section 九 says "no commands", which is
/// true for Claude Code's restricted mode, but Codex reads files only
/// through commands. Without this note it reads nothing and does nothing.
const CODEX_NOTE: &str = include_str!("../../../runners/codex.md");

const DATE: &str = "{{date}}";
const QUEUE_FILES: &str = "{{queue_files}}";

fn template(task: Task) -> &'static str {
    match task {
        Task::Ingest => INGEST,
        Task::LandRulings => LAND_RULINGS,
    }
}

/// The prompt for `task`: `date` is the local `YYYY-MM-DD`, `queue` the
/// repo-relative paths of the files to process (one `- path` line each).
pub fn task_prompt(task: Task, date: &str, queue: &[String]) -> String {
    let files = queue
        .iter()
        .map(|path| format!("- {path}"))
        .collect::<Vec<_>>()
        .join("\n");
    template(task)
        .replace(DATE, date)
        .replace(QUEUE_FILES, &files)
        .trim_end()
        .to_string()
}

/// The prompt for one run: [`task_prompt`], plus the note for Codex.
pub fn engine_prompt(task: Task, engine: EngineKind, date: &str, queue: &[String]) -> String {
    let prompt = task_prompt(task, date, queue);
    match engine {
        EngineKind::Claude => prompt,
        EngineKind::Codex => format!("{prompt}\n\n{}", CODEX_NOTE.trim_end()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> Vec<String> {
        vec![
            "inbox/2026-10-12T09-00-00-mac.md".to_string(),
            "inbox/2026-10-12T09-05-00-iphone.md".to_string(),
        ]
    }

    #[test]
    fn fills_the_date_and_the_queue_into_the_ingest_prompt() {
        let prompt = task_prompt(Task::Ingest, "2026-10-12", &queue());
        assert!(prompt.starts_with("你在我的个人决策底色知识库里工作。本次任务：入库。\n"));
        assert!(prompt.contains("1. 先完整读取 AGENTS.md。严格按第二节\"入库流程\"执行"));
        assert!(prompt.contains("2. 今天是 2026-10-12。\n"));
        assert!(prompt.contains(
            "（只处理这些，一个都不要漏）：\n- inbox/2026-10-12T09-00-00-mac.md\n- inbox/2026-10-12T09-05-00-iphone.md\n4. "
        ));
        assert!(prompt.contains("逐字写进 raw/inbox/2026-10-12-收件箱.md，App 会逐字核对。"));
        assert!(prompt.contains("不做任何 git 操作；不删除队列文件；跳过 Notion"));
        assert!(prompt.ends_with("最后一条回复只输出第九节规定的那一行 JSON，不要加任何别的文字。"));
        assert!(!prompt.contains("{{"), "{prompt}");
    }

    #[test]
    fn fills_the_land_rulings_prompt() {
        let answers = vec!["inbox/裁决/2026-10-12T10-00-00-mac.md".to_string()];
        let prompt = task_prompt(Task::LandRulings, "2026-10-12", &answers);
        assert!(prompt.starts_with("你在我的个人决策底色知识库里工作。本次任务：落裁决。\n"));
        assert!(prompt.contains("严格按第三节\"落裁决流程\"执行"));
        assert!(prompt.contains("：\n- inbox/裁决/2026-10-12T10-00-00-mac.md\n4. "));
        assert!(prompt.contains("逐字写进 raw/对话/2026-10-12-裁决.md"));
        assert!(prompt.contains("标\"AI 整理，待你确认\""));
        assert!(!prompt.contains("Notion"));
        assert!(!prompt.contains("{{"), "{prompt}");
    }

    #[test]
    fn the_templates_have_exactly_the_placeholders_the_app_fills() {
        for task in [Task::Ingest, Task::LandRulings] {
            let text = template(task);
            assert_eq!(text.matches(QUEUE_FILES).count(), 1, "{task:?}");
            assert_eq!(text.matches(DATE).count(), 2, "{task:?}");
            let rest = text.replace(DATE, "").replace(QUEUE_FILES, "");
            assert!(!rest.contains("{{") && !rest.contains("}}"), "{task:?}");
            // Both remind the engine of the two things the app checks hardest.
            assert!(text.contains("第九节"), "{task:?}");
            assert!(text.contains("逐字"), "{task:?}");
        }
    }

    #[test]
    fn codex_gets_a_note_about_reading_files_and_claude_does_not() {
        let claude = engine_prompt(Task::Ingest, EngineKind::Claude, "2026-10-12", &queue());
        assert_eq!(claude, task_prompt(Task::Ingest, "2026-10-12", &queue()));
        for task in [Task::Ingest, Task::LandRulings] {
            let codex = engine_prompt(task, EngineKind::Codex, "2026-10-12", &queue());
            let (task_part, note) = codex.split_once("\n\n补充说明（只给 Codex）").unwrap();
            assert_eq!(task_part, task_prompt(task, "2026-10-12", &queue()));
            for expected in ["cat、sed", "apply_patch", "不运行 git", "不删除", "那一行 JSON"] {
                assert!(note.contains(expected), "{expected}: {note}");
            }
            assert!(!codex.ends_with('\n'));
        }
    }
}
