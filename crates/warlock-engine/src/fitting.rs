use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::Described;
use crate::hash::length;
use crate::ignores;
use crate::languages;
use crate::pact::{DOCUMENT_FILE, Error, MANIFEST_DIR};
use crate::{agent, to_manifest_path};

const WALK_DEPTH: usize = 2;

pub const PER_FILE_BYTE_CAP: u64 = 1024 * 1024;



fn elided_or_whole(path: &Path, relative: String, size: u64, bytes: Vec<u8>) -> agent::File {
    let Ok(text) = str::from_utf8(&bytes) else {
        return agent::File::present(relative, bytes);
    };
    match languages::elide(path, text) {
        Some(elided) => agent::File::elided(relative, size, elided.text),
        None => agent::File::present(relative, bytes),
    }
}

// The files a directory's document holds a line for: its own, one level deep,
// prose excluded — the same set `gather_request` sends, decided by the same
// walk so the two cannot disagree about what is in a directory.
pub(crate) fn own_files(directory: &Path) -> Result<BTreeMap<String, PathBuf>, Error> {
    Ok(walk(directory)?.files)
}

// The documents of the directories below this one, which a synthesis pass needs
// for the same reason a whole-directory pass does: `## Directories` is written
// from a child's own document and from nothing else.
pub(crate) fn child_documents(directory: &Path) -> Result<Vec<agent::ChildDocument>, Error> {
    let mut found = Vec::new();
    for (child, path) in walk(directory)?.child_documents {
        if let Ok(text) = fs::read_to_string(&path) {
            found.push(agent::ChildDocument::new(child, text));
        }
    }
    Ok(found)
}

// What warlock knows about a directory without sending any of it: the declared
// names of every file the document holds a line for. It is the second witness
// `check` asks when the request carries something other than the files — a
// synthesis pass over assembled lines, where no file's text is in the request
// at all and every real name would otherwise be refused.
pub(crate) fn measured(directory: &Path) -> Result<Described, Error> {
    let mut described = Described::default();
    for (name, path) in own_files(directory)? {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(text) = str::from_utf8(&bytes) else {
            continue;
        };
        let names = languages::declared_names(&path, text);
        if !names.is_empty() {
            described.declared.insert(name, names);
        }
    }
    Ok(described)
}

// One file, reduced the way the same file would be inside a directory's
// request: the per-file cap still applies, `elide` still takes the bodies out,
// and the declared names are still measured here rather than guessed at later.
//
// No budget ladder and no `Problem`s. The ladder exists because a directory is
// a sum that need not fit; one file either fits under `PER_FILE_BYTE_CAP` or is
// sent as a name and a size, and there is nothing to demote it in favour of.
pub(crate) fn one_file(
    prompt: &str,
    directory: &Path,
    name: &str,
) -> Result<(agent::Request, Described, Option<Problem>), Error> {
    let path = directory.join(name);
    let size = fs::metadata(&path)
        .map_err(|source| Error::Walk {
            directory: directory.to_path_buf(),
            source: source.into(),
        })?
        .len();

    // Reported rather than passed over in silence, the same as a file a
    // directory's request had to give up: a line written from a name and a size
    // is a line the caller is owed the reason for.
    let mut problem = None;
    let file = if size > PER_FILE_BYTE_CAP {
        problem = Some(Problem {
            path: path.clone(),
            cause: Omission::TooLarge { size },
        });
        agent::File::omitted(name.to_owned(), size)
    } else {
        match fs::read(&path) {
            Ok(bytes) => elided_or_whole(&path, name.to_owned(), size, bytes),
            Err(source) => {
                problem = Some(Problem {
                    path: path.clone(),
                    cause: Omission::Unreadable { source },
                });
                agent::File::omitted(name.to_owned(), size)
            }
        }
    };

    let mut described = Described::default();
    let text = file.kept().or_else(|| {
        file.bytes()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    });
    if let Some(text) = text {
        let names = languages::declared_names(&path, text);
        if !names.is_empty() {
            described.declared.insert(name.to_owned(), names);
        }
    }

    Ok((
        agent::Request::new(prompt, directory).with_files([file]),
        described,
        problem,
    ))
}





const PROSE_EXTENSIONS: &[&str] = &["md", "markdown", "mdx"];

fn is_prose(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROSE_EXTENSIONS
                .iter()
                .any(|prose| extension.eq_ignore_ascii_case(prose))
        })
}

const CARRY_HASH_CONTEXT: &str = "warlock carry hash v1 2026-09-06";

pub(crate) fn carry_hash(directory: &Path) -> Option<String> {
    let found = walk(directory).ok()?;
    let mut hasher = blake3::Hasher::new_derive_key(CARRY_HASH_CONTEXT);

    // Two sections, each length-prefixed and each announced by its count, so no
    // arrangement of one can be read as the other: a directory holding a file
    // named `x` and one holding a child `x` with a document are different
    // inputs and must be different digests.
    hasher.update(&length(found.files.len()).to_le_bytes());
    for (relative, path) in &found.files {
        hasher.update(&length(relative.len()).to_le_bytes());
        hasher.update(relative.as_bytes());
        let bytes = fs::read(path).ok()?;
        hasher.update(&length(bytes.len()).to_le_bytes());
        hasher.update(&bytes);
    }

    hasher.update(&length(found.child_documents.len()).to_le_bytes());
    for (child, path) in &found.child_documents {
        hasher.update(&length(child.len()).to_le_bytes());
        hasher.update(child.as_bytes());
        let bytes = fs::read(path).ok()?;
        hasher.update(&length(bytes.len()).to_le_bytes());
        hasher.update(&bytes);
    }

    // The third section: the document itself, absent and empty told apart by
    // the marker byte, so a directory with no document cannot digest as one
    // holding a document of nothing.
    match fs::read(directory.join(DOCUMENT_FILE)) {
        Ok(bytes) => {
            hasher.update(&[1]);
            hasher.update(&length(bytes.len()).to_le_bytes());
            hasher.update(&bytes);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            hasher.update(&[0]);
        }
        Err(_) => return None,
    }

    Some(hasher.finalize().to_hex().to_string())
}

struct Found {
    files: BTreeMap<String, PathBuf>,
    child_documents: BTreeMap<String, PathBuf>,
}

fn walk(dir: &Path) -> Result<Found, Error> {
    let walker = WalkBuilder::new(dir)
        // The same three rules as `load` and `hash`, for the same reasons: a
        // symlinked cycle has to terminate, a fixture with a `.gitignore` and
        // no `.git` still has to be ignored properly, and `.warlock/` is
        // Warlock's own bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        .add_custom_ignore_filename(ignores::FILENAME)
        .max_depth(Some(WALK_DEPTH))
        .build();

    let mut found = Found {
        files: BTreeMap::new(),
        child_documents: BTreeMap::new(),
    };
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk {
            directory: dir.to_path_buf(),
            source,
        })?;
        // Rules that could not be read are the failure they are rather than a
        // verdict of "nothing is excluded"; see `pactable_directories`.
        if let Some(source) = entry.error() {
            return Err(Error::Walk {
                directory: dir.to_path_buf(),
                source: source.clone(),
            });
        }
        let depth = entry.depth();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();

        if depth == 1 {
            // Prose is excluded, and that deliberately takes the directory's
            // own `WARLOCK.md` with it: a previous pass's claim about this
            // directory is not evidence about it. See `agent::Request`.
            if !is_prose(&path) {
                found.files.insert(relative(dir, &path)?, path);
            }
        } else if depth == WALK_DEPTH && path.file_name() == Some(OsStr::new(DOCUMENT_FILE)) {
            let Some(child) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            found.child_documents.insert(relative(dir, &child)?, path);
        }
    }
    Ok(found)
}

fn relative(dir: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(dir, path).map_err(|source| Error::Path {
        directory: dir.to_path_buf(),
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

pub(crate) fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}



#[derive(Debug)]
pub struct Problem {
    pub path: PathBuf,
    pub cause: Omission,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Omission {
    TooLarge { size: u64 },
    OverBudget { size: u64 },
    Unreadable { source: std::io::Error },
    NotText { size: u64, source: Utf8Error },
    Unreducible { size: u64 },
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(
                f,
                "{size} bytes is over the {PER_FILE_BYTE_CAP}-byte per-file cap, so it is listed \
                 by name and size"
            ),
            // No number named, unlike the per-file cap above: the request
            // budget is derived from the agent's context window, so no one
            // figure is true of every run and quoting a stale one would be
            // worse than quoting none.
            Self::OverBudget { size } => write!(
                f,
                "the directory is over the request budget, so this file of {size} bytes is \
                 listed by name and size"
            ),
            Self::Unreadable { source } => write!(f, "it could not be read: {source}"),
            Self::NotText { size, source } => write!(
                f,
                "its {size} bytes are not text ({source}), so there is nothing to reduce and it \
                 is listed by name and size"
            ),
            Self::Unreducible { size } => write!(
                f,
                "there is nothing of its {size} bytes to lift — warlock has no reader for this \
                 kind of file — so it is listed by name and size"
            ),
        }
    }
}

impl std::error::Error for Omission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source } => Some(source),
            Self::NotText { source, .. } => Some(source),
            Self::TooLarge { .. } | Self::OverBudget { .. } | Self::Unreducible { .. } => None,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` was left out of the pact request: {}",
            self.path.display(),
            self.cause
        )
    }
}

impl std::error::Error for Problem {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Omission, PER_FILE_BYTE_CAP, Problem};

    use crate::agent;
    use crate::pact::DOCUMENT_FILE;
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
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
        assert_eq!(
            file.summary(),
            None,
            "nor is it prose about the file: it is the file's own lines"
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

    fn carried(request: &agent::Request) -> u64 {
        let files: u64 = request
            .files()
            .iter()
            .map(|file| {
                let bytes = file.bytes().map_or(0, <[u8]>::len);
                let summary = file.summary().map_or(0, str::len);
                (bytes + summary) as u64
            })
            .sum();
        let children: u64 = request
            .child_documents()
            .iter()
            .map(|child| child.text().len() as u64)
            .sum();
        files + children
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

        let files = super::own_files(dir.path()).expect("walks");

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

        let children = super::child_documents(dir.path()).expect("walks");
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

        let files = super::own_files(dir.path()).expect("walks");
        let children = super::child_documents(dir.path()).expect("walks");

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
            super::own_files(dir.path()).expect("walks"),
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

        let files = super::own_files(dir.path()).expect("walks");

        assert_eq!(files.keys().collect::<Vec<_>>(), ["lib.rs"]);
        assert!(super::child_documents(dir.path()).expect("walks").is_empty());
    }



    #[test]
    fn an_empty_directory_is_a_request_with_nothing_in_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        assert!(super::own_files(dir.path()).expect("walks").is_empty());
        assert!(super::child_documents(dir.path()).expect("walks").is_empty());
    }


    #[test]
    fn every_problem_says_what_was_left_out_and_why_on_one_line() {
        let problems = [
            Problem {
                path: PathBuf::from("/repo/Cargo.lock"),
                cause: Omission::TooLarge { size: 4_200_000 },
            },
            Problem {
                path: PathBuf::from("/repo/data.json"),
                cause: Omission::OverBudget { size: 90_000 },
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
            problems[2].to_string().contains("permission denied"),
            "{}",
            problems[2],
        );
        assert_eq!(
            problems
                .iter()
                .filter(|problem| problem.source().is_some())
                .count(),
            3,
            "every problem's cause is reachable as a source",
        );
        assert!(
            problems[2]
                .source()
                .and_then(std::error::Error::source)
                .is_some(),
            "and an unreadable file's cause names the io error under it",
        );
    }
}
