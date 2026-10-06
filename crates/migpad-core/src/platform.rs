//! What differs between the operating systems in handling files: identity, hard links, and
//! replacing a file while keeping its properties.

#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(unix)]
mod unix {
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, fchown};
    use std::path::Path;

    /// The device and inode of a file.
    pub(crate) fn file_id(file: &File) -> Option<(u64, u64)> {
        let metadata = file.metadata().ok()?;
        Some((metadata.dev(), metadata.ino()))
    }

    pub(crate) fn link_count(file: &File) -> io::Result<u64> {
        Ok(file.metadata()?.nlink())
    }

    /// Gives `to` the permissions, owner, ACL and extended attributes of `from`, as far as the
    /// system allows: only root can keep the owner, and only members the group.
    pub(crate) fn copy_metadata(from: &File, to: &File) -> io::Result<()> {
        let metadata = from.metadata()?;
        to.set_permissions(metadata.permissions())?;
        if fchown(to, Some(metadata.uid()), Some(metadata.gid())).is_err() {
            let _ = fchown(to, None, Some(metadata.gid()));
        }
        copy_attributes(from, to);
        Ok(())
    }

    /// ACL and extended attributes: Finder tags and comments, the text encoding hint and the like.
    /// Not `COPYFILE_METADATA`: its `COPYFILE_STAT` takes the times of the old file as well, and
    /// the saved file would look unchanged — to `make`, to backups.
    #[cfg(target_os = "macos")]
    fn copy_attributes(from: &File, to: &File) {
        let flags = libc::COPYFILE_ACL | libc::COPYFILE_XATTR;
        // SAFETY: both descriptors are open for the duration of the call; no state is passed.
        unsafe {
            libc::fcopyfile(from.as_raw_fd(), to.as_raw_fd(), std::ptr::null_mut(), flags);
        }
    }

    /// Extended attributes, POSIX ACLs and SELinux labels among them. Attributes the process may
    /// not set are skipped.
    #[cfg(not(target_os = "macos"))]
    fn copy_attributes(from: &File, to: &File) {
        let (from, to) = (from.as_raw_fd(), to.as_raw_fd());
        let Some(names) = read_sized(|buf, len| unsafe {
            // SAFETY: `buf` has room for `len` bytes, or is null with `len` 0 to ask for the size.
            libc::flistxattr(from, buf.cast(), len)
        }) else {
            return;
        };
        for name in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
            let Ok(name) = std::ffi::CString::new(name) else { continue };
            let value = read_sized(|buf, len| unsafe {
                // SAFETY: as above; `name` is a NUL-terminated string.
                libc::fgetxattr(from, name.as_ptr(), buf.cast(), len)
            });
            if let Some(value) = value {
                // SAFETY: `value` holds `value.len()` bytes; `name` is NUL-terminated.
                unsafe { libc::fsetxattr(to, name.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
            }
        }
    }

    /// Calls `read` first for the size, then with a buffer of that size, again if it grew.
    #[cfg(not(target_os = "macos"))]
    fn read_sized(read: impl Fn(*mut u8, usize) -> libc::ssize_t) -> Option<Vec<u8>> {
        loop {
            let size = usize::try_from(read(std::ptr::null_mut(), 0)).ok()?;
            let mut buf = vec![0u8; size];
            match usize::try_from(read(buf.as_mut_ptr(), buf.len())) {
                Ok(len) => {
                    buf.truncate(len);
                    return Some(buf);
                }
                Err(_) if io::Error::last_os_error().raw_os_error() == Some(libc::ERANGE) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Puts `replacement` in place of `target`, if there is one, and makes the change durable.
    pub(crate) fn replace(target: &Path, replacement: &Path) -> io::Result<()> {
        std::fs::rename(replacement, target)?;
        // The new directory entry must survive a power loss too; some file systems refuse.
        if let Some(dir) = target.parent()
            && let Ok(dir) = File::open(dir)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle, REPLACEFILE_IGNORE_ACL_ERRORS,
        REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
    };

    fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        // SAFETY: the structure is plain data; the handle is open for the duration of the call.
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info)
        }
    }

    /// The volume serial number and the file index.
    pub(crate) fn file_id(file: &File) -> Option<(u64, u64)> {
        let info = information(file).ok()?;
        Some((info.dwVolumeSerialNumber.into(), u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow)))
    }

    pub(crate) fn link_count(file: &File) -> io::Result<u64> {
        Ok(information(file)?.nNumberOfLinks.into())
    }

    /// Nothing to do: [`replace`] keeps the properties of the target.
    pub(crate) fn copy_metadata(_from: &File, _to: &File) -> io::Result<()> {
        Ok(())
    }

    /// Puts `replacement` in place of `target`, keeping the attributes, ACL, alternate data
    /// streams and creation time of `target`; a plain rename if there is no `target` yet.
    pub(crate) fn replace(target: &Path, replacement: &Path) -> io::Result<()> {
        if !target.exists() {
            return std::fs::rename(replacement, target);
        }
        let wide = |path: &Path| path.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
        let (target, replacement) = (wide(target), wide(replacement));
        let flags = REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_IGNORE_ACL_ERRORS;
        // SAFETY: both names are NUL-terminated and live through the call; there is no backup.
        let done = unsafe {
            ReplaceFileW(
                target.as_ptr(),
                replacement.as_ptr(),
                std::ptr::null(),
                flags,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if done == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}
