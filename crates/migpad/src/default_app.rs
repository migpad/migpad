//! MigPad as the program that opens texts on macOS, when one chooses so in the settings window:
//! plain text, logs, Markdown, JSON, XML, CSV, YAML and files of settings. Its bundle offers it
//! for them in Open With anyway, without taking them. Up to macOS 26.3 the system takes the choice
//! at once; from 26.4 it asks to confirm each type.

use block2::DynBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::sel;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSError, NSString, NSURL};

/// The identifier of the bundle, as its `Info.plist` has it.
const BUNDLE: &str = "com.migpad.MigPad";

/// The types MigPad becomes the program of. The last is that of the files of settings — `.ini`,
/// `.conf`, `.toml` and others — that the `Info.plist` of MigPad declares.
const TYPES: [&str; 8] = [
    "public.plain-text",
    "com.apple.log",
    "net.daringfireball.markdown",
    "public.json",
    "public.xml",
    "public.comma-separated-values-text",
    "public.yaml",
    "com.migpad.configuration-text",
];

/// Any role: viewing and editing.
const ALL_ROLES: u32 = 0xffff_ffff;

/// Whether MigPad runs from its bundle, which the system knows as a program: a build outside of
/// one cannot open types.
pub fn available() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some_and(|id| id.to_string() == BUNDLE)
}

/// Whether MigPad is the program of each of the types.
pub fn is_default() -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    TYPES.iter().all(|content| program_of(&workspace, content).as_deref() == Some(BUNDLE))
}

/// The bundle of the program of `content`.
fn program_of(workspace: &NSWorkspace, content: &str) -> Option<String> {
    if workspace.respondsToSelector(sel!(URLForApplicationToOpenContentType:)) {
        let content = content_type(content)?;
        // SAFETY: the method of macOS 12 and later, which it answers, with a type of content.
        let url: Option<Retained<NSURL>> =
            unsafe { msg_send![workspace, URLForApplicationToOpenContentType: &*content] };
        let url = url?;
        NSBundle::bundleWithURL(&url)?.bundleIdentifier().map(|id| id.to_string())
    } else {
        // SAFETY: LaunchServices before macOS 12; it gives the identifier with a reference to
        // release.
        let handler = unsafe { LSCopyDefaultRoleHandlerForContentType(&NSString::from_str(content), ALL_ROLES) };
        // SAFETY: the string it returned, owned now.
        unsafe { Retained::from_raw(handler) }.map(|handler| handler.to_string())
    }
}

/// Asks the system to make MigPad the program of the types; whether it did, `is_default` tells
/// once it has — at once, or after the questions of macOS 26.4.
pub fn make_default() {
    let workspace = NSWorkspace::sharedWorkspace();
    let modern = workspace.respondsToSelector(sel!(setDefaultApplicationAtURL:toOpenContentType:completionHandler:));
    let app = NSBundle::mainBundle().bundleURL();
    for content in TYPES {
        if modern {
            let Some(content) = content_type(content) else { continue };
            let done: Option<&DynBlock<dyn Fn(*mut NSError)>> = None;
            // SAFETY: the method of macOS 12 and later, which it answers: the bundle of MigPad, a
            // type of content, and no handler of the end.
            let _: () = unsafe {
                msg_send![&workspace, setDefaultApplicationAtURL: &*app, toOpenContentType: &*content, completionHandler: done]
            };
        } else {
            // SAFETY: LaunchServices before macOS 12, with two strings.
            unsafe {
                LSSetDefaultRoleHandlerForContentType(
                    &NSString::from_str(content),
                    ALL_ROLES,
                    &NSString::from_str(BUNDLE),
                )
            };
        }
    }
}

/// The `UTType` of an identifier: of macOS 11 and later, which the system has whenever the methods
/// that take it are there.
fn content_type(identifier: &str) -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"UTType")?;
    // SAFETY: the class method of `UTType`, with a string; it gives a type, or nothing.
    unsafe { msg_send![class, typeWithIdentifier: &*NSString::from_str(identifier)] }
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    // A `CFString` of these is an `NSString`.
    fn LSSetDefaultRoleHandlerForContentType(content: &NSString, roles: u32, bundle: &NSString) -> i32;
    fn LSCopyDefaultRoleHandlerForContentType(content: &NSString, roles: u32) -> *mut NSString;
}
