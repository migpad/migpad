//! The element that paints a document: prepaint lays out the visible lines, paint draws them and
//! the selection and the caret, and listens to the mouse.

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Entity, Focusable,
    GlobalElementId, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, Style, TextAlign, Window, fill, px, relative, rgb,
};

use crate::layout::Layout;
use crate::view::EditorView;

pub(crate) struct EditorElement {
    view: Entity<EditorView>,
}

impl EditorElement {
    pub(crate) fn new(view: Entity<EditorView>) -> Self {
        EditorElement { view }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Layout;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = match self.view.read(cx).fixed_height() {
            Some(height) => height.into(),
            None => relative(1.).into(),
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Layout {
        let mut layout = self.view.update(cx, |view, cx| view.layout(bounds, window, cx));
        layout.view_hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
        layout.hitbox = Some(window.insert_hitbox(layout.geometry.text_area, HitboxBehavior::Normal));
        layout
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        layout: &mut Layout,
        window: &mut Window,
        cx: &mut App,
    ) {
        let geometry = layout.geometry;
        let colors = layout.colors;
        window.paint_quad(fill(geometry.bounds, rgb(colors.background)));
        window.paint_quad(fill(geometry.gutter, rgb(colors.gutter)));
        window.with_content_mask(Some(ContentMask { bounds: layout.clip }), |window| {
            for &rect in &layout.selection {
                window.paint_quad(fill(rect, rgb(layout.selection_color)));
            }
            for &guide in &layout.guides {
                window.paint_quad(fill(guide, rgb(colors.guide)));
            }
            if let Some((placeholder, origin)) = &layout.placeholder {
                let _ = placeholder.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
            for (line, origin) in &layout.lines {
                let _ = line.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
            for (label, origin, background) in &layout.labels {
                window.paint_quad(fill(*background, rgb(colors.label_background)).corner_radii(px(3.)));
                let _ = label.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
            if let Some(caret) = layout.caret {
                window.paint_quad(fill(caret, rgb(colors.caret)));
            }
        });
        // The number of a row scrolled half out of view stays in the gutter, off the bars around.
        window.with_content_mask(Some(ContentMask { bounds: geometry.gutter }), |window| {
            for (number, origin) in &layout.numbers {
                let _ = number.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
        });
        window.paint_quad(fill(geometry.track, rgb(colors.track)));
        if let Some(thumb) = geometry.thumb {
            window.paint_quad(fill(thumb, rgb(colors.thumb)));
        }
        if let Some(hitbox) = &layout.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }
        let focus = self.view.read(cx).focus_handle(cx);
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);

        // Presses and the wheel only where nothing covers the view, such as an open menu; moves and
        // releases are heard outside the element too: a drag goes on past its edges.
        let Some(hitbox) = layout.view_hitbox.clone() else { return };
        let view = self.view.clone();
        let pressed = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && pressed.is_hovered(window) {
                view.update(cx, |view, cx| view.mouse_down(event, window, cx));
                cx.stop_propagation();
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |view, cx| view.mouse_move(event, window, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                view.update(cx, |view, _| view.mouse_up());
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                view.update(cx, |view, cx| view.scroll(event, window, cx));
            }
        });
    }
}
