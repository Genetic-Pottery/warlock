use std::collections::BTreeMap;
use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Omission, Problem, one_file};

use crate::agent;
fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
    fs::write(&path, contents).expect("writes a file");
    path
}

fn own_files(dir: &Path) -> Result<BTreeMap<String, u64>, crate::walk::Error> {
    Ok(super::Snapshot::take(dir)?
        .files
        .iter()
        .map(|(name, measured)| (name.clone(), measured.size))
        .collect())
}

fn child_documents(dir: &Path) -> Result<Vec<agent::ChildDocument>, crate::walk::Error> {
    Ok(super::Snapshot::take(dir)?
        .request
        .child_documents()
        .to_vec())
}

fn request_for(dir: &Path, name: &str) -> agent::Request {
    let (request, _, problem) = super::one_file("summarise", dir, name).expect("reads");
    assert!(problem.is_none(), "{problem:?}");
    request
}

#[test]
fn a_source_file_reaches_the_pass_with_its_test_bodies_elided() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "scope.rs",
        "\
pub fn covering() -> u32 {
    7
}

#[cfg(test)]
mod tests {
    #[test]
    fn refuses_when_scope_closed() {
        let a = super::covering();
        let b = super::covering();
        assert_eq!(a, 7);
        assert_eq!(b, 7);
        assert_eq!(a, b);
    }
}
",
    );
    let request = request_for(dir.path(), "scope.rs");
    let file = file(&request, "scope.rs");

    let kept = file.kept().expect("a Rust test module is elided");
    assert!(
        kept.contains("pub fn covering()"),
        "the code a reader needs stays: {kept}"
    );
    assert!(
        kept.contains("fn refuses_when_scope_closed()"),
        "and so does the test's name, which is a sentence about behaviour: {kept}"
    );
    assert!(
        !kept.contains("assert_eq!(a, b)"),
        "the body is what is given up: {kept}"
    );
    assert_eq!(
        file.bytes(),
        None,
        "what survives elision is not the file's bytes and never answers as them"
    );
}

#[test]
fn an_elided_file_still_reports_its_size_on_disk() {
    // The distinction the whole ladder rests on: how big a file is and how
    // much of it was sent are two facts, and a document that says the
    // second is a document that understates the directory.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let source = "\
pub fn work() {}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let one = 1;
        let two = 2;
        assert_eq!(one + two, 3);
    }
}
";
    write(dir.path(), "work.rs", source);
    let request = request_for(dir.path(), "work.rs");
    let file = file(&request, "work.rs");

    assert_eq!(
        file.size(),
        source.len() as u64,
        "the size is the file's, not the elision's"
    );
    assert!(
        file.kept().expect("elided").len() < source.len(),
        "and the elision really is smaller"
    );
}

#[test]
fn a_file_in_a_language_with_no_row_is_sent_exactly_as_it_is() {
    // The property that lets the table grow one language at a time: an
    // extension nobody has described is never guessed at.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let source = "section .text\nglobal _start\n_start:\n    mov eax, 1\n";
    write(dir.path(), "boot.asm", source);
    let request = request_for(dir.path(), "boot.asm");
    let file = file(&request, "boot.asm");

    assert_eq!(
        file.bytes(),
        Some(source.as_bytes()),
        "an unknown language is sent whole and untouched"
    );
    assert_eq!(file.kept(), None, "and nothing claims to have elided it");
}

fn file_paths(request: &agent::Request) -> Vec<&str> {
    request.files().iter().map(agent::File::path).collect()
}

fn file<'a>(request: &'a agent::Request, path: &str) -> &'a agent::File {
    request
        .files()
        .iter()
        .find(|file| file.path() == path)
        .unwrap_or_else(|| panic!("`{path}` is in the request: {:?}", file_paths(request)))
}

#[test]
fn a_directory_sends_its_own_files_and_its_children_summarise_themselves() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "Cargo.toml", "[package]\n");
    write(dir.path(), "build.rs", "fn main() {}\n");
    write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");
    write(dir.path(), "src/lib.rs", "//! Core engine.\n");
    write(dir.path(), "src/inner/lib.rs", "//! Deeper still.\n");
    write(dir.path(), "src/inner/WARLOCK.md", "# inner\n");
    write(dir.path(), "tests/it.rs", "#[test] fn works() {}\n");

    let files = own_files(dir.path()).expect("walks");

    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        ["Cargo.toml", "build.rs"],
        "only the directory's own files, sorted; nothing from below it"
    );
    let request = request_for(dir.path(), "build.rs");
    assert_eq!(
        file(&request, "build.rs").bytes(),
        Some(&b"fn main() {}\n"[..]),
        "and a file asked about carries its bytes",
    );

    let children = child_documents(dir.path()).expect("walks");
    assert_eq!(
        children
            .iter()
            .map(|child| (child.directory(), child.text()))
            .collect::<Vec<_>>(),
        [("src", "# src\n\nThe code.\n")],
        "a child with a document contributes it; `tests/` has none and \
             contributes no entry, which is not an error",
    );
    assert!(
        !format!("{children:?}").contains("Deeper still"),
        "a grandchild's document is already covered by its parent's, and \
             its source is never read at all",
    );
    assert_eq!(request.prompt(), "summarise");
}

#[test]
fn two_gathers_of_an_unchanged_directory_are_the_same_value() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Written in an order that is not the sorted one, so a request that
    // simply kept what the filesystem offered would have to be lucky.
    write(dir.path(), "zeta.rs", "//! z\n");
    write(dir.path(), "alpha.rs", "//! a\n");
    write(dir.path(), "zeta/WARLOCK.md", "# zeta\n");
    write(dir.path(), "alpha/WARLOCK.md", "# alpha\n");

    let files = own_files(dir.path()).expect("walks");
    let children = child_documents(dir.path()).expect("walks");

    assert_eq!(files.keys().collect::<Vec<_>>(), ["alpha.rs", "zeta.rs"]);
    assert_eq!(
        children
            .iter()
            .map(agent::ChildDocument::directory)
            .collect::<Vec<_>>(),
        ["alpha", "zeta"],
    );
    assert_eq!(
        files,
        own_files(dir.path()).expect("walks"),
        "two walks, one value",
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_is_neither_followed_nor_listed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "lib.rs", "//! Core engine.\n");
    std::os::unix::fs::symlink(dir.path(), dir.path().join("up")).expect("links to itself");
    std::os::unix::fs::symlink(dir.path().join("lib.rs"), dir.path().join("alias.rs"))
        .expect("links to a file");

    let files = own_files(dir.path()).expect("walks");

    assert_eq!(files.keys().collect::<Vec<_>>(), ["lib.rs"]);
    assert!(child_documents(dir.path()).expect("walks").is_empty());
}

#[test]
fn an_empty_directory_is_a_request_with_nothing_in_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    assert!(own_files(dir.path()).expect("walks").is_empty());
    assert!(child_documents(dir.path()).expect("walks").is_empty());
}

#[test]
fn every_problem_says_what_was_left_out_and_why_on_one_line() {
    let problems = [
        Problem {
            path: PathBuf::from("/repo/Cargo.lock"),
            cause: Omission::TooLarge { size: 4_200_000 },
        },
        Problem {
            path: PathBuf::from("/repo/secret.rs"),
            cause: Omission::Unreadable {
                source: std::io::Error::other("permission denied"),
            },
        },
    ];

    for problem in &problems {
        let rendered = problem.to_string();
        assert!(!rendered.contains('\n'), "{rendered}");
        assert!(
            rendered.contains(&problem.path.display().to_string()),
            "a problem names its file: {rendered}",
        );
    }
    assert!(
        problems[0].to_string().contains("4200000"),
        "{}",
        problems[0],
    );
    assert!(
        problems[1].to_string().contains("permission denied"),
        "{}",
        problems[1],
    );
    assert_eq!(
        problems
            .iter()
            .filter(|problem| problem.source().is_some())
            .count(),
        2,
        "every problem's cause is reachable as a source",
    );
    assert!(
        problems[1]
            .source()
            .and_then(std::error::Error::source)
            .is_some(),
        "and an unreadable file's cause names the io error under it",
    );
}

#[test]
fn a_name_only_a_comment_writes_is_not_token_evidence() {
    // The arm a synthesis pass leans on. `Described` is measured here and
    // answers for a directory whose files were never sent, so a comment
    // counting as evidence in this map is a lie witnessing itself in every
    // document too big to send whole.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "balance.rs",
        "//! Every Posting is validated by Decoder::decode().\n\
         \n\
         pub const VAULT_LIMIT: usize = 512;\n\
         pub fn is_settled(open: usize) -> bool {\n\
             open == 0\n\
         }\n",
    );

    let snapshot = super::Snapshot::take(dir.path()).expect("reads the directory");
    let described = &snapshot.described;

    assert!(
        described.written_anywhere("is_settled"),
        "code is evidence for a claim"
    );
    assert!(
        described.written_anywhere("VAULT_LIMIT"),
        "so is a constant the table declares"
    );
    assert!(
        !described.written_anywhere("Decoder"),
        "a comment is not evidence"
    );
    assert!(
        !described.written_anywhere("Posting"),
        "nor is the rest of the same sentence"
    );
}

#[test]
fn a_pass_is_shown_the_code_and_not_the_comments() {
    // The root of the loop this closes, asserted at the one funnel every byte
    // of text passes through on its way to a pass. Cutting comments out of the
    // evidence alone left the pass reading the lie and rewording it until no
    // check could see it; there is nothing to reword if it was never sent.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "balance.rs",
        "//! Every Posting is validated by Decoder::decode().\n\
         \n\
         /* A gorilla reconciles overnight. */\n\
         pub const VAULT_LIMIT: usize = 512;\n\
         pub fn is_settled(open: usize) -> bool {\n\
             open == 0 // counts the open ones\n\
         }\n",
    );
    // No comment form for `.wat`, so it is sent exactly as it is. A language
    // nobody has described is left alone here as everywhere else.
    write(dir.path(), "raw.wat", ";; a griffin wrote this\n(module)\n");

    let shown = |name: &str| {
        let (request, ..) =
            one_file(crate::document::FILE_PROMPT, dir.path(), name).expect("reads the file");
        let file = &request.files()[0];
        file.bytes()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .or_else(|| file.kept().map(str::to_owned))
            .expect("the file is sent")
    };

    let rust = shown("balance.rs");
    for gone in ["Posting", "Decoder", "gorilla", "counts the open ones"] {
        assert!(!rust.contains(gone), "{gone} reached the pass: {rust}");
    }
    for stands in ["VAULT_LIMIT", "is_settled", "open == 0"] {
        assert!(rust.contains(stands), "{stands} was lost: {rust}");
    }
    assert!(
        !rust.contains("\n\n\n"),
        "an emptied line is dropped rather than left blank: {rust:?}"
    );

    let wat = shown("raw.wat");
    assert!(wat.contains("griffin"), "left entirely alone: {wat}");
}
