//! macOS Cocoa shell for the ZX-screen lister: a custom `NSView` that blits the
//! RGBA frame from [`crate::viewer::model::State`], animates FLASH with an
//! `NSTimer`, and maps key/mouse events to the shared [`Action`] semantics. All
//! the display logic is in the view-model; this file is only the native window.
//!
//! Contract (verified against Double Commander's `uwlxmodule.pas`/`fviewer.pas`
//! and a working macOS WLX plugin): `ListLoad` receives the parent `NSView*` as
//! the handle, must `addSubview:` its own view and return that view (retained
//! +1); DC positions it via `setFrame:` and calls `ListCloseWindow` with the
//! handle to tear it down. `__stdcall` is empty on macOS, so `extern "system"`
//! (== C) is the right ABI.

use core::cell::{Cell, OnceCell};
use core::ffi::{c_char, c_int, c_void, CStr};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBitmapImageRep, NSColor, NSCompositingOperation, NSDeviceRGBColorSpace, NSEvent,
    NSEventModifierFlags, NSGraphicsContext, NSImage, NSImageInterpolation, NSRectFill,
    NSResponder, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSTimer};

use crate::viewer::model::{
    apply_action, build_state, digit_action, guard, Action, State, FLASH_MS,
};

/// On macOS the WLX handle is an opaque `NSView*`.
type Hwnd = *mut c_void;
const NULL: Hwnd = core::ptr::null_mut();

/// Pin this dylib in memory. Double Commander's Cocoa build `dlclose()`s a WLX
/// module (e.g. after a `ListLoad` returns null, or when the plugin list is
/// rebuilt). Our Objective-C class is registered at runtime via objc2, so - unlike
/// a compiled-ObjC plugin - dyld would actually unmap us, leaving the registered
/// class with method pointers into freed memory and making the next `dlopen`'s
/// class registration panic (the viewer would then be silently dead until DC
/// restarts). An `__objc_imageinfo` section marks the image as containing
/// Objective-C, which is exactly what makes dyld keep such images resident, so
/// `dlclose` becomes a no-op - the same treatment compiled-ObjC plugins get free.
#[used]
#[link_section = "__DATA,__objc_imageinfo,regular,no_dead_strip"]
static OBJC_IMAGE_INFO: [u32; 2] = [0, 0]; // { version = 0, flags = 0 }

/// Instance variables. `ivars()` hands out `&Ivars`, so the mutable bits use
/// interior mutability. Everything here is touched only on the main thread (the
/// view is `MainThreadOnly`), so `Cell`/`OnceCell` are sound.
struct Ivars {
    /// Owning raw pointer to the boxed view-model (freed in `ListCloseWindow`).
    state: Cell<*mut State>,
    /// The FLASH timer, kept so it can be invalidated on teardown.
    timer: OnceCell<Retained<NSTimer>>,
}

define_class!(
    // NSView is MainThreadOnly (!Send + !Sync); the subclass inherits that.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZxScreenWlxView"]
    #[ivars = Ivars]
    struct ZxView;

    impl ZxView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        // We fill the whole bounds (border field + screen), so AppKit can skip
        // drawing anything behind us.
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            guard((), || self.draw());
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // Keys we handle are consumed; the rest (Esc to close, n/p prev-next,
            // arrows...) go up the responder chain to DC's Lister form.
            if !guard(false, || self.handle_key(event)) {
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            guard((), || {
                self.grab_focus();
                // Control-click is macOS's secondary-click idiom, so treat it like
                // the right button (invert); a plain left-click cycles the palette.
                if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                    self.apply(Action::ToggleInvert);
                } else {
                    self.apply(Action::NextPalette);
                }
            });
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &NSEvent) {
            guard((), || {
                self.grab_focus();
                self.apply(Action::ToggleInvert); // right-click inverts
            });
        }

        #[unsafe(method(flashTick:))]
        fn flash_tick(&self, _t: &NSTimer) {
            guard((), || {
                if let Some(st) = self.state() {
                    st.tick();
                    self.setNeedsDisplay(true);
                }
            });
        }
    }
);

impl ZxView {
    /// The boxed view-model, or `None` once it has been torn down. The `&mut` is
    /// derived from the ivar's raw pointer, not from `self`, so it does not alias
    /// the `&self` borrow; every use is short-lived and non-overlapping.
    #[allow(clippy::mut_from_ref)]
    fn state(&self) -> Option<&mut State> {
        let p = self.ivars().state.get();
        if p.is_null() {
            None
        } else {
            Some(unsafe { &mut *p })
        }
    }

    /// Apply a gesture and repaint if anything changed.
    fn apply(&self, action: Action) {
        if let Some(st) = self.state() {
            if apply_action(st, action) {
                self.setNeedsDisplay(true);
            }
        }
    }

    fn grab_focus(&self) {
        if let Some(window) = self.window() {
            let v: &NSView = self;
            let r: &NSResponder = v; // NSView derefs to NSResponder
            window.makeFirstResponder(Some(r));
        }
    }

    /// Map a key event to an [`Action`]. Returns whether we consumed it.
    ///   Alt+0..7 fixed border   Alt+8 dominant   Shift+1..6 zoom   1..7 palette
    ///   Space invert   Enter brightness (mono) / attribute mode (colour)
    ///
    /// Digits are matched by **hardware key code**, not by character: macOS's
    /// `charactersIgnoringModifiers` still folds in Shift (so Shift+1 is '!', not
    /// '1') and is layout-dependent, which would make the zoom keys unreachable
    /// and the palette keys wrong on non-US layouts. Positional key codes mirror
    /// the Windows shell's virtual-key handling exactly and are layout-independent.
    fn handle_key(&self, event: &NSEvent) -> bool {
        let mods = event.modifierFlags();
        // Command is never one of our chords - forward Cmd+key to DC so a Cmd+digit
        // that survives the menu's key-equivalent pass does not silently act as a
        // palette/zoom/border key. (Control is intentionally left transparent: it
        // is the documented fallback for when DC's Lister eats plain digits.)
        if mods.contains(NSEventModifierFlags::Command) {
            return false;
        }
        let shift = mods.contains(NSEventModifierFlags::Shift);
        let option = mods.contains(NSEventModifierFlags::Option); // Alt
                                                                  // Main number-row digit for this key code (the row is non-contiguous), or
                                                                  // None. Numpad keys are deliberately excluded, matching the Windows VKs.
        let digit: Option<u8> = match event.keyCode() {
            0x1D => Some(0),
            0x12 => Some(1),
            0x13 => Some(2),
            0x14 => Some(3),
            0x15 => Some(4),
            0x17 => Some(5),
            0x16 => Some(6),
            0x1A => Some(7),
            0x1C => Some(8),
            0x19 => Some(9),
            _ => None,
        };
        // Shared digit+modifier semantics (identical to the Windows shell). Space /
        // Enter are our invert / cycle keys, but only unmodified by Option, so
        // Option+Space/Return are left for DC (mirrors the Windows Alt handling).
        let action = match digit_action(digit, shift, option) {
            Some(a) => a,
            None => match event.keyCode() {
                0x31 if !option => Action::ToggleInvert, // Space
                0x24 | 0x4C if !option => Action::Cycle, // Return / keypad Enter
                _ => return false,
            },
        };
        // The key is ours; consume it, but ignore OS auto-repeat so a held
        // Space/Enter does not strobe invert/cycle (digits would re-apply harmlessly).
        if !event.isARepeat() {
            self.apply(action);
        }
        true
    }

    /// Fill the bounds with the border colour, then blit the RGBA frame centred
    /// at its native (already-scaled) size - a nearest-neighbour, flip-agnostic
    /// mirror of the Windows `paint`.
    fn draw(&self) {
        let bounds = self.bounds();
        let st = match self.state() {
            Some(s) => s,
            None => return,
        };
        let [r, g, b] = st.render.border;
        let col = NSColor::colorWithSRGBRed_green_blue_alpha(
            r as f64 / 255.0,
            g as f64 / 255.0,
            b as f64 / 255.0,
            1.0,
        );
        col.set();
        NSRectFill(bounds);

        let iw = st.render.width as f64;
        let ih = st.render.height as f64;
        if iw <= 0.0 || ih <= 0.0 || st.render.pixels.is_empty() {
            return;
        }
        if let Some(img) = build_image(
            &st.render.pixels,
            st.render.width as isize,
            st.render.height as isize,
        ) {
            if let Some(ctx) = NSGraphicsContext::currentContext() {
                ctx.setImageInterpolation(NSImageInterpolation::None); // crisp pixels
            }
            let ox = ((bounds.size.width - iw) / 2.0).floor();
            let oy = ((bounds.size.height - ih) / 2.0).floor();
            let dest = NSRect::new(NSPoint::new(ox, oy), NSSize::new(iw, ih));
            let src = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(iw, ih));
            img.drawInRect_fromRect_operation_fraction(
                dest,
                src,
                NSCompositingOperation::Copy,
                1.0,
            );
        }
    }

    fn start_timer(&self) {
        let v: &NSView = self;
        let target: &AnyObject = v.as_ref();
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                FLASH_MS as f64 / 1000.0,
                target,
                sel!(flashTick:),
                None,
                true,
            )
        };
        let _ = self.ivars().timer.set(timer);
    }

    fn stop_timer(&self) {
        if let Some(t) = self.ivars().timer.get() {
            t.invalidate();
        }
    }
}

/// Build an `NSImage` from a top-row-first RGBA buffer. The bitmap rep references
/// the bytes (no copy), so `rgba` must stay valid for the draw - which it does,
/// as this is rebuilt from the live buffer inside each `drawRect:`.
fn build_image(rgba: &[u8], w: isize, h: isize) -> Option<Retained<NSImage>> {
    let mut plane: *mut u8 = rgba.as_ptr() as *mut u8;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            &mut plane,
            w,
            h,
            8,  // bits per sample
            4,  // samples per pixel (RGBA)
            true,
            false,
            NSDeviceRGBColorSpace,
            w * 4, // bytes per row
            32,    // bits per pixel
        )
    }?;
    let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(w as f64, h as f64));
    img.addRepresentation(&rep);
    Some(img)
}

// -------------------------------------------------------------- WLX exports ---

/// Open a file and, if it is a 6912/6144 ZX screen, return a native view showing
/// it (added as a subview of `parent`, retained +1). Returns null otherwise, so
/// Double Commander falls through to the next lister plugin.
///
/// Double Commander on macOS passes the path in the system encoding (UTF-8) and
/// does not export a wide entry point, so only the ANSI `ListLoad` is provided.
///
/// # Safety
/// `parent` must be a valid `NSView*` from DC and `file` a NUL-terminated string.
#[no_mangle]
pub unsafe extern "system" fn ListLoad(parent: Hwnd, file: *const c_char, _flags: c_int) -> Hwnd {
    guard(NULL, || load(parent, file))
}

/// Tear down the view returned by `ListLoad` (removes it and frees its state).
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
    // DC calls the lister entry points on the GUI/main thread; bail if not (the
    // NSView is main-thread-only and this is where we create it).
    let mtm = match MainThreadMarker::new() {
        Some(m) => m,
        None => return NULL,
    };
    let cstr = CStr::from_ptr(file);
    let path = match cstr.to_str() {
        Ok(s) => PathBuf::from(s),
        Err(_) => PathBuf::from(String::from_utf8_lossy(cstr.to_bytes()).into_owned()),
    };
    // Decode/parse/render before touching AppKit, so a non-screen just yields null
    // with nothing created.
    let state = match build_state(&path) {
        Some(s) => s,
        None => return NULL,
    };
    let has_flash = state.has_flash;

    let parent_view: &NSView = &*(parent as *const NSView);
    let frame = parent_view.bounds();

    let this = mtm.alloc::<ZxView>();
    let this = this.set_ivars(Ivars {
        state: Cell::new(core::ptr::null_mut()),
        timer: OnceCell::new(),
    });
    let view: Retained<ZxView> = msg_send![super(this), initWithFrame: frame];

    view.ivars().state.set(Box::into_raw(state));

    // Hand DC a closable handle up front: consume the +1 into a raw pointer, then
    // attach / arm the timer / grab focus through a borrow, wrapped in `guard`.
    // Even if one of those steps panicked, DC still receives a valid handle it can
    // `ListCloseWindow` (which frees the State and detaches the view), so a late
    // panic can never orphan the subview or leak the boxed State.
    let raw = Retained::into_raw(view);
    guard((), || unsafe {
        let v: &ZxView = &*raw;
        parent_view.addSubview(v);
        if has_flash {
            v.start_timer();
        }
        v.grab_focus();
    });
    raw as Hwnd
}

unsafe fn close(list_win: Hwnd) {
    if list_win.is_null() {
        return;
    }
    // NSView/NSTimer are main-thread-only and DC calls this on the main thread.
    // If ever called off-main, leak deliberately rather than release off-main (UB).
    if MainThreadMarker::new().is_none() {
        return;
    }
    let view: Retained<ZxView> = match Retained::from_raw(list_win as *mut ZxView) {
        Some(v) => v,
        None => return,
    };
    view.stop_timer();
    // Null the pointer before freeing, so any late draw sees no state.
    let p = view.ivars().state.replace(core::ptr::null_mut());
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
    view.removeFromSuperview();
    drop(view); // releases the +1 -> deallocs (superview no longer retains it)
}
