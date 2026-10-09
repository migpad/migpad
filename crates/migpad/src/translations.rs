//! Translations of the interface the user put in the folder of the data — `locales/en.toml`,
//! `locales/ru.toml` — read as MigPad starts, in place of the strings built in. What MigPad cannot
//! take of them is told in the first window.

use std::path::Path;

use gpui::{App, Global};

use crate::notices::{self, Notice};
use crate::strings::{self, Language};

/// What to tell about the translations, once the first window opens.
#[derive(Default)]
struct Told(Vec<Notice>);

impl Global for Told {}

/// Reads the translations in the folder of the data `root`, if there are any.
pub fn load(root: &Path, cx: &mut App) {
    let mut told = Vec::new();
    for language in [Language::English, Language::Russian] {
        let name = format!("{}.toml", language.code());
        let path = root.join("locales").join(&name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                told.push(notices::translation_unreadable(&name, &error.to_string()));
                continue;
            }
        };
        match strings::use_translation(language, &text) {
            Ok(unfit) if unfit.is_empty() => {}
            Ok(unfit) => told.push(notices::translation_unfit(&name, &unfit)),
            Err(reason) => told.push(notices::translation_unreadable(&name, &reason)),
        }
    }
    cx.set_global(Told(told));
}

/// What to tell about the translations as MigPad starts, once.
pub fn take_notices(cx: &mut App) -> Vec<Notice> {
    if cx.has_global::<Told>() { std::mem::take(&mut cx.global_mut::<Told>().0) } else { Vec::new() }
}
