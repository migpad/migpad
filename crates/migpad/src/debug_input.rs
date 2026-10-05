//! Debug builds only: input read from the `MIGPAD_DEBUG_INPUT` environment variable and
//! dispatched through GPUI as if the user typed and clicked. Key bindings, actions and mouse
//! handlers run as for real input, without the input layer of the system: a way to check the
//! interface with screenshots.
//!
//! Steps are separated by spaces:
//!
//! - a keystroke, such as `down`, `shift-end` or `cmd-a`;
//! - `click:X,Y`, or `click:X,Y,N` for N clicks;
//! - `press:X,Y`, `move:X,Y` with the button held, `release:X,Y`;
//! - `wait:MS`.
//!
//! `shift-click` and `shift-press` hold Shift. Coordinates are pixels from the top left corner of
//! the content of the window.

use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, point, px,
};

/// The pause before the first step, while the window shows its first frame.
const START: Duration = Duration::from_millis(500);
/// The pause after each step.
const PAUSE: Duration = Duration::from_millis(50);

enum Step {
    Key(Keystroke),
    Mouse(Vec<PlatformInput>),
    Wait(Duration),
}

/// Plays the steps of `MIGPAD_DEBUG_INPUT`, if it is set, in `window`; release builds ignore it.
pub fn play(window: AnyWindowHandle, cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(script) = std::env::var("MIGPAD_DEBUG_INPUT") else { return };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(START).await;
        for step in script.split_whitespace() {
            match parse(step) {
                Some(Step::Key(keystroke)) => {
                    let _ = window.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx));
                }
                Some(Step::Mouse(events)) => {
                    for event in events {
                        let _ = window.update(cx, |_, window, cx| window.dispatch_event(event, cx));
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

fn parse(step: &str) -> Option<Step> {
    if let Some(ms) = step.strip_prefix("wait:") {
        return ms.parse().ok().map(|ms| Step::Wait(Duration::from_millis(ms)));
    }
    let (modifiers, mouse) = match step.strip_prefix("shift-") {
        Some(rest) if rest.starts_with("click:") || rest.starts_with("press:") => (Modifiers::shift(), rest),
        _ => (Modifiers::none(), step),
    };
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
