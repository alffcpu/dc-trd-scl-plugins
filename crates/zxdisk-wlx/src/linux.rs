//! Linux Qt shell for the ZX-screen lister: a plain `QWidget` child that blits
//! the RGBA frame from [`crate::viewer::model::State`], animates FLASH with a
//! `QObject` timer, and maps key/mouse events to the shared [`Action`]
//! semantics. All the display logic is in the view-model; this file is only the
//! native window and event plumbing.
//!
//! Contract (verified against Double Commander's `uwlxmodule.pas`, v1.2.6): in
//! the Linux Qt builds (`LCLQT`/`LCLQT5`/`LCLQT6`) DC passes `ListLoad` the raw
//! `QWidget*` of the Lister/QuickView container (`WlxPrepareContainer`:
//! `HWND(TQtWidget(ParentWin).GetContainerWidget)`), and afterwards drives the
//! returned handle through the flat C Qt binding DC itself links (Qt5Pas /
//! Qt6Pas): `QWidget_move`/`QWidget_resize` in `ResizeWindow`,
//! `QWidget_setFocus` in `SetFocus`, and `QWidget_Destroy` when the plugin
//! exports no `ListCloseWindow`. So the plugin must create and return a real
//! `QWidget*` child - and the very library needed for that is guaranteed to be
//! loaded in the process already.
//!
//! That is why this shell links nothing: every Qt function it needs is resolved
//! once from the host process via `dlsym(RTLD_DEFAULT)`. Qt5Pas and Qt6Pas
//! export the same C symbols, so one binary serves DC's qt5 and qt6 builds
//! alike; in a GTK2 build of DC (no QtPas in the process) resolution fails and
//! `ListLoad` just returns null - the plugin is inert there instead of crashing
//! (a GTK2 shell can be added later the same way).
//!
//! Painting happens inside the widget's own paint event: the `QObject_hook`
//! event filter (the exact mechanism the LCL Qt widgetset itself paints
//! through) intercepts `QEvent::Paint` and draws with a `QPainter` on the
//! widget - border-colour fill, then the pre-rendered frame centred at native
//! size via a zero-copy `QImage` view (`Format_RGBA8888` matches the
//! view-model's buffer byte-for-byte). Qt double-buffers widget painting in its
//! backing store, so this is flicker-free like the other shells.

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::viewer::model::{
    apply_action, build_state, digit_action, guard, Action, State, FLASH_MS,
};

/// On Linux/Qt the WLX handle is an opaque `QWidget*`.
type Hwnd = *mut c_void;
const NULL: Hwnd = core::ptr::null_mut();

// ------------------------------------------------------- Qt C API (QtXPas) ---

// Stable Qt enum values (identical in Qt 5 and Qt 6).
const QEVENT_TIMER: c_int = 1;
const QEVENT_MOUSE_PRESS: c_int = 2;
const QEVENT_MOUSE_DBLCLICK: c_int = 4;
const QEVENT_KEY_PRESS: c_int = 6;
const QEVENT_PAINT: c_int = 12;
const QT_LEFT_BUTTON: c_int = 0x1;
const QT_RIGHT_BUTTON: c_int = 0x2;
const QT_SHIFT_MODIFIER: c_uint = 0x0200_0000;
const QT_ALT_MODIFIER: c_uint = 0x0800_0000;
const QT_STRONG_FOCUS: c_int = 0xb;
const QT_WA_OPAQUE_PAINT_EVENT: c_int = 4;
const QT_COARSE_TIMER: c_int = 1;
/// `QImage::Format_RGBA8888` - bytes in R,G,B,A order, exactly our frame.
const QIMAGE_FORMAT_RGBA8888: c_int = 17;
const QT_KEY_SPACE: c_int = 0x20;
const QT_KEY_RETURN: c_int = 0x0100_0004;
const QT_KEY_ENTER: c_int = 0x0100_0005; // keypad Enter

/// `QObject_hook_hook_events` takes this two-pointer struct by value; the C++
/// side calls back as `bool (*func)(void *data, QObject *sender, QEvent *ev)`.
#[repr(C)]
#[derive(Copy, Clone)]
struct QHook {
    func: *mut c_void,
    data: *mut c_void,
}

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
/// glibc/musl `RTLD_DEFAULT`: search the global scope of the host process.
const RTLD_DEFAULT: *mut c_void = core::ptr::null_mut();

/// Declare the QtXPas functions we use and a resolver that binds them all from
/// the host process at first use. Signatures match the Qt6Pas cbindings headers
/// (`qwidget_c.h` etc.); Qt5Pas exports the identical C API. `PRect` is a
/// Pascal TRect: four ints left,top,right,bottom with right/bottom exclusive.
macro_rules! qt_api {
    ($( fn $name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?; )+) => {
        #[allow(non_snake_case)]
        struct QtApi {
            $( $name: unsafe extern "C" fn($($ty),*) $(-> $ret)?, )+
        }
        impl QtApi {
            fn resolve() -> Option<QtApi> {
                unsafe {
                    Some(QtApi {
                        $( $name: {
                            let sym = concat!(stringify!($name), "\0");
                            let p = dlsym(RTLD_DEFAULT, sym.as_ptr().cast());
                            if p.is_null() {
                                return None;
                            }
                            core::mem::transmute::<*mut c_void, unsafe extern "C" fn($($ty),*) $(-> $ret)?>(p)
                        }, )+
                    })
                }
            }
        }
    };
}

qt_api! {
    fn QWidget_Create(parent: *mut c_void, f: c_uint) -> *mut c_void;
    fn QWidget_Destroy(handle: *mut c_void);
    fn QWidget_show(handle: *mut c_void);
    fn QWidget_resize(handle: *mut c_void, w: c_int, h: c_int);
    fn QWidget_setFocus(handle: *mut c_void);
    fn QWidget_setFocusPolicy(handle: *mut c_void, policy: c_int);
    fn QWidget_setAttribute(handle: *mut c_void, attr: c_int, on: bool);
    fn QWidget_update(handle: *mut c_void);
    fn QWidget_rect(handle: *mut c_void, rect: *mut [c_int; 4]);
    fn QWidget_to_QPaintDevice(handle: *mut c_void) -> *mut c_void;
    fn QPainter_Create2(device: *mut c_void) -> *mut c_void;
    fn QPainter_Destroy(handle: *mut c_void);
    fn QPainter_fillRect5(handle: *mut c_void, x: c_int, y: c_int, w: c_int, h: c_int, color: *mut c_void);
    fn QPainter_drawImage9(handle: *mut c_void, x: c_int, y: c_int, image: *mut c_void,
                           sx: c_int, sy: c_int, sw: c_int, sh: c_int, flags: c_uint);
    fn QImage_Create6(data: *const u8, w: c_int, h: c_int, bytes_per_line: c_int,
                      format: c_int, cleanup: *mut c_void, cleanup_info: *mut c_void) -> *mut c_void;
    fn QImage_Destroy(handle: *mut c_void);
    fn QColor_Create3(r: c_int, g: c_int, b: c_int, a: c_int) -> *mut c_void;
    fn QColor_Destroy(handle: *mut c_void);
    fn QObject_hook_Create(handle: *mut c_void) -> *mut c_void;
    fn QObject_hook_Destroy(handle: *mut c_void);
    fn QObject_hook_hook_events(handle: *mut c_void, hook: QHook);
    fn QObject_startTimer(handle: *mut c_void, interval: c_int, timer_type: c_int) -> c_int;
    fn QObject_killTimer(handle: *mut c_void, id: c_int);
    fn QEvent_type(handle: *mut c_void) -> c_int;
    fn QTimerEvent_timerId(handle: *mut c_void) -> c_int;
    fn QInputEvent_modifiers(handle: *mut c_void) -> c_uint;
    fn QMouseEvent_button(handle: *mut c_void) -> c_int;
    fn QKeyEvent_key(handle: *mut c_void) -> c_int;
    fn QKeyEvent_isAutoRepeat(handle: *mut c_void) -> bool;
    fn QKeyEvent_nativeScanCode(handle: *mut c_void) -> c_uint;
}

/// The resolved API, bound once. `None` = no QtPas in this process (a GTK2
/// build of DC): every entry point then just declines.
fn qt() -> Option<&'static QtApi> {
    static API: OnceLock<Option<QtApi>> = OnceLock::new();
    API.get_or_init(QtApi::resolve).as_ref()
}

// ------------------------------------------------------------- per-window ---

/// One open viewer: the shared view-model plus the Qt objects around it.
struct Ctx {
    state: Box<State>,
    widget: Hwnd,
    hook: *mut c_void,
    timer_id: c_int,
}

/// Live viewers, keyed by widget handle - `ListCloseWindow` gets only the
/// handle back, and a handle that is not ours must be left alone. All access
/// is from DC's GUI thread; the mutex is just for the static's soundness.
static WINDOWS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

fn windows() -> std::sync::MutexGuard<'static, Vec<(usize, usize)>> {
    WINDOWS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Apply a gesture and repaint if anything changed.
fn apply(api: &QtApi, ctx: &mut Ctx, action: Action) {
    if apply_action(&mut ctx.state, action) {
        unsafe { (api.QWidget_update)(ctx.widget) };
    }
}

/// Fill the widget with the border colour, then blit the RGBA frame centred at
/// its native (already-scaled) size - mirrors the Windows `paint` / macOS
/// `draw`. Runs inside the widget's paint event (see the module docs), where a
/// `QPainter` on the widget is valid.
unsafe fn paint(api: &QtApi, ctx: &Ctx) {
    let painter = (api.QPainter_Create2)((api.QWidget_to_QPaintDevice)(ctx.widget));
    if painter.is_null() {
        return;
    }
    let mut rc = [0 as c_int; 4]; // left, top, right, bottom (exclusive)
    (api.QWidget_rect)(ctx.widget, &mut rc);
    let (cw, ch) = (rc[2] - rc[0], rc[3] - rc[1]);
    let [r, g, b] = ctx.state.render.border;
    let color = (api.QColor_Create3)(r as c_int, g as c_int, b as c_int, 255);
    (api.QPainter_fillRect5)(painter, 0, 0, cw, ch, color);
    (api.QColor_Destroy)(color);
    let iw = ctx.state.render.width as c_int;
    let ih = ctx.state.render.height as c_int;
    if iw > 0 && ih > 0 && !ctx.state.render.pixels.is_empty() {
        // A zero-copy view of the frame; destroyed before the buffer can change.
        let img = (api.QImage_Create6)(
            ctx.state.render.pixels.as_ptr(),
            iw,
            ih,
            iw * 4,
            QIMAGE_FORMAT_RGBA8888,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
        if !img.is_null() {
            (api.QPainter_drawImage9)(painter, (cw - iw) / 2, (ch - ih) / 2, img, 0, 0, iw, ih, 0);
            (api.QImage_Destroy)(img);
        }
    }
    (api.QPainter_Destroy)(painter);
}

/// Map a key event to an [`Action`]. Returns `None` for keys we do not use, so
/// they propagate up to DC's Lister form (Esc to close, n/p prev-next...).
///   Alt+0..7 fixed border   Alt+8 dominant   Shift+1..6 zoom   1..7 palette
///   Space invert   Enter brightness (mono) / attribute mode (colour)
///
/// Digits are matched by **native scan code**, not by `QKeyEvent::key()`: Qt
/// folds Shift and the layout into `key()` (Shift+1 is `Key_Exclam` on a US
/// layout, and digits move elsewhere on others), which would break the zoom
/// keys - the exact bug the macOS shell had with `charactersIgnoringModifiers`.
/// Both the xcb and wayland platforms report xkb keycodes (evdev + 8), so the
/// main digit row is 10..19 - positional and layout-free, mirroring the
/// Windows VKs and macOS key codes. `key()` remains only as the fallback for a
/// platform that reports no scan code. Control is intentionally transparent
/// (Ctrl+digit acts as the digit - the documented fallback for when DC's
/// Lister eats plain digits), matching the other shells.
unsafe fn key_action(api: &QtApi, event: *mut c_void) -> Option<Action> {
    let mods = (api.QInputEvent_modifiers)(event);
    let shift = mods & QT_SHIFT_MODIFIER != 0;
    let alt = mods & QT_ALT_MODIFIER != 0;
    let scan = (api.QKeyEvent_nativeScanCode)(event);
    let key = (api.QKeyEvent_key)(event);
    // Main-row digit (the numpad has its own scan codes and is intentionally
    // excluded, matching the Windows VKs / macOS key codes).
    let digit: Option<u8> = match scan {
        10..=18 => Some((scan - 9) as u8), // 1..9
        19 => Some(0),
        0 => (0x30..=0x39).contains(&key).then(|| (key - 0x30) as u8),
        _ => None,
    };
    if let Some(a) = digit_action(digit, shift, alt) {
        return Some(a);
    }
    // Space / Enter are our invert / cycle keys, but only unmodified by Alt, so
    // Alt+Space (window menu) and Alt+Enter reach DC instead (mirrors the
    // Windows/macOS shells).
    if alt {
        return None;
    }
    match (scan, key) {
        (65, _) | (0, QT_KEY_SPACE) => Some(Action::ToggleInvert),
        (36, _) | (104, _) | (0, QT_KEY_RETURN) | (0, QT_KEY_ENTER) => Some(Action::Cycle),
        _ => None,
    }
}

/// The `QObject_hook` event filter on the viewer widget. Returning `true`
/// consumes the event; `false` lets Qt deliver it normally (an unconsumed key
/// then propagates to DC's Lister form, like the other shells forward keys).
/// Called directly from C++, so the body is panic-guarded.
unsafe extern "C" fn event_filter(
    data: *mut c_void,
    _sender: *mut c_void,
    event: *mut c_void,
) -> bool {
    guard(false, || {
        let api = match qt() {
            Some(a) => a,
            None => return false, // unreachable: the hook exists only if resolved
        };
        let ctx = &mut *(data as *mut Ctx);
        match (api.QEvent_type)(event) {
            QEVENT_PAINT => {
                paint(api, ctx);
                true
            }
            QEVENT_TIMER => {
                // Advance FLASH; anything else's timer is not ours to eat.
                if ctx.timer_id != 0 && (api.QTimerEvent_timerId)(event) == ctx.timer_id {
                    ctx.state.tick();
                    (api.QWidget_update)(ctx.widget);
                    true
                } else {
                    false
                }
            }
            QEVENT_KEY_PRESS => {
                if let Some(action) = key_action(api, event) {
                    // Ignore OS auto-repeat so a held Space/Enter does not
                    // strobe; consume the key either way - it is ours.
                    if !(api.QKeyEvent_isAutoRepeat)(event) {
                        apply(api, ctx, action);
                    }
                    true
                } else {
                    false
                }
            }
            // A double-click's second press arrives as DblClick, not Press;
            // treat both alike so it cycles the palette twice (as on Windows,
            // where the class has no CS_DBLCLKS).
            QEVENT_MOUSE_PRESS | QEVENT_MOUSE_DBLCLICK => {
                // Grab focus explicitly: consuming the press bypasses Qt's own
                // click-to-focus handling.
                match (api.QMouseEvent_button)(event) {
                    QT_LEFT_BUTTON => {
                        (api.QWidget_setFocus)(ctx.widget);
                        apply(api, ctx, Action::NextPalette);
                        true
                    }
                    QT_RIGHT_BUTTON => {
                        (api.QWidget_setFocus)(ctx.widget);
                        apply(api, ctx, Action::ToggleInvert);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    })
}

// -------------------------------------------------------------- WLX exports ---

/// Open a file and, if it is a 6912/6144 ZX screen, return a `QWidget` child of
/// `parent` showing it. Returns null otherwise (or when the host is not a Qt
/// Double Commander), so DC falls through to the next lister plugin.
///
/// DC's Linux builds pass the path in the system encoding (UTF-8 in practice)
/// and this is Unix, so only the ANSI `ListLoad` is exported, like on macOS.
///
/// # Safety
/// `parent` must be a valid `QWidget*` from DC and `file` a NUL-terminated
/// string. Called by Double Commander across the C ABI.
#[no_mangle]
pub unsafe extern "system" fn ListLoad(parent: Hwnd, file: *const c_char, _flags: c_int) -> Hwnd {
    guard(NULL, || load(parent, file))
}

/// Tear down the widget returned by `ListLoad` (frees its state). A handle
/// this plugin did not create is left untouched.
///
/// # Safety
/// `list_win` must be a handle previously returned by `ListLoad`, or null.
#[no_mangle]
pub unsafe extern "system" fn ListCloseWindow(list_win: Hwnd) {
    guard((), || close(list_win));
}

unsafe fn load(parent: Hwnd, file: *const c_char) -> Hwnd {
    if parent.is_null() || file.is_null() {
        return NULL;
    }
    // Decode/parse/render before touching Qt, so a non-screen just yields null
    // with nothing created. Unix paths are raw bytes, not necessarily UTF-8.
    let bytes = CStr::from_ptr(file).to_bytes();
    if bytes.is_empty() {
        return NULL;
    }
    let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
    let state = match build_state(path) {
        Some(s) => s,
        None => return NULL,
    };
    let api = match qt() {
        Some(a) => a,
        None => return NULL, // not a Qt build of DC
    };
    let has_flash = state.has_flash;

    let widget = (api.QWidget_Create)(parent, 0);
    if widget.is_null() {
        return NULL;
    }
    (api.QWidget_setFocusPolicy)(widget, QT_STRONG_FOCUS);
    // We repaint every pixel (border field + screen), so Qt can skip erasing.
    (api.QWidget_setAttribute)(widget, QT_WA_OPAQUE_PAINT_EVENT, true);
    // Fill the container until DC's first ResizeWindow arrives.
    let mut rc = [0 as c_int; 4];
    (api.QWidget_rect)(parent, &mut rc);
    (api.QWidget_resize)(widget, (rc[2] - rc[0]).max(1), (rc[3] - rc[1]).max(1));

    let ctx = Box::into_raw(Box::new(Ctx {
        state,
        widget,
        hook: core::ptr::null_mut(),
        timer_id: 0,
    }));
    let hook = (api.QObject_hook_Create)(widget);
    (api.QObject_hook_hook_events)(
        hook,
        QHook {
            func: event_filter as *mut c_void,
            data: ctx as *mut c_void,
        },
    );
    (*ctx).hook = hook;
    if has_flash {
        (*ctx).timer_id = (api.QObject_startTimer)(widget, FLASH_MS as c_int, QT_COARSE_TIMER);
    }
    windows().push((widget as usize, ctx as usize));

    (api.QWidget_show)(widget);
    (api.QWidget_setFocus)(widget); // grab keyboard focus so the hotkeys work
    widget
}

unsafe fn close(list_win: Hwnd) {
    if list_win.is_null() {
        return;
    }
    // Only touch handles we created (and only once).
    let ctx = {
        let mut ws = windows();
        match ws.iter().position(|&(w, _)| w == list_win as usize) {
            Some(i) => ws.swap_remove(i).1 as *mut Ctx,
            None => return,
        }
    };
    let api = match qt() {
        Some(a) => a,
        None => return, // unreachable: the window exists only if resolved
    };
    // Unhook first so widget teardown cannot re-enter the filter, then destroy
    // the widget (which also kills its timer) and free the state.
    (api.QObject_hook_Destroy)((*ctx).hook);
    if (*ctx).timer_id != 0 {
        (api.QObject_killTimer)(list_win, (*ctx).timer_id);
    }
    (api.QWidget_Destroy)(list_win);
    drop(Box::from_raw(ctx));
}
