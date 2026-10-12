//! The record of files imported from the phone.
//!
//! An imported file is deleted from iCloud only after its commit. If that
//! deletion fails (iCloud still holds the file, the folder is read-only for a
//! moment), the next pass must not import it again: each import is written
//! down here by a hash of its kind, name and bytes first. A file whose hash
//! is in the record is only deleted, never imported again.
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
    /// When it was imported (ISO 8601, local time).
    pub at: String,
    /// The knowledge-base file it became.
    pub path: String,
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
    /// Loads the record. A missing file is an empty record; a file that cannot
    /// be read is set aside (renamed with `.broken`) so it is not lost, and the
    /// record starts empty.
    pub fn load(file: Option<PathBuf>) -> Self {
        let entries = match file.as_deref().map(fs::read_to_string) {
            None => Vec::new(),
            Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Some(Ok(text)) => match serde_json::from_str::<Stored>(&text) {
                Ok(stored) => stored.imported,
                Err(error) => {
                    set_aside(file.as_deref(), &error.to_string());
                    Vec::new()
                }
            },
            Some(Err(error)) => {
                set_aside(file.as_deref(), &error.to_string());
                Vec::new()
            }
        };
        Ledger { file, entries }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|entry| entry.key == key)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Writes one import down (and saves the record at once).
    pub fn record(&mut self, entry: Entry) -> Result<(), String> {
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(&Stored {
            imported: self.entries.clone(),
        })
        .map_err(|error| error.to_string())?;
        files::write_atomic(file, &format!("{text}\n")).map_err(|error| error.to_string())
    }
}

fn set_aside(file: Option<&Path>, reason: &str) {
    let Some(file) = file else { return };
    log::warn!("iphone: the import record could not be read ({reason}); starting a new one");
    let mut broken = file.as_os_str().to_owned();
    broken.push(".broken");
    let _ = fs::rename(file, PathBuf::from(broken));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::test_support::TempDir;

    fn entry(key: &str) -> Entry {
        Entry {
            key: key.to_string(),
            at: "2026-10-12T14:03:22+08:00".to_string(),
            path: "inbox/2026-10-12T14-03-22-iphone.md".to_string(),
        }
    }

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
    fn remembers_imports_across_loads() {
        let dir = TempDir::new("ledger");
        let file = dir.path().join("app/iphone-imported.json");
        let mut ledger = Ledger::load(Some(file.clone()));
        assert!(!ledger.contains("k1"));
        ledger.record(entry("k1")).unwrap();
        let again = Ledger::load(Some(file.clone()));
        assert!(again.contains("k1"));
        assert_eq!(again.len(), 1);
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("inbox/2026-10-12T14-03-22-iphone.md"));
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut ledger = Ledger::load(None);
        for n in 0..MAX_ENTRIES + 5 {
            ledger.record(entry(&format!("k{n}"))).unwrap();
        }
        assert_eq!(ledger.len(), MAX_ENTRIES);
        assert!(!ledger.contains("k0"));
        assert!(ledger.contains(&format!("k{}", MAX_ENTRIES + 4)));
    }

    #[test]
    fn a_broken_record_is_set_aside() {
        let dir = TempDir::new("ledger-broken");
        let file = dir.write("iphone-imported.json", "{ broken");
        let ledger = Ledger::load(Some(file.clone()));
        assert_eq!(ledger.len(), 0);
        assert!(dir.path().join("iphone-imported.json.broken").exists());
        assert!(!file.exists());
    }
}
