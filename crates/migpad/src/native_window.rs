//! The pointer over the windows of macOS, for a tab dragged out of its window. AppKit sends the
//! moves of the mouse and the release of its button to the window the button was pressed in,
//! whatever window is under the pointer: that window finds where the tab was let go of itself.

use gpui::{App, Pixels, Point, Window, point, px};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSView, NSWindow};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Where the pointer is in `window`, from the top left corner of its content, if `window` is the
/// one in front under the pointer of all the windows on the screens — of MigPad or not.
pub fn pointer_in(window: &Window) -> Option<Point<Pixels>> {
    let mtm = MainThreadMarker::new()?;
    // `Window` has a method of its own by this name.
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw() else { return None };
    // SAFETY: the content view of a window of GPUI, which lives as long as the window.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let ns_window = view.window()?;
    let location = NSEvent::mouseLocation();
    if NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(location, 0, mtm) != ns_window.windowNumber() {
        return None;
    }
    let local = view.convertPoint_fromView(ns_window.convertPointFromScreen(location), None);
    let y = if view.isFlipped() { local.y } else { view.bounds().size.height - local.y };
    Some(point(px(local.x as f32), px(y as f32)))
}

/// Where the pointer is on the screens, from the top left corner of the main one, as GPUI places a
/// window on it: a window opened there is under the pointer on whichever screen it is.
pub fn pointer_on_screens(cx: &App) -> Option<Point<Pixels>> {
    let height = cx.primary_display()?.bounds().size.height;
    let location = NSEvent::mouseLocation();
    Some(point(px(location.x as f32), height - px(location.y as f32)))
}
