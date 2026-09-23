use std::fs;
use std::path::{Path, PathBuf};

use super::{Unviewable, Viewed, view_file};

fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
    fs::write(&path, contents).expect("writes a file");
    path
}

fn untouched(path: &Path, contents: &[u8]) {
    assert_eq!(
        fs::read(path).expect("the file is still there"),
        contents,
        "`{}` was changed by being looked at",
        path.display(),
    );
}

#[test]
fn a_view_of_an_ordinary_file_is_the_whole_of_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let contents = "# engine\n\nThe core.\n";
    let path = write(dir.path(), "WARLOCK.md", contents);

    let Viewed { text, cut } = view_file(&path).expect("an ordinary file reads");

    assert_eq!(text, contents);
    assert!(!cut, "a file under the cap is not cut");
    untouched(&path, contents.as_bytes());
}

#[test]
fn an_empty_file_is_an_empty_view_rather_than_a_failure() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(dir.path(), "empty.rs", "");

    let Viewed { text, cut } = view_file(&path).expect("an empty file reads");

    assert!(text.is_empty(), "{text:?}");
    assert!(!cut, "there is nothing past nothing");
    untouched(&path, b"");
}

#[test]
fn a_file_that_is_not_there_is_a_read_failure_naming_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("gone.rs");

    let error = view_file(&path).expect_err("nothing to read");

    assert!(matches!(error, Unviewable::Unreadable { .. }), "{error:?}");
    assert_eq!(error.path(), path);
    assert!(
        error.to_string().contains(&path.display().to_string()),
        "{error}",
    );
}

#[cfg(unix)]
#[test]
fn a_file_that_may_not_be_read_is_a_read_failure_naming_it() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("a temporary directory");
    let contents = "# engine\n\nThe core.\n";
    let path = write(dir.path(), "WARLOCK.md", contents);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmods");
    if fs::read(&path).is_ok() {
        // Running as root: no file is unreadable, so there is nothing here
        // to assert against.
        return;
    }

    let error = view_file(&path).expect_err("the permissions say no");

    assert!(matches!(error, Unviewable::Unreadable { .. }), "{error:?}");
    assert_eq!(error.path(), path);
    assert!(
        error.to_string().contains(&path.display().to_string()),
        "{error}",
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmods back");
    untouched(&path, contents.as_bytes());
}

#[test]
fn a_directory_is_a_read_failure_naming_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "src/lib.rs", "//! Core engine.\n");
    let path = dir.path().join("src");

    let error = view_file(&path).expect_err("a directory has no text");

    assert!(
        matches!(error, Unviewable::Unreadable { .. }),
        "a directory is the filesystem saying no, not a file that is not text: {error:?}",
    );
    assert_eq!(error.path(), path);
    assert!(
        error.to_string().contains(&path.display().to_string()),
        "{error}",
    );
    untouched(&path.join("lib.rs"), b"//! Core engine.\n");
}

#[test]
fn a_second_view_reads_the_file_as_it_is_now() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(dir.path(), "WARLOCK.md", "# engine\n");

    let first = view_file(&path).expect("reads").text;
    fs::write(&path, "# engine\n\nRewritten.\n").expect("rewrites the file");
    let second = view_file(&path).expect("reads again").text;

    assert_eq!(first, "# engine\n");
    assert_eq!(
        second, "# engine\n\nRewritten.\n",
        "every view is a read from disk, so nothing is cached to go stale",
    );
}
