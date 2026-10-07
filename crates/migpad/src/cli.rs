//! The command line: the files to open, each perhaps with a place — `notes.txt:120:15`, as
//! compilers write places. The first copy of MigPad opens them; a next one gives them to it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A file to open, and the line and the column to go to there, both from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileArg {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl FileArg {
    /// A file without a place.
    pub fn new(path: PathBuf) -> Self {
        FileArg { path, line: None, column: None }
    }
}

/// The files of the command line `args` — the arguments after the program — made absolute
/// against `cwd`. Options are passed over: `-AppleLanguages '(en)'`, which sets a user default of
/// macOS, with its value; `-psn_…`, which old macOS gives a program Finder starts, alone. After
/// `--` every argument is a file. `exists` tells whether a path is a file: one whose name itself
/// ends like a place opens as it is.
pub fn files(args: impl IntoIterator<Item = OsString>, cwd: &Path, exists: impl Fn(&Path) -> bool) -> Vec<FileArg> {
    let mut files = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            files.extend(args.map(|arg| file(arg, cwd, &exists)));
            break;
        }
        let bytes = arg.as_encoded_bytes();
        if bytes.starts_with(b"-psn_") {
            continue;
        }
        if bytes.starts_with(b"-") {
            args.next();
        } else {
            files.push(file(arg, cwd, &exists));
        }
    }
    files
}

/// The file of an argument: as it is, if there is such a file; otherwise without a place at its
/// end, `:line` or `:line:column`.
fn file(arg: OsString, cwd: &Path, exists: &impl Fn(&Path) -> bool) -> FileArg {
    let whole = cwd.join(&arg);
    if exists(&whole) {
        return FileArg::new(whole);
    }
    let Some(text) = arg.to_str() else { return FileArg::new(whole) };
    let number = |text: &str| {
        let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
        digits.then(|| text.parse::<usize>().ok()).flatten().filter(|&n| n > 0)
    };
    let Some((rest, last)) = text.rsplit_once(':').and_then(|(rest, last)| Some((rest, number(last)?))) else {
        return FileArg::new(whole);
    };
    // `log:12:3` is line 3 of the file `log:12`, if there is one.
    let split = rest.rsplit_once(':').and_then(|(name, line)| Some((name, number(line)?)));
    let (name, line, column) = match split {
        Some((name, line)) if !exists(&cwd.join(rest)) => (name, line, Some(last)),
        _ => (rest, last, None),
    };
    if name.is_empty() {
        return FileArg::new(whole);
    }
    FileArg { path: cwd.join(name), line: Some(line), column }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], existing: &[&str]) -> Vec<FileArg> {
        let cwd = Path::new("/work");
        let exists = |path: &Path| existing.iter().any(|name| cwd.join(name) == path);
        files(args.iter().map(OsString::from), cwd, exists)
    }

    fn at(path: &str, line: Option<usize>, column: Option<usize>) -> FileArg {
        FileArg { path: PathBuf::from(path), line, column }
    }

    #[test]
    fn files_are_the_arguments_that_are_not_options() {
        assert_eq!(parse(&["notes.txt"], &[]), [at("/work/notes.txt", None, None)]);
        assert_eq!(
            parse(&["-AppleLanguages", "(en)", "заметки.txt", "/tmp/more.txt"], &[]),
            [at("/work/заметки.txt", None, None), at("/tmp/more.txt", None, None)]
        );
        assert_eq!(
            parse(&["a.txt", "--", "-dash.txt"], &[]),
            [at("/work/a.txt", None, None), at("/work/-dash.txt", None, None)]
        );
        assert_eq!(parse(&["-psn_0_12345", "a.txt"], &[]), [at("/work/a.txt", None, None)]);
        assert!(parse(&["-AppleLanguages", "(en)"], &[]).is_empty());
        assert!(parse(&[], &[]).is_empty());
    }

    #[test]
    fn a_place_follows_the_name() {
        assert_eq!(parse(&["main.rs:120"], &[]), [at("/work/main.rs", Some(120), None)]);
        assert_eq!(parse(&["src/main.rs:120:15"], &[]), [at("/work/src/main.rs", Some(120), Some(15))]);
        assert_eq!(parse(&[r"C:\code\main.rs:7"], &[])[0].line, Some(7));
        // Not a place: zero, not digits, nothing before it.
        assert_eq!(parse(&["a.txt:0"], &[]), [at("/work/a.txt:0", None, None)]);
        assert_eq!(parse(&["a.txt:x"], &[]), [at("/work/a.txt:x", None, None)]);
        assert_eq!(parse(&[":12"], &[]), [at("/work/:12", None, None)]);
        // A column without a line is a line.
        assert_eq!(parse(&["a.txt:x:5"], &[]), [at("/work/a.txt:x", Some(5), None)]);
    }

    #[test]
    fn a_file_named_like_a_place_opens_as_it_is() {
        assert_eq!(parse(&["log:12"], &["log:12"]), [at("/work/log:12", None, None)]);
        assert_eq!(parse(&["log:12:3"], &["log:12"]), [at("/work/log:12", Some(3), None)]);
    }
}
