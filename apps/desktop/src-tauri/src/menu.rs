//! The native menu. Native items (Cut/Copy/Paste/Select All, window items)
//! are handled by the OS; every other item emits `menu-action { id }` to the
//! webview, where the id is a `CommandRegistry` command id.

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

use crate::events::emit_menu_action;

/// (id, label, accelerator) of custom items, grouped by submenu. `"-"` is a
/// separator.
type Item = (&'static str, &'static str, Option<&'static str>);

const FILE: &[Item] = &[
    ("file.new", "New", Some("CmdOrCtrl+N")),
    ("file.open", "Open…", Some("CmdOrCtrl+O")),
    ("-", "", None),
    ("file.save", "Save", Some("CmdOrCtrl+S")),
    ("file.saveAs", "Save As…", Some("Shift+CmdOrCtrl+S")),
    ("file.exportCsv", "Export CSV…", None),
    ("-", "", None),
    ("file.close", "Close", Some("CmdOrCtrl+W")),
];

const VIEW: &[Item] = &[
    (
        "view.toggleSidebar",
        "Toggle Sidebar",
        Some("CmdOrCtrl+Backslash"),
    ),
    ("-", "", None),
    ("view.zoomIn", "Zoom In", Some("CmdOrCtrl+Equal")),
    ("view.zoomOut", "Zoom Out", Some("CmdOrCtrl+Minus")),
    ("view.zoomReset", "Actual Size", Some("CmdOrCtrl+0")),
];

const SHEET: &[Item] = &[
    ("sheet.insert", "Insert Sheet", None),
    ("sheet.rename", "Rename Sheet…", None),
    ("sheet.delete", "Delete Sheet", None),
];

const HELP: &[Item] = &[("help.about", "About Tally", None)];

/// Every id that emits `menu-action` (tests and docs).
pub fn custom_ids() -> Vec<&'static str> {
    let edit = ["edit.undo", "edit.redo", "edit.delete"];
    FILE.iter()
        .chain(VIEW)
        .chain(SHEET)
        .chain(HELP)
        .map(|(id, _, _)| *id)
        .filter(|id| *id != "-")
        .chain(edit)
        .collect()
}

fn submenu<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    items: &[Item],
) -> tauri::Result<tauri::menu::Submenu<R>> {
    let mut b = SubmenuBuilder::new(app, title);
    for (id, label, accel) in items {
        if *id == "-" {
            b = b.separator();
            continue;
        }
        let mut item = MenuItemBuilder::with_id(*id, *label);
        if let Some(a) = accel {
            item = item.accelerator(*a);
        }
        b = b.item(&item.build(app)?);
    }
    b.build()
}

/// Builds the application menu.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let undo = MenuItemBuilder::with_id("edit.undo", "Undo")
        .accelerator("CmdOrCtrl+Z")
        .build(app)?;
    let redo = MenuItemBuilder::with_id("edit.redo", "Redo")
        .accelerator("Shift+CmdOrCtrl+Z")
        .build(app)?;
    let delete = MenuItemBuilder::with_id("edit.delete", "Delete").build(app)?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&undo)
        .item(&redo)
        .separator()
        .cut()
        .copy()
        .paste()
        .item(&delete)
        .separator()
        .select_all()
        .build()?;

    let mut menu = MenuBuilder::new(app);
    if cfg!(target_os = "macos") {
        let app_menu = SubmenuBuilder::new(app, "Tally")
            .item(&MenuItemBuilder::with_id("help.about", "About Tally").build(app)?)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        menu = menu.item(&app_menu);
    }
    menu = menu
        .item(&submenu(app, "File", FILE)?)
        .item(&edit)
        .item(&submenu(app, "View", VIEW)?)
        .item(&submenu(app, "Sheet", SHEET)?);
    if cfg!(target_os = "macos") {
        let window = SubmenuBuilder::new(app, "Window")
            .minimize()
            .maximize()
            .separator()
            .fullscreen()
            .build()?;
        menu = menu.item(&window);
    } else {
        menu = menu.item(&submenu(app, "Help", HELP)?);
    }
    menu.build()
}

/// Forwards custom items to the webview.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if custom_ids().contains(&id) {
        emit_menu_action(app, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_namespaced() {
        let ids = custom_ids();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        assert!(ids.iter().all(|id| id.contains('.')));
    }
}
