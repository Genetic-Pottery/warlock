use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::document::{self, Defect, Described};
use crate::pact::{Error, Observer, Refusal};
use crate::{Agent, agent, hash, languages, walk};

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

    fn expected(&self) -> document::Expected<'_> {
        document::Expected::of(&self.request)
    }

    pub(crate) fn render(&self, fill: &document::Fill) -> String {
        document::render(&self.name, fill, &self.expected(), &self.described)
    }

    /// Every line a directory's document needs, asking only about what moved.
    ///
    /// The document is the store: a file whose bytes hash to what the manifest
    /// recorded keeps the line already on the page, and every other file costs one
    /// pass. Both halves have to agree before a line is reused — a hash with no
    /// line on the page is a document somebody edited, and a line with no hash is a
    /// file nobody has measured — and either way the answer is to ask again, which
    /// costs a pass and never a wrong line.
    pub(crate) fn assemble(
        &self,
        carried: Option<(&str, &BTreeMap<String, String>)>,
        agent: &dyn Agent,
        observer: &mut dyn Observer,
    ) -> Result<Assembled, Error> {
        let directory = self.directory();
        let (page, recorded) = match carried {
            Some((page, recorded)) => (document::lines_of(page), recorded.clone()),
            None => (BTreeMap::new(), BTreeMap::new()),
        };

        // Every file is settled against the page before the first pass runs, so
        // that `Observer::describing` can be handed a denominator: what a front end
        // needs is the count of files this directory will *pay* for.
        //
        // A line is kept only where the recorded digest matches the file as it
        // stands *and* the line as it sits on the page — `hash::line_hash` binds
        // the two. A document is warlock's to write, so a line somebody edited by
        // hand simply fails to match and is described again, silently and at the
        // cost of that one file. Testing the file's hash alone was what let an
        // edited line be carried forward and then granted as though a pass had
        // written it.
        let planned: Vec<(&String, &Measured, Option<String>)> = self
            .files
            .iter()
            .map(|(name, measured)| {
                let kept = measured
                    .hash
                    .as_ref()
                    .zip(page.get(name))
                    .filter(|(hash, line)| recorded.get(name) == Some(&hash::line_hash(hash, line)))
                    .map(|(_, line)| line.clone());
                (name, measured, kept)
            })
            .collect();

        let paying = planned.iter().filter(|(.., kept)| kept.is_none()).count();

        let mut assembled = Assembled::default();
        let mut position = 0;
        for (name, measured, kept) in planned {
            if let Some(line) = kept {
                assembled.lines.insert(name.clone(), line);
                assembled.kept.push(name.clone());
            } else {
                position += 1;
                observer.describing(directory, name, measured.size, position, paying);
                let described = self.line(name, agent, observer)?;
                if described.mended {
                    assembled.mended.push(name.clone());
                }
                assembled.problems.extend(described.problem);
                assembled.lines.insert(name.clone(), described.line);
                assembled.asked.push(name.clone());
            }

            // Recorded from the line that actually went into the document, kept or
            // freshly described, so the next run compares against what is on the
            // page rather than against what this one meant to put there.
            if let Some((hash, line)) = measured.hash.as_ref().zip(assembled.lines.get(name)) {
                assembled
                    .hashes
                    .insert(name.clone(), hash::line_hash(hash, line));
            }
        }
        Ok(assembled)
    }

    // Ends with a line warlock wrote itself rather than a refusal, because one
    // unusable answer about one file is no reason to lose the directory it sits
    // in. What it cannot do is invent a file: a name that is not there is an
    // error, since the caller walked the directory to get it.
    pub(crate) fn line(
        &self,
        name: &str,
        agent: &dyn Agent,
        observer: &mut dyn Observer,
    ) -> Result<DescribedFile, Error> {
        let directory = self.directory();
        let (request, described, problem) = one_file(document::FILE_PROMPT, directory, name)
            .map_err(|source| Error::from_walk(directory, source))?;
        let expected = document::Expected::of(&request);

        let answered = ask(
            directory,
            agent,
            observer,
            |rejected| {
                request
                    .clone()
                    .with_prompt(document::file_instructions(name, rejected))
            },
            |answer| document::accept_file(answer, name, &expected, &described),
        )?;

        Ok(match answered {
            Some(line) => DescribedFile {
                line,
                mended: false,
                problem,
            },
            None => DescribedFile {
                line: document::file_fallback(name, &expected, &described),
                mended: true,
                problem,
            },
        })
    }

    // Shown the lines and never the source. Ends in a fill either way: the mend is
    // the floor under an exhausted loop, so a directory is never lost because its
    // synthesis could not be got right.
    pub(crate) fn fill(
        &self,
        lines: &BTreeMap<String, String>,
        agent: &dyn Agent,
        observer: &mut dyn Observer,
    ) -> Result<Synthesised, Error> {
        // Announced before the first attempt waits on a model, so a front end's
        // clock counts what is being waited on rather than going quiet after the
        // last file.
        observer.requesting(lines.len(), self.carried_bytes(lines));

        let expected = self.expected();
        let mut best = None;
        let answered = ask(
            self.directory(),
            agent,
            observer,
            |rejected| {
                self.request
                    .clone()
                    .with_prompt(document::synthesis_instructions(
                        &self.name, lines, &expected, rejected,
                    ))
            },
            |answer| match document::accept_synthesis(answer, lines, &expected, &self.described) {
                document::Accepted::Filled(fill) => Ok(fill),
                document::Accepted::Defective { fill, defects } => {
                    best = Some(fill);
                    Err(defects)
                }
                document::Accepted::Unparsed(defect) => Err(vec![defect]),
            },
        )?;
        if let Some(fill) = answered {
            return Ok(Synthesised {
                fill,
                mends: Vec::new(),
            });
        }

        let unusable = best.unwrap_or_else(|| document::Fill {
            files: lines.clone(),
            ..document::Fill::default()
        });
        let (fill, mends) = document::mend(&unusable, &expected, &self.described);
        Ok(Synthesised { fill, mends })
    }

    // What the synthesis request carries: the lines, and the documents of the
    // directories below. The files are in it by name and size only, so counting
    // them would report a payload that was never sent.
    fn carried_bytes(&self, lines: &BTreeMap<String, String>) -> u64 {
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

// A transport failure ends it at once: a pass that produced no answer is not a
// pass that produced a wrong one, and retrying a missing `claude` finds it
// still missing.
fn ask<T>(
    directory: &Path,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
    request: impl Fn(&[Defect]) -> agent::Request,
    mut accept: impl FnMut(&str) -> Result<T, Vec<Defect>>,
) -> Result<Option<T>, Error> {
    let mut rejected = Vec::new();
    for attempt in 1..=document::ATTEMPTS {
        let answer = agent
            .run(&request(&rejected))
            .map_err(|source| Error::Refused {
                directory: directory.to_path_buf(),
                cause: Refusal::Agent { source },
            })?;
        match accept(answer.text()) {
            Ok(taken) => return Ok(Some(taken)),
            Err(defects) => {
                observer.rejected(directory, &defects, attempt, document::ATTEMPTS);
                rejected = defects;
            }
        }
    }
    Ok(None)
}

#[derive(Debug, Default)]
pub(crate) struct Assembled {
    pub(crate) lines: BTreeMap<String, String>,
    // What each file hashed to as its line was settled, for the manifest to
    // record. A file that could not be hashed is absent and will be asked about
    // again.
    pub(crate) hashes: BTreeMap<String, String>,
    pub(crate) asked: Vec<String>,
    pub(crate) kept: Vec<String>,
    // Files whose every attempt was spent, so warlock wrote the line.
    pub(crate) mended: Vec<String>,
    pub(crate) problems: Vec<Problem>,
}

#[derive(Debug)]
pub(crate) struct DescribedFile {
    pub(crate) line: String,
    pub(crate) mended: bool,
    // Why the pass was shown a name and a size instead of the file, where it
    // was: too large for the per-file cap, or unreadable.
    pub(crate) problem: Option<Problem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Synthesised {
    pub(crate) fill: document::Fill,
    pub(crate) mends: Vec<document::Mend>,
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
// this set, which then witnessed the claim that repeated it: `VAULT_LIMIT caps
// postings validated by Decoder::decode()` reached a document with `Decoder`
// declared two directories away and no call to it anywhere.
//
// The cut has to hold on both sides. `elided_or_whole` keeps comments out of
// the text a pass is sent, and `describe` keeps them out of this set; putting
// either one back alone rebuilds the loop the other exists to close — evidence
// witnessing prose no pass ever read, or a pass reading a claim nothing will
// witness, which is the shape that sent `Decoder::decode()` round twice.
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
