//! Tally desktop shell (WP7): the Tauri window, native menu, file dialogs,
//! recent files, bundled templates and one command per `DocumentService`
//! method (docs/api.md).

#![forbid(unsafe_code)]

mod commands;
mod dialogs;
mod events;
mod files;
mod menu;
mod state;
mod stub;

use tally_api::DocumentService;
use tauri::Manager;

pub use state::AppState;

/// Starts the app.
pub fn run() {
    // The engine. WP1 swap (one line; also add `tally-engine` to Cargo.toml):
    // let service: Box<dyn DocumentService + Send> = Box::new(tally_engine::service::EngineService::new());
    let service: Box<dyn DocumentService + Send> = Box::new(stub::StubService::new());

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(service))
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .setup(|app| {
            let recent = files::load_recent(app.handle());
            let state = app.state::<AppState>();
            if let Ok(mut list) = state.recent.lock() {
                *list = recent;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::wb_new,
            dialogs::wb_open_dialog,
            commands::wb_open,
            dialogs::wb_save,
            commands::wb_close,
            commands::sheet_layout,
            commands::get_cells,
            commands::get_styles,
            commands::cell_edit_info,
            commands::apply,
            commands::undo,
            commands::redo,
            commands::selection_stats,
            commands::clipboard_copy,
            commands::navigate_edge,
            commands::resolve_name_box,
            commands::list_functions,
            commands::assist_suggest,
            commands::recent_files,
            commands::list_templates,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("tally: failed to start: {e}");
        std::process::exit(1);
    }
}
