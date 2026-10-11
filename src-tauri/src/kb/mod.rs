//! Knowledge-base access for the desktop app: where the knowledge base lives
//! (`root`), which request paths are safe (`paths`), listing and atomically
//! writing Markdown files (`files`), running git and committing (`git`),
//! replacing and creating files with one commit each (`ops`), quick captures
//! (`capture`), syncing with the remote (`sync`), and the lock that keeps
//! all of that one at a time (`lock`).
//!
//! Everything here is plain Rust with no Tauri types, so it is unit-tested
//! directly; `crate::commands` wires it up as Tauri commands.

pub mod capture;
pub mod files;
pub mod git;
pub mod lock;
pub mod ops;
pub mod paths;
pub mod root;
pub mod sync;

#[cfg(test)]
pub(crate) mod test_support;
