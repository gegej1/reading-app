fn main() {
    // Declaring the app commands makes tauri-build generate `allow-<command>`
    // and `deny-<command>` permissions for them, and makes the ACL check app
    // commands too: a window can only call what its capability grants.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&["kb_root", "kb_list", "kb_read", "kb_write"]),
    ))
    .expect("failed to run tauri-build");
}
