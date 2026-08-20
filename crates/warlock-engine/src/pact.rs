//! Scoping one directory into one request: what a model pass gets to see.
//!
//! Section 11 of the design doc calls context scoping "the actual
//! differentiator: maximal relevant context, minimal waste". This module is
//! that sentence made mechanical. [`gather_request`] turns a directory on disk
//! into the [`AgentRequest`](crate::AgentRequest) a pass runs on, and decides
//! the one hard question in it — what to do about a directory holding a
//! four-megabyte lockfile — without ever refusing to produce a request.
//!
//! What goes in, and nothing else:
//!
//! * **The directory's own files**, each with its bytes: the whole listing, its
//!   own `WARLOCK.md` among them as an ordinary file. Files below the immediate
//!   children are never read — that is the waste the scoping exists to avoid.
//! * **Each immediate child directory's `WARLOCK.md`**, where one exists. This
//!   is how a directory learns what is under it: the children have already
//!   described themselves, so their parent reads summaries instead of source.
//!   A child with no document contributes no entry and is not an error; it is
//!   the ordinary state of a directory nobody has pacted yet.
//!
//! The walk is the same walk as [`load`](crate::load) and [`hash`](crate::hash)
//! — the [`ignore`] crate, `follow_links(false)`, `require_git(false)`,
//! `.warlock/` pruned by name — so a file that is gitignored, hidden or
//! Warlock's own bookkeeping is as absent from a request as it is from a tree
//! or a digest. Symlinks are neither followed nor listed. Relative paths are
//! spelled by [`to_manifest_path`], forward slashes and all, and everything
//! comes out sorted, so two builds of an unchanged directory are equal values.
//!
//! # The two caps, and why neither can fail a pact
//!
//! A request has a budget, because a context window does:
//! [`PER_FILE_BYTE_CAP`] for any one file and [`REQUEST_BYTE_CAP`] for the
//! whole thing. What happens at the edge of a budget is a decision, and two
//! were made here.
//!
//! **Omit and list, never truncate.** A file over budget is still in the
//! request — as its name and its size in bytes, with no contents at all. Half a
//! source file invites confident wrong conclusions about the half that never
//! arrived; a name and a size is accurate information a model can document
//! honestly ("a 4.1 MB `Cargo.lock`, not read").
//!
//! **Over budget is never fatal.** Section 3 of the design doc says Warlock
//! never makes the wrong thing impossible, and failing here would do exactly
//! that: one committed lockfile or one generated schema would leave a directory
//! permanently unpactable, with no way out but deleting the file. So every
//! omission is a [`Problem`] reported alongside a request that is still
//! perfectly good — the same non-fatal shape [`LoadProblem`](crate::LoadProblem)
//! established, for the same reason: the thing that went wrong is said out
//! loud, once, rather than silently changing what happened.
//!
//! This does not contradict the rule that an unreadable file is fatal to a
//! *hash* (see [`hash`](crate::hash)): over budget is a disclosed policy this
//! module applies on purpose, while an undetected hole in a digest is a false
//! green nobody earned. A file that genuinely cannot be read is a third case
//! again, and gets its own cause ([`Omission::Unreadable`]) so it is never
//! mistaken for either.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{AgentChildDocument, AgentFile, AgentRequest, ManifestError, to_manifest_path};

/// The directory holding Warlock's own bookkeeping, never part of a request.
const MANIFEST_DIR: &str = ".warlock";

/// The document a directory is described by, and the only file name a child
/// directory contributes to its parent's request.
const DOCUMENT_FILE: &str = "WARLOCK.md";

/// How deep the walk goes: the directory itself (0), its own files and its
/// immediate children (1), and the files directly inside those children (2),
/// of which only `WARLOCK.md` is kept.
///
/// The depth limit *is* the scoping rule, enforced by the walker rather than by
/// remembering to check: source below an immediate child cannot reach a request
/// even by accident, because the walk never descends far enough to meet it.
const WALK_DEPTH: usize = 2;

/// The most bytes one file may contribute before it is listed instead of sent:
/// 128 KiB.
///
/// Roughly 37,000 tokens of source at the ~3.5 bytes per token that code
/// tokenises at — comfortably more than any hand-written source file (the
/// largest module in this repository is under 50 KiB), and comfortably less
/// than the generated artefacts this cap exists for: lockfiles, vendored
/// bundles, checked-in schemas, minified assets. A file that trips this cap is
/// almost never a file a model needed to read line by line; its name, and the
/// fact that it is enormous, is the part worth documenting.
pub const PER_FILE_BYTE_CAP: u64 = 128 * 1024;

/// The most bytes one whole request may carry: 256 KiB.
///
/// About 75,000 tokens by the same measure — a large but workable share of a
/// 200,000-token window, leaving the prompt, the children's documents and the
/// answer itself room to breathe. Twice [`PER_FILE_BYTE_CAP`] on purpose: even
/// a directory holding two maximal files still sends both, while the directory
/// that trips this cap is one holding hundreds of ordinary files, where sending
/// every one of them buys less than it costs.
///
/// The budget counts everything the request carries: the bytes of the files
/// sent whole, and the text of the children's documents. Only files are ever
/// dropped to get under it — see [`gather_request`].
pub const REQUEST_BYTE_CAP: u64 = 256 * 1024;

/// Build the request for one pass over `directory`, asking `prompt`.
///
/// The request carries `directory`'s own files and its immediate children's
/// documents, gathered under the ignore rules and the two byte caps this module
/// documents. Alongside it comes every [`Problem`] the budget caused — an empty
/// list on the ordinary directory, and never a reason to stop.
///
/// Trimming, when [`REQUEST_BYTE_CAP`] is exceeded, is largest-first: the
/// biggest file is turned into a name and a size, then the next, until the
/// request fits. Largest-first because it reaches the budget in the fewest
/// omissions, and because a directory's biggest file is the least likely to be
/// the one that explains what the directory is for. Ties are broken by path so
/// the result is a value, not a race. Children's documents count towards the
/// budget but are never dropped: a file left out still says its name and size,
/// while a document left out would replace the only account of a whole subtree
/// with nothing — so a pathological child document can leave a request over the
/// cap with every file listed rather than sent, which is a fact about that
/// document and still not a failure.
///
/// ```
/// use std::fs;
/// use warlock_engine::{Gathered, gather_request};
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("lib.rs"), "//! Core engine.\n")?;
/// fs::create_dir(dir.path().join("inner"))?;
/// fs::write(dir.path().join("inner/WARLOCK.md"), "# inner\n")?;
/// fs::write(dir.path().join("inner/deep.rs"), "fn deep() {}\n")?;
///
/// let Gathered { request, problems } = gather_request("summarise", dir.path())?;
///
/// // The directory's own files, with their bytes.
/// assert_eq!(request.files().len(), 1);
/// assert_eq!(request.files()[0].path(), "lib.rs");
/// assert_eq!(request.files()[0].bytes(), Some(&b"//! Core engine.\n"[..]));
/// // The child describes itself; its source is never read.
/// assert_eq!(request.child_documents().len(), 1);
/// assert_eq!(request.child_documents()[0].directory(), "inner");
/// assert!(problems.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Only the two ways there is no request to build at all — nothing about the
/// caps is here, because nothing about the caps is fatal:
///
/// * [`Error::Walk`] if `directory` cannot be walked: it is not there, it
///   cannot be listed, or something vanished from under the walk.
/// * [`Error::Path`] if a file's path has no relative, forward-slash, UTF-8
///   form, and so cannot be named to a model. The same rule, for the same
///   reason, as [`subtree_hash`](crate::subtree_hash).
pub fn gather_request(
    prompt: impl Into<String>,
    directory: impl AsRef<Path>,
) -> Result<Gathered, Error> {
    let directory = directory.as_ref();
    let found = walk(directory)?;

    let mut problems = Vec::new();
    let mut carried: u64 = 0;

    // Children first: their documents are part of the budget the files are
    // then fitted into, and they are the part that never gives way.
    let mut child_documents = Vec::new();
    for (child, path) in found.child_documents {
        match fs::read_to_string(&path) {
            Ok(text) => {
                carried = carried.saturating_add(byte_count(text.len()));
                child_documents.push(AgentChildDocument::new(child, text));
            }
            // Including a document that could not be read is not an option —
            // there is no text — so it contributes nothing and says so.
            Err(source) => problems.push(Problem {
                path,
                cause: Omission::Unreadable { source },
            }),
        }
    }

    // Files in sorted order, each sent whole unless it alone is too big. The
    // size comes from the filesystem before anything is opened, so an enormous
    // file is never read into memory just to be dropped again.
    let mut files = Vec::new();
    let mut on_disk = Vec::new();
    for (relative, path) in found.files {
        let size = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            // No size means nothing true to list, so the file is left out
            // entirely rather than listed at a made-up length.
            Err(source) => {
                problems.push(Problem {
                    path,
                    cause: Omission::Unreadable { source },
                });
                continue;
            }
        };

        let file = if size > PER_FILE_BYTE_CAP {
            problems.push(Problem {
                path: path.clone(),
                cause: Omission::TooLarge { size },
            });
            AgentFile::omitted(relative, size)
        } else {
            match fs::read(&path) {
                Ok(bytes) => {
                    carried = carried.saturating_add(byte_count(bytes.len()));
                    AgentFile::present(relative, bytes)
                }
                Err(source) => {
                    problems.push(Problem {
                        path: path.clone(),
                        cause: Omission::Unreadable { source },
                    });
                    AgentFile::omitted(relative, size)
                }
            }
        };
        files.push(file);
        on_disk.push(path);
    }

    trim_to_budget(&mut files, &on_disk, carried, &mut problems);

    Ok(Gathered {
        request: AgentRequest::new(prompt, directory)
            .with_files(files)
            .with_child_documents(child_documents),
        problems,
    })
}

/// Turn the biggest files into names and sizes until `carried` is inside
/// [`REQUEST_BYTE_CAP`], reporting each one.
///
/// `on_disk` is the path each entry of `files` came from, index for index, so a
/// problem can name a file on the filesystem rather than a relative spelling.
/// Stops when the budget is met or when there is nothing left to give up,
/// whichever comes first — the second case is over budget with every file
/// already listed, which is still a request and still not an error.
fn trim_to_budget(
    files: &mut [AgentFile],
    on_disk: &[PathBuf],
    carried: u64,
    problems: &mut Vec<Problem>,
) {
    if carried <= REQUEST_BYTE_CAP {
        return;
    }

    // Biggest first, and by path where two are the same size: the order files
    // are given up in has to be a property of the directory, not of how the
    // filesystem happened to enumerate it.
    let mut order: Vec<usize> = (0..files.len())
        .filter(|&index| !files[index].is_omitted())
        .collect();
    order.sort_by_key(|&index| (Reverse(files[index].size()), files[index].path().to_owned()));

    let mut carried = carried;
    for index in order {
        if carried <= REQUEST_BYTE_CAP {
            break;
        }
        let size = files[index].size();
        let path = files[index].path().to_owned();
        files[index] = AgentFile::omitted(path, size);
        carried = carried.saturating_sub(size);
        problems.push(Problem {
            path: on_disk[index].clone(),
            cause: Omission::OverBudget { size },
        });
    }
}

/// What one request is built from: the directory's own files, and its immediate
/// children's documents, each keyed by the relative path it will be named by.
///
/// [`BTreeMap`]s because the key order is the request's order, and the request's
/// order has to be the same on two machines that enumerate a directory
/// differently.
#[derive(Debug)]
struct Found {
    /// The files sitting directly in the directory, keyed by name.
    files: BTreeMap<String, PathBuf>,
    /// The `WARLOCK.md` of each immediate child that has one, keyed by the
    /// child directory's name.
    child_documents: BTreeMap<String, PathBuf>,
}

/// Everything at or just below `dir` that a request can be built from.
///
/// One pass, [`WALK_DEPTH`] deep, under the ignore rules the rest of the crate
/// walks by. Directories are not collected: a child directory matters here only
/// as the place a `WARLOCK.md` was found, and a child with none simply never
/// appears.
fn walk(dir: &Path) -> Result<Found, Error> {
    let walker = WalkBuilder::new(dir)
        // The same three rules as `load` and `hash`, for the same reasons: a
        // symlinked cycle has to terminate, a fixture with a `.gitignore` and
        // no `.git` still has to be ignored properly, and `.warlock/` is
        // Warlock's own bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        .max_depth(Some(WALK_DEPTH))
        .build();

    let mut found = Found {
        files: BTreeMap::new(),
        child_documents: BTreeMap::new(),
    };
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        let depth = entry.depth();
        // Regular files only. With `follow_links(false)` a symlink reports as a
        // symlink, so it is neither descended into nor listed as whatever it
        // points at.
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();

        if depth == 1 {
            // The directory's own file, its `WARLOCK.md` among them: an
            // existing document is an ordinary file of the directory that holds
            // it, and gets no slot of its own anywhere.
            found.files.insert(relative(dir, &path)?, path);
        } else if depth == WALK_DEPTH && path.file_name() == Some(OsStr::new(DOCUMENT_FILE)) {
            // A child's document, filed under the child directory rather than
            // under the document: the name is the same for every child, and the
            // directory is what a reader needs to place it.
            let Some(child) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            found.child_documents.insert(relative(dir, &child)?, path);
        }
    }
    Ok(found)
}

/// `path` named relative to `dir`, in the manifest's forward-slash spelling.
fn relative(dir: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(dir, path).map_err(|source| Error::Path {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// A byte count as the budget counts it.
///
/// Saturating rather than fallible, exactly as in [`hash`](crate::hash):
/// `usize` is at most 64 bits on every target this builds for, so the clamp is
/// unreachable, and a budget is no place to introduce a panic over a case that
/// cannot happen.
fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// What gathering produced: the request, and everything left out of it.
///
/// A plain pair for the same reason as [`Loaded`](crate::Loaded): the request is
/// the thing to send, the problems are the thing to report once, and they have
/// different lifetimes. Nothing on [`AgentRequest`] records that a file was
/// omitted beyond the file's own missing bytes — a request is what a model sees,
/// not a log of how it was built.
#[derive(Debug)]
pub struct Gathered {
    /// The request, ready to hand to an [`Agent`](crate::Agent).
    pub request: AgentRequest,
    /// Every file the caps left out, in the order they were given up: the
    /// per-file cases in path order, then the whole-request ones largest first.
    /// Empty is the normal case.
    pub problems: Vec<Problem>,
}

/// One file left out of a request, and why.
///
/// The shape [`LoadProblem`](crate::LoadProblem) established — a path, a cause,
/// one line of [`Display`](fmt::Display) — because it is the same kind of thing:
/// something that went wrong without being worth failing over, said once and in
/// full. A caller that ignores these gets a pact built on slightly less than the
/// whole directory, which is safe, just unexplained.
#[derive(Debug)]
pub struct Problem {
    /// The file that was left out, as it sits on disk.
    pub path: PathBuf,
    /// Why it was left out.
    pub cause: Omission,
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

/// Why one file's contents are not in a request.
///
/// Three separate answers rather than one "skipped", because they call for
/// three different reactions: nothing at all, since a huge generated file is
/// working as intended; nothing at all again, though a directory that keeps
/// tripping the whole-request cap is one worth splitting up; and a look at the
/// filesystem, because a file Warlock cannot read is a file nobody's tooling
/// can read.
#[derive(Debug)]
#[non_exhaustive]
pub enum Omission {
    /// The file is larger by itself than [`PER_FILE_BYTE_CAP`], so it was
    /// listed rather than sent. Nothing else in the directory is affected.
    TooLarge {
        /// Its size in bytes, which is what the request carries in place of it.
        size: u64,
    },
    /// The file fitted [`PER_FILE_BYTE_CAP`], but the directory as a whole was
    /// over [`REQUEST_BYTE_CAP`] and this was one of the largest files in it.
    OverBudget {
        /// Its size in bytes, which is what the request carries in place of it.
        size: u64,
    },
    /// The file could not be read at all. Not a budget decision and never
    /// counted as one: this is the filesystem saying no.
    Unreadable {
        /// What the filesystem said.
        source: std::io::Error,
    },
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(
                f,
                "{size} bytes is over the {PER_FILE_BYTE_CAP}-byte per-file cap, so it is listed \
                 by name and size"
            ),
            Self::OverBudget { size } => write!(
                f,
                "the directory is over the {REQUEST_BYTE_CAP}-byte request cap, so this file of \
                 {size} bytes is listed by name and size"
            ),
            Self::Unreadable { source } => write!(f, "it could not be read: {source}"),
        }
    }
}

impl std::error::Error for Omission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source } => Some(source),
            Self::TooLarge { .. } | Self::OverBudget { .. } => None,
        }
    }
}

/// Everything that can stop a directory becoming a request.
///
/// Hand-rolled like every other error in this crate, and deliberately short:
/// neither cap is in here, because neither cap can fail a pact.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The directory could not be walked: it is not there, it cannot be
    /// listed, or something vanished from under the walk.
    Walk {
        /// What the walker said, including which path it was on.
        source: ignore::Error,
    },
    /// A file's path has no relative, forward-slash, UTF-8 form, so it cannot
    /// be named to a model.
    Path {
        /// The path that could not be named.
        path: PathBuf,
        /// Why it could not be. Boxed for the same reason as
        /// [`HashError::Path`](crate::HashError::Path): a manifest error
        /// carries a parser error inside it, and the other variant here is an
        /// `ignore::Error`.
        source: Box<ManifestError>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Walk { source } => write!(f, "could not walk the directory: {source}"),
            Self::Path { path, source } => write!(
                f,
                "could not name `{}` relative to the directory being pacted: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Walk { source } => Some(source),
            Self::Path { source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Gathered, Omission, PER_FILE_BYTE_CAP, Problem, REQUEST_BYTE_CAP, gather_request};
    use crate::{AgentChildDocument, AgentFile, AgentRequest};

    /// Write `contents` at `dir/name`, creating whatever directories it needs.
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    /// `size` bytes of something, cheap to make and impossible to confuse with
    /// a fixture's real text.
    fn filler(size: u64) -> Vec<u8> {
        vec![b'x'; usize::try_from(size).expect("a test file fits in memory")]
    }

    /// The request for `dir`, insisting nothing was left out of it.
    ///
    /// Most of these fixtures are small enough to send whole, so an empty
    /// problem list is part of what they assert: a gather that quietly started
    /// dropping files would fail here rather than pass unnoticed.
    fn request_for(dir: &Path) -> AgentRequest {
        let Gathered { request, problems } = gather_request("summarise", dir).expect("gathers");
        assert!(problems.is_empty(), "{problems:?}");
        request
    }

    /// The paths of a request's files, in the order it carries them.
    fn file_paths(request: &AgentRequest) -> Vec<&str> {
        request.files().iter().map(AgentFile::path).collect()
    }

    /// The file a request carries at `path`.
    fn file<'a>(request: &'a AgentRequest, path: &str) -> &'a AgentFile {
        request
            .files()
            .iter()
            .find(|file| file.path() == path)
            .unwrap_or_else(|| panic!("`{path}` is in the request: {:?}", file_paths(request)))
    }

    /// How many bytes a request actually carries: file contents sent whole,
    /// plus the children's documents.
    fn carried(request: &AgentRequest) -> u64 {
        let files: u64 = request
            .files()
            .iter()
            .filter(|file| !file.is_omitted())
            .map(AgentFile::size)
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

        let request = request_for(dir.path());

        assert_eq!(
            file_paths(&request),
            ["Cargo.toml", "build.rs"],
            "only the directory's own files, sorted; nothing from below it"
        );
        assert_eq!(
            file(&request, "build.rs").bytes(),
            Some(&b"fn main() {}\n"[..]),
            "and they carry their bytes",
        );
        assert_eq!(
            request
                .child_documents()
                .iter()
                .map(|child| (child.directory(), child.text()))
                .collect::<Vec<_>>(),
            [("src", "# src\n\nThe code.\n")],
            "a child with a document contributes it; `tests/` has none and \
             contributes no entry, which is not an error",
        );
        assert!(
            !format!("{request:?}").contains("Deeper still"),
            "a grandchild's document is already covered by its parent's, and \
             its source is never read at all",
        );
        assert_eq!(request.directory(), dir.path());
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

        let request = request_for(dir.path());

        assert_eq!(file_paths(&request), ["alpha.rs", "zeta.rs"]);
        assert_eq!(
            request
                .child_documents()
                .iter()
                .map(AgentChildDocument::directory)
                .collect::<Vec<_>>(),
            ["alpha", "zeta"],
        );
        assert_eq!(request, request_for(dir.path()), "two gathers, one value");
    }

    #[test]
    fn the_request_obeys_the_same_ignore_rules_as_the_rest_of_the_crate() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), ".gitignore", "secret.txt\n/generated\n");
        write(dir.path(), "secret.txt", "shh\n");
        write(dir.path(), ".hidden", "shh\n");
        write(dir.path(), ".warlock/notes.md", "# ours\n");
        write(dir.path(), ".warlock/WARLOCK.md", "# not a module\n");
        write(
            dir.path(),
            "generated/WARLOCK.md",
            "# not a module either\n",
        );
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let request = request_for(dir.path());

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "gitignored, hidden and `.warlock/` files come through the same \
             walk as everything else, so they never arrive at all"
        );
        assert!(
            request.child_documents().is_empty(),
            "and a document inside an ignored or pruned directory is not a \
             child document: {:?}",
            request.child_documents(),
        );
    }

    /// Only on unix, because the fixture needs `std::os::unix::fs::symlink` to
    /// build the cycle at all. The behaviour under test — that the walk does
    /// not follow links — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_symlink_is_neither_followed_nor_listed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        std::os::unix::fs::symlink(dir.path(), dir.path().join("up")).expect("links to itself");
        std::os::unix::fs::symlink(dir.path().join("lib.rs"), dir.path().join("alias.rs"))
            .expect("links to a file");

        let request = request_for(dir.path());

        assert_eq!(file_paths(&request), ["lib.rs"]);
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn an_existing_document_is_an_ordinary_file_of_its_own_directory() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "WARLOCK.md", "# engine\n\nWhat it was.\n");
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let request = request_for(dir.path());

        assert_eq!(
            file_paths(&request),
            ["WARLOCK.md", "lib.rs"],
            "the directory's own document is listed like any other file",
        );
        assert_eq!(
            file(&request, "WARLOCK.md").bytes(),
            Some(&b"# engine\n\nWhat it was.\n"[..]),
        );
        assert!(
            request
                .child_documents()
                .iter()
                .all(|child| child.directory() != "." && child.directory() != "WARLOCK.md"),
            "and it is nobody's child document: {:?}",
            request.child_documents(),
        );
    }

    #[test]
    fn a_file_over_the_per_file_cap_is_listed_by_name_and_size_and_reported() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let size = PER_FILE_BYTE_CAP + 1;
        let lock = write(dir.path(), "Cargo.lock", filler(size));
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path()).expect("a huge file is not fatal");

        let listed = file(&request, "Cargo.lock");
        assert!(listed.is_omitted());
        assert_eq!(
            listed.size(),
            size,
            "the size is the fact that goes instead"
        );
        assert_eq!(
            listed.bytes(),
            None,
            "never truncated: no part of it is presented as if it were the whole"
        );
        assert_eq!(
            file(&request, "lib.rs").bytes(),
            Some(&b"//! Core engine.\n"[..]),
            "and the rest of the directory is untouched",
        );

        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, lock, "the problem names the file on disk");
        assert!(
            matches!(problems[0].cause, Omission::TooLarge { size: reported } if reported == size),
            "{:?}",
            problems[0],
        );
    }

    #[test]
    fn a_file_exactly_at_the_per_file_cap_is_still_sent_whole() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "big.bin", filler(PER_FILE_BYTE_CAP));

        let request = request_for(dir.path());

        assert_eq!(
            file(&request, "big.bin").bytes().map(<[u8]>::len),
            Some(usize::try_from(PER_FILE_BYTE_CAP).expect("fits")),
            "the cap is what a file may not exceed, not what it may not reach",
        );
    }

    #[test]
    fn a_directory_over_the_request_cap_gives_up_its_largest_files_first() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Named so that alphabetical order is the reverse of size order: a
        // gather that dropped files in path order would fail here.
        let sizes = [
            ("a.bin", 80 * 1024),
            ("b.bin", 90 * 1024),
            ("c.bin", 100 * 1024),
            ("d.bin", 110 * 1024),
            ("e.bin", 120 * 1024),
        ];
        for (name, size) in sizes {
            write(dir.path(), name, filler(size));
        }

        let Gathered { request, problems } =
            gather_request("summarise", dir.path()).expect("a fat directory is not fatal");

        assert_eq!(
            file_paths(&request),
            ["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"],
            "every file is still in the request, in path order",
        );
        for (name, size) in sizes {
            assert_eq!(
                file(&request, name).size(),
                size,
                "and every one of them still says how big it is",
            );
        }
        assert_eq!(
            request
                .files()
                .iter()
                .filter(|file| !file.is_omitted())
                .map(AgentFile::path)
                .collect::<Vec<_>>(),
            ["a.bin", "b.bin"],
            "the two smallest survive: the biggest are given up first, so the \
             fewest files are lost",
        );
        assert!(
            carried(&request) <= REQUEST_BYTE_CAP,
            "{} bytes is still over the {REQUEST_BYTE_CAP}-byte cap",
            carried(&request),
        );

        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.path.clone())
                .collect::<Vec<_>>(),
            ["e.bin", "d.bin", "c.bin"].map(|name| dir.path().join(name)),
            "reported in the order they were given up, largest first",
        );
        assert!(
            problems
                .iter()
                .all(|problem| matches!(problem.cause, Omission::OverBudget { .. })),
            "over budget is its own cause, not the per-file one: {problems:?}",
        );
    }

    #[test]
    fn a_directory_inside_the_request_cap_gives_up_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "a.bin", filler(REQUEST_BYTE_CAP / 2));
        write(dir.path(), "b.bin", filler(REQUEST_BYTE_CAP / 2));

        let request = request_for(dir.path());

        assert!(
            request.files().iter().all(|file| !file.is_omitted()),
            "exactly at the cap is inside it",
        );
        assert_eq!(carried(&request), REQUEST_BYTE_CAP);
    }

    #[test]
    fn a_childs_document_counts_towards_the_budget_and_is_never_the_thing_dropped() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(
            dir.path(),
            "src/WARLOCK.md",
            "x".repeat(usize::try_from(REQUEST_BYTE_CAP).expect("fits")),
        );
        write(dir.path(), "lib.rs", filler(1024));

        let Gathered { request, problems } = gather_request("summarise", dir.path())
            .expect("an enormous child document is not fatal either");

        assert_eq!(
            request.child_documents().len(),
            1,
            "the account of a whole subtree is the one thing that never gives \
             way: dropping it would leave nothing in its place",
        );
        assert!(
            file(&request, "lib.rs").is_omitted(),
            "the file gives way instead, and still says its name and size",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            matches!(problems[0].cause, Omission::OverBudget { size: 1024 }),
            "{:?}",
            problems[0],
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — that a file the filesystem refuses is
    /// its own case, and still not fatal — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_its_own_cause_and_still_not_fatal() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let unreadable = write(dir.path(), "secret.rs", "fn hidden() {}\n");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let Gathered { request, problems } =
            gather_request("summarise", dir.path()).expect("an unreadable file is not fatal");

        assert!(file(&request, "secret.rs").is_omitted());
        assert_eq!(
            file(&request, "lib.rs").bytes(),
            Some(&b"//! Core engine.\n"[..]),
            "one file nobody can read is one file's problem",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            matches!(problems[0].cause, Omission::Unreadable { .. }),
            "a refused file is never reported as a budget decision: {:?}",
            problems[0],
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    #[test]
    fn an_empty_directory_is_a_request_with_nothing_in_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let request = request_for(dir.path());

        assert!(request.files().is_empty());
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn a_directory_that_is_not_there_is_a_walk_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let error = gather_request("summarise", dir.path().join("nowhere"))
            .expect_err("there is nothing to walk");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(error.source().is_some(), "{error:?}");
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
