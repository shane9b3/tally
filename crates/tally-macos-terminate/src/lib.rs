//! macOS: lets the app veto `-[NSApp terminate:]`.
//!
//! The Dock's Quit, `quit app "Tally"`, log-out, restart and shutdown all
//! send `-[NSApplication terminate:]`. AppKit then asks the app delegate's
//! `applicationShouldTerminate:`; tao's delegate (Tauri's windowing layer)
//! does not implement it, so the app quits at once, past the
//! unsaved-changes prompt. [`install`] adds that one method to the class of
//! the delegate tao installed. It asks a handler the app registers:
//!
//! - nothing to ask ([`Answer::Now`]): the app terminates at once;
//! - questions to ask ([`Answer::Later`]): AppKit waits (`NSTerminateLater`;
//!   the run loop keeps going in its modal-panel mode, so windows, webviews
//!   and Tauri's event loop keep working) until the app calls [`reply`]:
//!   `true` lets the termination (or the log-out) go on, `false` cancels it
//!   (at log-out macOS then says "Tally interrupted logging out").
//!
//! Once the app has decided to quit ([`approve`], or a `reply(true)`), any
//! later `terminate:` goes through without asking again. A `terminate:`
//! arriving while one is already waiting asks the handler again (to bring
//! the open question forward) and is itself answered "cancel"; the waiting
//! one still gets the reply. Escape hatch: if that one has waited
//! [`ESCAPE_AFTER`] or more, or on the second such request, the app
//! terminates at once (a hung webview must not leave Force Quit, which
//! skips the exit snapshot, as the only way out).
//!
//! The decision logic ([`Gate`]) is plain Rust and tested on every
//! platform. All `unsafe` is in the macOS-only `appkit` module. On the
//! other platforms [`install`] does nothing and [`reply`] reports that no
//! request is waiting.

use std::io::Write;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod appkit;

/// What the app's handler says about a termination request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Nothing to ask: terminate now.
    Now,
    /// Questions are being asked; the app calls [`reply`] when they are answered.
    Later,
}

/// Why [`install`] did nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Not called on the main thread.
    NotMainThread,
    /// `NSApp` has no delegate yet (call it once the event loop runs).
    NoDelegate,
    /// [`install`] was already called.
    AlreadyInstalled,
    /// The Objective-C runtime refused to add the method.
    Refused,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMainThread => "not on the main thread",
            Self::NoDelegate => "NSApp has no delegate",
            Self::AlreadyInstalled => "already installed",
            Self::Refused => "the Objective-C runtime refused the method",
        })
    }
}

impl std::error::Error for InstallError {}

/// What `applicationShouldTerminate:` answers AppKit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// `NSTerminateNow`.
    Now,
    /// `NSTerminateLater`: a [`reply`] follows.
    Later,
    /// `NSTerminateCancel`.
    Cancel,
}

/// How long a request may wait for its reply before another `terminate:`
/// (the Dock's Quit again) quits anyway ([`Request::Escape`]).
pub const ESCAPE_AFTER: Duration = Duration::from_secs(5);

/// What [`Gate::request`] wants done before answering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// The quit was already approved: answer `Now`, do not ask.
    Approved,
    /// Ask the handler, then pass its answer to [`Gate::answered`].
    /// `nested`: another request is already waiting for its reply.
    Ask { nested: bool },
    /// The escape hatch: a request has waited [`ESCAPE_AFTER`] or more (or
    /// this is the second request while it waits), so the questions are
    /// presumably stuck (a hung or crashed webview). Answer `Now` without
    /// asking: the exit snapshot keeps the unanswered documents for recovery.
    Escape,
}

/// The termination state machine (no AppKit; one global instance behind a mutex).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// The app decided to quit: every request goes through.
    approved: bool,
    /// Since when a request answered `NSTerminateLater` waits for its reply.
    pending_since: Option<Instant>,
    /// Requests that arrived while that one waits.
    nested: u32,
}

impl Gate {
    /// `applicationShouldTerminate:` was called at `now`.
    pub fn request(&mut self, now: Instant) -> Request {
        if self.approved {
            return Request::Approved;
        }
        if let Some(since) = self.pending_since {
            self.nested += 1;
            if self.nested >= 2 || now.saturating_duration_since(since) >= ESCAPE_AFTER {
                self.finish(true);
                return Request::Escape;
            }
            return Request::Ask { nested: true };
        }
        // Set before the handler runs, so a reply it triggers always finds it.
        self.pending_since = Some(now);
        self.nested = 0;
        Request::Ask { nested: false }
    }

    /// The handler answered a request [`Self::request`] asked about.
    pub fn answered(&mut self, nested: bool, answer: Answer) -> Decision {
        match (answer, nested) {
            (Answer::Now, _) => {
                // Nothing left to ask (for a nested request too: terminating
                // now ends the waiting one with it).
                self.finish(true);
                Decision::Now
            }
            (Answer::Later, false) => Decision::Later,
            // The waiting request gets the reply; this one stops here.
            (Answer::Later, true) => Decision::Cancel,
        }
    }

    /// The handler failed (panicked): fall back to terminating, as without the guard.
    pub fn failed(&mut self) -> Decision {
        self.finish(true);
        Decision::Now
    }

    fn finish(&mut self, approved: bool) {
        self.pending_since = None;
        self.nested = 0;
        self.approved |= approved;
    }

    /// The app's verdict. Returns whether a request was waiting (then
    /// `-replyToApplicationShouldTerminate:` must be sent); a second reply
    /// is ignored.
    pub fn reply(&mut self, should_terminate: bool) -> bool {
        if self.pending_since.is_none() {
            return false;
        }
        self.finish(should_terminate);
        true
    }

    /// The app is quitting on its own (its Quit menu item): a `terminate:`
    /// arriving meanwhile (log-out) goes through.
    pub fn approve(&mut self) {
        self.approved = true;
    }

    /// Whether a request waits for [`reply`].
    pub fn pending(&self) -> bool {
        self.pending_since.is_some()
    }

    /// Whether the quit was approved.
    pub fn approved(&self) -> bool {
        self.approved
    }
}

type Handler = Box<dyn Fn() -> Answer + Send + Sync>;

static GATE: Mutex<Gate> = Mutex::new(Gate {
    approved: false,
    pending_since: None,
    nested: 0,
});
static HANDLER: OnceLock<Handler> = OnceLock::new();

fn gate() -> MutexGuard<'static, Gate> {
    // The gate is plain flags, consistent after any panic: recover the guard.
    GATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// A line on stderr that never panics (`eprintln!` does on a closed pipe;
/// this runs inside `-[NSApplication terminate:]`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr(), "{args}");
}

/// Runs one `applicationShouldTerminate:` through the gate and the handler,
/// never unwinding: any panic (the handler's or ours) means "terminate now",
/// as without the guard.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn decide() -> Decision {
    // AssertUnwindSafe: after a panic nothing is used again except `GATE`,
    // which is recovered and set consistently by `failed`.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(decide_inner)).unwrap_or_else(|_| {
        log(format_args!(
            "tally: the quit handler panicked; quitting without asking"
        ));
        gate().failed()
    })
}

/// The gate lock is not held while the handler runs (it may call [`is_pending`]).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn decide_inner() -> Decision {
    let nested = match gate().request(Instant::now()) {
        Request::Approved => return Decision::Now,
        Request::Escape => {
            log(format_args!(
                "tally: Quit again while the save questions wait: quitting; unsaved documents are kept for recovery"
            ));
            return Decision::Now;
        }
        Request::Ask { nested } => nested,
    };
    let answer = HANDLER.get().map_or(Answer::Now, |h| h());
    let decision = gate().answered(nested, answer);
    if cfg!(debug_assertions) {
        log(format_args!(
            "tally(debug): applicationShouldTerminate: nested={nested} -> {decision:?}"
        ));
    }
    decision
}

/// Main thread, once the app's event loop runs (`NSApp` has tao's
/// delegate): adds `applicationShouldTerminate:` to the delegate's class.
/// `handler` runs on the main thread inside `terminate:`; it must not block
/// (start the questions and return [`Answer::Later`]). No-op elsewhere.
pub fn install(handler: impl Fn() -> Answer + Send + Sync + 'static) -> Result<(), InstallError> {
    #[cfg(target_os = "macos")]
    {
        if HANDLER.get().is_some() {
            return Err(InstallError::AlreadyInstalled);
        }
        // The method first, so a failed install (off the main thread, no
        // delegate yet) can be retried. It cannot run before the handler is
        // set: AppKit calls it on the main thread, where we are now.
        appkit::add_should_terminate()?;
        HANDLER
            .set(Box::new(handler))
            .map_err(|_| InstallError::AlreadyInstalled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = handler;
        Ok(())
    }
}

/// Whether a termination request waits for [`reply`] (any thread).
pub fn is_pending() -> bool {
    gate().pending()
}

/// The app is quitting by itself: later `terminate:` requests go through.
pub fn approve() {
    gate().approve();
}

/// Any thread: answers the waiting request (`true`: terminate, which runs
/// `applicationWillTerminate:`, i.e. Tauri's `RunEvent::Exit`; `false`:
/// stay). Returns whether a request was waiting (the reply is then queued
/// on the main run loop (common modes), never sent inside tao's event
/// callback); `false` when none was, and on other platforms.
pub fn reply(should_terminate: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        appkit::reply(should_terminate)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = should_terminate;
        false
    }
}

/// Any thread: sends `-[NSApp terminate:nil]`, queued on the main run loop
/// (common modes), as the Dock's Quit does (manual testing). `false` on
/// other platforms.
pub fn terminate() -> bool {
    #[cfg(target_os = "macos")]
    {
        appkit::terminate()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed clock: `at(s)` is `s` seconds after a common start.
    fn at(secs: u64) -> Instant {
        static START: OnceLock<Instant> = OnceLock::new();
        *START.get_or_init(Instant::now) + Duration::from_secs(secs)
    }

    #[test]
    fn quitting_again_after_the_wait_limit_quits_anyway() {
        // The asked webview hung: the user quits from the Dock again.
        let mut g = Gate::default();
        assert_eq!(g.request(at(0)), Request::Ask { nested: false });
        assert_eq!(g.answered(false, Answer::Later), Decision::Later);
        assert_eq!(g.request(at(4)), Request::Ask { nested: true });
        assert_eq!(g.answered(true, Answer::Later), Decision::Cancel);
        let mut late = Gate::default();
        late.request(at(0));
        late.answered(false, Answer::Later);
        assert_eq!(late.request(at(5)), Request::Escape);
        assert!(!late.pending());
        assert!(late.approved());
        // The waiting request is not replied to afterwards (the app is gone).
        assert!(!late.reply(false));
    }

    #[test]
    fn the_second_request_while_one_waits_quits_anyway() {
        let mut g = Gate::default();
        g.request(at(0));
        g.answered(false, Answer::Later);
        assert_eq!(g.request(at(1)), Request::Ask { nested: true });
        g.answered(true, Answer::Later);
        assert_eq!(g.request(at(2)), Request::Escape);
        assert!(!g.pending());
    }

    #[test]
    fn the_wait_and_the_count_restart_with_each_request() {
        let mut g = Gate::default();
        g.request(at(0));
        g.answered(false, Answer::Later);
        g.request(at(1));
        g.answered(true, Answer::Later);
        // Cancelled; a new Dock Quit much later asks again, from scratch.
        assert!(g.reply(false));
        assert_eq!(g.request(at(60)), Request::Ask { nested: false });
        g.answered(false, Answer::Later);
        assert_eq!(g.request(at(61)), Request::Ask { nested: true });
    }

    #[test]
    fn a_request_waits_for_the_reply() {
        let mut g = Gate::default();
        assert_eq!(g.request(at(0)), Request::Ask { nested: false });
        assert!(g.pending());
        assert_eq!(g.answered(false, Answer::Later), Decision::Later);
        assert!(g.pending());
        assert!(g.reply(true));
        assert!(!g.pending());
        assert!(g.approved());
        // A second reply is not sent.
        assert!(!g.reply(true));
        assert!(!g.reply(false));
    }

    #[test]
    fn nothing_to_ask_terminates_now() {
        let mut g = Gate::default();
        assert_eq!(g.request(at(0)), Request::Ask { nested: false });
        assert_eq!(g.answered(false, Answer::Now), Decision::Now);
        assert!(!g.pending());
        assert!(g.approved());
        assert!(!g.reply(true));
    }

    #[test]
    fn cancel_keeps_running_and_asks_again_next_time() {
        let mut g = Gate::default();
        g.request(at(0));
        assert_eq!(g.answered(false, Answer::Later), Decision::Later);
        assert!(g.reply(false));
        assert!(!g.pending());
        assert!(!g.approved());
        assert_eq!(g.request(at(0)), Request::Ask { nested: false });
    }

    #[test]
    fn approved_requests_go_through_without_asking() {
        let mut g = Gate::default();
        g.approve();
        assert_eq!(g.request(at(0)), Request::Approved);
        assert!(!g.pending());
        // After a reply(true) too (the system may send terminate: again).
        let mut g = Gate::default();
        g.request(at(0));
        g.answered(false, Answer::Later);
        g.reply(true);
        assert_eq!(g.request(at(0)), Request::Approved);
    }

    #[test]
    fn a_second_request_while_one_waits_does_not_stack() {
        let mut g = Gate::default();
        g.request(at(0));
        g.answered(false, Answer::Later);
        // Dock Quit again, or a log-out, while the prompt is open.
        assert_eq!(g.request(at(0)), Request::Ask { nested: true });
        assert_eq!(g.answered(true, Answer::Later), Decision::Cancel);
        // The first one still waits, and gets exactly one reply.
        assert!(g.pending());
        assert!(g.reply(false));
        assert!(!g.reply(false));
    }

    #[test]
    fn a_nested_request_with_nothing_left_to_ask_terminates() {
        let mut g = Gate::default();
        g.request(at(0));
        g.answered(false, Answer::Later);
        assert_eq!(g.request(at(0)), Request::Ask { nested: true });
        assert_eq!(g.answered(true, Answer::Now), Decision::Now);
        assert!(!g.pending());
        assert!(!g.reply(true));
    }

    #[test]
    fn a_failing_handler_falls_back_to_terminating() {
        let mut g = Gate::default();
        g.request(at(0));
        assert_eq!(g.failed(), Decision::Now);
        assert!(!g.pending());
    }

    #[test]
    fn a_reply_without_a_request_is_ignored() {
        let mut g = Gate::default();
        assert!(!g.reply(true));
        assert!(!g.approved());
        assert!(!g.reply(false));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_have_nothing_to_install() {
        assert_eq!(install(|| Answer::Now), Ok(()));
        assert!(!reply(true));
        assert!(!terminate());
    }
}
