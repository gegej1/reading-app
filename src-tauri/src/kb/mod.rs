//! Knowledge-base access for the desktop app: where the knowledge base lives
//! (`root`), which request paths are safe (`paths`), listing and atomically
//! writing Markdown files (`files`), and committing changes (`git`).
//!
//! Everything here is plain Rust with no Tauri types, so it is unit-tested
//! directly; `crate::commands` wires it up as Tauri commands.

pub mod files;
pub mod git;
pub mod paths;
pub mod root;

#[cfg(test)]
pub(crate) mod test_support;
