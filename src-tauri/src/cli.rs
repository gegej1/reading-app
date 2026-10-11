//! Command-line actions of the app binary.
//!
//! ```text
//! dise --capture "<text>"   save the text as a capture (记一笔) in the inbox
//! dise --capture=<text>     the same
//! dise --sync               sync the knowledge base now
//! ```
//!
//! When an instance is already running, the single-instance plugin hands the
//! arguments to it and the new process exits; otherwise the app starts, does
//! the work and stays in the menu bar without opening its main window.

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliRequest {
    /// `--capture`: the text, or why it could not be read.
    pub capture: Option<Result<String, String>>,
    /// `--sync`.
    pub sync: bool,
    /// Arguments that were not understood (logged, otherwise ignored).
    pub unknown: Vec<String>,
}

impl CliRequest {
    /// True when the command line asked for some work, so the main window
    /// stays closed.
    pub fn has_action(&self) -> bool {
        self.capture.is_some() || self.sync
    }
}

/// Reads the arguments after the program name.
pub fn parse<I, S>(args: I) -> CliRequest
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut request = CliRequest::default();
    let mut args = args.into_iter().map(Into::into);
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
        } else if arg.starts_with("-psn_") {
            // Added by older macOS versions when launched from Finder.
        } else {
            request.unknown.push(arg);
        }
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
}
