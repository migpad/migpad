//! The element that paints a document: prepaint lays out the visible lines, paint draws them.

use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, ScrollWheelEvent, Style, TextAlign, Window, fill, relative, rgb,
};

use crate::view::{EditorView, Layout, colors};

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
        style.size.height = relative(1.).into();
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
        self.view.update(cx, |view, cx| view.layout(bounds, window, cx))
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
        window.paint_quad(fill(layout.bounds, rgb(colors::BACKGROUND)));
        window.paint_quad(fill(layout.gutter, rgb(colors::GUTTER)));
        window.with_content_mask(Some(ContentMask { bounds: layout.text_area }), |window| {
            for line in &layout.lines {
                let _ = line.shaped.paint(line.origin, layout.line_height, TextAlign::Left, None, window, cx);
            }
        });
        for (number, origin) in &layout.numbers {
            let _ = number.paint(*origin, layout.line_height, TextAlign::Left, None, window, cx);
        }
        window.paint_quad(fill(layout.track, rgb(colors::TRACK)));
        if let Some(thumb) = layout.thumb {
            window.paint_quad(fill(thumb, rgb(colors::THUMB)));
        }

        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && bounds.contains(&event.position) {
                view.update(cx, |view, cx| view.scroll(event, cx));
            }
        });
    }
}
