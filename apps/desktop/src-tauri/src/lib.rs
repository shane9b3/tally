//! Tally desktop shell (WP7): the document windows and tabs, native menu, file dialogs,
//! recent files, templates and one command per `DocumentService` method
//! (docs/api.md), over the real engine (`tally-engine` with ODS/XLSX/CSV IO) and
//! the rule-based ⌘K assist (`tally-assist`).

#![forbid(unsafe_code)]

mod assist;
mod autosave;
mod closing;
mod commands;
mod commands_v4;
mod dialogs;
mod events;
mod files;
mod images;
mod l10n;
mod limits;
mod links;
mod menu;
mod native;
mod prefs;
mod print;
mod registry;
mod state;
mod templates;
#[cfg(test)]
mod tests;
mod windows;

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
        .manage(registry::Windows::default())
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .on_window_event(closing::on_window_event)
        .register_uri_scheme_protocol(print::PRINT_SCHEME, print::protocol)
        .setup(|app| {
            let recent = files::load_recent(app.handle());
            let state = app.state::<AppState>();
            *state::lock(&state.recent) = recent;
            print::setup(app.handle());
            // Our own document list (the sidebar) replaces native macOS window tabs.
            native::disable_automatic_tabbing(app.handle());
            // The first document window (tauri.conf.json lists none: every
            // window is a `win-<n>` holding `doc-<n>` webviews, made by
            // `windows::open_document`).
            windows::open_document(app.handle(), registry::Target::NewWindow, None)
                .map_err(|e| e.message)?;
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
            // Wave 5 (W5-0b): docs/api.md "v0.4–v0.6"; stubs until W5-1..3.
            commands_v4::charts,
            commands_v4::chart_data,
            commands_v4::suggest_chart,
            commands_v4::images,
            commands_v4::pivots,
            commands_v4::pivot_field_items,
            commands_v4::pivot_at,
            commands_v4::autocomplete,
            commands_v4::cell_styles,
            commands_v4::scenarios,
            commands_v4::database_ranges,
            commands_v4::navigator,
            commands_v4::document_statistics,
            commands_v4::print_layout,
            commands_v4::page_breaks,
            commands_v4::prepare_print_job,
            commands_v4::print_job_page_png,
            commands_v4::print_job_pdf,
            commands_v4::calc_settings,
            commands_v4::solver,
            commands_v4::precedents,
            commands_v4::dependents,
            commands_v4::image_data,
            commands_v4::view_options,
            commands_v4::invalid_cells,
            commands_v4::custom_sort_lists,
            commands_v4::cells_with,
            commands_v4::preview_formula,
            commands_v4::wb_export_pdf,
            commands_v4::wb_print,
            // W5-5 shell commands: pictures and the window chrome.
            images::pick_image,
            windows::window_chrome,
            // Wave 5 (W5-6): prefs, autosave and recovery, templates, UI language.
            prefs::get_prefs,
            prefs::set_prefs,
            autosave::autosave_snapshot,
            autosave::list_recovery,
            autosave::recover_snapshot,
            autosave::discard_recovery,
            templates::save_as_template,
            templates::list_user_templates,
            templates::new_from_user_template,
            l10n::set_ui_language,
            l10n::ui_strings,
            l10n::localized_function_names,
            files::clear_recent,
            // Unsaved-changes guard and the macOS "edited" dot.
            closing::set_document_edited,
            closing::confirm_close,
            closing::cancel_quit,
            // Document windows (one document per window).
            windows::window_init,
            windows::place_document,
            prefs::broadcast_prefs,
            // Document tabs (several documents per window).
            windows::tab_activate,
            windows::tab_close,
            windows::tab_close_others,
            windows::tab_move,
            windows::tab_detach,
            windows::merge_windows,
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|handle, event| closing::on_run_event(handle, &event)),
        Err(e) => {
            eprintln!("tally: failed to start: {e}");
            std::process::exit(1);
        }
    }
}
