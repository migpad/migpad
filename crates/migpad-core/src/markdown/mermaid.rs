//! Mermaid diagrams as SVG for the Markdown preview, drawn by merman — Mermaid.js rewritten in
//! Rust — in its light theme or its dark one, graphs laid out by ELK, which keeps them tidier than
//! the dagre of Mermaid. The SVG is made for resvg, which GPUI draws SVG with: the HTML labels of
//! Mermaid, which resvg does not draw, become SVG text wrapped as a browser wraps them, and the SVG
//! reads no files.

use std::sync::LazyLock;
use std::time::Duration;

use merman::svg::SvgPipeline;
use merman::{Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};
use serde_json::json;

static LIGHT: LazyLock<Renderer> = LazyLock::new(|| renderer("default"));
static DARK: LazyLock<Renderer> = LazyLock::new(|| renderer("dark"));

/// The font of Mermaid, which merman measures text for. Its own name for it, "trebuchet ms", resvg
/// does not find: it takes names of fonts as they are written.
const FONT: &str = "\"Trebuchet MS\", Verdana, Arial, sans-serif";

fn renderer(theme: &str) -> Renderer {
    // The labels are HTML, as Mermaid has them: the pipeline of resvg makes them SVG text wrapped
    // as in a browser. Mermaid's own SVG text breaks words of a narrow label letter by letter.
    let config = json!({
        "theme": theme,
        "fontFamily": FONT,
        "themeVariables": { "fontFamily": FONT },
        "layout": "elk",
    });
    Renderer::new().with_engine(Engine::new().with_site_config(MermaidConfig::from_value(config)))
}

/// The longest a diagram is drawn: one that takes longer is told as too large.
const DEADLINE: Duration = Duration::from_secs(10);

/// A diagram as SVG, and its size in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Svg {
    pub text: String,
    pub width: f32,
    pub height: f32,
}

/// The SVG of the diagram `source`, in the dark theme or the light one; or what is wrong with the
/// source.
pub fn render(source: &str, dark: bool) -> Result<Svg, String> {
    let renderer = if dark { &*DARK } else { &*LIGHT };
    let svg = SvgRequest { pipeline: Some(SvgPipeline::resvg_safe()), ..SvgRequest::default() };
    let control = OperationControl::new().with_deadline(DEADLINE);
    match renderer.render(RenderRequest::svg(source, control, svg)) {
        Ok(RenderOutput::Svg(Some(svg))) => {
            let text = widen(svg.svg(), MARGIN);
            let (width, height) = size(&text).ok_or_else(|| "a diagram without a size".to_owned())?;
            Ok(Svg { text, width, height })
        }
        Ok(_) => Err("no Mermaid diagram".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Room around a diagram: labels of some diagrams reach past the box merman gives them, as those of
/// the relations of ER diagrams do, and text in a font other than that of Mermaid is wider.
const MARGIN: f64 = 16.0;

/// Where the view box of `svg` is in it, and its numbers: x, y, width and height.
fn view_box(svg: &str) -> Option<(std::ops::Range<usize>, [f64; 4])> {
    let tag = &svg[..svg.find('>')?];
    let start = tag.find(" viewBox=\"")? + " viewBox=\"".len();
    let len = tag[start..].find('"')?;
    let numbers: Vec<f64> = tag[start..start + len].split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let numbers: [f64; 4] = numbers.try_into().ok()?;
    Some((start..start + len, numbers))
}

/// The size of `svg`, as its view box tells.
fn size(svg: &str) -> Option<(f32, f32)> {
    let (_, [_, _, width, height]) = view_box(svg)?;
    (width > 0.0 && height > 0.0).then_some((width as f32, height as f32))
}

/// `svg` with `margin` more room on each side: its view box and its width grow.
fn widen(svg: &str, margin: f64) -> String {
    let Some((range, [x, y, width, height])) = view_box(svg) else { return svg.to_owned() };
    let (start, len) = (range.start, range.len());
    let Some(tag_end) = svg.find('>') else { return svg.to_owned() };
    let tag = &svg[..tag_end];
    let view_box = format!("{} {} {} {}", x - margin, y - margin, width + 2.0 * margin, height + 2.0 * margin);
    let mut tag = format!("{}{view_box}{}", &tag[..start], &tag[start + len..]);
    let max_width = format!("max-width:{width}px");
    if tag.contains(&max_width) {
        tag = tag.replace(&max_width, &format!("max-width:{}px", width + 2.0 * margin));
    }
    format!("{tag}{}", &svg[tag_end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagrams_get_room_around_them() {
        let svg = r#"<svg id="m" style="max-width:148.5px" viewBox="-8 -8 148.5 455"><g/></svg>"#;
        let wide = widen(svg, 16.0);
        assert_eq!(wide, r#"<svg id="m" style="max-width:180.5px" viewBox="-24 -24 180.5 487"><g/></svg>"#);
        assert_eq!(size(&wide), Some((180.5, 487.0)));
        // Without a view box, as it is.
        assert_eq!(widen("<svg><g/></svg>", 16.0), "<svg><g/></svg>");
    }

    #[test]
    fn diagrams_of_each_kind_are_drawn_with_their_labels() {
        let diagrams = [
            "flowchart LR\n  A[Начало] --> B{Файл открыт?}",
            "sequenceDiagram\n  Alice->>Боб: Привет",
            "classDiagram\n  class Tab[\"Вкладка\"] {\n    +bool закреплена\n  }",
            "stateDiagram-v2\n  [*] --> Просмотр",
            "erDiagram\n  WINDOW ||--o{ TAB : содержит",
            "gantt\n  dateFormat YYYY-MM-DD\n  section Вкладки\n  Закрепление :a1, 2026-10-14, 2d",
            "pie title Кодировки\n  \"UTF-8\" : 70\n  \"KOI8-R\" : 30",
            "journey\n  title Открытие\n  section Запуск\n    Открыть файл: 5: Пользователь",
            "gitGraph\n  commit\n  branch feat\n  commit",
            "mindmap\n  root((MigPad))\n    Вкладки\n    Кодировки",
            "timeline\n  title Выпуски\n  2026 : 0.1",
        ];
        for (i, source) in diagrams.iter().enumerate() {
            for dark in [false, true] {
                let svg = render(source, dark).unwrap_or_else(|error| panic!("diagram {i}: {error}"));
                assert!(svg.text.starts_with("<svg"), "diagram {i}");
                assert!(svg.width > 0.0 && svg.height > 0.0, "diagram {i}");
                // No label is HTML, which resvg does not draw.
                assert!(!svg.text.contains("<foreignObject"), "diagram {i}");
            }
        }
        let mindmap = render(diagrams[9], false).unwrap().text;
        assert!(mindmap.contains("Вкладки"));
        assert!(mindmap.contains("MigPad"));
    }

    #[test]
    fn what_is_wrong_is_told() {
        let error = render("flowchart LR\n  A[Начало --> B", false).unwrap_err();
        assert!(error.contains(']'), "{error}");
        assert!(render("not a diagram", false).is_err());
        assert!(render("", false).is_err());
    }
}
