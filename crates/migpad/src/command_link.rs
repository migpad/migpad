//! The command `migpad` on macOS, from the menu of the application: a link in `/usr/local/bin`,
//! which terminals have in `PATH`, to the program in its bundle. The installer of Windows and the
//! packages of Linux put the command in place themselves.

use std::io;
use std::path::Path;
use std::process::Command;

use gpui::{App, AppContext as _};

use crate::notices;
use crate::strings::{Key, fill};
use crate::windows;

/// The link.
const LINK: &str = "/usr/local/bin/migpad";

/// Whether there is a program to link to: MigPad runs from its bundle.
pub fn available() -> bool {
    cfg!(target_os = "macos")
        && migpad_core::data::program()
            .is_some_and(|program| program.parent().is_some_and(|folder| folder.ends_with("Contents/MacOS")))
}

/// Makes the link, asking for the password of an administrator where the folder is not the
/// user's, and tells over the window used last how it went.
pub fn install(cx: &mut App) {
    let Some(program) = migpad_core::data::program() else { return };
    if temporary(&program) {
        tell(notices::command_temporary(), cx);
        return;
    }
    let made = cx.background_spawn(async move { link(&program, Path::new(LINK)) });
    cx.spawn(async move |cx| {
        let notice = match made.await {
            Ok(()) => notices::command_installed(LINK),
            Err(Failure::Canceled) => return,
            Err(Failure::Error(reason)) => notices::command_failed(&reason),
        };
        cx.update(|cx| tell(notice, cx));
    })
    .detach();
}

/// Tells `notice` over the window used last.
fn tell(notice: notices::Notice, cx: &mut App) {
    if let Some(window) = windows::last_active(cx) {
        let _ = window.update(cx, |workspace, _, cx| workspace.notify(notice, cx));
    }
}

/// Whether the program runs from where it does not stay, so that a link to it would lead nowhere:
/// a disk image, until it is ejected, or the copy macOS runs of a program from the internet left
/// where it was downloaded (App Translocation), until it quits.
fn temporary(program: &Path) -> bool {
    program.to_string_lossy().contains("/AppTranslocation/") || read_only(program)
}

/// Whether the disk of `path` cannot be written, as that of a disk image.
#[cfg(unix)]
fn read_only(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a path that ends with a zero, and a structure for the system to fill.
    unsafe {
        let mut disk: libc::statvfs = std::mem::zeroed();
        libc::statvfs(path.as_ptr(), &mut disk) == 0 && disk.f_flag & libc::ST_RDONLY != 0
    }
}

#[cfg(not(unix))]
fn read_only(_path: &Path) -> bool {
    false
}

enum Failure {
    /// The password was not given.
    Canceled,
    Error(String),
}

/// Links `link` to `program`: a link that is there is replaced, a file is left alone.
fn link(program: &Path, link: &Path) -> Result<(), Failure> {
    match std::fs::read_link(link) {
        Ok(target) if target == program => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Failure::Error(fill(Key::NoticeCommandInTheWay, &[("path", &link.to_string_lossy())]))),
    }
    // Without a password where the folder is the user's, as Homebrew makes it on Intel.
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(link);
        if std::os::unix::fs::symlink(program, link).is_ok() {
            return Ok(());
        }
    }
    // As an administrator: macOS asks for the password.
    let quoted = |path: &Path| path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"do shell script "mkdir -p /usr/local/bin && ln -sfn " & quoted form of "{}" & " " & quoted form of "{}" with administrator privileges"#,
        quoted(program),
        quoted(link),
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| Failure::Error(error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr);
    // -128: the dialog of the password was cancelled.
    if message.contains("(-128)") { Err(Failure::Canceled) } else { Err(Failure::Error(message.trim().to_owned())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_macos_moved_out_of_quarantine_is_temporary() {
        let translocated = "/private/var/folders/x/T/AppTranslocation/5F1C/d/MigPad.app/Contents/MacOS/migpad";
        assert!(temporary(Path::new(translocated)));
    }
}
