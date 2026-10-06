# Contributing to MigPad

MigPad is in early development, and its design is still settling. Before starting on a larger change, please open an issue to discuss it.

## Building

MigPad is written in Rust. Install Rust with [rustup](https://rustup.rs/): the toolchain version is pinned in `rust-toolchain.toml` and is installed on the first build.

Platform requirements:

- **macOS:** Xcode with the Metal toolchain — GPUI compiles its shaders at build time. Since Xcode 26 the toolchain is a separate download: `xcodebuild -downloadComponent MetalToolchain`.
- **Linux:** a C compiler, `pkg-config` and the development packages for fontconfig, FreeType, xcb and xkbcommon. On Debian and Ubuntu:

  ```sh
  sudo apt install gcc pkg-config libfontconfig-dev libfreetype-dev libxcb1-dev libxkbcommon-x11-dev
  ```

- **Windows:** the MSVC build tools that rustup asks for (Visual Studio Build Tools with the C++ workload), including a Windows SDK — GPUI compiles its shaders with `fxc.exe` from it. The C runtime is linked statically (`.cargo/config.toml`), so `migpad.exe` starts without the Visual C++ Redistributable; `RUSTFLAGS` set in the environment would replace that flag.

Then:

```sh
cargo run                # debug build
cargo build --release    # target/release/migpad
```

## Code layout

A Cargo workspace of four crates in `crates/`; each depends only on the crates listed above it:

- `migpad-core` — text storage, encodings and line endings, editing and undo, the edit journal, search, settings; no UI;
- `migpad-editor` — the editor view on GPUI: rendering, selections, text input;
- `migpad-ui` — interface elements: controls, theme, menus, notification bars;
- `migpad` — the application: windows and tabs, commands, features.

## Checks

CI runs these on Linux, Windows and macOS. Run them before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also runs [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (`cargo deny check`, configured in `deny.toml`): dependency licenses, security advisories, crate sources, and a ban on networking crates — MigPad never goes online.

## Checking the interface

Debug builds play input from the `MIGPAD_DEBUG_INPUT` environment variable through GPUI, as if it were typed and clicked: key bindings, actions and mouse handlers run as for real input. Changes to the interface can then be checked with screenshots:

```sh
MIGPAD_DEBUG_INPUT="down shift-end click:300,120,2" cargo run -- notes.txt
```

Steps are keystrokes (`shift-end`, `cmd-a`), text as typed or committed by an input method (`type:TEXT`), text an input method composes (`mark:TEXT`, then `unmark` or `type:TEXT`), `click:X,Y` with an optional number of clicks, `press:X,Y`, `move:X,Y`, `release:X,Y` and `wait:MS`; see `crates/migpad/src/debug_input.rs`. The input bypasses the system (keyboard layouts, input methods, menus), so real keyboards and mice still need a check by hand.

The interface is in the language of the system: Russian on a Russian system, English otherwise. The strings are in `crates/migpad/locales/`; `en.toml` and `ru.toml` have the same keys and placeholders, or the build fails; in the strings of menus `&` marks the mnemonic, the letter that chooses the item with Alt on Windows and Linux. On macOS the other language can be tried without changing the system:

```sh
cargo run -- -AppleLanguages '(en)' notes.txt
```

The theme follows the appearance of the system; a debug build can be held light or dark, and on macOS it can show the menu bar that MigPad draws on Windows and Linux:

```sh
MIGPAD_THEME=dark MIGPAD_MENU_BAR=1 cargo run -- notes.txt
```

MigPad keeps its data — the journals of the documents, the session, the recent files — in `~/.migpad`, or in a folder `.migpad` next to the program if there is one (next to `MigPad.app` on macOS), which makes it portable. A debug build takes another folder from `MIGPAD_DATA_DIR`, so that a check starts from nothing and leaves the data of every day alone:

```sh
MIGPAD_DATA_DIR=/tmp/migpad-check cargo run -- notes.txt
```

Input fields — the single-line mode of the editor view — can be tried in an example window with two fields, as in a find bar:

```sh
cargo run -p migpad-editor --example fields
```

## Pull requests

- Branch from `main` and name the branch `<type>/<topic>`, for example `feat/gap-buffer`.
- Pull requests are squash-merged: the pull request title becomes the commit message on `main`.
- Titles and commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `<type>(<scope>): <summary>`, for example `feat(core): add the gap buffer`.
  - Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`.
  - Scopes: `core`, `editor`, `ui`, `app`, `ci`, `deps`, `docs`.
  - Incompatible changes to the settings or journal file formats are marked with `!`, for example `feat(core)!: …`.
- Code, comments and documentation are in English.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
