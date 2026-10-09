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

## Installation

Download MigPad from [the releases](https://github.com/migpad/migpad/releases/latest) or [migpad.com](https://migpad.com). It never goes online, so it does not look for updates either: new versions are there.

- **macOS** 10.15.7 or later, Apple silicon and Intel: `MigPad.dmg` — drag MigPad to Applications. MigPad ▸ Install the migpad Command… adds the command `migpad` for terminals; the button Make MigPad the Default in its settings opens text files in it.
- **Windows** 10 version 1903 or later and 11, x64 and ARM64: the installer, `MigPad-x64-setup.exe` or `MigPad-arm64-setup.exe`, installs MigPad for you alone without administrator rights, or for all users; it can add the command `migpad` to `PATH`, Open in MigPad to the context menu of files in Explorer, and start MigPad in place of Notepad. The portable `MigPad-x64-portable.zip` and `MigPad-arm64-portable.zip` keep the data in the folder `.migpad` next to `migpad.exe`. MigPad for Windows is not signed yet, and SmartScreen warns about it: choose More info, then Run anyway.
- **Linux**, x86_64 and ARM64, with glibc 2.35 or later — Ubuntu 22.04, Debian 12, Fedora 36 and later: `sudo apt install ./migpad-amd64.deb`, `sudo dnf install ./migpad-x86_64.rpm`, or `MigPad-x86_64.AppImage`, which runs as it is once made executable (`chmod +x`); a folder `.migpad` next to the AppImage makes it portable. MigPad draws with Vulkan: the system needs a Vulkan driver, such as Mesa's.

## Settings

The settings are in `settings.toml` in `~/.migpad` — or in a folder `.migpad` next to the program, which makes it portable — and in the settings window: the language, the theme, the font and its size, the width of a tab, whether the windows of the last time open again. The file can be edited by hand, kept with dotfiles and linked from there; MigPad takes a change when one of its windows comes back to the front.

A translation of your own, `locales/en.toml` or `locales/ru.toml` in the same folder, takes the place of the strings it has; its keys are those of [`crates/migpad/locales/`](crates/migpad/locales/).

## Building

See [CONTRIBUTING.md](CONTRIBUTING.md): build requirements, checks and pull requests.

## License

MigPad is released under the [MIT License](LICENSE).
