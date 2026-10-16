//! Mermaid diagrams as SVG for the Markdown preview, drawn by merman — Mermaid.js rewritten in
//! Rust — in its light theme or its dark one. Labels are SVG text: the SVG of GPUI (resvg) does
//! not draw HTML, which Mermaid puts in `foreignObject`. Mindmaps have no labels of another kind:
//! theirs become SVG text here.

use std::sync::LazyLock;

use merman::{Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};
use serde_json::json;

static LIGHT: LazyLock<Renderer> = LazyLock::new(|| renderer("default"));
static DARK: LazyLock<Renderer> = LazyLock::new(|| renderer("dark"));

fn renderer(theme: &str) -> Renderer {
    let config = json!({ "theme": theme, "htmlLabels": false, "flowchart": { "htmlLabels": false } });
    Renderer::new().with_engine(Engine::new().with_site_config(MermaidConfig::from_value(config)))
}

/// The SVG of the diagram `source`, in the dark theme or the light one; or what is wrong with the
/// source.
pub fn render(source: &str, dark: bool) -> Result<String, String> {
    let renderer = if dark { &*DARK } else { &*LIGHT };
    let request = RenderRequest::svg(source, OperationControl::new(), SvgRequest::default());
    match renderer.render(request) {
        Ok(RenderOutput::Svg(Some(svg))) => Ok(html_labels_to_text(svg.svg())),
        Ok(_) => Err("no Mermaid diagram".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// `svg` with its HTML labels — `foreignObject` — made SVG text in the middle of where they were;
/// empty ones go. Those in a `switch` stay: SVG text is beside them, and it is drawn instead.
fn html_labels_to_text(svg: &str) -> String {
    const OPEN: &str = "<foreignObject";
    const CLOSE: &str = "</foreignObject>";
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    let mut done = 0;
    while let Some(at) = rest.find(OPEN) {
        let start = done + at;
        let Some(end) = svg[start..].find(CLOSE).map(|end| start + end + CLOSE.len()) else { break };
        out.push_str(&svg[done..start]);
        let label = &svg[start..end];
        if in_switch(&svg[..start]) {
            out.push_str(label);
        } else {
            out.push_str(&label_as_text(label));
        }
        done = end;
        rest = &svg[done..];
    }
    out.push_str(&svg[done..]);
    out
}

/// Whether a `switch` is open at the end of `before`.
fn in_switch(before: &str) -> bool {
    match (before.rfind("<switch"), before.rfind("</switch>")) {
        (Some(open), Some(close)) => open > close,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// An HTML label as SVG text: its lines, centred in the box of the label.
fn label_as_text(label: &str) -> String {
    let tag_end = label.find('>').unwrap_or(label.len());
    let tag = &label[..tag_end];
    let number = |name: &str| attribute(tag, name).and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.0);
    let lines = label_lines(&label[(tag_end + 1).min(label.len())..]);
    if lines.is_empty() {
        return String::new();
    }
    let (x, y) = (number("x") + number("width") / 2.0, number("y") + number("height") / 2.0);
    // The middle of the lines in the middle of the box: 0.35em lowers a line to its middle; in
    // hundredths, which print as they are.
    let first = (35 - 60 * (lines.len() as i64 - 1)) as f64 / 100.0;
    let mut text = format!(r#"<text x="{x}" y="{y}" text-anchor="middle">"#);
    for (i, line) in lines.iter().enumerate() {
        let dy = if i == 0 { first } else { 1.2 };
        text.push_str(&format!(r#"<tspan x="{x}" dy="{dy}em">{line}</tspan>"#));
    }
    text.push_str("</text>");
    text
}

/// The value of the attribute `name` of the tag `tag`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[at..].find('"')?;
    Some(&tag[at..at + len])
}

/// The lines of text of an HTML label, without its tags: line breaks and paragraphs end lines.
fn label_lines(html: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        line.push_str(&rest[..at]);
        let end = rest[at..].find('>').map_or(rest.len(), |end| at + end + 1);
        let tag = rest[at..end].to_ascii_lowercase();
        if tag.starts_with("<br") || tag.starts_with("</p") || tag.starts_with("</div") {
            lines.push(std::mem::take(&mut line));
        }
        rest = &rest[end..];
    }
    line.push_str(rest);
    lines.push(line);
    lines
        .into_iter()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ").replace("&nbsp;", "&#160;"))
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
                // Every label the SVG of GPUI cannot draw is beside SVG text, in a switch.
                assert_eq!(svg.matches("<foreignObject").count(), svg.matches("<switch").count(), "diagram {i}");
            }
        }
        let mindmap = render(diagrams[9], false).unwrap();
        assert!(mindmap.contains(">Вкладки</tspan>"));
        assert!(mindmap.contains(">MigPad</tspan>"));
    }

    #[test]
    fn what_is_wrong_is_told() {
        let error = render("flowchart LR\n  A[Начало --> B", false).unwrap_err();
        assert!(error.contains(']'), "{error}");
        assert!(render("not a diagram", false).is_err());
        assert!(render("", false).is_err());
    }

    #[test]
    fn labels_become_text_in_the_middle_of_their_box() {
        let svg = r#"<g><foreignObject x="10" width="100" height="24"><div><span><p>Один &amp; два</p></span></div></foreignObject><foreignObject width="0" height="0"><div><span></span></div></foreignObject></g>"#;
        assert_eq!(
            html_labels_to_text(svg),
            r#"<g><text x="60" y="12" text-anchor="middle"><tspan x="60" dy="0.35em">Один &amp; два</tspan></text></g>"#
        );
        let lines = r#"<foreignObject width="40" height="40"><div>first<br/>second</div></foreignObject>"#;
        assert_eq!(
            html_labels_to_text(lines),
            r#"<text x="20" y="20" text-anchor="middle"><tspan x="20" dy="-0.25em">first</tspan><tspan x="20" dy="1.2em">second</tspan></text>"#
        );
        let switch =
            r#"<switch><foreignObject width="9" height="9"><div>x</div></foreignObject><text>x</text></switch>"#;
        assert_eq!(html_labels_to_text(switch), switch);
    }
}
