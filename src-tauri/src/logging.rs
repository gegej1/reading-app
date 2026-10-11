//! Logging: stdout plus a size-rotated file in the app's macOS log folder
//! (`~/Library/Logs/com.gegej1.dise/dise.log`), at info level. The frontend
//! writes to the same log through the plugin's `log` command.

use std::fmt::Arguments;

use tauri::plugin::TauriPlugin;
use tauri::Runtime;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};
use time::OffsetDateTime;

/// File name (without `.log`) inside the app log folder.
pub const LOG_FILE_NAME: &str = "dise";
/// A log file is rotated once it reaches this size.
const MAX_FILE_BYTES: u128 = 2 * 1024 * 1024;
/// Rotated files kept next to the current one.
const KEEP_FILES: usize = 3;

/// One log line: `2026-10-12 02:31:05.123 +08:00 INFO  [dise_lib] message`.
pub fn format_line(
    now: OffsetDateTime,
    level: log::Level,
    target: &str,
    message: &Arguments,
) -> String {
    let offset = now.offset();
    let sign = if offset.is_negative() { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {sign}{:02}:{:02} {:<5} [{target}] {message}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond(),
        offset.whole_hours().unsigned_abs(),
        offset.minutes_past_hour().unsigned_abs(),
        level,
    )
}

pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(Target::new(TargetKind::Stdout))
        .target(Target::new(TargetKind::LogDir {
            file_name: Some(LOG_FILE_NAME.into()),
        }))
        .level(log::LevelFilter::Info)
        .max_file_size(MAX_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEEP_FILES))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .format(|out, message, record| {
            let now = TimezoneStrategy::UseLocal.get_now();
            out.finish(format_args!(
                "{}",
                format_line(now, record.level(), record.target(), message)
            ))
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn formats_local_time_with_milliseconds_and_offset() {
        let line = format_line(
            datetime!(2026-10-12 02:31:05.123 +08:00),
            log::Level::Info,
            "dise_lib::commands",
            &format_args!("kb_list ok files=52 (14 ms)"),
        );
        assert_eq!(
            line,
            "2026-10-12 02:31:05.123 +08:00 INFO  [dise_lib::commands] kb_list ok files=52 (14 ms)"
        );
        let west = format_line(
            datetime!(2026-01-02 03:04:05.006 -03:30),
            log::Level::Warn,
            "webview",
            &format_args!("x"),
        );
        assert_eq!(west, "2026-01-02 03:04:05.006 -03:30 WARN  [webview] x");
    }
}
