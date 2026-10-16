//! The width of text as GPUI draws it in SVG, for merman to lay diagrams out by. merman itself
//! guesses a width by the kind of each character — a letter not Latin as wide as a capital, a
//! circled digit too — and a label wider than the guess outgrew its background. Here the fonts are
//! those of the system, found as usvg finds them — the first of the families there is, and for a
//! character the font lacks a font that has it, as GPUI chooses — and a text is as wide as the
//! advances of its glyphs. resvg shapes text too, with kerning, which makes it a little narrower
//! than measured: a label has room on its background.
//!
//! What is found is kept: the font of each list of families, the glyphs of each font, the fonts
//! found for characters — merman measures a label again for each place it could wrap at, and
//! reading a font means mapping its file and parsing it.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};
use regex::Regex;
use ttf_parser::{Face, GlyphId};

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

static FOUND: LazyLock<Mutex<Found>> = LazyLock::new(Mutex::default);

/// What was found in the fonts of the system.
#[derive(Default)]
struct Found {
    /// The font of each list of families, weight and style.
    fonts: HashMap<(String, u16, bool), Option<ID>>,
    /// The glyphs of the characters looked up in each font.
    glyphs: HashMap<ID, Glyphs>,
    /// The fonts found for characters their fonts lack, by the character and the fonts tried
    /// before; and those fonts in the order they were found.
    fallbacks: HashMap<(char, Vec<ID>), Option<ID>>,
    found: Vec<ID>,
}

/// The glyphs of a font: of each character, its number — 0 if the font lacks it — and its advance.
#[derive(Default)]
struct Glyphs {
    units_per_em: u16,
    of: HashMap<char, (u16, u16)>,
}

/// The width of `text` in pixels, in the first of `families` — a list as CSS writes it — the system
/// has, at `size` pixels, of `weight`, italic or not; none if the system has no fonts.
pub(super) fn width(text: &str, families: &str, size: f64, weight: u16, italic: bool) -> Option<f64> {
    let fonts = &*FONTS;
    let mut found = FOUND.lock().unwrap_or_else(PoisonError::into_inner);
    let font = found.font(fonts, families, weight, italic)?;
    let glyphs = found.glyphs_with_fallbacks(fonts, text, font);
    Some(glyphs.iter().map(|glyph| f64::from(glyph.advance) * size / f64::from(glyph.units_per_em.max(1))).sum())
}

/// The glyph of a character in a font: missing from it if its number is 0.
#[derive(Clone, Copy)]
struct Glyph {
    id: u16,
    c: char,
    advance: u16,
    units_per_em: u16,
}

impl Found {
    /// The font usvg takes for `families`: the first of them there is, else a serif one; and if
    /// none, as GPUI does, a sans-serif one, else any.
    fn font(&mut self, fonts: &Database, families: &str, weight: u16, italic: bool) -> Option<ID> {
        let key = (families.to_owned(), weight, italic);
        if let Some(font) = self.fonts.get(&key) {
            return *font;
        }
        let mut list: Vec<Family> = css_families(families).collect();
        list.push(Family::Serif);
        let style = if italic { Style::Italic } else { Style::Normal };
        let query = Query { families: &list, weight: Weight(weight), stretch: Stretch::Normal, style };
        let font = fonts
            .query(&query)
            .or_else(|| fonts.query(&Query { families: &[Family::SansSerif], ..Query::default() }))
            .or_else(|| fonts.faces().next().map(|face| face.id));
        self.fonts.insert(key, font);
        font
    }

    /// The glyphs of `text` in `font`, and those it lacks in fonts that have them, as usvg finds
    /// them: the font found for the first character missing takes the place of all the glyphs if
    /// it has them all, else of those missing.
    fn glyphs_with_fallbacks(&mut self, fonts: &Database, text: &str, font: ID) -> Vec<Glyph> {
        let mut glyphs = self.glyphs_in(fonts, text, font);
        let mut used = vec![font];
        while let Some(missing) = glyphs.iter().find(|glyph| glyph.id == 0).map(|glyph| glyph.c) {
            let Some(fallback) = self.fallback(fonts, missing, &used) else { break };
            let other = self.glyphs_in(fonts, text, fallback);
            if !other.is_empty() && other.iter().all(|glyph| glyph.id != 0) {
                return other;
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

    /// The glyphs of the characters of `text` in `font`, but those no font draws — joiners,
    /// selectors of variants, soft hyphens — which take no room. The font is read once for those
    /// not looked up in it before.
    fn glyphs_in(&mut self, fonts: &Database, text: &str, font: ID) -> Vec<Glyph> {
        let glyphs = self.glyphs.entry(font).or_default();
        let new: Vec<char> = text.chars().filter(|&c| !invisible(c) && !glyphs.of.contains_key(&c)).collect();
        if !new.is_empty() {
            fonts.with_face_data(font, |data, index| {
                let Ok(face) = Face::parse(data, index) else { return };
                glyphs.units_per_em = face.units_per_em();
                for c in new {
                    let id = face.glyph_index(c).unwrap_or(GlyphId(0));
                    glyphs.of.insert(c, (id.0, face.glyph_hor_advance(id).unwrap_or(0)));
                }
            });
        }
        let units_per_em = glyphs.units_per_em;
        text.chars()
            .filter(|&c| !invisible(c))
            .map(|c| {
                let (id, advance) = glyphs.of.get(&c).copied().unwrap_or_default();
                Glyph { id, c, advance, units_per_em }
            })
            .collect()
    }

    /// The font to draw `c` in, not one of `used`: for an emoji, one of emoji; else a font found
    /// before for another character, if it has `c`; else, as usvg finds it, the first font of the
    /// system that has it and is of the style of the first of `used`, its weight or its width. The
    /// fonts of a script come one after another: without the fonts found before, each character of
    /// Chinese a label starts with would go through every font of the system.
    fn fallback(&mut self, fonts: &Database, c: char, used: &[ID]) -> Option<ID> {
        let key = (c, used.to_vec());
        if let Some(found) = self.fallbacks.get(&key) {
            return *found;
        }
        let base = fonts.face(*used.first()?)?;
        let fits = |face: &fontdb::FaceInfo| {
            let other_style = base.style != face.style && base.weight != face.weight && base.stretch != face.stretch;
            !used.contains(&face.id) && !other_style
        };
        let emoji = EMOJI_PRESENTATION.as_ref().is_some_and(|emoji| emoji.is_match(c.encode_utf8(&mut [0; 4])));
        let emoji_font = || {
            EMOJI.iter().find_map(|family| {
                let families = [Family::Name(family)];
                let query =
                    Query { families: &families, weight: Weight(400), stretch: Stretch::Normal, style: Style::Normal };
                fonts.query(&query).filter(|id| !used.contains(id) && self.has(fonts, *id, c))
            })
        };
        let found = emoji
            .then(emoji_font)
            .flatten()
            .or_else(|| {
                let before = self.found.iter().rev().copied();
                before.filter(|id| fonts.face(*id).is_some_and(fits)).find(|id| self.has(fonts, *id, c))
            })
            .or_else(|| fonts.faces().find(|face| fits(face) && self.has(fonts, face.id, c)).map(|face| face.id));
        if let Some(found) = found
            && !self.found.contains(&found)
        {
            self.found.push(found);
        }
        self.fallbacks.insert(key, found);
        found
    }

    /// Whether `font` has a glyph for `c`: as looked up before, or in the font.
    fn has(&self, fonts: &Database, font: ID, c: char) -> bool {
        if let Some(&(id, _)) = self.glyphs.get(&font).and_then(|glyphs| glyphs.of.get(&c)) {
            return id != 0;
        }
        fonts.with_face_data(font, |data, index| Face::parse(data, index).ok()?.glyph_index(c)).flatten().is_some()
    }
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

/// Whether `c` takes no room in text and is in no font: a control, a joiner, a selector of a
/// variant, a soft hyphen, a tag.
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}')
        || matches!(c, '\u{E0000}'..='\u{E0FFF}')
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
        // Systems without fonts have nothing to measure; those of some of CI have one font, which
        // may have letters of one width.
        let Some(narrow) = width("iii", "sans-serif", 16.0, 400, false) else { return };
        let wide = width("WWW", "sans-serif", 16.0, 400, false).unwrap();
        assert!(narrow > 0.0 && wide >= narrow, "{narrow} {wide}");
        // Twice the letters, or twice the size, twice the width.
        let longer = width("WWWWWW", "sans-serif", 16.0, 400, false).unwrap();
        let larger = width("WWW", "sans-serif", 32.0, 400, false).unwrap();
        assert!((longer - wide * 2.0).abs() < 0.01 && (larger - wide * 2.0).abs() < 0.01, "{wide} {longer} {larger}");
        assert_eq!(width("", "sans-serif", 16.0, 400, false), Some(0.0));
        // A joiner takes no room.
        assert_eq!(width("W\u{200D}W", "sans-serif", 16.0, 400, false), width("WW", "sans-serif", 16.0, 400, false));
    }
}
