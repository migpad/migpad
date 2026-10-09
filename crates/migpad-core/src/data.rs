//! Where MigPad keeps its data: the folder `.migpad` in the home folder, or — in portable mode — a
//! folder `.migpad` next to the program, on macOS next to `MigPad.app`. Settings are there, and the
//! state the program keeps for itself: journals of documents, the session, recent files.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// The name of the folder of the data, at home or next to the program.
pub const FOLDER: &str = ".migpad";

/// The folder of the data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
    portable: bool,
}

impl DataDir {
    /// The folder of the data of this program: next to it if there is one there, at home
    /// otherwise; `None` without a home folder.
    pub fn find() -> Option<DataDir> {
        Self::find_for(program().as_deref(), std::env::home_dir().as_deref())
    }

    /// The folder of the data for the program at `exe` and the home folder `home`.
    pub fn find_for(exe: Option<&Path>, home: Option<&Path>) -> Option<DataDir> {
        let beside = exe.and_then(program_folder).map(|folder| folder.join(FOLDER));
        match beside {
            Some(beside) if beside.is_dir() => Some(DataDir { root: beside, portable: true }),
            _ => Some(DataDir { root: home?.join(FOLDER), portable: false }),
        }
    }

    /// The data in `root`, wherever that is.
    pub fn at(root: PathBuf) -> DataDir {
        DataDir { root, portable: false }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the data is next to the program rather than at home.
    pub fn is_portable(&self) -> bool {
        self.portable
    }

    /// What the program keeps for itself, apart from the settings, which one may keep with other
    /// dotfiles.
    pub fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    /// The journals of the open documents.
    pub fn journals(&self) -> PathBuf {
        self.state().join("journal")
    }

    /// Takes the folder for this running copy of the program: `None` if another copy has it. The
    /// folder is held while the file returned is open — until the program ends, a crash included.
    pub fn lock(&self) -> io::Result<Option<File>> {
        fs::create_dir_all(&self.root)?;
        let file = OpenOptions::new().create(true).truncate(false).write(true).open(self.root.join("migpad.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

/// The program as one sees it: the file that was started, links followed — the command `migpad`
/// is the program in its bundle — and for an AppImage the AppImage file rather than the program
/// in the folder it mounts while it runs.
pub fn program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = fs::canonicalize(&exe).unwrap_or(exe);
    Some(appimage_of(&exe, std::env::var_os("APPDIR"), std::env::var_os("APPIMAGE")).unwrap_or(exe))
}

/// The AppImage file that `exe` runs from. Its runtime mounts it at `APPDIR` and names the file in
/// `APPIMAGE`, for the programs started from it too — which are not in `APPDIR`.
fn appimage_of(exe: &Path, appdir: Option<OsString>, appimage: Option<OsString>) -> Option<PathBuf> {
    let appdir = PathBuf::from(appdir.filter(|dir| !dir.is_empty())?);
    let appdir = fs::canonicalize(&appdir).unwrap_or(appdir);
    let appimage = appimage.filter(|file| !file.is_empty())?;
    exe.starts_with(&appdir).then(|| PathBuf::from(appimage))
}

/// The folder the program is in as one sees it: for a program inside an application bundle of
/// macOS, `…/MigPad.app/Contents/MacOS/migpad`, the folder of the bundle.
fn program_folder(exe: &Path) -> Option<&Path> {
    let folder = exe.parent()?;
    if folder.ends_with("Contents/MacOS")
        && let Some(bundle) = folder.parent().and_then(Path::parent)
        && bundle.extension().is_some_and(|extension| extension == "app")
    {
        return bundle.parent();
    }
    Some(folder)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_data_is_at_home_unless_the_program_has_its_own() {
        let dir = TempDir::new("data-home");
        let (programs, home) = (dir.0.join("programs"), dir.0.join("home"));
        fs::create_dir_all(&programs).unwrap();
        let exe = programs.join("migpad");
        let at_home = DataDir::find_for(Some(&exe), Some(&home)).unwrap();
        assert_eq!((at_home.root(), at_home.is_portable()), (home.join(".migpad").as_path(), false));
        assert_eq!(at_home.journals(), home.join(".migpad").join("state").join("journal"));
        // A file named so does not make the program portable.
        fs::write(programs.join(".migpad"), b"").unwrap();
        assert!(!DataDir::find_for(Some(&exe), Some(&home)).unwrap().is_portable());
        assert_eq!(DataDir::find_for(Some(&exe), None), None);
        assert_eq!(DataDir::find_for(None, Some(&home)), Some(at_home));
    }

    #[test]
    fn a_folder_next_to_the_program_makes_it_portable() {
        let dir = TempDir::new("data-portable");
        let programs = dir.0.join("programs");
        fs::create_dir_all(programs.join(".migpad")).unwrap();
        let data = DataDir::find_for(Some(&programs.join("migpad.exe")), Some(&dir.0.join("home"))).unwrap();
        assert_eq!((data.root(), data.is_portable()), (programs.join(".migpad").as_path(), true));
        // Without a home folder too.
        assert_eq!(DataDir::find_for(Some(&programs.join("migpad.exe")), None), Some(data));
    }

    #[test]
    fn an_appimage_is_the_program_its_folder_is_next_to() {
        let mount = Some(OsString::from("/tmp/.mount_MigPad"));
        let file = Some(OsString::from("/home/me/Apps/MigPad.AppImage"));
        let inside = Path::new("/tmp/.mount_MigPad/usr/bin/migpad");
        assert_eq!(
            appimage_of(inside, mount.clone(), file.clone()),
            Some(PathBuf::from("/home/me/Apps/MigPad.AppImage"))
        );
        // A program started from an AppImage has its variables, but runs from elsewhere.
        assert_eq!(appimage_of(Path::new("/usr/bin/migpad"), mount.clone(), file.clone()), None);
        assert_eq!(appimage_of(inside, None, file), None);
        assert_eq!(appimage_of(inside, mount, Some(OsString::new())), None);
        let data = DataDir::find_for(Some(Path::new("/home/me/Apps/MigPad.AppImage")), Some(Path::new("/home/me")));
        assert_eq!(data.unwrap().root(), Path::new("/home/me/.migpad"), "no folder next to it");
    }

    #[test]
    fn one_running_copy_holds_the_folder() {
        let dir = TempDir::new("data-lock");
        let data = DataDir::at(dir.0.join(".migpad"));
        let first = data.lock().unwrap();
        assert!(first.is_some());
        assert!(data.lock().unwrap().is_none(), "another copy has it");
        drop(first);
        assert!(data.lock().unwrap().is_some(), "free once the first is done");
    }

    #[test]
    fn an_application_bundle_has_its_folder_next_to_it() {
        let dir = TempDir::new("data-bundle");
        let applications = dir.0.join("Applications");
        let exe = applications.join("MigPad.app").join("Contents").join("MacOS").join("migpad");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let home = dir.0.join("home");
        assert!(!DataDir::find_for(Some(&exe), Some(&home)).unwrap().is_portable());
        fs::create_dir_all(applications.join(".migpad")).unwrap();
        let data = DataDir::find_for(Some(&exe), Some(&home)).unwrap();
        assert_eq!((data.root(), data.is_portable()), (applications.join(".migpad").as_path(), true));
        // Not a bundle: a folder of that name that is not an application.
        let plain = dir.0.join("Tools").join("Contents").join("MacOS").join("migpad");
        assert_eq!(program_folder(&plain), plain.parent());
    }
}
