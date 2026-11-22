//! Places the GUI's "Monitoring" overlay over the whole primary display.
//!
//! A plain always-on-top window sits at the floating level, under the menu bar and the
//! Dock, and macOS keeps it below the menu bar. Raising it to the status level and
//! setting its frame to the full screen frame makes the tint cover everything.

use dp_platform::OVERLAY_WINDOW_TITLE;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSScreen, NSStatusWindowLevel, NSWindowCollectionBehavior};

/// Raises and resizes the overlay window. No-op off the main thread or without it.
pub(crate) fn cover_primary_display() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // The first screen is the one with the menu bar (primary), same as CGMainDisplayID
    // at launch; its frame includes the menu bar and the Dock.
    let Some(screen) = NSScreen::screens(mtm).firstObject() else {
        return;
    };
    let target = screen.frame();
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        if window.title().to_string() != OVERLAY_WINDOW_TITLE {
            continue;
        }
        if window.level() != NSStatusWindowLevel {
            window.setLevel(NSStatusWindowLevel);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            window.setIgnoresMouseEvents(true);
        }
        if window.frame() != target {
            window.setFrame_display(target, true);
        }
    }
}
