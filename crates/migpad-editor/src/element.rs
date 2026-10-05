//! The element that paints a document: prepaint lays out the visible lines, paint draws them and
//! the selection and the caret, and listens to the mouse.

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Entity, Focusable,
    GlobalElementId, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, Style, TextAlign, Window, fill, px, relative, rgb,
};

use crate::layout::Layout;
use crate::view::{EditorView, colors};

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
        window.paint_quad(fill(geometry.bounds, rgb(colors::BACKGROUND)));
        window.paint_quad(fill(geometry.gutter, rgb(colors::GUTTER)));
        window.with_content_mask(Some(ContentMask { bounds: layout.clip }), |window| {
            for &rect in &layout.selection {
                window.paint_quad(fill(rect, rgb(layout.selection_color)));
            }
            for &guide in &layout.guides {
                window.paint_quad(fill(guide, rgb(colors::GUIDE)));
            }
            for (line, origin) in &layout.lines {
                let _ = line.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
            for (label, origin, background) in &layout.labels {
                window.paint_quad(fill(*background, rgb(colors::LABEL_BACKGROUND)).corner_radii(px(3.)));
                let _ = label.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
            if let Some(caret) = layout.caret {
                window.paint_quad(fill(caret, rgb(colors::CARET)));
            }
        });
        for (number, origin) in &layout.numbers {
            let _ = number.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
        }
        window.paint_quad(fill(geometry.track, rgb(colors::TRACK)));
        if let Some(thumb) = geometry.thumb {
            window.paint_quad(fill(thumb, rgb(colors::THUMB)));
        }
        if let Some(hitbox) = &layout.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }
        let focus = self.view.read(cx).focus_handle(cx);
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);

        // Moves and releases are heard outside the element too: a drag goes on past its edges.
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && bounds.contains(&event.position) {
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
            if phase == DispatchPhase::Bubble && bounds.contains(&event.position) {
                view.update(cx, |view, cx| view.scroll(event, window, cx));
            }
        });
    }
}
