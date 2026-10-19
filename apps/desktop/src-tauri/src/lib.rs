//! Tally desktop shell (WP7): the Tauri window, native menu, file dialogs,
//! recent files, templates and one command per `DocumentService` method
//! (docs/api.md), over the real engine (`tally-engine` with ODS/XLSX/CSV IO) and
//! the rule-based ⌘K assist (`tally-assist`).

#![forbid(unsafe_code)]

mod assist;
mod commands;
mod dialogs;
mod events;
mod files;
mod limits;
mod links;
mod menu;
mod state;
#[cfg(test)]
mod tests;

use tally_api::DocumentService;
use tauri::Manager;

pub use state::AppState;

/// Starts the app.
pub fn run() {
    // The engine with ODS/CSV IO and the ⌘K rule provider installed.
    let service: Box<dyn DocumentService + Send> = Box::new(assist::build_service());

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(service))
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .setup(|app| {
            let recent = files::load_recent(app.handle());
            let state = app.state::<AppState>();
            *state::lock(&state.recent) = recent;
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
            commands::names,
            commands::filter_info,
            commands::find,
            commands::detect_header,
            commands::recalc_all,
            commands::conditional_formats,
            commands::validations,
            commands::validation_at,
            commands::note,
            commands::notes,
            commands::protection_info,
            commands::page_setup,
            commands::document_properties,
            commands::goal_seek,
            commands::list_locales,
            commands::set_locale,
            commands::resolve_name_box_in,
            commands::open_url,
            commands::user_name,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("tally: failed to start: {e}");
        std::process::exit(1);
    }
}
