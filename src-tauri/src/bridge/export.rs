//! Writing the exported 底色 layer into the bridge folder's `阅读/`.
//!
//! The main window builds the pages from the knowledge base (see
//! `src/export/`) and hands them over by name. Only these names are
//! accepted, so nothing can be written anywhere else:
//!
//! - `今日.html`, `底色.html`, `判例.html`, `待裁决.html`, `待抽查.html`,
//!   `最近入库.html`;
//! - `今日一条.txt`, `待裁决列表.txt`;
//! - `待裁决/Q-001.txt` (one per open ruling).
//!
//! Each file is written atomically and only when its content changed (so
//! iCloud uploads only what changed). Exported files that are no longer part
//! of the export (a ruling that was decided) are deleted; any other file in
//! `阅读/` is left alone, and nothing outside `阅读/` is ever touched.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::kb::files;

/// The fixed names of the export.
pub const PAGES: &[&str] = &[
    "今日.html",
    "底色.html",
    "判例.html",
    "待裁决.html",
    "待抽查.html",
    "最近入库.html",
    "今日一条.txt",
    "待裁决列表.txt",
];
/// The folder of the per-ruling text files.
pub const QUESTIONS: &str = "待裁决";
/// At most this many files in one export.
pub const MAX_FILES: usize = 500;
/// At most this many bytes per file.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// One exported file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExportFile {
    pub name: String,
    pub content: String,
}

/// What an export did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub written: usize,
    pub unchanged: usize,
    pub removed: usize,
}

/// True for `待裁决/Q-001.txt`.
fn is_question_file(name: &str) -> bool {
    name.strip_prefix(QUESTIONS)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|file| file.strip_suffix(".txt"))
        .and_then(|stem| stem.strip_prefix("Q-"))
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

/// Accepts only the names of the export.
pub fn check_name(name: &str) -> Result<(), String> {
    if PAGES.contains(&name) || is_question_file(name) {
        Ok(())
    } else {
        Err(format!("不能导出这个文件：{name}"))
    }
}

/// Writes the export into `reading` (the bridge folder's `阅读/`).
pub fn write_all(reading: &Path, export: &[ExportFile]) -> Result<Written, String> {
    if export.len() > MAX_FILES {
        return Err(format!("导出的文件太多（{} 个）", export.len()));
    }
    let mut names = BTreeSet::new();
    for file in export {
        check_name(&file.name)?;
        if file.content.len() > MAX_FILE_BYTES {
            return Err(format!("导出的文件太大：{}", file.name));
        }
        if !names.insert(file.name.as_str()) {
            return Err(format!("导出了两次同一个文件：{}", file.name));
        }
    }
    let questions = reading.join(QUESTIONS);
    fs::create_dir_all(&questions).map_err(|error| format!("没能建立 阅读/待裁决（{error}）"))?;

    let mut written = Written::default();
    for file in export {
        let target = reading.join(&file.name);
        if fs::read(&target).is_ok_and(|bytes| bytes == file.content.as_bytes()) {
            written.unchanged += 1;
            continue;
        }
        files::write_atomic(&target, &file.content)
            .map_err(|error| format!("没能写出 {}（{error}）", file.name))?;
        written.written += 1;
    }

    // Exported files that are not part of this export any more.
    let mut stale = Vec::new();
    for name in PAGES {
        if !names.contains(name) {
            stale.push(name.to_string());
        }
    }
    if let Ok(entries) = fs::read_dir(&questions) {
        for entry in entries.flatten() {
            if let Some(file) = entry.file_name().to_str() {
                let name = format!("{QUESTIONS}/{file}");
                if is_question_file(&name) && !names.contains(name.as_str()) {
                    stale.push(name);
                }
            }
        }
    }
    for name in stale {
        let path = reading.join(&name);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                fs::remove_file(&path)
                    .map_err(|error| format!("没能删掉过时的 {name}（{error}）"))?;
                written.removed += 1;
            }
            _ => {}
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    fn file(name: &str, content: &str) -> ExportFile {
        ExportFile {
            name: name.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn accepts_only_the_export_names() {
        for name in PAGES {
            assert!(check_name(name).is_ok(), "{name}");
        }
        assert!(check_name("待裁决/Q-001.txt").is_ok());
        for bad in [
            "../config.json",
            "/tmp/x.html",
            "待裁决/../../x.txt",
            "待裁决/Q-001.html",
            "待裁决/Q-.txt",
            "待裁决/Q-1/2.txt",
            "raw.html",
            "index.md",
            "今日.HTML",
            "",
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn writes_changed_files_and_removes_stale_exports_only() {
        let dir = TempDir::new("export");
        let reading = dir.mkdir("阅读");
        // Someone else's file in 阅读/ and a file outside it.
        dir.write("阅读/我的笔记.txt", "mine");
        dir.write("收件/记一笔/a.txt", "phone");

        let first = vec![
            file("今日.html", "<p>今日</p>"),
            file("待裁决列表.txt", "Q-001｜周会\nQ-002｜外包\n"),
            file("待裁决/Q-001.txt", "Q-001"),
            file("待裁决/Q-002.txt", "Q-002"),
        ];
        let done = write_all(&reading, &first).unwrap();
        assert_eq!(
            done,
            Written {
                written: 4,
                unchanged: 0,
                removed: 0
            }
        );
        assert_eq!(
            fs::read_to_string(reading.join("待裁决/Q-002.txt")).unwrap(),
            "Q-002"
        );

        let second = vec![
            file("今日.html", "<p>今日</p>"),
            file("待裁决列表.txt", "Q-001｜周会\n"),
            file("待裁决/Q-001.txt", "Q-001"),
        ];
        let done = write_all(&reading, &second).unwrap();
        assert_eq!(
            done,
            Written {
                written: 1,
                unchanged: 2,
                removed: 1
            }
        );
        assert!(!reading.join("待裁决/Q-002.txt").exists());

        let done = write_all(&reading, &[]).unwrap();
        assert_eq!(done.removed, 3);
        assert_eq!(
            fs::read_to_string(reading.join("我的笔记.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("收件/记一笔/a.txt")).unwrap(),
            "phone"
        );
        // No temp files are left behind.
        let left: Vec<String> = fs::read_dir(&reading)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let mut left = left;
        left.sort();
        assert_eq!(left, ["待裁决", "我的笔记.txt"]);
    }

    #[test]
    fn refuses_a_bad_export_before_writing_anything() {
        let dir = TempDir::new("export-bad");
        let reading = dir.mkdir("阅读");
        let bad = vec![file("今日.html", "x"), file("../outside.html", "x")];
        assert!(write_all(&reading, &bad).is_err());
        assert!(!reading.join("今日.html").exists());
        let twice = vec![file("今日.html", "x"), file("今日.html", "y")];
        assert!(write_all(&reading, &twice).unwrap_err().contains("两次"));
    }
}
