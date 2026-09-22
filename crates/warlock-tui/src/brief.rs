//! A path on disk, and the two values a project on the board is made of: a
//! name and a body — and, in the other direction, a body read back into the
//! slices its scope block was written as. Both directions here, because the
//! format is one format: every way a brief can fail to be one is said once
//! rather than at each caller — `warlock push` and the panel's `/push` are two
//! ways into the same act — and a reader kept apart from its writer is a
//! second opinion about what a slice is, waiting to drift.
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

use std::collections::HashMap;
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

/// A brief read back the way `for_the_board` wrote it: everything above the
/// scope heading, the slices under it in document order, and how many `### `
/// headings down there could not be read as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeBlock {
    brief: String,
    slices: Vec<Slice>,
    unreadable: usize,
    order: Vec<usize>,
}

impl ScopeBlock {
    #[must_use]
    pub fn brief(&self) -> &str {
        &self.brief
    }

    /// In document order, which is the order the positions count in.
    #[must_use]
    pub fn slices(&self) -> &[Slice] {
        &self.slices
    }

    /// The same slices, dependency-first: nothing is handed back before
    /// everything it depends on has been. Ties go to the lower position, so a
    /// ten-slice scope hands back the ninth before the tenth.
    #[must_use]
    pub fn ordered(&self) -> Vec<&Slice> {
        self.order
            .iter()
            .map(|index| &self.slices[*index])
            .collect()
    }

    /// Counted rather than named, because the only heading this parser cannot
    /// read is one with nothing in it — see [`scope_block_in`] — and there is
    /// no text in an empty heading for a caller to quote back.
    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }
}

/// One `### ` heading under the scope block and what was written under it.
///
/// The position and the written number are kept apart on purpose: a brief
/// renumbered by hand, or written `1.`, `2.`, `2.`, still has slices in an
/// order, and that order is where they sit in the document. The number is kept
/// only because `depends_on` is written in it; the references themselves arrive
/// here already resolved to positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    position: usize,
    number: Option<usize>,
    heading: String,
    depends_on: Vec<usize>,
    prose: String,
}

impl Slice {
    /// 1-based, so it reads against a document a person is looking at.
    #[must_use]
    pub fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub fn number(&self) -> Option<usize> {
        self.number
    }

    #[must_use]
    pub fn heading(&self) -> &str {
        &self.heading
    }

    /// Positions rather than the numbers the document spelled them with, in
    /// the order they were written and without repeats. A reference nothing
    /// answers, and a slice's reference to itself, are not here: see
    /// [`scope_block_in`].
    #[must_use]
    pub fn depends_on(&self) -> &[usize] {
        &self.depends_on
    }

    #[must_use]
    pub fn prose(&self) -> &str {
        &self.prose
    }
}

/// Read a project's content back into its brief and its slices.
///
/// ```
/// use warlock_tui::scope_block_in;
///
/// // Written on one line: rustdoc reads a `##` at the start of a line in an
/// // example as its own escape and hands the code a heading a level up.
/// let content = "What is wrong now.\n\n## Scope\n\n### 1. Read the file\n\n\
///     depends_on: []\n\nAnd what it decides.\n";
///
/// let block = scope_block_in(content)?;
///
/// assert_eq!(block.brief(), "What is wrong now.");
/// assert_eq!(block.slices()[0].heading(), "Read the file");
/// assert_eq!(block.slices()[0].prose(), "And what it decides.");
/// # Ok::<(), warlock_tui::ScopeBlockError>(())
/// ```
///
/// A heading is unreadable when there is nothing left of it once the optional
/// `N.` or `N)` is off — `### `, `### 4.`, `### 5)`. Nothing else can fail to
/// be a slice, since both the numbering and the `depends_on` line are optional,
/// so that is the whole of what [`ScopeBlock::unreadable`] counts.
///
/// A `depends_on` number is read through the numbers the slices were written
/// with and kept as a position. One that no slice was written with, and one a
/// slice writes about itself, are dropped: a stale reference is a line to fix
/// and not a reason to hand a caller nothing.
///
/// # Errors
///
/// [`ScopeBlockError`], in the three cases where there is nothing to hand back
/// at all: content with no scope heading, a scope block with no slices under
/// it, and slices that wait on each other in a circle. None is a parse that
/// half worked — a caller gets slices in an order, or a reason.
pub fn scope_block_in(content: &str) -> Result<ScopeBlock, ScopeBlockError> {
    let (brief, block) = split(content).ok_or(ScopeBlockError::NoScope)?;
    let (mut slices, unreadable) = sliced(block);
    if slices.is_empty() {
        return Err(ScopeBlockError::NoSlices);
    }
    resolved(&mut slices);
    let order = cutting(&slices)?;
    Ok(ScopeBlock {
        brief,
        slices,
        unreadable,
        order,
    })
}

// A slice is its position and never the number beside its heading: a scope
// renumbered by hand reads `1.`, `3.`, `7.` and still has a first, a second and
// a third slice, and a document written `1.`, `2.`, `2.` still has three of
// them. So the written numbers are an index into positions and nothing more —
// the one place the document's own numbering is believed is in reading what a
// `depends_on` line points at, and after this nothing downstream can see a
// number again and mistake it for an identity.
//
// Where two slices carry one number the earlier one answers to it, because a
// reader counting down the document for `3.` stops at the first one too. A
// reference no slice answers, and a slice's reference to itself, are dropped:
// either would otherwise be an edge to nowhere or a one-slice cycle, and a
// document with one stale line in it is still a document worth cutting.
fn resolved(slices: &mut [Slice]) {
    let mut positions: HashMap<usize, usize> = HashMap::new();
    for slice in slices.iter() {
        if let Some(number) = slice.number {
            positions.entry(number).or_insert(slice.position);
        }
    }

    for slice in slices.iter_mut() {
        let mut kept: Vec<usize> = Vec::new();
        for number in std::mem::take(&mut slice.depends_on) {
            let Some(position) = positions.get(&number).copied() else {
                continue;
            };
            if position != slice.position && !kept.contains(&position) {
                kept.push(position);
            }
        }
        slice.depends_on = kept;
    }
}

// Red's parser falls back to document order when the dependencies form a
// circle. That is the one fallback this format cannot afford: the order is not
// a display detail here but the order tickets are filed and blocked in, so a
// guess lands on somebody's board as a slice waiting on a slice filed after it,
// and the cost of being wrong is an afternoon of undoing rather than an edit to
// a line. Which of the two edges the author meant is a question only the author
// can answer, so it is asked rather than answered.
//
// `slices` is in document order and a position is 1-based, so a slice's
// position is its index here plus one: `placed` is read through that, and
// sorting this slice of slices anywhere upstream would silently break it.
fn cutting(slices: &[Slice]) -> Result<Vec<usize>, ScopeBlockError> {
    let mut order = Vec::with_capacity(slices.len());
    let mut placed = vec![false; slices.len()];

    while order.len() < slices.len() {
        let next = slices.iter().enumerate().position(|(index, slice)| {
            !placed[index] && slice.depends_on.iter().all(|position| placed[position - 1])
        });
        let Some(index) = next else {
            return Err(ScopeBlockError::Circle {
                slices: slices
                    .iter()
                    .zip(&placed)
                    .filter(|(_, placed)| !**placed)
                    .map(|(slice, _)| format!("slice {} `{}`", slice.position, slice.heading))
                    .collect(),
            });
        };
        placed[index] = true;
        order.push(index);
    }

    Ok(order)
}

/// The heading `scope_block_in` splits at, spelled as `DEFAULT_TEMPLATE`
/// spells it.
const SCOPE: &str = "Scope";

// Deliberately as generous as `template::missing_sections` is about the same
// question — any heading level, case ignored — rather than stricter. A document
// gets onto the board by satisfying that check, so a rule here that refused a
// heading it accepted would mean a project warlock pushed and cannot read back.
//
// The fence is tracked because a brief about warlock quotes markdown: a `##
// Scope` line inside a block is a document being shown, not this document's
// scope section. A substring search has already mis-parsed on the lesser
// version of this, a paragraph that merely says the words.
fn split(content: &str) -> Option<(String, &str)> {
    let mut before = 0;
    let mut fence: Option<String> = None;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        before += line.len();

        if let Some(marker) = &fence {
            if trimmed.starts_with(marker.as_str()) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fenced(trimmed) {
            fence = Some(marker.to_owned());
            continue;
        }

        if headed(trimmed).is_some_and(|heading| heading.eq_ignore_ascii_case(SCOPE)) {
            let above = content[..before - line.len()].trim_end();
            return Some((above.to_owned(), &content[before..]));
        }
    }
    None
}

fn sliced(block: &str) -> (Vec<Slice>, usize) {
    let mut slices: Vec<Slice> = Vec::new();
    let mut unreadable = 0;
    let mut open: Option<(Option<usize>, String)> = None;
    let mut body: Vec<&str> = Vec::new();
    let mut fence: Option<String> = None;

    for line in block.lines() {
        let trimmed = line.trim();

        if let Some(marker) = &fence {
            if trimmed.starts_with(marker.as_str()) {
                fence = None;
            }
            if open.is_some() {
                body.push(line);
            }
            continue;
        }
        if let Some(marker) = fenced(trimmed) {
            fence = Some(marker.to_owned());
            if open.is_some() {
                body.push(line);
            }
            continue;
        }

        let Some(heading) = headed(trimmed) else {
            if open.is_some() {
                body.push(line);
            }
            continue;
        };

        if let Some((number, heading)) = open.take() {
            slices.push(closed(slices.len() + 1, number, heading, &body));
        }
        body.clear();

        // Any other heading ends the prose and starts nothing: a `## ` after
        // the scope block is the next section, and a `#### ` under a slice is
        // that slice's own sub-heading rather than a slice of its own.
        if depth(trimmed) != 3 {
            continue;
        }

        let (number, heading) = numbered(heading);
        if heading.is_empty() {
            unreadable += 1;
            continue;
        }
        open = Some((number, heading.to_owned()));
    }

    if let Some((number, heading)) = open {
        slices.push(closed(slices.len() + 1, number, heading, &body));
    }
    (slices, unreadable)
}

// Only the first non-blank line is asked, so a brief that talks about
// `depends_on:` in its prose is talking about it. The line is consumed when it
// is one, because a document's dependency declaration is not also a paragraph
// of the ticket it becomes.
fn closed(position: usize, number: Option<usize>, heading: String, body: &[&str]) -> Slice {
    let first = body
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(body.len());
    let (depends_on, rest) = match body.get(first).and_then(|line| depended(line.trim())) {
        Some(depends_on) => (depends_on, &body[first + 1..]),
        None => (Vec::new(), &body[first..]),
    };

    Slice {
        position,
        number,
        heading,
        depends_on,
        prose: prose(rest),
    }
}

const DEPENDS: &str = "depends_on:";

// Anything that is not a number between the brackets is passed over rather than
// refused: `depends_on: [1, none]` is a document with a typo in one reference,
// and throwing away the other reference with it helps nobody. The brackets
// themselves are optional for the same reason.
fn depended(trimmed: &str) -> Option<Vec<usize>> {
    let rest = trimmed.strip_prefix(DEPENDS)?.trim();
    let inside = rest
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(rest);
    Some(
        inside
            .split(',')
            .filter_map(|number| number.trim().parse().ok())
            .collect(),
    )
}

// A number too large to hold leaves the heading stripped and the number absent:
// nothing can depend on a slice whose number will not parse, so there is
// nothing to lose by reading it as an unnumbered one.
fn numbered(heading: &str) -> (Option<usize>, &str) {
    let rest = heading.trim_start_matches(|character: char| character.is_ascii_digit());
    if rest.len() == heading.len() {
        return (None, heading);
    }
    let digits = &heading[..heading.len() - rest.len()];
    let Some(rest) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) else {
        return (None, heading);
    };
    (digits.parse().ok(), rest.trim())
}

fn depth(trimmed: &str) -> usize {
    trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count()
}

// The blank lines around a body go, and every line inside it stays: a fenced
// block, a bullet list and a paragraph break are all the author's.
fn prose(body: &[&str]) -> String {
    let first = body
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(body.len());
    let last = body
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(first, |line| line + 1);
    body[first..last].join("\n")
}

/// Three ways content is not a cuttable project, and no fourth: everything
/// else a document can get wrong about a slice is read as written.
#[derive(Debug)]
pub enum ScopeBlockError {
    NoScope,
    NoSlices,
    /// Every slice left without an order, named as the document shows it — a
    /// position and a heading, rather than an index of this parser's own that
    /// a reader would have to count out for themselves.
    Circle {
        slices: Vec<String>,
    },
}

impl fmt::Display for ScopeBlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoScope => write!(
                f,
                "this project has no `## Scope` heading, so there is nothing to cut into slices"
            ),
            Self::NoSlices => write!(
                f,
                "this project's `## Scope` section has no `### ` slice headings, so there is nothing to cut"
            ),
            Self::Circle { slices } => write!(
                f,
                "these slices wait on each other, so there is no order to cut them in: {}",
                crate::naming::and_listed(slices)
            ),
        }
    }
}

impl std::error::Error for ScopeBlockError {}

// `## Outcome and ## Scope`. The heading marker is put on here and the joining
// is `naming`'s, which `writing` and `error` also refuse through.
fn naming(missing: &[String]) -> String {
    let named: Vec<String> = missing
        .iter()
        .map(|section| format!("## {section}"))
        .collect();
    crate::naming::and_listed(&named)
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
