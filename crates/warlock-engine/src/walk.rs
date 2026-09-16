use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{ignores, manifest, to_manifest_path};

pub const DOCUMENT_FILE: &str = "WARLOCK.md";

pub(crate) const MANIFEST_DIR: &str = ".warlock";

const OWN_DEPTH: usize = 2;

const PROSE_EXTENSIONS: &[&str] = &["md", "markdown", "mdx"];

// What a tree shows, what a hash judges and what a pact covers are the same
// content only while every walk agrees on these, which is why no caller builds
// a walker of its own. A symlinked directory walked as a symlink is never
// descended into, so a cycle of them terminates. A fixture with a `.gitignore`
// and no `.git` still has to be ignored. `.warlock/` is left out by name rather
// than by the hidden-file rule, so it stays out even if it holds a document and
// even if hidden directories are ever let back in.
pub(crate) fn listing(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR));
    builder
}

// `listing` with `.warlockignore` honoured. The loader is the one caller that
// wants the listing without it, because it draws excluded directories rather
// than hiding them.
pub(crate) fn honouring_ignores(root: &Path) -> WalkBuilder {
    let mut builder = listing(root);
    builder.add_custom_ignore_filename(ignores::FILENAME);
    builder
}

// Every file at or below `dir`, keyed by its path relative to `dir`. A BTreeMap
// because the key order is the hash order: whatever sequence the walker produced
// is thrown away, which keeps a digest independent of the filesystem.
pub(crate) fn subtree_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>, Error> {
    // A walker never applies the rules to the root it was handed, so a directory
    // the repository excluded and then selected directly would otherwise hand
    // back the very content it asked warlock to keep out.
    if ignores::is_ignored(dir).map_err(Error::Walk)? {
        return Ok(BTreeMap::new());
    }

    let mut files = BTreeMap::new();
    for entry in honouring_ignores(dir).build() {
        let entry = usable(entry)?;
        // With `follow_links(false)` a symlink reports as a symlink, so this
        // drops it: its target is already here if it is inside the subtree, and
        // is not the subtree's content if it is not.
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();
        files.insert(relative(dir, &path)?, path);
    }
    Ok(files)
}

// Children before parents. A parent's request carries its children's documents,
// so pacting a parent first would hand the pass a stale account of the subtree.
// No "already has a document" or "has source in it" filter on top: an
// undocumented directory is exactly the one a pact exists to give a document to.
pub(crate) fn pactable_directories(root: &Path) -> Result<Vec<PathBuf>, Error> {
    if ignores::is_ignored(root).map_err(Error::Walk)? {
        return Ok(Vec::new());
    }

    // A set, so whatever order the walker offered is discarded rather than
    // reversed: every descendant sorts after its own ancestor, and the `.rev()`
    // below is the whole of the ordering.
    let mut directories = BTreeSet::new();
    for entry in honouring_ignores(root).build() {
        let entry = usable(entry)?;
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            directories.insert(entry.into_path());
        }
    }
    Ok(directories.into_iter().rev().collect())
}

#[derive(Debug, Default)]
pub(crate) struct Own {
    pub(crate) files: BTreeMap<String, PathBuf>,
    pub(crate) child_documents: BTreeMap<String, PathBuf>,
}

// The files a directory's document holds a line for, and the documents of the
// directories immediately below it. Prose is left out of `files`, and that takes
// the directory's own `WARLOCK.md` with it: a previous pass's claim about this
// directory is not evidence about it.
pub(crate) fn own(dir: &Path) -> Result<Own, Error> {
    let mut found = Own::default();
    for entry in honouring_ignores(dir).max_depth(Some(OWN_DEPTH)).build() {
        let entry = usable(entry)?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let depth = entry.depth();
        let path = entry.into_path();

        if depth == 1 && !is_prose(&path) {
            found.files.insert(relative(dir, &path)?, path);
        } else if depth == OWN_DEPTH && path.file_name() == Some(OsStr::new(DOCUMENT_FILE)) {
            let Some(child) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            found.child_documents.insert(relative(dir, &child)?, path);
        }
    }
    Ok(found)
}

// A rule file the walker could not use is reported beside its directory rather
// than in place of it, and taking that as "no rules" would let the content the
// repository excluded through.
fn usable(entry: Result<ignore::DirEntry, ignore::Error>) -> Result<ignore::DirEntry, Error> {
    let entry = entry.map_err(Error::Walk)?;
    if let Some(source) = entry.error() {
        return Err(Error::Walk(source.clone()));
    }
    Ok(entry)
}

fn relative(dir: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(dir, path).map_err(|source| Error::Path {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

fn is_prose(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROSE_EXTENSIONS
                .iter()
                .any(|prose| extension.eq_ignore_ascii_case(prose))
        })
}

#[derive(Debug)]
pub(crate) enum Error {
    Walk(ignore::Error),
    Path {
        path: PathBuf,
        source: Box<manifest::Error>,
    },
}
