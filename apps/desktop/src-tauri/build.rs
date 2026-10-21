// The app command list shared with the `generate_handler!` test (app_commands.rs).
include!("app_commands.rs");

fn main() {
    println!("cargo:rerun-if-changed=app_commands.rs");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .unwrap_or_else(|e| panic!("tauri-build failed: {e:#}"));
}
