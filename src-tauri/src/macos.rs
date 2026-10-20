//! What only the macOS app needs, through the Objective-C runtime (no crate). Not compiled on
//! Linux; covered (llvm-cov and mutants) by `macos.yml`.
//!
//! - ⌘Q, the Dock's "Quit" and logging out ask first (17.7).
//! - Asking the notification center (`UNUserNotificationCenter`) whether Hive may notify
//!   (17.19). The notifications themselves still go through `mac-notification-sys` (`main.rs`).
//!   Tested against stand-in classes: a test process is no app bundle, and the real center
//!   raises an exception for one.
//!
//! Nothing unwinds across this boundary: `applicationShouldTerminate:` catches any panic, the
//! center is asked only by an app bundle (`in_app_bundle`), and the completion blocks only hand
//! their answer over, inside `catch_unwind`.

use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;
use std::time::Duration;

use crate::{in_app_bundle, Permission};

/// An Objective-C object (or class) and a selector.
type Id = *mut c_void;
type Sel = *const c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_addMethod(class: Id, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
    /// Called through a cast to each message's own signature (arm64 has no variadic messages).
    fn objc_msgSend();
}

// The center's class is in UserNotifications; `_NSConcreteStackBlock` is libSystem's.
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {
    static _NSConcreteStackBlock: [*const c_void; 32];
}

/// The notification center's class.
pub const CENTER: &CStr = c"UNUserNotificationCenter";
/// How long the center may take to say whether Hive may notify.
const ANSWER_WAIT: Duration = Duration::from_secs(2);
/// `UNAuthorizationOptionAlert`: Hive's notifications are silent (it plays its own tone).
const ALERT: usize = 1 << 2;

/// A block's call: `getNotificationSettingsWithCompletionHandler:`'s, given the settings.
type SettingsDone = unsafe extern "C" fn(*mut c_void, Id);
/// `requestAuthorizationWithOptions:completionHandler:`'s, given `granted` and an error.
type Asked = unsafe extern "C" fn(*mut c_void, u8, Id);

/// A block literal (the Blocks ABI) on the stack, without copy or dispose helpers: the center
/// copies it byte for byte (`Block_copy`) to call it later.
#[repr(C)]
struct Block<F> {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: F,
    descriptor: &'static Descriptor,
    /// Where the answer goes, freed by the block's one call; null when no answer is wanted.
    answer: *mut SyncSender<isize>,
}

/// `BLOCK_HAS_SIGNATURE`.
const HAS_SIGNATURE: c_int = 1 << 30;

#[repr(C)]
struct Descriptor {
    reserved: c_ulong,
    size: c_ulong,
    /// The block's type encoding.
    signature: *const c_char,
}

// SAFETY: read-only, and its signature points to a static string.
unsafe impl Sync for Descriptor {}

static SETTINGS_BLOCK: Descriptor = Descriptor {
    reserved: 0,
    size: size_of::<Block<SettingsDone>>() as c_ulong,
    signature: c"v16@?0@8".as_ptr(),
};
static ASKED_BLOCK: Descriptor = Descriptor {
    reserved: 0,
    size: size_of::<Block<Asked>>() as c_ulong,
    signature: c"v20@?0B8@12".as_ptr(),
};

impl<F> Block<F> {
    fn new(invoke: F, descriptor: &'static Descriptor, answer: *mut SyncSender<isize>) -> Self {
        Self {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: HAS_SIGNATURE,
            reserved: 0,
            invoke,
            descriptor,
            answer,
        }
    }
}

fn selector(name: &CStr) -> Sel {
    // SAFETY: a C string; the runtime registers any name.
    unsafe { sel_registerName(name.as_ptr()) }
}

/// The class `name`'s `currentNotificationCenter`; none when there is no such class.
fn center(name: &CStr) -> Option<Id> {
    // SAFETY: a C string; null when no class has that name.
    let class = unsafe { objc_getClass(name.as_ptr()) };
    if class.is_null() {
        return None;
    }
    // SAFETY: a class method taking nothing and answering an object.
    let center = unsafe {
        let send = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel) -> Id>(
            objc_msgSend,
        );
        send(class, selector(c"currentNotificationCenter"))
    };
    (!center.is_null()).then_some(center)
}

/// Asks the center `name` to let Hive show alerts; macOS asks the user only while they have not
/// answered yet. The answer is not awaited: `permission` reads it later.
pub fn ask(name: &CStr) {
    let Some(center) = center(name) else { return };
    let mut block = Block::new(asked as Asked, &ASKED_BLOCK, null_mut());
    // SAFETY: the method's signature; the center copies the block before returning.
    unsafe {
        let send = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel, usize, *mut c_void),
        >(objc_msgSend);
        let request = selector(c"requestAuthorizationWithOptions:completionHandler:");
        send(center, request, ALERT, (&raw mut block).cast());
    }
}

/// `ask`'s completion: nothing to do, `permission` reads the answer.
unsafe extern "C" fn asked(_block: *mut c_void, _granted: u8, _error: Id) {}

/// Whether the center `name` lets Hive show notifications, waiting at most `wait` for it.
pub fn permission(name: &CStr, wait: Duration) -> Permission {
    let Some(center) = center(name) else {
        return Permission::Unknown;
    };
    let (answer, answered) = sync_channel(1);
    let answer = Box::into_raw(Box::new(answer));
    let mut block = Block::new(settings_done as SettingsDone, &SETTINGS_BLOCK, answer);
    // SAFETY: the method's signature; the center copies the block before returning. A block
    // never called leaks its sender, which is small.
    unsafe {
        let send = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel, *mut c_void),
        >(objc_msgSend);
        let get = selector(c"getNotificationSettingsWithCompletionHandler:");
        send(center, get, (&raw mut block).cast());
    }
    // `UNAuthorizationStatus`: not determined, denied, authorized, provisional, ephemeral.
    match answered.recv_timeout(wait) {
        Ok(1) => Permission::Denied,
        Ok(2..=4) => Permission::Granted,
        _ => Permission::Unknown,
    }
}

/// `permission`'s completion: hands the settings' `authorizationStatus` over.
unsafe extern "C" fn settings_done(block: *mut c_void, settings: Id) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: a block `permission` built (or the center's copy of it), called once, as the
        // center's contract says; `settings` answers `authorizationStatus` (nil answers 0).
        unsafe {
            let answer = Box::from_raw((*block.cast::<Block<SettingsDone>>()).answer);
            let send = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel) -> isize,
            >(objc_msgSend);
            let _ = answer.try_send(send(settings, selector(c"authorizationStatus")));
        }
    }));
}

/// Whether Hive may notify, as the app at `exe` can tell (17.19): an app bundle asks the center
/// `name` now (the first launch's prompt) and reads its answer on each call; anything else (a
/// development build) cannot ask and is always unknown.
pub fn permissions(
    exe: Option<&Path>,
    name: &'static CStr,
) -> impl Fn() -> Permission + Send + Sync + 'static {
    let center = exe.is_some_and(in_app_bundle).then_some(name);
    if let Some(center) = center {
        ask(center);
    }
    move || {
        center.map_or(Permission::Unknown, |center| {
            permission(center, ANSWER_WAIT)
        })
    }
}

/// Answers whether the app may quit now (`crate::should_quit` in the app).
type Ask = Box<dyn Fn() -> bool + Send + Sync>;

/// The answer `should_terminate` gives; AppKit calls it with no context of ours.
static ASK: OnceLock<Ask> = OnceLock::new();

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
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicIsize, AtomicU8, AtomicUsize, Ordering::SeqCst};
    use std::sync::Once;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
        fn objc_registerClassPair(class: Id);
        fn class_getMethodImplementation(class: Id, name: Sel) -> *const c_void;
        fn object_getClass(object: Id) -> Id;
    }

    // libSystem's blocks runtime, which the center copies its completion blocks with.
    unsafe extern "C" {
        fn _Block_copy(block: *const c_void) -> *mut c_void;
        fn _Block_release(block: *const c_void);
        fn _Block_signature(block: *const c_void) -> *const c_char;
    }

    /// The type encodings of the blocks the stand-ins were given.
    static SIGNATURES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    /// Copies `block` as the center does, keeping its signature; the copy is called and released.
    unsafe fn copied<F>(block: *mut Block<F>) -> *mut Block<F> {
        unsafe {
            let signature = _Block_signature(block.cast());
            let text = (!signature.is_null()).then(|| CStr::from_ptr(signature));
            let text = text.map_or(String::new(), |text| text.to_string_lossy().into_owned());
            SIGNATURES.lock().unwrap().push(text);
            _Block_copy(block.cast()).cast()
        }
    }

    /// What the stand-in centers' settings answer as `authorizationStatus`.
    static STATUS: AtomicIsize = AtomicIsize::new(0);
    /// The options of the last request, and how many requests came.
    static OPTIONS: AtomicUsize = AtomicUsize::new(0);
    static REQUESTS: AtomicUsize = AtomicUsize::new(0);

    // The stand-ins' class methods: each center is its class, which is also its settings.
    extern "C" fn itself(class: Id, _: Sel) -> Id {
        class
    }
    extern "C" fn none(_: Id, _: Sel) -> Id {
        null_mut()
    }
    extern "C" fn status(_: Id, _: Sel) -> isize {
        STATUS.load(SeqCst)
    }
    unsafe extern "C" fn settings(class: Id, _: Sel, block: *mut Block<SettingsDone>) {
        unsafe {
            let copy = copied(block);
            ((*copy).invoke)(copy.cast(), class);
            _Block_release(copy.cast());
        }
    }
    extern "C" fn silent(_: Id, _: Sel, _: *mut c_void) {}
    unsafe extern "C" fn request(_: Id, _: Sel, options: usize, block: *mut Block<Asked>) {
        OPTIONS.store(options, SeqCst);
        REQUESTS.fetch_add(1, SeqCst);
        unsafe {
            let copy = copied(block);
            ((*copy).invoke)(copy.cast(), 1, null_mut());
            _Block_release(copy.cast());
        }
    }

    /// Registers a stand-in center class `name`.
    fn stand_in(name: &CStr, current: *const c_void, get: *const c_void) {
        unsafe {
            let class =
                objc_allocateClassPair(objc_getClass(c"NSObject".as_ptr()), name.as_ptr(), 0);
            assert!(!class.is_null());
            let meta = object_getClass(class);
            let methods: [(&CStr, *const c_void, &CStr); 4] = [
                (c"currentNotificationCenter", current, c"@@:"),
                (
                    c"getNotificationSettingsWithCompletionHandler:",
                    get,
                    c"v@:@?",
                ),
                (
                    c"requestAuthorizationWithOptions:completionHandler:",
                    request as *const c_void,
                    c"v@:Q@?",
                ),
                (c"authorizationStatus", status as *const c_void, c"q@:"),
            ];
            for (name, imp, types) in methods {
                assert!(class_addMethod(meta, selector(name), imp, types.as_ptr()));
            }
            objc_registerClassPair(class);
        }
    }

    static STAND_INS: Once = Once::new();

    /// `HiveCenter` answers; `HiveSilentCenter` never calls back; `HiveNoCenter` has none.
    fn stand_ins() {
        STAND_INS.call_once(|| {
            let (itself, none) = (itself as *const c_void, none as *const c_void);
            let (settings, silent) = (settings as *const c_void, silent as *const c_void);
            stand_in(c"HiveCenter", itself, settings);
            stand_in(c"HiveSilentCenter", itself, silent);
            stand_in(c"HiveNoCenter", none, settings);
        });
    }

    #[test]
    fn the_real_center_is_linked() {
        assert!(center(c"HiveMissingCenter").is_none());
        // Only looked up: a test process is no app bundle, so the center would raise.
        assert!(!unsafe { objc_getClass(CENTER.as_ptr()) }.is_null());
    }

    #[test]
    fn an_app_bundle_asks_for_alerts_and_reads_the_answer() {
        stand_ins();
        let app = Path::new("/Applications/Hive.app/Contents/MacOS/hive-app");
        let check = permissions(Some(app), c"HiveCenter");
        assert_eq!(REQUESTS.load(SeqCst), 1, "asked once, at launch");
        assert_eq!(OPTIONS.load(SeqCst), 1 << 2, "alerts only");
        let answers = [
            (0, Permission::Unknown),
            (1, Permission::Denied),
            (2, Permission::Granted),
            (3, Permission::Granted),
            (4, Permission::Granted),
            (5, Permission::Unknown),
        ];
        for (status, permission) in answers {
            STATUS.store(status, SeqCst);
            assert_eq!(check(), permission, "status {status}");
        }
        assert_eq!(REQUESTS.load(SeqCst), 1, "reading asks no more");
        let signatures = SIGNATURES.lock().unwrap().clone();
        let mut expected = vec!["v20@?0B8@12".to_owned()];
        expected.extend(std::iter::repeat_n("v16@?0@8".to_owned(), answers.len()));
        assert_eq!(signatures, expected, "blocks that say their types");
        // A development build asks nothing.
        STATUS.store(1, SeqCst);
        let development = Path::new("/src/hive/target/debug/hive-app");
        for exe in [Some(development), None] {
            assert_eq!(permissions(exe, c"HiveCenter")(), Permission::Unknown);
        }
        assert_eq!(REQUESTS.load(SeqCst), 1);
    }

    #[test]
    fn a_missing_or_silent_center_is_unknown() {
        stand_ins();
        let wait = Duration::from_millis(50);
        for name in [c"HiveMissingCenter", c"HiveNoCenter", c"HiveSilentCenter"] {
            assert_eq!(permission(name, wait), Permission::Unknown, "{name:?}");
        }
        // Asking without a center does nothing (the stand-ins count only real requests).
        ask(c"HiveMissingCenter");
        ask(c"HiveNoCenter");
    }

    /// What the test's `ask` does: 0 keeps the app, 1 quits, 2 panics.
    static MODE: AtomicU8 = AtomicU8::new(0);

    #[test]
    fn the_delegate_class_gets_the_method_which_answers_and_never_unwinds() {
        let null = std::ptr::null_mut();
        // Asked before any answer: it quits.
        assert_eq!(should_terminate(null, null, null), TERMINATE_NOW);
        let ask = || match MODE.load(SeqCst) {
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
            MODE.store(mode, SeqCst);
            assert_eq!(should_terminate(null, null, null), reply, "mode {mode}");
        }
    }
}
