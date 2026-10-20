//! macOS-only glue of the app (17.7): ⌘Q, the Dock's "Quit" and logging out ask first.
//! Not compiled on Linux; covered (llvm-cov and mutants) by `macos.yml`.

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

/// Answers whether the app may quit now (`crate::should_quit` in the app).
type Ask = Box<dyn Fn() -> bool + Send + Sync>;

/// The answer `should_terminate` gives; AppKit calls it with no context of ours.
static ASK: OnceLock<Ask> = OnceLock::new();

// The Objective-C runtime, which every macOS app links.
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn class_addMethod(
        class: *mut c_void,
        name: *mut c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> i8;
}

/// `NSApplicationTerminateReply`'s `NSTerminateCancel`: the app stays (and asks the user).
const TERMINATE_CANCEL: usize = 0;
/// `NSTerminateNow`: the app quits.
const TERMINATE_NOW: usize = 1;

/// `applicationShouldTerminate:`. Nothing may unwind into AppKit, so a panic quits, as does
/// an answer asked before `ask_before_terminating` gave one.
extern "C" fn should_terminate(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> usize {
    let ask = || ASK.get().is_none_or(|ask| ask());
    match catch_unwind(AssertUnwindSafe(ask)) {
        Ok(false) => TERMINATE_CANCEL,
        _ => TERMINATE_NOW,
    }
}

/// Makes ⌘Q, the Dock's "Quit" and logging out ask `ask` first (D17.3). They all reach
/// `[NSApp terminate:]`, which quits at once unless the app delegate answers
/// `applicationShouldTerminate:`; tao's delegate class (`class`, `TaoAppDelegateParent`) has
/// no such method and Tauri sends no `ExitRequested` for them, so it is added to that class.
/// Only the first `ask` is kept. False (and the app keeps quitting at once) when the class is
/// missing or already answers it.
pub fn ask_before_terminating(
    class: &CStr,
    ask: impl Fn() -> bool + Send + Sync + 'static,
) -> bool {
    let _ = ASK.set(Box::new(ask));
    // SAFETY: `class` and the selector are NUL-terminated C strings; the class name is tao's,
    // borrowed (tao registers it before the app's `setup`, and a missing one is checked). The
    // method's type encoding "Q@:@" (NSUInteger from self, _cmd and the sender) matches
    // `should_terminate`: `usize` is `NSUInteger` on 64-bit macOS, every Mac Hive runs on.
    unsafe {
        let class = objc_getClass(class.as_ptr());
        !class.is_null()
            && class_addMethod(
                class,
                sel_registerName(c"applicationShouldTerminate:".as_ptr()),
                should_terminate as *const c_void,
                c"Q@:@".as_ptr(),
            ) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};

    unsafe extern "C" {
        fn objc_allocateClassPair(
            superclass: *mut c_void,
            name: *const c_char,
            extra: usize,
        ) -> *mut c_void;
        fn objc_registerClassPair(class: *mut c_void);
        fn class_getMethodImplementation(class: *mut c_void, name: *mut c_void) -> *const c_void;
    }

    /// What the test's `ask` does: 0 keeps the app, 1 quits, 2 panics.
    static MODE: AtomicU8 = AtomicU8::new(0);

    #[test]
    fn the_delegate_class_gets_the_method_which_answers_and_never_unwinds() {
        let null = std::ptr::null_mut();
        // Asked before any answer: it quits.
        assert_eq!(should_terminate(null, null, null), TERMINATE_NOW);
        let ask = || match MODE.load(Ordering::SeqCst) {
            0 => false,
            1 => true,
            _ => panic!("a broken answer"),
        };
        // No such class: nothing added, macOS keeps quitting at once.
        assert!(!ask_before_terminating(c"HiveNoSuchDelegate", ask));
        // SAFETY: a new subclass of NSObject, registered once.
        let class = unsafe {
            let class = objc_allocateClassPair(
                objc_getClass(c"NSObject".as_ptr()),
                c"HiveTestDelegate".as_ptr(),
                0,
            );
            objc_registerClassPair(class);
            class
        };
        assert!(ask_before_terminating(c"HiveTestDelegate", ask));
        // Already there: not added twice.
        assert!(!ask_before_terminating(c"HiveTestDelegate", ask));
        // SAFETY: the class and selector exist; the implementation is `should_terminate`.
        let method = unsafe {
            let selector = sel_registerName(c"applicationShouldTerminate:".as_ptr());
            class_getMethodImplementation(class, selector)
        };
        assert_eq!(method, should_terminate as *const c_void);
        for (mode, reply) in [
            (0, TERMINATE_CANCEL),
            (1, TERMINATE_NOW),
            (2, TERMINATE_NOW),
        ] {
            MODE.store(mode, Ordering::SeqCst);
            assert_eq!(should_terminate(null, null, null), reply, "mode {mode}");
        }
    }
}
