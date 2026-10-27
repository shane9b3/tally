//! The AppKit side (macOS only; the crate's only `unsafe`).
//!
//! Three unsafe operations:
//! 1. converting our `extern "C-unwind"` method function to the untyped
//!    `Imp` pointer the runtime takes (install time),
//! 2. `class_addMethod` / `class_replaceMethod` on the delegate's class
//!    (install time), and
//! 3. `-[NSRunLoop performInModes:block:]`, to queue the reply on the main
//!    run loop (unsafe only because the block must be sendable).
//!
//! Everything else (`NSApp`, its delegate, `replyToApplicationShouldTerminate:`,
//! `terminate:`) goes through the safe `objc2-app-kit` API on the main thread.

use std::ffi::CString;

use objc2::encode::Encode;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use std::sync::Mutex;

use block2::RcBlock;
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use objc2_core_foundation::CFRunLoop;
use objc2_foundation::{NSArray, NSRunLoop, NSString};

use crate::{decide, gate, log, Decision, InstallError};

/// `-(NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender`.
///
/// Called by AppKit on the main thread from inside `terminate:`. The
/// receiver and `sender` are not used (they are only passed as raw pointers
/// and never dereferenced). Panics cannot unwind into AppKit: [`decide`]
/// catches every panic, and the ABI is `extern "C"`, so anything that still
/// escaped would abort instead of unwinding through Objective-C frames.
extern "C" fn should_terminate(
    _this: *mut AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> NSApplicationTerminateReply {
    match decide() {
        Decision::Now => NSApplicationTerminateReply::TerminateNow,
        Decision::Later => NSApplicationTerminateReply::TerminateLater,
        Decision::Cancel => NSApplicationTerminateReply::TerminateCancel,
    }
}

type ShouldTerminateFn =
    extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> NSApplicationTerminateReply;

/// The method's Objective-C type encoding, built from the Rust types so it
/// cannot disagree with the function: `Q@:@` on 64-bit (return
/// `NSUInteger`, then `self`, `_cmd`, `sender`).
fn type_encoding() -> CString {
    let types = format!(
        "{}{}{}{}",
        NSApplicationTerminateReply::ENCODING,
        <*mut AnyObject>::ENCODING,
        Sel::ENCODING,
        <*mut AnyObject>::ENCODING,
    );
    // The encodings never contain NUL.
    CString::new(types).unwrap_or_default()
}

/// Adds (or, should tao ever implement it, replaces) the delegate's
/// `applicationShouldTerminate:`.
pub(crate) fn add_should_terminate() -> Result<(), InstallError> {
    let mtm = MainThreadMarker::new().ok_or(InstallError::NotMainThread)?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app.delegate().ok_or(InstallError::NoDelegate)?;
    let delegate: &AnyObject = delegate.as_ref();
    // The object's actual (dynamic) class: tao's `TaoAppDelegateParent`,
    // which tao registered at run time. Messages to the delegate are
    // looked up here, and no other object uses this class.
    let class: &AnyClass = delegate.class();
    let name = sel!(applicationShouldTerminate:);
    let types = type_encoding();
    let f: ShouldTerminateFn = should_terminate;
    // SAFETY: `Imp` is the runtime's untyped method pointer
    // (`unsafe extern "C-unwind" fn()`), the same size as any other function
    // pointer; `extern "C"` and `extern "C-unwind"` share the calling
    // convention (they differ only in what a panic does, and ours never
    // unwinds). The runtime calls it back with the exact
    // signature declared by `types` (built from these Rust types above), so
    // it is only ever called as `ShouldTerminateFn`.
    let imp: Imp = unsafe { std::mem::transmute::<ShouldTerminateFn, Imp>(f) };
    let cls = class as *const AnyClass as *mut AnyClass;
    let replacing = class.instance_method(name).is_some();
    if replacing {
        log(format_args!(
            "tally: replacing the app delegate's own applicationShouldTerminate:"
        ));
        // SAFETY: `cls` is a registered, live class (the class of the live
        // delegate object); `name` is a valid selector; `imp` matches
        // `types` (see above); `types` is a NUL-terminated C string that
        // outlives the call (the runtime copies it). We are on the main
        // thread, where AppKit sends this message, so no call to the old
        // implementation is in progress on another thread.
        unsafe {
            objc2::ffi::class_replaceMethod(cls, name, imp, types.as_ptr());
        }
        return Ok(());
    }
    // SAFETY: as for `class_replaceMethod` above: a live registered class,
    // a valid selector, an implementation matching `types`, and a C string
    // the runtime copies. Adding a method to a registered class is
    // explicitly supported by the runtime (it is thread-safe and takes
    // effect for the next message send).
    let added = unsafe { objc2::ffi::class_addMethod(cls, name, imp, types.as_ptr()) };
    if added.as_bool() {
        Ok(())
    } else {
        Err(InstallError::Refused)
    }
}

/// `-[NSApp replyToApplicationShouldTerminate:]` for the waiting request,
/// if any. The gate is settled at once (any thread); the AppKit call is
/// queued on the main run loop ([`on_main`]).
pub(crate) fn reply(should_terminate: bool) -> bool {
    if !gate().reply(should_terminate) {
        return false;
    }
    if cfg!(debug_assertions) {
        log(format_args!(
            "tally(debug): replyToApplicationShouldTerminate:{should_terminate}"
        ));
    }
    on_main(move |app| app.replyToApplicationShouldTerminate(should_terminate));
    true
}

/// `-[NSApp terminate:nil]`, queued like [`reply`].
pub(crate) fn terminate() -> bool {
    on_main(|app| app.terminate(None));
    true
}

/// Runs `f` once with `NSApp`, on the main thread, from the main run loop
/// in its common modes (`-[NSRunLoop performInModes:block:]`).
///
/// Why not inline, nor on the main dispatch queue:
/// - inline from Tauri's `run_on_main_thread` would run inside tao's event
///   callback; with `true` AppKit goes on to `applicationWillTerminate:`,
///   where tao takes its event-handler lock again: deadlock;
/// - the main dispatch queue is serial: if `terminate:` itself was sent
///   from a main-queue block, AppKit's wait for the reply runs inside that
///   block and a queued reply block would never run.
///
/// Run-loop blocks run in nested run loops too, and the common modes
/// include the modal-panel mode AppKit waits in. Queuing a block does not
/// wake a sleeping run loop, so it is woken explicitly (otherwise the block
/// could wait for the next input event).
fn on_main(f: impl FnOnce(&NSApplication) + Send + 'static) {
    let once = Mutex::new(Some(f));
    let block = RcBlock::new(move || {
        let f = once.lock().ok().and_then(|mut f| f.take());
        if let (Some(f), Some(mtm)) = (f, MainThreadMarker::new()) {
            f(&NSApplication::sharedApplication(mtm));
        }
    });
    // `NSRunLoopCommonModes` is the string `kCFRunLoopCommonModes` (run
    // loop modes are compared by value); spelled out to avoid reading the
    // extern static.
    let modes = NSArray::from_retained_slice(&[NSString::from_str("kCFRunLoopCommonModes")]);
    // SAFETY: the only requirement (objc2-foundation's docs) is that the
    // block is sendable: it runs on the main thread although it may be
    // queued from another one. Its only capture is a `Mutex` around an
    // `FnOnce + Send` closure, so it is `Send + Sync`; it touches AppKit
    // only after `MainThreadMarker::new()` confirms the main thread.
    unsafe { NSRunLoop::mainRunLoop().performInModes_block(&modes, &block) };
    if let Some(main) = CFRunLoop::main() {
        main.wake_up();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_is_nsuinteger_object_selector_object() {
        let expected = if cfg!(target_pointer_width = "64") {
            "Q@:@"
        } else {
            "I@:@"
        };
        assert_eq!(type_encoding().to_str().unwrap_or_default(), expected);
    }
}
