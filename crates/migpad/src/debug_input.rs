//! Debug builds only: input read from the `MIGPAD_DEBUG_INPUT` environment variable and
//! dispatched through GPUI as if the user typed and clicked. Key bindings, actions and mouse
//! handlers run as for real input, without the input layer of the system: a way to check the
//! interface with screenshots.
//!
//! Steps are separated by spaces:
//!
//! - a keystroke, such as `down`, `shift-end` or `cmd-a`;
//! - `type:TEXT`: text from the system, as typed or committed by an input method, to the view with
//!   the focus: the document, or a field of the find bar;
//! - `mark:TEXT`: text an input method composes, and `unmark` to take it as it is;
//! - `action:NAME`: an action by its name, such as `action:view::ToggleInvisibles`;
//! - `click:X,Y`, or `click:X,Y,N` for N clicks;
//! - `press:X,Y`, `move:X,Y` with the button held, `release:X,Y`;
//! - `wait:MS`.
//!
//! `\s` in a text is a space. `shift-click` and `shift-press` hold Shift. Coordinates are pixels
//! from the top left corner of the content of the window.

use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, EntityInputHandler, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, WindowHandle, point, px,
};

use crate::workspace::Workspace;

/// The pause before the first step, while the window shows its first frame.
const START: Duration = Duration::from_millis(500);
/// The pause after each step.
const PAUSE: Duration = Duration::from_millis(50);

enum Step {
    Key(Keystroke),
    Type(String),
    Mark(String),
    Unmark,
    Action(String),
    Mouse(Vec<PlatformInput>),
    Wait(Duration),
}

/// Plays the steps of `MIGPAD_DEBUG_INPUT`, if it is set, in `window`; release builds ignore it.
/// Text goes to the document of the window.
pub fn play(window: WindowHandle<Workspace>, cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(script) = std::env::var("MIGPAD_DEBUG_INPUT") else { return };
    // Events go through the window without holding its view, which handles them.
    let events = AnyWindowHandle::from(window);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(START).await;
        for step in script.split_whitespace() {
            match parse(step) {
                Some(Step::Key(keystroke)) => {
                    let _ = events.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx));
                }
                Some(Step::Type(text)) => {
                    let _ = window.update(cx, |workspace, window, cx| {
                        let target = workspace.input_target(window, cx);
                        target.update(cx, |view, cx| view.replace_text_in_range(None, &text, window, cx))
                    });
                }
                Some(Step::Mark(text)) => {
                    let _ = window.update(cx, |workspace, window, cx| {
                        let target = workspace.input_target(window, cx);
                        target.update(cx, |view, cx| view.replace_and_mark_text_in_range(None, &text, None, window, cx))
                    });
                }
                Some(Step::Unmark) => {
                    let _ = window.update(cx, |workspace, window, cx| {
                        let target = workspace.input_target(window, cx);
                        target.update(cx, |view, cx| view.unmark_text(window, cx))
                    });
                }
                Some(Step::Action(name)) => {
                    let _ = events.update(cx, |_, window, cx| match cx.build_action(&name, None) {
                        Ok(action) => window.dispatch_action(action, cx),
                        Err(error) => eprintln!("MIGPAD_DEBUG_INPUT: {error}"),
                    });
                }
                Some(Step::Mouse(input)) => {
                    for event in input {
                        let _ = events.update(cx, |_, window, cx| window.dispatch_event(event, cx));
                    }
                }
                Some(Step::Wait(time)) => cx.background_executor().timer(time).await,
                None => eprintln!("MIGPAD_DEBUG_INPUT: cannot read the step {step:?}"),
            }
            cx.background_executor().timer(PAUSE).await;
        }
    })
    .detach();
}

fn parse_mouse_modifiers(step: &str) -> (Modifiers, &str) {
    let mouse_kinds = ["click:", "press:", "move:", "release:"];
    let is_mouse = |s: &str| mouse_kinds.iter().any(|k| s.starts_with(k));
    let mut modifiers = Modifiers::none();
    let mut rest = step;
    loop {
        if let Some(tail) = rest.strip_prefix("shift-").filter(|t| is_mouse(t)) {
            modifiers.shift = true;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("alt-").filter(|t| is_mouse(t)) {
            modifiers.alt = true;
            rest = tail;
        } else {
            break;
        }
    }
    (modifiers, rest)
}

fn parse(step: &str) -> Option<Step> {
    if let Some(ms) = step.strip_prefix("wait:") {
        return ms.parse().ok().map(|ms| Step::Wait(Duration::from_millis(ms)));
    }
    if let Some(text) = step.strip_prefix("type:") {
        return Some(Step::Type(text.replace("\\s", " ")));
    }
    if let Some(text) = step.strip_prefix("mark:") {
        return Some(Step::Mark(text.replace("\\s", " ")));
    }
    if step == "unmark" {
        return Some(Step::Unmark);
    }
    if let Some(name) = step.strip_prefix("action:") {
        return Some(Step::Action(name.to_owned()));
    }
    let (modifiers, mouse) = parse_mouse_modifiers(step);
    let Some((kind, args)) = mouse.split_once(':') else { return Keystroke::parse(step).ok().map(Step::Key) };
    let numbers = args.split(',').map(|n| n.parse::<f32>().ok()).collect::<Option<Vec<_>>>()?;
    let at = point(px(*numbers.first()?), px(*numbers.get(1)?));
    let events = match kind {
        "click" => {
            let count = numbers.get(2).map_or(1, |&count| count as usize);
            vec![down(at, count, modifiers), up(at, count, modifiers)]
        }
        "press" => vec![down(at, 1, modifiers)],
        "move" => vec![PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: Some(MouseButton::Left),
            modifiers,
        })],
        "release" => vec![up(at, 1, modifiers)],
        _ => return None,
    };
    Some(Step::Mouse(events))
}

fn down(position: Point<Pixels>, click_count: usize, modifiers: Modifiers) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers,
        click_count,
        first_mouse: false,
    })
}

fn up(position: Point<Pixels>, click_count: usize, modifiers: Modifiers) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, modifiers, click_count })
}
