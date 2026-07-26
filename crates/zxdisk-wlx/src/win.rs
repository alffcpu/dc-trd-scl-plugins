//! Windows shell for the ZX-screen lister: a `WS_CHILD` window that blits the
//! RGBA frame from [`crate::viewer::model::State`] with GDI (double-buffered), an
//! FLASH timer, and palette/zoom/invert hotkeys. All the display logic is in the
//! shared view-model; this file is only the native window and event plumbing.

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::os::raw::{c_char, c_int};
use std::os::windows::ffi::OsStringExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Once;

use windows_sys::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Globalization::MultiByteToWideChar;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, GetStockObject, InvalidateRect, SelectObject, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLACK_BRUSH, DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetModuleHandleW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetParent, GetWindowLongPtrW,
    KillTimer, RegisterClassW, SendMessageW, SetTimer, SetWindowLongPtrW, UnregisterClassW,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, MA_ACTIVATE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_MOUSEACTIVATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_SYSCHAR, WM_SYSKEYDOWN, WM_TIMER,
    WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

use crate::viewer::model::{apply_action, build_state, digit_action, guard, Action, State, FLASH_MS};

const TIMER_ID: usize = 1;
const VK_SPACE: u32 = 0x20;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: i32 = 0x10;
const VK_MENU: i32 = 0x12; // Alt
const NULL_HWND: HWND = core::ptr::null_mut();

fn class_name() -> Vec<u16> {
    "ZxScreenWlxWindow\0".encode_utf16().collect()
}

unsafe fn state_mut<'a>(hwnd: HWND) -> Option<&'a mut State> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if raw == 0 {
        None
    } else {
        Some(&mut *(raw as *mut State))
    }
}

// ------------------------------------------------------------------ window ---

static REGISTER: Once = Once::new();

/// This DLL's own module handle. The window class must be registered with it (not
/// the host EXE's), and unregistered when the DLL unloads - otherwise a later
/// reload would leave a stale class whose `wndproc` points into an unmapped copy
/// of this DLL and crash DC.
static DLL_MODULE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

const DLL_PROCESS_DETACH: u32 = 0;

/// The module handle to register the window class under. Resolved from the
/// address of our own `wndproc`, so it is THIS DLL's handle (not the host EXE's).
/// Cached; falls back to the process module only if the lookup ever fails.
fn instance() -> HINSTANCE {
    let cached = DLL_MODULE.load(Ordering::Acquire);
    if !cached.is_null() {
        return cached;
    }
    let mut hmod: HMODULE = core::ptr::null_mut();
    unsafe {
        let ok = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            wndproc as *const c_void as *const u16,
            &mut hmod,
        );
        if ok == 0 || hmod.is_null() {
            hmod = GetModuleHandleW(core::ptr::null());
        }
    }
    DLL_MODULE.store(hmod, Ordering::Release);
    hmod
}

/// The C runtime calls this on load/unload; on unload we unregister the window
/// class so a later reload of this DLL cannot inherit a stale class whose
/// `wndproc` points into the old (freed) mapping.
#[no_mangle]
unsafe extern "system" fn DllMain(_hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_DETACH && REGISTER.is_completed() {
        let cn = class_name();
        UnregisterClassW(cn.as_ptr(), instance());
    }
    1
}

unsafe fn register_class() {
    REGISTER.call_once(|| {
        let inst = instance();
        let cn = class_name();
        // Defensively clear any same-named class a previous load left behind.
        UnregisterClassW(cn.as_ptr(), inst);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: core::ptr::null_mut(),
            hCursor: core::ptr::null_mut(),
            hbrBackground: core::ptr::null_mut(),
            lpszMenuName: core::ptr::null(),
            lpszClassName: cn.as_ptr(),
        };
        RegisterClassW(&wc);
    });
}

unsafe fn create_window(parent: HWND) -> HWND {
    register_class();
    let cn = class_name();
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(parent, &mut rc);
    let w = (rc.right - rc.left).max(1);
    let h = (rc.bottom - rc.top).max(1);
    let title = [0u16; 1];
    CreateWindowExW(
        0,
        cn.as_ptr(),
        title.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        0,
        0,
        w,
        h,
        parent,
        core::ptr::null_mut(),
        instance(),
        core::ptr::null(),
    )
}

/// Map a virtual-key press (modifiers read live via `GetKeyState`) to a viewer
/// [`Action`], or `None` for keys we do not use. The digit+modifier semantics
/// live in the shared [`digit_action`] so Windows and macOS stay in lockstep.
///   Alt+0..7 fixed border colour   Alt+8 dominant   Shift+1..6 zoom   1..7 palette
///   Space invert   Enter brightness (mono) / attribute mode (colour)
unsafe fn key_action(vk: u32) -> Option<Action> {
    let shift = GetKeyState(VK_SHIFT) < 0;
    let alt = GetKeyState(VK_MENU) < 0;
    // VK 0x30..0x39 are the main-row digits 0..9 (the numpad has its own VKs and
    // is intentionally excluded).
    let digit = (0x30..=0x39).contains(&vk).then(|| (vk - 0x30) as u8);
    if let Some(a) = digit_action(digit, shift, alt) {
        return Some(a);
    }
    // Space / Enter are our invert / cycle keys, but only unmodified by Alt, so
    // Alt+Space (the window's system menu) and Alt+Enter reach DC's default
    // handling instead of being swallowed.
    if alt {
        return None;
    }
    match vk {
        VK_SPACE => Some(Action::ToggleInvert),
        VK_RETURN => Some(Action::Cycle),
        _ => None,
    }
}

/// Window procedure. Called directly by user32, so any panic escaping it would
/// abort the whole Double Commander process (Rust 1.81+). Catch it and fall back
/// to the default handling instead.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| wndproc_impl(hwnd, msg, wp, lp))) {
        Ok(v) => v,
        Err(_) => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn wndproc_impl(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1, // painted fully in WM_PAINT; skip flicker
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_TIMER => {
            if let Some(st) = state_mut(hwnd) {
                st.tick();
                InvalidateRect(hwnd, core::ptr::null(), 0);
            }
            0
        }
        // Alt+<key> arrives as WM_SYSKEYDOWN, the rest as WM_KEYDOWN; both map the
        // same way.
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if let Some(action) = key_action(wp as u32) {
                // Ignore OS auto-repeat (lParam bit 30 = previous key state) so a
                // held Space/Enter does not strobe; a repeated digit is a no-op.
                if (lp & (1 << 30)) == 0 {
                    if let Some(st) = state_mut(hwnd) {
                        if apply_action(st, action) {
                            InvalidateRect(hwnd, core::ptr::null(), 0);
                        }
                    }
                }
                0 // the key is ours - consume it whether or not the state changed
            } else {
                // Keys we do not use (Esc to close, n/p prev-next, Alt+Space,
                // arrows...) must reach DC's Lister form; since focus is on us,
                // forward them.
                let parent = GetParent(hwnd);
                if !parent.is_null() {
                    SendMessageW(parent, msg, wp, lp)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
        }
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            if let Some(st) = state_mut(hwnd) {
                if apply_action(st, Action::NextPalette) {
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
            }
            0
        }
        WM_RBUTTONDOWN => {
            SetFocus(hwnd);
            if let Some(st) = state_mut(hwnd) {
                if apply_action(st, Action::ToggleInvert) {
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
            }
            0
        }
        WM_MOUSEACTIVATE => MA_ACTIVATE as LRESULT,
        WM_SYSCHAR => {
            // TranslateMessage pairs an Alt+digit (a border hotkey we consume at
            // WM_SYSKEYDOWN) with a WM_SYSCHAR; swallow the digit ones so the
            // default handler does not beep for an "unmatched menu mnemonic".
            // Other Alt chords (Alt+Space, Alt+F, ...) fall through to the default
            // menu handling. Test the FULL UTF-16 code unit, not `wp as u8`: on a
            // Cyrillic layout а..й are U+0430..U+0439, whose low byte is 0x30..0x39,
            // so a truncating test would wrongly swallow those menu mnemonics.
            if matches!(wp, 0x30..=0x39) {
                0
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if raw != 0 {
                KillTimer(hwnd, TIMER_ID);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw as *mut State));
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = core::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    if !hdc.is_null() {
        let mut rc: RECT = core::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let cw = rc.right - rc.left;
        let ch = rc.bottom - rc.top;
        // Double-buffer: compose in a memory DC, blit once -> no flicker. The
        // whole window is filled with the border colour so the framed screen
        // sits in a seamless border field.
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, cw.max(1), ch.max(1));
        let old = SelectObject(mem, bmp as _);
        if let Some(st) = state_mut(hwnd) {
            let [r, g, b] = st.render.border;
            let brush = CreateSolidBrush((r as u32) | ((g as u32) << 8) | ((b as u32) << 16));
            FillRect(mem, &rc, brush);
            DeleteObject(brush as _);
            let iw = st.render.width as i32;
            let ih = st.render.height as i32;
            // st.bgra is the render frame pre-converted to BGRA (rebuilt in
            // rerender), so a repaint is just a blit - no per-paint conversion.
            if iw > 0 && ih > 0 && !st.bgra.is_empty() {
                let ox = (cw - iw) / 2;
                let oy = (ch - ih) / 2;
                let bmi = bitmap_info(iw, ih);
                StretchDIBits(
                    mem, ox, oy, iw, ih, 0, 0, iw, ih,
                    st.bgra.as_ptr() as *const c_void, &bmi, DIB_RGB_COLORS, SRCCOPY,
                );
            }
        } else {
            FillRect(mem, &rc, GetStockObject(BLACK_BRUSH) as _);
        }
        BitBlt(hdc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
    }
    EndPaint(hwnd, &ps);
}

fn bitmap_info(w: i32, h: i32) -> BITMAPINFO {
    let mut bmi: BITMAPINFO = unsafe { core::mem::zeroed() };
    bmi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // negative = top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB as u32;
    bmi
}

// -------------------------------------------------------------- WLX exports ---

/// Open a file (Unicode path) and, if it is a ZX screen, return a child window
/// showing it. Returns null so DC falls back when the size is not 6912/6144.
///
/// # Safety
/// `file` must be a valid NUL-terminated wide string or null; `parent` a valid
/// window handle. Called by Double Commander across the C ABI.
#[no_mangle]
pub unsafe extern "system" fn ListLoadW(parent: HWND, file: *const u16, _flags: c_int) -> HWND {
    guard(NULL_HWND, || load(parent, wide_to_path(file).as_deref()))
}

/// ANSI-path fallback (Double Commander uses the wide entry on Windows).
///
/// # Safety
/// `file` must be a valid NUL-terminated string or null; `parent` a valid handle.
#[no_mangle]
pub unsafe extern "system" fn ListLoad(parent: HWND, file: *const c_char, _flags: c_int) -> HWND {
    guard(NULL_HWND, || load(parent, ansi_to_path(file).as_deref()))
}

/// Destroy the window returned by `ListLoad`/`ListLoadW` (frees its `State`).
///
/// # Safety
/// `list_win` must be a handle previously returned by this plugin, or null.
#[no_mangle]
pub unsafe extern "system" fn ListCloseWindow(list_win: HWND) {
    guard((), || {
        if !list_win.is_null() {
            DestroyWindow(list_win); // -> WM_NCDESTROY frees the State
        }
    });
}

unsafe fn load(parent: HWND, path: Option<&Path>) -> HWND {
    let path = match path {
        Some(p) => p,
        None => return NULL_HWND,
    };
    // Build and render the state BEFORE creating the window, so if the (allocating)
    // render ever panics the guard returns NULL with no half-built child window
    // orphaned (DC only calls ListCloseWindow for a non-null return).
    let state = match build_state(path) {
        Some(s) => s,
        None => return NULL_HWND,
    };
    let has_flash = state.has_flash;
    let hwnd = create_window(parent);
    if hwnd.is_null() {
        return NULL_HWND; // `state` drops here - nothing leaked
    }
    // The value arg is i32 on 32-bit Windows (SetWindowLongW) and isize on 64-bit;
    // `as _` casts the pointer to whichever the platform expects.
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as _);
    if has_flash {
        SetTimer(hwnd, TIMER_ID, FLASH_MS, None);
    }
    SetFocus(hwnd); // try to grab keyboard focus so the hotkeys work
    hwnd
}

unsafe fn wide_to_path(p: *const u16) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(p, len);
    Some(PathBuf::from(std::ffi::OsString::from_wide(slice)))
}

unsafe fn ansi_to_path(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // DC's non-Unicode ListLoad hands us the path in the system ANSI code page
    // (CP1251 etc.), not UTF-8; decode it properly so non-ASCII temp paths work.
    let bytes = std::ffi::CStr::from_ptr(p).to_bytes();
    if bytes.is_empty() {
        return None;
    }
    let n = MultiByteToWideChar(0, 0, bytes.as_ptr(), bytes.len() as i32, core::ptr::null_mut(), 0);
    if n <= 0 {
        return Some(PathBuf::from(String::from_utf8_lossy(bytes).into_owned()));
    }
    let mut wide = vec![0u16; n as usize];
    MultiByteToWideChar(0, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), n);
    Some(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
}
