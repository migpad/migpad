//! The settings window of macOS: a window of AppKit with the controls of the system — lists,
//! fields with steppers, a check box — as the settings of the applications there are ([ADR 0014]).
//! Its controls do not change the settings themselves: they queue what was chosen, and a task of
//! GPUI takes it, between the events of GPUI rather than inside one of them.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::task::{Poll, Waker};

use gpui::App;
use migpad_core::settings::{FONT_SIZES, Setting, Settings, TAB_WIDTHS};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSEvent,
    NSEventModifierFlags, NSFont, NSFontManager, NSFontTraitMask, NSGridCell, NSGridCellPlacement, NSGridRowAlignment,
    NSGridView, NSMenuItem, NSPopUpButton, NSResponder, NSStackView, NSStepper, NSTextField, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

use crate::settings;
use crate::settings_window::{self, LANGUAGES, THEMES, font_label, language_label, theme_label};
use crate::strings::{Key, Language, Plural, plural, tr};

/// Space around the fields, in points.
const MARGIN: f64 = 20.;
/// The width of a field of a number.
const NUMBER_WIDTH: f64 = 44.;

/// What a control of the window asks for.
enum Change {
    Set(Setting),
    OpenFile,
    /// A field has text that is not a number: it shows the setting again.
    Revert,
    /// The window is back in front: another program may have changed the file of the settings.
    CheckFile,
}

/// An item of the list of fonts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FontItem {
    /// The monospace font of the system, which the settings name by no name.
    System,
    Line,
    Family(String),
}

thread_local! {
    /// What the controls asked for, for the task of GPUI to take, and the waker of the task.
    static CHANGES: RefCell<(VecDeque<Change>, Option<Waker>)> = RefCell::default();
    /// The window, once it was opened: it is kept, hidden, when it closes.
    static WINDOW: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
}

fn push(change: Change) {
    let waker = CHANGES.with_borrow_mut(|(queue, waker)| {
        queue.push_back(change);
        waker.take()
    });
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// The next thing a control asked for, once there is one.
async fn next_change() -> Change {
    std::future::poll_fn(|context| {
        CHANGES.with_borrow_mut(|(queue, waker)| match queue.pop_front() {
            Some(change) => Poll::Ready(change),
            None => {
                *waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
    })
    .await
}

define_class!(
    // SAFETY: `NSWindow` has no requirements of its subclasses that this one breaks, and it does
    // not implement `Drop`.
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MigPadSettingsWindow"]
    /// The window: Cmd+W closes it, as it closes the windows of documents — whose command in the
    /// menu is not available while no window of GPUI is in front.
    struct Window;

    impl Window {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            if closes(event) {
                self.performClose(None);
                true
            } else {
                // SAFETY: the method of the superclass, with its argument.
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }

        #[unsafe(method(becomeKeyWindow))]
        fn become_key_window(&self) {
            // SAFETY: the method of the superclass.
            let _: () = unsafe { msg_send![super(self), becomeKeyWindow] };
            push(Change::CheckFile);
        }
    }
);

/// Whether `event` is Cmd+W: by the character with Command, which is Latin even in the layouts
/// of other scripts.
fn closes(event: &NSEvent) -> bool {
    let flags = event.modifierFlags().intersection(NSEventModifierFlags::DeviceIndependentFlagsMask);
    let modifiers = NSEventModifierFlags::Command
        | NSEventModifierFlags::Shift
        | NSEventModifierFlags::Option
        | NSEventModifierFlags::Control;
    flags.intersection(modifiers) == NSEventModifierFlags::Command
        && event.characters().is_some_and(|characters| characters.to_string().eq_ignore_ascii_case("w"))
}

define_class!(
    // SAFETY: `NSObject` has no requirements of its subclasses, and `Target` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MigPadSettingsTarget"]
    /// What the controls send their actions to: it queues what they ask for.
    struct Target;

    impl Target {
        #[unsafe(method(languageChosen:))]
        fn language_chosen(&self, sender: &NSPopUpButton) {
            if let Some(&language) = item(sender).and_then(|index| LANGUAGES.get(index)) {
                push(Change::Set(Setting::Language(language)));
            }
        }

        #[unsafe(method(themeChosen:))]
        fn theme_chosen(&self, sender: &NSPopUpButton) {
            if let Some(&theme) = item(sender).and_then(|index| THEMES.get(index)) {
                push(Change::Set(Setting::Theme(theme)));
            }
        }

        #[unsafe(method(fontChosen:))]
        fn font_chosen(&self, sender: &NSPopUpButton) {
            let chosen = WINDOW.with(|window| {
                let window = window.try_borrow().ok()?;
                window.as_ref()?.font_items.get(item(sender)?).cloned()
            });
            match chosen {
                Some(FontItem::System) => push(Change::Set(Setting::Font(None))),
                Some(FontItem::Family(family)) => push(Change::Set(Setting::Font(Some(family)))),
                Some(FontItem::Line) | None => {}
            }
        }

        #[unsafe(method(fontSizeChanged:))]
        fn font_size_changed(&self, sender: &NSControl) {
            match number(sender, FONT_SIZES) {
                Some(size) => push(Change::Set(Setting::FontSize(size))),
                None => push(Change::Revert),
            }
        }

        #[unsafe(method(tabWidthChanged:))]
        fn tab_width_changed(&self, sender: &NSControl) {
            match number(sender, TAB_WIDTHS) {
                Some(width) => push(Change::Set(Setting::TabWidth(width))),
                None => push(Change::Revert),
            }
        }

        #[unsafe(method(restoreToggled:))]
        fn restore_toggled(&self, sender: &NSButton) {
            push(Change::Set(Setting::RestoreSession(sender.state() == NSControlStateValueOn)));
        }

        #[unsafe(method(openFile:))]
        fn open_file(&self, _sender: &NSButton) {
            push(Change::OpenFile);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `init` of `NSObject`, which has no other initializer to call.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// The item chosen in a list.
fn item(popup: &NSPopUpButton) -> Option<usize> {
    usize::try_from(popup.indexOfSelectedItem()).ok()
}

/// The number in a field or a stepper, brought within `range`; `None` for text that is not a
/// number.
fn number(control: &NSControl, range: RangeInclusive<u32>) -> Option<u32> {
    let text = control.stringValue().to_string();
    let n: i64 = text.trim().parse().ok()?;
    Some(n.clamp(i64::from(*range.start()), i64::from(*range.end())) as u32)
}

/// The window and its controls.
struct SettingsWindow {
    window: Retained<NSWindow>,
    target: Retained<Target>,
    /// The monospace fonts of the system.
    fonts: Vec<String>,
    /// The items of the list of fonts.
    font_items: Vec<FontItem>,
    /// The language of the labels.
    language: Language,
    controls: Controls,
}

struct Controls {
    language: Retained<NSPopUpButton>,
    theme: Retained<NSPopUpButton>,
    font: Retained<NSPopUpButton>,
    font_size: Retained<NSTextField>,
    font_size_stepper: Retained<NSStepper>,
    tab_width: Retained<NSTextField>,
    tab_width_stepper: Retained<NSStepper>,
    tab_width_unit: Retained<NSTextField>,
    restore: Retained<NSButton>,
}

/// Opens the window, or brings it forward.
pub fn show(cx: &mut App) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let settings = settings::get(cx).clone();
    let made = WINDOW.with_borrow_mut(|window| {
        let made = window.is_none();
        let window = window.get_or_insert_with(|| SettingsWindow::new(&settings, mtm));
        if made {
            window.window.center();
        }
        window.show(&settings, mtm);
        made
    });
    if made {
        // What the controls ask for takes effect between the events of GPUI.
        cx.spawn(async move |cx| {
            loop {
                let change = next_change().await;
                cx.update(|cx| match change {
                    Change::Set(setting) => settings::set(setting, cx),
                    Change::OpenFile => settings::open_file(cx),
                    Change::Revert => refresh(cx),
                    Change::CheckFile => settings::check_file(cx),
                });
            }
        })
        .detach();
    }
    cx.activate(true);
    // The windows of GPUI hear that they are no longer in front as this one comes forward: not
    // while GPUI is being updated, as now.
    cx.spawn(async move |_| {
        WINDOW.with_borrow(|window| {
            if let Some(window) = window {
                window.window.makeKeyAndOrderFront(None);
            }
        });
    })
    .detach();
}

/// The settings changed, or the language: the window shows them, if it was opened.
pub fn refresh(cx: &mut App) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let settings = settings::get(cx).clone();
    WINDOW.with(|window| {
        // Not while the window is being made or shown: it shows the settings then anyway.
        if let Ok(mut window) = window.try_borrow_mut()
            && let Some(window) = window.as_mut()
        {
            window.show(&settings, mtm);
        }
    });
}

impl SettingsWindow {
    fn new(settings: &Settings, mtm: MainThreadMarker) -> Self {
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(400., 300.));
        // SAFETY: the initializer of `NSWindow`, with a style and a backing store it takes.
        let window: Retained<Window> = unsafe {
            msg_send![
                Window::alloc(mtm),
                initWithContentRect: frame,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        let window = Retained::into_super(window);
        // SAFETY: the window is kept here, not released by AppKit when it closes.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setAutorecalculatesKeyViewLoop(true);
        let target = Target::new(mtm);
        let fonts = monospace_fonts(mtm);
        let font_items = font_items(&fonts, settings.font.as_deref());
        let language = Language::current();
        let controls = build(&window, &target, &font_items, mtm);
        SettingsWindow { window, target, fonts, font_items, language, controls }
    }

    /// Shows `settings` in the controls: in the language of the interface now — the controls are
    /// made anew in another one — and with the font of the settings in the list.
    fn show(&mut self, settings: &Settings, mtm: MainThreadMarker) {
        let font_items = font_items(&self.fonts, settings.font.as_deref());
        if self.language != Language::current() || font_items != self.font_items {
            self.language = Language::current();
            self.font_items = font_items;
            self.controls = build(&self.window, &self.target, &self.font_items, mtm);
        }
        let controls = &self.controls;
        let index = |position: Option<usize>| position.map_or(-1, |position| position as isize);
        let language = LANGUAGES.iter().position(|&language| language == settings.language);
        controls.language.selectItemAtIndex(index(language));
        controls.theme.selectItemAtIndex(index(THEMES.iter().position(|&theme| theme == settings.theme)));
        let font = match &settings.font {
            Some(family) => FontItem::Family(family.clone()),
            None => FontItem::System,
        };
        controls.font.selectItemAtIndex(index(self.font_items.iter().position(|item| *item == font)));
        controls.font_size.setIntegerValue(settings.font_size as isize);
        controls.font_size_stepper.setIntegerValue(settings.font_size as isize);
        controls.tab_width.setIntegerValue(settings.tab_width as isize);
        controls.tab_width_stepper.setIntegerValue(settings.tab_width as isize);
        let unit = plural(Plural::SettingsTabWidthUnit, u64::from(settings.tab_width), &[]);
        controls.tab_width_unit.setStringValue(&NSString::from_str(&unit));
        let restore = if settings.restore_session { NSControlStateValueOn } else { NSControlStateValueOff };
        controls.restore.setState(restore);
    }
}

/// The items of the list of fonts: that of the system, a line, and the monospace fonts — with the
/// font of the settings among them if it is not one of them.
fn font_items(fonts: &[String], font: Option<&str>) -> Vec<FontItem> {
    let mut families = fonts.to_vec();
    if let Some(font) = font.filter(|font| !fonts.iter().any(|family| family == font)) {
        families.push(font.to_owned());
        families.sort_by_key(|family| family.to_lowercase());
    }
    [FontItem::System, FontItem::Line].into_iter().chain(families.into_iter().map(FontItem::Family)).collect()
}

/// The families of monospace fonts the system has, by their names, as `NSFontManager` tells them.
fn monospace_fonts(mtm: MainThreadMarker) -> Vec<String> {
    let manager = NSFontManager::sharedFontManager(mtm);
    let fixed_pitch = NSFontTraitMask::FixedPitchFontMask.bits();
    let mut fonts: Vec<String> = manager
        .availableFontFamilies()
        .iter()
        .filter(|family| {
            let Some(members) = manager.availableMembersOfFontFamily(family) else { return false };
            members.iter().any(|member| {
                member.count() > 3 && {
                    let traits = member.objectAtIndex(3);
                    // SAFETY: the fourth item of a member of a family is an `NSNumber`, its traits.
                    let traits: usize = unsafe { msg_send![&*traits, unsignedIntegerValue] };
                    traits & fixed_pitch != 0
                }
            })
        })
        .map(|family| family.to_string())
        .collect();
    fonts.sort_by_key(|family| family.to_lowercase());
    fonts
}

/// Makes the controls of the window, in the language of the interface now, and fits the window to
/// them.
fn build(window: &NSWindow, target: &Target, font_items: &[FontItem], mtm: MainThreadMarker) -> Controls {
    window.setTitle(&NSString::from_str(settings_window::title()));
    let label = |key: Key| NSTextField::labelWithString(&NSString::from_str(tr(key)), mtm);
    let popup = |items: &[String], action: Sel| {
        // A list that pops up rather than pulls down.
        let popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        for item in items {
            popup.addItemWithTitle(&NSString::from_str(item));
        }
        // SAFETY: the target answers the action, and lives as long as the window.
        unsafe {
            popup.setTarget(Some(target));
            popup.setAction(Some(action));
        }
        popup
    };
    let language = popup(&LANGUAGES.map(language_label), sel!(languageChosen:));
    let theme = popup(&THEMES.map(|theme| theme_label(theme).to_owned()), sel!(themeChosen:));
    let font = popup(&[], sel!(fontChosen:));
    for item in font_items {
        match item {
            FontItem::System => font.addItemWithTitle(&NSString::from_str(&font_label(None))),
            FontItem::Family(family) => font.addItemWithTitle(&NSString::from_str(family)),
            FontItem::Line => {
                if let Some(menu) = font.menu() {
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                }
            }
        }
    }
    // The lists are as wide as the widest of them.
    let width = [&language, &theme, &font].iter().map(|popup| popup.fittingSize().width).fold(160., f64::max);
    for popup in [&language, &theme, &font] {
        popup.widthAnchor().constraintEqualToConstant(width).setActive(true);
    }
    let field = |range: (u32, u32), action: Sel| {
        let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        field.widthAnchor().constraintEqualToConstant(NUMBER_WIDTH).setActive(true);
        let stepper = NSStepper::new(mtm);
        stepper.setMinValue(f64::from(range.0));
        stepper.setMaxValue(f64::from(range.1));
        stepper.setIncrement(1.);
        stepper.setValueWraps(false);
        stepper.setAutorepeat(true);
        // SAFETY: the target answers the action, and lives as long as the window. A field sends it
        // when the focus leaves it too.
        unsafe {
            field.setTarget(Some(target));
            field.setAction(Some(action));
            stepper.setTarget(Some(target));
            stepper.setAction(Some(action));
            if let Some(cell) = field.cell() {
                let _: () = msg_send![&*cell, setSendsActionOnEndEditing: true];
            }
        }
        (field, stepper)
    };
    let (font_size, font_size_stepper) = field((*FONT_SIZES.start(), *FONT_SIZES.end()), sel!(fontSizeChanged:));
    let (tab_width, tab_width_stepper) = field((*TAB_WIDTHS.start(), *TAB_WIDTHS.end()), sel!(tabWidthChanged:));
    let tab_width_unit = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    // SAFETY: the target answers the actions, and lives as long as the window.
    let restore = unsafe {
        NSButton::checkboxWithTitle_target_action(
            &NSString::from_str(tr(Key::SettingsRestoreSession)),
            Some(target),
            Some(sel!(restoreToggled:)),
            mtm,
        )
    };
    let note = NSTextField::wrappingLabelWithString(&NSString::from_str(tr(Key::SettingsRestoreNote)), mtm);
    note.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
    note.setTextColor(Some(&NSColor::secondaryLabelColor()));
    note.setPreferredMaxLayoutWidth(width + 60.);
    // SAFETY: the target answers the actions, and lives as long as the window.
    let open = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(tr(Key::SettingsOpenFile)),
            Some(target),
            Some(sel!(openFile:)),
            mtm,
        )
    };

    let stack = |views: &[&NSView]| {
        let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(views), mtm);
        stack.setSpacing(6.);
        Retained::into_super(stack)
    };
    let empty = || NSGridCell::emptyContentView(mtm);
    let label = |key: Key| into_view(&*label(key));
    let font_size_row: [&NSView; 2] = [&font_size, &font_size_stepper];
    let tab_width_row: [&NSView; 3] = [&tab_width, &tab_width_stepper, &tab_width_unit];
    let rows: [[Retained<NSView>; 2]; 8] = [
        [label(Key::SettingsLanguage), into_view(&*language)],
        [label(Key::SettingsTheme), into_view(&*theme)],
        [label(Key::SettingsFont), into_view(&*font)],
        [label(Key::SettingsFontSize), stack(&font_size_row)],
        [label(Key::SettingsTabWidth), stack(&tab_width_row)],
        [label(Key::SettingsStartup), into_view(&*restore)],
        [empty(), into_view(&*note)],
        [empty(), into_view(&*open)],
    ];
    let rows: Vec<Retained<NSArray<NSView>>> = rows.iter().map(|row| NSArray::from_retained_slice(row)).collect();
    let grid = NSGridView::gridViewWithViews(&NSArray::from_retained_slice(&rows), mtm);
    grid.setRowSpacing(10.);
    grid.setColumnSpacing(8.);
    grid.setRowAlignment(NSGridRowAlignment::FirstBaseline);
    grid.columnAtIndex(0).setXPlacement(NSGridCellPlacement::Trailing);
    // The note goes with the check box above it; the button stands apart.
    grid.rowAtIndex(6).setTopPadding(-6.);
    grid.rowAtIndex(7).setTopPadding(10.);

    let size = grid.fittingSize();
    grid.setFrame(NSRect::new(NSPoint::new(MARGIN, MARGIN), size));
    let content = NSView::new(mtm);
    content.addSubview(&grid);
    window.setContentView(Some(&content));
    window.setContentSize(NSSize::new(size.width + 2. * MARGIN, size.height + 2. * MARGIN));
    let first: &NSView = &language;
    window.setInitialFirstResponder(Some(first));
    Controls {
        language,
        theme,
        font,
        font_size,
        font_size_stepper,
        tab_width,
        tab_width_stepper,
        tab_width_unit,
        restore,
    }
}

/// A control as a view, for the grid.
fn into_view(control: &impl AsRef<NSView>) -> Retained<NSView> {
    control.as_ref().retain()
}
