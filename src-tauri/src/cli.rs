//! Command-line actions of the app binary.
//!
//! ```text
//! dise --capture "<text>"   save the text as a capture (记一笔) in the inbox
//! dise --capture=<text>     the same
//! dise --sync               sync the knowledge base now
//! dise --ingest             start an AI run: 入库 (work through inbox/)
//! dise --land-rulings       start an AI run: 落裁决 (work through inbox/裁决/)
//!      --engine claude|codex   the engine of that run, instead of the configured one
//!      --model <name>          the model of that run
//! ```
//!
//! When an instance is already running, the single-instance plugin hands the
//! arguments to it and the new process exits; otherwise the app starts, does
//! the work and stays in the menu bar without opening its main window. A run
//! writes its steps and its outcome to the app log.

use crate::run::Task;

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliRequest {
    /// `--capture`: the text, or why it could not be read.
    pub capture: Option<Result<String, String>>,
    /// `--sync`.
    pub sync: bool,
    /// `--ingest` or `--land-rulings`: the run to start, or why none is started.
    pub run: Option<Result<Task, String>>,
    /// `--engine`, as written (checked when the run starts).
    pub engine: Option<String>,
    /// `--model`.
    pub model: Option<String>,
    /// Arguments that were not understood (logged, otherwise ignored).
    pub unknown: Vec<String>,
}

impl CliRequest {
    /// True when the command line asked for some work, so the main window
    /// stays closed.
    pub fn has_action(&self) -> bool {
        self.capture.is_some() || self.sync || self.run.is_some()
    }
}

/// Reads the arguments after the program name.
pub fn parse<I, S>(args: I) -> CliRequest
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut request = CliRequest::default();
    // A run with a broken option is not started: it could cost money on the
    // wrong engine or model.
    let mut run_problems: Vec<String> = Vec::new();
    let ask_for = |request: &mut CliRequest, task: Task| {
        request.run = Some(match request.run.take() {
            None => Ok(task),
            Some(Ok(other)) if other == task => Ok(task),
            Some(_) => Err("--ingest 和 --land-rulings 一次只能用一个".to_string()),
        });
    };
    let mut args = args.into_iter().map(Into::into).peekable();
    while let Some(arg) = args.next() {
        if arg == "--capture" {
            request.capture = Some(
                args.next()
                    .ok_or_else(|| "--capture 后面缺少要记下的文字".to_string()),
            );
        } else if let Some(text) = arg.strip_prefix("--capture=") {
            request.capture = Some(Ok(text.to_string()));
        } else if arg == "--sync" {
            request.sync = true;
        } else if arg == "--ingest" {
            ask_for(&mut request, Task::Ingest);
        } else if arg == "--land-rulings" {
            ask_for(&mut request, Task::LandRulings);
        } else if arg == "--engine" || arg == "--model" {
            // The next option is not this option's value.
            match args.next_if(|value| !value.starts_with("--")) {
                Some(value) if arg == "--engine" => request.engine = Some(value),
                Some(value) => request.model = Some(value),
                None => run_problems.push(format!("{arg} 后面缺少值")),
            }
        } else if let Some(value) = arg.strip_prefix("--engine=") {
            request.engine = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--model=") {
            request.model = Some(value.to_string());
        } else if arg.starts_with("-psn_") {
            // Added by older macOS versions when launched from Finder.
        } else {
            request.unknown.push(arg);
        }
    }
    match &request.run {
        Some(Ok(_)) if !run_problems.is_empty() => {
            request.run = Some(Err(run_problems.join("；")));
        }
        None => {
            // --engine and --model only mean something for a run.
            for (option, value) in [("--engine", &request.engine), ("--model", &request.model)] {
                if value.is_some() {
                    request.unknown.push(format!(
                        "{option}（要和 --ingest 或 --land-rulings 一起用）"
                    ));
                }
            }
            request.unknown.extend(run_problems);
        }
        _ => {}
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_capture_text_verbatim() {
        let request = parse(["--capture", "  第一行\n第二行  "]);
        assert_eq!(request.capture, Some(Ok("  第一行\n第二行  ".to_string())));
        assert!(!request.sync);
        assert!(request.has_action());

        let request = parse(["--capture=a=b --sync"]);
        assert_eq!(request.capture, Some(Ok("a=b --sync".to_string())));
        assert!(!request.sync, "the text is not parsed further");

        // The text may look like an option.
        let request = parse(["--capture", "--sync"]);
        assert_eq!(request.capture, Some(Ok("--sync".to_string())));
        assert!(!request.sync);
    }

    #[test]
    fn reports_a_missing_capture_text() {
        let request = parse(["--capture"]);
        assert!(matches!(request.capture, Some(Err(ref error)) if error.contains("缺少")));
        assert!(request.has_action());
    }

    #[test]
    fn reads_sync_and_ignores_the_rest() {
        let request = parse(["--sync", "-psn_0_12345", "--what"]);
        assert!(request.sync);
        assert_eq!(request.capture, None);
        assert_eq!(request.unknown, vec!["--what"]);
        assert!(!parse(Vec::<String>::new()).has_action());
        assert!(!parse(["--what"]).has_action());
    }

    #[test]
    fn reads_the_run_to_start_with_its_engine_and_model() {
        let request = parse(["--ingest", "--model", "sonnet"]);
        assert_eq!(request.run, Some(Ok(Task::Ingest)));
        assert_eq!(request.model.as_deref(), Some("sonnet"));
        assert_eq!(request.engine, None);
        assert!(request.has_action());
        assert!(request.unknown.is_empty());

        let request = parse(["--engine=codex", "--land-rulings", "--model=gpt-x"]);
        assert_eq!(request.run, Some(Ok(Task::LandRulings)));
        assert_eq!(request.engine.as_deref(), Some("codex"));
        assert_eq!(request.model.as_deref(), Some("gpt-x"));

        // A capture and a run in one go.
        let request = parse(["--capture", "一句话", "--ingest", "--engine", "claude"]);
        assert_eq!(request.capture, Some(Ok("一句话".to_string())));
        assert_eq!(request.run, Some(Ok(Task::Ingest)));
        assert_eq!(request.engine.as_deref(), Some("claude"));
    }

    #[test]
    fn a_run_with_a_broken_option_is_not_started() {
        let both = parse(["--ingest", "--land-rulings"]);
        assert!(matches!(both.run, Some(Err(ref error)) if error.contains("一次只能用一个")));
        assert!(both.has_action(), "the window still stays closed");
        // Asking twice for the same thing is fine.
        assert_eq!(parse(["--ingest", "--ingest"]).run, Some(Ok(Task::Ingest)));

        for args in [vec!["--ingest", "--model"], vec!["--engine", "--ingest"]] {
            let request = parse(args.clone());
            assert!(
                matches!(request.run, Some(Err(ref error)) if error.contains("缺少值")),
                "{args:?}: {:?}",
                request.run
            );
        }
    }

    #[test]
    fn engine_and_model_without_a_run_are_reported() {
        let request = parse(["--model", "sonnet", "--sync"]);
        assert_eq!(request.run, None);
        assert!(request.sync);
        assert_eq!(request.unknown.len(), 1);
        assert!(request.unknown[0].starts_with("--model"), "{:?}", request.unknown);
        let request = parse(["--engine"]);
        assert!(!request.has_action());
        assert_eq!(request.unknown, vec!["--engine 后面缺少值"]);
    }
}
