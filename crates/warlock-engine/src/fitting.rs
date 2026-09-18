use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::document::{self, Described};
use crate::{agent, hash, languages, walk};

pub const PER_FILE_BYTE_CAP: u64 = 1024 * 1024;

// Comments come off before anything else, and they come off for the pass and
// not just for the check.
//
// Cutting them out of the evidence only was half a measure, and the half that
// remained is a loop: the pass reads a comment, believes it, writes the claim
// into a line, the line is refused, and the pass writes the same claim in a
// shape no name check can see. `engine/core` did exactly that within one run —
// refused for `Decoder::decode()`, re-asked, and back came "VAULT_LIMIT = 512
// applied post-decode", the same invention with the names filed off. There is
// no narrowing that closes it, because the input was wrong rather than the
// output.
//
// What is lost is real and was weighed: a pass shown a comment sometimes
// catches one that is wrong about its own code and says so. That clause costs
// `ENTRY_CHARS` that a document has for routing, and a reader who wants to know
// why a file exists is a model that will infer it from the code — so what
// letting comments in reliably buys is somebody's stale prose, presented to the
// next reader as fact.
//
// A file whose comments came off is sent stripped even where `elide` found
// nothing to drop, which is why the `None` arm no longer returns the original
// bytes. An extension with no comment form is still sent exactly as it is.
fn elided_or_whole(path: &Path, relative: String, size: u64, bytes: Vec<u8>) -> agent::File {
    let Ok(text) = str::from_utf8(&bytes) else {
        return agent::File::present(relative, bytes);
    };
    let stripped = languages::without_comments(path, text);
    let source = stripped.as_deref().unwrap_or(text);
    match languages::elide(path, source) {
        Some(elided) => agent::File::elided(relative, size, elided.text),
        None => match stripped {
            Some(stripped) => agent::File::present(relative, stripped.into_bytes()),
            None => agent::File::present(relative, bytes),
        },
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
                        describe(&mut described, &path, &name, text);
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

        let children: Vec<agent::ChildDocument> = own
            .child_documents
            .into_iter()
            .filter_map(|(child, path)| {
                let text = fs::read_to_string(&path).ok()?;
                Some(agent::ChildDocument::new(child, text))
            })
            .collect();

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

    fn expected(&self) -> document::Expected<'_> {
        document::Expected::of(&self.request)
    }

    pub(crate) fn synthesis_request(
        &self,
        lines: &BTreeMap<String, String>,
        rejected: &[document::Defect],
    ) -> agent::Request {
        let instructions =
            document::synthesis_instructions(&self.name, lines, &self.expected(), rejected);
        self.request.clone().with_prompt(instructions)
    }

    pub(crate) fn accept_synthesis(
        &self,
        answer: &str,
        lines: &BTreeMap<String, String>,
    ) -> document::Accepted {
        document::accept_synthesis(answer, lines, &self.expected(), &self.described)
    }

    pub(crate) fn mend(&self, fill: &document::Fill) -> (document::Fill, Vec<document::Mend>) {
        document::mend(fill, &self.expected(), &self.described)
    }

    pub(crate) fn render(&self, fill: &document::Fill) -> String {
        document::render(&self.name, fill, &self.expected(), &self.described)
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

fn describe(described: &mut Described, path: &Path, name: &str, text: &str) {
    let names = languages::declared_names(path, text);
    if !names.is_empty() {
        described.declared.insert(name.to_owned(), names);
    }
    let evidence = languages::without_comments(path, text);
    described.tokens.insert(
        name.to_owned(),
        tokens_of(evidence.as_deref().unwrap_or(text)),
    );
}

// Read whole rather than capped, and for the same reason the declared list is
// no longer capped: this is evidence and not a rendered line. Nothing here
// reaches a document or a request — it is compared against, and the comparison
// is the only thing standing between a true claim and a dropped one.
//
// Comments are cut out of that comparison above. A module comment asserting a
// mechanism its file does not implement put every name in the assertion into
// this set, which witnessed the claim that repeated it: `VAULT_LIMIT caps
// postings validated by Decoder::decode()` reached a document with `Decoder`
// declared two directories away and no call to it anywhere. A comment is not a
// declaration, and the text that reaches the model is untouched — a pass still
// reads every comment, and may still say what one claims, so long as it says
// whose claim it is.
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
    let left_out = |cause| {
        (
            agent::File::omitted(name.to_owned(), size),
            Some(Problem {
                path: path.clone(),
                cause,
            }),
        )
    };
    let (file, problem) = if size > PER_FILE_BYTE_CAP {
        left_out(Omission::TooLarge { size })
    } else {
        match fs::read(&path) {
            Ok(bytes) => (elided_or_whole(&path, name.to_owned(), size, bytes), None),
            Err(source) => left_out(Omission::Unreadable { source }),
        }
    };

    let mut described = Described::default();
    let text = file
        .kept()
        .or_else(|| file.bytes().and_then(|bytes| str::from_utf8(bytes).ok()));
    if let Some(text) = text {
        describe(&mut described, &path, name, text);
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
#[path = "tests/fitting.rs"]
mod tests;
