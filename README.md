# MigPad

A fast cross-platform text editor for macOS, Linux and Windows, inspired by [AkelPad](https://akelpad.sourceforge.net/) (an independent project, not affiliated with it).

> **Status:** early development: MigPad opens, edits, finds and replaces, saves files in any of 38 encodings, brings everything back after quitting and has its settings, but there is no release yet.

## Goals

- **Small and fast** — instant startup, low memory use, large files without freezing the interface.
- **Careful with text** — Unicode and legacy encodings (Windows-1251, KOI8-R, CP866 and more), encoding auto-detection, BOM, LF / CRLF / CR line endings.
- **Powerful editing** — column selection, find and replace with regular expressions, multi-level undo.
- **Nothing is lost** — edits are saved continuously and survive quitting or a crash, so quitting never asks "Save changes?"; closing a tab with unsaved changes asks whether to save them.
- **Offline** — no telemetry, accounts or online services.
- **Everything built in** — no plugins: features ship with the editor, and the modular architecture lets new ones be added without reworking the core.

## Platforms

macOS, Linux (X11 and Wayland) and Windows from a single Rust codebase built on GPUI, the UI framework of the Zed editor. The interface is in English and Russian.

Windows: Windows 10 version 1903 or later and Windows 11, 64-bit, with nothing else to install. Windows 7, 8 and 8.1 are not supported: neither Rust nor GPUI supports them.

## Settings

The settings are in `settings.toml` in `~/.migpad` — or in a folder `.migpad` next to the program, which makes it portable — and in the settings window: the language, the theme, the font and its size, the width of a tab, whether the windows of the last time open again. The file can be edited by hand, kept with dotfiles and linked from there; MigPad takes a change when one of its windows comes back to the front.

A translation of your own, `locales/en.toml` or `locales/ru.toml` in the same folder, takes the place of the strings it has; its keys are those of [`crates/migpad/locales/`](crates/migpad/locales/).

## Building

See [CONTRIBUTING.md](CONTRIBUTING.md): build requirements, checks and pull requests.

## License

MigPad is released under the [MIT License](LICENSE).
