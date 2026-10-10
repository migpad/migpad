# Vendored crates

Copies of crates from crates.io with fixes MigPad needs before they are in a release. The root `Cargo.toml` puts them in place of the published crates with `[patch.crates-io]` and keeps them out of the workspace: they are built as they were published, without the formatting and the lints of MigPad.

## gpui-pre-linux 0.3.7

The Linux platform of GPUI ([gpui-pre](https://crates.io/crates/gpui-pre), a snapshot of the UI framework of [Zed](https://github.com/zed-industries/zed)), Apache-2.0 (`gpui-pre-linux/LICENSE-APACHE`).

The fix, `gpui-pre-linux.patch`: on X11 a window takes an opaque visual of depth 24 instead of a transparent one of depth 32. The software Vulkan of Mesa before 24.2 (lavapipe), which runs where there is no GPU — virtual machines, remote desktops — sends its frames to the X server with depth 24 when the server has no DRI3 (xrdp, VNC, Xvfb); a window of depth 32 refuses them and stays empty. MigPad's windows are opaque and decorated by the window manager, so they need no transparency.

To update GPUI: copy the new version of the crate from the cargo registry (`~/.cargo/registry/src/*/gpui-pre-linux-VERSION`) here without `Cargo.lock`, `.cargo-ok` and `Cargo.toml.orig`, apply the patch (`patch -p1 -d vendor/gpui-pre-linux < vendor/gpui-pre-linux.patch`), and drop the copy once the published crate has the fix.
