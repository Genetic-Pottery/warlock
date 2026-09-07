use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::Described;
use crate::hash::length;
use crate::ignores;
use crate::languages;
use crate::pact::{DOCUMENT_FILE, Error, MANIFEST_DIR, Observer};
use crate::{Agent, agent, to_manifest_path};

#[derive(Debug)]
pub(crate) struct Fitted {
    pub(crate) request: agent::Request,
    pub(crate) problems: Vec<Problem>,
    pub(crate) described: Described,
}

pub(crate) fn fit(
    prompt: &str,
    directory: &Path,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Fitted, Error> {
    // Asked once, at the top, and carried down every rung: a budget that
    // changed under the ladder would let a file be given up against one number
    // and reported against another.
    let cap = request_byte_cap(agent.context_tokens());

    let Gathered {
        request,
        mut problems,
    } = gather_request(prompt, directory, cap)?;

    let mut described = Described::default();
    for file in request.files() {
        let text = file.kept().or_else(|| {
            file.bytes()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
        });
        if let Some(text) = text {
            let names = languages::declared_names(Path::new(file.path()), text);
            if !names.is_empty() {
                described.declared.insert(file.path().to_owned(), names);
            }
        }
    }

    let mut passes = Reducing {
        directory,
        described: &mut described,
    };
    let request = lift_over_cap(request, &mut problems, &mut passes);

    let request = demote_to_budget(request, cap, &mut problems, &mut passes);

    // Announced here rather than by the caller, and after the two reductions
    // above rather than before: this is the only point at which both numbers
    // are true of the request that actually goes out.
    observer.requesting(
        request.files().len(),
        carried_bytes(request.files(), request.child_documents()),
    );

    Ok(Fitted {
        request,
        problems,
        described,
    })
}
const WALK_DEPTH: usize = 2;

pub const PER_FILE_BYTE_CAP: u64 = 1024 * 1024;

const BYTES_PER_TOKEN_NUMERATOR: u64 = 5;

const BYTES_PER_TOKEN_DENOMINATOR: u64 = 2;

const RESERVED_TOKENS: u64 = 40_000;

const SAMPLE_BYTE_CAP: u64 = 6 * 1024;

const SAMPLE_LINE_CAP: usize = 60;

const MINIMUM_REQUEST_BYTES: u64 = 64 * 1024;

pub(crate) const fn request_byte_cap(context_tokens: u64) -> u64 {
    let budget = context_tokens
        .saturating_sub(RESERVED_TOKENS)
        .saturating_mul(BYTES_PER_TOKEN_NUMERATOR)
        / BYTES_PER_TOKEN_DENOMINATOR;
    if budget < MINIMUM_REQUEST_BYTES {
        MINIMUM_REQUEST_BYTES
    } else {
        budget
    }
}

#[cfg(test)]
pub(crate) const REQUEST_BYTE_CAP: u64 = request_byte_cap(crate::agent::DEFAULT_CONTEXT_TOKENS);

fn elided_or_whole(path: &Path, relative: String, size: u64, bytes: Vec<u8>) -> agent::File {
    let Ok(text) = str::from_utf8(&bytes) else {
        return agent::File::present(relative, bytes);
    };
    match languages::elide(path, text) {
        Some(elided) => agent::File::elided(relative, size, elided.text),
        None => agent::File::present(relative, bytes),
    }
}

fn trim_to_budget(
    files: &mut [agent::File],
    on_disk: &[PathBuf],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) {
    if carried <= cap {
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
        if carried <= cap {
            break;
        }
        // What the file costs the request, which is not what it costs the
        // disk: an elided file is carried as its surviving lines and reported
        // as its full size, and subtracting the second would credit the budget
        // with bytes that were never in it — leaving the loop convinced it had
        // met a cap it was still over.
        let spent = file_bytes(&files[index]);

        // A file given up here is not gone for good: it is on the cliff, and
        // `lift_from_the_cliff` buys an account of it back wherever one fits.
        // So the room that account will want is charged for now, at the moment
        // the file is given up, rather than discovered to be missing later.
        //
        // Without this the two steps work against each other. This one has no
        // agent and so has only one move, and it stops the instant the request
        // is inside the cap — leaving every byte of the budget claimed and the
        // lift nothing to spend. Measured on this workspace's own
        // `warlock-tui/src`: eleven files cliffed, 17KB of headroom left, and
        // seven of them — including the four largest and most important files
        // in the crate — reaching the pass as nothing but names, which is what
        // a document written by guessing is made of.
        //
        // Charging per file rather than reserving a flat share of the budget is
        // what keeps a directory that cliffs one file from paying for twenty.
        let Some(freed) = spent.checked_sub(SAMPLE_BYTE_CAP).filter(|&f| f > 0) else {
            // Its account would cost about what the file costs. Naming it would
            // buy nothing and lose its text, so it stays as it is and the next
            // file down is asked instead.
            continue;
        };

        let size = files[index].size();
        let path = files[index].path().to_owned();
        files[index] = sampled(&files[index]).unwrap_or_else(|| agent::File::omitted(path, size));
        carried = carried.saturating_sub(freed);
        problems.push(Problem {
            path: on_disk[index].clone(),
            cause: Omission::OverBudget { size },
        });
    }
}

fn lift_over_cap(
    request: agent::Request,
    problems: &mut Vec<Problem>,
    passes: &mut Reducing<'_>,
) -> agent::Request {
    let directory = passes.directory;
    let mut files = request.files().to_vec();
    // The problems whose files ended up described, so their entries can go. Held
    // rather than removed as they are found, because removing from under the
    // loop would move every index still to be matched.
    let mut settled = Vec::new();
    let mut replaced = false;

    for file in &mut files {
        if !file.is_omitted() {
            continue;
        }
        let on_disk = directory.join(file.path());
        let Some(index) = problems.iter().position(|problem| {
            matches!(problem.cause, Omission::TooLarge { .. }) && problem.path == on_disk
        }) else {
            continue;
        };

        // Read here rather than in `gather_request`, which measured this file
        // and deliberately never opened it: the bytes are only worth holding for
        // as long as the passes over them take.
        let bytes = match fs::read(&on_disk) {
            Ok(bytes) => bytes,
            // It was over the cap a moment ago and is unreadable now. Whatever
            // happened to it, the honest cause is the filesystem's, and it is
            // the same one gather reports for a file it could not read.
            Err(source) => {
                problems[index].cause = Omission::Unreadable { source };
                continue;
            }
        };

        match passes.skeleton_of(file.path(), &bytes) {
            Ok(skeleton) => {
                let (path, size) = (file.path().to_owned(), file.size());
                // The size on disk, not the length of the skeleton: a file is
                // as big as it is however little of it is worth sending.
                *file = agent::File::elided(path, size, skeleton);
                settled.push(index);
                replaced = true;
            }
            Err(cause) => {
                // The entry stays on the list even though a sample is about to
                // be attached: most of the file really is missing, and the
                // entry is the only thing that says so.
                problems[index].cause = cause;
                if let Ok(text) = std::str::from_utf8(&bytes)
                    && let Some(head) = sample_of(text)
                {
                    let (path, size) = (file.path().to_owned(), file.size());
                    *file = agent::File::elided(path, size, head);
                    replaced = true;
                }
            }
        }
    }

    // Sorted so the removals are back to front whatever order the files were
    // matched in, and so no earlier removal shifts a later index.
    settled.sort_unstable();
    for index in settled.into_iter().rev() {
        problems.remove(index);
    }

    if !replaced {
        return request;
    }

    // Rebuilt rather than mutated: `agent::Request`'s builders append, so there
    // is no way to exchange one file in place.
    agent::Request::new(request.prompt().to_owned(), directory)
        .with_files(files)
        .with_child_documents(request.child_documents().to_vec())
}

fn demote_to_budget(
    request: agent::Request,
    cap: u64,
    problems: &mut Vec<Problem>,
    passes: &mut Reducing<'_>,
) -> agent::Request {
    let directory = passes.directory;
    let carried = carried_bytes(request.files(), request.child_documents());
    let cliffed = problems
        .iter()
        .any(|problem| matches!(problem.cause, Omission::OverBudget { .. }));
    if carried <= cap && !cliffed {
        return request;
    }

    // The children's documents are never demoted and never dropped, so once
    // they alone are over the cap no rung below can buy room that exists. The
    // request goes out over the cap with its files intact rather than paying
    // for model passes that cannot help.
    let immovable = carried_bytes(&[], request.child_documents());
    if immovable > cap {
        return request;
    }

    let mut files = request.files().to_vec();
    // Computed once and walked three times: the rung a file stands on changes
    // under these loops, but which file is the biggest must not, or two runs of
    // the same pact could demote in different orders.
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&index| (Reverse(files[index].size()), files[index].path().to_owned()));

    let carried = demote_whole_files(passes, &mut files, &order, carried, cap, problems);
    let carried = lift_from_the_cliff(passes, &mut files, &order, carried, cap, problems);
    list_over_budget(directory, &mut files, &order, carried, cap, problems);

    if files == request.files() {
        return request;
    }

    agent::Request::new(request.prompt().to_owned(), directory)
        .with_files(files)
        .with_child_documents(request.child_documents().to_vec())
}

fn whole_bytes(directory: &Path, file: &agent::File) -> Option<Vec<u8>> {
    if let Some(bytes) = file.bytes() {
        return Some(bytes.to_vec());
    }
    file.kept()?;
    fs::read(directory.join(file.path())).ok()
}

fn demote_whole_files(
    passes: &mut Reducing<'_>,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) -> u64 {
    let mut carried = carried;
    for &index in order {
        if carried <= cap {
            break;
        }
        let spent = file_bytes(&files[index]);
        if spent == 0 {
            continue;
        }
        let Some(bytes) = whole_bytes(passes.directory, &files[index]) else {
            continue;
        };
        let (path, size) = (files[index].path().to_owned(), files[index].size());

        match passes.skeleton_of(&path, &bytes) {
            Ok(skeleton) if byte_count(skeleton.len()) < spent => {
                carried = carried
                    .saturating_sub(spent)
                    .saturating_add(byte_count(skeleton.len()));
                files[index] = agent::File::elided(path, size, skeleton);
            }
            Ok(_) => {}
            Err(cause) => {
                if let Some(sample) = sampled(&files[index]) {
                    carried = carried
                        .saturating_sub(spent)
                        .saturating_add(file_bytes(&sample));
                    files[index] = sample;
                } else {
                    carried = carried.saturating_sub(spent);
                    files[index] = agent::File::omitted(path.clone(), size);
                }
                report(problems, passes.directory.join(path.clone()), cause);
            }
        }
    }
    carried
}

fn lift_from_the_cliff(
    passes: &mut Reducing<'_>,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) -> u64 {
    let mut carried = carried;
    for &index in order {
        if carried > cap {
            break;
        }
        if !files[index].is_omitted() {
            continue;
        }
        let on_disk = passes.directory.join(files[index].path());
        let Some(reported) = problems.iter().position(|problem| {
            matches!(problem.cause, Omission::OverBudget { .. }) && problem.path == on_disk
        }) else {
            continue;
        };

        let bytes = match fs::read(&on_disk) {
            Ok(bytes) => bytes,
            Err(source) => {
                problems[reported].cause = Omission::Unreadable { source };
                continue;
            }
        };

        match passes.skeleton_of(files[index].path(), &bytes) {
            Ok(skeleton) => {
                let length = byte_count(skeleton.len());
                if carried.saturating_add(length) > cap {
                    break;
                }
                let (path, size) = (files[index].path().to_owned(), files[index].size());
                files[index] = agent::File::elided(path, size, skeleton);
                problems.remove(reported);
                carried = carried.saturating_add(length);
            }
            Err(cause) => problems[reported].cause = cause,
        }
    }
    carried
}

fn list_over_budget(
    directory: &Path,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) {
    let mut carried = carried;
    for &index in order {
        if carried <= cap {
            break;
        }
        let spent = file_bytes(&files[index]);
        if spent == 0 {
            continue;
        }
        let (path, size) = (files[index].path().to_owned(), files[index].size());
        // The last rung is a sample rather than a name: a file the cap cannot
        // afford whole is still one whose first lines cost almost nothing. Only
        // text that cannot be sampled at all falls the rest of the way.
        if let Some(sample) = sampled(&files[index]).filter(|s| file_bytes(s) < spent) {
            carried = carried
                .saturating_sub(spent)
                .saturating_add(file_bytes(&sample));
            files[index] = sample;
            report(
                problems,
                directory.join(path),
                Omission::OverBudget { size },
            );
        } else {
            files[index] = agent::File::omitted(path.clone(), size);
            carried = carried.saturating_sub(spent);
            report(
                problems,
                directory.join(path),
                Omission::OverBudget { size },
            );
        }
    }
}

fn report(problems: &mut Vec<Problem>, path: PathBuf, cause: Omission) {
    match problems.iter().position(|problem| problem.path == path) {
        Some(index) => problems[index].cause = cause,
        None => problems.push(Problem { path, cause }),
    }
}

struct Reducing<'a> {
    directory: &'a Path,
    described: &'a mut Described,
}

impl Reducing<'_> {
    fn skeleton_of(&mut self, path: &str, bytes: &[u8]) -> Result<String, Omission> {
        let size = byte_count(bytes.len());
        let text =
            std::str::from_utf8(bytes).map_err(|source| Omission::NotText { size, source })?;
        let on_disk = Path::new(path);
        let skeleton = languages::skeleton(on_disk, text).ok_or(Omission::Unreducible { size })?;

        let declared = languages::declared_names(on_disk, text);
        if !declared.is_empty() {
            self.described.declared.insert(path.to_owned(), declared);
        }
        Ok(skeleton.text)
    }
}

#[derive(Debug)]
struct Found {
    files: BTreeMap<String, PathBuf>,
    child_documents: BTreeMap<String, PathBuf>,
}

pub(crate) fn gather_request(
    prompt: impl Into<String>,
    directory: impl AsRef<Path>,
    cap: u64,
) -> Result<Gathered, Error> {
    let directory = directory.as_ref();
    let found = walk(directory)?;

    let mut problems = Vec::new();

    // Children first: their documents are part of the budget the files are then
    // fitted into, and they are the part that never gives way.
    let mut child_documents = Vec::new();
    for (child, path) in found.child_documents {
        match fs::read_to_string(&path) {
            Ok(text) => {
                child_documents.push(agent::ChildDocument::new(child, text));
            }
            Err(source) => problems.push(Problem {
                path,
                cause: Omission::Unreadable { source },
            }),
        }
    }

    // The size comes from the filesystem before anything is opened, so an
    // enormous file is never read into memory just to be dropped again.
    let mut files = Vec::new();
    let mut on_disk = Vec::new();
    for (relative, path) in found.files {
        let size = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(source) => {
                problems.push(Problem {
                    path,
                    cause: Omission::Unreadable { source },
                });
                continue;
            }
        };

        // The per-file cap, but never above the budget for the whole request.
        // The two used to disagree: 1 MiB against a 400 KB request meant a
        // 900 KB file passed this gate, was read whole, and then single-handedly
        // blew the budget by more than twice over — guaranteeing a demotion the
        // read had already been paid for. A gate that admits what the next gate
        // must reject is not a gate, so this one is clamped to it.
        let file = if size > PER_FILE_BYTE_CAP.min(cap) {
            problems.push(Problem {
                path: path.clone(),
                cause: Omission::TooLarge { size },
            });
            agent::File::omitted(relative, size)
        } else {
            match fs::read(&path) {
                Ok(bytes) => elided_or_whole(&path, relative, size, bytes),
                Err(source) => {
                    problems.push(Problem {
                        path: path.clone(),
                        cause: Omission::Unreadable { source },
                    });
                    agent::File::omitted(relative, size)
                }
            }
        };
        files.push(file);
        on_disk.push(path);
    }

    // Counted from what was gathered rather than added up as the loops went:
    // what a file spends is a property of the file that ended up in the
    // request, not of the branch it came out of.
    let carried = carried_bytes(&files, &child_documents);
    trim_to_budget(&mut files, &on_disk, carried, cap, &mut problems);

    let request = agent::Request::new(prompt, directory)
        .with_files(files)
        .with_child_documents(child_documents);

    Ok(Gathered { request, problems })
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

fn sample_of(text: &str) -> Option<String> {
    let mut head = String::new();
    let mut taken = 0usize;
    for line in text.lines().take(SAMPLE_LINE_CAP) {
        // An over-long line is cut on a character boundary rather than dropped:
        // a minified bundle is one line, and dropping it would sample nothing.
        let cap = usize::try_from(SAMPLE_BYTE_CAP).unwrap_or(usize::MAX);
        let room = cap.saturating_sub(head.len());
        if room == 0 {
            break;
        }
        let cut = if line.len() <= room {
            line
        } else {
            let mut end = room;
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            &line[..end]
        };
        head.push_str(cut);
        head.push('\n');
        taken += 1;
        if head.len() >= cap {
            break;
        }
    }

    let total = text.lines().count();
    if taken >= total && head.len() >= text.len() {
        return None;
    }
    if head.is_empty() {
        return None;
    }
    // Whichever unit is true: bytes where the file was one long line and what
    // happened to it was a cut. "0 further lines" about a minified bundle would
    // be the truncation this marker exists to rule out.
    let left = total.saturating_sub(taken);
    if left > 0 {
        let _ = write!(head, "… {left} further lines not shown …");
    } else {
        let bytes = text.len().saturating_sub(head.len());
        let _ = write!(head, "… {bytes} further bytes not shown …");
    }
    Some(head)
}

fn sampled(file: &agent::File) -> Option<agent::File> {
    let owned;
    let text = if let Some(bytes) = file.bytes() {
        owned = std::str::from_utf8(bytes).ok()?;
        owned
    } else {
        file.kept()?
    };
    let head = sample_of(text)?;
    Some(agent::File::elided(
        file.path().to_owned(),
        file.size(),
        head,
    ))
}

fn file_bytes(file: &agent::File) -> u64 {
    if let Some(bytes) = file.bytes() {
        return byte_count(bytes.len());
    }
    if let Some(kept) = file.kept() {
        return byte_count(kept.len());
    }
    if let Some(summary) = file.summary() {
        return byte_count(summary.len());
    }
    0
}

pub(crate) fn carried_bytes(
    files: &[agent::File],
    child_documents: &[agent::ChildDocument],
) -> u64 {
    let mut carried: u64 = 0;
    for file in files {
        carried = carried.saturating_add(file_bytes(file));
    }
    for child in child_documents {
        carried = carried.saturating_add(byte_count(child.text().len()));
    }
    carried
}

#[derive(Debug)]
pub(crate) struct Gathered {
    pub(crate) request: agent::Request,
    pub(crate) problems: Vec<Problem>,
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

    use super::{
        BYTES_PER_TOKEN_DENOMINATOR, BYTES_PER_TOKEN_NUMERATOR, Gathered, MINIMUM_REQUEST_BYTES,
        Omission, PER_FILE_BYTE_CAP, Problem, REQUEST_BYTE_CAP, RESERVED_TOKENS, SAMPLE_BYTE_CAP,
        carried_bytes, gather_request, request_byte_cap,
    };

    use crate::agent;
    use crate::pact::DOCUMENT_FILE;
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    fn filler(size: u64) -> Vec<u8> {
        vec![b'x'; usize::try_from(size).expect("a test file fits in memory")]
    }

    fn share(percent: u64) -> u64 {
        REQUEST_BYTE_CAP * percent / 100
    }

    const AMPLE_CAP: u64 = PER_FILE_BYTE_CAP * 4;

    fn request_for(dir: &Path) -> agent::Request {
        let Gathered { request, problems } =
            gather_request("summarise", dir, AMPLE_CAP).expect("gathers");
        assert!(problems.is_empty(), "{problems:?}");
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
        let request = request_for(dir.path());
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
        let request = request_for(dir.path());
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
        let request = request_for(dir.path());
        let file = file(&request, "boot.asm");

        assert_eq!(
            file.bytes(),
            Some(source.as_bytes()),
            "an unknown language is sent whole and untouched"
        );
        assert_eq!(file.kept(), None, "and nothing claims to have elided it");
    }

    #[test]
    fn the_request_budget_follows_the_window_it_will_be_read_in() {
        // The defect this replaced: a fixed 2 MiB cap sat above every window it
        // was meant to respect, so nothing was ever demoted and the overflow
        // was left to whatever the transport does about it.
        let small = request_byte_cap(200_000);
        let large = request_byte_cap(1_000_000);
        assert!(
            small < large,
            "a bigger window buys a bigger request: {small} vs {large}"
        );
        assert!(
            small < 200_000 * BYTES_PER_TOKEN_NUMERATOR / BYTES_PER_TOKEN_DENOMINATOR,
            "and the prompt, the children's documents and the answer are kept back"
        );
    }

    #[test]
    fn trimming_an_elided_file_credits_the_budget_only_what_it_carried() {
        // The bug this pins: `trim_to_budget` demoted a file and then credited
        // the budget with the file's size *on disk*. That is right for a file
        // sent whole and wrong for an elided one, which is carried as its
        // surviving lines and reported as its full size — so the loop
        // over-counted what it had recovered, stopped early, and handed back a
        // request still over the cap while believing it had met it. Silently
        // over the cap is the one outcome the whole ladder exists to prevent.
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Each file is real code that survives elision plus a test module that
        // does not, so after eliding they are still collectively over the cap
        // and trimming has to run — which is the only situation in which the
        // miscounting could bite.
        for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            let code = (0..400).fold(String::new(), |mut code, n| {
                use std::fmt::Write as _;
                let _ = write!(code, "pub fn work_{n}() -> u32 {{\n    {n}\n}}\n\n");
                code
            });
            let body = "        let filler = 1;\n".repeat(2_000);
            write(
                dir.path(),
                name,
                format!(
                    "{code}#[cfg(test)]\nmod tests {{\n    #[test]\n    \
                     fn it_works() {{\n{body}    }}\n}}\n"
                ),
            );
        }

        let cap = 40_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        let carried = carried_bytes(request.files(), request.child_documents());
        assert!(
            carried <= cap,
            "the request really is inside the cap it reports meeting: {carried} > {cap}"
        );
        assert!(
            !problems.is_empty(),
            "and it said out loud what it gave up to get there"
        );
    }

    #[test]
    fn a_file_whose_account_would_cost_more_than_it_frees_is_not_given_up() {
        // Naming a file frees what it was carrying and commits the lift to
        // buying an account back, so for a small file the trade loses before it
        // starts. It also used to be how the biggest files in a directory ended
        // as bare names: the trim stopped the instant it was inside the cap,
        // leaving the lift nothing to spend, and `warlock-tui/src` reached its
        // pass with `app.rs`, `ui.rs`, `pacting.rs` and `claude.rs` as names
        // alone. Charging each cliff for the account it will want is what fixed
        // that, and this is the other end of the same rule.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "small.txt", filler(512));
        write(dir.path(), "big.txt", filler(200_000));

        let cap = 100_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        assert!(
            file(&request, "big.txt").is_omitted(),
            "the file whose account is worth buying is given up",
        );
        assert!(
            !file(&request, "small.txt").is_omitted(),
            "the file whose account would cost more than it frees is kept whole",
        );
        assert_eq!(
            problems.len(),
            1,
            "and only the real trade is reported: {problems:?}"
        );
    }

    #[test]
    fn the_trim_leaves_room_for_the_accounts_the_lift_will_buy() {
        // The defect this pins, measured on `warlock-tui/src`: the trim met the
        // cap exactly, so 17KB of headroom stood against eleven cliffed files
        // and seven of them stayed bare names all the way to the pass. A
        // document written about a file nobody could read is a document written
        // by guessing.
        let dir = tempfile::tempdir().expect("a temporary directory");
        for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            write(dir.path(), name, filler(60_000));
        }

        let cap = 100_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        let carried = carried_bytes(request.files(), request.child_documents());
        let cliffed = problems.len() as u64;
        assert!(cliffed > 0, "the fixture is over the cap");
        assert!(
            cap - carried >= cliffed * SAMPLE_BYTE_CAP,
            "every cliffed file has room left for an account of it: {} spare for \
             {cliffed} files",
            cap - carried,
        );
    }

    #[test]
    fn the_budget_fits_the_window_at_the_measured_density_of_real_code() {
        // The arithmetic nobody did the first time, written down so a future
        // tightening of these constants has to argue with a measurement.
        //
        // Measured by sending two files of different sizes through the model
        // and dividing the difference in reported input tokens by the
        // difference in bytes: Rust 2.62 bytes per token, TypeScript 2.60. Also
        // measured, and the item most easily forgotten: about 16,700 tokens of
        // the agent's own system prompt and tool definitions sit in the window
        // before this request contributes anything.
        const MEASURED_BYTES_PER_TOKEN: u64 = 26; // tenths, the denser of the two
        const AGENT_OVERHEAD_TOKENS: u64 = 16_700;
        const WINDOW: u64 = 200_000;

        let cap = request_byte_cap(WINDOW);
        let content_tokens = cap * 10 / MEASURED_BYTES_PER_TOKEN;
        let used = content_tokens + AGENT_OVERHEAD_TOKENS;

        assert!(
            used < WINDOW,
            "a full request has to fit the window it is read in: {cap} bytes is \
             {content_tokens} tokens of code, plus {AGENT_OVERHEAD_TOKENS} of \
             agent overhead, against a {WINDOW}-token window",
        );
        // And enough left for the answer, which is the other thing in there.
        assert!(
            WINDOW - used >= 12_000,
            "only {} tokens left for the document itself",
            WINDOW - used,
        );
    }

    #[test]
    fn an_implausible_window_still_yields_a_usable_budget() {
        // No budget may fail a fit, this module's oldest rule: an agent that
        // reports a window smaller than the room reserved gets a poor request
        // rather than an impossible one.
        assert_eq!(request_byte_cap(0), MINIMUM_REQUEST_BYTES);
        assert_eq!(request_byte_cap(RESERVED_TOKENS), MINIMUM_REQUEST_BYTES);
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

    const FAT_SHARES: [(&str, u64); 5] = [
        ("a.bin", 31),
        ("b.bin", 35),
        ("c.bin", 39),
        ("d.bin", 43),
        ("e.bin", 47),
    ];

    fn fat() -> [(&'static str, u64); 5] {
        FAT_SHARES.map(|(name, percent)| (name, share(percent)))
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
                .map(agent::ChildDocument::directory)
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
        // And the repository's own exclusions, which is the same walk again.
        // `sketches.md` is small enough to be sent whole and `logo.png` is far
        // too big for the per-file cap, so a rule the walk failed to read would
        // show up twice over: as a file in the request, and as the `Problem`
        // that says the big one was listed rather than sent.
        write(dir.path(), ".warlockignore", "sketches.md\nassets/\n");
        write(dir.path(), "sketches.md", "# the author's notebook\n");
        write(dir.path(), "assets/logo.png", filler(PER_FILE_BYTE_CAP + 1));
        write(dir.path(), "assets/WARLOCK.md", "# not a module at all\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "gitignored, hidden, `.warlock/` and `.warlockignore`d files come \
             through the same walk as everything else, so they never arrive at \
             all"
        );
        assert!(
            request.child_documents().is_empty(),
            "and a document inside an ignored, excluded or pruned directory is \
             not a child document: {:?}",
            request.child_documents(),
        );
        assert!(
            problems.is_empty(),
            "excluded content is not in the request and is not reported \
             missing from it either — it is no part of what Warlock covers: \
             {problems:?}",
        );
    }

    #[test]
    fn rules_the_request_walk_cannot_parse_fail_the_gather_and_name_the_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        // A range that runs backwards: a glob the matcher will not compile.
        write(dir.path(), ".warlockignore", "a[z-a]\n");

        let error = gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
            .expect_err("rules that cannot be read are not no rules");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(
            error.to_string().contains(".warlockignore"),
            "the failure names the file to go and fix: {error}"
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

        let request = request_for(dir.path());

        assert_eq!(file_paths(&request), ["lib.rs"]);
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn prose_reaches_no_pass_and_is_in_no_document() {
        // The defect this closes, measured on this repository: a route's
        // symbol is checked by looking for it in the target's text, so a
        // readme sentence naming `action_for` verified a route to the readme
        // while the function itself sat in another directory. Prose cannot be
        // told from a declaration by a substring test, so prose does not
        // travel.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        write(
            dir.path(),
            "README.md",
            "Every binding is in `action_for`.\n",
        );
        write(
            dir.path(),
            "CLAUDE.md",
            "Read the WARLOCK.md files first.\n",
        );
        write(dir.path(), "NOTES.MARKDOWN", "# notes\n");
        write(dir.path(), "guide.mdx", "# guide\n");
        write(dir.path(), DOCUMENT_FILE, "# engine\n\nWhat it was.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "every markdown file is out, whatever its case or spelling, and the \
             directory's own document with them",
        );
        assert!(
            problems.is_empty(),
            "and dropping prose is the ordinary case, not something to report: {problems:?}",
        );
    }

    #[test]
    fn a_directory_of_nothing_but_prose_gathers_no_files_and_still_fits() {
        // A workspace's parent directory, near enough: a readme, a licence in
        // markdown, and one child that has described itself. What is left to
        // route with is the child's document, which is the point.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "README.md", "The pitch.\n");
        write(dir.path(), "CONTRIBUTING.md", "How to help.\n");
        write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert!(
            file_paths(&request).is_empty(),
            "{:?}",
            file_paths(&request)
        );
        assert_eq!(
            request.child_documents().len(),
            1,
            "the child's document is the one prose warlock wrote itself, and it stays",
        );
        assert_eq!(request.child_documents()[0].directory(), "src");
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn an_existing_document_reaches_no_pass_at_all() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "WARLOCK.md", "# engine\n\nWhat it was.\n");
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "the directory's own document is not one of its files: it is the \
             previous pass's claim about the directory, and no pass is shown \
             its predecessor",
        );
        assert!(
            request
                .child_documents()
                .iter()
                .all(|child| child.directory() != "." && child.directory() != "WARLOCK.md"),
            "and it is nobody's child document: {:?}",
            request.child_documents(),
        );
        assert!(
            problems.is_empty(),
            "and dropping it is the ordinary case, not something to report: {problems:?}"
        );
        assert!(
            !request.prompt().contains("What it was"),
            "nor does it reach the pass by any other road"
        );
    }

    #[test]
    fn an_enormous_existing_document_costs_the_budget_nothing() {
        // A document is dropped before it is measured, so a hand-edited or
        // generated accident of any size leaves the files' whole budget to the
        // files.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), DOCUMENT_FILE, filler(PER_FILE_BYTE_CAP * 3));
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP).expect("gathers");

        assert_eq!(file_paths(&request), ["lib.rs"]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            carried_bytes(request.files(), request.child_documents()),
            "//! Core engine.\n".len() as u64
        );
    }

    #[test]
    fn a_file_over_the_per_file_cap_is_listed_by_name_and_size_and_reported() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let size = PER_FILE_BYTE_CAP + 1;
        let lock = write(dir.path(), "Cargo.lock", filler(size));
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("a huge file is not fatal");

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
    fn a_sample_is_the_files_own_lines_and_says_what_it_left_off() {
        let lines: String = (0..500)
            .map(|n: u32| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let sample = super::sample_of(&lines).expect("a long file is sampled");

        for line in sample.lines().filter(|line| !line.starts_with('…')) {
            assert!(
                lines.lines().any(|original| original == line),
                "every line is one the file really has: {line}",
            );
        }
        assert!(
            sample.starts_with("0\n1\n2\n"),
            "and they are its first lines, in order: {sample}",
        );
        assert!(
            sample.contains("further lines not shown"),
            "with a marker saying what was left off, which is what makes it a \
             sample rather than a truncation: {sample}",
        );
        assert!(
            super::byte_count(sample.len()) <= SAMPLE_BYTE_CAP + 64,
            "inside the cap the trim reserves for it",
        );
    }

    #[test]
    fn one_enormous_line_is_cut_and_reports_bytes_rather_than_lines() {
        // A minified bundle is one line. Lines are the wrong unit to report
        // what was left off it, and "0 further lines" would be a truncation
        // dressed as a sample.
        let minified = "x".repeat(200_000);
        let sample = super::sample_of(&minified).expect("one long line is still sampled");

        assert!(
            sample.contains("further bytes not shown"),
            "the unit has to be the one that is true: {}",
            &sample[sample.len().saturating_sub(60)..],
        );
        assert!(
            !sample.contains("0 further"),
            "and it must not claim nothing was left off",
        );
        assert!(super::byte_count(sample.len()) <= SAMPLE_BYTE_CAP + 64);
    }

    #[test]
    fn a_file_that_already_fits_is_not_swapped_for_a_copy_of_itself() {
        assert!(
            super::sample_of("fn small() {}\n").is_none(),
            "there is no sample to take of a file that is already whole",
        );
    }

    #[test]
    fn bytes_that_are_not_text_are_the_one_thing_still_reduced_to_a_name() {
        let not_text = agent::File::present("logo.png", vec![0xff, 0xfe, 0x00, 0x01]);
        assert!(
            super::sampled(&not_text).is_none(),
            "there is no honest sample of bytes with no lines to cut on",
        );
    }

    #[test]
    fn a_directory_over_the_request_cap_gives_up_its_largest_files_first() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Named so that alphabetical order is the reverse of size order: a
        // gather that dropped files in path order would fail here.
        let sizes = fat();
        for (name, size) in sizes {
            write(dir.path(), name, filler(size));
        }

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("a fat directory is not fatal");

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
                .map(agent::File::path)
                .collect::<Vec<_>>(),
            ["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"],
            "not one of them is reduced to a bare name: the biggest are still \
             given up first, but what they are given up to is a sample of \
             their own text",
        );
        assert_eq!(
            request
                .files()
                .iter()
                .filter(|file| file.kept().is_some())
                .map(agent::File::path)
                .collect::<Vec<_>>(),
            ["c.bin", "d.bin", "e.bin"],
            "and the three given up are exactly the three carrying a sample",
        );
        for name in ["c.bin", "d.bin", "e.bin"] {
            let kept = file(&request, name).kept().expect("sampled");
            assert!(
                kept.contains("not shown"),
                "a sample says what it left off, or it is a truncation: {kept}",
            );
            assert!(
                super::byte_count(kept.len()) <= SAMPLE_BYTE_CAP + 64,
                "and it stays inside the room the trim reserved for it",
            );
        }
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
        assert!(
            problems.iter().all(|problem| {
                let name = problem.path.file_name().expect("a file name");
                file(&request, &name.to_string_lossy()).kept().is_some()
            }),
            "a file the budget gave up is still reported — most of it really \
             is missing — but it is reported alongside a sample of itself \
             rather than instead of one",
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

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("an enormous child document is not fatal either");

        assert_eq!(
            request.child_documents().len(),
            1,
            "the account of a whole subtree is the one thing that never gives \
             way: dropping it would leave nothing in its place",
        );
        // And the file does not give way either, which is the newer half of
        // this. Naming a 1KB file frees 1KB and commits the lift to buying an
        // account that costs several times that, so the trade is a loss before
        // it starts — and here it cannot even help, since the child document
        // alone is already the whole budget. Giving it up would cost its text
        // and buy nothing at all.
        assert!(
            !file(&request, "lib.rs").is_omitted(),
            "a file too small for the trade to pay keeps its contents",
        );
        assert!(
            problems.is_empty(),
            "and nothing is reported as given up, because nothing was: {problems:?}",
        );
        assert!(
            carried_bytes(request.files(), request.child_documents()) > REQUEST_BYTE_CAP,
            "the request stays honestly over the cap, which this module allows",
        );
    }

    #[test]
    fn the_budget_counts_a_summarised_file_as_its_summary_and_never_its_size() {
        // Nothing gathered from a directory can be summarised yet, so the
        // accounting is exercised where it is decided: on a request built by
        // hand carrying one of each of the three states.
        let summary = "The locked dependency graph: 412 crates, no path dependencies.";
        let request = agent::Request::new("summarise", "/repo")
            .with_files([
                agent::File::present("lib.rs", *b"//! Core engine.\n"),
                agent::File::omitted("vendor.js", 900_000),
                agent::File::summarised("Cargo.lock", 4_200_000, summary),
            ])
            .with_child_documents([agent::ChildDocument::new("src", "# src\n")]);

        assert_eq!(
            super::carried_bytes(request.files(), request.child_documents()),
            (b"//! Core engine.\n".len() + summary.len() + "# src\n".len()) as u64,
            "sent whole costs its bytes, listed costs nothing, and summarised \
             costs its summary — the way the child document beside it does",
        );
        assert_eq!(
            super::carried_bytes(
                &[agent::File::summarised("Cargo.lock", 4_200_000, summary)],
                &[]
            ),
            summary.len() as u64,
            "the account travels; the 4.2 MB it stands for never does",
        );
        assert_eq!(
            carried(&request),
            super::carried_bytes(request.files(), request.child_documents()),
            "and the tests' own count of what a request carries agrees",
        );
    }

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
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("an unreadable file is not fatal");

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

        let error = gather_request("summarise", dir.path().join("nowhere"), REQUEST_BYTE_CAP)
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
