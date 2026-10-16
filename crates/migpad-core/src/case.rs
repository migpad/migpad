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
    let mut lower = Lower::of(text);
    let mut in_word = false;
    for (at, c) in text.char_indices() {
        let joins = || is_apostrophe(c) && text[at + c.len_utf8()..].chars().next().is_some_and(char::is_alphanumeric);
        let word = is_word_char(c) || (in_word && joins());
        let lowered = lower.next(c);
        match (word, in_word) {
            (true, false) => push_title(&mut out, c),
            (true, true) => out.push_str(lowered),
            (false, _) => out.push(c),
        }
        in_word = word;
    }
    out
}

/// A sentence starts at the start of the text, after `.`, `!`, `?` or `…` and a space or a line
/// break, and after an empty line. Closing quotes and brackets may stand between the end of a
/// sentence and the space: `"Yes." He…`.
fn sentence_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lower = Lower::of(text);
    let mut start = true;
    let mut after_end = false;
    let mut line_empty = true;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let lowered = lower.next(c);
        if c.is_alphanumeric() && start {
            push_title(&mut out, c);
        } else {
            out.push_str(lowered);
        }
        if c.is_alphanumeric() {
            start = false;
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
            let crlf = c == '\r' && chars.peek() == Some(&'\n');
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
    out
}

/// Lower case of a title case letter such as `ǅ`, of an upper case one and of others alike: these
/// have no lower case but themselves.
fn invert_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lower = Lower::of(text);
    for c in text.chars() {
        let lowered = lower.next(c);
        if c.is_lowercase() {
            out.extend(c.to_uppercase());
        } else {
            out.push_str(lowered);
        }
    }
    out
}

/// The lower case of a text, character by character, from its lower case whole: a Greek sigma at
/// the end of a word is final, whatever the case of the letters around it is made.
struct Lower {
    text: String,
    at: usize,
}

impl Lower {
    fn of(text: &str) -> Self {
        Lower { text: text.to_lowercase(), at: 0 }
    }

    /// The lower case of `c`, the next character of the text. The text in lower case is that of
    /// its characters one by one, but for the sigma, whose two forms are as long.
    fn next(&mut self, c: char) -> &str {
        let len: usize = c.to_lowercase().map(char::len_utf8).sum();
        self.at += len;
        &self.text[self.at - len..self.at]
    }
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
        assert_eq!(title("ΩΣ ΕΙΠΑ"), "Ως Ειπα");
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
        assert_eq!(sentence("ΟΣ ΕΙΠΑ. ΩΣ"), "Ος ειπα. Ως");
    }

    #[test]
    fn invert_case_swaps_upper_and_lower_case() {
        assert_eq!(invert("Hello World"), "hELLO wORLD");
        assert_eq!(invert("Привет, Мир! 42"), "пРИВЕТ, мИР! 42");
        assert_eq!(invert("ßǅ"), "SSǆ");
        assert_eq!(invert("ΟΔΟΣ ΟΔΟΣ"), "οδος οδος");
        assert_eq!(invert("οδοΣ ΣΟ"), "ΟΔΟς σο");
        assert_eq!(invert(""), "");
    }
}
