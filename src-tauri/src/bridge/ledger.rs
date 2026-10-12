//! The record of files imported from the phone.
//!
//! An imported file is deleted from iCloud only after its commit. If that
//! deletion fails (iCloud still holds the file, the folder is read-only for a
//! moment), the next pass must not import it again. So every import is
//! written down here, by a hash of its kind, name and bytes, in two steps:
//!
//! 1. before the commit, as *pending*, with the knowledge-base path it is
//!    about to become ([`Ledger::begin`]); when that cannot be written, the
//!    file is not imported at all;
//! 2. after the commit, as done, with the path it became ([`Ledger::finish`]).
//!
//! A file whose entry is done is only deleted, never imported again. A file
//! whose entry is still pending (the app stopped between the commit and the
//! second step, or the second step could not be written) is looked up in the
//! knowledge base first (see `super::import`).
//!
//! The record lives in the app's own folder (`iphone-imported.json`), keeps
//! the newest [`MAX_ENTRIES`] imports and holds no content, only hashes and
//! the knowledge-base paths the imports became.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::kb::files;

/// File name inside the app's folder.
pub const FILE_NAME: &str = "iphone-imported.json";
/// How many imports are remembered.
pub const MAX_ENTRIES: usize = 2000;

/// One import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    /// The capture time written into the file (ISO 8601, local time).
    pub at: String,
    /// The knowledge-base file it became (while pending: is about to become).
    pub path: String,
    /// Written down before the commit; not yet confirmed after it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    imported: Vec<Entry>,
}

/// The record, loaded from and saved to one file.
#[derive(Debug)]
pub struct Ledger {
    file: Option<PathBuf>,
    entries: Vec<Entry>,
}

/// The key of one incoming file: its kind (`记一笔`, `回答`), its name and
/// its bytes.
pub fn key(kind: &str, name: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(name.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Ledger {
    /// Loads the record. A missing file is an empty record. A file that is
    /// there but is not a record (cut off, edited by hand) is set aside
    /// (renamed with `.broken`) so it is not lost, and the record starts
    /// empty. A file that cannot be read now (permissions, a busy disk) is an
    /// error and stays as it is: no import may run without its record.
    pub fn load(file: Option<PathBuf>) -> Result<Self, String> {
        let entries = match file.as_deref().map(fs::read_to_string) {
            None => Vec::new(),
            Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Some(Err(error)) => return Err(format!("导入记录读不出来（{error}）")),
            Some(Ok(text)) => match serde_json::from_str::<Stored>(&text) {
                Ok(stored) => stored.imported,
                Err(error) => {
                    set_aside(file.as_deref(), &error.to_string())?;
                    Vec::new()
                }
            },
        };
        Ok(Ledger { file, entries })
    }

    /// The entry for `key`, if the file was (being) imported before.
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Writes down, before the commit, that `key` is about to become `path`.
    /// Nothing may be committed when this fails.
    pub fn begin(&mut self, key: &str, at: &str, path: &str) -> Result<(), String> {
        self.put(Entry {
            key: key.to_string(),
            at: at.to_string(),
            path: path.to_string(),
            pending: true,
        })
    }

    /// Writes down, after the commit, that `key` became `path`.
    pub fn finish(&mut self, key: &str, at: &str, path: &str) -> Result<(), String> {
        self.put(Entry {
            key: key.to_string(),
            at: at.to_string(),
            path: path.to_string(),
            pending: false,
        })
    }

    /// Replaces the entry for the same key (or adds it) and saves the record;
    /// the record in memory changes only once the file was written.
    fn put(&mut self, entry: Entry) -> Result<(), String> {
        let mut entries: Vec<Entry> = self
            .entries
            .iter()
            .filter(|old| old.key != entry.key)
            .cloned()
            .collect();
        entries.push(entry);
        if entries.len() > MAX_ENTRIES {
            let excess = entries.len() - MAX_ENTRIES;
            entries.drain(..excess);
        }
        if let Some(file) = &self.file {
            write(file, &entries).map_err(|error| format!("导入记录写不进去（{error}）"))?;
        }
        self.entries = entries;
        Ok(())
    }
}

fn write(file: &Path, entries: &[Entry]) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&Stored {
        imported: entries.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    files::write_atomic(file, &format!("{text}\n")).map_err(|error| error.to_string())
}

/// Moves a record that is not one out of the way. When even that fails, the
/// pass stops: a new record must not overwrite it.
fn set_aside(file: Option<&Path>, reason: &str) -> Result<(), String> {
    let Some(file) = file else { return Ok(()) };
    let mut broken = file.as_os_str().to_owned();
    broken.push(".broken");
    fs::rename(file, PathBuf::from(broken))
        .map_err(|error| format!("导入记录坏了（{reason}），也没能挪开（{error}）"))?;
    log::warn!(
        "iphone: the import record was not readable as a record ({reason}); kept as {}.broken, starting a new one",
        FILE_NAME
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;
    use std::os::unix::fs::PermissionsExt;

    const AT: &str = "2026-10-12T14:03:22+08:00";
    const PATH: &str = "inbox/2026-10-12T14-03-22-iphone.md";

    #[test]
    fn keys_depend_on_kind_name_and_bytes() {
        let a = key("记一笔", "a.txt", b"x");
        assert_eq!(a.len(), 64);
        assert_eq!(a, key("记一笔", "a.txt", b"x"));
        assert_ne!(a, key("回答", "a.txt", b"x"));
        assert_ne!(a, key("记一笔", "b.txt", b"x"));
        assert_ne!(a, key("记一笔", "a.txt", b"y"));
        // The separator keeps name and bytes apart.
        assert_ne!(key("k", "ab", b"c"), key("k", "a", b"bc"));
    }

    #[test]
    fn remembers_pending_and_done_imports_across_loads() {
        let dir = TempDir::new("ledger");
        let file = dir.path().join("app/iphone-imported.json");
        let mut ledger = Ledger::load(Some(file.clone())).unwrap();
        assert!(ledger.get("k1").is_none());
        ledger.begin("k1", AT, PATH).unwrap();
        let again = Ledger::load(Some(file.clone())).unwrap();
        assert!(again.get("k1").unwrap().pending);
        ledger
            .finish("k1", AT, "inbox/2026-10-12T14-03-22-iphone-2.md")
            .unwrap();
        let again = Ledger::load(Some(file.clone())).unwrap();
        let entry = again.get("k1").unwrap();
        assert!(!entry.pending);
        assert_eq!(entry.path, "inbox/2026-10-12T14-03-22-iphone-2.md");
        assert_eq!(again.len(), 1, "one entry per key");
        let text = fs::read_to_string(&file).unwrap();
        assert!(!text.contains("pending"), "{text}");
    }

    #[test]
    fn reads_records_written_before_the_pending_step() {
        let dir = TempDir::new("ledger-old");
        let file = dir.write(
            "iphone-imported.json",
            &format!(r#"{{ "imported": [{{ "key": "k1", "at": "{AT}", "path": "{PATH}" }}] }}"#),
        );
        let ledger = Ledger::load(Some(file)).unwrap();
        assert!(!ledger.get("k1").unwrap().pending);
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut ledger = Ledger::load(None).unwrap();
        for n in 0..MAX_ENTRIES + 5 {
            ledger.finish(&format!("k{n}"), AT, PATH).unwrap();
        }
        assert_eq!(ledger.len(), MAX_ENTRIES);
        assert!(ledger.get("k0").is_none());
        assert!(ledger.get(&format!("k{}", MAX_ENTRIES + 4)).is_some());
    }

    #[test]
    fn an_unwritable_record_changes_nothing() {
        let dir = TempDir::new("ledger-readonly");
        let app = dir.mkdir("app");
        let file = app.join(FILE_NAME);
        let mut ledger = Ledger::load(Some(file.clone())).unwrap();
        fs::set_permissions(&app, fs::Permissions::from_mode(0o555)).unwrap();
        let error = ledger.begin("k1", AT, PATH).unwrap_err();
        fs::set_permissions(&app, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(error.contains("导入记录写不进去"), "{error}");
        assert!(ledger.get("k1").is_none(), "not in memory either");
        assert!(!file.exists());
    }

    #[test]
    fn a_broken_record_is_set_aside() {
        let dir = TempDir::new("ledger-broken");
        let file = dir.write("iphone-imported.json", "{ broken");
        let ledger = Ledger::load(Some(file.clone())).unwrap();
        assert_eq!(ledger.len(), 0);
        assert!(dir.path().join("iphone-imported.json.broken").exists());
        assert!(!file.exists());
    }

    #[test]
    fn a_record_that_cannot_be_read_now_is_left_alone() {
        let dir = TempDir::new("ledger-unreadable");
        let content =
            format!(r#"{{ "imported": [{{ "key": "k1", "at": "{AT}", "path": "{PATH}" }}] }}"#);
        let file = dir.write("iphone-imported.json", &content);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        let result = Ledger::load(Some(file.clone()));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let error = result.unwrap_err();
        assert!(error.contains("读不出来"), "{error}");
        assert_eq!(fs::read_to_string(&file).unwrap(), content);
        assert!(!dir.path().join("iphone-imported.json.broken").exists());
        // A folder where the file should be: also a read error, not a broken record.
        let odd = dir.mkdir("odd.json");
        assert!(Ledger::load(Some(odd.clone())).is_err());
        assert!(odd.is_dir());
    }
}
