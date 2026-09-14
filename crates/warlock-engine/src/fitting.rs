use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::document::{self, Described};
use crate::{agent, hash, languages, walk};

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

// One reading of a directory, and every question its document is written
// against answered from it. The synthesis pass is checked, mended and rendered
// through the same `Expected` and the same `Described`, so a file that appears
// or changes while a run is paying for passes cannot make the check and the page
// disagree about what the directory holds.
#[derive(Debug)]
pub(crate) struct Snapshot {
    name: String,
    request: agent::Request,
    described: Described,
    files: BTreeMap<String, Measured>,
}

#[derive(Debug)]
pub(crate) struct Measured {
    pub(crate) size: u64,
    // `None` for a file that could not be read, which is a file nothing can be
    // said to know: its line is asked for again and no hash is recorded.
    pub(crate) hash: Option<String>,
}

impl Snapshot {
    pub(crate) fn take(directory: &Path) -> Result<Self, walk::Error> {
        let own = walk::own(directory)?;

        let mut described = Described::default();
        let mut files = BTreeMap::new();
        let mut omitted = Vec::new();
        for (name, path) in own.files {
            let measured = match fs::read(&path) {
                Ok(bytes) => {
                    if let Ok(text) = str::from_utf8(&bytes) {
                        let names = languages::declared_names(&path, text);
                        if !names.is_empty() {
                            described.declared.insert(name.clone(), names);
                        }
                        described.tokens.insert(name.clone(), tokens_of(text));
                    }
                    Measured {
                        size: byte_count(bytes.len()),
                        hash: Some(hash::bytes_hash(&bytes)),
                    }
                }
                Err(_) => Measured {
                    size: fs::metadata(&path).map_or(0, |found| found.len()),
                    hash: None,
                },
            };
            omitted.push(agent::File::omitted(name.clone(), measured.size));
            files.insert(name, measured);
        }

        let mut children = Vec::new();
        for (child, path) in own.child_documents {
            if let Ok(text) = fs::read_to_string(&path) {
                children.push(agent::ChildDocument::new(child, text));
            }
        }

        // The name and not the path: the path is absolute, it is the reader's
        // home directory, and it would be committed.
        let name = directory.file_name().map_or_else(
            || directory.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Ok(Self {
            name,
            request: agent::Request::new(document::SYNTHESIS_PROMPT, directory)
                .with_files(omitted)
                .with_child_documents(children),
            described,
            files,
        })
    }

    pub(crate) fn directory(&self) -> &Path {
        self.request.directory()
    }

    pub(crate) fn files(&self) -> &BTreeMap<String, Measured> {
        &self.files
    }

    pub(crate) fn synthesis_request(
        &self,
        lines: &BTreeMap<String, String>,
        rejected: &[document::Defect],
    ) -> agent::Request {
        let instructions = document::synthesis_instructions(
            &self.name,
            lines,
            &document::Expected::of(&self.request),
            rejected,
        );
        self.request.clone().with_prompt(instructions)
    }

    pub(crate) fn accept_synthesis(
        &self,
        answer: &str,
        lines: &BTreeMap<String, String>,
    ) -> document::Accepted {
        document::accept_synthesis(
            answer,
            lines,
            &document::Expected::of(&self.request),
            &self.described,
        )
    }

    pub(crate) fn mend(&self, fill: &document::Fill) -> (document::Fill, Vec<document::Mend>) {
        document::mend(
            fill,
            &document::Expected::of(&self.request),
            &self.described,
        )
    }

    pub(crate) fn render(&self, fill: &document::Fill) -> String {
        document::render(
            &self.name,
            fill,
            &document::Expected::of(&self.request),
            &self.described,
        )
    }

    // What the synthesis request carries: the lines, and the documents of the
    // directories below. The files are in it by name and size only, so counting
    // them would report a payload that was never sent.
    pub(crate) fn carried_bytes(&self, lines: &BTreeMap<String, String>) -> u64 {
        lines
            .values()
            .map(|line| byte_count(line.len()))
            .sum::<u64>()
            + self
                .request
                .child_documents()
                .iter()
                .map(|child| byte_count(child.text().len()))
                .sum::<u64>()
    }
}

// Read whole rather than capped, and for the same reason the declared list is
// no longer capped: this is evidence and not a rendered line. Nothing here
// reaches a document or a request — it is compared against, and the comparison
// is the only thing standing between a true claim and a dropped one.
fn tokens_of(text: &str) -> BTreeSet<String> {
    document::identifiers(text).map(str::to_owned).collect()
}

// No budget ladder. The ladder existed because a directory is a sum that need
// not fit; one file either fits under `PER_FILE_BYTE_CAP` or is sent as a name
// and a size, and there is nothing to demote it in favour of.
pub(crate) fn one_file(
    prompt: &str,
    directory: &Path,
    name: &str,
) -> Result<(agent::Request, Described, Option<Problem>), walk::Error> {
    let path = directory.join(name);
    let size = fs::metadata(&path)
        .map_err(|source| walk::Error::Walk(source.into()))?
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
        described.tokens.insert(name.to_owned(), tokens_of(text));
    }

    Ok((
        agent::Request::new(prompt, directory).with_files([file]),
        described,
        problem,
    ))
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
    Unreadable { source: std::io::Error },
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(
                f,
                "{size} bytes is over the {PER_FILE_BYTE_CAP}-byte per-file cap, so it is listed \
                 by name and size"
            ),
            Self::Unreadable { source } => write!(f, "it could not be read: {source}"),
        }
    }
}

impl std::error::Error for Omission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source } => Some(source),
            Self::TooLarge { .. } => None,
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
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Omission, Problem};

    use crate::agent;
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    fn own_files(dir: &Path) -> Result<BTreeMap<String, u64>, crate::walk::Error> {
        Ok(super::Snapshot::take(dir)?
            .files()
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
}
