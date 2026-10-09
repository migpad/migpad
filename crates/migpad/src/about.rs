//! About MigPad: its version, its license and the licenses of the components it is built from. On
//! macOS — the panel of the system, from the menu of the application; on Windows and Linux — a
//! question over the window, from the Help menu.

use std::path::{Path, PathBuf};

use gpui::App;

/// The version of this build.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The site of MigPad.
const SITE: &str = "https://migpad.com";
/// The file of the licenses of the components MigPad is built from, which the packages carry: plain
/// text, which MigPad opens itself — a browser of a sandbox, such as a snap, may not see the file.
const THIRD_PARTY_LICENSES: &str = "THIRD-PARTY-LICENSES.txt";

pub fn show(cx: &mut App) {
    let licenses = third_party_licenses();
    // A debug build shows the question of Windows and Linux on macOS too, for checks.
    #[cfg(target_os = "macos")]
    if !cfg!(debug_assertions) || std::env::var_os("MIGPAD_OWN_ABOUT").is_none() {
        macos::show(licenses.as_deref());
        return;
    }
    own::show(licenses, cx);
}

/// The file of the licenses of the components, where the package put it: in the bundle on macOS,
/// next to `migpad.exe` on Windows, in `share/doc/migpad` on Linux. A build that is not in a
/// package has none.
fn third_party_licenses() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    licenses_for(&exe, Path::is_file).map(without_verbatim)
}

/// A path of Windows without `\\?\`, which canonical paths start with there: as the path of a tab.
fn without_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

fn licenses_for(exe: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let folder = exe.parent()?;
    let places = [folder.join("../Resources"), folder.to_path_buf(), folder.join("../share/doc/migpad")];
    places.into_iter().map(|place| place.join(THIRD_PARTY_LICENSES)).find(|file| exists(file))
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits,
        NSAboutPanelOptionVersion, NSApplication, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
        NSLinkAttributeName, NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSTextAlignment,
    };
    use objc2_foundation::{NSAttributedString, NSDictionary, NSMutableAttributedString, NSString, NSURL};

    use super::{SITE, VERSION};
    use crate::strings::{Key, tr};

    /// The panel of the system, with the icon of the bundle and the copyright of its `Info.plist`.
    pub fn show(licenses: Option<&Path>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let name = NSString::from_str("MigPad");
        let version = NSString::from_str(VERSION);
        // The panel shows the number of the build after the version: there is none apart from it.
        let build = NSString::from_str("");
        let credits = credits(licenses);
        let keys = unsafe {
            [
                NSAboutPanelOptionApplicationName,
                NSAboutPanelOptionApplicationVersion,
                NSAboutPanelOptionVersion,
                NSAboutPanelOptionCredits,
            ]
        };
        let values: [&AnyObject; 4] = [&name, &version, &build, &credits];
        let options = NSDictionary::from_slices(&keys, &values);
        unsafe { NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options) };
    }

    /// The lines under the version: the license and the site, then the licenses of the components,
    /// as links.
    fn credits(licenses: Option<&Path>) -> Retained<NSAttributedString> {
        let text = NSMutableAttributedString::new();
        let site = NSURL::URLWithString(&NSString::from_str(SITE));
        text.appendAttributedString(&run(tr(Key::AboutLicense), None));
        text.appendAttributedString(&run("  ·  ", None));
        text.appendAttributedString(&run(SITE.trim_start_matches("https://"), site.as_deref()));
        if let Some(licenses) = licenses {
            let file = NSURL::fileURLWithPath(&NSString::from_str(&licenses.to_string_lossy()));
            text.appendAttributedString(&run("\n", None));
            text.appendAttributedString(&run(tr(Key::AboutThirdParty), Some(&file)));
        }
        Retained::into_super(text)
    }

    /// `part` in the small font of the system, centered, in a color of labels — which follows the
    /// light and dark appearance; a link if it has one.
    fn run(part: &str, link: Option<&NSURL>) -> Retained<NSAttributedString> {
        let font = NSFont::systemFontOfSize(NSFont::smallSystemFontSize());
        let color = NSColor::secondaryLabelColor();
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setAlignment(NSTextAlignment::Center);
        let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName] };
        let values: [&AnyObject; 3] = [&font, &color, &paragraph];
        let (mut keys, mut values) = (keys.to_vec(), values.to_vec());
        if let Some(link) = link {
            keys.push(unsafe { NSLinkAttributeName });
            values.push(link);
        }
        let attributes = NSDictionary::from_slices(&keys, &values);
        unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &NSString::from_str(part),
                Some(&attributes),
            )
        }
    }
}

mod own {
    use std::path::PathBuf;

    use gpui::{App, PromptButton, PromptLevel};

    use super::{SITE, VERSION};
    use crate::strings::{Key, fill, tr};
    use crate::windows;

    /// A question over the active window, as the systems tell about a program; its second button
    /// opens the licenses of the components in a tab.
    pub fn show(licenses: Option<PathBuf>, cx: &mut App) {
        let Some(window) = cx.active_window() else { return };
        let detail = format!(
            "{}\n{}\n\n© 2026 Iaroslav Vorobev\n{}  ·  {}",
            fill(Key::AboutVersion, &[("version", VERSION)]),
            tr(Key::AboutSummary),
            tr(Key::AboutLicense),
            SITE.trim_start_matches("https://"),
        );
        let mut buttons = vec![PromptButton::ok(tr(Key::AboutOk))];
        if licenses.is_some() {
            buttons.push(PromptButton::new(tr(Key::AboutThirdParty)));
        }
        let Ok(answer) =
            window.update(cx, |_, window, cx| window.prompt(PromptLevel::Info, "MigPad", Some(&detail), &buttons, cx))
        else {
            return;
        };
        cx.spawn(async move |cx| {
            if let (Ok(1), Some(licenses)) = (answer.await, licenses) {
                cx.update(|cx| {
                    if let Some(window) = windows::last_active(cx) {
                        let _ =
                            window.update(cx, |workspace, window, cx| workspace.open_paths(&[licenses], window, cx));
                    }
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_licenses_are_found_where_each_package_puts_them() {
        let found = |exe: &str, file: &str| licenses_for(Path::new(exe), |path| path == Path::new(file));
        let bundle = "/Applications/MigPad.app/Contents/MacOS/../Resources/THIRD-PARTY-LICENSES.txt";
        assert_eq!(found("/Applications/MigPad.app/Contents/MacOS/migpad", bundle), Some(PathBuf::from(bundle)));
        let windows = "/Programs/MigPad/THIRD-PARTY-LICENSES.txt";
        assert_eq!(found("/Programs/MigPad/migpad.exe", windows), Some(PathBuf::from(windows)));
        let linux = "/usr/bin/../share/doc/migpad/THIRD-PARTY-LICENSES.txt";
        assert_eq!(found("/usr/bin/migpad", linux), Some(PathBuf::from(linux)));
        assert_eq!(found("/work/target/debug/migpad", linux), None);
    }

    #[test]
    fn a_tab_has_no_verbatim_path_of_windows() {
        let plain = without_verbatim(PathBuf::from(r"\\?\C:\Program Files\MigPad\THIRD-PARTY-LICENSES.txt"));
        assert_eq!(plain, PathBuf::from(r"C:\Program Files\MigPad\THIRD-PARTY-LICENSES.txt"));
        let share = PathBuf::from(r"\\?\UNC\server\share\x.txt");
        assert_eq!(without_verbatim(share.clone()), share);
        assert_eq!(without_verbatim(PathBuf::from("/usr/share/doc/x")), PathBuf::from("/usr/share/doc/x"));
    }
}
