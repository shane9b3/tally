//! The application menu, defined once ([`tree`]) and shown two ways.
//!
//! macOS and Linux: a native menu built from the tree ([`build_localized`]).
//! Native items (Cut/Copy/Paste/Select All, Minimize, Zoom…) are handled by
//! the OS; every other item emits `menu-action { id }` to the front
//! document's webview (the shown tab of the most recently focused window;
//! see [`route`]), where the id is a `CommandRegistry` command id. With no
//! document window (macOS keeps running without one) File ▸ New and Open
//! are handled here and open a window.
//!
//! Windows: no native menu. The document windows are undecorated
//! (`windows.rs`), and for such a window tao answers `WM_NCCALCSIZE` with
//! 0, making the whole window client area: the Win32 menu bar, which lives
//! in the non-client area, is attached (`SetMenu`) but never shown. So the
//! title bar strip of the document webviews draws the menu itself
//! (`packages/app/src/chrome/MenuBar.tsx`) from the tree's serialisable
//! [`model`] (`menu_model`, kept fresh by `menu-changed` from [`changed`])
//! and hands a chosen id to `menu_activate`, which validates it against
//! the current tree and runs the same [`dispatch`] as a native selection,
//! the calling webview standing in for the front document
//! ([`in_app_route`]). The OS items become webview commands or window
//! actions there ([`Native::in_app`]). Both commands refuse every call on
//! the platforms with a native menu, and Quit is never accepted from a
//! webview. Without a native menu there are no
//! native accelerators: the webview's keymap handles the shortcuts
//! (a hidden menu's accelerators would at best never fire while a webview
//! has the focus, at worst fire a second time).
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

use std::sync::{Mutex, PoisonError};

use tally_api::{ApiError, MenuNode, MenuNodeKind};
use tauri::menu::{
    CheckMenuItemBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, MenuItemKind, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};

use crate::commands::CmdResult;
use crate::events::{emit_menu_action, MENU_CHANGED};
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
    Item("insert.functionList", "Function List", None),
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

/// Edit ▸ Fill (AOO menubar.xml: Down, Right, Up, Left, Sheet…, Series…).
/// Fill Sheets… is enabled in the registry only with several sheets
/// selected.
const FILL: &[Entry] = &[
    Item("edit.fillDown", "Fill Down", Some("CmdOrCtrl+D")),
    Item("edit.fillRight", "Fill Right", Some("CmdOrCtrl+R")),
    Item("edit.fillUp", "Fill Up", None),
    Item("edit.fillLeft", "Fill Left", None),
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
    Item("format.loadStyles", "Load Styles…", None),
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
            Item("chart.dataTable", "Data Table…", None),
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
    Item("data.selectDatabaseRange", "Select Range…", None),
    Item("data.refreshRange", "Refresh Range", None),
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
    Item("view.zoomDialog", "Zoom…", None),
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
    Item("help.checkUpdates", "Check for Updates…", None),
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

/// An item the OS handles in a native menu (a predefined item).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Maximize,
    Fullscreen,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    BringAllToFront,
}

impl Native {
    /// The item's id in the menu model.
    pub fn id(self) -> &'static str {
        match self {
            Native::Cut => "native.cut",
            Native::Copy => "native.copy",
            Native::Paste => "native.paste",
            Native::SelectAll => "native.selectAll",
            Native::Minimize => "native.minimize",
            Native::Maximize => "native.maximize",
            Native::Fullscreen => "native.fullscreen",
            Native::Services => "native.services",
            Native::Hide => "native.hide",
            Native::HideOthers => "native.hideOthers",
            Native::ShowAll => "native.showAll",
            Native::BringAllToFront => "native.bringAllToFront",
        }
    }

    /// What the in-app menu bar does for the item, which has no OS menu to
    /// handle it: the clipboard items run the webview's own commands, the
    /// window items act on the caller's window. `None`: not offered there
    /// (the macOS-only items).
    pub fn in_app(self) -> Option<InApp> {
        match self {
            Native::Cut => Some(InApp::Command("edit.cut")),
            Native::Copy => Some(InApp::Command("edit.copy")),
            Native::Paste => Some(InApp::Command("edit.paste")),
            Native::SelectAll => Some(InApp::Command("edit.selectAll")),
            Native::Minimize => Some(InApp::Window(WindowAction::Minimize)),
            Native::Maximize => Some(InApp::Window(WindowAction::ToggleMaximize)),
            _ => None,
        }
    }
}

/// How the in-app menu bar carries out a [`Native`] item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InApp {
    /// `menu-action` with this command id to the calling webview.
    Command(&'static str),
    /// An action on the caller's window.
    Window(WindowAction),
}

/// What a [`Node`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A custom item (emits its id).
    Item,
    /// A check item (the Window menu's document list).
    Check(bool),
    /// A separator.
    Separator,
    /// A submenu.
    Submenu(Vec<Node>),
    /// A predefined item.
    Native(Native),
}

/// One node of the application menu: the single definition both the native
/// menu ([`build_localized`]) and the in-app menu bar's model ([`model`])
/// are made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Item id; submenus are `menu/<Title>/…` (the Window menu
    /// [`WINDOW_MENU_ID`]); empty for separators.
    pub id: String,
    /// Label in the UI language (plain text: `&` is not a mnemonic marker).
    pub label: String,
    /// Accelerator in Tauri's notation (`CmdOrCtrl+S`). For a predefined
    /// item it is the key the OS binds, listed for display only.
    pub accel: Option<&'static str>,
    pub kind: Kind,
}

impl Node {
    fn item(id: &str, label: String, accel: Option<&'static str>) -> Self {
        Node {
            id: id.to_owned(),
            label,
            accel,
            kind: Kind::Item,
        }
    }

    fn sep() -> Self {
        Node {
            id: String::new(),
            label: String::new(),
            accel: None,
            kind: Kind::Separator,
        }
    }

    fn native(native: Native, label: String, accel: Option<&'static str>) -> Self {
        Node {
            id: native.id().to_owned(),
            label,
            accel,
            kind: Kind::Native(native),
        }
    }

    fn sub(id: String, label: String, items: Vec<Node>) -> Self {
        Node {
            id,
            label,
            accel: None,
            kind: Kind::Submenu(items),
        }
    }
}

/// The nodes of a table; `path` is the id of the submenu they are in.
fn nodes(locale: Locale, path: &str, entries: &[Entry]) -> Vec<Node> {
    entries
        .iter()
        .map(|e| match e {
            Sep => Node::sep(),
            Item(id, label, accel) => Node::item(id, l10n::item_label(locale, id, label), *accel),
            Sub(title, inner) => table_menu(locale, path, title, inner),
        })
        .collect()
}

/// A submenu made of a table, titled `title` (English; the label is localised).
fn table_menu(locale: Locale, path: &str, title: &str, entries: &[Entry]) -> Node {
    let id = format!("{path}/{title}");
    let items = nodes(locale, &id, entries);
    Node::sub(id, l10n::submenu_label(locale, title), items)
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

/// The Edit menu: Undo, Redo, the OS's Cut/Copy/Paste, the paste items,
/// Select All, then the tail table.
fn edit_menu(locale: Locale) -> Node {
    let id = "menu/Edit".to_owned();
    let label = |cmd: &str, text: &str| l10n::item_label(locale, cmd, text);
    let mut items = vec![
        Node::item("edit.undo", label("edit.undo", "Undo"), Some("CmdOrCtrl+Z")),
        Node::item(
            "edit.redo",
            label("edit.redo", "Redo"),
            Some("Shift+CmdOrCtrl+Z"),
        ),
        Node::sep(),
        Node::native(Native::Cut, label("edit.cut", "Cut"), Some("CmdOrCtrl+X")),
        Node::native(
            Native::Copy,
            label("edit.copy", "Copy"),
            Some("CmdOrCtrl+C"),
        ),
        Node::native(
            Native::Paste,
            label("edit.paste", "Paste"),
            Some("CmdOrCtrl+V"),
        ),
    ];
    items.extend(nodes(locale, &id, EDIT_PASTE));
    items.push(Node::sep());
    items.push(Node::native(
        Native::SelectAll,
        label("edit.selectAll", "Select All"),
        Some("CmdOrCtrl+A"),
    ));
    items.push(Node::sep());
    items.extend(nodes(locale, &id, EDIT_TAIL));
    Node::sub(id, l10n::submenu_label(locale, "Edit"), items)
}

/// The macOS app menu.
fn app_menu(locale: Locale) -> Node {
    let plain = |native: Native, text: &str| Node::native(native, text.to_owned(), None);
    Node::sub(
        "menu/Tally".to_owned(),
        "Tally".to_owned(),
        vec![
            Node::item(
                "help.about",
                l10n::item_label(locale, "help.about", "About Tally"),
                None,
            ),
            Node::item(
                "help.checkUpdates",
                l10n::item_label(locale, "help.checkUpdates", "Check for Updates…"),
                None,
            ),
            Node::sep(),
            // The OS names these itself; the labels are for the model only.
            plain(Native::Services, "Services"),
            Node::sep(),
            plain(Native::Hide, "Hide Tally"),
            plain(Native::HideOthers, "Hide Others"),
            plain(Native::ShowAll, "Show All"),
            Node::sep(),
            // Not the predefined Quit (`terminate:` bypasses the unsaved-changes
            // guard): [`route`] sends it to `closing::request_quit`.
            Node::item(
                crate::closing::QUIT_ID,
                quit_label(locale),
                Some("CmdOrCtrl+Q"),
            ),
        ],
    )
}

/// The Window menu: Minimize, Zoom (Maximize off macOS), Enter Full Screen
/// (macOS), the tab items, Bring All to Front, then the open documents
/// (`list`) after the trailing separator.
fn window_menu(locale: Locale, mac: bool, list: &[ListEntry]) -> Node {
    let item = |id: &str, key: &str, text: &str, accel: Option<&'static str>| {
        Node::item(id, ui_label(locale, key, text), accel)
    };
    let mut items = vec![Node::native(
        Native::Minimize,
        ui_label(locale, "menu.window.minimize", "Minimize"),
        None,
    )];
    if mac {
        items.push(Node::native(
            Native::Maximize,
            ui_label(locale, "menu.window.zoom", "Zoom"),
            None,
        ));
        items.push(Node::sep());
        items.push(Node::native(
            Native::Fullscreen,
            "Enter Full Screen".to_owned(),
            None,
        ));
    } else {
        items.push(Node::native(
            Native::Maximize,
            ui_label(locale, "menu.window.maximize", "Maximize"),
            None,
        ));
    }
    items.extend([
        Node::sep(),
        item(
            NEXT_TAB_ID,
            "menu.window.showNextTab",
            "Show Next Tab",
            Some("Ctrl+Tab"),
        ),
        item(
            PREV_TAB_ID,
            "menu.window.showPrevTab",
            "Show Previous Tab",
            Some("Ctrl+Shift+Tab"),
        ),
        item(
            DETACH_TAB_ID,
            "menu.window.moveTabToNewWindow",
            "Move Tab to New Window",
            None,
        ),
        item(
            MERGE_ID,
            "menu.window.mergeAllWindows",
            "Merge All Windows",
            None,
        ),
        Node::sep(),
    ]);
    let bring = ui_label(locale, "menu.window.bringAllToFront", "Bring All to Front");
    items.push(if mac {
        Node::native(Native::BringAllToFront, bring, None)
    } else {
        Node::item(BRING_ALL_ID, bring, None)
    });
    items.push(Node::sep());
    items.extend(list.iter().map(|e| Node {
        id: format!("{FOCUS_PREFIX}{}", e.label),
        label: e.title.clone(),
        accel: None,
        kind: Kind::Check(e.checked),
    }));
    Node::sub(
        WINDOW_MENU_ID.to_owned(),
        l10n::submenu_label(locale, "Window"),
        items,
    )
}

/// Pure: the application menu (the menus of the bar, in order) in `locale`
/// for macOS (`mac`) or Windows/Linux, with the Window menu's document
/// `list`.
pub fn tree(locale: Locale, mac: bool, list: &[ListEntry]) -> Vec<Node> {
    let table = |title: &str, entries: &[Entry]| table_menu(locale, "menu", title, entries);
    let mut top = Vec::new();
    if mac {
        top.push(app_menu(locale));
    }
    top.extend([
        table("File", FILE),
        edit_menu(locale),
        table("Insert", INSERT),
        table("Format", FORMAT),
        table("Data", DATA),
        table("View", VIEW),
        table("Sheet", SHEET),
        table("Tools", TOOLS),
        window_menu(locale, mac, list),
        // "Check for Updates…" and "About" live in the app menu on macOS.
        table("Help", if mac { &HELP[..1] } else { HELP }),
    ]);
    top
}

/// Pure: an accelerator in Tauri's notation as the text a menu shows
/// (`Ctrl+Shift+S` on Windows/Linux, `⇧⌘S` on macOS).
pub fn accel_text(accel: &str, mac: bool) -> String {
    let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
    let mut key = "";
    for part in accel.split('+') {
        match part {
            "CmdOrCtrl" if mac => cmd = true,
            "CmdOrCtrl" | "Ctrl" => ctrl = true,
            "Alt" => alt = true,
            "Shift" => shift = true,
            other => key = other,
        }
    }
    let key = match key {
        "Comma" => ",",
        "Backslash" => "\\",
        "Equal" => "=",
        "Minus" => "-",
        "Slash" => "/",
        "Backquote" => "`",
        "Tab" if mac => "⇥",
        other => other,
    };
    let mods: [(bool, &str); 4] = if mac {
        [(ctrl, "⌃"), (alt, "⌥"), (shift, "⇧"), (cmd, "⌘")]
    } else {
        [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+"), (cmd, "")]
    };
    let mut out: String = mods.iter().filter(|m| m.0).map(|m| m.1).collect();
    out.push_str(key);
    out
}

/// Pure: the serialisable model of a [`tree`] (`menu_model`, `menu-changed`).
pub fn model(tree: &[Node], mac: bool) -> Vec<MenuNode> {
    tree.iter()
        .map(|n| {
            let (kind, enabled, checked, items) = match &n.kind {
                // Quit (the macOS app menu) is never accepted from a window.
                Kind::Item => (
                    MenuNodeKind::Item,
                    n.id != crate::closing::QUIT_ID,
                    None,
                    Vec::new(),
                ),
                Kind::Check(c) => (MenuNodeKind::Check, true, Some(*c), Vec::new()),
                Kind::Separator => (MenuNodeKind::Separator, false, None, Vec::new()),
                Kind::Submenu(inner) => (MenuNodeKind::Submenu, true, None, model(inner, mac)),
                Kind::Native(native) => (
                    MenuNodeKind::Item,
                    native.in_app().is_some(),
                    None,
                    Vec::new(),
                ),
            };
            MenuNode {
                kind,
                id: n.id.clone(),
                label: n.label.clone(),
                accelerator: n.accel.map(|a| accel_text(a, mac)),
                enabled,
                checked,
                items,
            }
        })
        .collect()
}

/// Pure: the node with `id` that can be chosen (not a submenu or separator).
pub fn find<'a>(tree: &'a [Node], id: &str) -> Option<&'a Node> {
    tree.iter().find_map(|n| match &n.kind {
        Kind::Submenu(inner) => find(inner, id),
        Kind::Separator => None,
        _ => (n.id == id).then_some(n),
    })
}

/// Whether this platform draws the menu in the window (the in-app menu bar
/// of the custom title bar, Windows) instead of installing a native menu:
/// an undecorated Win32 window has no non-client area for a menu bar.
pub fn in_app_bar() -> bool {
    crate::windows::chrome_for(std::env::consts::OS).custom_title_bar
}

/// The model last sent with `menu-changed` ([`changed`]).
static LAST_MODEL: Mutex<Option<Vec<MenuNode>>> = Mutex::new(None);

/// The UI language of the menu ([`set_locale`]).
static LOCALE: Mutex<Option<Locale>> = Mutex::new(None);

/// The menu's UI language: what [`set_locale`] stored, else the prefs'.
fn locale<R: Runtime>(app: &AppHandle<R>) -> Locale {
    let cached = *LOCALE.lock().unwrap_or_else(PoisonError::into_inner);
    cached.unwrap_or_else(|| l10n::current_locale(app))
}

/// Takes `locale` as the menu's language: the Window menu's list names
/// untitled windows in it.
pub fn set_locale<R: Runtime>(app: &AppHandle<R>, locale: Locale) {
    *LOCALE.lock().unwrap_or_else(PoisonError::into_inner) = Some(locale);
    if let Some(windows) = app.try_state::<Windows>() {
        let untitled = ui_label(locale, "app.untitled", "Untitled");
        windows.with(|r| r.set_untitled(&untitled));
    }
}

/// The menu as it is now: the UI language and the open documents.
fn current_tree<R: Runtime>(app: &AppHandle<R>) -> Vec<Node> {
    let list = app
        .try_state::<Windows>()
        .map(|w| w.with(|r| r.window_list()))
        .unwrap_or_default();
    tree(locale(app), cfg!(target_os = "macos"), &list)
}

fn native_submenu<R: Runtime>(app: &AppHandle<R>, node: &Node) -> tauri::Result<Submenu<R>> {
    let mut b = if node.id == WINDOW_MENU_ID {
        SubmenuBuilder::with_id(app, WINDOW_MENU_ID, &node.label)
    } else {
        SubmenuBuilder::new(app, &node.label)
    };
    let Kind::Submenu(items) = &node.kind else {
        return b.build();
    };
    for n in items {
        b = match &n.kind {
            Kind::Separator => b.separator(),
            Kind::Item => {
                let mut item = MenuItemBuilder::with_id(n.id.as_str(), &n.label);
                if let Some(a) = n.accel {
                    item = item.accelerator(a);
                }
                b.item(&item.build(app)?)
            }
            Kind::Check(checked) => b.item(
                &CheckMenuItemBuilder::with_id(n.id.as_str(), native_text(&n.label))
                    .checked(*checked)
                    .build(app)?,
            ),
            Kind::Submenu(_) => b.item(&native_submenu(app, n)?),
            Kind::Native(native) => match native {
                Native::Cut => b.cut(),
                Native::Copy => b.copy(),
                Native::Paste => b.paste(),
                Native::SelectAll => b.select_all(),
                Native::Minimize => b.minimize_with_text(&n.label),
                Native::Maximize => b.maximize_with_text(&n.label),
                Native::Fullscreen => b.fullscreen(),
                Native::Services => b.services(),
                Native::Hide => b.hide(),
                Native::HideOthers => b.hide_others(),
                Native::ShowAll => b.show_all(),
                Native::BringAllToFront => b.bring_all_to_front_with_text(&n.label),
            },
        };
    }
    b.build()
}

/// Pure: free text (a document title) as a native menu item's text. `&` is
/// doubled on every platform: muda reads a single `&` as a mnemonic marker
/// (and drops it) on Windows, GTK and macOS alike.
fn native_text(text: &str) -> String {
    text.replace('&', "&&")
}

/// Pure: the Window menu's list items as (menu id, native text, checked).
pub fn window_list_items(list: &[ListEntry]) -> Vec<(String, String, bool)> {
    list.iter()
        .map(|e| {
            (
                format!("{FOCUS_PREFIX}{}", e.label),
                native_text(&e.title),
                e.checked,
            )
        })
        .collect()
}

/// Replaces the document list at the end of the native Window menu.
fn set_window_list<R: Runtime>(app: &AppHandle<R>, list: &[ListEntry]) -> tauri::Result<()> {
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

/// The one place the menu changes after it was made (the open documents,
/// the front document, the language): updates the native Window menu's
/// document list and sends `menu-changed` (the new model) to the document
/// webviews that draw the in-app bar. Call on the main thread
/// (`windows::refresh_window_menu`).
pub fn changed<R: Runtime>(app: &AppHandle<R>) {
    let list = app.state::<Windows>().with(|r| r.window_list());
    if let Err(e) = set_window_list(app, &list) {
        eprintln!("tally: window menu: {e}");
    }
    if !in_app_bar() {
        return;
    }
    let mac = cfg!(target_os = "macos");
    let model = model(&tree(locale(app), mac, &list), mac);
    // Focus, title and tab changes call this often; most leave the menu as
    // it was. A webview that starts later asks `menu_model` itself.
    {
        let mut last = LAST_MODEL.lock().unwrap_or_else(PoisonError::into_inner);
        if last.as_ref() == Some(&model) {
            return;
        }
        *last = Some(model.clone());
    }
    for entry in &list {
        if let Err(e) = app.emit_to(entry.label.as_str(), MENU_CHANGED, &model) {
            eprintln!("tally: emitting {MENU_CHANGED} to {}: {e}", entry.label);
        }
    }
}

/// Builds the native application menu in the UI language stored in the prefs.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    build_localized(app, l10n::current_locale(app))
}

/// Builds the native application menu with `tally_l10n` labels for
/// `locale` (W5-6). The document list of the Window menu is filled in by
/// [`changed`].
pub fn build_localized<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> tauri::Result<Menu<R>> {
    set_locale(app, locale);
    let mut menu = MenuBuilder::new(app);
    for node in tree(locale, cfg!(target_os = "macos"), &[]) {
        menu = menu.item(&native_submenu(app, &node)?);
    }
    menu.build()
}

/// Puts the menu in `locale`: a fresh native menu, or with the in-app bar
/// (no native menu) only the language. The caller then refreshes the
/// Window list (`windows::refresh_window_menu`), which also tells the
/// in-app bars.
pub fn install<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> tauri::Result<()> {
    if in_app_bar() {
        set_locale(app, locale);
    } else {
        app.set_menu(build_localized(app, locale)?)?;
    }
    Ok(())
}

/// A Window-menu action the shell carries out on a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Show Next Tab.
    NextTab,
    /// Show Previous Tab.
    PrevTab,
    /// Move Tab to New Window (the window's shown document).
    Detach,
    /// Merge All Windows (into the window).
    Merge,
    /// Bring All to Front (off macOS).
    BringAll,
    /// Minimize (the in-app bar; the OS handles the native item).
    Minimize,
    /// Maximize or restore (the in-app bar; the OS handles the native item).
    ToggleMaximize,
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

/// Pure: what the in-app menu bar of document webview `caller` does for
/// item `id`: the route and the id it carries (the command id of
/// `menu-action`). The item must be in the menu as it is now (`tree`) and
/// offered in the window; everything else is refused (`None`). The caller
/// stands where the native path has the front document, so a webview only
/// ever sends a command to itself.
pub fn in_app_route(tree: &[Node], id: &str, caller: &str) -> Option<(Route, String)> {
    let node = find(tree, id)?;
    if let Kind::Native(native) = node.kind {
        return match native.in_app()? {
            InApp::Command(command) => Some((Route::Emit(caller.to_owned()), command.to_owned())),
            InApp::Window(action) => Some((Route::Window(action), id.to_owned())),
        };
    }
    match route(id, Some(caller), None) {
        // Quit is the app menu's (macOS, native only): never from a webview.
        Route::Ignore | Route::Quit => None,
        route => Some((route, id.to_owned())),
    }
}

/// Carries out a Window-menu action on window `win`.
fn window_action<R: Runtime>(app: &AppHandle<R>, action: WindowAction, win: String) {
    let windows = app.state::<Windows>();
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
        WindowAction::Minimize | WindowAction::ToggleMaximize => {
            let Some(window) = app.get_window(&win) else {
                return;
            };
            let result = if action == WindowAction::Minimize {
                window.minimize()
            } else if window.is_maximized().unwrap_or(false) {
                window.unmaximize()
            } else {
                window.maximize()
            };
            if let Err(e) = result {
                eprintln!("tally: window {win}: {e}");
            }
        }
    }
}

/// Carries out a routed menu selection: the one handler of the native
/// menu ([`on_event`]) and the in-app menu bar ([`menu_activate`]). `id` is
/// the id a [`Route::Emit`] sends; `win` the window a [`Route::Window`]
/// acts on.
fn dispatch<R: Runtime>(app: &AppHandle<R>, route: Route, id: &str, win: Option<String>) {
    match route {
        Route::Quit => crate::closing::request_quit(app),
        Route::Focus(label) => {
            crate::windows::reveal(app, &label);
            // The check item toggled itself; put the marks right.
            crate::windows::refresh_window_menu(app);
        }
        Route::Window(action) => {
            if let Some(win) = win {
                window_action(app, action, win);
            }
        }
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

/// Handles a native menu selection ([`route`]): the front document and
/// the front window.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    let (front, win, key_print) = app.state::<Windows>().with(|r| {
        (
            r.front(),
            r.front_window(),
            r.key_print().map(str::to_owned),
        )
    });
    let route = route(id, front.as_deref(), key_print.as_deref());
    dispatch(app, route, id, win);
}

fn refused() -> ApiError {
    ApiError::invalid_argument("this menu item is not available")
}

/// The in-app bar's commands exist only where the bar does (Windows): with
/// a native menu (macOS, Linux) a webview has no business reading the menu
/// or choosing from it.
fn require_in_app_bar(in_app_bar: bool) -> CmdResult<()> {
    if in_app_bar {
        Ok(())
    } else {
        Err(refused())
    }
}

/// `menu_model()`: the application menu as it is now, for the in-app menu
/// bar (document webviews only; refused where the menu is native).
#[tauri::command]
pub async fn menu_model(app: AppHandle, webview: Webview) -> CmdResult<Vec<MenuNode>> {
    require_in_app_bar(in_app_bar())?;
    crate::windows::require_doc_window(webview.label())?;
    Ok(model(&current_tree(&app), cfg!(target_os = "macos")))
}

/// `menu_activate(id)`: an item chosen in the in-app menu bar of the
/// calling document webview. The id must be an item of the current menu;
/// the selection then takes the native menu's path ([`dispatch`]) with the
/// caller as the front document and the caller's window as the front
/// window. Refused where the menu is native (macOS, Linux).
#[tauri::command]
pub async fn menu_activate(app: AppHandle, webview: Webview, id: String) -> CmdResult<()> {
    require_in_app_bar(in_app_bar())?;
    crate::limits::text_max("the menu item", &id, crate::limits::MAX_NAME_CHARS)?;
    let caller = webview.label().to_owned();
    crate::windows::require_doc_window(&caller)?;
    let win = app
        .state::<Windows>()
        .with(|r| r.window_of_tab(&caller))
        .ok_or_else(refused)?;
    let (route, emit) = in_app_route(&current_tree(&app), &id, &caller).ok_or_else(refused)?;
    dispatch(&app, route, &emit, Some(win));
    Ok(())
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
        // route() never produces the in-app bar's window actions from an id.
        for id in ["native.minimize", "native.maximize", "native.cut"] {
            assert_eq!(route(id, Some("doc-1"), None), Route::Ignore, "{id}");
        }
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
            "help.checkUpdates",
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
            "insert.functionList",
            "insert.image",
            "chart.edit",
            "chart.dataTable",
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
            "view.zoomDialog",
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

    fn list() -> Vec<ListEntry> {
        vec![
            ListEntry {
                label: "doc-1".into(),
                title: "Q&A.ods".into(),
                checked: true,
            },
            ListEntry {
                label: "doc-2".into(),
                title: "Untitled".into(),
                checked: false,
            },
        ]
    }

    /// Every node of a tree, depth first.
    fn flat(tree: &[Node]) -> Vec<&Node> {
        let mut out = Vec::new();
        for n in tree {
            out.push(n);
            if let Kind::Submenu(inner) = &n.kind {
                out.extend(flat(inner));
            }
        }
        out
    }

    fn flat_model(model: &[MenuNode]) -> Vec<&MenuNode> {
        let mut out = Vec::new();
        for n in model {
            out.push(n);
            out.extend(flat_model(&n.items));
        }
        out
    }

    #[test]
    fn ids_and_accelerators_are_unique() {
        for mac in [false, true] {
            let tree = tree(Locale::En, mac, &list());
            let nodes = flat(&tree);
            let mut ids: Vec<&str> = nodes
                .iter()
                .filter(|n| n.kind != Kind::Separator)
                .map(|n| n.id.as_str())
                .collect();
            assert!(ids.iter().all(|id| !id.is_empty()));
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n, "mac={mac}");
            let mut accels: Vec<&str> = nodes.iter().filter_map(|n| n.accel).collect();
            let n = accels.len();
            assert!(n > 40);
            accels.sort_unstable();
            accels.dedup();
            assert_eq!(accels.len(), n, "mac={mac}");
        }
    }

    #[test]
    fn the_tree_holds_every_menu_item() {
        for mac in [false, true] {
            let tree = tree(Locale::En, mac, &[]);
            let ids: Vec<&str> = flat(&tree).iter().map(|n| n.id.as_str()).collect();
            for id in custom_ids() {
                assert!(ids.contains(&id), "{id} (mac={mac})");
            }
            for id in [NEXT_TAB_ID, PREV_TAB_ID, DETACH_TAB_ID, MERGE_ID] {
                assert!(ids.contains(&id), "{id}");
            }
            assert_eq!(ids.contains(&BRING_ALL_ID), !mac);
            assert_eq!(ids.contains(&crate::closing::QUIT_ID), mac);
            let titles: Vec<&str> = tree.iter().map(|n| n.label.as_str()).collect();
            let all = [
                "Tally", "File", "Edit", "Insert", "Format", "Data", "View", "Sheet", "Tools",
                "Window", "Help",
            ];
            assert_eq!(titles, if mac { &all[..] } else { &all[1..] });
        }
        // The menus follow the language.
        let de = tree(Locale::De, false, &[]);
        assert_eq!(de[0].label, "Datei");
        assert_eq!(
            find(&de, "edit.undo").map(|n| n.label.as_str()),
            Some("Rückgängig")
        );
        assert_eq!(
            find(&de, "native.copy").map(|n| n.label.as_str()),
            Some("Kopieren")
        );
    }

    /// The native menu is built from the tree node by node and the model is
    /// the tree's image: every native item has a model entry and vice versa.
    #[test]
    fn the_model_mirrors_the_tree() {
        for mac in [false, true] {
            let tree = tree(Locale::En, mac, &list());
            let model = model(&tree, mac);
            let (nodes, entries) = (flat(&tree), flat_model(&model));
            assert_eq!(nodes.len(), entries.len());
            for (n, m) in nodes.iter().zip(&entries) {
                assert_eq!((&n.id, &n.label), (&m.id, &m.label));
                let kind = match &n.kind {
                    Kind::Item | Kind::Native(_) => MenuNodeKind::Item,
                    Kind::Check(_) => MenuNodeKind::Check,
                    Kind::Separator => MenuNodeKind::Separator,
                    Kind::Submenu(_) => MenuNodeKind::Submenu,
                };
                assert_eq!(m.kind, kind, "{}", n.id);
                assert_eq!(m.accelerator.is_some(), n.accel.is_some(), "{}", n.id);
                assert_eq!(m.checked.is_some(), kind == MenuNodeKind::Check);
                assert_eq!(
                    m.items.is_empty(),
                    kind != MenuNodeKind::Submenu,
                    "{}",
                    n.id
                );
            }
        }
        let model = model(&tree(Locale::En, false, &list()), false);
        let entries = flat_model(&model);
        let get = |id: &str| entries.iter().find(|m| m.id == id).copied();
        assert_eq!(
            get("file.saveAs").and_then(|m| m.accelerator.as_deref()),
            Some("Ctrl+Shift+S")
        );
        assert_eq!(
            get("native.copy").and_then(|m| m.accelerator.as_deref()),
            Some("Ctrl+C")
        );
        // The document list: plain titles (no native `&&`), the front one checked.
        let doc = get("window.focus.doc-1");
        assert_eq!(doc.map(|m| m.label.as_str()), Some("Q&A.ods"));
        assert_eq!(doc.and_then(|m| m.checked), Some(true));
        assert_eq!(
            get("window.focus.doc-2").and_then(|m| m.checked),
            Some(false)
        );
        assert!(entries
            .iter()
            .all(|m| m.enabled || m.kind == MenuNodeKind::Separator));
        // The macOS-only items are not offered in a window.
        let mac_model = super::model(&tree(Locale::En, true, &[]), true);
        let off: Vec<&str> = flat_model(&mac_model)
            .iter()
            .filter(|m| !m.enabled && m.kind != MenuNodeKind::Separator)
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            off,
            [
                "native.services",
                "native.hide",
                "native.hideOthers",
                "native.showAll",
                "app-quit",
                "native.fullscreen",
                "native.bringAllToFront"
            ]
        );
    }

    #[test]
    fn accelerators_are_shown_in_the_platform_notation() {
        for (accel, win, mac) in [
            ("CmdOrCtrl+S", "Ctrl+S", "⌘S"),
            ("Shift+CmdOrCtrl+S", "Ctrl+Shift+S", "⇧⌘S"),
            ("Alt+CmdOrCtrl+K", "Ctrl+Alt+K", "⌥⌘K"),
            ("CmdOrCtrl+Comma", "Ctrl+,", "⌘,"),
            ("CmdOrCtrl+Backslash", "Ctrl+\\", "⌘\\"),
            ("Ctrl+Backquote", "Ctrl+`", "⌃`"),
            ("Ctrl+Shift+Tab", "Ctrl+Shift+Tab", "⌃⇧⇥"),
            ("Shift+F2", "Shift+F2", "⇧F2"),
            ("F12", "F12", "F12"),
        ] {
            assert_eq!(accel_text(accel, false), win, "{accel}");
            assert_eq!(accel_text(accel, true), mac, "{accel}");
        }
    }

    #[test]
    fn the_in_app_bar_takes_the_native_path_for_its_own_document() {
        let tree = tree(Locale::En, false, &list());
        let emit = |id: &str| Some((Route::Emit("doc-2".into()), id.to_owned()));
        assert_eq!(in_app_route(&tree, "file.save", "doc-2"), emit("file.save"));
        // With a document there is always a front one: New goes to the caller.
        assert_eq!(in_app_route(&tree, "file.new", "doc-2"), emit("file.new"));
        // The OS's clipboard items become the webview's own commands.
        for (id, command) in [
            ("native.cut", "edit.cut"),
            ("native.copy", "edit.copy"),
            ("native.paste", "edit.paste"),
            ("native.selectAll", "edit.selectAll"),
        ] {
            assert_eq!(in_app_route(&tree, id, "doc-2"), emit(command), "{id}");
        }
        for (id, action) in [
            ("native.minimize", WindowAction::Minimize),
            ("native.maximize", WindowAction::ToggleMaximize),
            (NEXT_TAB_ID, WindowAction::NextTab),
            (MERGE_ID, WindowAction::Merge),
            (BRING_ALL_ID, WindowAction::BringAll),
        ] {
            assert_eq!(
                in_app_route(&tree, id, "doc-2"),
                Some((Route::Window(action), id.to_owned())),
                "{id}"
            );
        }
        assert_eq!(
            in_app_route(&tree, "window.focus.doc-1", "doc-2"),
            Some((
                Route::Focus("doc-1".into()),
                "window.focus.doc-1".to_owned()
            ))
        );
        // Every item the model offers is accepted.
        let model = model(&tree, false);
        for m in flat_model(&model) {
            let offered = m.enabled && m.kind != MenuNodeKind::Submenu;
            assert_eq!(
                in_app_route(&tree, &m.id, "doc-2").is_some(),
                offered,
                "{}",
                m.id
            );
        }
    }

    #[test]
    fn the_in_app_bar_refuses_what_is_not_in_the_menu() {
        let tree = tree(Locale::En, false, &list());
        for id in [
            "",
            "not.a.command",
            // A command of the registry that is no menu item.
            "palette.toggle",
            "edit.cut",
            // Submenus and the bar's menus are not items.
            "menu/File",
            "menu/File/Export",
            WINDOW_MENU_ID,
            // A document that is not open, the macOS-only items, Quit.
            "window.focus.doc-9",
            "native.services",
            "native.fullscreen",
            crate::closing::QUIT_ID,
            "os-open:/etc/passwd",
        ] {
            assert_eq!(in_app_route(&tree, id, "doc-1"), None, "{id}");
        }
        // With a native menu (macOS, Linux) the commands refuse everything.
        assert!(require_in_app_bar(true).is_ok());
        assert_eq!(
            require_in_app_bar(false).map_err(|e| e.code),
            Err(ApiError::INVALID_ARGUMENT.to_owned())
        );
        // And the macOS tree itself never yields Quit or the app-menu items:
        // only what a window may do.
        let mac_tree = super::tree(Locale::En, true, &list());
        assert!(find(&mac_tree, crate::closing::QUIT_ID).is_some());
        for id in [
            crate::closing::QUIT_ID,
            "native.services",
            "native.hide",
            "native.hideOthers",
            "native.showAll",
            "native.fullscreen",
            "native.bringAllToFront",
        ] {
            assert_eq!(in_app_route(&mac_tree, id, "doc-1"), None, "{id}");
        }
        let mac_model = super::model(&mac_tree, true);
        for m in flat_model(&mac_model) {
            let offered = m.enabled && m.kind != MenuNodeKind::Submenu;
            let route = in_app_route(&mac_tree, &m.id, "doc-1");
            assert_eq!(route.is_some(), offered, "{}", m.id);
            assert!(!matches!(route, Some((Route::Quit, _))), "{}", m.id);
        }
        // The macOS tree lists Hide, Services…; a window never gets them.
        let mac = super::tree(Locale::En, true, &list());
        assert_eq!(in_app_route(&mac, "native.hide", "doc-1"), None);
    }

    /// The browser build's mock menu (`packages/app/src/dev/mockMenu.ts`,
    /// e2e) is this model, not a copy kept by hand: the Windows/Linux menu
    /// in English with two open documents. Regenerate with
    /// `TALLY_WRITE_MENU_FIXTURE=1 cargo test -p tally-desktop mock_menu`.
    #[test]
    fn the_mock_menu_fixture_is_the_real_model() {
        let list = [
            ListEntry {
                label: "doc-1".into(),
                title: "Untitled 1".into(),
                checked: true,
            },
            ListEntry {
                label: "doc-2".into(),
                title: "Budget & Plan.ods".into(),
                checked: false,
            },
        ];
        let model = model(&tree(Locale::En, false, &list), false);
        let mut json = serde_json::to_string_pretty(&model).unwrap_or_default();
        json.push('\n');
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/app/src/dev/menuModel.json");
        if std::env::var_os("TALLY_WRITE_MENU_FIXTURE").is_some() {
            assert!(std::fs::write(&path, &json).is_ok());
        }
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            on_disk == json,
            "packages/app/src/dev/menuModel.json is stale: run TALLY_WRITE_MENU_FIXTURE=1 cargo test -p tally-desktop mock_menu"
        );
    }
}
