//! A tab dragged out of its window on macOS. AppKit sends the moves of the mouse and the release of
//! its button to the window the button was pressed in, whatever window is under the pointer: that
//! window finds where the tab goes itself. GPUI draws the dragged tab only in that window: outside
//! of it, a window of the system draws it.

use std::cell::RefCell;

use gpui::{App, Pixels, Point, Window, point, px};
use migpad_ui::theme::theme;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSEvent, NSFont, NSPopUpMenuWindowLevel, NSScreen, NSTextField,
    NSTitlePosition, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The padding of the dragged tab around its title, as GPUI draws it in its window.
const PADDING: NSSize = NSSize::new(12., 6.);
/// Where the dragged tab is from the pointer, right and down: the line between the tabs where it
/// would go stays in sight.
const OFFSET: NSPoint = NSPoint::new(10., 14.);

thread_local! {
    /// The window of the tab dragged outside its window, while it is.
    static DRAGGED: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

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
    // The window itself, or the question over it.
    let under = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(location, 0, mtm);
    let sheet = ns_window.attachedSheet().map(|sheet| sheet.windowNumber());
    if under != ns_window.windowNumber() && Some(under) != sheet {
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

/// `origin`, the top left corner of a window on the screens as [`pointer_on_screens`] counts, moved
/// down out from under the menu bar of the screen under the pointer: GPUI places a window as it is
/// told, and one with its title bar there could not be moved.
pub fn title_bar_in_sight(origin: Point<Pixels>, cx: &App) -> Point<Pixels> {
    let (Some(mtm), Some(main)) = (MainThreadMarker::new(), cx.primary_display()) else { return origin };
    let location = NSEvent::mouseLocation();
    let screen = NSScreen::screens(mtm).iter().find(|screen| {
        let frame = screen.frame();
        (frame.origin.x..frame.origin.x + frame.size.width).contains(&location.x)
            && (frame.origin.y..frame.origin.y + frame.size.height).contains(&location.y)
    });
    let Some(screen) = screen else { return origin };
    let visible = screen.visibleFrame();
    let top = main.bounds().size.height - px((visible.origin.y + visible.size.height) as f32);
    point(origin.x, origin.y.max(top))
}

/// Shows the tab `title`, dragged outside its window, by the pointer: in a window of its own above
/// the others, which the mouse goes through — the window under the pointer is the one under the
/// tab.
pub fn show_dragged_tab(title: &str, cx: &App) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    DRAGGED.with_borrow_mut(|dragged| {
        let window = dragged.get_or_insert_with(|| dragged_tab(title, cx, mtm));
        let location = NSEvent::mouseLocation();
        window.setFrameTopLeftPoint(NSPoint::new(location.x + OFFSET.x, location.y - OFFSET.y));
    });
}

/// Hides the tab dragged outside its window: it is back in it, or the drag is over.
pub fn hide_dragged_tab() {
    if let Some(window) = DRAGGED.take() {
        window.orderOut(None);
    }
}

/// A window of the tab `title`, drawn as GPUI draws a dragged tab: the title on the color of the
/// active tab, in a rounded border.
fn dragged_tab(title: &str, cx: &App, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let theme = theme(cx);
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(f32::from(migpad_ui::text_size()).into())));
    label.setTextColor(Some(&color(theme.text)));
    let text = label.fittingSize();
    let size = NSSize::new(text.width + 2. * PADDING.width, text.height + 2. * PADDING.height);
    let frame = NSRect::new(NSPoint::new(0., 0.), size);
    let plate = NSBox::new(mtm);
    plate.setBoxType(NSBoxType::Custom);
    plate.setTitlePosition(NSTitlePosition::NoTitle);
    plate.setContentViewMargins(NSSize::new(0., 0.));
    plate.setCornerRadius(4.);
    plate.setBorderWidth(1.);
    plate.setBorderColor(&color(theme.border));
    plate.setFillColor(&color(theme.tab_active));
    plate.setFrame(frame);
    if let Some(content) = plate.contentView() {
        label.setFrame(NSRect::new(NSPoint::new(PADDING.width, PADDING.height), text));
        content.addSubview(&label);
    }
    // SAFETY: the initializer of `NSWindow`, with a style and a backing store it takes.
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            NSWindow::alloc(mtm),
            initWithContentRect: frame,
            styleMask: NSWindowStyleMask::Borderless,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    // SAFETY: the window is kept here, not released by AppKit when it closes.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(true);
    window.setAlphaValue(0.9);
    window.setIgnoresMouseEvents(true);
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setContentView(Some(&plate));
    window.orderFrontRegardless();
    window
}

/// A color of the theme, `0xRRGGBB`.
fn color(rgb: u32) -> Retained<NSColor> {
    let channel = |shift: u32| f64::from((rgb >> shift) & 0xff) / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.)
}
