//! A path on disk, and the two values a project on the board is made of: a
//! name and a body. One function, and every way a brief can fail to be one
//! said here rather than at each caller — `warlock push` and the panel's
//! `/push` are two ways into the same act, and a rule written twice is two
//! rules that drift into refusing different documents.
//!
//! Path in, never text: a caller holding bytes in memory could push a document
//! nobody can open afterwards, and a brief edited with `e` after `/write` would
//! not be the brief that went up. The file is the document, and it is read here
//! at the moment of the push.
//!
//! Nothing is summarised, re-headed or re-ordered, and no word is warlock's:
//! the name is the title line with its marker taken off and the content is the
//! rest of the document, because a body that says something the file does not
//! is a board nobody can diff against the repository.
//!
//! Two things are done to the bytes on the way, both of them about the page
//! rather than about the prose, and both in [`for_the_board`]: the paragraphs
//! are unwrapped, and the success criteria are made checkable.

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use crate::template::{self, brief_template, missing_sections};

/// The name and content of the project a brief would become.
///
/// Two strings and no path: what happens to them is the sending side's, and a
/// value that cannot carry anything else is one that cannot leak anything
/// else. No key is read on this path at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    name: String,
    content: String,
}

impl Brief {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Read the brief at `path`, held to the shape `root` asks for.
///
/// Both paths are needed and neither is derived from the other: the brief can
/// be anywhere a person points at, and the template is the repository's, found
/// through [`brief_template`] rather than by spelling `.warlock/` again here.
///
/// ```no_run
/// use warlock_tui::brief_at;
///
/// let brief = brief_at("/repo", "/repo/docs/warlock-brief-22-pushing.md")?;
///
/// // The title line with its marker stripped, and the document under it.
/// assert_eq!(brief.name(), "Push a brief to the board");
/// # Ok::<(), warlock_tui::BriefError>(())
/// ```
///
/// # Errors
///
/// [`Error`] and nothing sent: a file that is not there, one that will not
/// read, a template that will not read, a document with no title line, and a
/// document missing a section of the shape are five separate refusals, each a
/// single line. Every one of them is raised before anything leaves this
/// machine.
pub fn brief_at(root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<Brief, Error> {
    let path = path.as_ref();
    let document = read(path)?;

    // The title first, because it is a fact about the document alone: a
    // repository whose template will not read still gets told the truth about
    // the file in front of it, rather than a sentence about `.warlock/`.
    let brief = titled(&document).ok_or_else(|| Error::NoTitle {
        path: path.to_owned(),
    })?;

    let shape = brief_template(root).map_err(|source| Error::Shape { source })?;
    // The one section check in warlock, borrowed rather than repeated: a
    // second one worded here would be a second opinion about what a brief is,
    // and `/write` and `/push` would come to disagree about the same document.
    let missing = missing_sections(&shape, &document);
    if !missing.is_empty() {
        return Err(Error::Sections {
            path: path.to_owned(),
            missing: missing.into_iter().map(str::to_owned).collect(),
        });
    }

    Ok(brief)
}

// Absent is kept apart from unreadable on `template.rs`'s rule, less its third
// answer: a path nobody has written a file at is a path that was typed wrong,
// and a file that exists and will not decode is a file to go and look at. The
// read that decodes is the read that fails, so bytes that are not UTF-8 arrive
// here as an ordinary I/O error.
fn read(path: &Path) -> Result<String, Error> {
    match fs::read_to_string(path) {
        Ok(document) => Ok(document),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Err(Error::Absent {
            path: path.to_owned(),
        }),
        Err(source) => Err(Error::Unreadable {
            path: path.to_owned(),
            source,
        }),
    }
}

// `slug_of`'s rule in `writing.rs`, to the letter, because the name a project
// is filed under and the filename a brief was proposed under are the same
// title: the first `# ` line, with the prefix matched exactly, so `#Title`,
// `## Section` and an indented `  # Title` are all not it. The first such line
// is the title even when it is blank — a document whose first heading says
// nothing is refused rather than searched past, since the heading below it is a
// section and not a title somebody meant.
//
// The content is the document with that one line lifted out, put through
// `for_the_board` and nothing else: no trim, no newline appended, no heading
// demoted. Whatever sits above the title stays where it is rather than being
// dropped on the floor.
fn titled(document: &str) -> Option<Brief> {
    let mut before = 0;
    for line in document.split_inclusive('\n') {
        if let Some(title) = line.strip_prefix("# ") {
            let name = title.trim();
            if name.is_empty() {
                return None;
            }
            return Some(Brief {
                name: name.to_owned(),
                content: for_the_board(&format!(
                    "{}{}",
                    &document[..before],
                    &document[before + line.len()..]
                )),
            });
        }
        before += line.len();
    }
    None
}

/// The section whose bullets become checkable, and the only one: a criterion is
/// the part of a brief somebody ticks off on the board, where a constraint and
/// an out-of-scope line are decisions rather than tasks. Spelled as the built-in
/// template spells it, and matched exactly — a repository that has renamed the
/// section in its own template gets plain bullets rather than warlock guessing
/// at which of its headings means this one.
const CRITERIA: &str = "Success criteria";

// Markdown reads a single newline inside a paragraph as a space. Linear's
// editor reads it as a line break, and this repository's briefs are hard
// wrapped at about seventy-six columns for a terminal, so sending the bytes as
// they sit puts the file's column width onto a page that has its own and every
// paragraph lands ragged. Joining them is not warlock rewriting the author: it
// is warlock declining to impose a width the document never meant to carry, and
// the words, their order and the blank lines between the paragraphs are
// untouched.
//
// A line break is content in a fenced block, a heading, a list item, a table row
// and a quote, so each of those keeps its own line. An indented code block is
// deliberately not recognised: telling one from the continuation of a wrapped
// bullet needs a parser rather than a rule, and the brief template produces
// fenced blocks.
//
// Written over `lines`, so a document with CRLF endings comes back with LF
// ones. That is the same normalising `.gitattributes` already asks of every
// text file here, and a body is bytes bound for somebody else's editor rather
// than for this repository.
fn for_the_board(content: &str) -> String {
    let mut board = String::with_capacity(content.len());
    let mut fence: Option<String> = None;
    let mut joining = false;
    let mut criteria = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(marker) = &fence {
            if trimmed.starts_with(marker.as_str()) {
                fence = None;
            }
            board.push_str(line);
            board.push('\n');
            continue;
        }

        if let Some(marker) = fenced(trimmed) {
            ended(&mut board, &mut joining);
            fence = Some(marker.to_owned());
            board.push_str(line);
            board.push('\n');
            continue;
        }

        if trimmed.is_empty() {
            ended(&mut board, &mut joining);
            board.push('\n');
            continue;
        }

        if let Some(heading) = headed(trimmed) {
            ended(&mut board, &mut joining);
            criteria = heading == CRITERIA;
            board.push_str(trimmed);
            board.push('\n');
            continue;
        }

        // A bullet is left open, because the lines under it are its own wrapped
        // text; a row and a quote are not, because the line after one of those
        // is the next row or the next quoted line.
        if let Some(item) = bulleted(trimmed) {
            ended(&mut board, &mut joining);
            board.push_str(&checkable(trimmed, item, criteria));
            joining = true;
            continue;
        }

        if trimmed.starts_with('|') || trimmed.starts_with('>') {
            ended(&mut board, &mut joining);
            board.push_str(trimmed);
            board.push('\n');
            continue;
        }

        if joining {
            board.push(' ');
        }
        board.push_str(trimmed);
        joining = true;
    }

    ended(&mut board, &mut joining);
    board
}

fn ended(board: &mut String, joining: &mut bool) {
    if *joining {
        board.push('\n');
        *joining = false;
    }
}

// The marker itself and not just its length, so a block opened with backticks
// is closed by backticks: a `~~~` inside a ``` block is content.
fn fenced(trimmed: &str) -> Option<&str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| trimmed.starts_with(marker))
}

fn headed(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('#')
        .map(|rest| rest.trim_start_matches('#').trim())
        .filter(|_| trimmed.starts_with('#'))
}

// The three markers markdown takes, and a numbered item, which is what a
// `### 1.` slice's prose sometimes sits under.
fn bulleted(trimmed: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = trimmed.trim_start_matches(|character: char| character.is_ascii_digit());
    if digits.len() < trimmed.len() {
        return digits.strip_prefix(". ");
    }
    None
}

// A box only under the criteria, only on a `-` item, and never on one that
// already carries one — a document written with boxes in it is a document that
// keeps them rather than one that grows `- [ ] [ ]`.
fn checkable(trimmed: &str, item: &str, criteria: bool) -> String {
    if !criteria
        || !trimmed.starts_with("- ")
        || item.starts_with("[ ] ")
        || item.starts_with("[x] ")
    {
        return trimmed.to_owned();
    }
    format!("- [ ] {item}")
}

// `## Outcome and ## Scope`, in `writing::missing_line`'s shape: a refusal
// naming more than one section is read as a sentence, and a comma before the
// last of them would be read as a third section.
fn naming(missing: &[String]) -> String {
    let named: Vec<String> = missing
        .iter()
        .map(|section| format!("## {section}"))
        .collect();
    let Some((last, rest)) = named.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        return last.clone();
    }
    format!("{} and {last}", rest.join(", "))
}

/// Five ways a file is not a brief, and no sixth: nothing here is a failure to
/// *send* one, which belongs to the side that opens the socket.
#[derive(Debug)]
pub enum Error {
    Absent {
        path: PathBuf,
    },
    Unreadable {
        path: PathBuf,
        source: io::Error,
    },
    Shape {
        source: template::Error,
    },
    /// A document with no `# ` title line, and a document whose title says
    /// nothing once the marker is off, are one refusal on purpose. There is no
    /// `untitled` here as there is behind `/write`: a filename a reader can see
    /// in a field and retype is one thing, and a project filed on somebody's
    /// board under a placeholder is another.
    NoTitle {
        path: PathBuf,
    },
    /// Every section that is missing, not the first one noticed: the fix is to
    /// go back to the document once, and a refusal naming a section at a time
    /// is three visits.
    Sections {
        path: PathBuf,
        missing: Vec<String>,
    },
}

// Every variant is one line, and the two that wrap another error's text can
// only wrap an `io::Error` or `template::Error`'s sentence about one — both of
// which are single-line, as `error.rs` says of every I/O error in this
// workspace. Nothing here quotes a parser's diagnostic, so there is nothing to
// flatten.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent { path } => write!(
                f,
                "there is no file at `{}`, so there is nothing to push",
                path.display()
            ),
            Self::Unreadable { path, source } => {
                write!(f, "could not read `{}`: {source}", path.display())
            }
            // The template's own sentence names the file it could not read,
            // and what is worth adding to it is the half it cannot say: the
            // brief is still on disk and nothing went to the board.
            Self::Shape { source } => {
                write!(
                    f,
                    "could not read the brief shape, so nothing was pushed: {source}"
                )
            }
            // Says the rule rather than only the fact, because the usual cause
            // is a heading that looks like a title and is not — a `#Title`
            // with no space after the hash.
            Self::NoTitle { path } => write!(
                f,
                "`{}` has no `# ` title line, so there is no project name to push",
                path.display()
            ),
            Self::Sections { path, missing } => write!(
                f,
                "`{}` is missing {}, so nothing was pushed",
                path.display(),
                naming(missing)
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            Self::Shape { source } => Some(source),
            // No source and none to have: a path nobody wrote a file at, a
            // document with no title and a document missing a section are
            // facts about what was typed and what was written, rather than
            // failures something underneath reported.
            Self::Absent { .. } | Self::NoTitle { .. } | Self::Sections { .. } => None,
        }
    }
}

// Files in a directory of the test's own and nothing else: no socket, no
// process, no model pass and no key read anywhere on this path.
#[cfg(test)]
#[path = "tests/brief.rs"]
mod tests;
