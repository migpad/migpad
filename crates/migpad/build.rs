//! Turns the tables of interface strings, `locales/<language>.toml`, into code: enums of the keys
//! of strings and of strings about a number, and for each language arrays of the strings, of their
//! mnemonics and of the forms of the strings about a number, so that nothing is parsed when MigPad
//! starts. A key missing from a table, or found in one table only, a placeholder that one table
//! has and another has not, forms other than those of the language, or a stray "&" fails the
//! build.

use std::fmt::Write as _;
use std::path::Path;
use std::{env, fs};

#[path = "src/table.rs"]
mod table;

use table::{Entry, forms, mnemonic, placeholders, plural_forms};

/// The tables, English first: the base language, whose keys the others have too.
const LANGUAGES: [&str; 2] = ["en", "ru"];

fn main() {
    println!("cargo::rerun-if-changed=locales");
    println!("cargo::rerun-if-changed=src/table.rs");
    let tables: Vec<Vec<(String, Entry)>> = LANGUAGES.iter().map(|language| read(language)).collect();
    let base = &tables[0];
    for (language, table) in LANGUAGES.iter().zip(&tables).skip(1) {
        let has: Vec<&str> = table.iter().map(|(key, _)| key.as_str()).collect();
        let missing: Vec<&str> = base.iter().map(|(key, _)| key.as_str()).filter(|key| !has.contains(key)).collect();
        let unknown: Vec<&str> = has.iter().copied().filter(|key| !base.iter().any(|(name, _)| name == key)).collect();
        if !missing.is_empty() || !unknown.is_empty() {
            panic!("locales/{language}.toml: missing keys {missing:?}, keys that en.toml has not {unknown:?}");
        }
    }
    let texts: Vec<&str> = keys_of(base, |entry| matches!(entry, Entry::Text(_)));
    let plurals: Vec<&str> = keys_of(base, |entry| matches!(entry, Entry::Plural(_)));

    let mut code = String::from("// Made by build.rs from locales/*.toml.\n\n");
    code += "/// A string of the interface, by its key in the tables.\n";
    code += "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub enum Key {\n";
    for key in &texts {
        writeln!(code, "    /// `{key}`\n    {},", variant(key)).unwrap();
    }
    code += "}\n\n/// A string of the interface about a number, by its key in the tables.\n";
    code += "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub enum Plural {\n";
    for key in &plurals {
        writeln!(code, "    /// `{key}`\n    {},", variant(key)).unwrap();
    }
    code += "}\n";
    writeln!(
        code,
        "\n/// The keys of the strings, as the tables name them.\npub const KEYS: [&str; {}] = {texts:?};",
        texts.len()
    )
    .unwrap();
    writeln!(
        code,
        "/// The keys of the strings about a number, as the tables name them.\npub const PLURAL_KEYS: [&str; {}] = {plurals:?};",
        plurals.len()
    )
    .unwrap();
    for (language, table) in LANGUAGES.iter().zip(&tables) {
        let entry = |key: &str| &table.iter().find(|(name, _)| name == key).expect("checked above").1;
        let (strings, mnemonics): (Vec<String>, Vec<Option<usize>>) = texts
            .iter()
            .map(|key| {
                let Entry::Text(string) = entry(key) else {
                    panic!("locales/{language}.toml: {key} is the forms of a number, not a string as in en.toml")
                };
                if placeholders(string) != placeholders(text_of(base, key)) {
                    panic!("locales/{language}.toml: {key} has other placeholders than in en.toml");
                }
                mnemonic(string).unwrap_or_else(|| panic!("locales/{language}.toml: {key}: a stray \"&\""))
            })
            .unzip();
        let forms_of: Vec<Vec<String>> = plurals
            .iter()
            .map(|key| {
                let Entry::Plural(got) = entry(key) else {
                    panic!("locales/{language}.toml: {key} is a string, not the forms of a number as in en.toml")
                };
                let Entry::Plural(english) = entry_of(base, key) else { unreachable!() };
                let base = placeholders(&english[0].1);
                let ordered = plural_forms(language, got, &base)
                    .unwrap_or_else(|why| panic!("locales/{language}.toml: {key}: {why}"));
                ordered
                    .into_iter()
                    .map(|string| match mnemonic(string) {
                        Some((plain, None)) => plain,
                        _ => panic!("locales/{language}.toml: {key}: a \"&\" in the forms of a number"),
                    })
                    .collect()
            })
            .collect();
        let name = language.to_uppercase();
        let (count, plural_count, form_count) = (texts.len(), plurals.len(), forms(language).len());
        writeln!(
            code,
            "\n/// The strings of `locales/{language}.toml`, by key, without the marks of mnemonics.\n\
             const {name}: [&str; {count}] = {strings:?};\n\
             /// Where the mnemonic of each string is, in bytes.\n\
             const {name}_MNEMONICS: [Option<usize>; {count}] = {mnemonics:?};\n\
             /// The forms of each string about a number, in the order of the forms of the language.\n\
             const {name}_PLURALS: [[&str; {form_count}]; {plural_count}] = {forms_of:?};"
        )
        .unwrap();
    }
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("strings.rs"), code).unwrap();
}

/// The strings of the table of `language`, in their order there.
fn read(language: &str) -> Vec<(String, Entry)> {
    let path = format!("locales/{language}.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    table::parse(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The keys of the strings of `table` that are `kind`.
fn keys_of(table: &[(String, Entry)], kind: fn(&Entry) -> bool) -> Vec<&str> {
    table.iter().filter(|(_, entry)| kind(entry)).map(|(key, _)| key.as_str()).collect()
}

fn entry_of<'a>(table: &'a [(String, Entry)], key: &str) -> &'a Entry {
    &table.iter().find(|(name, _)| name == key).expect("a key of en.toml").1
}

fn text_of<'a>(table: &'a [(String, Entry)], key: &str) -> &'a str {
    match entry_of(table, key) {
        Entry::Text(string) => string,
        Entry::Plural(_) => unreachable!("a key of a string"),
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
