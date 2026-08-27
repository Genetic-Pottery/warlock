//! The subtree hash: what "changed" means, mechanically.
//!
//! Section 6 of the design doc puts a hash at the trigger and a human (or an
//! AI) at the judgement: Warlock never decides whether a document is *right*,
//! it only decides whether the content it was granted against is still the
//! content on disk. [`subtree_hash`] is that mechanical half — one digest over
//! everything at and below a directory, to be compared against the
//! `granted_hash` a pact recorded.
//!
//! Everything about it is chosen so that two clones of the same commit, on two
//! machines, at two absolute paths, agree:
//!
//! * **The input is the files, and only the files.** For each file, its path
//!   relative to the hashed directory — forward slashes, the same spelling the
//!   manifest stores paths in — followed by that file's bytes. Nothing else
//!   goes in: no mtime, no permissions, no inode or device number, no absolute
//!   path, no directory entry the filesystem happened to hand back first.
//! * **Order is the sorted relative paths**, not the order the walk produced
//!   them in, so the digest cannot depend on how a filesystem enumerates a
//!   directory.
//! * **The same rules as the walk in [`load`](crate::load)**: the [`ignore`]
//!   crate, so `.gitignore` at every level, hidden entries and global excludes
//!   are honoured as git honours them, `.warlock/` is pruned by name, and
//!   symlinks are never followed, so a symlinked directory cycle terminates
//!   instead of hanging.
//! * **And the repository's own `.warlockignore`**, read by
//!   that same crate as one more ignore file, so a directory of images or a
//!   notebook the repository excluded contributes no path and no byte. A
//!   directory that is itself excluded hashes as an empty directory does —
//!   selecting it directly is not a way round the rules, which is why the
//!   digest starts with a check the walker cannot make for its own root.
//!
//! `.warlockignore` is not otherwise special: it is an ordinary file in the
//! walk, hashed like any other, at whatever level it sits. So editing the
//! rules restales twice over — the set of covered files changed, and the bytes
//! of a covered file changed — and adopting or removing one restales every
//! directory whose covered content it moves, all at once. That is the intended
//! consequence rather than a bug: the content a document was granted against
//! is not the content on disk any more.
//!
//! And one rule that is a decision rather than a detail: **a file that cannot
//! be read is an error, never a skip.** A file contributing nothing when
//! unreadable would hash exactly as it would if it had been deleted, so a
//! subtree granted while that file was absent could come back fresh on a run
//! where it merely could not be opened — a false green, the one outcome the
//! design says must be earned. Hashing the error text instead would make the
//! digest depend on the operating system's wording and break the property that
//! a fresh clone hashes the same. So the whole call fails, and it is the
//! caller's business what to do about it.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::ignores;
use crate::{ManifestError, to_manifest_path};

/// The directory holding Warlock's own bookkeeping, never part of a hash.
const MANIFEST_DIR: &str = ".warlock";

/// Domain separation for the digest, via blake3's key derivation.
///
/// It costs nothing in the input and buys two things: this hash can never
/// collide with a plain `blake3` of the same bytes computed for some other
/// purpose, and the `v1` is where a future change to what goes into the digest
/// announces itself, instead of silently making every recorded grant wrong in
/// a way that still looks like a hash.
///
/// The rule that governs it: **the version moves when a repository that
/// changed nothing would hash differently.** Not when this file changes, and
/// not when the rules gain a source — teaching the walk to read
/// `.warlockignore` is inert for a repository that has none, since no rules
/// are added, the walk yields the same files and the digest is byte-identical,
/// so it does not move. Bumping it there would restale every repository in the
/// world for a feature none of them opted into.
const HASH_CONTEXT: &str = "warlock subtree hash v1 2026-08-19";

/// The hash of everything at and below `dir`, as lowercase hex.
///
/// The digest covers every file the ignore rules keep, at any depth, including
/// `dir`'s own `WARLOCK.md` — so editing any file at or below `dir`, adding
/// one, deleting one or renaming one changes the result, and editing a file
/// inside a gitignored, `.warlockignore`d or `.warlock/` directory does not.
/// Directories themselves are not part of the input, so an empty directory is
/// invisible to the hash.
///
/// `dir` itself is checked against `.warlockignore` before anything is read.
/// A directory the repository excluded hashes as an empty one does, whoever
/// asked and however they got here: being handed to this function directly is
/// not a way past the rules.
///
/// The returned string is what a [`PactEntry::granted_hash`] holds: 64 hex
/// characters, opaque to everything else, compared only for equality.
///
/// [`PactEntry::granted_hash`]: crate::PactEntry::granted_hash
///
/// ```
/// use std::fs;
/// use warlock_engine::subtree_hash;
///
/// let dir = tempfile::tempdir()?;
/// fs::write(dir.path().join("WARLOCK.md"), "# module\n")?;
///
/// let before = subtree_hash(dir.path())?;
/// assert_eq!(before, subtree_hash(dir.path())?, "the same bytes hash the same");
///
/// fs::write(dir.path().join("WARLOCK.md"), "# module, revised\n")?;
/// assert_ne!(before, subtree_hash(dir.path())?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// * [`Error::Walk`] if the tree cannot be walked — `dir` is not there, or a
///   directory in it cannot be listed, or an entry vanished mid-walk, or a
///   `.warlockignore` governing it cannot be parsed. An unusable rule file is
///   never read as "no rules": content the reader excluded is not hashed on
///   the strength of a file this could not understand.
/// * [`Error::Read`] if a file the walk found cannot be read, naming that file.
/// * [`Error::Path`] if a file's path has no relative, forward-slash, UTF-8
///   form, and so cannot be part of a portable digest.
///
/// Any of them means no hash at all. A partial digest is never returned.
pub fn subtree_hash(dir: impl AsRef<Path>) -> Result<String, Error> {
    let dir = dir.as_ref();
    let mut hasher = blake3::Hasher::new_derive_key(HASH_CONTEXT);

    // The walker will not apply the rules to the root it is given, so ask
    // separately, first, and hash nothing at all if the answer is yes. An
    // excluded directory then hashes exactly as an empty one does, which is
    // what it is as far as Warlock is concerned.
    if ignores::is_ignored(dir).map_err(|source| Error::Walk { source })? {
        return Ok(hasher.finalize().to_hex().to_string());
    }

    for (relative, path) in files_under(dir)? {
        // Length-prefixed, so no arrangement of names and contents can be
        // mistaken for another: `a/b` holding `c` and `a` holding `bc` are
        // different inputs and must be different digests.
        hasher.update(&length(relative.len()).to_le_bytes());
        hasher.update(relative.as_bytes());

        // Read whole rather than streamed: the length has to go in ahead of
        // the bytes, and a length taken from metadata is a length that can
        // disagree with what is then read. Source trees are the scale this
        // works at.
        let bytes = fs::read(&path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        hasher.update(&length(bytes.len()).to_le_bytes());
        hasher.update(&bytes);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Every file at or below `dir` that survives the ignore rules, keyed by its
/// path relative to `dir` in the manifest's forward-slash spelling.
///
/// A [`BTreeMap`] because the key order *is* the hash order: whatever sequence
/// the walker produced is thrown away here, and that is what keeps the digest
/// independent of the filesystem.
fn files_under(dir: &Path) -> Result<BTreeMap<String, PathBuf>, Error> {
    let walker = WalkBuilder::new(dir)
        // The same three rules the loader walks by, for the same reasons: a
        // symlinked cycle has to terminate, a fixture with a `.gitignore` and
        // no `.git` still has to be ignored properly, and `.warlock/` is
        // Warlock's own bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        // The repository's own exclusions, read by the same crate that reads
        // `.gitignore` and with the same semantics — nesting, negation,
        // anchoring, directory-only — because it is the same matcher.
        .add_custom_ignore_filename(ignores::FILENAME)
        .build();

    let mut files = BTreeMap::new();
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk { source })?;
        // An ignore file the walker could not use is reported beside the
        // directory it sits in rather than in place of it, and taking that as
        // "no rules" would hash the very content the repository excluded. So
        // it is promoted to the failure it is, naming the file and the line.
        if let Some(source) = entry.error() {
            return Err(Error::Walk {
                source: source.clone(),
            });
        }
        // Regular files only. With `follow_links(false)` a symlink reports as
        // a symlink, so links are neither followed nor hashed as whatever they
        // point at — a link's target is already hashed if it is inside the
        // subtree, and is not the subtree's content if it is not.
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let relative = to_manifest_path(dir, &path).map_err(|source| Error::Path {
            path: path.clone(),
            source: Box::new(source),
        })?;
        files.insert(relative, path);
    }
    Ok(files)
}

/// A byte count as it goes into the digest.
///
/// Saturating rather than fallible or panicking: `usize` is at most 64 bits on
/// every target this builds for, so the clamp is unreachable, and a hash
/// function is the last place to introduce a panic over a case that cannot
/// happen.
fn length(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// Everything that can stop a subtree being hashed.
///
/// Hand-rolled like [`ManifestError`] and [`LoadError`](crate::LoadError):
/// three variants do not pay for an error-handling dependency, and each one
/// names the path it happened to so a caller can say which file to go and look
/// at.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The tree could not be walked: the directory is not there, cannot be
    /// listed, or something vanished from under the walk.
    Walk {
        /// What the walker said, including which path it was on.
        source: ignore::Error,
    },
    /// A file was found but could not be read. Deliberately fatal: skipping it
    /// would hash exactly like deleting it.
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// A file's path has no relative, forward-slash, UTF-8 form, so it cannot
    /// contribute a portable name to the digest.
    Path {
        /// The path that could not be named.
        path: PathBuf,
        /// Why it could not be. Boxed because a manifest error carries a
        /// parser error inside it, and every other variant here is a path and
        /// an errno — there is no reason for all of them to pay for that.
        source: Box<ManifestError>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Walk { source } => write!(f, "could not walk the subtree: {source}"),
            Self::Read { path, source } => write!(
                f,
                "could not read `{}`, so the subtree has no hash: {source}",
                path.display()
            ),
            Self::Path { path, source } => write!(
                f,
                "could not name `{}` relative to the hashed directory: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Walk { source } => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::Path { source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{Error, subtree_hash};

    /// A small repository: a root document, two modules, a nested one, a plain
    /// `README.md` that is nobody's document and an ordinary file here, a
    /// `.gitignore` that ignores `target/`, something inside `target/`, a
    /// `.warlockignore` excluding the author's `notes/` with a note in it, and
    /// a `.warlock/` directory with a manifest in it.
    ///
    /// Everything the hashing tests assert on is built here, in a temporary
    /// directory, so no test says anything about the warlock repository
    /// itself.
    fn fixture() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        write(repo.path(), "WARLOCK.md", "# repo\n");
        write(repo.path(), ".gitignore", "/target\n");
        write(repo.path(), ".warlockignore", "notes/\n");
        write(repo.path(), "notes/scratch.md", "thinking out loud\n");
        write(repo.path(), "crates/engine/WARLOCK.md", "# engine\n");
        write(repo.path(), "crates/engine/src/lib.rs", "pub fn one() {}\n");
        write(repo.path(), "crates/engine/src/deep/WARLOCK.md", "# deep\n");
        write(repo.path(), "crates/tui/WARLOCK.md", "# tui\n");
        write(repo.path(), "crates/tui/README.md", "# for people\n");
        write(repo.path(), "target/debug/junk.bin", "build output\n");
        write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
        repo
    }

    /// Write `contents` at `relative` under `root`, creating parents.
    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(path, contents).expect("writes a file");
    }

    /// Copy the whole of `from` into `to`, files, hidden files and all.
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("creates the destination");
        for entry in fs::read_dir(from).expect("reads a directory") {
            let entry = entry.expect("an entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("a file type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copies a file");
            }
        }
    }

    /// The hash of `relative` under `root`, or a panic naming what failed.
    fn hash(root: &Path, relative: &str) -> String {
        subtree_hash(root.join(relative)).expect("hashes")
    }

    /// The four nodes on the spine of the fixture, root first.
    const SPINE: [&str; 4] = ["", "crates", "crates/engine", "crates/engine/src"];

    #[test]
    fn the_same_tree_hashes_the_same_twice() {
        let repo = fixture();
        let first = hash(repo.path(), "");

        assert_eq!(first, hash(repo.path(), ""));
        assert_eq!(first.len(), 64, "blake3 as lowercase hex: {first}");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "{first}"
        );
    }

    #[test]
    fn a_copy_at_a_different_absolute_path_hashes_the_same() {
        let repo = fixture();
        let elsewhere = tempfile::tempdir().expect("a second temporary directory");
        let copy = elsewhere.path().join("under/another/name");
        copy_dir(repo.path(), &copy);

        assert_ne!(repo.path(), copy, "two genuinely different absolute paths");
        for node in SPINE {
            assert_eq!(
                hash(repo.path(), node),
                hash(&copy, node),
                "`{node}` holds the same bytes in both, so it hashes the same"
            );
        }
    }

    #[test]
    fn editing_a_leaf_changes_that_leaf_and_every_ancestor() {
        let repo = fixture();
        let before: Vec<String> = SPINE.iter().map(|node| hash(repo.path(), node)).collect();
        let untouched = hash(repo.path(), "crates/tui");

        write(repo.path(), "crates/engine/src/lib.rs", "pub fn two() {}\n");

        for (node, was) in SPINE.iter().zip(&before) {
            assert_ne!(
                hash(repo.path(), node),
                *was,
                "`{node}` is at or above the edited file, so its hash moves"
            );
        }
        assert_eq!(
            hash(repo.path(), "crates/tui"),
            untouched,
            "a sibling subtree holding none of the edited file is unmoved"
        );
    }

    #[test]
    fn editing_inside_an_ignored_or_warlock_directory_changes_nothing() {
        let repo = fixture();
        let before: Vec<String> = SPINE.iter().map(|node| hash(repo.path(), node)).collect();

        write(
            repo.path(),
            "target/debug/junk.bin",
            "different build output\n",
        );
        write(
            repo.path(),
            "target/debug/extra.bin",
            "a whole new artefact\n",
        );
        write(
            repo.path(),
            ".warlock/pacts.toml",
            "version = 1\n\n[[pact]]\n",
        );
        write(repo.path(), ".warlock/WARLOCK.md", "# not a module\n");
        // And the third exclusion, the repository's own: a note rewritten, and
        // a new one written, in the `notes/` the `.warlockignore` names.
        write(repo.path(), "notes/scratch.md", "thought better of it\n");
        write(repo.path(), "notes/plan.md", "a whole new note\n");
        fs::remove_file(repo.path().join("notes/plan.md")).expect("removes");
        write(repo.path(), "notes/plan.md", "and a different one again\n");

        for (node, was) in SPINE.iter().zip(&before) {
            assert_eq!(
                hash(repo.path(), node),
                *was,
                "`target/` is gitignored, `notes/` is `.warlockignore`d and \
                 `.warlock/` is ours, so `{node}` sees none of them"
            );
        }
    }

    #[test]
    fn a_directory_the_repository_excluded_hashes_as_an_empty_one() {
        let repo = fixture();
        let nothing = subtree_hash(tempfile::tempdir().expect("a temporary directory").path())
            .expect("hashes");

        assert_eq!(
            hash(repo.path(), "notes"),
            nothing,
            "handing an excluded directory over directly is not a way round \
             the rules: it holds nothing Warlock covers, so it hashes as an \
             empty directory does"
        );

        write(repo.path(), "notes/scratch.md", "rewritten\n");
        write(
            repo.path(),
            "notes/deep/further.md",
            "and one further down\n",
        );
        assert_eq!(hash(repo.path(), "notes"), nothing, "still nothing");

        assert_ne!(
            hash(repo.path(), "crates/tui"),
            nothing,
            "a directory the rules did not name is hashed as ever"
        );
    }

    #[test]
    fn the_rules_are_read_where_they_sit_not_only_at_the_root() {
        let repo = fixture();
        write(repo.path(), "crates/tui/.warlockignore", "sketches/\n");
        write(repo.path(), "crates/tui/sketches/one.svg", "<svg/>\n");
        let before = hash(repo.path(), "crates/tui");
        let root_before = hash(repo.path(), "");

        write(
            repo.path(),
            "crates/tui/sketches/one.svg",
            "<svg>two</svg>\n",
        );
        write(repo.path(), "crates/tui/sketches/two.svg", "<svg/>\n");

        assert_eq!(
            hash(repo.path(), "crates/tui"),
            before,
            "a `.warlockignore` in a subdirectory governs that subdirectory"
        );
        assert_eq!(
            hash(repo.path(), ""),
            root_before,
            "and it still governs it when the hash is taken from above"
        );
    }

    #[test]
    fn a_negated_rule_puts_a_file_back_in_the_digest() {
        let repo = fixture();
        write(repo.path(), ".warlockignore", "*.log\n!keep.log\n");
        write(repo.path(), "crates/engine/noisy.log", "one\n");
        write(repo.path(), "crates/engine/keep.log", "one\n");
        let before = hash(repo.path(), "crates/engine");

        write(repo.path(), "crates/engine/noisy.log", "two\n");
        assert_eq!(
            hash(repo.path(), "crates/engine"),
            before,
            "`*.log` excludes it"
        );

        write(repo.path(), "crates/engine/keep.log", "two\n");
        assert_ne!(
            hash(repo.path(), "crates/engine"),
            before,
            "`!keep.log` puts that one back, exactly as git would"
        );
    }

    #[test]
    fn an_anchored_rule_names_one_directory_and_not_its_namesakes() {
        let repo = fixture();
        write(repo.path(), ".warlockignore", "/docs\n");
        write(repo.path(), "docs/plan.md", "the plan\n");
        write(
            repo.path(),
            "crates/engine/docs/plan.md",
            "the engine's plan\n",
        );
        let root_before = hash(repo.path(), "");
        let engine_before = hash(repo.path(), "crates/engine");

        write(repo.path(), "docs/plan.md", "the plan, revised\n");
        assert_eq!(
            hash(repo.path(), ""),
            root_before,
            "`/docs` is anchored to the directory the rules sit in"
        );

        write(
            repo.path(),
            "crates/engine/docs/plan.md",
            "the engine's plan, revised\n",
        );
        assert_ne!(
            hash(repo.path(), "crates/engine"),
            engine_before,
            "a `docs` further down is a different directory and is covered"
        );
    }

    #[test]
    fn a_directory_only_rule_leaves_a_file_of_the_same_name_alone() {
        let repo = fixture();
        write(repo.path(), ".warlockignore", "assets/\n");
        write(repo.path(), "crates/engine/assets/logo.png", "one\n");
        write(
            repo.path(),
            "crates/tui/assets",
            "a file, not a directory\n",
        );
        let engine_before = hash(repo.path(), "crates/engine");
        let tui_before = hash(repo.path(), "crates/tui");

        write(repo.path(), "crates/engine/assets/logo.png", "two\n");
        assert_eq!(
            hash(repo.path(), "crates/engine"),
            engine_before,
            "`assets/` excludes the directory"
        );

        write(repo.path(), "crates/tui/assets", "still a file\n");
        assert_ne!(
            hash(repo.path(), "crates/tui"),
            tui_before,
            "and the trailing slash means it excludes only directories, so a \
             file called `assets` is covered"
        );
    }

    #[test]
    fn editing_the_rules_moves_the_hash_twice_over() {
        let repo = fixture();
        let before = hash(repo.path(), "");

        write(
            repo.path(),
            ".warlockignore",
            "notes/\ncrates/tui/README.md\n",
        );
        let excluding_more = hash(repo.path(), "");
        assert_ne!(
            excluding_more, before,
            "the rules changed, and the rules are also an ordinary file in \
             the digest — either alone would move it"
        );

        write(repo.path(), ".warlockignore", "notes/\n");
        assert_eq!(
            hash(repo.path(), ""),
            before,
            "and putting them back puts the digest back"
        );
    }

    #[test]
    fn rules_that_cannot_be_parsed_are_an_error_not_an_absence_of_rules() {
        let repo = fixture();
        // A range that runs backwards: a glob the matcher will not compile.
        write(repo.path(), "crates/engine/.warlockignore", "a[z-a]\n");

        for node in ["", "crates", "crates/engine", "crates/engine/src"] {
            let error = subtree_hash(repo.path().join(node))
                .expect_err("rules that cannot be read are not no rules");
            assert!(matches!(error, Error::Walk { .. }), "{node}: {error:?}");
            assert!(
                error.to_string().contains(".warlockignore"),
                "the failure names the file to go and fix: {error}"
            );
        }

        write(repo.path(), "crates/engine/.warlockignore", "notes/\n");
        hash(repo.path(), "crates/engine");
    }

    #[test]
    fn renaming_a_file_with_identical_bytes_changes_the_hash() {
        let repo = fixture();
        let before = hash(repo.path(), "crates/engine");

        fs::rename(
            repo.path().join("crates/engine/src/lib.rs"),
            repo.path().join("crates/engine/src/main.rs"),
        )
        .expect("renames");

        assert_ne!(
            hash(repo.path(), "crates/engine"),
            before,
            "the path is part of the input, not just the bytes"
        );
    }

    #[test]
    fn editing_the_nodes_own_document_changes_its_hash() {
        let repo = fixture();
        let before = hash(repo.path(), "crates/engine");
        let root_before = hash(repo.path(), "");

        write(
            repo.path(),
            "crates/engine/WARLOCK.md",
            "# engine\n\nHand-edited, which is exactly what section 9 expects.\n",
        );

        assert_ne!(hash(repo.path(), "crates/engine"), before);
        assert_ne!(hash(repo.path(), ""), root_before);
    }

    #[test]
    fn a_plain_readme_goes_into_the_digest_like_any_other_file() {
        let repo = fixture();
        let before = hash(repo.path(), "crates/tui");

        write(
            repo.path(),
            "crates/tui/README.md",
            "# for people, revised\n",
        );

        assert_ne!(
            hash(repo.path(), "crates/tui"),
            before,
            "a `README.md` is not Warlock's document and is not special to the \
             hash either: it is a file at or below the node, so it counts"
        );
    }

    #[test]
    fn a_file_added_or_deleted_changes_the_hash() {
        let repo = fixture();
        let before = hash(repo.path(), "crates/engine");

        write(repo.path(), "crates/engine/src/extra.rs", "");
        let with_extra = hash(repo.path(), "crates/engine");
        assert_ne!(with_extra, before, "an empty new file is still a new path");

        fs::remove_file(repo.path().join("crates/engine/src/extra.rs")).expect("removes");
        assert_eq!(hash(repo.path(), "crates/engine"), before);
    }

    #[test]
    fn a_directory_with_no_files_hashes_and_holds_no_bytes() {
        let empty = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(empty.path().join("a/b/c")).expect("creates empty directories");

        let other = tempfile::tempdir().expect("a second temporary directory");

        assert_eq!(
            subtree_hash(empty.path()).expect("hashes"),
            subtree_hash(other.path()).expect("hashes"),
            "directories are not part of the input, only files are"
        );
    }

    /// Only on unix, because the fixture needs `std::os::unix::fs::symlink` to
    /// build the cycle at all. What is under test — that the walk does not
    /// follow links, so hashing terminates — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_cycle_terminates() {
        let repo = fixture();
        let before = hash(repo.path(), "crates/engine");

        std::os::unix::fs::symlink(repo.path(), repo.path().join("crates/engine/up"))
            .expect("links back to the root");
        std::os::unix::fs::symlink(
            repo.path().join("crates"),
            repo.path().join("crates/engine/sideways"),
        )
        .expect("links back to an ancestor");

        assert_eq!(
            hash(repo.path(), "crates/engine"),
            before,
            "a symlink is neither followed nor hashed as its target, so the \
             hash terminates and is unchanged"
        );
    }

    /// Chmod cannot deny root anything, so the test checks the fixture really
    /// is unreadable before asserting on it and steps aside when it is not.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_an_error_not_a_hash() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = fixture();
        let path = repo.path().join("crates/engine/src/lib.rs");
        let good = hash(repo.path(), "crates/engine");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&path).is_ok() {
            // Running as root: there is no such thing as an unreadable file
            // here, so there is nothing to assert against.
            return;
        }

        let error = subtree_hash(repo.path().join("crates/engine"))
            .expect_err("a file that cannot be read has no hash");
        assert!(matches!(error, Error::Read { .. }), "{error:?}");
        assert!(error.to_string().contains("lib.rs"), "{error}");

        // The point of failing: neither skipping the file nor hashing the
        // error text may produce a digest, least of all one that could match a
        // grant.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmods back");
        assert_eq!(hash(repo.path(), "crates/engine"), good);

        let deleted = fixture();
        fs::remove_file(deleted.path().join("crates/engine/src/lib.rs")).expect("removes");
        assert_ne!(
            hash(deleted.path(), "crates/engine"),
            good,
            "an unreadable file must not hash like a deleted one — and it \
             cannot, because it does not hash at all"
        );
    }

    #[test]
    fn a_directory_that_vanished_is_an_error() {
        let repo = fixture();
        let gone = repo.path().join("crates/engine/src/deep");
        fs::remove_dir_all(&gone).expect("removes");

        let error = subtree_hash(&gone).expect_err("nothing there to hash");
        assert!(matches!(error, Error::Walk { .. }), "{error:?}");
        assert!(error.to_string().contains("could not walk"), "{error}");
    }

    #[test]
    fn every_error_variant_says_what_happened_and_where() {
        let read = Error::Read {
            path: Path::new("/repo/src/lib.rs").to_path_buf(),
            source: std::io::Error::other("boom"),
        };
        assert_eq!(
            read.to_string(),
            "could not read `/repo/src/lib.rs`, so the subtree has no hash: boom"
        );

        let named = Error::Path {
            path: Path::new("/elsewhere/x").to_path_buf(),
            source: Box::new(crate::ManifestError::NonUtf8Path {
                path: Path::new("/elsewhere/x").to_path_buf(),
            }),
        };
        assert!(
            named
                .to_string()
                .starts_with("could not name `/elsewhere/x`")
        );
    }
}
