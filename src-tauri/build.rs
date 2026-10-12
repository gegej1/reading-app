fn main() {
    // Declaring the app commands makes tauri-build generate `allow-<command>`
    // and `deny-<command>` permissions for them, and makes the ACL check app
    // commands too: a window can only call what its capability grants.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "kb_root",
            "kb_list",
            "kb_read",
            "kb_read_head",
            "kb_write",
            "kb_create",
            "kb_status",
            "kb_sync",
            "capture_show",
            "capture_hide",
            "tray_set_counts",
            "desktop_info",
            "run_start",
            "run_cancel",
            "run_status",
            "run_steps",
            "run_gate_input",
            "run_gate_report",
            "run_discard",
            "run_keep",
            "run_approve",
            "run_retry_commit",
            "run_recheck",
            "run_history",
            "run_record",
            "run_settings",
            "settings_get",
            "settings_save_ai",
            "settings_check_kb",
            "settings_pick_kb",
            "settings_set_kb",
            "settings_set_shortcut",
            "settings_set_autostart",
            "settings_open_folder",
        ]),
    ))
    .expect("failed to run tauri-build");
}
