//! The native menu. Native items (Cut/Copy/Paste/Select All, Minimize,
//! Zoom…) are handled by the OS; every other item emits `menu-action { id }`
//! to the front document's webview (the shown tab of the most recently
//! focused window; see [`route`]), where the id is a `CommandRegistry`
//! command id. With no document window (macOS keeps running without one)
//! File ▸ New and Open are handled here and open a window.
//!
//! The Window menu is the shell's own on every platform (not AppKit's
//! `windowsMenu`, which would list windows rather than documents and add
//! the native tab items): Minimize, Zoom, Show Next/Previous Tab, Move Tab
//! to New Window, Merge All Windows, Bring All to Front, then the open
//! documents ([`set_window_list`]; choosing one shows its window and tab).
//! Those items are handled here and never forwarded.
//!
//! Wave-5 items (W5-0b) are listed already so the menu, the registry and
//! `menuIds.test.ts` stay in sync; the registry has them `enabled: false`
//! ("coming in wave 5") until W5-5/W5-6 implement them. Labels are English
//! for now; the `t()` keys are the `cmd.<id>` entries of
//! `crates/tally-l10n/ui/en.json` (W5-6E wires the native strings).

use tauri::menu::{
    CheckMenuItemBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, MenuItemKind, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, Manager, Runtime};

use crate::events::emit_menu_action;
use crate::l10n;
use crate::registry::{ListEntry, Windows};
use crate::windows::FOCUS_PREFIX;
use tally_l10n::Locale;

/// Id of the Window menu (deliberately not `WINDOW_SUBMENU_ID`, which
/// would make it AppKit's `windowsMenu`).
pub const WINDOW_MENU_ID: &str = "window-menu";
/// Window ▸ Show Next Tab (⌃Tab).
pub const NEXT_TAB_ID: &str = "window.nextTab";
/// Window ▸ Show Previous Tab (⌃⇧Tab).
pub const PREV_TAB_ID: &str = "window.prevTab";
/// Window ▸ Move Tab to New Window.
pub const DETACH_TAB_ID: &str = "window.moveTabToNewWindow";
/// Window ▸ Merge All Windows.
pub const MERGE_ID: &str = "window.mergeAllWindows";
/// Window ▸ Bring All to Front off macOS (macOS uses the predefined item).
pub const BRING_ALL_ID: &str = "window.bringAllToFront";

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
    Item("file.openReadOnly", "Open Read-Only…", None),
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

/// AOO 4.1 `menubar.xml` `.uno:AuditMenu`, in its order.
const DETECTIVE: &[Entry] = &[
    Item(
        "tools.tracePrecedents",
        "Trace Precedents",
        Some("Shift+F7"),
    ),
    Item("tools.removePrecedents", "Remove Precedents", None),
    Item(
        "tools.traceDependents",
        "Trace Dependents",
        Some("Shift+F5"),
    ),
    Item("tools.removeDependents", "Remove Dependents", None),
    Item("tools.removeTraces", "Remove All Traces", None),
    Sep,
    Item("tools.traceError", "Trace Error", None),
    Item("tools.markInvalid", "Mark Invalid Data", None),
    Item("tools.refreshTraces", "Refresh Traces", None),
    Item("tools.autoRefreshTraces", "AutoRefresh", None),
    Item("tools.detectiveFillMode", "Fill Mode", None),
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

/// Edit ▸ Fill (AOO: Down, Right, Sheet…, Series…). Fill Sheets… is
/// enabled in the registry only with several sheets selected.
const FILL: &[Entry] = &[
    Item("edit.fillDown", "Fill Down", Some("CmdOrCtrl+D")),
    Item("edit.fillRight", "Fill Right", Some("CmdOrCtrl+R")),
    Item("edit.fillSheet", "Fill Sheets…", None),
    Item("edit.fillSeries", "Fill Series…", None),
];

const EDIT_TAIL: &[Entry] = &[
    Sub("Fill", FILL),
    Sep,
    Item("edit.find", "Find…", Some("CmdOrCtrl+F")),
    Item(
        "edit.findReplace",
        "Find & Replace…",
        Some("Alt+CmdOrCtrl+F"),
    ),
    Sep,
    // AOO 4.1's "Edit File" (`.uno:EditDoc`, Standard toolbar, no shortcut);
    // Tally adds the menu item and ⇧⌘M (LibreOffice's Edit Mode).
    Item("edit.editMode", "Edit Mode", Some("Shift+CmdOrCtrl+M")),
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

/// View ▸ Toolbars (AOO): the bars on and off, and Customize Toolbars…
/// (buttons per toolbar). The webview keeps the state in its prefs.
const TOOLBARS: &[Entry] = &[
    Item("view.toolbar", "Toolbar", None),
    Item("view.formattingBar", "Formatting Bar", None),
    Item("view.formulaBar", "Formula Bar", None),
    Item("view.statusBar", "Status Bar", None),
    Sep,
    Item("view.customizeToolbars", "Customize Toolbars…", None),
];

const VIEW: &[Entry] = &[
    Sub("Toolbars", TOOLBARS),
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
    Item("sheet.selectAll", "Select All Sheets", None),
    Item("sheet.deselectAll", "Deselect All Sheets", None),
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

/// A label from the UI table (`key`), English keeping `fallback`.
fn ui_label(locale: Locale, key: &str, fallback: &str) -> String {
    if locale == Locale::En || !tally_l10n::has_ui_key(key) {
        return fallback.to_owned();
    }
    tally_l10n::t(locale, key).to_owned()
}

/// The app menu's Quit label (`app.quit`, "Quit Tally").
fn quit_label(locale: Locale) -> String {
    ui_label(locale, "app.quit", "Quit Tally")
}

/// The Window menu: Minimize, Zoom (Maximize off macOS), Enter Full Screen
/// (macOS), the tab items, Bring All to Front; [`set_window_list`] appends
/// the documents after the trailing separator.
fn window_menu<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> tauri::Result<Submenu<R>> {
    let mac = cfg!(target_os = "macos");
    let item = |id: &str, key: &str, text: &str, accel: Option<&str>| {
        let mut b = MenuItemBuilder::with_id(id, ui_label(locale, key, text));
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        b.build(app)
    };
    let b = SubmenuBuilder::with_id(app, WINDOW_MENU_ID, l10n::submenu_label(locale, "Window"))
        .minimize_with_text(ui_label(locale, "menu.window.minimize", "Minimize"));
    let b = if mac {
        b.maximize_with_text(ui_label(locale, "menu.window.zoom", "Zoom"))
            .separator()
            .fullscreen()
    } else {
        b.maximize_with_text(ui_label(locale, "menu.window.maximize", "Maximize"))
    };
    let b = b
        .separator()
        .item(&item(
            NEXT_TAB_ID,
            "menu.window.showNextTab",
            "Show Next Tab",
            Some("Ctrl+Tab"),
        )?)
        .item(&item(
            PREV_TAB_ID,
            "menu.window.showPrevTab",
            "Show Previous Tab",
            Some("Ctrl+Shift+Tab"),
        )?)
        .item(&item(
            DETACH_TAB_ID,
            "menu.window.moveTabToNewWindow",
            "Move Tab to New Window",
            None,
        )?)
        .item(&item(
            MERGE_ID,
            "menu.window.mergeAllWindows",
            "Merge All Windows",
            None,
        )?)
        .separator();
    let bring = ui_label(locale, "menu.window.bringAllToFront", "Bring All to Front");
    let b = if mac {
        b.bring_all_to_front_with_text(bring)
    } else {
        b.item(&MenuItemBuilder::with_id(BRING_ALL_ID, bring).build(app)?)
    };
    b.separator().build()
}

/// Pure: the Window menu's list items as (menu id, text, checked). `&`
/// is doubled on every platform: muda reads a single `&` as a mnemonic
/// marker (and drops it) on Windows, GTK and macOS alike.
pub fn window_list_items(list: &[ListEntry]) -> Vec<(String, String, bool)> {
    list.iter()
        .map(|e| {
            (
                format!("{FOCUS_PREFIX}{}", e.label),
                e.title.replace('&', "&&"),
                e.checked,
            )
        })
        .collect()
}

/// Replaces the document list at the end of the Window menu.
pub fn set_window_list<R: Runtime>(app: &AppHandle<R>, list: &[ListEntry]) -> tauri::Result<()> {
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    let Some(MenuItemKind::Submenu(window)) = menu.get(WINDOW_MENU_ID) else {
        return Ok(());
    };
    for item in window.items()? {
        if item.id().as_ref().starts_with(FOCUS_PREFIX) {
            window.remove(&item)?;
        }
    }
    for (id, text, checked) in window_list_items(list) {
        let item = CheckMenuItemBuilder::with_id(id, text)
            .checked(checked)
            .build(app)?;
        window.append(&item)?;
    }
    Ok(())
}

/// Builds the application menu in the UI language stored in the prefs.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    build_localized(app, l10n::current_locale(app))
}

/// Builds the application menu with `tally_l10n` labels for `locale` (W5-6).
pub fn build_localized<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> tauri::Result<Menu<R>> {
    // The Window menu's list names untitled windows in the UI language.
    if let Some(windows) = app.try_state::<Windows>() {
        let untitled = ui_label(locale, "app.untitled", "Untitled");
        windows.with(|r| r.set_untitled(&untitled));
    }
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
    menu = menu.item(&window_menu(app, locale)?);
    let help = if cfg!(target_os = "macos") {
        // "About" lives in the app menu on macOS.
        submenu(app, locale, "Help", &HELP[..1])?
    } else {
        submenu(app, locale, "Help", HELP)?
    };
    menu = menu.item(&help);
    menu.build()
}

/// A Window-menu action the shell carries out on the front window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Show Next Tab.
    NextTab,
    /// Show Previous Tab.
    PrevTab,
    /// Move Tab to New Window (the front document).
    Detach,
    /// Merge All Windows (into the front window).
    Merge,
    /// Bring All to Front (off macOS).
    BringAll,
}

/// Where a menu selection goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// The guarded Quit (`closing::request_quit`).
    Quit,
    /// Show this document (its window and tab; the Window menu's list).
    Focus(String),
    /// A Window-menu action.
    Window(WindowAction),
    /// `menu-action { id }` to this document webview.
    Emit(String),
    /// No document window: open a blank one (File ▸ New).
    NewWindow,
    /// No document window: the Open dialog, then a window (File ▸ Open…).
    OpenDialog,
    /// No document window: File ▸ Open Read-Only….
    OpenReadOnlyDialog,
    /// ⌘W while a print window is the key window: close that window.
    ClosePrint(String),
    /// Nothing to do.
    Ignore,
}

/// Pure: routes menu item `id` given the front document and the key
/// window when that is a print window.
pub fn route(id: &str, front: Option<&str>, key_print: Option<&str>) -> Route {
    if id == crate::closing::QUIT_ID {
        return Route::Quit;
    }
    if let Some(label) = id.strip_prefix(FOCUS_PREFIX) {
        return Route::Focus(label.to_owned());
    }
    let action = match id {
        NEXT_TAB_ID => Some(WindowAction::NextTab),
        PREV_TAB_ID => Some(WindowAction::PrevTab),
        DETACH_TAB_ID => Some(WindowAction::Detach),
        MERGE_ID => Some(WindowAction::Merge),
        BRING_ALL_ID => Some(WindowAction::BringAll),
        _ => None,
    };
    if let Some(action) = action {
        return Route::Window(action);
    }
    if !custom_ids().contains(&id) {
        return Route::Ignore;
    }
    if let (Some(print), "file.close") = (key_print, id) {
        return Route::ClosePrint(print.to_owned());
    }
    match (front, id) {
        (Some(label), _) => Route::Emit(label.to_owned()),
        (None, "file.new") => Route::NewWindow,
        (None, "file.open") => Route::OpenDialog,
        (None, "file.openReadOnly") => Route::OpenReadOnlyDialog,
        (None, _) => Route::Ignore,
    }
}

/// Carries out a Window-menu action on the front window.
fn window_action<R: Runtime>(app: &AppHandle<R>, action: WindowAction) {
    let windows = app.state::<Windows>();
    let Some(win) = windows.with(|r| r.front_window()) else {
        return;
    };
    match action {
        WindowAction::NextTab | WindowAction::PrevTab => {
            let delta = if action == WindowAction::NextTab {
                1
            } else {
                -1
            };
            if let Some(label) = windows.with(|r| r.cycle(&win, delta)) {
                crate::windows::show_tab(app, &label);
            }
        }
        WindowAction::Detach => {
            let Some(label) = windows.with(|r| r.active_of(&win)) else {
                return;
            };
            // A new window: off the main thread (wry#583 on Windows).
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::windows::detach(&app, &label) {
                    eprintln!("tally: {}", e.message);
                }
            });
        }
        WindowAction::Merge => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { crate::windows::merge(&app, &win) });
        }
        WindowAction::BringAll => {
            // Oldest first, so the front window ends up in front again.
            for w in windows.with(|r| r.windows_mru()).iter().rev() {
                crate::windows::focus_window(app, w);
            }
        }
    }
}

/// Handles a menu selection ([`route`]).
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    let (front, key_print) = app
        .state::<Windows>()
        .with(|r| (r.front(), r.key_print().map(str::to_owned)));
    match route(id, front.as_deref(), key_print.as_deref()) {
        Route::Quit => crate::closing::request_quit(app),
        Route::Focus(label) => {
            crate::windows::reveal(app, &label);
            // The check item toggled itself; put the marks right.
            crate::windows::refresh_window_menu(app);
        }
        Route::Window(action) => window_action(app, action),
        Route::Emit(label) => emit_menu_action(app, &label, id),
        Route::NewWindow => crate::windows::spawn_doc_window(app, None),
        Route::OpenDialog => crate::dialogs::spawn_open(app, false),
        Route::OpenReadOnlyDialog => crate::dialogs::spawn_open(app, true),
        Route::ClosePrint(label) => {
            if let Some(w) = app.get_webview_window(&label) {
                if let Err(e) = w.destroy() {
                    eprintln!("tally: closing window {label}: {e}");
                }
            }
        }
        Route::Ignore => {}
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
    fn menu_actions_go_to_the_front_window_or_open_one() {
        assert_eq!(route("app-quit", Some("doc-1"), None), Route::Quit);
        assert_eq!(route("app-quit", None, None), Route::Quit);
        assert_eq!(
            route("file.close", Some("doc-2"), None),
            Route::Emit("doc-2".into())
        );
        assert_eq!(
            route("file.new", Some("doc-2"), None),
            Route::Emit("doc-2".into())
        );
        assert_eq!(route("file.new", None, None), Route::NewWindow);
        assert_eq!(route("file.open", None, None), Route::OpenDialog);
        assert_eq!(route("file.close", None, None), Route::Ignore);
        assert_eq!(route("format.bold", None, None), Route::Ignore);
        assert_eq!(route("not.a.command", Some("doc-1"), None), Route::Ignore);
        assert_eq!(
            route("window.focus.doc-3", Some("doc-1"), None),
            Route::Focus("doc-3".into())
        );
        // ⌘W in a print window closes that window, not its document window;
        // other items still go to the front document window.
        assert_eq!(
            route("file.close", Some("doc-1"), Some("tally-print-4")),
            Route::ClosePrint("tally-print-4".into())
        );
        assert_eq!(
            route("format.bold", Some("doc-1"), Some("tally-print-4")),
            Route::Emit("doc-1".into())
        );
        // The window list is the shell's own, never forwarded.
        assert!(!custom_ids().iter().any(|id| id.starts_with(FOCUS_PREFIX)));
    }

    #[test]
    fn window_menu_tab_items_are_handled_by_the_shell() {
        for (id, action) in [
            (NEXT_TAB_ID, WindowAction::NextTab),
            (PREV_TAB_ID, WindowAction::PrevTab),
            (DETACH_TAB_ID, WindowAction::Detach),
            (MERGE_ID, WindowAction::Merge),
            (BRING_ALL_ID, WindowAction::BringAll),
        ] {
            assert_eq!(
                route(id, Some("doc-1"), None),
                Route::Window(action),
                "{id}"
            );
            assert_eq!(route(id, None, None), Route::Window(action), "{id}");
            // Never forwarded as menu-action, and not an accidental command id clash.
            assert!(!custom_ids().contains(&id), "{id}");
        }
        assert_eq!(
            route("window.focus.doc-7", None, None),
            Route::Focus("doc-7".into())
        );
        assert_ne!(WINDOW_MENU_ID, tauri::menu::WINDOW_SUBMENU_ID);
    }

    #[test]
    fn window_list_items_focus_their_window() {
        let list = [
            ListEntry {
                label: "doc-1".into(),
                title: "Q&A.ods".into(),
                checked: false,
            },
            ListEntry {
                label: "doc-4".into(),
                title: "Untitled".into(),
                checked: true,
            },
        ];
        // `&` is escaped on every platform (a mnemonic marker for muda).
        assert_eq!(
            window_list_items(&list),
            [
                (
                    "window.focus.doc-1".to_owned(),
                    "Q&&A.ods".to_owned(),
                    false
                ),
                ("window.focus.doc-4".to_owned(), "Untitled".to_owned(), true),
            ]
        );
        for (id, _, _) in window_list_items(&list) {
            let label = id.strip_prefix(FOCUS_PREFIX).unwrap_or_default();
            assert!(crate::registry::is_doc_label(label));
        }
    }

    #[test]
    fn untitled_windows_are_listed_in_the_ui_language() {
        assert_eq!(ui_label(Locale::En, "app.untitled", "Untitled"), "Untitled");
        for locale in [Locale::De, Locale::Fr, Locale::Es] {
            assert_ne!(ui_label(locale, "app.untitled", "Untitled"), "Untitled");
        }
    }

    #[test]
    fn window_menu_labels_are_localised() {
        assert_eq!(ui_label(Locale::En, "menu.window.zoom", "Zoom"), "Zoom");
        for key in [
            "menu.window.minimize",
            "menu.window.zoom",
            "menu.window.maximize",
            "menu.window.bringAllToFront",
            "menu.window.showNextTab",
            "menu.window.showPrevTab",
            "menu.window.moveTabToNewWindow",
            "menu.window.mergeAllWindows",
        ] {
            assert!(tally_l10n::has_ui_key(key), "{key}");
            for locale in [Locale::De, Locale::Fr, Locale::Es] {
                assert_ne!(ui_label(locale, key, "?"), "?", "{key}");
            }
        }
        assert_eq!(
            ui_label(
                Locale::De,
                "menu.window.bringAllToFront",
                "Bring All to Front"
            ),
            "Alle nach vorne bringen"
        );
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
            "view.toolbar",
            "view.formattingBar",
            "view.formulaBar",
            "view.statusBar",
            "view.customizeToolbars",
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
            "edit.fillSheet",
            "sheet.selectAll",
            "sheet.deselectAll",
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
            "tools.removePrecedents",
            "tools.removeDependents",
            "tools.traceError",
            "tools.refreshTraces",
            "tools.autoRefreshTraces",
            "tools.detectiveFillMode",
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
        // Undo/Redo and the Window menu's tab items are built outside the tables.
        let mut accels = vec![
            "CmdOrCtrl+Z",
            "Shift+CmdOrCtrl+Z",
            "Ctrl+Tab",
            "Ctrl+Shift+Tab",
        ];
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
