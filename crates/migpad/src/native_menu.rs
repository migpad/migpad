//! The context menus of macOS: menus of the system, as the applications of macOS show them. Windows
//! and Linux have the ones MigPad draws ([`migpad_ui::ContextMenu`]).

use std::cell::Cell;

use gpui::{Pixels, Point};
use migpad_ui::ItemSpec;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSControlStateValueOn, NSEvent, NSMenu, NSMenuItem, NSWindow};
use objc2_foundation::{NSPoint, NSString};

thread_local! {
    /// The tag of the item chosen in the menu being shown.
    static CHOSEN: Cell<Option<isize>> = const { Cell::new(None) };
}

define_class!(
    // SAFETY: `NSObject` has no requirements of its subclasses, and `MenuTarget` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MigPadMenuTarget"]
    /// What the items of a menu send their action to: it notes the item chosen.
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(chosen:))]
        fn chosen(&self, item: &NSMenuItem) {
            CHOSEN.set(Some(item.tag()));
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `init` of `NSObject`, which has no other initializer to call.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// Shows the menu of `items` at `position` in the window under the pointer — the window of MigPad
/// that was clicked, active or not — and waits until it closes. Returns the path of the item
/// chosen, if one was.
///
/// The menu runs a loop of its own until it closes: call it when nothing of GPUI is being updated,
/// as from a task.
pub fn pop_up(items: &[ItemSpec], position: Point<Pixels>) -> Option<Vec<usize>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let under = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(NSEvent::mouseLocation(), 0, mtm);
    let view = app.windowWithWindowNumber(under).or_else(|| app.keyWindow())?.contentView()?;
    let target = MenuTarget::new(mtm);
    let mut paths = Vec::new();
    let menu = build(items, &[], &target, &mut paths, mtm);
    // GPUI counts from the top left corner of the content.
    let (x, y) = (f64::from(f32::from(position.x)), f64::from(f32::from(position.y)));
    let y = if view.isFlipped() { y } else { view.bounds().size.height - y };
    CHOSEN.set(None);
    menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(x, y), Some(&view));
    let tag = CHOSEN.take()?;
    paths.get(usize::try_from(tag).ok()?).cloned()
}

/// The menu of `items`, the items of a menu down `prefix`: each item that acts gets the place of
/// its path in `paths` as its tag.
fn build(
    items: &[ItemSpec],
    prefix: &[usize],
    target: &MenuTarget,
    paths: &mut Vec<Vec<usize>>,
    mtm: MainThreadMarker,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    // Gray is what the items say, not what the responders of AppKit could do.
    menu.setAutoenablesItems(false);
    for (i, item) in items.iter().enumerate() {
        let path = [prefix, &[i]].concat();
        let ns_item = match item {
            ItemSpec::Separator => NSMenuItem::separatorItem(mtm),
            ItemSpec::Action { label, checked, enabled, .. } => {
                let ns_item = new_item(label, Some(sel!(chosen:)), mtm);
                // SAFETY: the target answers `chosen:`; the menu does not keep it, and it lives
                // until the menu closes.
                unsafe { ns_item.setTarget(Some(target)) };
                ns_item.setTag(paths.len() as isize);
                paths.push(path);
                ns_item.setEnabled(*enabled);
                if *checked == Some(true) {
                    ns_item.setState(NSControlStateValueOn);
                }
                ns_item
            }
            ItemSpec::Submenu { label, enabled, items, .. } => {
                let ns_item = new_item(label, None, mtm);
                ns_item.setSubmenu(Some(&build(items, &path, target, paths, mtm)));
                ns_item.setEnabled(*enabled);
                ns_item
            }
        };
        menu.addItem(&ns_item);
    }
    menu
}

fn new_item(label: &str, action: Option<Sel>, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let (title, keys) = (NSString::from_str(label), NSString::from_str(""));
    // SAFETY: `chosen:` is a selector of the target of the item; an item without one does nothing.
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &title, action, &keys) }
}
