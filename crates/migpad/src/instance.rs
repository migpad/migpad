//! One MigPad for a folder of data: the copy that takes the folder listens on a local channel — a
//! Unix socket, or a named pipe on Windows; not the network — and the next ones give it their files
//! and are done. On macOS the files of Finder and the Dock come by an event of the system instead,
//! to the same place: the inbox the main thread takes requests from.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use gpui::App;

use crate::cli::FileArg;
use crate::windows;

/// What a next start asks of the first copy: to open these files, or just to come forward.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub files: Vec<FileArg>,
}

/// The start of a request on the channel, which tells it from anything else.
const MAGIC: &[u8; 8] = b"MIGPAD1\n";
/// A request is not longer than this.
const MAX_REQUEST: u32 = 1 << 20;
/// How long a next start waits for the first copy to take its request.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Requests for the main thread, from the thread of the channel or from the events of the system:
/// the main thread waits for them without holding a thread of GPUI.
pub struct Inbox {
    state: Mutex<(VecDeque<Request>, Option<Waker>)>,
}

impl Inbox {
    pub fn new() -> Arc<Inbox> {
        Arc::new(Inbox { state: Mutex::new((VecDeque::new(), None)) })
    }

    pub fn push(&self, request: Request) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.0.push_back(request);
        if let Some(waker) = state.1.take() {
            waker.wake();
        }
    }

    /// The requests that came so far.
    pub fn take_all(&self) -> Vec<Request> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.0.drain(..).collect()
    }

    /// The next request, once there is one.
    pub async fn next(&self) -> Request {
        std::future::poll_fn(|cx| {
            let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.0.pop_front() {
                Some(request) => Poll::Ready(request),
                None => {
                    state.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }
}

/// The bytes of `request` on the channel: the magic, the length of the rest, and each file — its
/// path, line and column, 0 for none.
fn encode(request: &Request) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(request.files.len() as u32).to_le_bytes());
    for file in &request.files {
        let path = path_bytes(&file.path);
        body.extend_from_slice(&(path.len() as u32).to_le_bytes());
        body.extend_from_slice(&path);
        for number in [file.line, file.column] {
            body.extend_from_slice(&(number.unwrap_or(0) as u32).to_le_bytes());
        }
    }
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The request in `body`, the bytes after the magic and the length; `None` for what is not one.
fn decode(body: &[u8]) -> Option<Request> {
    let mut rest = body;
    let u32_at = |rest: &mut &[u8]| -> Option<u32> {
        let (bytes, tail) = rest.split_first_chunk::<4>()?;
        *rest = tail;
        Some(u32::from_le_bytes(*bytes))
    };
    let count = u32_at(&mut rest)?;
    let mut files = Vec::new();
    for _ in 0..count {
        let len = u32_at(&mut rest)? as usize;
        let (path, tail) = (rest.get(..len)?, rest.get(len..)?);
        rest = tail;
        let path = path_from(path)?;
        let place = |n: u32| (n > 0).then_some(n as usize);
        let (line, column) = (place(u32_at(&mut rest)?), place(u32_at(&mut rest)?));
        files.push(FileArg { path, line, column });
    }
    rest.is_empty().then_some(Request { files })
}

/// A path as the system has it: bytes on Unix, UTF-16 on Windows.
#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().flat_map(u16::to_le_bytes).collect()
}

#[cfg(windows)]
fn path_from(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let (units, rest) = bytes.as_chunks::<2>();
    let units: Vec<u16> = units.iter().map(|unit| u16::from_le_bytes(*unit)).collect();
    rest.is_empty().then(|| PathBuf::from(std::ffi::OsString::from_wide(&units)))
}

/// A hash of `bytes` that stays the same from one run to the next: FNV-1a.
fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3))
}

/// The longest path of a Unix socket that works everywhere: the field has 104 bytes on macOS.
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 100;

/// Where the socket of the copy with the folder of data `data` is: in it, or — when that path is
/// too long for a socket — under a hash of it in a folder of the user's own: the runtime folder of
/// Linux, the temporary folder of macOS, which is the user's there.
#[cfg(unix)]
pub fn socket_path(data: &Path) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_dir());
    socket_path_in(data, &runtime.unwrap_or_else(std::env::temp_dir))
}

#[cfg(unix)]
fn socket_path_in(data: &Path, temp: &Path) -> PathBuf {
    let path = data.join("migpad.sock");
    if path.as_os_str().len() <= MAX_SOCKET_PATH {
        return path;
    }
    temp.join(format!("migpad-{:016x}.sock", stable_hash(&path_bytes(data))))
}

/// The name of the pipe of the copy with the folder of data `data`.
#[cfg(windows)]
fn pipe_name(data: &Path) -> String {
    format!(r"\\.\pipe\migpad-{:016x}", stable_hash(&path_bytes(data)))
}

/// Gives `request` to the copy that runs with the folder of data `data`: done once it has it. A
/// copy that does not answer in [`TIMEOUT`] did not take it — a pipe of Windows has no timeout of
/// its own, so the exchange runs on a thread of its own, left behind if it hangs.
pub fn send(data: &Path, request: &Request) -> io::Result<()> {
    let (data, bytes) = (data.to_path_buf(), encode(request));
    let (done, outcome) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("migpad-send".into()).spawn(move || {
        let exchange = || -> io::Result<()> {
            let mut channel = connect(&data)?;
            channel.write_all(&bytes)?;
            channel.flush()?;
            let mut answer = [0u8; 1];
            channel.read_exact(&mut answer)?;
            if answer[0] == 1 { Ok(()) } else { Err(io::Error::other("the request was not taken")) }
        };
        let _ = done.send(exchange());
    })?;
    outcome.recv_timeout(TIMEOUT).unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut)))
}

#[cfg(unix)]
fn connect(data: &Path) -> io::Result<std::os::unix::net::UnixStream> {
    use std::os::unix::fs::MetadataExt;
    let path = socket_path(data);
    // Only a socket of the owner of the folder: one another user left in a shared folder is not
    // given the files.
    if std::fs::metadata(&path)?.uid() != std::fs::metadata(data)?.uid() {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    Ok(stream)
}

#[cfg(windows)]
fn connect(data: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(pipe_name(data))
}

/// Listens on the channel of the folder of data `data`, which this copy holds, and puts the
/// requests of next starts into `inbox`.
pub fn listen(data: &Path, inbox: Arc<Inbox>) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let path = socket_path(data);
        // A socket left by a copy that crashed: this one holds the folder now.
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        // Only the user may give files to their MigPad, the socket in a shared folder too.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        std::thread::Builder::new().name("migpad-channel".into()).spawn(move || {
            // Each start on a thread of its own: a slow one does not hold the next.
            for stream in listener.incoming().flatten() {
                let inbox = inbox.clone();
                let _ = std::thread::Builder::new().name("migpad-request".into()).spawn(move || {
                    let _ = stream.set_read_timeout(Some(TIMEOUT));
                    serve_one(stream, &inbox);
                });
            }
        })?;
    }
    #[cfg(windows)]
    {
        let name: Vec<u16> = pipe_name(data).encode_utf16().chain([0]).collect();
        // The first instance of the pipe is there before this returns: a next start finds it.
        let first = windows_pipe::create(&name)?;
        std::thread::Builder::new().name("migpad-channel".into()).spawn(move || {
            let mut next = Some(first);
            while let Some(pipe) = next.take().or_else(|| windows_pipe::create(&name).ok()) {
                if windows_pipe::wait(&pipe) {
                    // Each start on a thread of its own: a slow one does not hold the next.
                    let inbox = inbox.clone();
                    let _ = std::thread::Builder::new()
                        .name("migpad-request".into())
                        .spawn(move || serve_one(pipe, &inbox));
                }
            }
        })?;
    }
    Ok(())
}

/// Reads a request from `channel`, answers that it is taken, and puts it into `inbox`.
fn serve_one(mut channel: impl Read + Write, inbox: &Inbox) {
    let mut head = [0u8; 12];
    if channel.read_exact(&mut head).is_err() || &head[..8] != MAGIC {
        return;
    }
    let len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if len > MAX_REQUEST {
        return;
    }
    let mut body = vec![0u8; len as usize];
    if channel.read_exact(&mut body).is_err() {
        return;
    }
    let Some(request) = decode(&body) else { return };
    // Taken before the answer: the next start is done once it has it.
    inbox.push(request);
    let _ = channel.write_all(&[1]).and_then(|()| channel.flush());
}

#[cfg(windows)]
mod windows_pipe {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    /// An instance of the pipe `name`, NUL-terminated UTF-16, for the next start to connect to.
    pub fn create(name: &[u16]) -> io::Result<File> {
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: `name` is a NUL-terminated wide string; no security attributes: the default ones
        // of the user.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                mode,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle is a pipe just made, owned by the file from now on, which closes it.
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }

    /// Waits until a next start connects to `pipe`; `false` if it came and went.
    pub fn wait(pipe: &File) -> bool {
        let handle = pipe.as_raw_handle();
        // SAFETY: a blocking wait on a pipe this thread owns, without an OVERLAPPED.
        unsafe { ConnectNamedPipe(handle as _, std::ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED }
    }
}

/// Whether this start leaves the terminal at once, the program going on in a process of its own:
/// `migpad notes.txt` does not wait for MigPad to quit. Only from a terminal, only on macOS and
/// Linux — a release build on Windows does not hold its console — and not in a debug build, whose
/// messages are for the terminal. Returns whether this process is done.
pub fn leave_terminal() -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        if cfg!(debug_assertions) || std::env::var_os("MIGPAD_DETACHED").is_some() {
            return false;
        }
        if !(io::stdin().is_terminal() || io::stdout().is_terminal() || io::stderr().is_terminal()) {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        // A group of its own: keys and the end of the terminal do not reach it.
        Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env("MIGPAD_DETACHED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .is_ok()
    }
    #[cfg(not(unix))]
    false
}

/// The files of the URLs that macOS gives the program: from Finder, the Dock, `open`.
pub fn files_of_urls(urls: &[String]) -> Vec<FileArg> {
    urls.iter().filter_map(|url| file_of_url(url)).map(FileArg::new).collect()
}

/// The file of a `file://` URL, its escapes undone.
fn file_of_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hex = [chars.next()?, chars.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    #[cfg(unix)]
    return path_from(&bytes);
    #[cfg(not(unix))]
    return String::from_utf8(bytes).ok().map(PathBuf::from);
}

/// Opens what the next starts and the system give, as it comes.
pub fn serve(inbox: Arc<Inbox>, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let request = inbox.next().await;
            cx.update(|cx| open(request, cx));
        }
    })
    .detach();
}

/// Opens the files of `request` in the window used last, at their places, and brings MigPad
/// forward; without files, it just comes forward. Without a window — on macOS — in a new one.
fn open(request: Request, cx: &mut App) {
    let paths: Vec<PathBuf> = request.files.iter().map(|file| file.path.clone()).collect();
    match windows::last_active(cx) {
        Some(window) => {
            let _ = window.update(cx, |workspace, window, cx| {
                workspace.open_paths(&paths, window, cx);
                window.activate_window();
            });
        }
        None => {
            let _ = windows::open_window(&paths, cx).or_else(|| windows::open_window(&[], cx));
        }
    }
    go_to_places(&request.files, cx);
    cx.activate(true);
}

/// Puts the caret at the places of the files given with one, in the windows where they are open.
pub fn go_to_places(files: &[FileArg], cx: &mut App) {
    for file in files {
        let Some(line) = file.line else { continue };
        let column = file.column.map_or(0, |column| column - 1);
        if let Some((window, index)) = windows::find_open(&file.path, cx) {
            let _ =
                window.update(cx, |workspace, window, cx| workspace.go_to_place(index, line - 1, column, window, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, line: Option<usize>, column: Option<usize>) -> FileArg {
        FileArg { path: PathBuf::from(path), line, column }
    }

    #[test]
    fn requests_read_back_as_they_were_written() {
        let request =
            Request { files: vec![file("/notes/план.txt", Some(120), Some(15)), file("/a b.txt", None, None)] };
        let bytes = encode(&request);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(decode(&bytes[12..]), Some(request));
        assert_eq!(decode(&encode(&Request::default())[12..]), Some(Request::default()));
        // Cut short, or with bytes after it: not a request.
        assert_eq!(decode(&bytes[12..bytes.len() - 1]), None);
        assert_eq!(decode(&[bytes[12..].to_vec(), vec![0]].concat()), None);
        assert_eq!(decode(&[9, 0, 0, 0]), None);
    }

    #[test]
    fn the_first_copy_takes_what_the_next_one_gives() {
        let dir = std::env::temp_dir().join(format!("migpad-instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inbox = Inbox::new();
        listen(&dir, inbox.clone()).unwrap();
        let request = Request { files: vec![file("/notes/a.txt", Some(3), None)] };
        send(&dir, &request).unwrap();
        // Taken once answered.
        assert_eq!(inbox.take_all(), [request]);
        let _ = std::fs::remove_dir_all(&dir);
        // No copy runs with another folder.
        assert!(send(&dir.join("none"), &Request::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_too_deep_goes_to_the_temporary_folder() {
        let temp = Path::new("/tmp");
        assert_eq!(socket_path_in(Path::new("/Users/me/.migpad"), temp), Path::new("/Users/me/.migpad/migpad.sock"));
        let deep = PathBuf::from(format!("/Users/me/{}/.migpad", "very-long-folder-name/".repeat(5)));
        let path = socket_path_in(&deep, temp);
        assert!(path.starts_with(temp) && path.to_string_lossy().ends_with(".sock"), "{path:?}");
        assert_eq!(path, socket_path_in(&deep, temp), "the same name each time");
    }

    #[test]
    fn urls_of_finder_are_files() {
        let urls = ["file:///Users/me/%D0%BF%D0%BB%D0%B0%D0%BD%20A.txt".to_owned(), "https://x".to_owned()];
        assert_eq!(files_of_urls(&urls), [FileArg::new(PathBuf::from("/Users/me/план A.txt"))]);
        assert_eq!(file_of_url("file://localhost/tmp/a.txt"), Some(PathBuf::from("/tmp/a.txt")));
        assert_eq!(file_of_url("file:///bad%2"), None);
    }
}
