//! Changing the case of text: upper and lower case, Title Case, Sentence case and inverted case.
//!
//! The changes may change the length of the text: `ß` in upper case is `SS`. The standard library
//! has no title case mapping, so the first letter of a word takes its upper case, with the rest
//! of it in lower case (`ß` → `Ss`); the digraphs `ǆ`, `ǉ`, `ǌ`, `ǳ` take their title case forms.

/// How [`change_case`] changes the case of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// UPPER CASE.
    Upper,
    /// lower case.
    Lower,
    /// Every Word With A Capital: the first letter of a word in upper case, the rest in lower.
    Title,
    /// Sentence case: the first letter of a sentence in upper case, the rest in lower.
    Sentence,
    /// iNVERTED cASE: every letter in the other case.
    Invert,
}

/// `text` with its case changed as `case` tells.
pub fn change_case(text: &str, case: Case) -> String {
    match case {
        Case::Upper => text.to_uppercase(),
        Case::Lower => text.to_lowercase(),
        Case::Title => title_case(text),
        Case::Sentence => sentence_case(text),
        Case::Invert => invert_case(text),
    }
}

/// Words are letters, digits and the combining marks after them; an apostrophe between letters
/// is a part of a word: "don't" is one word.
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !is_word_char(c) {
            out.push(c);
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(at, next)) = chars.peek() {
            let joins =
                is_apostrophe(next) && text[at + next.len_utf8()..].chars().next().is_some_and(char::is_alphanumeric);
            if !is_word_char(next) && !joins {
                break;
            }
            end = at + next.len_utf8();
            chars.next();
        }
        push_title(&mut out, c);
        // The rest at once: lower case of a Greek sigma depends on the letters around it.
        out.push_str(&text[start + c.len_utf8()..end].to_lowercase());
    }
    out
}

/// A sentence starts at the start of the text, after `.`, `!`, `?` or `…` and a space or a line
/// break, and after an empty line. Closing quotes and brackets may stand between the end of a
/// sentence and the space: `"Yes." He…`.
fn sentence_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // The start of the text not yet written to `out`: it goes in lower case.
    let mut pending = 0;
    let mut start = true;
    let mut after_end = false;
    let mut line_empty = true;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c.is_alphanumeric() {
            if start {
                out.push_str(&text[pending..at].to_lowercase());
                push_title(&mut out, c);
                pending = at + c.len_utf8();
                start = false;
            }
            after_end = false;
            line_empty = false;
        } else if matches!(c, '.' | '!' | '?' | '…') {
            after_end = true;
            line_empty = false;
        } else if c.is_whitespace() {
            if after_end {
                start = true;
                after_end = false;
            }
            // CR LF is one line break: the LF counts it.
            let crlf = c == '\r' && chars.peek().is_some_and(|&(_, next)| next == '\n');
            if is_line_break(c) && !crlf {
                start |= line_empty;
                line_empty = true;
            }
        } else if is_closing(c) && after_end {
            line_empty = false;
        } else {
            after_end = false;
            line_empty = false;
        }
    }
    out.push_str(&text[pending..].to_lowercase());
    out
}

fn invert_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // Run by run, as lower case of a Greek sigma depends on the letters around it. Lower case of a
    // title case letter such as `ǅ`, of an upper case one and of others alike: these have no lower
    // case but themselves.
    while let Some(first) = rest.chars().next() {
        let lower = first.is_lowercase();
        let (run, after) = rest.split_at(rest.find(|c: char| c.is_lowercase() != lower).unwrap_or(rest.len()));
        out.push_str(&if lower { run.to_uppercase() } else { run.to_lowercase() });
        rest = after;
    }
    out
}

/// Writes `c`, the first letter of a word or a sentence, in title case.
fn push_title(out: &mut String, c: char) {
    let title = match c {
        'Ǆ' | 'ǅ' | 'ǆ' => 'ǅ',
        'Ǉ' | 'ǈ' | 'ǉ' => 'ǈ',
        'Ǌ' | 'ǋ' | 'ǌ' => 'ǋ',
        'Ǳ' | 'ǲ' | 'ǳ' => 'ǲ',
        _ => {
            // Of several letters, as with `ß` → `SS`, the first in upper case: `Ss`.
            let mut upper = c.to_uppercase();
            out.extend(upper.next());
            out.extend(upper.flat_map(char::to_lowercase));
            return;
        }
    };
    out.push(title);
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// Combining marks of the scripts that have case: an accent typed as a separate character stays
/// within its word.
fn is_combining_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0483}'..='\u{0489}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | 'ʼ')
}

fn is_closing(c: char) -> bool {
    matches!(c, '"' | '\'' | '»' | '”' | '’' | '›' | ')' | ']' | '}')
}

fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(text: &str) -> String {
        change_case(text, Case::Title)
    }

    fn sentence(text: &str) -> String {
        change_case(text, Case::Sentence)
    }

    fn invert(text: &str) -> String {
        change_case(text, Case::Invert)
    }

    #[test]
    fn upper_and_lower_case_change_every_letter() {
        assert_eq!(change_case("Straße und Привет", Case::Upper), "STRASSE UND ПРИВЕТ");
        assert_eq!(change_case("STRASSE und ПРИВЕТ", Case::Lower), "strasse und привет");
    }

    #[test]
    fn title_case_capitalizes_every_word() {
        assert_eq!(title("hello wORLD"), "Hello World");
        assert_eq!(title("привет, МИР!"), "Привет, Мир!");
        assert_eq!(title("  two\tspaces\nand lines  "), "  Two\tSpaces\nAnd Lines  ");
        assert_eq!(title("well-known foo_bar a.b"), "Well-Known Foo_Bar A.B");
        assert_eq!(title(""), "");
    }

    #[test]
    fn title_case_keeps_an_apostrophe_within_a_word() {
        assert_eq!(title("don't STOP"), "Don't Stop");
        assert_eq!(title("it’s o'neil"), "It’s O'neil");
        // Not between letters, an apostrophe is a quote.
        assert_eq!(title("'quoted' words'"), "'Quoted' Words'");
    }

    #[test]
    fn title_case_keeps_digits_within_a_word() {
        assert_eq!(title("the 1ST and 2nd x2Y"), "The 1st And 2nd X2y");
        assert_eq!(title("route66 is 4you"), "Route66 Is 4you");
    }

    #[test]
    fn title_case_keeps_combining_marks_within_a_word() {
        // "cafe" with a separate acute accent, then "s".
        assert_eq!(title("CAFE\u{301}S"), "Cafe\u{301}s");
    }

    #[test]
    fn title_case_changes_the_length_of_the_text() {
        assert_eq!(title("ßa STRASSE"), "Ssa Strasse");
        assert_eq!(title("ǆungla ǄUNGLA"), "ǅungla ǅungla");
        assert_eq!(title("ΟΔΟΣ ΟΔΟΣ"), "Οδος Οδος");
    }

    #[test]
    fn sentence_case_capitalizes_the_first_letter_of_a_sentence() {
        assert_eq!(sentence("HELLO WORLD. how ARE you? fine! ok… yes"), "Hello world. How are you? Fine! Ok… Yes");
        assert_eq!(sentence("один. ДВА"), "Один. Два");
        // A name becomes lower case: what is a name, the editor does not know.
        assert_eq!(sentence("i met JOHN."), "I met john.");
    }

    #[test]
    fn sentence_case_needs_a_space_after_the_end() {
        assert_eq!(sentence("see e.g.this and 3.14 more. next"), "See e.g.this and 3.14 more. Next");
        assert_eq!(sentence("end.\nnext"), "End.\nNext");
        assert_eq!(sentence("end.\r\nnext"), "End.\r\nNext");
    }

    #[test]
    fn sentence_case_starts_after_closing_quotes() {
        assert_eq!(sentence("he said \"YES.\" then LEFT"), "He said \"yes.\" Then left");
        assert_eq!(sentence("(so it goes.) and"), "(So it goes.) And");
        assert_eq!(sentence("«да.» НЕТ"), "«Да.» Нет");
    }

    #[test]
    fn sentence_case_starts_after_an_empty_line() {
        assert_eq!(sentence("TITLE\nSOME TEXT"), "Title\nsome text");
        assert_eq!(sentence("TITLE\n\nSOME TEXT"), "Title\n\nSome text");
        assert_eq!(sentence("TITLE\r\n\r\nSOME TEXT"), "Title\r\n\r\nSome text");
        assert_eq!(sentence("TITLE\n  \nSOME TEXT"), "Title\n  \nSome text");
    }

    #[test]
    fn sentence_case_starts_with_the_first_letter_or_digit() {
        assert_eq!(sentence("  ...WAIT"), "  ...Wait");
        assert_eq!(sentence("2ND PLACE. 1. FIRST"), "2nd place. 1. First");
        assert_eq!(sentence("ßA. ǄA"), "Ssa. ǅa");
    }

    #[test]
    fn invert_case_swaps_upper_and_lower_case() {
        assert_eq!(invert("Hello World"), "hELLO wORLD");
        assert_eq!(invert("Привет, Мир! 42"), "пРИВЕТ, мИР! 42");
        assert_eq!(invert("ßǅ"), "SSǆ");
        assert_eq!(invert("ΟΔΟΣ ΟΔΟΣ"), "οδος οδος");
        assert_eq!(invert(""), "");
    }
}
