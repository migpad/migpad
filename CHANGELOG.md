# Changelog

What changed in each version of MigPad. MigPad follows [Semantic Versioning](https://semver.org/).

## [0.1.0] — 2026-10-10

### Features

- Add the text store, gap buffer and line index ([#2](https://github.com/migpad/migpad/pull/2))
- Add encodings and line endings ([#3](https://github.com/migpad/migpad/pull/3))
- Add documents and staged file loading ([#4](https://github.com/migpad/migpad/pull/4))
- Add edit transactions with undo and redo ([#5](https://github.com/migpad/migpad/pull/5))
- Add the edit journal format with replay ([#6](https://github.com/migpad/migpad/pull/6))
- Keep the edit journal in documents ([#7](https://github.com/migpad/migpad/pull/7))
- Save documents without losses ([#8](https://github.com/migpad/migpad/pull/8))
- Add find and replace ([#9](https://github.com/migpad/migpad/pull/9))
- Show documents with line numbers and scrolling ([#10](https://github.com/migpad/migpad/pull/10))
- Add the caret and selection ([#12](https://github.com/migpad/migpad/pull/12))
- Add text input, the clipboard, undo and IME ([#13](https://github.com/migpad/migpad/pull/13))
- Show whitespace and indent guides; exact columns of long lines ([#14](https://github.com/migpad/migpad/pull/14))
- Wrap lines to the width of the view ([#15](https://github.com/migpad/migpad/pull/15))
- Add the single-line mode for input fields ([#16](https://github.com/migpad/migpad/pull/16))
- Add commands and the menu bar of macOS ([#17](https://github.com/migpad/migpad/pull/17))
- Add the shell: tabs and windows, the menu bar of Windows and Linux, the status bar, themes, notification bars ([#18](https://github.com/migpad/migpad/pull/18))
- Work with files: open and save, the question on closing, journals, the session, recent files, changes by other programs ([#19](https://github.com/migpad/migpad/pull/19))
- Find and replace, go to line, context menus, encodings and line endings, one MigPad and the migpad command ([#21](https://github.com/migpad/migpad/pull/21))
- Settings: settings.toml, the settings window, the language and translations ([#22](https://github.com/migpad/migpad/pull/22))
- Packages for macOS, Windows and Linux, About MigPad and the release workflow ([#23](https://github.com/migpad/migpad/pull/23))

### Fixes

- Read mostly valid UTF-8 as UTF-8 ([#11](https://github.com/migpad/migpad/pull/11))

### Build

- Set up the Cargo workspace and CI ([#1](https://github.com/migpad/migpad/pull/1))
- Add release artifacts and link the C runtime statically on Windows ([#20](https://github.com/migpad/migpad/pull/20))
