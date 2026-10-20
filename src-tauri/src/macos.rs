//! What only the macOS app needs, through the Objective-C runtime (no crate). Not compiled on
//! Linux; covered (llvm-cov and mutants) by `macos.yml`.
//!
//! - ⌘Q, the Dock's "Quit" and logging out ask first (17.7).
//! - Asking the notification center (`UNUserNotificationCenter`) whether Hive may notify
//!   (17.19). The notifications themselves still go through `mac-notification-sys` (`main.rs`).
//!   Tested against stand-in classes: a test process is no app bundle, and the real center
//!   raises an exception for one.
//! - Where the traffic lights go (18.4), from a close button laid out by this macOS.
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

#[link(name = "objc")]
unsafe extern "C" {
    fn object_getClass(object: Id) -> Id;
    fn class_getInstanceMethod(class: Id, name: Sel) -> Id;
    fn class_getMethodImplementation(class: Id, name: Sel) -> *const c_void;
    fn class_replaceMethod(
        class: Id,
        name: Sel,
        imp: *const c_void,
        types: *const c_char,
    ) -> *const c_void;
    fn method_getTypeEncoding(method: Id) -> *const c_char;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    /// The pasteboard type of a drag's file paths: an array of strings (what wry reads too).
    static NSFilenamesPboardType: Id;
}

/// `objc_msgSend` as a function of type `F`, an `unsafe extern "C" fn` pointer with the
/// message's own signature (arm64 has no variadic messages).
unsafe fn message<F: Copy>() -> F {
    const { assert!(size_of::<F>() == size_of::<unsafe extern "C" fn()>()) };
    // SAFETY: `F` is a function pointer (the size is checked); the caller gives its signature.
    unsafe { std::mem::transmute_copy(&(objc_msgSend as unsafe extern "C" fn())) }
}

/// `NSRect`: an origin and a size, in points (`CGFloat` is `f64` on 64-bit macOS).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The title bar's height (`--bar-h` in `styles.css`) and its bottom border: the bar's items
/// (`align-items: center`) sit on the middle line of the 33 px above the border.
const BAR: f64 = 34.0;
const BAR_BORDER: f64 = 1.0;
/// The close button's distance from the window's left edge (17.15).
const TRAFFIC_LIGHTS_X: f64 = 10.0;
/// Titled (bit 0), closable (1), miniaturizable (2), resizable (3) and full-size content view
/// (15): Hive's window (`titleBarStyle: Overlay`).
const WINDOW_STYLE: usize = 0x800f;
/// `NSBackingStoreBuffered`.
const BUFFERED: usize = 2;

/// `trafficLightPosition`'s `y` that centres buttons laid out like `close` (its frame in the
/// title bar view, `y` from the view's bottom) on the bar's middle line (18.4). tao and wry
/// make the title bar view `close.height + y` tall from the window's top and leave each button
/// `close.y` above its bottom, so the button's top lands `y - close.y` from the window's top.
/// The top is kept on a half point: a whole device pixel on a Retina screen. None for a frame
/// that does not fit in the bar.
pub fn traffic_light_y(close: Rect) -> Option<f64> {
    let room = BAR - BAR_BORDER;
    if !(close.height > 0.0 && close.height <= room && close.y >= 0.0) {
        return None;
    }
    let top = (room - close.height).round() / 2.0;
    Some(top + close.y)
}

/// Where the traffic lights go in a window of the class `window` (`NSWindow` in the app): the
/// close button of a window with Hive's style, made and dropped here and never shown, gives its
/// height and its place in the title bar as this macOS lays them out, so the buttons stay
/// centred whatever the macOS version (17.15 hard-coded 19, measured on macOS 26). None when
/// AppKit gives no such button; the config's position then stays. Main thread only, as any
/// `NSWindow`.
pub fn traffic_lights(window: &CStr) -> Option<tauri::utils::config::LogicalPosition> {
    let close = close_button(window)?;
    let y = traffic_light_y(close)?;
    Some(tauri::utils::config::LogicalPosition {
        x: TRAFFIC_LIGHTS_X,
        y,
    })
}

/// The close button's frame in a new window of the class `window` with Hive's style.
fn close_button(window: &CStr) -> Option<Rect> {
    // SAFETY: a C string; null when no class has that name.
    let class = unsafe { objc_getClass(window.as_ptr()) };
    if class.is_null() {
        return None;
    }
    let content = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 500.0,
    };
    // SAFETY: `NSWindow`'s own signatures: `alloc`, then
    // `initWithContentRect:styleMask:backing:defer:` (which answers nil or the window, owned
    // here and released below), `standardWindowButton:` (a button the window owns, nil when
    // there is none), the button's `frame` and `release`.
    unsafe {
        let send = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel) -> Id>(
            objc_msgSend,
        );
        let init = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel, Rect, usize, usize, bool) -> Id,
        >(objc_msgSend);
        let button = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel, usize) -> Id,
        >(objc_msgSend);
        let frame = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel) -> Rect,
        >(objc_msgSend_rect);
        let window = send(class, selector(c"alloc"));
        let window = init(
            window,
            selector(c"initWithContentRect:styleMask:backing:defer:"),
            content,
            WINDOW_STYLE,
            BUFFERED,
            true,
        );
        if window.is_null() {
            return None;
        }
        // `NSWindowCloseButton`.
        let close = button(window, selector(c"standardWindowButton:"), 0);
        let rect = (!close.is_null()).then(|| frame(close, selector(c"frame")));
        send(window, selector(c"release"));
        rect
    }
}

#[cfg(not(target_arch = "x86_64"))]
use objc_msgSend as objc_msgSend_rect;
/// `objc_msgSend` for a message answering an `NSRect`: arm64 returns it in registers, x86_64
/// through memory (`objc_msgSend_stret`).
#[cfg(target_arch = "x86_64")]
use objc_msgSend_stret as objc_msgSend_rect;

#[cfg(target_arch = "x86_64")]
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_msgSend_stret();
}

/// The `bounds` of `view`.
///
/// # Safety
/// `view` is an `NSView`.
unsafe fn bounds(view: Id) -> Rect {
    // SAFETY: the variant of `objc_msgSend` for an `NSRect` answer on this architecture, with
    // the message's signature; an `NSView` answers `bounds`.
    unsafe {
        let bounds = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel) -> Rect,
        >(objc_msgSend_rect);
        bounds(view, selector(c"bounds"))
    }
}

/// `NSPoint` and `NSSize`: two doubles.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pair {
    x: f64,
    y: f64,
}

/// The most paths one drop types (`take_file_drops`); past that, the first ones.
pub const DROP_LIMIT: usize = 1000;
/// The longest path (in bytes) a drop types; a longer one is left out (macOS' own limit is
/// 1024).
pub const PATH_LIMIT: usize = 4096;

/// Gets the paths of the files dropped from another app and where (`take_file_drops`).
type OnDrop = Box<dyn Fn(Vec<String>, f64, f64) + Send + Sync>;
static ON_DROP: OnceLock<OnDrop> = OnceLock::new();
/// The view class's `performDragOperation:` before `take_file_drops` replaced it.
static PERFORM: OnceLock<usize> = OnceLock::new();
/// `performDragOperation:`'s signature: YES when the drop was taken.
type Perform = unsafe extern "C" fn(Id, Sel, Id) -> Bool;

/// Objective-C's `BOOL`: a signed char on Intel (where only its low byte is set, so it is read
/// as a byte and compared with `NO`), a C `bool` on arm64; both one byte, 0 for NO.
type Bool = i8;
const NO: Bool = 0;
const YES: Bool = 1;
/// `NSUTF8StringEncoding`.
const UTF8: usize = 4;

/// Makes files dropped from another app (Finder) on the webview `view` go to `on_drop` (18.1)
/// with their paths and where they were dropped, in points from the view's top left; the
/// webview never gets them, so it never opens a file in place of the app.
///
/// Tauri's own drag and drop (`dragDropEnabled`) stays off: wry's handler answers every drag,
/// so WebKit would get none, and the app's own HTML5 drags (tabs, the sidebar's agents, the
/// Files tree, #57) go through the same AppKit methods (`draggingEntered:` …) of the same
/// view. Here only the drop itself (`performDragOperation:`) is replaced, on the view's class,
/// and only a drag from another app (no `draggingSource`) carrying file paths is taken: every
/// other drop goes on to the class's own method (wry's, which hands it to WebKit). A taken drop
/// is ended for WebKit as a drag that left (`draggingExited:`), so the page's drag state ends.
/// At most [`DROP_LIMIT`] paths of at most [`PATH_LIMIT`] bytes.
///
/// Only the first call does anything; false when it did not (an earlier call, no view, or a
/// class that does not take drops).
///
/// # Safety
/// `view` is an Objective-C object (the webview, an `NSView`) or null, used on the main
/// thread, where AppKit sends its drags.
pub unsafe fn take_file_drops(
    view: *mut c_void,
    on_drop: impl Fn(Vec<String>, f64, f64) + Send + Sync + 'static,
) -> bool {
    if view.is_null() || ON_DROP.get().is_some() {
        return false;
    }
    let name = selector(c"performDragOperation:");
    // SAFETY: `view` is an object (the caller's promise); the class, its method and its type
    // encoding are the runtime's. The replacement has the same signature ("c@:@" / "B@:@": a
    // BOOL from self, _cmd and the dragging info), so it is installed with the method's own
    // encoding.
    unsafe {
        let class = object_getClass(view);
        let method = class_getInstanceMethod(class, name);
        if method.is_null() {
            return false;
        }
        let _ = ON_DROP.set(Box::new(on_drop));
        let _ = PERFORM.set(class_getMethodImplementation(class, name) as usize);
        let types = method_getTypeEncoding(method);
        class_replaceMethod(class, name, perform_drag as *const c_void, types);
    }
    true
}

/// The replaced `performDragOperation:`: a drop `take_drop` takes, or the class's own.
/// Nothing unwinds into AppKit: a panic leaves the drop to the class's method.
unsafe extern "C" fn perform_drag(view: Id, name: Sel, info: Id) -> Bool {
    // SAFETY: AppKit calls this as `performDragOperation:` on a view, with its dragging info.
    let taken = catch_unwind(AssertUnwindSafe(|| unsafe { take_drop(view, info) }));
    if taken.unwrap_or(false) {
        return YES;
    }
    // SAFETY: the class's own implementation of this very message, kept by `take_file_drops`
    // before it replaced it.
    let own = |&original: &usize| unsafe { std::mem::transmute::<usize, Perform>(original) };
    PERFORM
        .get()
        .map(own)
        .map_or(NO, |own| unsafe { own(view, name, info) })
}

/// Takes the drop `info` on `view` when it comes from another app with file paths: gives them
/// to `ON_DROP` with the point, and tells WebKit the drag left. False for any other drop.
///
/// # Safety
/// `view` is an `NSView` and `info` an `NSDraggingInfo`.
unsafe fn take_drop(view: Id, info: Id) -> bool {
    let (Some(on_drop), false) = (ON_DROP.get(), info.is_null()) else {
        return false;
    };
    // SAFETY: messages `NSDraggingInfo` and `NSView` answer, with their signatures.
    unsafe {
        let object = message::<unsafe extern "C" fn(Id, Sel) -> Id>();
        if !object(info, selector(c"draggingSource")).is_null() {
            return false;
        }
        let paths = file_paths(object(info, selector(c"draggingPasteboard")));
        if paths.is_empty() {
            return false;
        }
        let location = message::<unsafe extern "C" fn(Id, Sel) -> Pair>();
        let location = location(info, selector(c"draggingLocation"));
        let convert = message::<unsafe extern "C" fn(Id, Sel, Pair, Id) -> Pair>();
        let at = convert(
            view,
            selector(c"convertPoint:fromView:"),
            location,
            null_mut(),
        );
        let flipped = message::<unsafe extern "C" fn(Id, Sel) -> Bool>();
        let y = if flipped(view, selector(c"isFlipped")) != NO {
            at.y
        } else {
            bounds(view).height - at.y
        };
        on_drop(paths, at.x, y);
        let exited = message::<unsafe extern "C" fn(Id, Sel, Id)>();
        exited(view, selector(c"draggingExited:"), info);
    }
    true
}

/// The file paths on `pasteboard` (`NSFilenamesPboardType`): of the first [`DROP_LIMIT`]
/// strings, those of at most [`PATH_LIMIT`] bytes that may be typed (`quote::usable`:
/// absolute, no control character); none when it has none.
///
/// # Safety
/// `pasteboard` is an `NSPasteboard` (or nil).
unsafe fn file_paths(pasteboard: Id) -> Vec<String> {
    let mut paths = Vec::new();
    // SAFETY: messages `NSPasteboard`, `NSArray` and `NSString` answer, with their signatures;
    // each object is checked to be of its class first. nil answers nil, 0 and NO.
    unsafe {
        let object = message::<unsafe extern "C" fn(Id, Sel, Id) -> Id>();
        let list = object(
            pasteboard,
            selector(c"propertyListForType:"),
            NSFilenamesPboardType,
        );
        let is = |object: Id, class: &CStr| {
            let is = message::<unsafe extern "C" fn(Id, Sel, Id) -> Bool>();
            let class = objc_getClass(class.as_ptr());
            is(object, selector(c"isKindOfClass:"), class) != NO
        };
        if !is(list, c"NSArray") {
            return paths;
        }
        let count = message::<unsafe extern "C" fn(Id, Sel) -> usize>();
        let at = message::<unsafe extern "C" fn(Id, Sel, usize) -> Id>();
        let bytes = message::<unsafe extern "C" fn(Id, Sel, usize) -> usize>();
        let text = message::<unsafe extern "C" fn(Id, Sel) -> *const c_char>();
        for index in 0..count(list, selector(c"count")).min(DROP_LIMIT) {
            let item = at(list, selector(c"objectAtIndex:"), index);
            // Measured before it is copied out.
            let fits = is(item, c"NSString")
                && bytes(item, selector(c"lengthOfBytesUsingEncoding:"), UTF8) <= PATH_LIMIT;
            let utf8 = if fits {
                text(item, selector(c"UTF8String"))
            } else {
                std::ptr::null()
            };
            if utf8.is_null() {
                continue;
            }
            let path = CStr::from_ptr(utf8).to_string_lossy();
            // Never a control character or a relative path (`quote::usable`).
            if crate::quote::usable(&path) {
                paths.push(path.into_owned());
            }
        }
    }
    paths
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

    #[link(name = "objc")]
    unsafe extern "C" {
        fn class_createInstance(class: Id, extra: usize) -> Id;
    }

    // What the stand-in view and dragging info answer, and what they were asked.
    /// The info's `draggingSource` (an address, 0 for nil: a drag from another app).
    static SOURCE: AtomicUsize = AtomicUsize::new(0);
    /// The info's pasteboard's `propertyListForType:` answer (an address, 0 for nil).
    static LIST: AtomicUsize = AtomicUsize::new(0);
    /// The type the pasteboard was last asked for.
    static ASKED_TYPE: AtomicUsize = AtomicUsize::new(0);
    /// The view's `isFlipped`.
    static FLIPPED: AtomicU8 = AtomicU8::new(0);
    /// Calls of the view's own `performDragOperation:` and of its `draggingExited:`.
    static PERFORMED: AtomicUsize = AtomicUsize::new(0);
    static EXITED: AtomicUsize = AtomicUsize::new(0);
    /// What `take_file_drops`' handler got; it panics while `PANIC` is set.
    static DROPPED: std::sync::Mutex<Vec<(Vec<String>, f64, f64)>> =
        std::sync::Mutex::new(Vec::new());
    static PANIC: AtomicU8 = AtomicU8::new(0);

    extern "C" fn own_perform(_: Id, _: Sel, _: Id) -> Bool {
        PERFORMED.fetch_add(1, SeqCst);
        NO
    }
    extern "C" fn exited(_: Id, _: Sel, _: Id) {
        EXITED.fetch_add(1, SeqCst);
    }
    /// Any byte but 0 is YES (Intel's `BOOL` is a signed char).
    extern "C" fn flipped(_: Id, _: Sel) -> Bool {
        FLIPPED.load(SeqCst) as Bool
    }
    extern "C" fn bounds(_: Id, _: Sel) -> Rect {
        frame(0.0, 0.0, 800.0, 600.0)
    }
    /// The view starts 10 points right and 20 up of the window's origin.
    extern "C" fn convert(_: Id, _: Sel, point: Pair, from: Id) -> Pair {
        assert!(from.is_null(), "from the window");
        Pair {
            x: point.x - 10.0,
            y: point.y - 20.0,
        }
    }
    extern "C" fn source(_: Id, _: Sel) -> Id {
        SOURCE.load(SeqCst) as Id
    }
    extern "C" fn list(_: Id, _: Sel, kind: Id) -> Id {
        ASKED_TYPE.store(kind as usize, SeqCst);
        LIST.load(SeqCst) as Id
    }
    extern "C" fn location(_: Id, _: Sel) -> Pair {
        Pair { x: 110.0, y: 520.0 }
    }

    /// A new subclass of NSObject `name` with the instance methods `methods`, and one instance.
    fn instance(name: &CStr, methods: &[(&CStr, *const c_void, &CStr)]) -> Id {
        // SAFETY: a new class, registered once, with methods of the signatures they say.
        unsafe {
            let class =
                objc_allocateClassPair(objc_getClass(c"NSObject".as_ptr()), name.as_ptr(), 0);
            assert!(!class.is_null());
            for (name, imp, types) in methods {
                assert!(class_addMethod(class, selector(name), *imp, types.as_ptr()));
            }
            objc_registerClassPair(class);
            class_createInstance(class, 0)
        }
    }

    /// A Foundation string of `text`.
    fn string(text: &str) -> Id {
        let text = std::ffi::CString::new(text).unwrap();
        // SAFETY: `[[NSString alloc] initWithUTF8String:]`, never released (a test's).
        unsafe {
            let new = message::<unsafe extern "C" fn(Id, Sel) -> Id>();
            let alloc = new(objc_getClass(c"NSString".as_ptr()), selector(c"alloc"));
            let init = message::<unsafe extern "C" fn(Id, Sel, *const c_char) -> Id>();
            init(alloc, selector(c"initWithUTF8String:"), text.as_ptr())
        }
    }

    /// A Foundation array of `items`.
    fn array(items: &[Id]) -> Id {
        // SAFETY: `[[NSArray alloc] initWithObjects:count:]`, never released (a test's).
        unsafe {
            let new = message::<unsafe extern "C" fn(Id, Sel) -> Id>();
            let alloc = new(objc_getClass(c"NSArray".as_ptr()), selector(c"alloc"));
            let init = message::<unsafe extern "C" fn(Id, Sel, *const Id, usize) -> Id>();
            init(
                alloc,
                selector(c"initWithObjects:count:"),
                items.as_ptr(),
                items.len(),
            )
        }
    }

    fn frame(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn files_dropped_from_another_app_go_to_the_handler_and_every_other_drop_to_the_view() {
        let view = instance(
            c"HiveDropView",
            &[
                (
                    c"performDragOperation:",
                    own_perform as *const c_void,
                    c"B@:@",
                ),
                (c"draggingExited:", exited as *const c_void, c"v@:@"),
                (c"isFlipped", flipped as *const c_void, c"B@:"),
                (c"bounds", bounds as *const c_void, c"{CGRect=dddd}@:"),
                (
                    c"convertPoint:fromView:",
                    convert as *const c_void,
                    c"{CGPoint=dd}@:{CGPoint=dd}@",
                ),
            ],
        );
        let info = instance(
            c"HiveDragInfo",
            &[
                (c"draggingSource", source as *const c_void, c"@@:"),
                // The info is its own pasteboard.
                (c"draggingPasteboard", itself as *const c_void, c"@@:"),
                (c"propertyListForType:", list as *const c_void, c"@@:@"),
                (
                    c"draggingLocation",
                    location as *const c_void,
                    c"{CGPoint=dd}@:",
                ),
            ],
        );
        let no_drops = instance(c"HiveNoDropView", &[]);
        let handler = |paths: Vec<String>, x: f64, y: f64| {
            assert_eq!(PANIC.load(SeqCst), 0, "a broken handler");
            DROPPED.lock().unwrap().push((paths, x, y));
        };
        // No view, or one that takes no drops: nothing replaced.
        // SAFETY: stand-in objects (or null), on the test's thread.
        let take = |view: Id| unsafe { take_file_drops(view, handler) };
        assert!(!take(null_mut()));
        assert!(!take(no_drops));
        assert!(take(view));
        assert!(!take(view), "only once");

        // AppKit's call, through the class's (replaced) method.
        let drop = |info: Id| {
            // SAFETY: the stand-in view's `performDragOperation:`.
            unsafe {
                let perform = message::<unsafe extern "C" fn(Id, Sel, Id) -> Bool>();
                perform(view, selector(c"performDragOperation:"), info)
            }
        };
        let dropped = || std::mem::take(&mut *DROPPED.lock().unwrap());
        let counts = || (PERFORMED.swap(0, SeqCst), EXITED.swap(0, SeqCst));

        let long = format!("/{}", "x".repeat(PATH_LIMIT - 1));
        let paths = [
            string("/Users/me/My Shots/a.png"),
            // Not a string: left out.
            no_drops,
            string(&long),
            // One byte too long: left out.
            string(&format!("{long}y")),
            string("/tmp/日本語"),
            // Never typed: control characters (they would act as keys), a relative path.
            string("/tmp/a\x1b[201~\x15touch ~/PWNED\n"),
            string("/tmp/\u{9b}201~"),
            string("/tmp/a\rb"),
            string("a.png"),
        ];
        LIST.store(array(&paths) as usize, SeqCst);
        // From another app, with files: taken, and WebKit told the drag left.
        assert_eq!(drop(info), YES);
        let expected = vec![
            "/Users/me/My Shots/a.png".to_owned(),
            long,
            "/tmp/日本語".into(),
        ];
        // The window's (110, 520) is the view's (100, 500), 100 from its top.
        assert_eq!(dropped(), [(expected.clone(), 100.0, 100.0)]);
        assert_eq!(counts(), (0, 1));
        // A flipped view counts from its top already.
        FLIPPED.store(2, SeqCst);
        assert_eq!(drop(info), YES);
        assert_eq!(dropped(), [(expected, 100.0, 500.0)]);
        assert_eq!(counts(), (0, 1));
        // SAFETY: AppKit's constant, read only.
        let paths_type = unsafe { NSFilenamesPboardType } as usize;
        assert_eq!(ASKED_TYPE.load(SeqCst), paths_type, "asked for file paths");

        // Past the limit, the first ones.
        let many: Vec<Id> = (0..=DROP_LIMIT).map(|i| string(&format!("/{i}"))).collect();
        LIST.store(array(&many) as usize, SeqCst);
        assert_eq!(drop(info), YES);
        let got = dropped();
        assert_eq!(got[0].0.len(), DROP_LIMIT);
        assert_eq!(got[0].0.last().map(String::as_str), Some("/999"));
        counts();

        // Every other drop is the view's own: the app's own drags (a source), no paths, no
        // array, an empty one, only paths never typed, no info, or a handler that panics.
        let own = |info: Id| {
            assert_eq!(drop(info), NO, "the view's answer");
            assert!(dropped().is_empty());
            assert_eq!(counts(), (1, 0));
        };
        SOURCE.store(view as usize, SeqCst);
        own(info);
        SOURCE.store(0, SeqCst);
        for list in [
            null_mut(),
            string("/tmp/a"),
            array(&[]),
            array(&[no_drops]),
            array(&paths[5..]),
        ] {
            LIST.store(list as usize, SeqCst);
            own(info);
        }
        own(null_mut());
        LIST.store(array(&paths) as usize, SeqCst);
        PANIC.store(1, SeqCst);
        own(info);
    }

    #[test]
    fn the_traffic_lights_sit_on_the_bars_middle_line() {
        // Measured on macOS 26 (18.4): a 14 pt button 9 pt above the title bar view's bottom.
        // Its top goes 9.5 pt down, its middle on the 33 pt above the border's (16.5).
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 14.0, 14.0)), Some(18.5));
        // An older macOS's 28 pt title bar, the button centred 7 pt above its bottom.
        assert_eq!(traffic_light_y(frame(7.0, 7.0, 14.0, 14.0)), Some(16.5));
        // Other sizes: the top on a half point, the middle as near 16.5 as that allows.
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 16.0, 16.0)), Some(17.5));
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 13.0, 13.0)), Some(19.0));
        assert_eq!(traffic_light_y(frame(9.0, 8.0, 15.0, 15.0)), Some(17.0));
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 13.4, 13.4)), Some(19.0));
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 13.8, 13.8)), Some(18.5));
        // The edges: a button as tall as the bar, one at the view's bottom.
        assert_eq!(traffic_light_y(frame(9.0, 9.0, 14.0, 33.0)), Some(9.0));
        assert_eq!(traffic_light_y(frame(9.0, 0.0, 14.0, 14.0)), Some(9.5));
        // Frames that cannot be centred keep the config's position.
        for close in [
            frame(9.0, 9.0, 14.0, 0.0),
            frame(9.0, 9.0, 14.0, -14.0),
            frame(9.0, 9.0, 14.0, 33.5),
            frame(9.0, -1.0, 14.0, 14.0),
            frame(9.0, 9.0, 14.0, f64::NAN),
        ] {
            assert_eq!(traffic_light_y(close), None, "{close:?}");
        }
    }

    /// What the stand-in window does: 0 has a close button, 1 is no window, 2 has no button.
    static WINDOW_MODE: AtomicU8 = AtomicU8::new(0);
    /// The stand-in close button's frame.
    static BUTTON_FRAME: std::sync::Mutex<Rect> = std::sync::Mutex::new(Rect {
        x: 9.0,
        y: 9.0,
        width: 14.0,
        height: 14.0,
    });
    /// What the stand-in window was made with and asked for, and its releases.
    static MADE: std::sync::Mutex<Vec<(Rect, usize, usize, bool)>> =
        std::sync::Mutex::new(Vec::new());
    static BUTTONS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
    static RELEASES: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn window_init(
        window: Id,
        _: Sel,
        content: Rect,
        style: usize,
        backing: usize,
        defer: bool,
    ) -> Id {
        MADE.lock().unwrap().push((content, style, backing, defer));
        if WINDOW_MODE.load(SeqCst) == 1 {
            null_mut()
        } else {
            window
        }
    }
    extern "C" fn window_button(_: Id, _: Sel, which: usize) -> Id {
        BUTTONS.lock().unwrap().push(which);
        if WINDOW_MODE.load(SeqCst) == 2 {
            return null_mut();
        }
        // SAFETY: NSObject's `alloc` on a registered class.
        unsafe {
            let send = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel) -> Id,
            >(objc_msgSend);
            send(
                objc_getClass(c"HiveTestButton".as_ptr()),
                selector(c"alloc"),
            )
        }
    }
    extern "C" fn window_release(_: Id, _: Sel) {
        RELEASES.fetch_add(1, SeqCst);
    }
    extern "C" fn button_frame(_: Id, _: Sel) -> Rect {
        *BUTTON_FRAME.lock().unwrap()
    }

    /// Registers `HiveTestWindow` and `HiveTestButton`, stand-ins for `NSWindow` and its button.
    fn window_stand_ins() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            let object = objc_getClass(c"NSObject".as_ptr());
            let window = objc_allocateClassPair(object, c"HiveTestWindow".as_ptr(), 0);
            let rect = "{CGRect={CGPoint=dd}{CGSize=dd}}";
            let init = std::ffi::CString::new(format!("@@:{rect}QQB")).unwrap();
            let methods: [(&CStr, *const c_void, &CStr); 3] = [
                (
                    c"initWithContentRect:styleMask:backing:defer:",
                    window_init as *const c_void,
                    &init,
                ),
                (
                    c"standardWindowButton:",
                    window_button as *const c_void,
                    c"@@:Q",
                ),
                (c"release", window_release as *const c_void, c"v@:"),
            ];
            for (name, imp, types) in methods {
                assert!(class_addMethod(window, selector(name), imp, types.as_ptr()));
            }
            objc_registerClassPair(window);
            let button = objc_allocateClassPair(object, c"HiveTestButton".as_ptr(), 0);
            let types = std::ffi::CString::new(format!("{rect}@:")).unwrap();
            let frame = button_frame as *const c_void;
            assert!(class_addMethod(
                button,
                selector(c"frame"),
                frame,
                types.as_ptr()
            ));
            objc_registerClassPair(button);
        });
    }

    #[test]
    fn the_traffic_lights_follow_a_window_with_hives_style() {
        window_stand_ins();
        assert!(!unsafe { objc_getClass(c"NSWindow".as_ptr()) }.is_null());
        assert_eq!(traffic_lights(c"HiveNoSuchWindow"), None);
        let made = |mode| {
            WINDOW_MODE.store(mode, SeqCst);
            traffic_lights(c"HiveTestWindow").map(|position| (position.x, position.y))
        };
        assert_eq!(made(0), Some((10.0, 18.5)));
        let content = frame(0.0, 0.0, 800.0, 500.0);
        let style = 1 | 2 | 4 | 8 | 1 << 15;
        assert_eq!(MADE.lock().unwrap()[0], (content, style, 2, true));
        assert_eq!(*BUTTONS.lock().unwrap(), [0], "the close button");
        assert_eq!(RELEASES.load(SeqCst), 1, "the window is dropped");
        // Another macOS's layout.
        *BUTTON_FRAME.lock().unwrap() = frame(7.0, 7.0, 14.0, 14.0);
        assert_eq!(made(0), Some((10.0, 16.5)));
        // A frame that does not fit, no window, no button: the config's position stays.
        *BUTTON_FRAME.lock().unwrap() = frame(7.0, 7.0, 40.0, 40.0);
        assert_eq!(made(0), None);
        assert_eq!(RELEASES.load(SeqCst), 3);
        assert_eq!(made(1), None);
        assert_eq!(RELEASES.load(SeqCst), 3, "no window to release");
        assert_eq!(made(2), None);
        assert_eq!(RELEASES.load(SeqCst), 4);
        assert_eq!(
            BUTTONS.lock().unwrap().len(),
            4,
            "no button asked of no window"
        );
    }
}
