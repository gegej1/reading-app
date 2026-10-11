//! Tauri desktop entry point for 底色 (Dise).
//!
//! The shell only opens the main window for now. Commands, the menu-bar icon,
//! and plugins are added in later milestones.

/// Builds the desktop app from `tauri.conf.json` and runs it until exit.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run the Dise desktop app");
}
