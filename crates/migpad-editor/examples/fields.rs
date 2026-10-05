//! Input fields on the single-line editor view, as a find bar would have them: the element around
//! the fields binds Enter to what they are for and Tab to moving the focus.
//!
//! ```sh
//! cargo run -p migpad-editor --example fields
//! ```

use gpui::{
    App, AppContext, Bounds, Context, Entity, Focusable, KeyBinding, Render, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use migpad_editor::EditorView;

actions!(fields, [Submit, SubmitBackwards, FocusNext, FocusPrevious]);

/// The key context of the window.
const CONTEXT: &str = "Fields";

struct Fields {
    find: Entity<EditorView>,
    replace: Entity<EditorView>,
    /// What the last Enter did.
    status: SharedString,
}

impl Fields {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Fields {
            find: cx.new(|cx| EditorView::single_line(window, cx)),
            replace: cx.new(|cx| EditorView::single_line(window, cx)),
            status: "Enter or Shift+Enter in a field; Tab or Shift+Tab to the other one".into(),
        }
    }

    /// Enter in a field: here it tells what the field holds; a find bar would find it.
    fn submit(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let fields = [("Find", &self.find), ("Replace", &self.replace)];
        let Some((name, field)) = fields.into_iter().find(|(_, field)| field.focus_handle(cx).is_focused(window))
        else {
            return;
        };
        let key = if backwards { "Shift+Enter" } else { "Enter" };
        self.status = format!("{key} in {name}: {:?}", field.read(cx).text(cx)).into();
        cx.notify();
    }
}

impl Render for Fields {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |label: &'static str, field: &Entity<EditorView>| {
            div().flex().items_center().gap_2().child(div().w(px(64.)).child(label)).child(
                div().flex_1().py(px(3.)).border_1().border_color(rgb(0x9a9a9a)).bg(rgb(0xffffff)).child(field.clone()),
            )
        };
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(|fields, _: &Submit, window, cx| fields.submit(false, window, cx)))
            .on_action(cx.listener(|fields, _: &SubmitBackwards, window, cx| fields.submit(true, window, cx)))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(rgb(0xececec))
            .text_color(rgb(0x1f1f1f))
            .text_sm()
            .child(row("Find", &self.find))
            .child(row("Replace", &self.replace))
            .child(self.status.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        migpad_editor::init(cx);
        cx.bind_keys([
            KeyBinding::new("enter", Submit, Some(CONTEXT)),
            KeyBinding::new("shift-enter", SubmitBackwards, Some(CONTEXT)),
            KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
            KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
        ]);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(420.), px(150.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("Fields".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let fields = cx.new(|cx| Fields::new(window, cx));
            window.focus(&fields.read(cx).find.focus_handle(cx), cx);
            fields
        })
        .expect("failed to open the window");
        cx.activate(true);
    });
}
