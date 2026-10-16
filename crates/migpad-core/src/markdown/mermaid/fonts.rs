//! The width of text as GPUI draws it in SVG, for merman to lay diagrams out by. merman itself
//! guesses a width by the kind of each character — a letter not Latin as wide as a capital, a
//! circled digit too — and a label wider than the guess outgrew its background. Here the fonts are
//! those of the system, found as usvg finds them — the first of the families there is, and for a
//! character the font lacks a font that has it, as GPUI chooses — and the text is shaped by
//! rustybuzz, which resvg shapes it with.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};
use regex::Regex;
use rustybuzz::{Face, UnicodeBuffer, ttf_parser};

/// The fonts of the system, found once: it takes a while.
static FONTS: LazyLock<Database> = LazyLock::new(|| {
    let mut fonts = Database::new();
    fonts.load_system_fonts();
    fonts
});

/// The fonts GPUI takes for emoji before any other.
#[cfg(target_os = "macos")]
const EMOJI: &[&str] = &["Apple Color Emoji", ".AppleColorEmojiUI"];
#[cfg(target_os = "windows")]
const EMOJI: &[&str] = &["Segoe UI Emoji", "Segoe UI Symbol"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const EMOJI: &[&str] = &["Noto Color Emoji", "Emoji One", "Twitter Color Emoji", "JoyPixels"];

static EMOJI_PRESENTATION: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\p{Emoji_Presentation}").ok());

/// The fonts found for characters their fonts lack, by the character and the fonts tried before:
/// the search may go through every font of the system.
type Fallbacks = HashMap<(char, Vec<ID>), Option<ID>>;
static FALLBACKS: LazyLock<Mutex<Fallbacks>> = LazyLock::new(Mutex::default);

/// The widths measured, by the text and its font: merman measures a label again for each place it
/// could wrap at.
type Widths = HashMap<(String, String, u64, u16, bool), f64>;
static WIDTHS: LazyLock<Mutex<Widths>> = LazyLock::new(Mutex::default);
const MOST_WIDTHS: usize = 20_000;

/// The width of `text` in pixels, in the first of `families` — a list as CSS writes it — the system
/// has, at `size` pixels, of `weight`, italic or not; none if the system has no fonts.
pub(super) fn width(text: &str, families: &str, size: f64, weight: u16, italic: bool) -> Option<f64> {
    let key = (text.to_owned(), families.to_owned(), size.to_bits(), weight, italic);
    if let Some(width) = WIDTHS.lock().ok().and_then(|widths| widths.get(&key).copied()) {
        return Some(width);
    }
    let fonts = &*FONTS;
    let style = if italic { Style::Italic } else { Style::Normal };
    let font = font(fonts, families, weight, style)?;
    let width = shape_with_fallbacks(fonts, text, font)
        .iter()
        .map(|glyph| f64::from(glyph.advance) * size / f64::from(glyph.units_per_em.max(1)))
        .sum();
    if let Ok(mut widths) = WIDTHS.lock() {
        if widths.len() >= MOST_WIDTHS {
            widths.clear();
        }
        widths.insert(key, width);
    }
    Some(width)
}

/// The font usvg takes for `families`: the first of them there is, else a serif one; and if none,
/// as GPUI does, a sans-serif one, else any.
fn font(fonts: &Database, families: &str, weight: u16, style: Style) -> Option<ID> {
    let mut list: Vec<Family> = css_families(families).collect();
    list.push(Family::Serif);
    let query = Query { families: &list, weight: Weight(weight), stretch: Stretch::Normal, style };
    fonts
        .query(&query)
        .or_else(|| fonts.query(&Query { families: &[Family::SansSerif], ..Query::default() }))
        .or_else(|| fonts.faces().next().map(|face| face.id))
}

/// The families of a list as CSS writes them: names, quoted or not, and generic families.
fn css_families(list: &str) -> impl Iterator<Item = Family<'_>> {
    list.split(',').map(|name| name.trim().trim_matches(['"', '\'']).trim()).filter(|name| !name.is_empty()).map(
        |name| match name {
            "serif" => Family::Serif,
            "sans-serif" => Family::SansSerif,
            "monospace" => Family::Monospace,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            name => Family::Name(name),
        },
    )
}

/// A glyph shaped: missing from its font if its number is 0.
#[derive(Clone, Copy)]
struct Glyph {
    id: u32,
    /// The byte of the text it starts at.
    cluster: u32,
    advance: i32,
    units_per_em: i32,
}

/// `text` shaped in `font`, and the glyphs it lacks in fonts that have them, as usvg does: the text
/// is shaped again in the font found for the first character missing, which takes the place of
/// all the glyphs if it has them all, else of those missing.
fn shape_with_fallbacks(fonts: &Database, text: &str, font: ID) -> Vec<Glyph> {
    let mut glyphs = shape(fonts, text, font);
    let mut used = vec![font];
    loop {
        let missing = glyphs.iter().find(|glyph| glyph.id == 0);
        let Some(missing) = missing.and_then(|glyph| text.get(glyph.cluster as usize..)?.chars().next()) else {
            break;
        };
        let Some(fallback) = fallback(fonts, missing, &used) else { break };
        let other = shape(fonts, text, fallback);
        if !other.is_empty() && other.iter().all(|glyph| glyph.id != 0) {
            return other;
        }
        if other.len() != glyphs.len() {
            break;
        }
        for (glyph, other) in glyphs.iter_mut().zip(other) {
            if glyph.id == 0 && other.id != 0 {
                *glyph = other;
            }
        }
        used.push(fallback);
    }
    glyphs
}

fn shape(fonts: &Database, text: &str, font: ID) -> Vec<Glyph> {
    fonts
        .with_face_data(font, |data, index| {
            let face = Face::from_slice(data, index)?;
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(text);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            let units_per_em = face.units_per_em();
            let glyphs = shaped.glyph_infos().iter().zip(shaped.glyph_positions());
            Some(
                glyphs
                    .map(|(info, position)| Glyph {
                        id: info.glyph_id,
                        cluster: info.cluster,
                        advance: position.x_advance,
                        units_per_em,
                    })
                    .collect(),
            )
        })
        .flatten()
        .unwrap_or_default()
}

/// The font to draw `c` in, not one of `used`: for an emoji, one of emoji; else, as usvg finds it,
/// the first font of the system that has it and is of the style of the first of `used`, its weight
/// or its width.
fn fallback(fonts: &Database, c: char, used: &[ID]) -> Option<ID> {
    let key = (c, used.to_vec());
    if let Some(found) = FALLBACKS.lock().ok().and_then(|fallbacks| fallbacks.get(&key).copied()) {
        return found;
    }
    let found = find_fallback(fonts, c, used);
    if let Ok(mut fallbacks) = FALLBACKS.lock() {
        fallbacks.insert(key, found);
    }
    found
}

fn find_fallback(fonts: &Database, c: char, used: &[ID]) -> Option<ID> {
    let emoji = EMOJI_PRESENTATION.as_ref().is_some_and(|emoji| emoji.is_match(c.encode_utf8(&mut [0; 4])));
    if emoji {
        for family in EMOJI {
            let families = [Family::Name(family)];
            let query =
                Query { families: &families, weight: Weight(400), stretch: Stretch::Normal, style: Style::Normal };
            if let Some(id) = fonts.query(&query)
                && !used.contains(&id)
                && has_char(fonts, id, c)
            {
                return Some(id);
            }
        }
    }
    let base = fonts.face(*used.first()?)?;
    let found = fonts.faces().find(|face| {
        let other_style = base.style != face.style && base.weight != face.weight && base.stretch != face.stretch;
        !used.contains(&face.id) && !other_style && has_char(fonts, face.id, c)
    });
    found.map(|face| face.id)
}

fn has_char(fonts: &Database, id: ID, c: char) -> bool {
    fonts
        .with_face_data(id, |data, index| ttf_parser::Face::parse(data, index).ok()?.glyph_index(c))
        .flatten()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_as_css_writes_them() {
        let families: Vec<Family> = css_families("\"Trebuchet MS\", Verdana ,'Arial', sans-serif,").collect();
        assert_eq!(
            families,
            [Family::Name("Trebuchet MS"), Family::Name("Verdana"), Family::Name("Arial"), Family::SansSerif]
        );
    }

    #[test]
    fn text_is_as_wide_as_its_font_draws_it() {
        // Systems without fonts, as some of CI, have nothing to measure.
        let Some(narrow) = width("iii", "sans-serif", 16.0, 400, false) else { return };
        let wide = width("WWW", "sans-serif", 16.0, 400, false).unwrap();
        assert!(narrow > 0.0 && wide > narrow * 2.0, "{narrow} {wide}");
        // Twice the size, twice the width.
        let double = width("WWW", "sans-serif", 32.0, 400, false).unwrap();
        assert!((double - wide * 2.0).abs() < 0.01, "{wide} {double}");
        assert_eq!(width("", "sans-serif", 16.0, 400, false), Some(0.0));
    }
}
