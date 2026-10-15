# Vendored crates

Copies of crates from crates.io with fixes MigPad needs before they are in a release. The root `Cargo.toml` puts them in place of the published crates with `[patch.crates-io]` and keeps them out of the workspace: they are built as they were published, without the formatting and the lints of MigPad.

## gpui-pre-linux 0.3.7

The Linux platform of GPUI ([gpui-pre](https://crates.io/crates/gpui-pre), a snapshot of the UI framework of [Zed](https://github.com/zed-industries/zed)), Apache-2.0 (`gpui-pre-linux/LICENSE-APACHE`). The changes are in `gpui-pre-linux.patch` and marked `MigPad:` in the code:

- **An opaque visual.** A window takes an opaque visual of depth 24 instead of a transparent one of depth 32. The software Vulkan of Mesa before 24.2 (lavapipe), which runs where there is no GPU — virtual machines, remote desktops — sends its frames to the X server with depth 24 when the server has no DRI3 (xrdp, VNC, Xvfb); a window of depth 32 refuses them and stays empty. MigPad's windows are opaque and decorated by the window manager, so they need no transparency.
- **Where a window is.** The origin of a window comes from the server (`TranslateCoordinates`) after each ConfigureNotify. GPUI took it from the event, which is relative to the frame of the window manager when the event is real, and kept an old one when the window was resized: after maximizing or tiling, window managers that follow ICCCM to the letter (Mutter) send no synthetic event with the position on the screen. MigPad finds the window under a tab dropped outside its own by these origins, and keeps them in the session.
- **No minimized windows in the stack.** On X11, `window_stack` leaves out minimized windows, which keep their place in `_NET_CLIENT_LIST_STACKING`: a tab dropped where one was would go into a window that is not to be seen.
- **The type of the file to save.** The save dialog of the desktop portal gets two filters, the extension of the suggested name (`*.txt` for a new document) chosen and all files. KDE shows the suggested name without its extension and, with no type chosen, saves the file without one.

## gpui-pre-windows 0.3.7

The Windows platform of GPUI, Apache-2.0 (`gpui-pre-windows/LICENSE-APACHE`). The change is in `gpui-pre-windows.patch` and marked `MigPad:` in the code:

- **The type of the file to save.** The save dialog gets the extension of the suggested name as its first type, chosen, and as the default extension, before All files. Where Explorer hides the extensions of known types, the dialog can show the suggested name without its extension; with All files alone the file was saved without one, as was a name typed without an extension. Now the extension is added, as Notepad does, unless All files is chosen.

To update GPUI: copy the new version of each crate from the cargo registry (`~/.cargo/registry/src/*/gpui-pre-linux-VERSION`, and so on) here without `Cargo.lock`, `.cargo-ok` and `Cargo.toml.orig`, apply its patch (`patch -p1 -d vendor/gpui-pre-linux < vendor/gpui-pre-linux.patch`), and drop a change once the published crate has it. If the version here no longer matches the one GPUI asks for, Cargo only warns that the patch was not used and builds the published crate without the changes: the warning must not be ignored.
