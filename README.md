# MigPad

A fast cross-platform text editor for macOS, Linux and Windows, inspired by [AkelPad](https://akelpad.sourceforge.net/) (an independent project, not affiliated with it). The name comes from the Russian «миг», "an instant".

> **Status:** early design stage, no code yet.

## Goals

- **Small and fast** — instant startup, low memory use, large files without freezing the interface.
- **Careful with text** — Unicode and legacy encodings (Windows-1251, KOI8-R, CP866 and more), encoding auto-detection, BOM, LF / CRLF / CR line endings.
- **Powerful editing** — column selection, find and replace with regular expressions, multi-level undo.
- **Nothing is lost** — edits are saved continuously and survive closing the editor or a crash, so it never asks "Save changes?".
- **Offline** — no telemetry, accounts or online services.
- **Everything built in** — no plugins: features ship with the editor, and the modular architecture lets new ones be added without reworking the core.

## Platforms

macOS, Linux (X11 and Wayland) and Windows from a single Rust codebase built on GPUI, the UI framework of the Zed editor. The interface will be available in English and Russian.

## License

Not chosen yet; it will be decided before the first public release.
