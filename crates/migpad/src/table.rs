//! A table of the strings of the interface, `<language>.toml`: build.rs turns the tables built in
//! into code, and MigPad reads a translation the user put in the folder of the data. A key is the
//! name of a table and of a string in it: `undo` in `[edit]` is `edit.undo`. A string is text, or
//! — for a string about a number — the forms the language has for numbers:
//! `{ one = "{count} line", other = "{count} lines" }`.

use std::collections::BTreeSet;
use std::fmt;

use toml_edit::{Document, Item, Table, Value};

/// A string of a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Text(String),
    /// The forms of a string about a number, by name, in the order of the table.
    Plural(Vec<(String, String)>),
}

/// The forms a language has for numbers, in the order the code keeps them: Russian has one, few
/// and many — 1, 2, 5 lines — English one and other.
pub fn forms(language: &str) -> &'static [&'static str] {
    match language {
        "ru" => &["one", "few", "many"],
        _ => &["one", "other"],
    }
}

/// The strings of the table `text`, in their order there; what is not a table of strings is told.
pub fn parse(text: &str) -> Result<Vec<(String, Entry)>, String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    collect(document.as_table(), "", &mut entries)?;
    Ok(entries)
}

fn collect(table: &Table, prefix: &str, entries: &mut Vec<(String, Entry)>) -> Result<(), String> {
    for (name, item) in table.iter() {
        let key = format!("{prefix}{name}");
        match item {
            Item::Table(table) => collect(table, &format!("{key}."), entries)?,
            Item::Value(Value::String(string)) => entries.push((key, Entry::Text(string.value().clone()))),
            Item::Value(Value::InlineTable(forms)) => {
                let forms = forms
                    .iter()
                    .map(|(form, value)| match value.as_str() {
                        Some(string) => Ok((form.to_owned(), string.to_owned())),
                        None => Err(format!("{key}.{form} is not a string")),
                    })
                    .collect::<Result<_, _>>()?;
                entries.push((key, Entry::Plural(forms)));
            }
            _ => return Err(format!("{key} is neither a string nor the forms of one")),
        }
    }
    Ok(())
}

/// The string without the mark of its mnemonic, and where the mnemonic is: "Cu&t" is "Cut" with
/// the mnemonic at 2; "&&" is "&". None for a "&" before nothing or a second mnemonic.
pub fn mnemonic(string: &str) -> Option<(String, Option<usize>)> {
    let mut plain = String::with_capacity(string.len());
    let mut at = None;
    let mut chars = string.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            plain.push(c);
            continue;
        }
        match chars.next()? {
            '&' => plain.push('&'),
            next if at.is_none() => {
                at = Some(plain.len());
                plain.push(next);
            }
            _ => return None,
        }
    }
    Some((plain, at))
}

/// The names of the placeholders of a string: `{count}` is `count`.
pub fn placeholders(string: &str) -> BTreeSet<&str> {
    string.split('{').skip(1).filter_map(|part| part.split_once('}').map(|(name, _)| name)).collect()
}

/// What is wrong with the forms of a string about a number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormsError {
    /// A form the language has is not there.
    Missing(&'static str),
    /// A form the language has not.
    Extra(String),
    /// Placeholders other than those of the string in English.
    Placeholders,
}

impl fmt::Display for FormsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormsError::Missing(form) => write!(f, "no form {form}"),
            FormsError::Extra(form) => write!(f, "a form {form} that the language has not"),
            FormsError::Placeholders => f.write_str("other placeholders than in English"),
        }
    }
}

/// The forms `got` of a string about a number in `language`, in the order of [`forms`]; `base` are
/// the placeholders of the string in English, which each form has too.
pub fn plural_forms<'a>(
    language: &str,
    got: &'a [(String, String)],
    base: &BTreeSet<&str>,
) -> Result<Vec<&'a str>, FormsError> {
    let names = forms(language);
    let mut ordered = Vec::with_capacity(names.len());
    for name in names {
        let string = got.iter().find(|(form, _)| form == name).map(|(_, string)| string.as_str());
        ordered.push(string.ok_or(FormsError::Missing(name))?);
    }
    if let Some((form, _)) = got.iter().find(|(form, _)| !names.contains(&form.as_str())) {
        return Err(FormsError::Extra(form.clone()));
    }
    if ordered.iter().any(|string| placeholders(string) != *base) {
        return Err(FormsError::Placeholders);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms_of(forms: &[(&str, &str)]) -> Vec<(String, String)> {
        forms.iter().map(|(form, string)| (form.to_string(), string.to_string())).collect()
    }

    #[test]
    fn the_forms_are_those_of_the_language() {
        let base = placeholders("{count} lines");
        let russian = forms_of(&[("many", "{count} строк"), ("one", "{count} строка"), ("few", "{count} строки")]);
        assert_eq!(plural_forms("ru", &russian, &base), Ok(vec!["{count} строка", "{count} строки", "{count} строк"]));
        assert_eq!(plural_forms("ru", &russian[..2], &base), Err(FormsError::Missing("few")));
        let english = forms_of(&[("one", "{count} line"), ("other", "{count} lines"), ("few", "{count} lines")]);
        assert_eq!(plural_forms("en", &english, &base), Err(FormsError::Extra("few".to_owned())));
        let english = forms_of(&[("one", "a line"), ("other", "{count} lines")]);
        assert_eq!(plural_forms("en", &english, &base), Err(FormsError::Placeholders));
    }

    #[test]
    fn a_table_has_strings_and_forms() {
        let table = "top = \"Top\"\n[edit]\nundo = \"&Undo\"\nlines = { one = \"line\", other = \"lines\" }\n";
        assert_eq!(
            parse(table),
            Ok(vec![
                ("top".to_owned(), Entry::Text("Top".to_owned())),
                ("edit.undo".to_owned(), Entry::Text("&Undo".to_owned())),
                ("edit.lines".to_owned(), Entry::Plural(forms_of(&[("one", "line"), ("other", "lines")]))),
            ])
        );
        assert!(parse("[edit]\nundo = 1\n").is_err());
        assert!(parse("[edit]\nlines = { one = 1 }\n").is_err());
        assert_eq!(mnemonic("Cu&t"), Some(("Cut".to_owned(), Some(2))));
        assert_eq!(mnemonic("A && B"), Some(("A & B".to_owned(), None)));
        assert_eq!(mnemonic("&A&B"), None);
    }
}
