//! The macOS-only parts: reading the camera notch of each screen, and the window level and
//! spaces behaviour that let the island hang from the notch above the menu bar.
//! (Replaces the Electron app's `native/notch-info.m` helper program and `platform.js`:
//! the same AppKit calls, made directly.)
//!
//! Only public AppKit geometry is used, in logical points (not Retina pixels). No camera,
//! screen recording or Accessibility permission is involved.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSFloatingWindowLevel, NSScreen, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior,
};
use tauri::WebviewWindow;

/// A screen that has a notch, as AppKit describes it (in points).
#[derive(Debug, Clone, Copy)]
pub struct ScreenNotch {
    /// The screen's left edge and width: how the matching Tauri monitor is found.
    pub screen_x: f64,
    pub screen_width: f64,
    pub notch_width: f64,
    pub notch_height: f64,
    /// The notch's centre, from the screen's left edge.
    pub center_x: f64,
}

/// Every screen with a notch. Must run on the main thread.
pub fn read_notches(mtm: MainThreadMarker) -> Vec<ScreenNotch> {
    let mut found = Vec::new();
    for screen in NSScreen::screens(mtm).iter() {
        let height = screen.safeAreaInsets().top;
        let left = screen.auxiliaryTopLeftArea();
        let right = screen.auxiliaryTopRightArea();
        let width = right.origin.x - (left.origin.x + left.size.width);
        let empty = |w: f64, h: f64| w <= 0.0 || h <= 0.0;
        if height <= 0.0
            || empty(left.size.width, left.size.height)
            || empty(right.size.width, right.size.height)
            || width <= 0.0
        {
            continue;
        }
        let frame = screen.frame();
        found.push(ScreenNotch {
            screen_x: frame.origin.x,
            screen_width: frame.size.width,
            notch_width: width,
            notch_height: height,
            center_x: ((left.origin.x + left.size.width) + right.origin.x) / 2.0 - frame.origin.x,
        });
    }
    found
}

/// Reads the notches by asking the main thread.
pub fn read_notches_blocking(win: &WebviewWindow) -> Vec<ScreenNotch> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ask = win.run_on_main_thread(move || {
        let notches = MainThreadMarker::new()
            .map(read_notches)
            .unwrap_or_default();
        let _ = tx.send(notches);
    });
    if ask.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default()
}

/// The island appears on every space, including over full-screen apps.
pub fn show_on_all_spaces(win: &WebviewWindow) {
    let target = win.clone();
    let _ = win.run_on_main_thread(move || {
        if let Ok(ptr) = target.ns_window() {
            // SAFETY: Tauri gives the window's NSWindow, alive as long as `target` is, and
            // this runs on the main thread.
            let window: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
        }
    });
}

/// Attached to the notch the island draws over the menu-bar strip, which needs the "status"
/// level. This stays below pop-up menus and system or security windows, unlike the screen
/// saver level. Otherwise it floats like a normal always-on-top window.
pub fn set_level(win: &WebviewWindow, attached: bool) {
    let target = win.clone();
    let _ = win.run_on_main_thread(move || {
        if let Ok(ptr) = target.ns_window() {
            // SAFETY: as above.
            let window: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
            window.setLevel(if attached {
                NSStatusWindowLevel
            } else {
                NSFloatingWindowLevel
            });
        }
    });
}
