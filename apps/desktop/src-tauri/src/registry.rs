//! Book-keeping of the document windows (one document per window): which
//! window shows which document, each window's unsaved-changes state, focus
//! order, the quit sequence, the print windows each document window owns,
//! and the window list of the Window menu.
//!
//! Everything here is plain data with no Tauri types, so the decisions are
//! unit-tested; `closing.rs`, `windows.rs` and `menu.rs` act on the results.
//! The managed [`Windows`] wraps it in a mutex that is a leaf lock: it is
//! held only for these in-memory updates and never across an engine call, a
//! dialog, a window or menu call or an emit, so the main thread (window and
//! exit events) never waits for the engine or for itself.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tally_api::{DocId, WorkbookInfo};

/// Label prefix of document windows (`doc-1`, `doc-2`, …).
pub const DOC_PREFIX: &str = "doc-";
/// Label prefix of print windows (`print.rs`).
pub const PRINT_PREFIX: &str = "tally-print-";

/// Whether `label` names a document window.
pub fn is_doc_label(label: &str) -> bool {
    label
        .strip_prefix(DOC_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Most document windows open at once ([`Registry::create`] refuses more).
pub const MAX_WINDOWS: usize = 100;

/// One document window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocWindow {
    /// Creation order (window list order).
    pub seq: u32,
    /// The document it shows, once known.
    pub doc: Option<DocId>,
    /// Unsaved changes (the webview's `set_document_edited`).
    pub edited: bool,
    /// The user answered Save / Don't Save: the next close or exit goes through.
    pub confirmed: bool,
    /// The document's title (Window menu).
    pub title: String,
    /// Tauri has built the window (until then an exit must not end the app).
    pub built: bool,
}

/// Where a newly created or opened document is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In the calling window, replacing its untouched untitled document.
    Here,
    /// In a new window.
    NewWindow,
}

/// The next step of a quit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuitStep {
    /// Ask this window about its unsaved changes (focus it first).
    Ask(String),
    /// Nothing is left to ask: exit.
    Exit,
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
/// A window still being built (`pending`) keeps the app alive (the runtime
/// asks to exit when its last *built* window closes). With no document
/// window left, macOS keeps running (`keep_alive`) and the other platforms
/// exit; with windows, it becomes a guarded quit.
pub fn exit_action(
    code: Option<i32>,
    keep_alive: bool,
    doc_windows: usize,
    pending: usize,
) -> ExitAction {
    match code {
        Some(_) => ExitAction::Proceed,
        None if pending > 0 => ExitAction::StayAlive,
        None if doc_windows == 0 && keep_alive => ExitAction::StayAlive,
        None if doc_windows == 0 => ExitAction::Proceed,
        None => ExitAction::Quit,
    }
}

/// What the shell does after a document window is destroyed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Destroyed {
    /// Close this document in the engine (and drop its autosave snapshot).
    pub close_doc: Option<DocId>,
    /// Print windows the destroyed window owned (destroy them).
    pub prints: Vec<String>,
    /// The quit sequence was asking this window: continue with this step.
    pub quit: Option<QuitStep>,
}

/// What the final exit does with each document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitPlan {
    /// Edited and unanswered: write a last autosave snapshot.
    pub snapshot: Vec<DocId>,
    /// Clean or answered: remove this session's autosave snapshot.
    pub discard: Vec<DocId>,
}

/// One row of the Window menu's window list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    pub label: String,
    pub title: String,
    pub checked: bool,
}

/// Why the registry refused something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a known document window.
    UnknownWindow,
    /// The document is shown in this other window.
    ShownIn(String),
    /// [`MAX_WINDOWS`] are open.
    TooManyWindows,
}

impl Refused {
    /// The error the commands return.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownWindow => "not a document window".to_owned(),
            Self::ShownIn(_) => "the document is already shown in another window".to_owned(),
            Self::TooManyWindows => format!("at most {MAX_WINDOWS} windows can be open"),
        }
    }
}

/// The document windows.
#[derive(Debug)]
pub struct Registry {
    windows: BTreeMap<String, DocWindow>,
    /// Most recently focused first.
    mru: Vec<String>,
    next: u32,
    /// The window the quit sequence is asking.
    quitting: Option<String>,
    recovery_offered: bool,
    /// Print window → owning document window.
    prints: BTreeMap<String, String>,
    /// The key window when it is a print window (⌘W closes it).
    key_print: Option<String>,
    /// Documents of windows destroyed while edited and unanswered (only on
    /// the way out): still snapshotted at exit.
    orphans: Vec<DocId>,
    /// "Untitled" in the UI language (Window menu list).
    untitled: String,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            windows: BTreeMap::new(),
            mru: Vec::new(),
            next: 0,
            quitting: None,
            recovery_offered: false,
            prints: BTreeMap::new(),
            key_print: None,
            orphans: Vec::new(),
            untitled: "Untitled".to_owned(),
        }
    }
}

impl Registry {
    /// Allocates the label of a new document window and registers it with
    /// the document it will show (`None`: the webview creates a blank one).
    /// A recovered (dirty) document counts as edited from the start, so it
    /// is asked about and snapshotted even before its webview reports.
    pub fn create(&mut self, doc: Option<&WorkbookInfo>) -> Result<String, Refused> {
        if self.windows.len() >= MAX_WINDOWS {
            return Err(Refused::TooManyWindows);
        }
        if let Some(other) = doc.and_then(|i| self.window_of(i.doc)) {
            return Err(Refused::ShownIn(other));
        }
        self.next += 1;
        let label = format!("{DOC_PREFIX}{}", self.next);
        let entry = DocWindow {
            seq: self.next,
            doc: doc.map(|i| i.doc),
            edited: doc.is_some_and(|i| i.dirty),
            title: doc.map(|i| i.title.clone()).unwrap_or_default(),
            ..DocWindow::default()
        };
        self.windows.insert(label.clone(), entry);
        Ok(label)
    }

    /// Tauri built the window of `label`.
    pub fn built(&mut self, label: &str) {
        if let Some(w) = self.windows.get_mut(label) {
            w.built = true;
        }
    }

    /// Windows registered but not built yet.
    pub fn pending(&self) -> usize {
        self.windows.values().filter(|w| !w.built).count()
    }

    /// Forgets a window whose creation failed.
    pub fn forget(&mut self, label: &str) -> Option<DocWindow> {
        self.mru.retain(|l| l != label);
        self.windows.remove(label)
    }

    /// A window's state.
    #[cfg(test)]
    pub fn get(&self, label: &str) -> Option<&DocWindow> {
        self.windows.get(label)
    }

    /// Number of document windows.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether there is no document window.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Sets the Window menu's name for untitled windows (UI language).
    pub fn set_untitled(&mut self, untitled: &str) {
        untitled.clone_into(&mut self.untitled);
    }

    /// `set_document_edited`: records a known window's state. A document
    /// shown in another window is refused. Any report after the user
    /// answered during a quit re-arms the guard when it says "edited" (the
    /// webview reports again after `confirm_close(true)`). Returns whether
    /// the Window menu's list changed (a new title).
    pub fn report(
        &mut self,
        label: &str,
        edited: bool,
        doc: Option<DocId>,
        title: Option<&str>,
    ) -> Result<bool, Refused> {
        if let Some(other) = doc.and_then(|d| self.window_of(d)) {
            if other != label {
                return Err(Refused::ShownIn(other));
            }
        }
        let w = self.windows.get_mut(label).ok_or(Refused::UnknownWindow)?;
        if edited && w.confirmed && self.quitting.as_deref() != Some(label) {
            w.confirmed = false;
        }
        w.edited = edited;
        if doc.is_some() {
            w.doc = doc;
        }
        Ok(match title {
            Some(t) if t != w.title => {
                w.title = t.to_owned();
                true
            }
            _ => false,
        })
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

    /// The most recently focused window that still exists (menu actions go
    /// there), else the first one created. Includes a window still being
    /// built, so a second File ▸ New / Dock click does not open another.
    pub fn front(&self) -> Option<String> {
        self.mru
            .iter()
            .find(|l| self.windows.contains_key(*l))
            .cloned()
            .or_else(|| self.by_seq().first().map(|(l, _)| (*l).clone()))
    }

    fn by_seq(&self) -> Vec<(&String, &DocWindow)> {
        let mut all: Vec<_> = self.windows.iter().collect();
        all.sort_by_key(|(_, w)| w.seq);
        all
    }

    /// The window showing `doc`.
    pub fn window_of(&self, doc: DocId) -> Option<String> {
        self.windows
            .iter()
            .find(|(_, w)| w.doc == Some(doc))
            .map(|(l, _)| l.clone())
    }

    /// Pure placement: a document is shown in the calling window only when
    /// the webview asked for it (its document is untitled and untouched)
    /// and the shell agrees (the window exists and reported no unsaved
    /// changes); otherwise it gets a new window. Refused when the caller is
    /// not a known document window or the document is shown in another one.
    pub fn place(&self, caller: &str, doc: DocId, reuse: bool) -> Result<Placement, Refused> {
        let w = self.windows.get(caller).ok_or(Refused::UnknownWindow)?;
        if let Some(other) = self.window_of(doc) {
            if other != caller {
                return Err(Refused::ShownIn(other));
            }
        }
        Ok(if reuse && !w.edited {
            Placement::Here
        } else {
            Placement::NewWindow
        })
    }

    /// Shows `info` in `label` (a [`Placement::Here`]); refused like [`Self::place`].
    pub fn bind(&mut self, label: &str, info: &WorkbookInfo) -> Result<(), Refused> {
        if let Some(other) = self.window_of(info.doc) {
            if other != label {
                return Err(Refused::ShownIn(other));
            }
        }
        let w = self.windows.get_mut(label).ok_or(Refused::UnknownWindow)?;
        w.doc = Some(info.doc);
        info.title.clone_into(&mut w.title);
        Ok(())
    }

    /// `window_init`: the document the window shows (also after a reload of
    /// its webview) and whether this is the first window to start, which
    /// offers the crash recovery.
    pub fn init(&mut self, label: &str) -> Result<(Option<DocId>, bool), Refused> {
        let doc = self.windows.get(label).ok_or(Refused::UnknownWindow)?.doc;
        let offer = !self.recovery_offered;
        self.recovery_offered = true;
        Ok((doc, offer))
    }

    /// Whether closing `label` must ask first (edited and not yet answered).
    pub fn must_ask(&self, label: &str) -> bool {
        self.windows
            .get(label)
            .is_some_and(|w| w.edited && !w.confirmed)
    }

    /// The user answered Save / Don't Save in `label` (closing it).
    pub fn confirm(&mut self, label: &str) {
        if let Some(w) = self.windows.get_mut(label) {
            w.confirmed = true;
        }
    }

    /// Windows in quit order: most recently focused first, then by creation.
    fn quit_order(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .mru
            .iter()
            .filter(|l| self.windows.contains_key(*l))
            .cloned()
            .collect();
        for (l, _) in self.by_seq() {
            if !out.contains(l) {
                out.push(l.clone());
            }
        }
        out
    }

    fn next_to_ask(&self) -> Option<String> {
        self.quit_order().into_iter().find(|l| self.must_ask(l))
    }

    /// The window a running quit sequence is asking.
    #[cfg(test)]
    pub fn quitting(&self) -> Option<&str> {
        self.quitting.as_deref()
    }

    /// Quit (⌘Q, app menu Quit, an exit request): asks the edited windows
    /// one at a time, the front one first. A quit already in progress asks
    /// its current window again.
    pub fn begin_quit(&mut self) -> QuitStep {
        if let Some(l) = &self.quitting {
            if self.must_ask(l) {
                return QuitStep::Ask(l.clone());
            }
        }
        self.advance()
    }

    fn advance(&mut self) -> QuitStep {
        self.quitting = self.next_to_ask();
        match &self.quitting {
            Some(l) => QuitStep::Ask(l.clone()),
            None => QuitStep::Exit,
        }
    }

    /// `label` answered Save / Don't Save during a quit: the next window, or
    /// exit. `None` (ignored) unless the quit is asking `label`.
    pub fn quit_answered(&mut self, label: &str) -> Option<QuitStep> {
        if self.quitting.as_deref() != Some(label) {
            return None;
        }
        self.confirm(label);
        Some(self.advance())
    }

    /// Cancel (or a cancelled Save As) in the window the quit is asking
    /// aborts the whole quit: earlier answers no longer count, so those
    /// windows ask again next time. Ignored from any other window.
    pub fn cancel_quit(&mut self, label: &str) -> bool {
        if self.quitting.as_deref() != Some(label) {
            return false;
        }
        self.abort_quit();
        true
    }

    fn abort_quit(&mut self) {
        self.quitting = None;
        for w in self.windows.values_mut() {
            w.confirmed = false;
        }
    }

    /// A print window opened by `owner`.
    pub fn add_print(&mut self, print: &str, owner: &str) {
        self.prints.insert(print.to_owned(), owner.to_owned());
    }

    /// A window was destroyed. If the quit was asking it, the quit goes on
    /// only when the user had answered there (Save / Don't Save closed the
    /// window); a window gone without an answer aborts the quit.
    pub fn destroyed(&mut self, label: &str) -> Destroyed {
        if self.key_print.as_deref() == Some(label) {
            self.key_print = None;
        }
        if self.prints.remove(label).is_some() || !self.windows.contains_key(label) {
            return Destroyed::default();
        }
        let mut out = Destroyed::default();
        self.mru.retain(|l| l != label);
        let mut answered = false;
        if let Some(w) = self.windows.remove(label) {
            answered = w.confirmed;
            if w.edited && !w.confirmed {
                // Only on the way out (closing is guarded): keep the
                // document for the exit snapshot.
                self.orphans.extend(w.doc);
            } else {
                out.close_doc = w.doc;
            }
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
        out.prints = owned;
        if self.quitting.as_deref() == Some(label) {
            if answered {
                out.quit = Some(self.advance());
            } else {
                self.abort_quit();
            }
        }
        out
    }

    /// What the final exit writes or removes.
    pub fn exit_plan(&self) -> ExitPlan {
        let mut plan = ExitPlan::default();
        for w in self.windows.values() {
            let Some(doc) = w.doc else { continue };
            if w.edited && !w.confirmed {
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

    /// The Window menu's list: every document window by creation order,
    /// titled "Untitled" (UI language) when it has no title yet, the front
    /// one checked.
    pub fn window_list(&self) -> Vec<ListEntry> {
        let front = self.front();
        self.by_seq()
            .into_iter()
            .map(|(l, w)| ListEntry {
                label: l.clone(),
                title: if w.title.is_empty() {
                    self.untitled.clone()
                } else {
                    w.title.clone()
                },
                checked: front.as_deref() == Some(l.as_str()),
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

    /// A registered, built window.
    fn window(r: &mut Registry, doc: Option<&WorkbookInfo>) -> String {
        let l = r.create(doc).unwrap_or_default();
        r.built(&l);
        l
    }

    fn report(r: &mut Registry, l: &str, edited: bool, doc: Option<u32>) {
        assert_eq!(
            r.report(l, edited, doc.map(DocId), None).map(|_| ()),
            Ok(())
        );
    }

    #[test]
    fn labels_are_unique_and_recognised() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, Some(&info(4, "B.ods")));
        assert_eq!((a.as_str(), b.as_str()), ("doc-1", "doc-2"));
        r.forget(&a);
        assert_eq!(window(&mut r, None), "doc-3", "labels are never reused");
        assert!(is_doc_label("doc-12"));
        for l in ["main", "doc-", "doc-x", "tally-print-3", "doc-1a"] {
            assert!(!is_doc_label(l), "{l}");
        }
        assert_eq!(r.window_of(DocId(4)), Some(b.clone()));
        // A document gets one window only.
        assert_eq!(r.create(Some(&info(4, "B.ods"))), Err(Refused::ShownIn(b)));
    }

    #[test]
    fn the_number_of_windows_is_capped() {
        let mut r = Registry::default();
        for _ in 0..MAX_WINDOWS {
            assert!(r.create(None).is_ok());
        }
        assert_eq!(r.create(None), Err(Refused::TooManyWindows));
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
            r.place("doc-9", DocId(1), true),
            Err(Refused::UnknownWindow)
        );
        assert_eq!(r.bind("doc-9", &info(1, "x")), Err(Refused::UnknownWindow));
        r.confirm("doc-9");
        assert!(!r.focus_changed("doc-9", true));
        assert!(r.is_empty(), "nothing was registered on the way");
        assert!(r.get("doc-9").is_none());
    }

    #[test]
    fn init_returns_the_bound_document_every_time_and_offers_recovery_once() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, Some(&info(7, "Book.ods")));
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
        assert_eq!(r.begin_quit(), QuitStep::Ask(a));
    }

    #[test]
    fn reuse_only_an_untouched_window_that_asked_for_it() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        report(&mut r, &a, false, Some(1));
        assert_eq!(r.place(&a, DocId(2), true), Ok(Placement::Here));
        assert_eq!(r.place(&a, DocId(2), false), Ok(Placement::NewWindow));
        // The shell's own record vetoes reuse of an edited window.
        report(&mut r, &a, true, Some(1));
        assert_eq!(r.place(&a, DocId(2), true), Ok(Placement::NewWindow));
        // A document already shown elsewhere is refused.
        let b = window(&mut r, Some(&info(9, "Nine")));
        assert_eq!(
            r.place(&a, DocId(9), true),
            Err(Refused::ShownIn(b.clone()))
        );
        assert!(r.bind(&a, &info(2, "Two.ods")).is_ok());
        assert_eq!(r.get(&a).map(|w| w.title.as_str()), Some("Two.ods"));
        assert_eq!(r.window_of(DocId(2)), Some(a.clone()));
        // Neither a report nor a bind can take another window's document.
        assert_eq!(
            r.report(&a, false, Some(DocId(9)), None),
            Err(Refused::ShownIn(b.clone()))
        );
        assert_eq!(r.bind(&a, &info(9, "Nine")), Err(Refused::ShownIn(b)));
        assert_eq!(r.window_of(DocId(2)), Some(a));
    }

    #[test]
    fn menu_actions_go_to_the_front_window() {
        let mut r = Registry::default();
        assert_eq!(r.front(), None);
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        assert_eq!(r.front(), Some(a.clone()), "never focused: first created");
        assert!(r.focus_changed(&b, true));
        assert!(!r.focus_changed(&b, true));
        assert_eq!(r.front(), Some(b.clone()));
        r.focus_changed(&a, true);
        r.destroyed(&a);
        assert_eq!(r.front(), Some(b));
    }

    #[test]
    fn a_key_print_window_is_tracked() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        r.add_print("tally-print-3", &a);
        r.focus_changed(&a, true);
        assert_eq!(r.key_print(), None);
        assert!(!r.focus_changed("tally-print-3", true));
        assert_eq!(r.key_print(), Some("tally-print-3"));
        assert_eq!(
            r.front(),
            Some(a.clone()),
            "the owner stays the front document window"
        );
        r.focus_changed("tally-print-3", false);
        assert_eq!(r.key_print(), None);
        r.focus_changed("tally-print-3", true);
        r.focus_changed(&a, true);
        assert_eq!(r.key_print(), None);
        r.focus_changed("tally-print-3", true);
        r.destroyed("tally-print-3");
        assert_eq!(r.key_print(), None);
    }

    #[test]
    fn a_window_being_built_dedupes_new_windows_and_blocks_the_exit() {
        let mut r = Registry::default();
        let a = r.create(None).unwrap_or_default();
        assert_eq!(r.pending(), 1);
        // A second File ▸ New / Dock click sees a front window and opens nothing.
        assert_eq!(r.front(), Some(a.clone()));
        assert_eq!(
            exit_action(None, false, r.len(), r.pending()),
            ExitAction::StayAlive
        );
        r.built(&a);
        assert_eq!(r.pending(), 0);
        assert_eq!(
            exit_action(None, false, r.len(), r.pending()),
            ExitAction::Quit
        );
    }

    #[test]
    fn quit_asks_each_edited_window_in_turn() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        let c = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, false, Some(2));
        report(&mut r, &c, true, Some(3));
        r.focus_changed(&c, true);
        // The front window first; clean windows are never asked.
        assert_eq!(r.begin_quit(), QuitStep::Ask(c.clone()));
        // ⌘Q again while asking: the same window again.
        assert_eq!(r.begin_quit(), QuitStep::Ask(c.clone()));
        // Only the asked window can answer.
        assert_eq!(r.quit_answered(&a), None);
        assert!(!r.must_ask(&c) || r.quitting() == Some(c.as_str()));
        assert_eq!(r.quit_answered(&c), Some(QuitStep::Ask(a.clone())));
        assert_eq!(r.quitting(), Some(a.as_str()));
        assert_eq!(r.quit_answered(&a), Some(QuitStep::Exit));
        assert_eq!(r.quitting(), None);
        let plan = r.exit_plan();
        assert!(plan.snapshot.is_empty());
        assert_eq!(plan.discard, [DocId(1), DocId(2), DocId(3)]);
    }

    #[test]
    fn cancel_in_the_asked_window_aborts_the_whole_quit() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), QuitStep::Ask(a.clone()));
        // a: Don't Save; b: Cancel. A cancel from a window not being asked is ignored.
        assert_eq!(r.quit_answered(&a), Some(QuitStep::Ask(b.clone())));
        assert!(!r.cancel_quit(&a));
        assert_eq!(r.quitting(), Some(b.as_str()));
        assert!(r.cancel_quit(&b));
        assert_eq!(r.quitting(), None);
        // a's answer no longer counts: closing it asks again.
        assert!(r.must_ask(&a) && r.must_ask(&b));
        assert_eq!(r.begin_quit(), QuitStep::Ask(a));
    }

    #[test]
    fn an_edit_after_answering_re_arms_the_guard() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), QuitStep::Ask(a.clone()));
        assert_eq!(r.quit_answered(&a), Some(QuitStep::Ask(b.clone())));
        assert!(!r.must_ask(&a));
        // While b is asked, a is edited again (its webview reports anew).
        report(&mut r, &a, true, Some(1));
        assert!(r.must_ask(&a));
        // b answers: a is asked again instead of being dropped at exit.
        assert_eq!(r.quit_answered(&b), Some(QuitStep::Ask(a)));
    }

    #[test]
    fn quit_with_nothing_edited_exits_at_once() {
        let mut r = Registry::default();
        assert_eq!(r.begin_quit(), QuitStep::Exit);
        let a = window(&mut r, None);
        report(&mut r, &a, false, Some(1));
        assert_eq!(r.begin_quit(), QuitStep::Exit);
    }

    #[test]
    fn a_window_saved_meanwhile_is_skipped() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, None);
        report(&mut r, &b, true, None);
        assert_eq!(r.begin_quit(), QuitStep::Ask(a.clone()));
        report(&mut r, &b, false, None);
        assert_eq!(r.quit_answered(&a), Some(QuitStep::Exit));
    }

    #[test]
    fn closing_the_asked_window_with_an_answer_continues_the_quit() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), QuitStep::Ask(a.clone()));
        // a had its own window prompt up: Don't Save there closes it (confirm + destroy).
        r.confirm(&a);
        let d = r.destroyed(&a);
        assert_eq!(d.close_doc, Some(DocId(1)));
        assert_eq!(d.quit, Some(QuitStep::Ask(b)));
    }

    #[test]
    fn a_stale_quit_never_exits_when_the_asked_window_closes() {
        // ⌘Q while a's window prompt was up; the user cancelled that prompt
        // (the webview then calls cancel_quit) and later closed a normally.
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        assert_eq!(r.begin_quit(), QuitStep::Ask(a.clone()));
        assert!(r.cancel_quit(&a));
        report(&mut r, &a, false, Some(1));
        assert_eq!(r.destroyed(&a).quit, None);
        // Even without the cancel, a window gone unanswered aborts instead of advancing.
        assert_eq!(r.begin_quit(), QuitStep::Ask(b.clone()));
        r.report(&b, false, Some(DocId(2)), None).ok();
        let d = r.destroyed(&b);
        assert_eq!(d.quit, None);
        assert_eq!(r.quitting(), None);
    }

    #[test]
    fn closing_a_window_closes_its_document_and_print_windows() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "A")));
        let b = window(&mut r, Some(&info(2, "B")));
        r.add_print("tally-print-5", &a);
        r.add_print("tally-print-6", &b);
        // A print window closing on its own changes nothing else.
        assert_eq!(r.destroyed("tally-print-6"), Destroyed::default());
        assert_eq!(r.len(), 2);
        let d = r.destroyed(&a);
        assert_eq!(d.close_doc, Some(DocId(1)));
        assert_eq!(d.prints, ["tally-print-5"]);
        assert_eq!(d.quit, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r.destroyed("doc-99"), Destroyed::default());
    }

    #[test]
    fn an_edited_unanswered_window_destroyed_on_the_way_out_is_snapshotted() {
        let mut r = Registry::default();
        let a = window(&mut r, None);
        let b = window(&mut r, None);
        report(&mut r, &a, true, Some(1));
        report(&mut r, &b, true, Some(2));
        r.confirm(&b);
        let d = r.destroyed(&a);
        assert_eq!(d.close_doc, None, "kept for the exit snapshot");
        let plan = r.exit_plan();
        assert_eq!(plan.snapshot, [DocId(1)]);
        assert_eq!(plan.discard, [DocId(2)]);
    }

    #[test]
    fn exit_snapshots_every_edited_unanswered_document() {
        let mut r = Registry::default();
        for (doc, edited) in [(1, true), (2, false), (3, true)] {
            let l = window(&mut r, None);
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
        // A window still being created keeps the app alive on every platform.
        assert_eq!(exit_action(None, false, 1, 1), ExitAction::StayAlive);
    }

    #[test]
    fn window_list_is_in_creation_order_with_the_front_one_checked() {
        let mut r = Registry::default();
        let a = window(&mut r, Some(&info(1, "Budget.ods")));
        let b = window(&mut r, None);
        let c = window(&mut r, None);
        assert_eq!(
            r.report(&c, false, Some(DocId(3)), Some("Report.xlsx")),
            Ok(true)
        );
        r.focus_changed(&c, true);
        r.set_untitled("Unbenannt");
        let list = r.window_list();
        let rows: Vec<_> = list
            .iter()
            .map(|e| (e.label.as_str(), e.title.as_str(), e.checked))
            .collect();
        assert_eq!(
            rows,
            [
                (a.as_str(), "Budget.ods", false),
                (b.as_str(), "Unbenannt", false),
                (c.as_str(), "Report.xlsx", true),
            ]
        );
        // A rename (Save As) changes the list; an unchanged title does not.
        assert_eq!(r.report(&a, false, None, Some("Budget 2026.ods")), Ok(true));
        assert_eq!(
            r.report(&a, false, None, Some("Budget 2026.ods")),
            Ok(false)
        );
        assert_eq!(r.report(&a, true, None, None), Ok(false));
    }
}
