//! Turns the tables of interface strings, `locales/<language>.toml`, into code: an enum of the
//! keys and an array of the strings of each language, so that nothing is parsed when MigPad
//! starts. A key missing from a table, or found in one table only, fails the build.

use std::fmt::Write as _;
use std::path::Path;
use std::{env, fs};

use toml_edit::{Document, Item, Table};

/// The tables, English first: the base language, whose keys the others have too.
const LANGUAGES: [&str; 2] = ["en", "ru"];

fn main() {
    println!("cargo::rerun-if-changed=locales");
    let tables: Vec<Vec<(String, String)>> = LANGUAGES.iter().map(|language| read(language)).collect();
    let keys: Vec<&str> = tables[0].iter().map(|(key, _)| key.as_str()).collect();
    for (language, table) in LANGUAGES.iter().zip(&tables).skip(1) {
        let has: Vec<&str> = table.iter().map(|(key, _)| key.as_str()).collect();
        let missing: Vec<&str> = keys.iter().copied().filter(|key| !has.contains(key)).collect();
        let unknown: Vec<&str> = has.iter().copied().filter(|key| !keys.contains(key)).collect();
        if !missing.is_empty() || !unknown.is_empty() {
            panic!("locales/{language}.toml: missing keys {missing:?}, keys that en.toml has not {unknown:?}");
        }
    }

    let mut code = String::from("// Made by build.rs from locales/*.toml.\n\n");
    code += "/// A string of the interface, by its key in the tables.\n";
    code += "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub enum Key {\n";
    for key in &keys {
        writeln!(code, "    /// `{key}`\n    {},", variant(key)).unwrap();
    }
    code += "}\n";
    for (language, table) in LANGUAGES.iter().zip(&tables) {
        let strings: Vec<&str> = keys
            .iter()
            .map(|key| table.iter().find(|(name, _)| name == key).expect("checked above").1.as_str())
            .collect();
        let name = language.to_uppercase();
        writeln!(
            code,
            "\n/// The strings of `locales/{language}.toml`, by key.\nconst {name}: [&str; {}] = {strings:?};",
            keys.len()
        )
        .unwrap();
    }
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("strings.rs"), code).unwrap();
}

/// The keys and the strings of the table of `language`, in their order there; `undo` in the
/// table `[edit]` is the key `edit.undo`.
fn read(language: &str) -> Vec<(String, String)> {
    let path = format!("locales/{language}.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let document = Document::parse(text).unwrap_or_else(|error| panic!("{path}: {error}"));
    let mut strings = Vec::new();
    collect(document.as_table(), "", &path, &mut strings);
    strings
}

fn collect(table: &Table, prefix: &str, path: &str, strings: &mut Vec<(String, String)>) {
    for (name, item) in table.iter() {
        let key = format!("{prefix}{name}");
        match item {
            Item::Table(table) => collect(table, &format!("{key}."), path, strings),
            Item::Value(value) if value.is_str() => strings.push((key, value.as_str().unwrap_or_default().to_owned())),
            _ => panic!("{path}: {key} is not a string"),
        }
    }
}

/// The variant of the enum for a key: `edit.select_all` is `EditSelectAll`.
fn variant(key: &str) -> String {
    key.split(['.', '_'])
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}
