//! The native menu. Native items (Cut/Copy/Paste/Select All, window items)
//! are handled by the OS; every other item emits `menu-action { id }` to the
//! webview, where the id is a `CommandRegistry` command id.
//!
//! Wave-5 items (W5-0b) are listed already so the menu, the registry and
//! `menuIds.test.ts` stay in sync; the registry has them `enabled: false`
//! ("coming in wave 5") until W5-5/W5-6 implement them. Labels are English
//! for now; the `t()` keys are the `cmd.<id>` entries of
//! `crates/tally-l10n/ui/en.json` (W5-6E wires the native strings).

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

use crate::events::emit_menu_action;
use crate::l10n;
use tally_l10n::Locale;

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
    Item("file.templates", "New from Template…", None),
    Item("file.open", "Open…", Some("CmdOrCtrl+O")),
    Sub(
        "Open Recent",
        &[Item("file.clearRecent", "Clear Menu", None)],
    ),
    Sep,
    Item("file.save", "Save", Some("CmdOrCtrl+S")),
    Item("file.saveAs", "Save As…", Some("Shift+CmdOrCtrl+S")),
    Item("file.saveAsTemplate", "Save as Template…", None),
    Sub(
        "Export",
        &[
            Item("file.exportXlsx", "Excel Workbook (.xlsx)…", None),
            Item("file.exportOds", "OpenDocument (.ods)…", None),
            Item("file.exportCsv", "CSV (.csv)…", None),
            Item("file.exportPdf", "PDF (.pdf)…", None),
        ],
    ),
    Sep,
    Item("file.printPreview", "Print Preview", None),
    Item("file.print", "Print…", Some("CmdOrCtrl+P")),
    Sep,
    Item("file.close", "Close", Some("CmdOrCtrl+W")),
];

const INSERT: &[Entry] = &[
    Item("insert.function", "Function…", Some("CmdOrCtrl+F2")),
    Item("insert.chart", "Chart…", None),
    Item("insert.image", "Image…", None),
    Sep,
    Item("insert.note", "Note", Some("Shift+F2")),
    Item("insert.hyperlink", "Hyperlink…", Some("Alt+CmdOrCtrl+K")),
    Sep,
    Item("insert.pageBreak", "Page Break Above Row", None),
    Item("insert.colPageBreak", "Page Break Left of Column", None),
    Item(
        "insert.removePageBreak",
        "Remove Page Breaks at Cursor",
        None,
    ),
];

const LOCALES: &[Entry] = &[
    Item("view.locale.enUS", "English (US)", None),
    Item("view.locale.enGB", "English (UK)", None),
    Item("view.locale.deDE", "Deutsch (Deutschland)", None),
    Item("view.locale.frFR", "Français (France)", None),
    Item("view.locale.esES", "Español (España)", None),
    Item("view.locale.jaJP", "日本語 (日本)", None),
];

const DETECTIVE: &[Entry] = &[
    Item(
        "tools.tracePrecedents",
        "Trace Precedents",
        Some("Shift+F7"),
    ),
    Item(
        "tools.traceDependents",
        "Trace Dependents",
        Some("Shift+F5"),
    ),
    Item("tools.removeTraces", "Remove All Traces", None),
    Item("tools.markInvalid", "Mark Invalid Data", None),
];

const TOOLS: &[Entry] = &[
    Item("tools.protectSheet", "Protect Sheet…", None),
    Item("tools.protectDocument", "Protect Document…", None),
    Sep,
    Item("tools.goalSeek", "Goal Seek…", None),
    Item("tools.solver", "Solver…", None),
    Item("tools.scenarios", "Scenarios…", None),
    Sub("Detective", DETECTIVE),
    Sep,
    Item("tools.autoCorrect", "AutoCorrect Options…", None),
    Item("tools.autoInput", "AutoInput", None),
    Sep,
    Item("file.properties", "Document Properties…", None),
    Item("file.documentStats", "Document Statistics…", None),
    Item("file.pageSetup", "Page Setup…", None),
    Item("tools.options", "Options…", Some("CmdOrCtrl+Comma")),
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
    Item("edit.fillSeries", "Fill Series…", None),
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
    Item("format.cells", "Cells…", Some("CmdOrCtrl+1")),
    Item("view.stylesPanel", "Styles", Some("CmdOrCtrl+F11")),
    Item("format.autoFormat", "AutoFormat…", None),
    Sep,
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
    Item("format.conditional", "Conditional Formatting…", None),
    Item("data.validation", "Data Validation…", None),
    Item("format.fitRows", "Fit Row Height to Contents", None),
    Sep,
    Sub(
        "Chart",
        &[
            Item("chart.edit", "Edit Chart…", None),
            Item("chart.delete", "Delete Chart", None),
        ],
    ),
    Sep,
    Item("edit.clearFormats", "Clear Formats", None),
];

const FILTERS: &[Entry] = &[
    Item("data.filter", "AutoFilter", None),
    Item("data.standardFilter", "Standard Filter…", None),
    Item("data.advancedFilter", "Advanced Filter…", None),
    Sep,
    Item("data.resetFilter", "Reset Filter", None),
    Item("data.clearFilters", "Clear Filters", None),
    Item("data.refreshFilter", "Refresh Filter", None),
];

const OUTLINE: &[Entry] = &[
    Item("data.group", "Group…", Some("F12")),
    Item("data.ungroup", "Ungroup…", Some("CmdOrCtrl+F12")),
    Item("data.autoOutline", "AutoOutline", None),
    Item("data.showOutlineLevel", "Show Level…", None),
    Item("data.clearOutline", "Remove Outline", None),
];

const DATA_PILOT: &[Entry] = &[
    Item("data.dataPilot", "Start…", None),
    Item("data.refreshPivot", "Refresh", None),
];

const DATA: &[Entry] = &[
    Item("data.sortAsc", "Sort Ascending", None),
    Item("data.sortDesc", "Sort Descending", None),
    Item("data.sort", "Sort…", None),
    Item("data.sortOptions", "Sort with Options…", None),
    Sep,
    Sub("Filter", FILTERS),
    Item("data.subtotals", "Subtotals…", None),
    Item("data.consolidate", "Consolidate…", None),
    Item("data.textToColumns", "Text to Columns…", None),
    Item("data.removeDuplicates", "Remove Duplicates…", None),
    Item("data.multipleOperations", "Multiple Operations…", None),
    Sub("Group and Outline", OUTLINE),
    Sub("DataPilot", DATA_PILOT),
    Sep,
    Item("data.defineName", "Define Name…", Some("CmdOrCtrl+F3")),
    Item("data.manageNames", "Manage Names…", None),
    Item(
        "data.createNamesFromLabels",
        "Create Names from Labels…",
        None,
    ),
    Item("data.defineDatabaseRange", "Define Database Range…", None),
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
    Item("view.navigator", "Navigator", Some("F5")),
    Item("view.theme", "Theme: System / Light / Dark", None),
    Sep,
    Item("view.showFormulas", "Show Formulas", Some("Ctrl+Backquote")),
    Item(
        "view.valueHighlighting",
        "Value Highlighting",
        Some("CmdOrCtrl+F8"),
    ),
    Item("view.zeroValues", "Zero Values", None),
    Item("view.gridLines", "Grid Lines", None),
    Item("view.headers", "Column and Row Headers", None),
    Sep,
    Item("view.freeze", "Freeze Panes at Cursor", None),
    Item("view.freezeRows", "Freeze Rows Above Cursor", None),
    Item("view.freezeCols", "Freeze Columns Left of Cursor", None),
    Item("view.unfreeze", "Unfreeze Panes", None),
    Item("view.split", "Split Window", None),
    Item("view.removeSplit", "Remove Split", None),
    Sep,
    Item("view.pageBreaks", "Page Breaks", None),
    Item("view.pageBreakPreview", "Page Break Preview", None),
    Sub("Locale", LOCALES),
    Sep,
    Item("sheet.hideRows", "Hide Rows", None),
    Item("sheet.showRows", "Show Rows", None),
    Item("sheet.hideCols", "Hide Columns", None),
    Item("sheet.showCols", "Show Columns", None),
    Sep,
    Item("view.zoomIn", "Zoom In", Some("CmdOrCtrl+Equal")),
    Item("view.zoomOut", "Zoom Out", Some("CmdOrCtrl+Minus")),
    Item("view.zoomReset", "Actual Size", Some("CmdOrCtrl+0")),
    Item("view.zoomOptimal", "Optimal Zoom", None),
    Item("view.zoomPageWidth", "Zoom to Page Width", None),
    Item("view.zoomWholePage", "Zoom to Whole Page", None),
    Sep,
    Item("view.fullScreen", "Full Screen", None),
];

const SHEET: &[Entry] = &[
    Item("sheet.insertCells", "Insert Cells…", None),
    Item("sheet.deleteCells", "Delete Cells…", None),
    Sep,
    Item("sheet.insert", "Insert Sheet", None),
    Item("sheet.duplicate", "Copy Sheet…", None),
    Item("sheet.rename", "Rename Sheet…", None),
    Item("sheet.delete", "Delete Sheet", None),
    Sep,
    Item("sheet.hide", "Hide Sheet", None),
    Item("sheet.show", "Show Sheet…", None),
    Item("sheet.tabColor", "Tab Colour…", None),
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
    for group in [
        FILE, EDIT_PASTE, EDIT_TAIL, INSERT, FORMAT, DATA, VIEW, SHEET, TOOLS, HELP,
    ] {
        collect(group, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn add_entries<'a, R: Runtime>(
    app: &'a AppHandle<R>,
    locale: Locale,
    mut b: SubmenuBuilder<'a, R, AppHandle<R>>,
    entries: &[Entry],
) -> tauri::Result<SubmenuBuilder<'a, R, AppHandle<R>>> {
    for e in entries {
        b = match e {
            Sep => b.separator(),
            Item(id, label, accel) => {
                let mut item = MenuItemBuilder::with_id(*id, l10n::item_label(locale, id, label));
                if let Some(a) = accel {
                    item = item.accelerator(*a);
                }
                b.item(&item.build(app)?)
            }
            Sub(title, inner) => b.item(&submenu(app, locale, title, inner)?),
        };
    }
    Ok(b)
}

fn submenu<R: Runtime>(
    app: &AppHandle<R>,
    locale: Locale,
    title: &str,
    items: &[Entry],
) -> tauri::Result<Submenu<R>> {
    add_entries(
        app,
        locale,
        SubmenuBuilder::new(app, l10n::submenu_label(locale, title)),
        items,
    )?
    .build()
}

/// The app menu's Quit label (`app.quit`, "Quit Tally").
fn quit_label(locale: Locale) -> String {
    if locale == Locale::En || !tally_l10n::has_ui_key("app.quit") {
        return "Quit Tally".to_owned();
    }
    tally_l10n::t(locale, "app.quit").to_owned()
}

/// Builds the application menu in the UI language stored in the prefs.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    build_localized(app, l10n::current_locale(app))
}

/// Builds the application menu with `tally_l10n` labels for `locale` (W5-6).
pub fn build_localized<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> tauri::Result<Menu<R>> {
    let undo = MenuItemBuilder::with_id("edit.undo", l10n::item_label(locale, "edit.undo", "Undo"))
        .accelerator("CmdOrCtrl+Z")
        .build(app)?;
    let redo = MenuItemBuilder::with_id("edit.redo", l10n::item_label(locale, "edit.redo", "Redo"))
        .accelerator("Shift+CmdOrCtrl+Z")
        .build(app)?;
    let edit = SubmenuBuilder::new(app, l10n::submenu_label(locale, "Edit"))
        .item(&undo)
        .item(&redo)
        .separator()
        .cut()
        .copy()
        .paste();
    let edit = add_entries(app, locale, edit, EDIT_PASTE)?
        .separator()
        .select_all()
        .separator();
    let edit = add_entries(app, locale, edit, EDIT_TAIL)?.build()?;

    let mut menu = MenuBuilder::new(app);
    if cfg!(target_os = "macos") {
        let app_menu = SubmenuBuilder::new(app, "Tally")
            .item(
                &MenuItemBuilder::with_id(
                    "help.about",
                    l10n::item_label(locale, "help.about", "About Tally"),
                )
                .build(app)?,
            )
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            // Not the predefined Quit (`terminate:` bypasses the unsaved-changes
            // guard): `on_event` routes it to `closing::request_quit`.
            .item(
                &MenuItemBuilder::with_id(crate::closing::QUIT_ID, quit_label(locale))
                    .accelerator("CmdOrCtrl+Q")
                    .build(app)?,
            )
            .build()?;
        menu = menu.item(&app_menu);
    }
    menu = menu
        .item(&submenu(app, locale, "File", FILE)?)
        .item(&edit)
        .item(&submenu(app, locale, "Insert", INSERT)?)
        .item(&submenu(app, locale, "Format", FORMAT)?)
        .item(&submenu(app, locale, "Data", DATA)?)
        .item(&submenu(app, locale, "View", VIEW)?)
        .item(&submenu(app, locale, "Sheet", SHEET)?)
        .item(&submenu(app, locale, "Tools", TOOLS)?);
    if cfg!(target_os = "macos") {
        let window = SubmenuBuilder::new(app, l10n::submenu_label(locale, "Window"))
            .minimize()
            .maximize()
            .separator()
            .fullscreen()
            .build()?;
        menu = menu.item(&window);
    }
    let help = if cfg!(target_os = "macos") {
        // "About" lives in the app menu on macOS.
        submenu(app, locale, "Help", &HELP[..1])?
    } else {
        submenu(app, locale, "Help", HELP)?
    };
    menu = menu.item(&help);
    menu.build()
}

/// Forwards custom items to the webview.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if id == crate::closing::QUIT_ID {
        crate::closing::request_quit(app);
    } else if custom_ids().contains(&id) {
        emit_menu_action(app, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_is_a_guarded_custom_item() {
        assert_eq!(quit_label(Locale::En), "Quit Tally");
        assert_eq!(quit_label(Locale::De), "Tally beenden");
        // Handled in Rust (closing::request_quit), never forwarded to the webview.
        assert!(!custom_ids().contains(&crate::closing::QUIT_ID));
    }

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
            "format.conditional",
            "data.validation",
            "data.refreshFilter",
            "insert.note",
            "insert.hyperlink",
            "tools.protectSheet",
            "tools.protectDocument",
            "tools.goalSeek",
            "file.properties",
            "file.pageSetup",
            "view.locale.deDE",
            "view.pageBreaks",
            // wave 5 (W5-0b seams)
            "insert.function",
            "insert.image",
            "chart.edit",
            "chart.delete",
            "data.dataPilot",
            "data.standardFilter",
            "data.advancedFilter",
            "data.resetFilter",
            "data.subtotals",
            "data.consolidate",
            "data.textToColumns",
            "data.removeDuplicates",
            "data.group",
            "data.ungroup",
            "data.autoOutline",
            "data.showOutlineLevel",
            "data.multipleOperations",
            "data.defineDatabaseRange",
            "data.createNamesFromLabels",
            "edit.fillSeries",
            "format.cells",
            "format.autoFormat",
            "view.stylesPanel",
            "view.pageBreakPreview",
            "view.split",
            "view.navigator",
            "view.showFormulas",
            "view.valueHighlighting",
            "view.zeroValues",
            "view.gridLines",
            "view.headers",
            "view.zoomOptimal",
            "view.zoomPageWidth",
            "view.zoomWholePage",
            "view.fullScreen",
            "tools.options",
            "tools.solver",
            "tools.scenarios",
            "tools.tracePrecedents",
            "tools.traceDependents",
            "tools.removeTraces",
            "tools.autoCorrect",
            "tools.autoInput",
            "file.printPreview",
            "file.print",
            "file.exportPdf",
            "file.saveAsTemplate",
            "file.templates",
            "file.clearRecent",
            "sheet.duplicate",
            "sheet.hide",
            "sheet.show",
            "sheet.tabColor",
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
        for group in [
            FILE, EDIT_PASTE, EDIT_TAIL, INSERT, FORMAT, DATA, VIEW, SHEET, TOOLS, HELP,
        ] {
            walk(group, &mut accels);
        }
        let n = accels.len();
        accels.sort_unstable();
        accels.dedup();
        assert_eq!(accels.len(), n);
    }
}
