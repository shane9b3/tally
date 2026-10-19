//! The native menu. Native items (Cut/Copy/Paste/Select All, window items)
//! are handled by the OS; every other item emits `menu-action { id }` to the
//! webview, where the id is a `CommandRegistry` command id.

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

use crate::events::emit_menu_action;

/// A custom menu entry. `Item` ids are `CommandRegistry` command ids
/// (packages/app/src/commands.ts); choosing one emits `menu-action { id }`.
pub enum Entry {
    /// (id, label, accelerator).
    Item(&'static str, &'static str, Option<&'static str>),
    /// A separator.
    Sep,
    /// A nested submenu.
    Sub(&'static str, &'static [Entry]),
}

use Entry::{Item, Sep, Sub};

const FILE: &[Entry] = &[
    Item("file.new", "New", Some("CmdOrCtrl+N")),
    Item("file.open", "Open…", Some("CmdOrCtrl+O")),
    Sep,
    Item("file.save", "Save", Some("CmdOrCtrl+S")),
    Item("file.saveAs", "Save As…", Some("Shift+CmdOrCtrl+S")),
    Sub(
        "Export",
        &[
            Item("file.exportXlsx", "Excel Workbook (.xlsx)…", None),
            Item("file.exportOds", "OpenDocument (.ods)…", None),
            Item("file.exportCsv", "CSV (.csv)…", None),
        ],
    ),
    Sep,
    Item("file.close", "Close", Some("CmdOrCtrl+W")),
];

/// Custom Edit items after the native Cut/Copy/Paste.
const EDIT_PASTE: &[Entry] = &[
    Item(
        "edit.pasteValues",
        "Paste Values",
        Some("Shift+CmdOrCtrl+V"),
    ),
    Item(
        "edit.pasteSpecial",
        "Paste Special…",
        Some("Alt+CmdOrCtrl+V"),
    ),
    Item("edit.delete", "Delete", None),
];

const EDIT_TAIL: &[Entry] = &[
    Item("edit.fillDown", "Fill Down", Some("CmdOrCtrl+D")),
    Item("edit.fillRight", "Fill Right", Some("CmdOrCtrl+R")),
    Sep,
    Item("edit.find", "Find…", Some("CmdOrCtrl+F")),
    Item(
        "edit.findReplace",
        "Find & Replace…",
        Some("Alt+CmdOrCtrl+F"),
    ),
];

const NUMBER_FORMATS: &[Entry] = &[
    Item("format.number.general", "General", None),
    Item("format.number.number", "Number", None),
    Item("format.number.thousands", "Number with Separators", None),
    Item("format.currency", "Currency", None),
    Item("format.percent", "Percent", None),
    Item("format.number.date", "Date", None),
    Item("format.number.time", "Time", None),
    Item("format.number.scientific", "Scientific", None),
    Item("format.number.text", "Text", None),
    Sep,
    Item("format.number.custom", "Custom…", None),
];

const BORDERS: &[Entry] = &[
    Item("format.borders.all", "All Borders", None),
    Item("format.borders.outer", "Outer Borders", None),
    Item("format.borders.inner", "Inner Borders", None),
    Sep,
    Item("format.borders.top", "Top Border", None),
    Item("format.borders.bottom", "Bottom Border", None),
    Item("format.borders.left", "Left Border", None),
    Item("format.borders.right", "Right Border", None),
    Sep,
    Item("format.borders.none", "No Borders", None),
];

const FORMAT: &[Entry] = &[
    Item("format.bold", "Bold", Some("CmdOrCtrl+B")),
    Item("format.italic", "Italic", Some("CmdOrCtrl+I")),
    Item("format.underline", "Underline", Some("CmdOrCtrl+U")),
    Item("format.strike", "Strikethrough", None),
    Sep,
    Sub("Number Format", NUMBER_FORMATS),
    Sub("Borders", BORDERS),
    Sep,
    Item("format.merge", "Merge Cells", None),
    Item("format.unmerge", "Unmerge Cells", None),
    Sep,
    Item("edit.clearFormats", "Clear Formats", None),
];

const DATA: &[Entry] = &[
    Item("data.sortAsc", "Sort Ascending", None),
    Item("data.sortDesc", "Sort Descending", None),
    Item("data.sort", "Sort…", None),
    Sep,
    Item("data.filter", "AutoFilter", None),
    Item("data.clearFilters", "Clear Filters", None),
    Sep,
    Item("data.defineName", "Define Name…", Some("CmdOrCtrl+F3")),
    Item("data.manageNames", "Manage Names…", None),
    Sep,
    Item("data.recalc", "Recalculate All", Some("Shift+CmdOrCtrl+F9")),
];

const VIEW: &[Entry] = &[
    Item(
        "view.toggleSidebar",
        "Toggle Sidebar",
        Some("CmdOrCtrl+Backslash"),
    ),
    Item("view.glance", "At a Glance", None),
    Item("view.theme", "Theme: System / Light / Dark", None),
    Sep,
    Item("view.freeze", "Freeze Panes at Cursor", None),
    Item("view.freezeRows", "Freeze Rows Above Cursor", None),
    Item("view.freezeCols", "Freeze Columns Left of Cursor", None),
    Item("view.unfreeze", "Unfreeze Panes", None),
    Sep,
    Item("sheet.hideRows", "Hide Rows", None),
    Item("sheet.showRows", "Show Rows", None),
    Item("sheet.hideCols", "Hide Columns", None),
    Item("sheet.showCols", "Show Columns", None),
    Sep,
    Item("view.zoomIn", "Zoom In", Some("CmdOrCtrl+Equal")),
    Item("view.zoomOut", "Zoom Out", Some("CmdOrCtrl+Minus")),
    Item("view.zoomReset", "Actual Size", Some("CmdOrCtrl+0")),
];

const SHEET: &[Entry] = &[
    Item("sheet.insert", "Insert Sheet", None),
    Item("sheet.rename", "Rename Sheet…", None),
    Item("sheet.delete", "Delete Sheet", None),
    Sep,
    Item("sheet.moveLeft", "Move Sheet Left", None),
    Item("sheet.moveRight", "Move Sheet Right", None),
];

const HELP: &[Entry] = &[
    Item(
        "help.shortcuts",
        "Keyboard Shortcuts",
        Some("CmdOrCtrl+Slash"),
    ),
    Item("help.about", "About Tally", None),
];

fn collect(entries: &'static [Entry], out: &mut Vec<&'static str>) {
    for e in entries {
        match e {
            Item(id, _, _) => out.push(id),
            Sep => {}
            Sub(_, inner) => collect(inner, out),
        }
    }
}

/// Every id that emits `menu-action` (tests and docs).
pub fn custom_ids() -> Vec<&'static str> {
    let mut out = vec!["edit.undo", "edit.redo"];
    for group in [FILE, EDIT_PASTE, EDIT_TAIL, FORMAT, DATA, VIEW, SHEET, HELP] {
        collect(group, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn add_entries<'a, R: Runtime>(
    app: &'a AppHandle<R>,
    mut b: SubmenuBuilder<'a, R, AppHandle<R>>,
    entries: &[Entry],
) -> tauri::Result<SubmenuBuilder<'a, R, AppHandle<R>>> {
    for e in entries {
        b = match e {
            Sep => b.separator(),
            Item(id, label, accel) => {
                let mut item = MenuItemBuilder::with_id(*id, *label);
                if let Some(a) = accel {
                    item = item.accelerator(*a);
                }
                b.item(&item.build(app)?)
            }
            Sub(title, inner) => b.item(&submenu(app, title, inner)?),
        };
    }
    Ok(b)
}

fn submenu<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    items: &[Entry],
) -> tauri::Result<Submenu<R>> {
    add_entries(app, SubmenuBuilder::new(app, title), items)?.build()
}

/// Builds the application menu.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let undo = MenuItemBuilder::with_id("edit.undo", "Undo")
        .accelerator("CmdOrCtrl+Z")
        .build(app)?;
    let redo = MenuItemBuilder::with_id("edit.redo", "Redo")
        .accelerator("Shift+CmdOrCtrl+Z")
        .build(app)?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&undo)
        .item(&redo)
        .separator()
        .cut()
        .copy()
        .paste();
    let edit = add_entries(app, edit, EDIT_PASTE)?
        .separator()
        .select_all()
        .separator();
    let edit = add_entries(app, edit, EDIT_TAIL)?.build()?;

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
        .item(&submenu(app, "Format", FORMAT)?)
        .item(&submenu(app, "Data", DATA)?)
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
    }
    let help = if cfg!(target_os = "macos") {
        // "About" lives in the app menu on macOS.
        submenu(app, "Help", &HELP[..1])?
    } else {
        submenu(app, "Help", HELP)?
    };
    menu = menu.item(&help);
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
    fn ids_are_namespaced() {
        let ids = custom_ids();
        assert!(ids.iter().all(|id| id.contains('.')));
        for id in [
            "edit.find",
            "edit.findReplace",
            "edit.pasteSpecial",
            "edit.fillDown",
            "format.borders.outer",
            "format.merge",
            "data.sortAsc",
            "data.filter",
            "data.defineName",
            "view.freeze",
            "view.unfreeze",
            "sheet.hideRows",
            "sheet.moveLeft",
            "help.shortcuts",
            "file.exportXlsx",
        ] {
            assert!(ids.contains(&id), "{id}");
        }
    }

    #[test]
    fn accelerators_are_unique() {
        fn walk(entries: &'static [Entry], out: &mut Vec<&'static str>) {
            for e in entries {
                match e {
                    Item(_, _, Some(a)) => out.push(a),
                    Sub(_, inner) => walk(inner, out),
                    _ => {}
                }
            }
        }
        let mut accels = vec!["CmdOrCtrl+Z", "Shift+CmdOrCtrl+Z"];
        for group in [FILE, EDIT_PASTE, EDIT_TAIL, FORMAT, DATA, VIEW, SHEET, HELP] {
            walk(group, &mut accels);
        }
        let n = accels.len();
        accels.sort_unstable();
        accels.dedup();
        assert_eq!(accels.len(), n);
    }
}
