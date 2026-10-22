//! Book-keeping of the documents and the windows that show them: every open
//! document is a child webview `doc-<n>` shown as a tab of a native window
//! `win-<n>` (a window shows one tab at a time; every webview lists all open
//! documents in its sidebar). It tracks which tab shows which document, each tab's
//! unsaved-changes state, each window's tab order and active tab, window
//! focus order, the question sequences (Quit, closing a window with several
//! tabs, Close Other Tabs), the print windows each document owns, and the
//! Window menu's document list.
//!
//! Everything here is plain data with no Tauri types, so the decisions are
//! unit-tested; `closing.rs`, `windows.rs` and `menu.rs` act on the results.
//! The managed [`Windows`] wraps it in a mutex that is a leaf lock: it is
//! held only for these in-memory updates and never across an engine call, a
//! dialog, a window, webview or menu call or an emit, so the main thread
//! (window and exit events) never waits for the engine or for itself.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde::Serialize;
use tally_api::{DocId, WorkbookInfo};

/// Label prefix of document webviews (`doc-1`, `doc-2`, …).
pub const DOC_PREFIX: &str = "doc-";
/// Label prefix of the native windows that hold them (`win-1`, …).
pub const WIN_PREFIX: &str = "win-";
/// Label prefix of print windows (`print.rs`).
pub const PRINT_PREFIX: &str = "tally-print-";

fn numbered(label: &str, prefix: &str) -> bool {
    label
        .strip_prefix(prefix)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// A title as stored and shown (tabs, window title, Window menu): control
/// characters (C0, C1) and bidi embedding/override/isolate characters
/// removed, so a document name cannot reorder or hide what is around it.
pub fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| {
            !c.is_control()
                && !('\u{202A}'..='\u{202E}').contains(c)
                && !('\u{2066}'..='\u{2069}').contains(c)
        })
        .collect()
}

/// Whether `label` names a document webview.
pub fn is_doc_label(label: &str) -> bool {
    numbered(label, DOC_PREFIX)
}

/// Whether `label` names a document window (the native window of tabs).
pub fn is_win_label(label: &str) -> bool {
    numbered(label, WIN_PREFIX)
}

/// Most documents open at once ([`Registry::create`] refuses more); each
/// is a webview (and its web content process).
pub const MAX_DOCS: usize = 100;

/// One document: a child webview shown as a tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocTab {
    /// Creation order.
    pub seq: u32,
    /// The native window it is a tab of.
    pub window: String,
    /// The document it shows, once known.
    pub doc: Option<DocId>,
    /// Unsaved changes (the webview's `set_document_edited`).
    pub edited: bool,
    /// The user answered Save / Don't Save: the next close or exit goes through.
    pub confirmed: bool,
    /// The document's title (tab, window title, Window menu).
    pub title: String,
    /// Tauri has built the webview (until then an exit must not end the app).
    pub built: bool,
}

/// One native window and its tabs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocWindow {
    /// Creation order (Window menu order).
    pub seq: u32,
    /// Its tabs, in tab order.
    pub tabs: Vec<String>,
    /// The tab shown (once one is built).
    pub active: Option<String>,
    /// Tauri has built the window.
    pub built: bool,
}

/// The "Open documents in" setting (Tools ▸ Options; prefs `openDocumentsIn`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenMode {
    /// New documents become tabs of the window they were opened from.
    #[default]
    Tabs,
    /// Every document gets its own window.
    Windows,
}

/// Where a newly created or opened document is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In the calling tab, replacing its untouched untitled document.
    Here,
    /// In a new tab of this window.
    NewTab(String),
    /// In a new window.
    NewWindow,
}

/// Where [`Registry::create`] puts a new document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A new window (allocated with the tab).
    NewWindow,
    /// A new tab of this window.
    Window(String),
}

/// A document registered by [`Registry::create`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    /// Its webview label.
    pub doc: String,
    /// Its window.
    pub window: String,
    /// The window is new too (build it first).
    pub new_window: bool,
}

/// How a document's webview is asked about its unsaved changes
/// (`close-requested { kind }`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseKind {
    /// Closing this one document (⌘W, its tab's ×, the red button of a
    /// one-tab window): Save / Don't Save closes it at once.
    Window,
    /// One step of a Quit: answers are applied when the quit completes.
    Quit,
    /// One step of closing several tabs (the red button of a window with
    /// several tabs, Close Other Tabs): like a quit step.
    Tabs,
}

/// What a sequence of questions does once every edited tab has answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goal {
    /// Quit the app.
    Exit,
    /// Close this window with all its tabs.
    CloseWindow(String),
    /// Close these tabs.
    CloseTabs(Vec<String>),
}

/// The next step of a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Ask this tab (bring its window to the front and show the tab first).
    Ask(String, CloseKind),
    /// Nothing is left to ask: carry out the goal.
    Done(Goal),
}

/// What the red close button of a window does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowClose {
    /// Nothing to ask: let it close.
    Proceed,
    /// Hold it and run this step (a one-tab window asks its tab with
    /// [`CloseKind::Window`]; with several tabs a sequence starts).
    Hold(Step),
    /// Another sequence is asking: hold the close and ask that tab again
    /// ([`Registry::reask`]; also recovers a question that got lost).
    Busy,
}

/// What closing a tab (its ×, ⌘W, `tab_close`) does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabClose {
    /// Ask it first (show it, then `close-requested { kind: "window" }`).
    Ask,
    /// The last tab of its window: close the window.
    Window(String),
    /// Close the tab now.
    Now,
}

/// What an exit request should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitAction {
    /// Let it through.
    Proceed,
    /// Prevent it and keep running (macOS without windows, or a window is
    /// still being created).
    StayAlive,
    /// Prevent it unless the quit sequence finds nothing to ask.
    Quit,
}

/// Pure: an exit request with `code` (`None` for one the user or the last
/// window closing started; our own `exit(n)` and restarts carry a code).
/// A document or window still being built (`pending`) keeps the app alive
/// (the runtime asks to exit when its last *built* window closes). With no
/// document left, macOS keeps running (`keep_alive`) and the other
/// platforms exit; with documents, it becomes a guarded quit.
pub fn exit_action(code: Option<i32>, keep_alive: bool, docs: usize, pending: usize) -> ExitAction {
    match code {
        Some(_) => ExitAction::Proceed,
        None if pending > 0 => ExitAction::StayAlive,
        None if docs == 0 && keep_alive => ExitAction::StayAlive,
        None if docs == 0 => ExitAction::Proceed,
        None => ExitAction::Quit,
    }
}

/// Pure: where a document goes. `reuse`: the calling tab asked for its
/// untouched blank workbook to be replaced and the shell agrees (the tab is
/// not edited); otherwise the setting decides.
pub fn placement_for(mode: OpenMode, reuse: bool, window: &str) -> Placement {
    match (reuse, mode) {
        (true, _) => Placement::Here,
        (false, OpenMode::Tabs) => Placement::NewTab(window.to_owned()),
        (false, OpenMode::Windows) => Placement::NewWindow,
    }
}

/// What the shell does after tabs went away (a tab closed or a window destroyed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gone {
    /// Close these documents in the engine (and drop their autosave snapshots).
    pub close_docs: Vec<DocId>,
    /// Print windows the closed tabs owned (destroy them).
    pub prints: Vec<String>,
    /// A sequence was asking one of them: continue with this step.
    pub step: Option<Step>,
    /// For a closed tab: the tab its window shows now.
    pub activate: Option<String>,
}

/// What the final exit does with each document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitPlan {
    /// Edited and unanswered: write a last autosave snapshot.
    pub snapshot: Vec<DocId>,
    /// Clean or answered: remove this session's autosave snapshot.
    pub discard: Vec<DocId>,
}

/// One row of the Window menu's document list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    /// The document's webview label.
    pub label: String,
    pub title: String,
    pub checked: bool,
}

/// One open document of `tabs-changed` / `window_init`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    /// The document's webview label.
    pub label: String,
    /// The window it is in.
    pub window: String,
    /// Its title (`""`: untitled; the UI names it).
    pub title: String,
    /// Unsaved changes.
    pub edited: bool,
    /// Its file (the shell fills it in from its own record of paths).
    pub path: Option<String>,
    /// The document (used by the shell to look up the path).
    #[serde(skip)]
    pub doc: Option<DocId>,
}

/// The open documents as one document webview sees them (`tabs-changed`
/// payload, the sidebar's "Open" list): its own window's tabs, then the
/// documents of the other windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabsState {
    /// The native window.
    pub window: String,
    /// Its tabs, in order.
    pub tabs: Vec<TabInfo>,
    /// The tab shown.
    pub active: String,
    /// The documents of the other windows (windows in creation order, each
    /// window's tabs in order).
    pub others: Vec<TabInfo>,
}

/// A tab to show and the ones to hide (its window's others).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub window: String,
    pub show: String,
    pub hide: Vec<String>,
}

/// A tab moved to another window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// The window it left.
    pub from: String,
    /// The tab that window shows now (`None`: it is empty).
    pub from_active: Option<String>,
}

/// Merge All Windows: the tabs moved into the target window (in order,
/// each with the window it left) and the windows they left (now empty).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merge {
    pub moved: Vec<(String, String)>,
    pub emptied: Vec<String>,
}

/// Move Tab to New Window, registered by [`Registry::begin_detach`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detach {
    /// The window the tab leaves.
    pub from: String,
    /// The new (empty, unbuilt) window.
    pub to: String,
}

/// Why the registry refused something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a known document (or not one of the caller's window).
    UnknownWindow,
    /// The document is shown in this other tab.
    ShownIn(String),
    /// [`MAX_DOCS`] are open.
    TooManyWindows,
    /// A close sequence is asking, or this tab is already being moved.
    Busy,
}

impl Refused {
    /// The error the commands return.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownWindow => "not a document window".to_owned(),
            Self::ShownIn(_) => "the document is already shown in another window".to_owned(),
            Self::TooManyWindows => format!("at most {MAX_DOCS} documents can be open"),
            Self::Busy => "another close or move is in progress".to_owned(),
        }
    }
}

/// A running sequence of questions.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sequence {
    goal: Goal,
    /// The tab being asked.
    asking: String,
}

/// The documents and their windows.
#[derive(Debug)]
pub struct Registry {
    docs: BTreeMap<String, DocTab>,
    windows: BTreeMap<String, DocWindow>,
    /// Windows, most recently focused first.
    mru: Vec<String>,
    next_doc: u32,
    next_win: u32,
    sequence: Option<Sequence>,
    /// Tabs being moved to a new window ([`Self::begin_detach`]).
    detaching: BTreeSet<String>,
    recovery_offered: bool,
    /// Print window → owning document (tab label).
    prints: BTreeMap<String, String>,
    /// The key window when it is a print window (⌘W closes it).
    key_print: Option<String>,
    /// Documents of tabs destroyed while edited and unanswered (only on
    /// the way out): still snapshotted at exit.
    orphans: Vec<DocId>,
    /// "Untitled" in the UI language (Window menu list, window titles).
    untitled: String,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            docs: BTreeMap::new(),
            windows: BTreeMap::new(),
            mru: Vec::new(),
            next_doc: 0,
            next_win: 0,
            sequence: None,
            detaching: BTreeSet::new(),
            recovery_offered: false,
            prints: BTreeMap::new(),
            key_print: None,
            orphans: Vec::new(),
            untitled: "Untitled".to_owned(),
        }
    }
}

impl Registry {
    // -- creation ------------------------------------------------------------------

    /// A new window label, or `None` when [`MAX_DOCS`] windows exist
    /// (empty windows being built count too).
    fn alloc_window_label(&mut self) -> Option<String> {
        if self.windows.len() >= MAX_DOCS {
            return None;
        }
        self.next_win += 1;
        let label = format!("{WIN_PREFIX}{}", self.next_win);
        self.windows.insert(
            label.clone(),
            DocWindow {
                seq: self.next_win,
                ..DocWindow::default()
            },
        );
        Some(label)
    }

    /// Allocates the label of a new document webview and registers it,
    /// with the document it will show (`None`: the webview creates a blank
    /// one), as the last tab of `target` (a new window is allocated too). A
    /// recovered (dirty) document counts as edited from the start, so it is
    /// asked about and snapshotted even before its webview reports. The tab
    /// becomes active once built ([`Self::activate`]).
    pub fn create(
        &mut self,
        target: Target,
        doc: Option<&WorkbookInfo>,
    ) -> Result<Created, Refused> {
        if self.docs.len() >= MAX_DOCS {
            return Err(Refused::TooManyWindows);
        }
        if let Some(other) = doc.and_then(|i| self.tab_of(i.doc)) {
            return Err(Refused::ShownIn(other));
        }
        let (window, new_window) = match target {
            Target::Window(w) if self.windows.contains_key(&w) => (w, false),
            Target::Window(_) => return Err(Refused::UnknownWindow),
            Target::NewWindow => (
                self.alloc_window_label().ok_or(Refused::TooManyWindows)?,
                true,
            ),
        };
        self.next_doc += 1;
        let label = format!("{DOC_PREFIX}{}", self.next_doc);
        let entry = DocTab {
            seq: self.next_doc,
            window: window.clone(),
            doc: doc.map(|i| i.doc),
            edited: doc.is_some_and(|i| i.dirty),
            title: doc.map(|i| clean_title(&i.title)).unwrap_or_default(),
            ..DocTab::default()
        };
        self.docs.insert(label.clone(), entry);
        if let Some(w) = self.windows.get_mut(&window) {
            w.tabs.push(label.clone());
        }
        Ok(Created {
            doc: label,
            window,
            new_window,
        })
    }

    /// Move Tab to New Window, checked and registered in one step: `label`
    /// must be a tab of a window with another tab, not already being moved,
    /// and no close sequence may be asking. Allocates the new (empty)
    /// window; the caller builds it, moves the tab in with
    /// [`Self::move_to_window`] and always calls [`Self::end_detach`].
    /// `Ok(None)`: a window's only tab stays where it is.
    pub fn begin_detach(&mut self, label: &str) -> Result<Option<Detach>, Refused> {
        let from = self
            .docs
            .get(label)
            .ok_or(Refused::UnknownWindow)?
            .window
            .clone();
        if self.sequence.is_some() || self.detaching.contains(label) {
            return Err(Refused::Busy);
        }
        if self.tabs(&from).len() < 2 {
            return Ok(None);
        }
        let to = self.alloc_window_label().ok_or(Refused::TooManyWindows)?;
        self.detaching.insert(label.to_owned());
        Ok(Some(Detach { from, to }))
    }

    /// The move of `label` is over (done or failed).
    pub fn end_detach(&mut self, label: &str) {
        self.detaching.remove(label);
    }

    /// Tauri built the webview of `label`.
    pub fn built(&mut self, label: &str) {
        if let Some(t) = self.docs.get_mut(label) {
            t.built = true;
        }
    }

    /// Tauri built the window `win`.
    pub fn window_built(&mut self, win: &str) {
        if let Some(w) = self.windows.get_mut(win) {
            w.built = true;
        }
    }

    /// Documents and windows registered but not built yet.
    pub fn pending(&self) -> usize {
        self.docs.values().filter(|t| !t.built).count()
            + self.windows.values().filter(|w| !w.built).count()
    }

    /// Forgets a tab whose webview could not be created. Returns its window
    /// when that is now empty and built (the caller closes it); an empty
    /// window that was never built is forgotten too.
    pub fn forget(&mut self, label: &str) -> Option<String> {
        let tab = self.docs.remove(label)?;
        let win = tab.window;
        let empty = match self.windows.get_mut(&win) {
            Some(w) => {
                w.tabs.retain(|l| l != label);
                if w.active.as_deref() == Some(label) {
                    w.active = w.tabs.first().cloned();
                }
                w.tabs.is_empty()
            }
            None => false,
        };
        if !empty {
            return None;
        }
        if self.windows.get(&win).is_some_and(|w| w.built) {
            Some(win)
        } else {
            self.forget_window(&win);
            None
        }
    }

    /// Forgets an empty window (creation failed, or it is being closed).
    pub fn forget_window(&mut self, win: &str) {
        if self.windows.get(win).is_some_and(|w| w.tabs.is_empty()) {
            self.windows.remove(win);
            self.mru.retain(|l| l != win);
        }
    }

    // -- lookups -------------------------------------------------------------------

    /// A tab's state.
    #[cfg(test)]
    pub fn tab(&self, label: &str) -> Option<&DocTab> {
        self.docs.get(label)
    }

    /// Number of open documents (tabs).
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether no document is open.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Number of document windows.
    #[cfg(test)]
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// The native window of a tab.
    pub fn window_of_tab(&self, label: &str) -> Option<String> {
        self.docs.get(label).map(|t| t.window.clone())
    }

    /// The tab showing `doc`.
    pub fn tab_of(&self, doc: DocId) -> Option<String> {
        self.docs
            .iter()
            .find(|(_, t)| t.doc == Some(doc))
            .map(|(l, _)| l.clone())
    }

    /// The tabs of a window, in tab order.
    pub fn tabs(&self, win: &str) -> Vec<String> {
        self.windows
            .get(win)
            .map(|w| w.tabs.clone())
            .unwrap_or_default()
    }

    /// The tab a window shows (its first one until one is shown).
    pub fn active_of(&self, win: &str) -> Option<String> {
        let w = self.windows.get(win)?;
        w.active.clone().or_else(|| w.tabs.first().cloned())
    }

    /// Sets the name of untitled documents (UI language).
    pub fn set_untitled(&mut self, untitled: &str) {
        untitled.clone_into(&mut self.untitled);
    }

    fn display_title(&self, t: &DocTab) -> String {
        if t.title.is_empty() {
            self.untitled.clone()
        } else {
            t.title.clone()
        }
    }

    /// The native title of a window: its active tab's title, and whether
    /// that tab is edited.
    pub fn window_title(&self, win: &str) -> Option<(String, bool)> {
        let t = self.docs.get(&self.active_of(win)?)?;
        Some((self.display_title(t), t.edited))
    }

    /// Whether any tab of a window has unsaved changes (the macOS dot on
    /// its red button, which closes all of them).
    pub fn window_edited(&self, win: &str) -> bool {
        self.windows.get(win).is_some_and(|w| {
            w.tabs
                .iter()
                .any(|l| self.docs.get(l).is_some_and(|t| t.edited))
        })
    }

    fn tab_infos(&self, win: &str) -> Vec<TabInfo> {
        let Some(w) = self.windows.get(win) else {
            return Vec::new();
        };
        w.tabs
            .iter()
            .filter_map(|l| {
                let t = self.docs.get(l)?;
                Some(TabInfo {
                    label: l.clone(),
                    window: win.to_owned(),
                    title: t.title.clone(),
                    edited: t.edited,
                    path: None,
                    doc: t.doc,
                })
            })
            .collect()
    }

    /// The open documents as seen from window `win`: its tabs, then the
    /// other windows' documents.
    pub fn tabs_state(&self, win: &str) -> Option<TabsState> {
        self.windows.get(win)?;
        let others = self
            .windows_by_seq()
            .into_iter()
            .filter(|(l, _)| l.as_str() != win)
            .flat_map(|(l, _)| self.tab_infos(l))
            .collect();
        Some(TabsState {
            window: win.to_owned(),
            tabs: self.tab_infos(win),
            active: self.active_of(win).unwrap_or_default(),
            others,
        })
    }

    /// Every window, in creation order.
    pub fn window_labels(&self) -> Vec<String> {
        self.windows_by_seq()
            .into_iter()
            .map(|(l, _)| l.clone())
            .collect()
    }

    // -- reports and focus -------------------------------------------------------

    /// `set_document_edited`: records a known tab's state. A document shown
    /// in another tab is refused. Any report after the user answered during
    /// a sequence re-arms the guard when it says "edited" (the webview
    /// reports again after `confirm_close(true)`). Returns whether the
    /// title or the edited state changed (document lists, window title, menu).
    pub fn report(
        &mut self,
        label: &str,
        edited: bool,
        doc: Option<DocId>,
        title: Option<&str>,
    ) -> Result<bool, Refused> {
        if let Some(other) = doc.and_then(|d| self.tab_of(d)) {
            if other != label {
                return Err(Refused::ShownIn(other));
            }
        }
        let asking = self.sequence.as_ref().map(|s| s.asking.clone());
        let t = self.docs.get_mut(label).ok_or(Refused::UnknownWindow)?;
        if edited && t.confirmed && asking.as_deref() != Some(label) {
            t.confirmed = false;
        }
        let mut changed = t.edited != edited;
        t.edited = edited;
        if doc.is_some() {
            t.doc = doc;
        }
        if let Some(title) = title {
            if clean_title(title) != t.title {
                t.title = clean_title(title);
                changed = true;
            }
        }
        Ok(changed)
    }

    /// A window was focused (or lost focus). Returns whether the front
    /// document window changed.
    pub fn focus_changed(&mut self, label: &str, focused: bool) -> bool {
        if self.prints.contains_key(label) {
            if focused {
                self.key_print = Some(label.to_owned());
            } else if self.key_print.as_deref() == Some(label) {
                self.key_print = None;
            }
            return false;
        }
        if focused {
            self.key_print = None;
        }
        if !focused || !self.windows.contains_key(label) {
            return false;
        }
        let changed = self.mru.first().map(String::as_str) != Some(label);
        self.mru.retain(|l| l != label);
        self.mru.insert(0, label.to_owned());
        changed
    }

    /// The print window that is the key window, if any.
    pub fn key_print(&self) -> Option<&str> {
        self.key_print.as_deref()
    }

    fn windows_by_seq(&self) -> Vec<(&String, &DocWindow)> {
        let mut all: Vec<_> = self.windows.iter().collect();
        all.sort_by_key(|(_, w)| w.seq);
        all
    }

    /// The most recently focused window that still exists, else the first
    /// one created. Includes a window still being built, so a second File ▸
    /// New / Dock click does not open another.
    pub fn front_window(&self) -> Option<String> {
        self.mru
            .iter()
            .find(|l| self.windows.contains_key(*l))
            .cloned()
            .or_else(|| self.windows_by_seq().first().map(|(l, _)| (*l).clone()))
    }

    /// The front document: the active tab of the front window (menu
    /// actions go there).
    pub fn front(&self) -> Option<String> {
        self.front_window().and_then(|w| self.active_of(&w))
    }

    // -- placement ----------------------------------------------------------------

    /// Pure placement of `doc` opened from tab `caller`: it replaces the
    /// caller's document only when the webview asked (its document is
    /// untitled and untouched) and the shell agrees (the tab reported no
    /// unsaved changes); otherwise `mode` decides. Refused when the caller
    /// is not a known tab or the document is shown in another one.
    pub fn place(
        &self,
        caller: &str,
        doc: DocId,
        reuse: bool,
        mode: OpenMode,
    ) -> Result<Placement, Refused> {
        let t = self.docs.get(caller).ok_or(Refused::UnknownWindow)?;
        if let Some(other) = self.tab_of(doc) {
            if other != caller {
                return Err(Refused::ShownIn(other));
            }
        }
        Ok(placement_for(mode, reuse && !t.edited, &t.window))
    }

    /// File ▸ New from tab `caller`: a new tab of its window or a new window.
    pub fn place_new(&self, caller: &str, mode: OpenMode) -> Result<Placement, Refused> {
        let t = self.docs.get(caller).ok_or(Refused::UnknownWindow)?;
        Ok(placement_for(mode, false, &t.window))
    }

    /// Shows `info` in `label` (a [`Placement::Here`]); refused like [`Self::place`].
    pub fn bind(&mut self, label: &str, info: &WorkbookInfo) -> Result<(), Refused> {
        if let Some(other) = self.tab_of(info.doc) {
            if other != label {
                return Err(Refused::ShownIn(other));
            }
        }
        let t = self.docs.get_mut(label).ok_or(Refused::UnknownWindow)?;
        t.doc = Some(info.doc);
        t.title = clean_title(&info.title);
        Ok(())
    }

    /// `window_init`: the document the tab shows (also after a reload of
    /// its webview) and whether this is the first tab to start, which
    /// offers the crash recovery.
    pub fn init(&mut self, label: &str) -> Result<(Option<DocId>, bool), Refused> {
        let doc = self.docs.get(label).ok_or(Refused::UnknownWindow)?.doc;
        let offer = !self.recovery_offered;
        self.recovery_offered = true;
        Ok((doc, offer))
    }

    // -- tabs ---------------------------------------------------------------------

    /// Makes `label` the shown tab of its window; returns what to show and
    /// hide. `None` for an unknown tab.
    pub fn activate(&mut self, label: &str) -> Option<Shown> {
        let win = self.docs.get(label)?.window.clone();
        let w = self.windows.get_mut(&win)?;
        w.active = Some(label.to_owned());
        Some(Shown {
            window: win,
            show: label.to_owned(),
            hide: w.tabs.iter().filter(|l| *l != label).cloned().collect(),
        })
    }

    /// The tab `delta` steps from the active one of `win` (wrapping).
    pub fn cycle(&self, win: &str, delta: isize) -> Option<String> {
        let w = self.windows.get(win)?;
        let n = w.tabs.len();
        if n == 0 {
            return None;
        }
        let cur = self
            .active_of(win)
            .and_then(|a| w.tabs.iter().position(|l| *l == a))
            .unwrap_or(0);
        let n = isize::try_from(n).ok()?;
        let cur = isize::try_from(cur).ok()?;
        let next = usize::try_from((cur + delta).rem_euclid(n)).ok()?;
        w.tabs.get(next).cloned()
    }

    /// Moves `label` to position `index` (clamped) of its window.s tab order.
    /// Returns whether the order changed.
    pub fn move_tab(&mut self, label: &str, index: usize) -> bool {
        let Some(win) = self.docs.get(label).map(|t| t.window.clone()) else {
            return false;
        };
        let Some(w) = self.windows.get_mut(&win) else {
            return false;
        };
        let Some(from) = w.tabs.iter().position(|l| l == label) else {
            return false;
        };
        let to = index.min(w.tabs.len() - 1);
        if from == to {
            return false;
        }
        let tab = w.tabs.remove(from);
        w.tabs.insert(to, tab);
        true
    }

    /// The tab a window shows after `label` leaves it: the next one to the
    /// right, else the one to the left.
    fn neighbour(w: &DocWindow, label: &str) -> Option<String> {
        let i = w.tabs.iter().position(|l| l == label)?;
        w.tabs
            .get(i + 1)
            .or_else(|| i.checked_sub(1).and_then(|j| w.tabs.get(j)))
            .cloned()
    }

    /// Takes `label` out of its window.s tab order (fixing that window's active
    /// tab). Returns the window and the tab it shows now.
    fn unlink(&mut self, label: &str) -> Option<(String, Option<String>)> {
        let win = self.docs.get(label)?.window.clone();
        let w = self.windows.get_mut(&win)?;
        let was_active = w.active.as_deref() == Some(label);
        let next = Self::neighbour(w, label);
        w.tabs.retain(|l| l != label);
        if was_active || w.active.is_none() {
            w.active = next;
        }
        let active = w.active.clone();
        Some((win, active))
    }

    /// Moves a tab into window `to` as its last, shown tab (Move Tab to New
    /// Window, Merge All Windows).
    pub fn move_to_window(&mut self, label: &str, to: &str) -> Result<Moved, Refused> {
        if !self.windows.contains_key(to) {
            return Err(Refused::UnknownWindow);
        }
        let (from, from_active) = self.unlink(label).ok_or(Refused::UnknownWindow)?;
        if let Some(t) = self.docs.get_mut(label) {
            to.clone_into(&mut t.window);
        }
        if let Some(w) = self.windows.get_mut(to) {
            w.tabs.push(label.to_owned());
            w.active = Some(label.to_owned());
        }
        Ok(Moved { from, from_active })
    }

    /// Merge All Windows: every tab of every other window moves to the end
    /// of `into` (windows in creation order, tabs in tab order); `into`
    /// keeps its active tab. The emptied windows are left for the caller
    /// to close ([`Self::window_destroyed`] then has nothing to clean up).
    /// Nothing moves while a close sequence is asking or a tab is being
    /// moved to a new window.
    pub fn merge_into(&mut self, into: &str) -> Merge {
        let mut out = Merge::default();
        if !self.windows.contains_key(into) || self.sequence.is_some() || !self.detaching.is_empty()
        {
            return out;
        }
        let others: Vec<String> = self
            .windows_by_seq()
            .into_iter()
            .filter(|(l, _)| l.as_str() != into)
            .map(|(l, _)| l.clone())
            .collect();
        for win in others {
            let tabs = self.windows.get_mut(&win).map(|w| {
                w.active = None;
                std::mem::take(&mut w.tabs)
            });
            for label in tabs.unwrap_or_default() {
                if let Some(t) = self.docs.get_mut(&label) {
                    into.clone_into(&mut t.window);
                }
                out.moved.push((label, win.clone()));
            }
            out.emptied.push(win);
        }
        if let Some(w) = self.windows.get_mut(into) {
            w.tabs.extend(out.moved.iter().map(|(l, _)| l.clone()));
            if w.active.is_none() {
                w.active = w.tabs.first().cloned();
            }
        }
        out
    }

    // -- the unsaved-changes guard -------------------------------------------------

    /// Whether closing `label` must ask first (edited and not yet answered).
    pub fn must_ask(&self, label: &str) -> bool {
        self.docs
            .get(label)
            .is_some_and(|t| t.edited && !t.confirmed)
    }

    /// The user answered Save / Don't Save in `label`.
    pub fn confirm(&mut self, label: &str) {
        if let Some(t) = self.docs.get_mut(label) {
            t.confirmed = true;
        }
    }

    /// A window's tabs in asking order: the shown one first, then strip order.
    fn window_order(&self, win: &str) -> Vec<String> {
        let Some(w) = self.windows.get(win) else {
            return Vec::new();
        };
        let mut out: Vec<String> = self.active_of(win).into_iter().collect();
        out.extend(
            w.tabs
                .iter()
                .filter(|l| !out.contains(l))
                .cloned()
                .collect::<Vec<_>>(),
        );
        out
    }

    /// Windows, most recently focused first, then by creation (quit order).
    pub fn windows_mru(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .mru
            .iter()
            .filter(|l| self.windows.contains_key(*l))
            .cloned()
            .collect();
        for (l, _) in self.windows_by_seq() {
            if !out.contains(l) {
                out.push(l.clone());
            }
        }
        out
    }

    /// The tabs a goal asks, in order.
    fn targets(&self, goal: &Goal) -> Vec<String> {
        match goal {
            Goal::Exit => self
                .windows_mru()
                .iter()
                .flat_map(|w| self.window_order(w))
                .collect(),
            Goal::CloseWindow(win) => self.window_order(win),
            Goal::CloseTabs(list) => list
                .iter()
                .filter(|l| self.docs.contains_key(*l))
                .cloned()
                .collect(),
        }
    }

    fn kind_of(goal: &Goal) -> CloseKind {
        match goal {
            Goal::Exit => CloseKind::Quit,
            Goal::CloseWindow(_) | Goal::CloseTabs(_) => CloseKind::Tabs,
        }
    }

    /// Asks the next edited tab of `goal`, or finishes.
    fn run(&mut self, goal: Goal) -> Step {
        let next = self.targets(&goal).into_iter().find(|l| self.must_ask(l));
        match next {
            Some(label) => {
                let kind = Self::kind_of(&goal);
                self.sequence = Some(Sequence {
                    goal,
                    asking: label.clone(),
                });
                Step::Ask(label, kind)
            }
            None => {
                self.sequence = None;
                Step::Done(goal)
            }
        }
    }

    /// The tab a running sequence is asking.
    pub fn asking(&self) -> Option<&str> {
        self.sequence.as_ref().map(|s| s.asking.as_str())
    }

    /// Quit (⌘Q, app menu Quit, an exit request): asks the edited tabs one
    /// at a time, the front window first (its shown tab first). A quit
    /// already in progress asks its current tab again; a window or tab
    /// sequence in progress becomes the quit (answers given so far count).
    pub fn begin_quit(&mut self) -> Step {
        if let Some(s) = &self.sequence {
            if self.must_ask(&s.asking) {
                let label = s.asking.clone();
                self.sequence = Some(Sequence {
                    goal: Goal::Exit,
                    asking: label.clone(),
                });
                return Step::Ask(label, CloseKind::Quit);
            }
        }
        self.run(Goal::Exit)
    }

    /// The question a running sequence is asking, to ask it again (the red
    /// button pressed again, a reloaded webview, a lost event).
    pub fn reask(&self) -> Option<Step> {
        let s = self.sequence.as_ref()?;
        Some(Step::Ask(s.asking.clone(), Self::kind_of(&s.goal)))
    }

    /// The red close button of `win`. A window with one edited tab asks it
    /// like ⌘W ([`CloseKind::Window`]); with several tabs, the edited ones
    /// are asked in turn and the window closes once all have answered.
    /// While another sequence is asking, the close is held ([`Self::reask`]).
    pub fn begin_close_window(&mut self, win: &str) -> WindowClose {
        let Some(w) = self.windows.get(win) else {
            return WindowClose::Proceed;
        };
        if self.sequence.is_some() {
            return WindowClose::Busy;
        }
        if !w.tabs.iter().any(|l| self.must_ask(l)) {
            return WindowClose::Proceed;
        }
        if let [only] = w.tabs.as_slice() {
            return WindowClose::Hold(Step::Ask(only.clone(), CloseKind::Window));
        }
        WindowClose::Hold(self.run(Goal::CloseWindow(win.to_owned())))
    }

    /// Close Other Tabs from `label`: the other tabs of its window, the
    /// edited ones asked in turn. `None` when `label` is unknown or another
    /// sequence is asking.
    pub fn begin_close_others(&mut self, label: &str) -> Option<Step> {
        if self.sequence.is_some() {
            return None;
        }
        let win = self.docs.get(label)?.window.clone();
        let others: Vec<String> = self.tabs(&win).into_iter().filter(|l| l != label).collect();
        Some(self.run(Goal::CloseTabs(others)))
    }

    /// Closing a tab at the end of Close Other Tabs: only one that is still
    /// open and has no unanswered changes (edited again since, or never
    /// asked: skipped, never closed unasked). The last tab of a window
    /// closes the window, which then holds that tab only.
    pub fn closable(&self, label: &str) -> Option<TabClose> {
        match self.close_tab(label)? {
            TabClose::Ask => None,
            c => Some(c),
        }
    }

    /// Closing one tab (⌘W, its ×): ask it first when edited; the last tab
    /// of a window closes the window.
    pub fn close_tab(&self, label: &str) -> Option<TabClose> {
        let t = self.docs.get(label)?;
        if self.must_ask(label) {
            return Some(TabClose::Ask);
        }
        let only = self
            .windows
            .get(&t.window)
            .is_some_and(|w| w.tabs.len() <= 1);
        Some(if only {
            TabClose::Window(t.window.clone())
        } else {
            TabClose::Now
        })
    }

    /// `label` answered Save / Don't Save in a sequence: the next tab, or
    /// done. `None` (ignored) unless the sequence is asking `label`.
    pub fn answered(&mut self, label: &str) -> Option<Step> {
        let goal = match &self.sequence {
            Some(s) if s.asking == label => s.goal.clone(),
            _ => return None,
        };
        self.confirm(label);
        Some(self.run(goal))
    }

    /// Cancel (or a cancelled Save As) in the tab a sequence is asking
    /// aborts the whole sequence: earlier answers no longer count, so those
    /// tabs ask again next time. Ignored from any other tab.
    pub fn cancel(&mut self, label: &str) -> bool {
        if self.asking() != Some(label) {
            return false;
        }
        self.abort();
        true
    }

    fn abort(&mut self) {
        self.sequence = None;
        for t in self.docs.values_mut() {
            t.confirmed = false;
        }
    }

    // -- closing and destruction -------------------------------------------------

    /// A print window opened by tab `owner`.
    pub fn add_print(&mut self, print: &str, owner: &str) {
        self.prints.insert(print.to_owned(), owner.to_owned());
    }

    /// Removes a tab's record; collects what closing it means into `out`.
    /// Returns whether the user had answered there.
    fn drop_tab(&mut self, label: &str, out: &mut Gone) -> bool {
        let Some(t) = self.docs.remove(label) else {
            return false;
        };
        if t.edited && !t.confirmed {
            // Only on the way out (closing is guarded): keep the document
            // for the exit snapshot.
            self.orphans.extend(t.doc);
        } else {
            out.close_docs.extend(t.doc);
        }
        let owned: Vec<String> = self
            .prints
            .iter()
            .filter(|(_, o)| o.as_str() == label)
            .map(|(p, _)| p.clone())
            .collect();
        for p in &owned {
            self.prints.remove(p);
        }
        out.prints.extend(owned);
        t.confirmed
    }

    /// The sequence after tab `label` went away: it goes on only when the
    /// user had answered there; a tab gone without an answer aborts it.
    fn after_drop(&mut self, label: &str, answered: bool) -> Option<Step> {
        let goal = match &self.sequence {
            Some(s) if s.asking == label => s.goal.clone(),
            _ => return None,
        };
        if answered {
            Some(self.run(goal))
        } else {
            self.abort();
            None
        }
    }

    /// A tab that is not the last of its window is closed (its webview is
    /// being closed by the caller). Returns the clean-up and the tab its
    /// window shows next.
    pub fn tab_closed(&mut self, label: &str) -> Gone {
        let mut out = Gone::default();
        let Some((_, active)) = self.unlink(label) else {
            return out;
        };
        out.activate = active;
        let answered = self.drop_tab(label, &mut out);
        out.step = self.after_drop(label, answered);
        out
    }

    /// A window was destroyed: its tabs go with it (print windows are just
    /// forgotten). If a sequence was asking one of its tabs, it goes on
    /// only when the user had answered there.
    pub fn window_destroyed(&mut self, label: &str) -> Gone {
        let mut out = Gone::default();
        if self.key_print.as_deref() == Some(label) {
            self.key_print = None;
        }
        if self.prints.remove(label).is_some() {
            return out;
        }
        let Some(w) = self.windows.remove(label) else {
            return out;
        };
        self.mru.retain(|l| l != label);
        let asking = self.asking().map(str::to_owned);
        let mut step = None;
        for tab in &w.tabs {
            let answered = self.drop_tab(tab, &mut out);
            if asking.as_deref() == Some(tab.as_str()) {
                step = self.after_drop(tab, answered);
            }
        }
        // A sequence closing this very window that was still asking elsewhere ends here.
        if matches!(&self.sequence, Some(s) if s.goal == Goal::CloseWindow(label.to_owned())) {
            self.abort();
            step = None;
        }
        out.step = step;
        out
    }

    /// What the final exit writes or removes.
    pub fn exit_plan(&self) -> ExitPlan {
        let mut plan = ExitPlan::default();
        for t in self.docs.values() {
            let Some(doc) = t.doc else { continue };
            if t.edited && !t.confirmed {
                plan.snapshot.push(doc);
            } else {
                plan.discard.push(doc);
            }
        }
        plan.snapshot.extend(self.orphans.iter().copied());
        plan.snapshot.sort_by_key(|d| d.0);
        plan.snapshot.dedup();
        plan.discard.sort_by_key(|d| d.0);
        plan
    }

    /// The Window menu's document list: windows in creation order, each
    /// window's tabs in tab order, titled "Untitled" (UI language) when
    /// untitled, the front document checked.
    pub fn window_list(&self) -> Vec<ListEntry> {
        let front = self.front();
        self.windows_by_seq()
            .into_iter()
            .flat_map(|(_, w)| w.tabs.iter())
            .filter_map(|l| {
                let t = self.docs.get(l)?;
                Some(ListEntry {
                    label: l.clone(),
                    title: self.display_title(t),
                    checked: front.as_deref() == Some(l.as_str()),
                })
            })
            .collect()
    }
}

/// Managed state: the registry behind a leaf lock (see the module docs).
#[derive(Debug, Default)]
pub struct Windows(pub Mutex<Registry>);

impl Windows {
    /// Runs `f` with the registry locked (never call Tauri, the engine or a
    /// dialog inside `f`).
    pub fn with<T>(&self, f: impl FnOnce(&mut Registry) -> T) -> T {
        f(&mut crate::state::lock(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(doc: u32, title: &str) -> WorkbookInfo {
        WorkbookInfo {
            doc: DocId(doc),
            title: title.to_owned(),
            path: None,
            sheets: Vec::new(),
            active_sheet: 0,
            dirty: false,
            version: 0,
            warnings: Vec::new(),
            external_links: None,
        }
    }

    /// A registered, built document in a new built window.
    fn window(r: &mut Registry, doc: Option<&WorkbookInfo>) -> String {
        let c = r
            .create(Target::NewWindow, doc)
            .unwrap_or_else(|_| Created {
                doc: String::new(),
                window: String::new(),
                new_window: false,
            });
        r.window_built(&c.window);
        r.built(&c.doc);
        r.activate(&c.doc);
        c.doc
    }

    /// A registered, built, shown document as a new tab of `of`'s window.
    fn tab(r: &mut Registry, of: &str, doc: Option<&WorkbookInfo>) -> String {
        let win = r.window_of_tab(of).unwrap_or_default();
        let label = r
            .create(Target::Window(win), doc)
            .map(|c| c.doc)
            .unwrap_or_default();
        r.built(&label);
        r.activate(&label);
        label
    }

    fn win(r: &Registry, label: &str) -> String {
        r.window_of_tab(label).unwrap_or_default()
    }

    fn report(r: &mut Registry, l: &str, edited: bool, doc: Option<u32>) {
        assert!(r.report(l, edited, doc.map(DocId), None).is_ok(), "{l}");
    }

    #[test]
    fn labels_are_unique_and_recognised() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, Some(&info(4, "B.ods")));
        assert_eq!((a.as_str(), b.as_str()), ("doc-1", "doc-2"));
        assert_eq!(
            (win(&r, &a).as_str(), win(&r, &b).as_str()),
            ("win-1", "win-2")
        );
        r.tab_closed(&a);
        r.window_destroyed("win-1");
        assert_eq!(window(&mut r, None), "doc-3", "labels are never reused");
        assert!(is_doc_label("doc-12") && is_win_label("win-3"));
        for l in ["main", "doc-", "doc-x", "tally-print-3", "doc-1a", "win-2"] {
            assert!(!is_doc_label(l), "{l}");
        }
        for l in ["win-", "doc-1", "window-1", "win-1x"] {
            assert!(!is_win_label(l), "{l}");
        }
        assert_eq!(r.tab_of(DocId(4)), Some(b.clone()));
        // A document gets one tab only, in any window.
        assert_eq!(
            r.create(Target::NewWindow, Some(&info(4, "B.ods"))),
            Err(Refused::ShownIn(b.clone()))
        );
        assert_eq!(
            r.create(Target::Window(win(&r, &b)), Some(&info(4, "B.ods"))),
            Err(Refused::ShownIn(b))
        );
    }

    #[test]
    fn the_number_of_documents_is_capped_across_windows() {
        let mut r = Registry::default();
        let first = window(&mut r, None);
        let w = win(&r, &first);
        for i in 1..MAX_DOCS {
            let target = if i % 2 == 0 {
                Target::NewWindow
            } else {
                Target::Window(w.clone())
            };
            assert!(r.create(target, None).is_ok());
        }
        assert_eq!(
            r.create(Target::NewWindow, None),
            Err(Refused::TooManyWindows)
        );
        assert_eq!(
            r.create(Target::Window(w), None),
            Err(Refused::TooManyWindows)
        );
        assert!(Refused::TooManyWindows.message().contains("100"));
    }

    #[test]
    fn unknown_labels_are_refused_not_registered() {
        let mut r = Registry::default();
        assert_eq!(
            r.report("doc-9", true, Some(DocId(1)), None),
            Err(Refused::UnknownWindow)
        );
        assert_eq!(r.init("tally-print-1"), Err(Refused::UnknownWindow));
        assert_eq!(
            r.place("doc-9", DocId(1), true, OpenMode::Tabs),
            Err(Refused::UnknownWindow)
        );
        assert_eq!(
            r.place_new("doc-9", OpenMode::Tabs),
            Err(Refused::UnknownWindow)
        );
        assert_eq!(r.bind("doc-9", &info(1, "x")), Err(Refused::UnknownWindow));
        assert_eq!(
            r.create(Target::Window("win-4".into()), None),
            Err(Refused::UnknownWindow)
        );
        r.confirm("doc-9");
        assert!(!r.focus_changed("win-9", true));
        assert!(r.activate("doc-9").is_none());
        assert!(!r.move_tab("doc-9", 0));
        assert_eq!(
            r.move_to_window("doc-9", "win-1"),
            Err(Refused::UnknownWindow)
        );
        assert_eq!(r.close_tab("doc-9"), None);
        assert_eq!(r.begin_close_others("doc-9"), None);
        assert!(
            r.is_empty() && r.window_count() == 0,
            "nothing was registered on the way"
        );
    }

    #[test]
    fn init_returns_the_bound_document_every_time_and_offers_recovery_once() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, Some(&info(7, "Book.ods")));
        assert_eq!(r.init(&a), Ok((None, true)));
        assert_eq!(r.init(&b), Ok((Some(DocId(7)), false)));
        // A reload of the webview gets its document again (no leak, no blank one).
        assert_eq!(r.init(&b), Ok((Some(DocId(7)), false)));
        report(&mut r, &a, false, Some(3));
        assert_eq!(r.init(&a), Ok((Some(DocId(3)), false)));
    }

    #[test]
    fn a_recovered_dirty_document_counts_as_edited_before_its_webview_reports() {
        let mut r = Registry::default();
        let mut rec = info(5, "Untitled");
        rec.dirty = true;
        let a = window(&mut r, Some(&rec));
        assert!(r.must_ask(&a));
        assert_eq!(r.exit_plan().snapshot, [DocId(5)]);
        assert_eq!(r.begin_quit(), Step::Ask(a, CloseKind::Quit));
    }

    #[test]
    fn the_setting_decides_between_a_tab_and_a_window() {
        assert_eq!(
            placement_for(OpenMode::Tabs, true, "win-1"),
            Placement::Here
        );
        assert_eq!(
            placement_for(OpenMode::Windows, true, "win-1"),
            Placement::Here
        );
        assert_eq!(
            placement_for(OpenMode::Tabs, false, "win-1"),
            Placement::NewTab("win-1".into())
        );
        assert_eq!(
            placement_for(OpenMode::Windows, false, "win-1"),
            Placement::NewWindow
        );
        assert_eq!(OpenMode::default(), OpenMode::Tabs);
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let w = win(&r, &a);
        assert_eq!(r.place_new(&a, OpenMode::Tabs), Ok(Placement::NewTab(w)));
        assert_eq!(r.place_new(&a, OpenMode::Windows), Ok(Placement::NewWindow));
    }

    #[test]
    fn reuse_only_an_untouched_tab_that_asked_for_it() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let w = win(&r, &a);
        report(&mut r, &a, false, Some(1));
        assert_eq!(
            r.place(&a, DocId(2), true, OpenMode::Tabs),
            Ok(Placement::Here)
        );
        assert_eq!(
            r.place(&a, DocId(2), false, OpenMode::Tabs),
            Ok(Placement::NewTab(w.clone()))
        );
        assert_eq!(
            r.place(&a, DocId(2), false, OpenMode::Windows),
            Ok(Placement::NewWindow)
        );
        // The shell's own record vetoes reuse of an edited tab.
        report(&mut r, &a, true, Some(1));
        assert_eq!(
            r.place(&a, DocId(2), true, OpenMode::Tabs),
            Ok(Placement::NewTab(w))
        );
        // A document already shown elsewhere is refused.
        let b = window(&mut r, Some(&info(9, "Nine")));
        assert_eq!(
            r.place(&a, DocId(9), true, OpenMode::Tabs),
            Err(Refused::ShownIn(b.clone()))
        );
        assert!(r.bind(&a, &info(2, "Two.ods")).is_ok());
        assert_eq!(r.tab(&a).map(|t| t.title.as_str()), Some("Two.ods"));
        assert_eq!(r.tab_of(DocId(2)), Some(a.clone()));
        // Neither a report nor a bind can take another tab's document.
        assert_eq!(
            r.report(&a, false, Some(DocId(9)), None),
            Err(Refused::ShownIn(b.clone()))
        );
        assert_eq!(r.bind(&a, &info(9, "Nine")), Err(Refused::ShownIn(b)));
        assert_eq!(r.tab_of(DocId(2)), Some(a));
    }

    #[test]
    fn tabs_are_added_at_the_end_and_shown_once_built() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A.ods")));
        let w = win(&r, &a);
        let c = r
            .create(Target::Window(w.clone()), None)
            .unwrap_or_else(|_| Created {
                doc: String::new(),
                window: String::new(),
                new_window: true,
            });
        assert_eq!((c.window.as_str(), c.new_window), (w.as_str(), false));
        assert_eq!(r.tabs(&w), [a.clone(), c.doc.clone()]);
        assert_eq!(
            r.active_of(&w),
            Some(a.clone()),
            "still the old tab until built"
        );
        assert_eq!(r.pending(), 1);
        r.built(&c.doc);
        let shown = r.activate(&c.doc);
        assert_eq!(
            shown,
            Some(Shown {
                window: w.clone(),
                show: c.doc.clone(),
                hide: vec![a.clone()],
            })
        );
        let state = r.tabs_state(&w).map(|s| {
            (
                s.active,
                s.tabs
                    .iter()
                    .map(|t| (t.label.clone(), t.title.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(
            state,
            Some((
                c.doc.clone(),
                vec![(a, "A.ods".to_owned()), (c.doc, String::new())]
            ))
        );
    }

    #[test]
    fn a_failed_webview_is_forgotten_and_an_empty_window_with_it() {
        let mut r = Registry::default();
        let c = r
            .create(Target::NewWindow, None)
            .unwrap_or_else(|_| Created {
                doc: String::new(),
                window: String::new(),
                new_window: false,
            });
        assert_eq!(r.pending(), 2, "a window and its webview");
        // Never built: both go.
        assert_eq!(r.forget(&c.doc), None);
        assert_eq!((r.len(), r.window_count()), (0, 0));
        // A built window left empty is handed back to be closed.
        let c = r
            .create(Target::NewWindow, None)
            .unwrap_or_else(|_| Created {
                doc: String::new(),
                window: String::new(),
                new_window: false,
            });
        r.window_built(&c.window);
        assert_eq!(r.forget(&c.doc), Some(c.window.clone()));
        assert_eq!(r.window_destroyed(&c.window), Gone::default());
        // A window with other tabs keeps them (and its active one).
        let a = window(&mut r, None);
        let w = win(&r, &a);
        let b = r
            .create(Target::Window(w), None)
            .map(|c| c.doc)
            .unwrap_or_default();
        assert_eq!(r.forget(&b), None);
        assert_eq!(r.front(), Some(a));
    }

    #[test]
    fn cycling_wraps_and_moving_reorders() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let c = tab(&mut r, &a, None);
        let w = win(&r, &a);
        assert_eq!(r.active_of(&w), Some(c.clone()));
        assert_eq!(r.cycle(&w, 1), Some(a.clone()), "wraps past the end");
        assert_eq!(r.cycle(&w, -1), Some(b.clone()));
        r.activate(&a);
        assert_eq!(r.cycle(&w, -1), Some(c.clone()), "wraps before the start");
        assert!(r.move_tab(&a, 2));
        assert_eq!(r.tabs(&w), [b.clone(), c.clone(), a.clone()]);
        assert!(!r.move_tab(&a, 99), "clamped to the end: already there");
        assert!(r.move_tab(&c, 0));
        assert_eq!(r.tabs(&w), [c, b, a.clone()]);
        assert_eq!(r.active_of(&w), Some(a), "moving keeps the shown tab");
        assert_eq!(r.cycle("win-9", 1), None);
    }

    #[test]
    fn closing_a_tab_shows_its_right_neighbour_else_the_left_one() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let b = tab(&mut r, &a, Some(&info(2, "B")));
        let c = tab(&mut r, &a, Some(&info(3, "C")));
        let w = win(&r, &a);
        r.activate(&b);
        assert_eq!(r.close_tab(&b), Some(TabClose::Now));
        let gone = r.tab_closed(&b);
        assert_eq!(gone.activate, Some(c.clone()));
        assert_eq!(gone.close_docs, [DocId(2)]);
        let gone = r.tab_closed(&c);
        assert_eq!(
            gone.activate,
            Some(a.clone()),
            "no right neighbour: the left one"
        );
        // Closing a hidden tab keeps the shown one.
        let d = tab(&mut r, &a, None);
        r.activate(&a);
        assert_eq!(r.tab_closed(&d).activate, Some(a.clone()));
        // The last tab closes its window.
        assert_eq!(r.close_tab(&a), Some(TabClose::Window(w)));
        // An edited tab is asked first.
        report(&mut r, &a, true, Some(1));
        assert_eq!(r.close_tab(&a), Some(TabClose::Ask));
        r.confirm(&a);
        assert!(matches!(r.close_tab(&a), Some(TabClose::Window(_))));
    }

    #[test]
    fn move_tab_to_a_new_window_and_merge_all_windows() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let b = tab(&mut r, &a, Some(&info(2, "B")));
        let w1 = win(&r, &a);
        // Move Tab to New Window.
        let d = r.begin_detach(&b).ok().flatten();
        let w2 = d.map(|d| d.to).unwrap_or_default();
        r.window_built(&w2);
        assert_eq!(
            r.move_to_window(&b, &w2),
            Ok(Moved {
                from: w1.clone(),
                from_active: Some(a.clone()),
            })
        );
        r.end_detach(&b);
        assert_eq!(r.window_of_tab(&b), Some(w2.clone()));
        assert_eq!(
            (r.tabs(&w1), r.tabs(&w2)),
            (vec![a.clone()], vec![b.clone()])
        );
        assert_eq!(r.active_of(&w2), Some(b.clone()));
        let c = tab(&mut r, &b, Some(&info(3, "C")));
        let d = window(&mut r, Some(&info(4, "D")));
        let w3 = win(&r, &d);
        // Merge into the second window: the others' tabs follow in creation order.
        let m = r.merge_into(&w2);
        assert_eq!(m.moved, [(a.clone(), w1.clone()), (d.clone(), w3.clone())]);
        assert_eq!(m.emptied, [w1.clone(), w3.clone()]);
        assert_eq!(r.tabs(&w2), [b.clone(), c.clone(), a.clone(), d.clone()]);
        assert_eq!(r.active_of(&w2), Some(c), "the target keeps its shown tab");
        assert!(r.tabs(&w1).is_empty());
        // The emptied windows close without touching any document.
        assert_eq!(r.window_destroyed(&w1), Gone::default());
        assert_eq!(r.window_destroyed(&w3), Gone::default());
        assert_eq!((r.len(), r.window_count()), (4, 1));
        assert_eq!(r.window_of_tab(&a), Some(w2.clone()));
        assert_eq!(r.merge_into("win-77"), Merge::default());
    }

    #[test]
    fn menu_actions_go_to_the_shown_tab_of_the_front_window() {
        let mut r = Registry::default();
        assert_eq!(r.front(), None);
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        let b2 = tab(&mut r, &b, None);
        assert_eq!(r.front(), Some(a.clone()), "never focused: first created");
        assert!(r.focus_changed(&win(&r, &b), true));
        assert!(!r.focus_changed(&win(&r, &b), true));
        assert_eq!(r.front(), Some(b2));
        r.activate(&b);
        assert_eq!(r.front(), Some(b.clone()));
        // Focus events of tab labels (not windows) change nothing.
        assert!(!r.focus_changed(&a, true));
        r.focus_changed(&win(&r, &a), true);
        r.window_destroyed(&win(&r, &a));
        assert_eq!(r.front(), Some(b));
    }

    #[test]
    fn a_key_print_window_is_tracked() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let w = win(&r, &a);
        r.add_print("tally-print-3", &a);
        r.focus_changed(&w, true);
        assert_eq!(r.key_print(), None);
        assert!(!r.focus_changed("tally-print-3", true));
        assert_eq!(r.key_print(), Some("tally-print-3"));
        assert_eq!(
            r.front(),
            Some(a.clone()),
            "the owner stays the front document"
        );
        r.focus_changed("tally-print-3", false);
        assert_eq!(r.key_print(), None);
        r.focus_changed("tally-print-3", true);
        r.focus_changed(&w, true);
        assert_eq!(r.key_print(), None);
        r.focus_changed("tally-print-3", true);
        r.window_destroyed("tally-print-3");
        assert_eq!(r.key_print(), None);
    }

    #[test]
    fn anything_being_built_dedupes_new_windows_and_blocks_the_exit() {
        let mut r = Registry::default();
        let c = r
            .create(Target::NewWindow, None)
            .unwrap_or_else(|_| Created {
                doc: String::new(),
                window: String::new(),
                new_window: false,
            });
        assert_eq!(r.pending(), 2);
        // A second File ▸ New / Dock click sees a front document and opens nothing.
        assert_eq!(r.front(), Some(c.doc.clone()));
        assert_eq!(
            exit_action(None, false, r.len(), r.pending()),
            ExitAction::StayAlive
        );
        r.window_built(&c.window);
        r.built(&c.doc);
        assert_eq!(r.pending(), 0);
        assert_eq!(
            exit_action(None, false, r.len(), r.pending()),
            ExitAction::Quit
        );
    }

    #[test]
    fn quit_asks_each_edited_tab_in_every_window_in_turn() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let c = window(&mut r, None);
        let d = tab(&mut r, &c, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, false, Some(2));
        report(&mut r, &c, true, Some(3));
        report(&mut r, &d, true, Some(4));
        r.focus_changed(&win(&r, &c), true);
        // The front window first, its shown tab (d) first; clean tabs are never asked.
        assert_eq!(r.begin_quit(), Step::Ask(d.clone(), CloseKind::Quit));
        // ⌘Q again while asking: the same tab again.
        assert_eq!(r.begin_quit(), Step::Ask(d.clone(), CloseKind::Quit));
        // Only the asked tab can answer.
        assert_eq!(r.answered(&a), None);
        assert_eq!(r.answered(&d), Some(Step::Ask(c.clone(), CloseKind::Quit)));
        assert_eq!(r.answered(&c), Some(Step::Ask(a.clone(), CloseKind::Quit)));
        assert_eq!(r.asking(), Some(a.as_str()));
        assert_eq!(r.answered(&a), Some(Step::Done(Goal::Exit)));
        assert_eq!(r.asking(), None);
        let plan = r.exit_plan();
        assert!(plan.snapshot.is_empty());
        assert_eq!(plan.discard, [DocId(1), DocId(2), DocId(3), DocId(4)]);
    }

    #[test]
    fn cancel_in_the_asked_tab_aborts_the_whole_quit() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        // a: Don't Save; b: Cancel. A cancel from a tab not being asked is ignored.
        assert_eq!(r.answered(&a), Some(Step::Ask(b.clone(), CloseKind::Quit)));
        assert!(!r.cancel(&a));
        assert_eq!(r.asking(), Some(b.as_str()));
        assert!(r.cancel(&b));
        assert_eq!(r.asking(), None);
        // a's answer no longer counts: closing it asks again.
        assert!(r.must_ask(&a) && r.must_ask(&b));
        assert_eq!(r.begin_quit(), Step::Ask(a, CloseKind::Quit));
    }

    #[test]
    fn an_edit_after_answering_re_arms_the_guard() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        assert_eq!(r.answered(&a), Some(Step::Ask(b.clone(), CloseKind::Quit)));
        assert!(!r.must_ask(&a));
        // While b is asked, a is edited again (its webview reports anew).
        report(&mut r, &a, true, Some(1));
        assert!(r.must_ask(&a));
        // b answers: a is asked again instead of being dropped at exit.
        assert_eq!(r.answered(&b), Some(Step::Ask(a, CloseKind::Quit)));
    }

    #[test]
    fn quit_with_nothing_edited_exits_at_once() {
        let mut r = Registry::default();
        assert_eq!(r.begin_quit(), Step::Done(Goal::Exit));
        let a = window(&mut r, None);
        report(&mut r, &a, false, Some(1));
        assert_eq!(r.begin_quit(), Step::Done(Goal::Exit));
    }

    #[test]
    fn a_tab_saved_meanwhile_is_skipped() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        r.activate(&a);
        report(&mut r, &a, true, None);
        report(&mut r, &b, true, None);
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        report(&mut r, &b, false, None);
        assert_eq!(r.answered(&a), Some(Step::Done(Goal::Exit)));
    }

    #[test]
    fn the_red_button_of_a_window_with_several_edited_tabs_asks_each_in_turn() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let b = tab(&mut r, &a, Some(&info(2, "B")));
        let c = tab(&mut r, &a, Some(&info(3, "C")));
        let other = window(&mut r, Some(&info(4, "D")));
        let w = win(&r, &a);
        r.activate(&b);
        report(&mut r, &a, true, None);
        report(&mut r, &c, true, None);
        report(&mut r, &other, true, None);
        // The shown tab (b) is clean: a then c, never the other window's tab.
        assert_eq!(
            r.begin_close_window(&w),
            WindowClose::Hold(Step::Ask(a.clone(), CloseKind::Tabs))
        );
        // The red button again (or another window's) while asking: ignored.
        assert_eq!(r.begin_close_window(&w), WindowClose::Busy);
        assert_eq!(r.begin_close_window(&win(&r, &other)), WindowClose::Busy);
        assert_eq!(r.answered(&a), Some(Step::Ask(c.clone(), CloseKind::Tabs)));
        assert_eq!(
            r.answered(&c),
            Some(Step::Done(Goal::CloseWindow(w.clone())))
        );
        // The window closes: its documents are closed, nothing is orphaned.
        let gone = r.window_destroyed(&w);
        assert_eq!(gone.close_docs, [DocId(1), DocId(2), DocId(3)]);
        assert_eq!(gone.step, None);
        assert_eq!(r.exit_plan().snapshot, [DocId(4)]);
    }

    #[test]
    fn cancel_aborts_a_window_close_and_its_answers() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let w = win(&r, &a);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(
            r.begin_close_window(&w),
            WindowClose::Hold(Step::Ask(b.clone(), CloseKind::Tabs))
        );
        assert_eq!(r.answered(&b), Some(Step::Ask(a.clone(), CloseKind::Tabs)));
        assert!(r.cancel(&a));
        assert!(
            r.must_ask(&a) && r.must_ask(&b),
            "b's Don't Save no longer counts"
        );
        assert_eq!(r.tabs(&w).len(), 2, "nothing was closed");
        // A window with one edited tab asks it like ⌘W; a clean window just closes.
        let solo = window(&mut r, None);
        report(&mut r, &solo, true, Some(3));
        assert_eq!(
            r.begin_close_window(&win(&r, &solo)),
            WindowClose::Hold(Step::Ask(solo.clone(), CloseKind::Window))
        );
        report(&mut r, &solo, false, Some(3));
        assert_eq!(r.begin_close_window(&win(&r, &solo)), WindowClose::Proceed);
        assert_eq!(r.begin_close_window("win-99"), WindowClose::Proceed);
    }

    #[test]
    fn quit_during_a_window_close_takes_it_over() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let c = window(&mut r, None);
        let w = win(&r, &a);
        r.activate(&a);
        for (l, d) in [(&a, 1), (&b, 2), (&c, 3)] {
            report(&mut r, l, true, Some(d));
        }
        assert_eq!(
            r.begin_close_window(&w),
            WindowClose::Hold(Step::Ask(a.clone(), CloseKind::Tabs))
        );
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        assert_eq!(r.answered(&a), Some(Step::Ask(b.clone(), CloseKind::Quit)));
        assert_eq!(r.answered(&b), Some(Step::Ask(c.clone(), CloseKind::Quit)));
        assert_eq!(r.answered(&c), Some(Step::Done(Goal::Exit)));
    }

    #[test]
    fn close_other_tabs_asks_the_edited_ones_then_closes_them() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let b = tab(&mut r, &a, Some(&info(2, "B")));
        let c = tab(&mut r, &a, Some(&info(3, "C")));
        report(&mut r, &a, true, None);
        report(&mut r, &c, true, None);
        assert_eq!(
            r.begin_close_others(&b),
            Some(Step::Ask(a.clone(), CloseKind::Tabs))
        );
        assert_eq!(r.begin_close_others(&b), None, "busy");
        assert_eq!(r.answered(&a), Some(Step::Ask(c.clone(), CloseKind::Tabs)));
        let goal = Goal::CloseTabs(vec![a.clone(), c.clone()]);
        assert_eq!(r.answered(&c), Some(Step::Done(goal)));
        // The shell then closes them one by one.
        r.activate(&b);
        assert_eq!(r.tab_closed(&a).close_docs, [DocId(1)]);
        assert_eq!(r.tab_closed(&c).close_docs, [DocId(3)]);
        assert_eq!(r.tabs(&win(&r, &b)), std::slice::from_ref(&b));
        // Nothing edited: done at once.
        let d = tab(&mut r, &b, None);
        assert_eq!(
            r.begin_close_others(&b),
            Some(Step::Done(Goal::CloseTabs(vec![d])))
        );
    }

    #[test]
    fn closing_the_asked_tab_with_an_answer_continues_the_sequence() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let a2 = tab(&mut r, &a, None);
        let b = window(&mut r, None);
        r.activate(&a);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &a2, false, Some(5));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        // a had its own ⌘W prompt up: Don't Save there closes the tab (confirm + close).
        r.confirm(&a);
        let gone = r.tab_closed(&a);
        assert_eq!(gone.close_docs, [DocId(1)]);
        assert_eq!(gone.activate, Some(a2));
        assert_eq!(gone.step, Some(Step::Ask(b.clone(), CloseKind::Quit)));
        // The same through a destroyed one-tab window.
        r.confirm(&b);
        let gone = r.window_destroyed(&win(&r, &b));
        assert_eq!(gone.close_docs, [DocId(2)]);
        assert_eq!(gone.step, Some(Step::Done(Goal::Exit)));
    }

    #[test]
    fn a_stale_sequence_never_finishes_when_the_asked_tab_closes_unanswered() {
        // ⌘Q while a's ⌘W prompt was up; the user cancelled that prompt
        // (the webview then calls cancel_quit) and later closed a normally.
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), Step::Ask(a.clone(), CloseKind::Quit));
        assert!(r.cancel(&a));
        report(&mut r, &a, false, Some(1));
        assert_eq!(r.window_destroyed(&win(&r, &a)).step, None);
        // Even without the cancel, a tab gone unanswered aborts instead of advancing.
        assert_eq!(r.begin_quit(), Step::Ask(b.clone(), CloseKind::Quit));
        assert!(r.report(&b, false, Some(DocId(2)), None).is_ok());
        let gone = r.window_destroyed(&win(&r, &b));
        assert_eq!(gone.step, None);
        assert_eq!(r.asking(), None);
    }

    #[test]
    fn closing_tabs_and_windows_closes_their_documents_and_print_windows() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let a2 = tab(&mut r, &a, Some(&info(3, "A2")));
        let b = window(&mut r, Some(&info(2, "B")));
        r.add_print("tally-print-5", &a);
        r.add_print("tally-print-7", &a2);
        r.add_print("tally-print-6", &b);
        // A print window closing on its own changes nothing else.
        assert_eq!(r.window_destroyed("tally-print-6"), Gone::default());
        assert_eq!(r.len(), 3);
        // A tab takes its own print windows only.
        let gone = r.tab_closed(&a2);
        assert_eq!(gone.close_docs, [DocId(3)]);
        assert_eq!(gone.prints, ["tally-print-7"]);
        let gone = r.window_destroyed(&win(&r, &a));
        assert_eq!(gone.close_docs, [DocId(1)]);
        assert_eq!(gone.prints, ["tally-print-5"]);
        assert_eq!(gone.step, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r.window_destroyed("win-99"), Gone::default());
        assert_eq!(r.tab_closed("doc-99"), Gone::default());
    }

    #[test]
    fn an_edited_unanswered_tab_destroyed_on_the_way_out_is_snapshotted() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let c = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, false, Some(3));
        report(&mut r, &c, true, Some(2));
        r.confirm(&c);
        let gone = r.window_destroyed(&win(&r, &a));
        assert_eq!(
            gone.close_docs,
            [DocId(3)],
            "a is kept for the exit snapshot"
        );
        let plan = r.exit_plan();
        assert_eq!(plan.snapshot, [DocId(1)]);
        assert_eq!(plan.discard, [DocId(2)]);
    }

    #[test]
    fn exit_snapshots_every_edited_unanswered_document() {
        let mut r = Registry::default();
        let first = window(&mut r, None);
        report(&mut r, &first, true, Some(1));
        for (doc, edited) in [(2, false), (3, true)] {
            let l = tab(&mut r, &first, None);
            report(&mut r, &l, edited, Some(doc));
        }
        let blank = window(&mut r, None);
        report(&mut r, &blank, true, None);
        let plan = r.exit_plan();
        assert_eq!(plan.snapshot, [DocId(1), DocId(3)]);
        assert_eq!(plan.discard, [DocId(2)]);
    }

    #[test]
    fn exit_requests_follow_the_platform_rule() {
        assert_eq!(exit_action(Some(0), true, 2, 0), ExitAction::Proceed);
        assert_eq!(
            exit_action(Some(tauri::RESTART_EXIT_CODE), false, 1, 1),
            ExitAction::Proceed
        );
        // The last window closed: macOS keeps running, others exit.
        assert_eq!(exit_action(None, true, 0, 0), ExitAction::StayAlive);
        assert_eq!(exit_action(None, false, 0, 0), ExitAction::Proceed);
        assert_eq!(exit_action(None, true, 3, 0), ExitAction::Quit);
        assert_eq!(exit_action(None, false, 1, 0), ExitAction::Quit);
        // Something still being created keeps the app alive on every platform.
        assert_eq!(exit_action(None, false, 1, 1), ExitAction::StayAlive);
    }

    #[test]
    fn titles_edited_dots_and_the_document_list() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "Budget.ods")));
        let b = tab(&mut r, &a, None);
        let c = window(&mut r, None);
        assert_eq!(
            r.report(&c, false, Some(DocId(3)), Some("Report.xlsx")),
            Ok(true)
        );
        r.focus_changed(&win(&r, &c), true);
        r.set_untitled("Unbenannt");
        let rows: Vec<_> = r
            .window_list()
            .into_iter()
            .map(|e| (e.label, e.title, e.checked))
            .collect();
        assert_eq!(
            rows,
            [
                (a.clone(), "Budget.ods".to_owned(), false),
                (b.clone(), "Unbenannt".to_owned(), false),
                (c.clone(), "Report.xlsx".to_owned(), true),
            ]
        );
        let w = win(&r, &a);
        assert_eq!(r.window_title(&w), Some(("Unbenannt".to_owned(), false)));
        assert!(!r.window_edited(&w));
        // A hidden tab's edit marks the window (its red button closes it too).
        assert_eq!(r.report(&a, true, None, None), Ok(true));
        assert!(r.window_edited(&w));
        assert_eq!(r.window_title(&w), Some(("Unbenannt".to_owned(), false)));
        r.activate(&a);
        assert_eq!(r.window_title(&w), Some(("Budget.ods".to_owned(), true)));
        // A rename (Save As) is a change; an unchanged report is not.
        assert_eq!(r.report(&a, true, None, Some("Budget 2026.ods")), Ok(true));
        assert_eq!(r.report(&a, true, None, Some("Budget 2026.ods")), Ok(false));
        assert_eq!(r.report(&a, true, None, None), Ok(false));
        let json = serde_json::to_string(&r.tabs_state(&w)).unwrap_or_default();
        assert_eq!(
            json,
            r#"{"window":"win-1","tabs":[{"label":"doc-1","window":"win-1","title":"Budget 2026.ods","edited":true,"path":null},{"label":"doc-2","window":"win-1","title":"","edited":false,"path":null}],"active":"doc-1","others":[{"label":"doc-3","window":"win-2","title":"Report.xlsx","edited":false,"path":null}]}"#
        );
    }

    #[test]
    fn detach_is_one_checked_step() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        // A window's only tab stays.
        assert_eq!(r.begin_detach(&a), Ok(None));
        let b = tab(&mut r, &a, None);
        let w1 = win(&r, &a);
        let d = r.begin_detach(&b);
        assert!(matches!(&d, Ok(Some(Detach { from, .. })) if *from == w1));
        // A second move of the same tab while the first runs is refused.
        assert_eq!(r.begin_detach(&b), Err(Refused::Busy));
        // Merge waits too.
        assert_eq!(r.merge_into(&w1), Merge::default());
        r.end_detach(&b);
        assert!(matches!(r.begin_detach(&b), Ok(Some(_))));
        r.end_detach(&b);
        assert_eq!(r.begin_detach("doc-99"), Err(Refused::UnknownWindow));
        // Not while a close sequence is asking.
        report(&mut r, &a, true, Some(1));
        assert!(matches!(r.begin_quit(), Step::Ask(..)));
        assert_eq!(r.begin_detach(&b), Err(Refused::Busy));
        let other = window(&mut r, None);
        assert_eq!(r.merge_into(&win(&r, &other)), Merge::default());
    }

    #[test]
    fn windows_are_capped_too() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        // Racing detaches allocate (empty) windows: they count against the cap.
        for _ in 0..MAX_DOCS - 1 {
            assert!(r.alloc_window_label().is_some());
        }
        assert_eq!(r.begin_detach(&b), Err(Refused::TooManyWindows));
        assert_eq!(
            r.create(Target::NewWindow, None),
            Err(Refused::TooManyWindows)
        );
    }

    #[test]
    fn a_pending_question_is_asked_again() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let w = win(&r, &a);
        assert_eq!(r.reask(), None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        let first = r.begin_close_window(&w);
        assert_eq!(
            first,
            WindowClose::Hold(Step::Ask(b.clone(), CloseKind::Tabs))
        );
        // The red button again (or Close Others, or a reloaded webview): the same question.
        assert_eq!(r.begin_close_window(&w), WindowClose::Busy);
        assert_eq!(r.reask(), Some(Step::Ask(b.clone(), CloseKind::Tabs)));
        assert_eq!(r.begin_close_others(&a), None);
        assert!(r.cancel(&b));
        assert_eq!(r.reask(), None);
    }

    #[test]
    fn close_other_tabs_never_closes_a_tab_unasked() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = tab(&mut r, &a, None);
        let c = tab(&mut r, &a, None);
        report(&mut r, &b, true, Some(2));
        assert_eq!(
            r.begin_close_others(&a),
            Some(Step::Ask(b.clone(), CloseKind::Tabs))
        );
        assert_eq!(
            r.answered(&b),
            Some(Step::Done(Goal::CloseTabs(vec![b.clone(), c.clone()])))
        );
        // c was edited after the sequence finished asking: skipped, not closed.
        report(&mut r, &c, true, Some(3));
        assert_eq!(r.closable(&c), None);
        assert_eq!(r.closable(&b), Some(TabClose::Now));
        // The kept tab went meanwhile: closing b then empties a window holding only b.
        r.tab_closed(&a);
        r.report(&c, false, Some(DocId(3)), None).ok();
        r.tab_closed(&c);
        assert_eq!(r.closable(&b), Some(TabClose::Window(win(&r, &b))));
        assert_eq!(r.closable("doc-99"), None);
    }

    #[test]
    fn titles_lose_control_and_bidi_characters() {
        assert_eq!(clean_title("Budget\u{202E}sdo.exe"), "Budgetsdo.exe");
        assert_eq!(clean_title("A\u{2066}B\u{2069}\u{0007}\n\u{0085}C"), "ABC");
        assert_eq!(clean_title("Q&A — Ü.ods"), "Q&A — Ü.ods");
        let mut r = Registry::default();
        let mut i = info(1, "x\u{202D}y");
        i.title = "x\u{202D}y".into();
        let a = window(&mut r, Some(&i));
        assert_eq!(r.tab(&a).map(|t| t.title.clone()), Some("xy".to_owned()));
        assert_eq!(r.report(&a, false, None, Some("p\u{0000}q")), Ok(true));
        assert_eq!(r.tab(&a).map(|t| t.title.clone()), Some("pq".to_owned()));
        assert_eq!(r.report(&a, false, None, Some("pq")), Ok(false));
        assert!(r.bind(&a, &info(5, "z\u{2067}")).is_ok());
        assert_eq!(r.tab(&a).map(|t| t.title.clone()), Some("z".to_owned()));
    }
}
