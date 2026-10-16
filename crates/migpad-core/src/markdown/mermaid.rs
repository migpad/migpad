//! Mermaid diagrams as SVG for the Markdown preview, drawn by merman — Mermaid.js rewritten in
//! Rust — in its light theme or its dark one. The SVG is made for resvg, which GPUI draws SVG
//! with: labels are SVG text, not the HTML Mermaid puts in `foreignObject`, which resvg does not
//! draw, and the SVG reads no files.

use std::sync::LazyLock;

use merman::svg::SvgPipeline;
use merman::{Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};
use serde_json::json;

static LIGHT: LazyLock<Renderer> = LazyLock::new(|| renderer("default"));
static DARK: LazyLock<Renderer> = LazyLock::new(|| renderer("dark"));

/// The font of Mermaid, which merman measures text for. Its own name for it, "trebuchet ms", resvg
/// does not find: it takes names of fonts as they are written.
const FONT: &str = "\"Trebuchet MS\", Verdana, Arial, sans-serif";

fn renderer(theme: &str) -> Renderer {
    // SVG text where Mermaid can lay it out so; the labels it lays out only as HTML — those of
    // mindmaps — the pipeline of resvg makes SVG text.
    let config = json!({
        "theme": theme,
        "fontFamily": FONT,
        "themeVariables": { "fontFamily": FONT },
        "htmlLabels": false,
        "flowchart": { "htmlLabels": false },
    });
    Renderer::new().with_engine(Engine::new().with_site_config(MermaidConfig::from_value(config)))
}

/// The SVG of the diagram `source`, in the dark theme or the light one; or what is wrong with the
/// source.
pub fn render(source: &str, dark: bool) -> Result<String, String> {
    let renderer = if dark { &*DARK } else { &*LIGHT };
    let svg = SvgRequest { pipeline: Some(SvgPipeline::resvg_safe()), ..SvgRequest::default() };
    match renderer.render(RenderRequest::svg(source, OperationControl::new(), svg)) {
        Ok(RenderOutput::Svg(Some(svg))) => Ok(widen(svg.svg(), MARGIN)),
        Ok(_) => Err("no Mermaid diagram".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Room around a diagram: labels of some diagrams reach past the box merman gives them, as those of
/// the relations of ER diagrams do, and text in a font other than that of Mermaid is wider.
const MARGIN: f64 = 16.0;

/// `svg` with `margin` more room on each side: its view box and its width grow.
fn widen(svg: &str, margin: f64) -> String {
    let Some(tag_end) = svg.find('>') else { return svg.to_owned() };
    let tag = &svg[..tag_end];
    let Some(start) = tag.find(" viewBox=\"").map(|at| at + " viewBox=\"".len()) else { return svg.to_owned() };
    let Some(len) = tag[start..].find('"') else { return svg.to_owned() };
    let numbers: Vec<f64> = tag[start..start + len].split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let [x, y, width, height] = numbers[..] else { return svg.to_owned() };
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
        assert_eq!(widen(svg, 16.0), r#"<svg id="m" style="max-width:180.5px" viewBox="-24 -24 180.5 487"><g/></svg>"#);
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
                assert!(svg.starts_with("<svg"), "diagram {i}");
                // No label is HTML, which resvg does not draw.
                assert!(!svg.contains("<foreignObject"), "diagram {i}");
            }
        }
        let mindmap = render(diagrams[9], false).unwrap();
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
