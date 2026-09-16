use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::agent::{File, Request};

pub const ENTRY_CHARS: usize = 280;

pub const ENTRY_MINIMUM: usize = 20;

pub const PURPOSE_CHARS: usize = 700;

pub const LIST_CAP: usize = 12;

// Sixteen, measured rather than chosen. Eight and sixteen route to the right
// file equally well — 87.0% against 90.7% over three runs of 36 routing
// questions, a difference of four answers inside the run-to-run spread — but
// naming the right symbol goes 33.3% to 45.4%, and that gap held in every run.
// Thirty-two and sixty-four buy none of it back at a third and a half again the
// tokens. See docs/warlock-aider-baseline-measurement.md.
pub const DECLARED_SHOWN: usize = 16;

// Two was the old number, on the reasoning that a model shown its own defects
// either fixes the slot or cannot. It does not hold: the observed failure is a
// model that overshoots in the other direction — a slot left empty, then
// filled at 300 characters against a cap of 280 — and with two attempts that
// oscillation ends in the mend, which keeps nothing the model wrote for the
// slot.
pub const ATTEMPTS: usize = 4;

// No date in here, though every instinct says to put one: `granted_at` in
// `.warlock/pacts.toml` already records when the document was granted, and a
// date in the stamp would make a re-pact that changed no prose a diff.
pub const STAMP: &str = "<!-- warlock -->\n\
> Written by a model pass over this directory alone, to be read before its \
source and to say which source to read. A map, not a specification: check \
anything you are about to rely on against the files themselves, and where this \
document and the code disagree, the code is right.\n";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub directories: BTreeMap<String, String>,
    #[serde(default)]
    pub structure: Vec<Entry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stated")]
pub struct Entry {
    pub line: String,
    #[serde(default)]
    pub names: Vec<String>,
}

// A pass that answers a list entry with a bare string rather than the object
// asked for is the commonest shape a model gets wrong, and reading it strictly
// would answer that slip with `NotJson` — the one defect with nothing to repair
// from, which throws away the whole pass and every grant above it. Read
// leniently and the same slip is a claim naming nothing, which is a defect the
// repair road already handles: asked about once, and dropped by `mend` if the
// answer comes back no better.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stated {
    Line(String),
    Entry {
        #[serde(default)]
        line: String,
        #[serde(default)]
        names: Vec<String>,
    },
}

impl From<Stated> for Entry {
    fn from(stated: Stated) -> Self {
        match stated {
            Stated::Line(line) => Self {
                line,
                names: Vec::new(),
            },
            Stated::Entry { line, names } => Self { line, names },
        }
    }
}

#[cfg(test)]
impl Entry {
    fn of(line: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            names: Vec::new(),
        }
    }

    fn naming(line: impl Into<String>, name: &str) -> Self {
        Self {
            line: line.into(),
            names: vec![name.to_owned()],
        }
    }
}

/// What warlock measured of a directory itself, as against what it sent.
///
/// [`Evidence`] checks a claim's names against the request first and against
/// this second. That used to be a convenience; since the per-file road it is
/// the whole of the evidence, because a synthesis request carries names and
/// sizes and no text at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    pub declared: BTreeMap<String, Vec<String>>,
    /// Every identifier written in each file, whether or not it is declared
    /// there.
    ///
    /// `declared` is a prefix table's reading of a file and will never cover
    /// every language: stripping `public` off a Java method leaves `long
    /// total()`, which opens with a type rather than a keyword and matches no
    /// row, so `Invoice.java` measures as declaring only `Invoice`. Adding rows
    /// does not fix the shape of that, and the cost of the gap is a real claim
    /// refused and dropped out of the document.
    ///
    /// So this is the witness the per-file road took away, put back without
    /// putting the text back: `knows` used to accept a name that any sent
    /// file's text contained.
    pub tokens: BTreeMap<String, BTreeSet<String>>,
}

impl Described {
    /// Whether some file here writes `name` — every identifier in it, so that a
    /// qualified name like `Pipeline.Stage` is met by a file writing both
    /// halves. Cutting the name into identifiers is what keeps this stricter
    /// than the containment it replaced: a file writing `AuditApplyer` does
    /// not witness a claim about `Apply`.
    #[must_use]
    pub fn written_anywhere(&self, name: &str) -> bool {
        self.tokens.values().any(|tokens| written(name, tokens))
    }

    /// Whether any file here writes the tool's own name, which is what stands
    /// the [`Defect::ToolNamed`] guard down.
    #[must_use]
    pub fn mentions_tool(&self) -> bool {
        self.tokens
            .values()
            .flatten()
            .any(|token| names_tool(token))
    }
}

fn written(name: &str, tokens: &BTreeSet<String>) -> bool {
    let mut parts = identifiers(name).peekable();
    parts.peek().is_some() && parts.all(|part| tokens.contains(part))
}

/// The identifiers in a piece of text, which is what both a file's tokens and a
/// claim's name are cut into so that the two can be compared. Deliberately
/// crude: anything that is not a letter, a digit or an underscore separates,
/// and a run that does not open like an identifier is dropped.
pub fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| {
            word.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        })
}

impl Fill {
    #[must_use]
    pub fn stub(request: &Request) -> Self {
        const ENTRY: &str = "a stand-in entry, filled by a test double";
        let expected = Expected::of(request);
        Self {
            purpose: format!(
                "A stand-in description of a directory of {} files and {} subdirectories, \
                 written by a test double that read none of them.",
                expected.files.len(),
                expected.directories.len()
            ),
            files: expected
                .asked()
                .map(|path| (path.to_owned(), ENTRY.to_owned()))
                .collect(),
            directories: expected
                .directories
                .keys()
                .map(|name| ((*name).to_owned(), ENTRY.to_owned()))
                .collect(),
            structure: Vec::new(),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a fill is plain strings and serialises")
    }
}

#[must_use]
pub fn stub_answer(request: &Request) -> String {
    const PROSE: &str = "A stand-in account of some contents, written by a test double that read \
                         none of them: records and pins, listed one after another with no code \
                         among them.";
    const LINE: &str = "A stand-in line about one file, written by a test double that read none \
                        of it.";
    // Two kinds of pass and one test double, told apart the only way they can
    // be from the outside: by what they were asked. A double that answered a
    // per-file pass with a whole fill would be testing the parser and not the
    // pipeline.
    if request.prompt().starts_with(FILE_PROMPT) {
        format!("{{\"line\": {LINE:?}}}")
    } else if request.prompt().starts_with(SYNTHESIS_PROMPT) {
        let mut fill = Fill::stub(request);
        fill.files.clear();
        fill.to_json()
    } else {
        PROSE.to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown<'a> {
    Text(&'a str),
    NotText,
    Unsent,
}

#[derive(Debug)]
pub struct Expected<'a> {
    // The directory's own name, the way `render` is given it: the last
    // component of the request's directory and never the path, which is
    // absolute and would be the reader's home directory. It is here for
    // [`fallback::purpose`], the one repair whose fact is the directory itself
    // rather than a file in it, so [`mend`] needs no fourth argument to say
    // what the directory is called.
    name: String,
    files: BTreeMap<&'a str, (u64, Shown<'a>)>,
    directories: BTreeMap<&'a str, &'a str>,
}

impl<'a> Expected<'a> {
    #[must_use]
    pub fn of(request: &'a Request) -> Self {
        let name = request
            .directory()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let files = request
            .files()
            .iter()
            .map(|file| (file.path(), (file.size(), shown(file))))
            .collect();
        let directories = request
            .child_documents()
            .iter()
            .map(|child| (child.directory(), child.text()))
            .collect();
        Self {
            name,
            files,
            directories,
        }
    }

    fn asked(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.files
            .iter()
            .filter(|(_, (_, shown))| matches!(shown, Shown::Text(_)))
            .map(|(path, _)| *path)
    }

    fn holds(&self, target: &str) -> bool {
        self.files.contains_key(target) || self.directories.contains_key(target)
    }
}

/// The two witnesses to a directory, which are only ever asked together.
///
/// [`Expected`] is built from a request and can answer nothing but what the
/// request carried. [`Described`] is warlock's own reading of the directory and
/// answers what is true of it. Those were the same thing while a request
/// carried every file's text; a synthesis request carries names and sizes, and
/// six separate bugs came of asking the first where the second was meant —
/// every one of them silent, because a witness that has lost its evidence
/// answers "no" rather than failing.
///
/// So every witness-shaped question lives here and none on `Expected`, which
/// keeps only what is honestly about the request: `holds`, `asked`, the files
/// and the children. A new check that forgets the second witness does not
/// compile, rather than passing its tests and thinning every document.
/// `the_road_without_the_text_checks_no_more_weakly_than_the_road_with_it` is
/// the same invariant asserted from the outside.
#[derive(Debug, Clone, Copy)]
pub struct Evidence<'a> {
    expected: &'a Expected<'a>,
    described: &'a Described,
}

impl<'a> Evidence<'a> {
    #[must_use]
    pub const fn new(expected: &'a Expected<'a>, described: &'a Described) -> Self {
        Self {
            expected,
            described,
        }
    }

    #[must_use]
    pub const fn expected(&self) -> &'a Expected<'a> {
        self.expected
    }

    fn sent(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.expected
            .files
            .values()
            .filter_map(|(_, shown)| match shown {
                Shown::Text(text) => Some(*text),
                Shown::NotText | Shown::Unsent => None,
            })
    }

    // A name warlock can find, asked of both witnesses because the request is
    // not always one of them.
    //
    // The request is the better witness where it has the file: containment
    // rather than a declaration on purpose — a claim may name a lint, a
    // manifest key or a constant that no language table declares.
    //
    // `Described` is warlock's own measurement, taken while the request was
    // built and true of the directory whatever the request ended up carrying.
    // It is what keeps this working for a pass shown something other than the
    // files: a directory too big to send whole, or one shown assembled lines
    // rather than source. Without it such a pass has every real name refused.
    fn knows(&self, name: &str) -> bool {
        self.expected.holds(name)
            || self.described.declared.contains_key(name)
            || self
                .described
                .declared
                .values()
                .flatten()
                .any(|declared| declared == name)
            || self.described.written_anywhere(name)
            || self.sent().any(|text| text.contains(name))
    }

    // Whether the directory itself uses the tool's name, which is what stands
    // the [`Defect::ToolNamed`] guard down. A leaf directory sends no text and
    // has no child document, so the request half answers `false` for every one
    // of them — which, in warlock's own repository, refused every true claim
    // about warlock until the second witness was asked here.
    fn mentions_tool(&self) -> bool {
        self.sent().any(names_tool)
            || self
                .expected
                .directories
                .values()
                .any(|text| names_tool(text.strip_prefix(STAMP).unwrap_or(text)))
            || self.described.mentions_tool()
    }
}

fn names_tool(text: &str) -> bool {
    text.to_ascii_lowercase().contains("warlock")
}

fn shown(file: &File) -> Shown<'_> {
    if let Some(kept) = file.kept() {
        return Shown::Text(kept);
    }
    match file.bytes().map(std::str::from_utf8) {
        Some(Ok(text)) => Shown::Text(text),
        Some(Err(_)) => Shown::NotText,
        None => Shown::Unsent,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Defect {
    NotJson {
        detail: String,
    },
    Missing {
        field: String,
    },
    Empty {
        field: String,
    },
    Multiline {
        field: String,
    },
    TooShort {
        field: String,
        chars: usize,
        minimum: usize,
    },
    TooLong {
        field: String,
        chars: usize,
        cap: usize,
    },
    TooMany {
        field: String,
        count: usize,
        cap: usize,
    },
    UnknownTarget {
        field: String,
        name: String,
    },
    ToolNamed {
        field: String,
    },
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson { detail } => write!(f, "the answer is not a JSON object: {detail}"),
            Self::Missing { field } => write!(f, "{field} is missing"),
            Self::Empty { field } => write!(f, "{field} is empty"),
            Self::Multiline { field } => write!(f, "{field} runs to more than one line"),
            Self::TooShort {
                field,
                chars,
                minimum,
            } => write!(
                f,
                "{field} is {chars} characters, under the {minimum} it has to reach"
            ),
            Self::TooLong { field, chars, cap } => {
                write!(f, "{field} is {chars} characters, over the cap of {cap}")
            }
            Self::TooMany { field, count, cap } => {
                write!(f, "{field} has {count} entries, over the cap of {cap}")
            }
            Self::ToolNamed { field } => write!(
                f,
                "{field} names warlock, which is the tool writing this document and not \
                 something the files mention"
            ),
            Self::UnknownTarget { field, name } => write!(
                f,
                "{field} names `{name}`, which is not a file, a subdirectory, or a name \
                 declared in one of them"
            ),
        }
    }
}

impl std::error::Error for Defect {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    Filled(Fill),
    Defective { fill: Fill, defects: Vec<Defect> },
    Unparsed(Defect),
}

// The synthesis pass: the slots that are about a directory rather than about
// one file in it, written from the lines and not from the source.
//
// Per-file granularity has no pass that reads the whole directory, so nothing
// is left to write `purpose` and `## Structure` from except the lines already
// written. What makes that safe to check rather than merely cheap is the second
// witness: `check` asks `Described` for a name the request cannot vouch for, and
// a synthesis request carries no file text at all.
pub const SYNTHESIS_PROMPT: &str = "\
Fill in the JSON object at the end of these instructions, describing the \
directory whose file lines follow them, and output the filled object and \
nothing else.

You are writing the parts of a WARLOCK.md that are about the directory as a \
whole. It is read by a model, not a person, before any source file is opened, \
and its one job is routing: to say what is here and which file to open for a \
given question. Warlock is the tool that lays the document out from your \
answer; it is not the project being described, and its name belongs in no \
value unless the files themselves use it.

You are not shown the source. You are shown the directory's name and the line \
already written for each file in it, and every claim you make has to come from \
those lines.

\"purpose\": one to three sentences. What this directory is and what it does, \
in the words a question about it would use.

\"structure\": how the files here fit together, one fact per entry: what calls \
what, in what order, which way a dependency runs. Only what the lines you were \
shown show. Each entry is {\"line\": ..., \"names\": [...]}, where \"names\" \
lists every file, type, function or constant the line refers to, spelt as the \
lines spell it. A structure entry names at least one. An empty list is fine.

\"directories\": one line per key. What is under it and the kind of question \
that should send a reader there. Write it from the subdirectory's own \
WARLOCK.md, which follows below, and do not restate that document's contents.

Every value is one line. Write about the directory in its own voice: no first \
person, and nothing about this request or about what you were or were not \
shown.";

fn turned_down(text: &mut String, rejected: &[Defect]) {
    if rejected.is_empty() {
        return;
    }
    text.push_str(
        "\n\nA previous answer to exactly this request was turned down. Do not repeat \
         these defects:",
    );
    for defect in rejected {
        let _ = write!(text, "\n- {defect}");
    }
}

#[must_use]
pub fn synthesis_instructions(
    name: &str,
    lines: &BTreeMap<String, String>,
    expected: &Expected<'_>,
    rejected: &[Defect],
) -> String {
    let mut text = SYNTHESIS_PROMPT.to_owned();
    turned_down(&mut text, rejected);
    let _ = write!(
        text,
        "\n\nCaps: {ENTRY_MINIMUM} to {ENTRY_CHARS} characters per value, {PURPOSE_CHARS} for \
         the purpose, {LIST_CAP} entries per list.\n\nThe directory is `{name}`, and its \
         files are:\n"
    );
    for (path, line) in lines {
        let _ = write!(text, "\n- `{path}` — {line}");
    }
    for (child, document) in &expected.directories {
        let _ = write!(
            text,
            "\n\nThe directory `{child}/` holds this document:\n\n{}",
            document.trim()
        );
    }
    // The children by name, so the pass fills a key in rather than inventing
    // one. An empty `{}` here left the spelling to be guessed from the prose
    // above, which writes a directory as `crates/` — and a key nothing asked
    // for is a `## Directories` line lost. `files` is left out of the shape
    // altogether rather than shown empty: this pass is not asked about them,
    // and a slot on screen is an invitation to fill it.
    let shape = serde_json::json!({
        "purpose": "",
        "directories": expected
            .directories
            .keys()
            .map(|child| ((*child).to_owned(), String::new()))
            .collect::<BTreeMap<String, String>>(),
        "structure": [],
    });
    let _ = write!(
        text,
        "\n\nReturn exactly this object with every empty string filled in and the \
         lists populated, as JSON, with no code fence and nothing before or after \
         it:\n\n{shape}",
    );
    text
}

/// The directory-wide slots, checked the way a whole answer's are.
///
/// `files` and `directories` are the caller's: this pass is not shown them and
/// is not asked about them, so a fill returned here carries the lines it was
/// given and whatever the pass wrote about them together.
#[must_use]
pub fn accept_synthesis(
    answer: &str,
    lines: &BTreeMap<String, String>,
    expected: &Expected<'_>,
    described: &Described,
) -> Accepted {
    let mut parsed = match parse(answer) {
        Ok(parsed) => parsed,
        Err(defect) => return Accepted::Unparsed(defect),
    };

    // Checked without the lines and answered with them. The pass was asked
    // about no file — a synthesis request carries names and sizes, never text,
    // so `Expected::asked` is empty — and each line was checked as it was
    // accepted, by `accept_file`.
    //
    // The children it *is* asked about are kept, under the key the request
    // spells them with. `render` writes a child as `crates/`, and every
    // synthesis pass is shown its children's documents, so a pass that has read
    // one offers the trailing slash back; a child directory's key is a single
    // path segment and never carries one, so correcting it keeps a real
    // `## Directories` line that would otherwise be dropped on the floor.
    //
    // Anything still unasked for is taken out rather than faulted: no re-ask
    // could make it right, because there was no question. Leaving one in place
    // would put a key nothing wanted in front of `keyed`, which trips its debug
    // assertion.
    parsed.files.clear();
    parsed.directories = parsed
        .directories
        .into_iter()
        .map(|(child, line)| (child.trim_end_matches('/').to_owned(), line))
        .filter(|(child, _)| expected.directories.contains_key(child.as_str()))
        .collect();
    let defects = check(&parsed, expected, described);
    let fill = Fill {
        files: lines.clone(),
        ..parsed
    };
    if defects.is_empty() {
        Accepted::Filled(fill)
    } else {
        Accepted::Defective { fill, defects }
    }
}

/// Read the file lines back out of a document warlock wrote.
///
/// This is what makes the document its own store: a run re-asks about the files
/// whose hashes moved and takes the rest from the page, so nothing is kept
/// twice and nothing can drift from what is committed.
///
/// The parse is deliberately strict about the shape `render` writes and
/// deliberately lax about what a line says — the entry is whatever sits between
/// the em dash and the declared names, because that is the only part a pass
/// wrote. A line this cannot read is left out rather than guessed at: a run
/// that cannot find a file's line asks for it again, which costs a pass and
/// never a wrong line.
///
/// ```
/// use warlock_engine::document::lines_of;
///
/// let page = "\n## Files\n\n\
///     - `reading.rs` (40 B) — The reading half. · declares `read_one`\n\
///     - `table.bin` (4 B) — not text; name and size only\n\
///     \n## Where to look\n\n- a question → `reading.rs` `read_one`\n";
///
/// let lines = lines_of(page);
/// assert_eq!(lines.get("reading.rs").map(String::as_str), Some("The reading half."));
/// assert_eq!(lines.len(), 2, "only `## Files` holds file lines");
/// ```
#[must_use]
pub fn lines_of(document: &str) -> BTreeMap<String, String> {
    let mut lines = BTreeMap::new();
    let mut in_files = false;
    for row in document.lines() {
        if let Some(heading) = row.strip_prefix("## ") {
            in_files = heading.trim() == "Files";
            continue;
        }
        if !in_files {
            continue;
        }
        let Some((path, entry)) = file_row(row) else {
            continue;
        };
        lines.insert(path, entry);
    }
    lines
}

// ``- `name` (12.3 KB) — entry · declares `a`, `b` (+4)``, which is what
// `render` writes and what `the_document_is_laid_out_by_warlock_and_not_by_the_pass`
// pins. Split on the em dash rather than parsed as a whole: a size is warlock's
// own and the declared names are too, so the only part to recover is between
// them.
fn file_row(row: &str) -> Option<(String, String)> {
    let rest = row.strip_prefix("- `")?;
    let (path, rest) = rest.split_once("` (")?;
    let (_, entry) = rest.split_once(") — ")?;
    let entry = entry
        .split_once(" · declares ")
        .map_or(entry, |(before, _)| before);
    let entry = entry.trim();
    if path.is_empty() || entry.is_empty() {
        return None;
    }
    Some((path.to_owned(), entry.to_owned()))
}

pub const FILE_PROMPT: &str = "\
Describe the one file that follows these instructions, and output the filled \
object and nothing else.

The line you write becomes one row of the WARLOCK.md for the directory this \
file sits in. That document is read by a model, not a person, before any \
source file is opened, and its one job is routing: to say what is here and \
which file to open for a given question. Warlock is the tool that lays the \
document out from your answer; it is not the project being described, and its \
name belongs in the line only if the file itself uses it.

\"line\": what the file is and what it holds, naming the types, functions or \
constants a reader would come to it for, spelt as the file spells them. A file \
that is small, generated, or a re-export gets a line saying so. Write about \
the file in its own voice: no first person, nothing about this request or \
about what you were or were not shown, and no guess at what the rest of the \
directory holds — you have not been shown it.

You are given the file's name and size, and its text with function bodies \
elided where that was needed to fit. Where the name and the text disagree, the \
text is right.";

#[must_use]
pub fn file_instructions(path: &str, rejected: &[Defect]) -> String {
    let mut text = FILE_PROMPT.to_owned();
    turned_down(&mut text, rejected);
    let _ = write!(
        text,
        "\n\nBetween {ENTRY_MINIMUM} and {ENTRY_CHARS} characters, on one line.\n\n\
         Return exactly this object, as JSON, with no code fence and nothing before or \
         after it:\n\n{{\"line\": \"\"}}\n\nThe file is `{path}`."
    );
    text
}

/// A defective line is asked for again whole, and [`file_fallback`] is the
/// floor when the asking runs out.
pub fn accept_file(
    answer: &str,
    path: &str,
    expected: &Expected<'_>,
    described: &Described,
) -> Result<String, Vec<Defect>> {
    let field = format!("files[{path:?}]");
    let parsed: serde_json::Value = match serde_json::from_str(answer) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Err(vec![Defect::NotJson {
                detail: error.to_string(),
            }]);
        }
    };
    let Some(line) = parsed.get("line").and_then(serde_json::Value::as_str) else {
        return Err(vec![Defect::Missing { field }]);
    };

    let mut defects = Vec::new();
    self::line(&field, line, ENTRY_MINIMUM, ENTRY_CHARS, &mut defects);
    if !Evidence::new(expected, described).mentions_tool() && names_tool(line) {
        defects.push(Defect::ToolNamed { field });
    }
    if defects.is_empty() {
        Ok(line.trim().to_owned())
    } else {
        Err(defects)
    }
}

/// The line warlock writes itself when a per-file pass never produced one.
///
/// The same builder a mended directory answer falls to, for the same reason:
/// it is the file's own name, size and declared symbols, and nothing that
/// sounds like it read the file.
#[must_use]
pub fn file_fallback(path: &str, expected: &Expected<'_>, described: &Described) -> String {
    fallback::file(path, expected, described)
}

fn parse(answer: &str) -> Result<Fill, Defect> {
    let object = match (answer.find('{'), answer.rfind('}')) {
        (Some(start), Some(end)) if start <= end => &answer[start..=end],
        _ => {
            return Err(Defect::NotJson {
                detail: "no object found in the answer".to_owned(),
            });
        }
    };
    serde_json::from_str(object).map_err(|error| Defect::NotJson {
        detail: error.to_string(),
    })
}

// `files` is never looked at here. Every line in it was checked by `accept_file`
// as it was accepted, and a synthesis request sends no text, so
// `Expected::asked` is empty and keying the lines against it would report every
// one of them as a slot nothing asked for.
fn check(fill: &Fill, expected: &Expected<'_>, described: &Described) -> Vec<Defect> {
    let mut defects = Vec::new();
    let evidence = Evidence::new(expected, described);

    line(
        "purpose",
        &fill.purpose,
        ENTRY_MINIMUM,
        PURPOSE_CHARS,
        &mut defects,
    );

    let children: Vec<&str> = expected.directories.keys().copied().collect();
    keyed("directories", &fill.directories, &children, &mut defects);

    stated("structure", &fill.structure, evidence, &mut defects);

    // Measured, not hypothetical: told it is filling in "the WARLOCK.md" and
    // that "warlock lays the document out", a pass over a crate that never
    // mentions warlock called it "a toy freshness ledger belonging to
    // Warlock". The instructions say the name is the tool's; this is the check
    // behind the sentence, and it stands down the moment the files use it.
    if !evidence.mentions_tool() {
        for (field, value) in values(fill) {
            if names_tool(value) {
                defects.push(Defect::ToolNamed { field });
            }
        }
    }

    defects
}

fn values(fill: &Fill) -> impl Iterator<Item = (String, &str)> {
    std::iter::once(("purpose".to_owned(), fill.purpose.as_str()))
        .chain(
            fill.directories
                .iter()
                .map(|(key, line)| (format!("directories[{key:?}]"), line.as_str())),
        )
        .chain(
            fill.structure
                .iter()
                .enumerate()
                .map(|(index, entry)| (format!("structure[{index}]"), entry.line.as_str())),
        )
}

fn line(field: &str, value: &str, minimum: usize, cap: usize, defects: &mut Vec<Defect>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        defects.push(Defect::Empty {
            field: field.to_owned(),
        });
        return;
    }
    if trimmed.contains(['\n', '\r']) {
        defects.push(Defect::Multiline {
            field: field.to_owned(),
        });
    }
    let chars = trimmed.chars().count();
    if chars < minimum {
        defects.push(Defect::TooShort {
            field: field.to_owned(),
            chars,
            minimum,
        });
    }
    if chars > cap {
        defects.push(Defect::TooLong {
            field: field.to_owned(),
            chars,
            cap,
        });
    }
}

fn keyed(name: &str, given: &BTreeMap<String, String>, wanted: &[&str], defects: &mut Vec<Defect>) {
    for key in wanted {
        if !given.contains_key(*key) {
            defects.push(Defect::Missing {
                field: format!("{name}[{key:?}]"),
            });
        }
    }
    for (key, value) in given {
        // An entry for something the request does not hold cannot reach here:
        // `accept_synthesis` and `mended` take it out before the check, because
        // no answer could make it right.
        debug_assert!(
            wanted.contains(&key.as_str()),
            "{name}[{key:?}] was never asked for"
        );
        let field = format!("{name}[{key:?}]");
        line(&field, value, ENTRY_MINIMUM, ENTRY_CHARS, defects);
    }
}

// The claim is prose and goes unchecked; every name it makes is checked. Two
// other roads were rejected and are worth naming, because both look cheaper
// from a standing start.
//
// Asking the pass to backtick whatever it names, and validating the backticks,
// leaves the check at the mercy of typography: across the six documents this
// repository held when this was written, the claim sections carried eight
// backticked tokens between them, so that check would have read almost nothing
// and reported a clean section.
//
// Reading names back out of the prose instead makes an inconsistent writer the
// authority on what gets checked, and a check that silently matches nothing is
// exactly how a section ends up unverified with a green mark on it.
//
// So a claim carries its own targets, and the rendered document is unchanged by
// any of it: `names` is validated and never printed.
//
// An entry must name something — it is a statement about how the files here
// fit together, and one that names no file is not that statement.
fn stated(name: &str, given: &[Entry], evidence: Evidence<'_>, defects: &mut Vec<Defect>) {
    if given.len() > LIST_CAP {
        defects.push(Defect::TooMany {
            field: name.to_owned(),
            count: given.len(),
            cap: LIST_CAP,
        });
    }
    for (index, entry) in given.iter().enumerate() {
        line(
            &format!("{name}[{index}]"),
            &entry.line,
            ENTRY_MINIMUM,
            ENTRY_CHARS,
            defects,
        );
        if entry.names.is_empty() {
            defects.push(Defect::Empty {
                field: format!("{name}[{index}].names"),
            });
            continue;
        }
        for (which, named) in entry.names.iter().enumerate() {
            let named = named.trim();
            let field = format!("{name}[{index}].names[{which}]");
            if named.is_empty() {
                defects.push(Defect::Empty { field });
            } else if !evidence.knows(named) {
                defects.push(Defect::UnknownTarget {
                    field,
                    name: named.to_owned(),
                });
            }
        }
    }
}

#[must_use]
pub fn render(name: &str, fill: &Fill, expected: &Expected<'_>, described: &Described) -> String {
    let mut text = format!("{STAMP}\n# {name}\n\n{}\n", fill.purpose.trim());

    if !expected.files.is_empty() {
        text.push_str("\n## Files\n\n");
        for (path, (size, shown)) in &expected.files {
            let written = fill
                .files
                .get(*path)
                .map(|line| line.trim())
                .filter(|line| !line.is_empty());
            let entry = written.unwrap_or(match shown {
                Shown::Text(_) => "",
                Shown::NotText => "not text; name and size only",
                Shown::Unsent => "not read by the pass; name and size only",
            });
            let _ = write!(text, "- `{path}` ({}) — {entry}", human(*size));
            if let Some(names) = described
                .declared
                .get(*path)
                .filter(|names| !names.is_empty())
            {
                let listed: Vec<String> = names
                    .iter()
                    .take(DECLARED_SHOWN)
                    .map(|name| format!("`{name}`"))
                    .collect();
                let _ = write!(text, " · declares {}", listed.join(", "));
                if names.len() > DECLARED_SHOWN {
                    let _ = write!(text, " (+{})", names.len() - DECLARED_SHOWN);
                }
            }
            text.push('\n');
        }
    }

    if !expected.directories.is_empty() {
        text.push_str("\n## Directories\n\n");
        for child in expected.directories.keys() {
            let entry = fill.directories.get(*child).map_or("", |line| line.trim());
            let _ = writeln!(text, "- `{child}/` — {entry}");
        }
    }

    list(&mut text, "Structure", &fill.structure);

    text
}

fn list(text: &mut String, heading: &str, entries: &[Entry]) {
    if entries.is_empty() {
        return;
    }
    let _ = write!(text, "\n## {heading}\n\n");
    for entry in entries {
        let _ = writeln!(text, "- {}", entry.line.trim());
    }
}

fn human(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;
    if bytes < KB {
        return format!("{bytes} B");
    }
    let (unit, scale) = if bytes < MB { ("KB", KB) } else { ("MB", MB) };
    let tenths = bytes * 10 / scale;
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
}

// The factual floor under a defective fill: a value assembled from what
// warlock already holds — the name it renders, the size it measured, the
// symbols `languages.rs` extracted and the counts of what is in the directory.
// Nothing here states intent, nothing here is a guess about what a file is
// for, and nothing here comes from a model.
//
// On the tool name. `names_tool` fires on any text holding "warlock" in any
// case, and `check` applies it to every value whenever `Expected::mentions_tool`
// is false, so a file called `warlock.rs` or a directory called `warlock` would
// otherwise make its own fallback line defective — and, under the mend, a line
// that is dropped and rebuilt for ever. Quoting cannot help: the rule is a
// substring test, not a parse. So while the rule is live a fact that names the
// tool is left out of the line rather than reworded — the name is dropped and a
// declared symbol naming the tool is dropped from the list. The document loses
// nothing by it, because `render` prints the path and the declared names itself
// either way. Once the files themselves use the word the rule has stood down
// and every fact goes in whole.
mod fallback {
    use super::{
        DECLARED_SHOWN, Described, ENTRY_CHARS, ENTRY_MINIMUM, Expected, human, names_tool,
    };

    // One bound for all three builders, and it is the tightest of the caps any
    // slot they fill is held to: a purpose may run to `PURPOSE_CHARS`, but a
    // fallback purpose is a name and two counts and has no use for the room.
    const CAP: usize = ENTRY_CHARS;

    // A file entry: its name, its size as `render` prints it, and the symbols
    // `languages.rs` extracted from it.
    //
    // Every one of those facts is already on the line this value is written
    // into — `render` prints ``- `reading.rs` (40 B) — `` in front of it and
    // `· declares `read_one`` after it — so the mended line reads as a
    // restatement of its own surroundings and routes a reader no better than
    // an empty entry would. It is here for the shape and not for the routing:
    // an empty value is `Defect::Empty`, so a fill mended with one would still
    // be defective, and the rendered line would trail off after its em dash.
    // Read off `render` rather than assumed — the name, the size and the first
    // `DECLARED_SHOWN` symbols are printed by `render` itself, on this very
    // line — and the alternative, leaving the entry empty and letting `check`
    // pass it, was rejected on the two grounds above. Do not dress this line up
    // into something that sounds like it read the file.
    pub(super) fn file(path: &str, expected: &Expected<'_>, described: &Described) -> String {
        let guarded = !super::Evidence::new(expected, described).mentions_tool();
        let size = expected.files.get(path).map(|(size, _)| human(*size));
        let named = !(path.is_empty() || guarded && names_tool(path));
        let mut line = match (size, named) {
            (Some(size), true) => format!("A {size} file named `{path}`"),
            (Some(size), false) => format!("A {size} file in this directory"),
            (None, true) => format!("The file `{path}`"),
            (None, false) => "A file in this directory".to_owned(),
        };

        let mut written = 0;
        for name in described
            .declared
            .get(path)
            .into_iter()
            .flatten()
            .filter(|name| !(name.trim().is_empty() || guarded && names_tool(name)))
            .take(DECLARED_SHOWN)
        {
            let piece = if written == 0 {
                format!(", declaring `{name}`")
            } else {
                format!(", `{name}`")
            };
            // The list stops at the cap rather than running into it, so a cut
            // lands between names and not through one.
            if written > 0 && line.chars().count() + piece.chars().count() + 1 > CAP {
                break;
            }
            line.push_str(&piece);
            written += 1;
        }
        if written == 0 {
            line.push_str(", with no symbols extracted from it");
        }
        line.push('.');
        fit(&line, "A file in this directory.")
    }

    // A purpose: the directory's own name and how much is under it. What it is
    // for is exactly what a pass that did not answer never said.
    pub(super) fn purpose(name: &str, expected: &Expected<'_>, described: &Described) -> String {
        let guarded = !super::Evidence::new(expected, described).mentions_tool();
        let subject = if name.is_empty() || (guarded && names_tool(name)) {
            "This directory".to_owned()
        } else {
            format!("`{name}`")
        };
        let line = format!(
            "{subject} holds {} and {}.",
            counted(expected.files.len(), "file", "files"),
            counted(expected.directories.len(), "subdirectory", "subdirectories"),
        );
        fit(&line, "A directory of this repository.")
    }

    // A `directories` entry: the child's name and nothing else. Its own
    // document says what is under it, and restating that here would be a
    // second-hand claim.
    pub(super) fn directory(child: &str, expected: &Expected<'_>, described: &Described) -> String {
        let guarded = !super::Evidence::new(expected, described).mentions_tool();
        let line = if child.is_empty() || (guarded && names_tool(child)) {
            "A subdirectory of this directory.".to_owned()
        } else {
            format!("A subdirectory named `{child}`.")
        };
        fit(&line, "A subdirectory of this directory.")
    }

    fn counted(count: usize, one: &str, many: &str) -> String {
        match count {
            0 => format!("no {many}"),
            1 => format!("1 {one}"),
            _ => format!("{count} {many}"),
        }
    }

    // The shape every fallback value has to hold: one line, at least
    // `ENTRY_MINIMUM` characters and at most the cap, counted as characters and
    // cut on a character boundary so a multibyte name cannot split.
    pub(super) fn fit(line: &str, pad: &str) -> String {
        let mut line = flattened(line);
        // A non-empty pad adds at least one character a turn, so this ends.
        // In practice it runs once or not at all: every pad here is longer than
        // the floor on its own.
        while line.chars().count() < ENTRY_MINIMUM && !pad.is_empty() {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(pad);
        }
        let cut: String = line.chars().take(CAP).collect();
        cut.trim_end().to_owned()
    }

    // Whitespace of any kind collapses to one space: a name or a symbol that
    // carried a newline would otherwise make a one-line value into two.
    fn flattened(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

// Four, and the shape of the worst chain is why. A repair can make a slot
// defective in a new way — an entry cut to the cap can land under
// `ENTRY_MINIMUM`, and a value dropped for naming the tool is then missing —
// so the mend is a fixpoint and not a single sweep: repair, check again,
// repair what the repair left. The longest chain a rule here can start is
// three links (drop, then fall back, then check clean), and a fourth pass is
// the margin. It is a stop and not a schedule: the loop leaves as soon as
// `check` comes back empty, and if it ever did not, an unbounded version would
// spin between two rules instead of writing a document.
pub const MEND_PASSES: usize = 4;

// `field` is the slot in `Defect`'s own spelling — `files["writing.rs"]`,
// `purpose`, `structure` — so a caller can line a mend up against the defect it
// answers without parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mend {
    pub field: String,
    pub done: Mended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mended {
    // The value spanned lines and keeps its first.
    FirstLine,
    // The value ran over its cap and was cut to it, counting characters.
    Cut { from: usize, to: usize },
    // The list ran over `LIST_CAP` and keeps its first entries.
    Shortened { from: usize, to: usize },
    // The entry could not be made right from the model's own text — it names
    // something that is not here, or names the tool — so it is gone.
    Dropped,
    // The slot fell to the factual line: the name, the size and the symbols
    // warlock measured. See [`fallback`].
    Supplied,
}

impl fmt::Display for Mend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = &self.field;
        match self.done {
            Mended::FirstLine => {
                write!(f, "{field} ran to more than one line and keeps its first")
            }
            Mended::Cut { from, to } => {
                write!(f, "{field} was {from} characters and was cut to {to}")
            }
            Mended::Shortened { from, to } => {
                write!(
                    f,
                    "{field} had {from} entries and was cut to the first {to}"
                )
            }
            Mended::Dropped => write!(
                f,
                "{field} could not be repaired from the answer and was dropped"
            ),
            Mended::Supplied => write!(
                f,
                "{field} was not answered and was filled in from what warlock measured"
            ),
        }
    }
}

// The mechanical mend: the floor under an exhausted attempt loop. Every
// [`Defect`] but `NotJson` has a repair here, and none of them reaches a model
// — the evidence is the answer's own text, the file names and sizes warlock
// measured and the symbols `languages.rs` extracted. A fill that comes back
// from here is not defective: `check` over it is empty, which is the property
// the loop below exists to hold.
#[must_use]
pub fn mend(fill: &Fill, expected: &Expected<'_>, described: &Described) -> (Fill, Vec<Mend>) {
    let (fill, mends, _) = mended(fill, expected, described);
    (fill, mends)
}

// The same operation, saying how many passes it took. Private because the
// count is a fact about this function and not about the document; the test for
// the bound is the only caller that has any use for it.
fn mended(fill: &Fill, expected: &Expected<'_>, described: &Described) -> (Fill, Vec<Mend>, usize) {
    let mut fill = fill.clone();
    fill.directories
        .retain(|key, _| expected.directories.contains_key(key.as_str()));

    let mut mends = Vec::new();
    let mut passes = 0;
    for _ in 0..MEND_PASSES {
        let defects = check(&fill, expected, described);
        if defects.is_empty() {
            break;
        }
        passes += 1;
        sweep(&mut fill, &defects, expected, described, &mut mends);
    }

    (fill, mends, passes)
}

// One pass of the fixpoint. The order inside it is what keeps a list's indices
// meaning what the defects say they mean: what to drop and what to fill is
// decided first, then the values that survive are rewritten in place, and only
// then does anything move — so a defect naming `structure[3]` is never applied
// to whatever slid into position 3.
fn sweep(
    fill: &mut Fill,
    defects: &[Defect],
    expected: &Expected<'_>,
    described: &Described,
    mends: &mut Vec<Mend>,
) {
    let mut plan = Plan::default();
    // Drops before fills before rewrites. An entry that names something which
    // is not here goes whatever else is wrong with it, so deciding that first
    // keeps the pass from recording a fill or a cut it then undoes.
    for defect in defects {
        plan.note_drop(defect, mends);
    }
    for defect in defects {
        plan.note_fill(defect, mends);
    }
    for defect in defects {
        let (field, done) = match defect {
            Defect::Multiline { field } => (field, Mended::FirstLine),
            Defect::TooLong { field, chars, cap } => (
                field,
                Mended::Cut {
                    from: *chars,
                    to: *cap,
                },
            ),
            _ => continue,
        };
        // A slot already on its way out, or already being filled in whole, is
        // not worth rewriting first: the record would name work the same pass
        // undoes.
        if plan.covers(field) {
            continue;
        }
        let Some(value) = target(fill, field) else {
            continue;
        };
        match done {
            // The first line of the value as the check reads it: `line`
            // measures the trimmed value, so a value that opens with a blank
            // line keeps the first line of what was actually written.
            Mended::FirstLine => {
                *value = value.trim().lines().next().unwrap_or_default().to_owned();
            }
            // Characters, not bytes, and so on a character boundary. No trim:
            // the cut lands where it lands, and `render` trims on the way out.
            Mended::Cut { to, .. } => *value = value.chars().take(to).collect(),
            _ => {}
        }
        mends.push(Mend {
            field: field.clone(),
            done,
        });
    }
    plan.carry_out(fill, expected, described);
}

#[derive(Debug, Default)]
struct Plan {
    purpose: bool,
    filled_directories: BTreeSet<String>,
    dropped_directories: BTreeSet<String>,
    dropped_structure: BTreeSet<usize>,
    // The lists to cut back to `LIST_CAP`, by the names `check` and `Slot`
    // spell them.
    cut: BTreeSet<&'static str>,
}

impl Plan {
    // What goes: an entry naming a file, a subdirectory or a symbol that is
    // not here, an entry naming the tool, and whatever hangs off the end of a
    // list over its cap. A claim has no cut and no fallback — there is nothing
    // factual to put in its place — so the entry is dropped and, where the
    // slot is a keyed one, the next pass finds it missing and fills it in.
    fn note_drop(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let field = match defect {
            Defect::UnknownTarget { field, .. } | Defect::ToolNamed { field } => field,
            Defect::TooMany { field, count, cap } => {
                let Slot::List(list) = slot(field) else {
                    return;
                };
                self.cut.insert(list);
                mends.push(Mend {
                    field: field.clone(),
                    done: Mended::Shortened {
                        from: *count,
                        to: *cap,
                    },
                });
                return;
            }
            _ => return,
        };
        let (recorded, done) = match slot(field) {
            // The purpose is the one slot a drop cannot answer: a document
            // with no purpose is not a document, and there is no next rule
            // under it. So it falls to the fallback line here too, which is
            // built not to name the tool.
            Slot::Purpose => (
                !std::mem::replace(&mut self.purpose, true),
                Mended::Supplied,
            ),
            Slot::Directory(key) => (self.dropped_directories.insert(key), Mended::Dropped),
            // A list entry has no name, size or symbols behind it, and the
            // prompt says an empty list is fine, so there is nothing to fall
            // back to and nothing lost by the gap.
            Slot::Entry(index) => (self.dropped_structure.insert(index), Mended::Dropped),
            Slot::List(_) | Slot::Unknown => (false, Mended::Dropped),
        };
        if recorded {
            mends.push(Mend {
                field: field.clone(),
                done,
            });
        }
    }

    // What was never answered: a key left out, a value left empty, a value
    // too short to hold a fact. Warlock says what it measured instead.
    fn note_fill(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let (Defect::Missing { field } | Defect::Empty { field } | Defect::TooShort { field, .. }) =
            defect
        else {
            return;
        };
        let (recorded, done) = match slot(field) {
            Slot::Purpose => (
                !std::mem::replace(&mut self.purpose, true),
                Mended::Supplied,
            ),
            Slot::Directory(key) => (
                !self.dropped_directories.contains(&key) && self.filled_directories.insert(key),
                Mended::Supplied,
            ),
            // A list entry is dropped rather than filled, and says so.
            Slot::Entry(index) => (self.dropped_structure.insert(index), Mended::Dropped),
            Slot::List(_) | Slot::Unknown => (false, Mended::Dropped),
        };
        if recorded {
            mends.push(Mend {
                field: field.clone(),
                done,
            });
        }
    }

    // Whether the slot a value rewrite names is already being dropped, filled
    // in whole, or cut off the end of its list this pass.
    fn covers(&self, field: &str) -> bool {
        match slot(field) {
            Slot::Purpose => self.purpose,
            Slot::Directory(key) => {
                self.dropped_directories.contains(&key) || self.filled_directories.contains(&key)
            }
            Slot::Entry(index) => {
                self.dropped_structure.contains(&index) || self.cut_off("structure", index)
            }
            Slot::List(_) | Slot::Unknown => false,
        }
    }

    // Whether an entry is past the cap of a list this pass cuts back, and so
    // about to go anyway.
    fn cut_off(&self, list: &'static str, index: usize) -> bool {
        index >= LIST_CAP && self.cut.contains(list)
    }

    fn carry_out(self, fill: &mut Fill, expected: &Expected<'_>, described: &Described) {
        if self.purpose {
            fill.purpose = fallback::purpose(&expected.name, expected, described);
        }
        for key in &self.filled_directories {
            fill.directories
                .insert(key.clone(), fallback::directory(key, expected, described));
        }
        fill.directories
            .retain(|key, _| !self.dropped_directories.contains(key));

        // Dropped by the index the defects named, then cut to the cap: both
        // read the same pre-pass list, so they are applied in that order and
        // an index is never used after the list has moved under it.
        drop_indexes(&mut fill.structure, &self.dropped_structure);
        if self.cut.contains("structure") {
            fill.structure.truncate(LIST_CAP);
        }
    }
}

fn drop_indexes<T>(list: &mut Vec<T>, dropped: &BTreeSet<usize>) {
    if dropped.is_empty() {
        return;
    }
    let mut index = 0;
    list.retain(|_| {
        let keep = !dropped.contains(&index);
        index += 1;
        keep
    });
}

// The slot a defect's `field` names, read back out of the spelling `check`
// wrote it in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Purpose,
    Directory(String),
    List(&'static str),
    Entry(usize),
    Unknown,
}

fn slot(field: &str) -> Slot {
    match field {
        "purpose" => return Slot::Purpose,
        "structure" => return Slot::List("structure"),
        _ => {}
    }
    let Some((head, rest)) = field.split_once('[') else {
        return Slot::Unknown;
    };
    let inside = rest.split_once(']').map_or(rest, |(inside, _)| inside);
    match head {
        // `check` writes a key with `{key:?}`, which is JSON's own escaping of
        // a string, so serde reads it back.
        "directories" => {
            let Ok(key) = serde_json::from_str::<String>(inside) else {
                return Slot::Unknown;
            };
            Slot::Directory(key)
        }
        "structure" => {
            let Ok(index) = inside.parse::<usize>() else {
                return Slot::Unknown;
            };
            Slot::Entry(index)
        }
        _ => Slot::Unknown,
    }
}

// The value a rewrite writes over.
fn target<'f>(fill: &'f mut Fill, field: &str) -> Option<&'f mut String> {
    match slot(field) {
        Slot::Purpose => Some(&mut fill.purpose),
        Slot::Directory(key) => fill.directories.get_mut(&key),
        Slot::Entry(index) => fill.structure.get_mut(index).map(|e| &mut e.line),
        Slot::List(_) | Slot::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Accepted, Defect, Described, ENTRY_CHARS, ENTRY_MINIMUM, Entry, Evidence, Expected,
        FILE_PROMPT, Fill, LIST_CAP, MEND_PASSES, Mend, Mended, PURPOSE_CHARS, STAMP,
        SYNTHESIS_PROMPT, accept_file, accept_synthesis, check, fallback, human, lines_of, mend,
        mended, names_tool, render, stub_answer, synthesis_instructions,
    };
    use std::collections::{BTreeMap, BTreeSet};

    use crate::agent::{ChildDocument, File, Request};

    fn request() -> Request {
        Request::new("describe", "/repo/crates/engine")
            .with_files([
                File::present("lib.rs", *b"pub mod pact;\npub fn subtree_hash() {}\n"),
                File::present("Cargo.toml", *b"[package]\nname = \"engine\"\n"),
                File::elided(
                    "app.rs",
                    900_000,
                    "pub fn draw() {}\n… 40000 lines elided …\n",
                ),
                File::omitted("Cargo.lock", 4_200_000),
                File::present("logo.png", vec![0x89, b'P', b'N', b'G', 0xff]),
            ])
            .with_child_documents([ChildDocument::new(
                "src",
                "# src\n\n## Where to look\n\n- hashing → `hash.rs` `subtree_hash`\n",
            )])
    }

    fn good() -> Fill {
        let mut fill = Fill::stub(&request());
        fill.purpose = "The engine crate: pacts, hashes and the manifest.".to_owned();
        fill.structure = vec![Entry::naming(
            "`lib.rs` re-exports `pact` for the crate.",
            "lib.rs",
        )];
        fill
    }

    fn defects(fill: &Fill) -> Vec<Defect> {
        let request = request();
        check(fill, &Expected::of(&request), &Described::default())
    }

    #[test]
    fn a_synthesis_answer_that_fills_files_anyway_is_taken_at_its_lines() {
        // The model was asked for everything but the file lines, and answered
        // with them too. They are not a defect and they are not kept: the
        // document's `## Files` is the lines that were assembled and checked
        // one at a time, and an entry nothing asked for has no question behind
        // it to ask again.
        let request = Request::new("synthesise", "/repo/crates/engine")
            .with_files([File::omitted("lib.rs", 38)])
            .with_child_documents([ChildDocument::new("src", "# src\n\nA document below.\n")]);
        let lines = [(
            "lib.rs".to_owned(),
            "The crate root, and what it re-exports.".to_owned(),
        )]
        .into_iter()
        .collect();

        let answer = Fill {
            purpose: "The engine crate: pacts, hashes and the manifest.".to_owned(),
            directories: [(
                "src".to_owned(),
                "Where the hashing and the manifest live.".to_owned(),
            )]
            .into_iter()
            .collect(),
            files: [(
                "lib.rs".to_owned(),
                "a line the synthesis pass was never asked for".to_owned(),
            )]
            .into_iter()
            .collect(),
            ..Fill::default()
        };

        let accepted = accept_synthesis(
            &answer.to_json(),
            &lines,
            &Expected::of(&request),
            &Described::default(),
        );

        let Accepted::Filled(fill) = accepted else {
            panic!("the answer was turned down: {accepted:?}");
        };
        assert_eq!(fill.files, lines, "the assembled lines, not the answer's");
    }

    #[test]
    fn a_stub_is_accepted_for_any_request() {
        // As a pass sees it: the prompt a pass runs under is what tells a
        // stand-in which shape to answer in.
        let request = request();
        let expected = Expected::of(&request);
        let synthesis = request.clone().with_prompt(SYNTHESIS_PROMPT);
        let lines = BTreeMap::new();
        assert!(matches!(
            accept_synthesis(
                &stub_answer(&synthesis),
                &lines,
                &expected,
                &Described::default()
            ),
            Accepted::Filled(_)
        ));

        let one = Request::new(FILE_PROMPT, "/repo/crates/engine")
            .with_files([File::present("lib.rs", *b"pub mod pact;\n")]);
        accept_file(
            &stub_answer(&one),
            "lib.rs",
            &Expected::of(&one),
            &Described::default(),
        )
        .expect("accepted too");

        // Anything that is neither kind of pass gets plain prose.
        assert!(!stub_answer(&request).trim_start().starts_with('{'));
    }

    #[test]
    fn a_good_fill_is_accepted() {
        assert_eq!(defects(&good()), []);
    }

    #[test]
    fn an_answer_wrapped_in_a_fence_or_a_sentence_is_still_read() {
        let request = request();
        let expected = Expected::of(&request);
        let json = good().to_json();
        for wrapped in [
            format!("```json\n{json}\n```"),
            format!("Here is the object:\n\n{json}\n\nLet me know if you need more."),
        ] {
            let accepted =
                accept_synthesis(&wrapped, &BTreeMap::new(), &expected, &Described::default());
            assert!(
                matches!(accepted, Accepted::Filled(_)),
                "read from the outermost braces: {accepted:?}"
            );
        }
    }

    #[test]
    fn an_answer_that_is_not_an_object_is_one_defect() {
        let request = request();
        let expected = Expected::of(&request);
        for answer in [
            "",
            "no.",
            "# engine\n\nProse about the directory.",
            "[1, 2]",
            "{",
        ] {
            let outcome =
                accept_synthesis(answer, &BTreeMap::new(), &expected, &Described::default());
            assert!(
                matches!(outcome, Accepted::Unparsed(Defect::NotJson { .. })),
                "{answer:?}: {outcome:?}"
            );
        }
    }

    #[test]
    fn a_bare_word_is_a_skipped_slot_and_not_an_entry() {
        // Measured: a pass answered `"duplicate"` for `clock.rs`, one word with
        // no fact in it, and every check of the day passed.
        let request = request();
        assert_eq!(
            accept_file(
                r#"{"line": "duplicate"}"#,
                "lib.rs",
                &Expected::of(&request),
                &Described::default()
            ),
            Err(vec![Defect::TooShort {
                field: "files[\"lib.rs\"]".to_owned(),
                chars: 9,
                minimum: ENTRY_MINIMUM
            }])
        );
    }

    #[test]
    fn every_value_is_one_line_under_its_cap() {
        let mut fill = good();
        fill.purpose = "   ".to_owned();
        fill.directories.insert(
            "src".to_owned(),
            "two\nlines, and long enough besides".to_owned(),
        );
        fill.structure = vec![Entry::naming("x".repeat(ENTRY_CHARS + 1), "lib.rs")];
        let found = defects(&fill);
        assert!(found.contains(&Defect::Empty {
            field: "purpose".to_owned()
        }));
        assert!(found.contains(&Defect::Multiline {
            field: "directories[\"src\"]".to_owned()
        }));
        assert!(found.contains(&Defect::TooLong {
            field: "structure[0]".to_owned(),
            chars: ENTRY_CHARS + 1,
            cap: ENTRY_CHARS
        }));
        assert_eq!(found.len(), 3, "{found:?}");

        let request = request();
        assert_eq!(
            accept_file(
                &format!("{{\"line\": \"{}\"}}", "x".repeat(ENTRY_CHARS + 1)),
                "lib.rs",
                &Expected::of(&request),
                &Described::default()
            ),
            Err(vec![Defect::TooLong {
                field: "files[\"lib.rs\"]".to_owned(),
                chars: ENTRY_CHARS + 1,
                cap: ENTRY_CHARS
            }]),
            "a file's line is held to the same cap by the pass that writes it"
        );
    }

    #[test]
    fn the_purpose_has_its_own_longer_cap() {
        let mut fill = good();
        fill.purpose = "p".repeat(PURPOSE_CHARS);
        assert_eq!(defects(&fill), []);
        fill.purpose.push('p');
        assert_eq!(defects(&fill).len(), 1);
    }

    #[test]
    fn a_list_over_the_cap_is_a_defect_and_so_is_each_bad_entry_in_it() {
        let mut fill = good();
        fill.structure =
            vec![Entry::naming("an entry long enough to count", "lib.rs"); LIST_CAP + 1];
        fill.structure[3] = Entry::naming(String::new(), "lib.rs");
        assert_eq!(
            defects(&fill),
            [
                Defect::TooMany {
                    field: "structure".to_owned(),
                    count: LIST_CAP + 1,
                    cap: LIST_CAP
                },
                Defect::Empty {
                    field: "structure[3]".to_owned()
                }
            ]
        );
    }

    #[test]
    fn every_line_render_writes_is_a_line_lines_of_reads_back() {
        // The round trip the store rests on. If `render` ever writes a file row
        // this cannot read, a run takes that file to be unrecorded and pays for
        // a pass it did not need — which is why this compares against the fill
        // rather than against a string written out by hand here.
        let request = request();
        let expected = Expected::of(&request);
        let described = Described {
            declared: [("app.rs".to_owned(), vec!["draw".to_owned()])]
                .into_iter()
                .collect(),
            ..Described::default()
        };
        let fill = good();

        let read = lines_of(&render("engine", &fill, &expected, &described));

        for (path, entry) in &fill.files {
            assert_eq!(
                read.get(path).map(String::as_str),
                Some(entry.trim()),
                "`{path}` did not survive the page",
            );
        }
        // The two rows no pass wrote — a file that is not text and one that was
        // never sent — are read back as well. They are warlock's own words and
        // it would write them again identically, so there is nothing to gain by
        // telling them apart here.
        assert_eq!(read.len(), expected.files.len(), "{read:?}");
    }

    #[test]
    fn a_row_that_is_not_a_file_line_is_left_out_rather_than_guessed_at() {
        let read = lines_of(
            "\n## Files\n\n\
             - `reading.rs` (40 B) — The reading half.\n\
             - a row with no backticks at all\n\
             - `writing.rs` (16 B)\n\
             - `shared.rs` (2 B) — \n",
        );

        assert_eq!(read.keys().collect::<Vec<_>>(), ["reading.rs"]);
    }

    #[test]
    fn a_claim_naming_something_that_is_not_here_is_a_defect() {
        let mut fill = good();
        fill.structure = vec![Entry {
            line: "`load_tree` walks the repository from the crate root.".to_owned(),
            names: vec!["load_tree".to_owned()],
        }];

        assert_eq!(
            defects(&fill),
            [Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "load_tree".to_owned(),
            }],
            "no file in the request holds that name, so the claim is not checkable",
        );
    }

    #[test]
    fn a_claim_names_a_file_a_child_or_a_word_some_file_here_actually_holds() {
        let mut fill = good();
        for name in ["lib.rs", "src", "subtree_hash", "pact"] {
            fill.structure = vec![Entry::naming(
                "a fact about this directory, spelt out",
                name,
            )];
            assert_eq!(defects(&fill), [], "`{name}` is evidenced by the request");
        }
    }

    #[test]
    fn a_name_the_request_cannot_vouch_for_is_taken_from_what_warlock_measured() {
        // A file the request could not carry — too big for the budget, or a
        // pass shown assembled lines rather than source. The request holds the
        // name and none of the text, so the claim's symbol is unfindable there.
        let request = Request::new("describe", "/repo/crates/engine")
            .with_files([File::omitted("huge.rs", 9_000_000)]);
        let expected = Expected::of(&request);
        let mut fill = Fill::stub(&request);
        fill.purpose = "The engine crate, in one line about what it is for.".to_owned();
        fill.structure = vec![Entry {
            line: "`huge.rs` hands every row through `walk_one_deep`.".to_owned(),
            names: vec!["walk_one_deep".to_owned()],
        }];

        assert_eq!(
            check(&fill, &expected, &Described::default()),
            [Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "walk_one_deep".to_owned(),
            }],
            "nothing witnesses the name: not the request, and nothing measured",
        );

        let measured = Described {
            declared: [("huge.rs".to_owned(), vec!["walk_one_deep".to_owned()])]
                .into_iter()
                .collect(),
            ..Described::default()
        };
        assert_eq!(
            check(&fill, &expected, &measured),
            [],
            "warlock walked the directory and found the name, which is evidence \
             whatever the request ended up carrying",
        );
    }

    #[test]
    fn a_name_neither_witness_has_ever_seen_is_still_refused() {
        let request = request();
        let expected = Expected::of(&request);
        let measured = Described {
            declared: [("lib.rs".to_owned(), vec!["subtree_hash".to_owned()])]
                .into_iter()
                .collect(),
            ..Described::default()
        };
        let mut fill = good();
        fill.structure = vec![Entry {
            line: "`lib.rs` hands the tree to `load_tree` on the way past.".to_owned(),
            names: vec!["load_tree".to_owned()],
        }];

        assert_eq!(
            check(&fill, &expected, &measured),
            [Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "load_tree".to_owned(),
            }],
            "a second witness widens the evidence and does not retire the check",
        );
    }

    #[test]
    fn a_structure_entry_must_name_something() {
        let mut fill = good();
        fill.structure = vec![Entry::of("The files here fit together somehow.")];
        assert_eq!(
            defects(&fill),
            [Defect::Empty {
                field: "structure[0].names".to_owned(),
            }],
            "a statement about how files fit together that names no file is not that statement",
        );
    }

    #[test]
    fn an_entry_answered_as_a_bare_string_is_read_rather_than_refused() {
        let request = request();
        let expected = Expected::of(&request);
        let mut fill = good();
        fill.structure.clear();
        let answer = fill.to_json().replace(
            "\"structure\": []",
            "\"structure\": [\"a line where an object was asked for\"]",
        );

        let accepted = accept_synthesis(&answer, &fill.files, &expected, &Described::default());

        // Not `NotJson`: that is the one defect with nothing to repair from, and
        // spending a whole pass on a model's punctuation is what this avoids.
        assert_eq!(
            accepted,
            Accepted::Defective {
                fill: Fill {
                    structure: vec![Entry::of("a line where an object was asked for")],
                    ..fill
                },
                defects: vec![Defect::Empty {
                    field: "structure[0].names".to_owned(),
                }],
            },
        );
    }

    #[test]
    fn the_names_a_claim_carries_are_checked_and_never_written_out() {
        let request = request();
        let expected = Expected::of(&request);
        let described = Described::default();
        let mut bare = good();
        bare.structure = vec![Entry {
            line: "`lib.rs` re-exports `pact` for the crate.".to_owned(),
            names: vec![
                "lib.rs".to_owned(),
                "pact".to_owned(),
                "subtree_hash".to_owned(),
            ],
        }];

        assert_eq!(
            render("engine", &bare, &expected, &described),
            render("engine", &good(), &expected, &described),
            "the document is the same bytes whatever a claim names: `names` is \
             evidence for the check and never reaches the page",
        );
    }

    #[test]
    fn naming_the_tool_is_a_defect_unless_the_files_name_it() {
        let mut fill = good();
        fill.purpose = "A toy ledger belonging to Warlock.".to_owned();
        fill.structure = vec![Entry::naming(
            "warlock-style grants, one per module",
            "lib.rs",
        )];
        assert_eq!(
            defects(&fill),
            [
                Defect::ToolNamed {
                    field: "purpose".to_owned()
                },
                Defect::ToolNamed {
                    field: "structure[0]".to_owned()
                },
            ]
        );

        // A child's document always carries the stamp, and the stamp is not the
        // files naming the tool.
        let parent = Request::new("describe", "/repo")
            .with_child_documents([ChildDocument::new("src", format!("{STAMP}\n# src\n"))]);
        let mut fill = Fill::stub(&parent);
        fill.purpose = "Warlock's source, in one crate.".to_owned();
        assert!(matches!(
            check(&fill, &Expected::of(&parent), &Described::default()).as_slice(),
            [Defect::ToolNamed { .. }]
        ));

        // Warlock's own repository names itself, and may say so.
        let own = Request::new("describe", "/repo")
            .with_files([File::present("lib.rs", *b"//! Core engine for warlock.\n")]);
        let mut fill = Fill::stub(&own);
        fill.purpose = "Warlock's engine crate, in one line.".to_owned();
        assert_eq!(check(&fill, &Expected::of(&own), &Described::default()), []);
    }

    #[test]
    fn the_document_is_laid_out_by_warlock_and_not_by_the_pass() {
        let request = request();
        let expected = Expected::of(&request);
        let described = Described {
            declared: [
                (
                    "lib.rs".to_owned(),
                    vec!["pact".to_owned(), "subtree_hash".to_owned()],
                ),
                (
                    "app.rs".to_owned(),
                    // Two past the cap, so the line below pins the truncation
                    // itself rather than the number it happens to sit at.
                    (0..super::DECLARED_SHOWN + 2)
                        .map(|i| format!("draw{i}"))
                        .collect(),
                ),
            ]
            .into_iter()
            .collect(),
            ..Described::default()
        };
        let text = render("engine", &good(), &expected, &described);
        assert!(text.starts_with(STAMP), "{text}");
        assert_eq!(
            text.strip_prefix(STAMP).expect("stamped"),
            "\n# engine\n\n\
             The engine crate: pacts, hashes and the manifest.\n\
             \n## Files\n\n\
             - `Cargo.lock` (4.0 MB) — not read by the pass; name and size only\n\
             - `Cargo.toml` (26 B) — a stand-in entry, filled by a test double\n\
             - `app.rs` (878.9 KB) — a stand-in entry, filled by a test double · declares `draw0`, `draw1`, `draw2`, `draw3`, `draw4`, `draw5`, `draw6`, `draw7`, `draw8`, `draw9`, `draw10`, `draw11`, `draw12`, `draw13`, `draw14`, `draw15` (+2)\n\
             - `lib.rs` (39 B) — a stand-in entry, filled by a test double · declares `pact`, `subtree_hash`\n\
             - `logo.png` (5 B) — not text; name and size only\n\
             \n## Directories\n\n\
             - `src/` — a stand-in entry, filled by a test double\n\
             \n## Structure\n\n\
             - `lib.rs` re-exports `pact` for the crate.\n"
        );
    }

    #[test]
    fn an_empty_list_is_no_section_at_all() {
        let bare = Request::new("describe", "/repo/empty");
        let mut fill = Fill::stub(&bare);
        fill.purpose = "Nothing here yet, but the directory exists.".to_owned();
        let text = render("empty", &fill, &Expected::of(&bare), &Described::default());
        assert_eq!(
            text.strip_prefix(STAMP),
            Some("\n# empty\n\nNothing here yet, but the directory exists.\n")
        );
        for heading in ["## Files", "## Directories", "## Structure", "## Where"] {
            assert!(!text.contains(heading), "{text}");
        }
    }

    #[test]
    fn values_are_trimmed_on_the_way_out_and_not_on_the_way_in() {
        let request = request();
        let mut fill = good();
        fill.directories
            .insert("src".to_owned(), "  padded, but long enough  ".to_owned());
        assert_eq!(defects(&fill), [], "padding is not a defect");
        fill.files.insert(
            "lib.rs".to_owned(),
            "  padded, but long enough  ".to_owned(),
        );
        let text = render(
            "engine",
            &fill,
            &Expected::of(&request),
            &Described::default(),
        );
        assert!(
            text.contains("- `lib.rs` (39 B) — padded, but long enough\n"),
            "{text}"
        );
        assert!(
            text.contains("- `src/` — padded, but long enough\n"),
            "{text}"
        );
    }

    fn writing(files: [(&str, &str); 2]) -> Described {
        Described {
            tokens: files
                .into_iter()
                .map(|(path, text)| {
                    (
                        path.to_owned(),
                        super::identifiers(text).map(str::to_owned).collect(),
                    )
                })
                .collect(),
            ..Described::default()
        }
    }

    #[test]
    fn the_road_without_the_text_checks_no_more_weakly_than_the_road_with_it() {
        // The invariant six bugs broke, in one test. Every one of them had the
        // same shape: a question about the *directory* answered from the
        // *request*. Before per-file a request carried every file's text, so
        // `Expected`'s witnesses were complete and the two were the same thing.
        // A synthesis request carries names and sizes, so each of those
        // witnesses silently answers "nothing", and every caller that trusted
        // one changed behaviour without a single test failing.
        //
        // So: the same directory, measured the same way, must reach the same
        // verdict whether or not the text went over. A new witness-shaped
        // question on `Expected` that forgets `Described` fails here.
        // `subtree_hash` is declared; `Digest` and `finalize` are only
        // *written*, which is the Java-method case — a prefix table reads no
        // declaration for them, so only the tokens witness them. Both kinds are
        // named below, so reverting either witness fails this test.
        let source = "pub fn subtree_hash() -> Digest {\n    hasher.finalize()\n}\n\
                      // Warlock grants a pact over a directory.\n";
        let described = Described {
            declared: [("hash.rs".to_owned(), vec!["subtree_hash".to_owned()])]
                .into_iter()
                .collect(),
            tokens: [(
                "hash.rs".to_owned(),
                super::identifiers(source).map(str::to_owned).collect(),
            )]
            .into_iter()
            .collect(),
        };

        let with_text = Request::new("describe", "/repo/crates/warlock-engine/src")
            .with_files([File::present("hash.rs", source.as_bytes().to_vec())]);
        let without = Request::new("synthesise", "/repo/crates/warlock-engine/src")
            .with_files([File::omitted("hash.rs", source.len() as u64)]);

        // The file lines are the one honest difference: the road with the text
        // is asked for them, the road without has them already. Everything
        // below the `## Files` line is what has to agree.
        let directory_wide = Fill {
            purpose: "Hashing for warlock's pacts: one digest per subtree.".to_owned(),
            structure: vec![
                Entry::naming(
                    "`subtree_hash` digests everything at and below a directory.",
                    "subtree_hash",
                ),
                Entry::naming("The digest comes back as a `Digest`.", "Digest"),
                Entry::naming("A digest is closed out by `finalize`.", "finalize"),
            ],
            ..Fill::default()
        };
        let lined = Fill {
            files: [(
                "hash.rs".to_owned(),
                "Digests a directory and everything below it.".to_owned(),
            )]
            .into_iter()
            .collect(),
            ..directory_wide.clone()
        };

        assert_eq!(
            check(&lined, &Expected::of(&with_text), &described),
            [],
            "the road with the text turned down a true claim"
        );
        assert_eq!(
            check(&directory_wide, &Expected::of(&without), &described),
            [],
            "the road without the text is weaker: a name, a symbol, or the \
             tool's own name went unwitnessed"
        );

        // And neither road is merely permissive: a name the directory does not
        // hold is refused on both.
        let invented = Fill {
            structure: vec![Entry::naming(
                "`load_tree` reads the repository.",
                "load_tree",
            )],
            ..directory_wide
        };
        let refused = [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "load_tree".to_owned(),
        }];
        assert_eq!(
            check(&invented, &Expected::of(&without), &described),
            refused
        );
        assert_eq!(
            check(
                &Fill {
                    files: lined.files.clone(),
                    ..invented
                },
                &Expected::of(&with_text),
                &described
            ),
            refused,
        );
    }

    #[test]
    fn a_leaf_that_writes_the_tools_name_may_say_so() {
        // Found on warlock's own repository. `Expected::mentions_tool` reads
        // `Shown::Text`, and a synthesis request carries no text — so a leaf,
        // having no child document either, always read as never mentioning
        // warlock and had every honest claim about it refused. The two leaf
        // `src` directories lost nine claims between them; their parents, which
        // do have child documents, lost none.
        let request = Request::new("synthesise", "/repo/crates/warlock-engine/src")
            .with_files([File::omitted("pact.rs", 900)]);
        let expected = Expected::of(&request);
        assert!(
            !Evidence::new(&expected, &Described::default()).mentions_tool(),
            "the request alone cannot know, which is the whole problem"
        );

        let described = writing([
            (
                "pact.rs",
                "// Warlock grants a pact over a directory.\npub fn pact_subtree() {}",
            ),
            ("hash.rs", "pub fn subtree_hash() {}"),
        ]);
        let fill = Fill {
            purpose: "Pacts and refreshes a subtree, recording what warlock granted.".to_owned(),
            ..Fill::default()
        };

        assert_eq!(
            check(&fill, &expected, &described),
            [],
            "a directory whose own files write the name may use it"
        );
        assert_eq!(
            check(&fill, &expected, &Described::default()),
            [Defect::ToolNamed {
                field: "purpose".to_owned(),
            }],
            "and one whose files never mention it still may not",
        );
    }

    #[test]
    fn a_name_a_file_writes_witnesses_a_claim_the_table_cannot_read() {
        // The case the token witness exists for. A prefix table reads
        // `public class Invoice` and stops: strip the visibility off a Java
        // method and `long total()` opens with a type, which matches no row. So
        // `declared` holds only `Invoice`, and on the per-file road — where no
        // file's text is in the request — a true claim about `total` had no
        // witness at all and was dropped out of the document.
        let request = Request::new("synthesise", "/repo/services/billing")
            .with_files([File::omitted("Invoice.java", 400)]);
        let expected = Expected::of(&request);
        let described = writing([
            (
                "Invoice.java",
                "public class Invoice { public long total() {} }",
            ),
            ("Ledger.kt", "class Ledger { fun sum(): Long = 0 }"),
        ]);

        let fill = Fill {
            purpose: "Billing types: invoices, ledgers and what they total.".to_owned(),
            structure: vec![Entry::naming(
                "`total` sums the lines of an invoice.",
                "total",
            )],
            ..Fill::default()
        };

        assert_eq!(
            check(&fill, &expected, &described),
            [],
            "a name the file plainly writes was refused"
        );
        assert_eq!(
            check(&fill, &expected, &Described::default()),
            [Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "total".to_owned(),
            }],
            "and without the measurement there is nothing to witness it",
        );
    }

    #[test]
    fn a_name_merely_contained_in_a_longer_one_is_not_witnessed() {
        // Stricter than the witness it replaces, on purpose. The old road
        // accepted any name a file's text *contained*, so a file full of
        // `AuditApplyer` witnessed a claim about `Apply` — a symbol that is not
        // there.
        let request = Request::new("synthesise", "/repo/monolith")
            .with_files([File::omitted("audit.go", 900)]);
        let described = writing([
            ("audit.go", "type AuditApplyer struct{}"),
            ("other.go", "package monolith"),
        ]);
        let fill = Fill {
            purpose: "Audit persistence wrappers, one per verb, over a shared store.".to_owned(),
            structure: vec![Entry::naming("`Apply` stamps and saves.", "Apply")],
            ..Fill::default()
        };

        assert_eq!(
            check(&fill, &Expected::of(&request), &described),
            [Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "Apply".to_owned(),
            }]
        );
    }

    #[test]
    fn a_qualified_name_is_met_by_a_file_writing_both_halves() {
        // Elixir and the like name a module and a function together. Cutting
        // the claim into identifiers the same way the file was cut is what lets
        // `Pipeline.Stage` be witnessed without loosening the check back into
        // substring containment.
        let request = Request::new("synthesise", "/repo/pipeline")
            .with_files([File::omitted("stage.ex", 300)]);
        let described = writing([
            (
                "stage.ex",
                "defmodule Pipeline.Stage do\n  def sum(items), do: items\nend",
            ),
            ("stage_test.exs", "defmodule Pipeline.StageTest do\nend"),
        ]);
        let fill = Fill {
            purpose: "The pipeline stage that sums the items handed to it.".to_owned(),
            structure: vec![Entry::naming(
                "`Pipeline.Stage` sums the items it is given.",
                "Pipeline.Stage",
            )],
            ..Fill::default()
        };

        assert_eq!(check(&fill, &Expected::of(&request), &described), []);
    }

    #[test]
    fn a_mend_keeps_the_lines_the_run_paid_for() {
        // The synthesis road, which is the only road: every file is in the
        // request by name and size, none by text, so `Expected::asked` is empty
        // and every assembled line is a key the pass was never asked about.
        // They still have to come out the other side — they are the document's
        // whole `## Files`, and each was settled by `accept_file` already.
        let request = Request::new("synthesise", "/repo/crates/engine")
            .with_files([File::omitted("lib.rs", 38), File::omitted("pact.rs", 90)])
            .with_child_documents([ChildDocument::new("src", "# src\n\nThe code.\n")]);
        let lines: BTreeMap<String, String> = [
            (
                "lib.rs".to_owned(),
                "The crate root and what it re-exports.".to_owned(),
            ),
            (
                "pact.rs".to_owned(),
                "Granting, refreshing and un-pacting a subtree.".to_owned(),
            ),
        ]
        .into_iter()
        .collect();

        // An answer bad enough to reach the mend: the purpose is too short, so
        // the attempts run out and `synthesise` falls through to here.
        let unusable = Fill {
            purpose: "Too short.".to_owned(),
            files: lines.clone(),
            ..Fill::default()
        };

        let (repaired, mends) = mend(&unusable, &Expected::of(&request), &Described::default());

        assert!(!mends.is_empty(), "the mend this is about did not happen");
        assert_eq!(
            repaired.files, lines,
            "the mend dropped the lines the run paid for"
        );
    }

    #[test]
    fn a_refused_name_is_told_that_a_declared_name_would_have_done() {
        // The sentence is the only part a retry can act on. A claim naming a
        // symbol is told symbols are allowed and this is not one of them:
        // telling it the name must be "a file or subdirectory" sends the next
        // pass looking for the wrong thing.
        assert_eq!(
            Defect::UnknownTarget {
                field: "structure[0].names[8]".to_owned(),
                name: "areas".to_owned(),
            }
            .to_string(),
            "structure[0].names[8] names `areas`, which is not a file, a subdirectory, \
             or a name declared in one of them"
        );
    }

    #[test]
    fn the_synthesis_is_shown_the_child_it_must_key_by_and_is_never_shown_files() {
        // The shape the pass is told to return carries the children by name, so
        // there is a key to fill in rather than one to invent from the prose —
        // which spells a directory `crates/`, the way a rendered document does.
        // And no `files` slot at all: this pass is not asked about them.
        let request = Request::new("synthesise", "/repo")
            .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
        let lines = [(
            "Cargo.toml".to_owned(),
            "The workspace manifest.".to_owned(),
        )]
        .into_iter()
        .collect();

        let text = synthesis_instructions("repo", &lines, &Expected::of(&request), &[]);

        assert!(
            text.ends_with(
                "{\"purpose\":\"\",\"directories\":{\"crates\":\"\"},\
                 \"structure\":[]}"
            ),
            "the shape is last and carries the child: {text}"
        );
        assert!(
            !text.contains("\"files\""),
            "a slot nobody asked about: {text}"
        );
    }

    #[test]
    fn a_child_keyed_the_way_a_document_renders_it_is_corrected_and_kept() {
        // `render` writes a child as `crates/`, and every synthesis pass is
        // shown its children's documents — so the trailing slash comes back. It
        // used to reach `keyed` as a key nothing asked for, which is a panic in
        // a debug build and a dropped `## Directories` line in any build.
        let request = Request::new("synthesise", "/repo")
            .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
        let answer = Fill {
            purpose: "The workspace root: two crates and the files that wire them.".to_owned(),
            directories: [(
                "crates/".to_owned(),
                "Both workspace members live here.".to_owned(),
            )]
            .into_iter()
            .collect(),
            ..Fill::default()
        };

        let accepted = accept_synthesis(
            &answer.to_json(),
            &BTreeMap::new(),
            &Expected::of(&request),
            &Described::default(),
        );

        let Accepted::Filled(fill) = accepted else {
            panic!("the answer was turned down: {accepted:?}");
        };
        assert_eq!(
            fill.directories.keys().collect::<Vec<_>>(),
            ["crates"],
            "the slash is corrected rather than the line dropped"
        );
    }

    #[test]
    fn a_child_nothing_asked_about_is_dropped_and_not_faulted() {
        let request = Request::new("synthesise", "/repo")
            .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
        let answer = Fill {
            purpose: "The workspace root: two crates and the files that wire them.".to_owned(),
            directories: [
                (
                    "crates".to_owned(),
                    "Both workspace members live here.".to_owned(),
                ),
                (
                    "docs".to_owned(),
                    "A directory with no document of its own.".to_owned(),
                ),
            ]
            .into_iter()
            .collect(),
            ..Fill::default()
        };

        let accepted = accept_synthesis(
            &answer.to_json(),
            &BTreeMap::new(),
            &Expected::of(&request),
            &Described::default(),
        );

        let Accepted::Filled(fill) = accepted else {
            panic!("an unasked child was faulted rather than dropped: {accepted:?}");
        };
        assert_eq!(fill.directories.keys().collect::<Vec<_>>(), ["crates"]);
    }

    #[test]
    fn the_prompt_says_warlock_is_the_tool_and_not_the_project() {
        // Measured on a scratch crate that never mentions warlock: two of its
        // three documents called it "a toy freshness ledger belonging to
        // Warlock", because the instructions name warlock and ask for the
        // product the directory belongs to.
        for prompt in [SYNTHESIS_PROMPT, FILE_PROMPT] {
            assert!(
                prompt.contains("it is not the project being described"),
                "{prompt}"
            );
        }
        assert!(
            SYNTHESIS_PROMPT
                .contains("its name belongs in no value unless the files themselves use it"),
            "{SYNTHESIS_PROMPT}"
        );
        assert!(
            FILE_PROMPT.contains("its name belongs in the line only if the file itself uses it"),
            "{FILE_PROMPT}"
        );
    }

    #[test]
    fn every_defect_reads_as_one_line_naming_its_slot() {
        let all = [
            Defect::NotJson {
                detail: "expected value at line 1".to_owned(),
            },
            Defect::Missing {
                field: "files[\"a\"]".to_owned(),
            },
            Defect::Empty {
                field: "purpose".to_owned(),
            },
            Defect::Multiline {
                field: "structure[0]".to_owned(),
            },
            Defect::TooShort {
                field: "files[\"c\"]".to_owned(),
                chars: 3,
                minimum: ENTRY_MINIMUM,
            },
            Defect::TooLong {
                field: "structure[1]".to_owned(),
                chars: 300,
                cap: ENTRY_CHARS,
            },
            Defect::TooMany {
                field: "structure".to_owned(),
                count: 20,
                cap: LIST_CAP,
            },
            Defect::UnknownTarget {
                field: "structure[0].names[0]".to_owned(),
                name: "x".to_owned(),
            },
            Defect::ToolNamed {
                field: "purpose".to_owned(),
            },
        ];
        for defect in all {
            let text = defect.to_string();
            assert!(!text.contains('\n'), "{text}");
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn sizes_read_the_way_a_person_reads_them() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1024), "1.0 KB");
        assert_eq!(human(22_938), "22.4 KB");
        assert_eq!(human(1024 * 1024), "1.0 MB");
        assert_eq!(human(4_200_000), "4.0 MB");
    }

    #[test]
    fn a_described_record_is_only_what_warlock_measured() {
        let described = Described {
            declared: [("a.rs".to_owned(), vec!["one".to_owned()])]
                .into_iter()
                .collect(),
            ..Described::default()
        };
        assert_eq!(described.declared["a.rs"], ["one"]);
        assert_eq!(Described::default().declared, BTreeMap::new());
    }

    fn declared() -> Described {
        Described {
            declared: [
                (
                    "lib.rs".to_owned(),
                    vec!["pact".to_owned(), "subtree_hash".to_owned()],
                ),
                (
                    "app.rs".to_owned(),
                    (0..20).map(|i| format!("draw{i}")).collect(),
                ),
            ]
            .into_iter()
            .collect(),
            ..Described::default()
        }
    }

    #[track_caller]
    fn holds_the_shape(line: &str) {
        assert!(!line.contains(['\n', '\r']), "more than one line: {line:?}");
        let chars = line.chars().count();
        assert!(
            (ENTRY_MINIMUM..=ENTRY_CHARS).contains(&chars),
            "{chars} characters: {line:?}"
        );
    }

    #[test]
    fn a_fallback_line_is_the_name_the_size_and_the_symbols() {
        let request = request();
        let expected = Expected::of(&request);
        let described = declared();

        assert_eq!(
            fallback::file("lib.rs", &expected, &described),
            "A 39 B file named `lib.rs`, declaring `pact`, `subtree_hash`."
        );
        // A file warlock extracted nothing from still gets its name and size,
        // and says only that and that it found no names.
        assert_eq!(
            fallback::file("Cargo.toml", &expected, &described),
            "A 26 B file named `Cargo.toml`, with no symbols extracted from it."
        );
        // The purpose is the directory's name and what is in it: five files,
        // one child. No sentence about what any of it is for.
        assert_eq!(
            fallback::purpose("engine", &expected, &Described::default()),
            "`engine` holds 5 files and 1 subdirectory."
        );
        assert_eq!(
            fallback::directory("src", &expected, &Described::default()),
            "A subdirectory named `src`."
        );

        let bare = Request::new("describe", "/repo/empty");
        assert_eq!(
            fallback::purpose("empty", &Expected::of(&bare), &Described::default()),
            "`empty` holds no files and no subdirectories."
        );
    }

    #[test]
    fn every_fallback_line_holds_one_line_and_both_caps() {
        let request = request();
        let expected = Expected::of(&request);
        let described = declared();

        for path in [
            "lib.rs",
            "Cargo.toml",
            "app.rs",
            "Cargo.lock",
            "logo.png",
            // A name the request does not carry: no size to state, and the
            // line is still a line.
            "not-here.rs",
            "a",
            "",
        ] {
            holds_the_shape(&fallback::file(path, &expected, &described));
        }
        for name in ["engine", "a", ""] {
            holds_the_shape(&fallback::purpose(name, &expected, &Described::default()));
            holds_the_shape(&fallback::directory(name, &expected, &Described::default()));
        }

        // The symbol list stops at the cap rather than being cut through a
        // name: twenty long declarations do not run the line over.
        let long = Described {
            declared: [(
                "lib.rs".to_owned(),
                (0..20).map(|i| format!("a_long_symbol_name_{i}")).collect(),
            )]
            .into_iter()
            .collect(),
            ..Described::default()
        };
        let line = fallback::file("lib.rs", &expected, &long);
        holds_the_shape(&line);
        assert!(line.ends_with("`."), "cut between names: {line:?}");
    }

    #[test]
    fn short_facts_are_padded_and_a_long_line_is_cut_to_the_cap() {
        let padded = fallback::fit("`a`, 5 B.", "A file in this directory.");
        holds_the_shape(&padded);
        assert!(padded.starts_with("`a`, 5 B."), "{padded}");

        let cut = fallback::fit(&"x".repeat(ENTRY_CHARS + 50), "pad");
        assert_eq!(cut.chars().count(), ENTRY_CHARS);

        // Whatever the facts, the value is one line.
        assert_eq!(
            fallback::fit("a name\nover\ttwo lines and some", "pad"),
            "a name over two lines and some"
        );
    }

    #[test]
    fn a_name_warlock_shares_does_not_trip_the_tool_rule() {
        // The rule is a substring test, so a line naming `warlock.rs` cannot be
        // quoted out of it: while the rule is live the name is left out
        // instead, and `render` prints the path either way.
        let request = Request::new("describe", "/repo/crates").with_files([File::present(
            "warlock.rs",
            *b"pub fn draw() {}\npub fn run() {}\n",
        )]);
        let expected = Expected::of(&request);
        assert!(
            !Evidence::new(&expected, &Described::default()).mentions_tool(),
            "the rule is live here"
        );
        let described = Described {
            declared: [(
                "warlock.rs".to_owned(),
                vec!["draw".to_owned(), "warlock_run".to_owned()],
            )]
            .into_iter()
            .collect(),
            ..Described::default()
        };

        let line = fallback::file("warlock.rs", &expected, &described);
        assert!(!names_tool(&line), "{line}");
        assert!(line.contains("`draw`"), "the other symbols stay: {line}");
        holds_the_shape(&line);
        for named in [
            fallback::purpose("warlock", &expected, &Described::default()),
            fallback::directory("warlock-tui", &expected, &Described::default()),
        ] {
            assert!(!names_tool(&named), "{named}");
            holds_the_shape(&named);
        }

        // And the rule the caller applies cannot fire on a fill built of them.
        let fill = Fill {
            purpose: fallback::purpose("crates", &expected, &Described::default()),
            files: [(
                "warlock.rs".to_owned(),
                fallback::file("warlock.rs", &expected, &described),
            )]
            .into_iter()
            .collect(),
            ..Fill::default()
        };
        assert_eq!(check(&fill, &expected, &Described::default()), []);

        // Where the files use the word the rule has stood down, and the facts
        // go in whole.
        let own = Request::new("describe", "/repo")
            .with_files([File::present("warlock.rs", *b"//! The warlock engine.\n")]);
        let own = Expected::of(&own);
        assert!(Evidence::new(&own, &Described::default()).mentions_tool());
        let line = fallback::file("warlock.rs", &own, &described);
        assert!(line.contains("`warlock.rs`"), "{line}");
        assert!(line.contains("`warlock_run`"), "{line}");
        assert!(
            fallback::purpose("warlock", &own, &Described::default()).contains("`warlock`"),
            "the directory names itself too"
        );
    }

    #[test]
    fn no_fallback_line_speaks_in_the_stubs_words() {
        let request = request();
        let expected = Expected::of(&request);
        let described = declared();
        let lines = [
            fallback::file("lib.rs", &expected, &described),
            fallback::file("logo.png", &expected, &described),
            fallback::purpose("engine", &expected, &Described::default()),
            fallback::directory("src", &expected, &Described::default()),
        ];
        // `Fill::stub` is a test double's wording; a repaired document says
        // what warlock measured, and never that.
        for line in &lines {
            let lowered = line.to_ascii_lowercase();
            for wording in ["a stand-in entry", "stand-in", "test double"] {
                assert!(!lowered.contains(wording), "{wording:?} in {line:?}");
            }
        }
    }

    // The mend over the fixture directory, asserting the property every one of
    // these tests shares: what comes back is not itself defective.
    #[track_caller]
    fn mend_of(fill: &Fill) -> (Fill, Vec<Mend>) {
        let request = request();
        let expected = Expected::of(&request);
        let (repaired, mends) = mend(fill, &expected, &declared());
        assert_eq!(
            check(&repaired, &expected, &Described::default()),
            [],
            "a mended fill is not itself defective: {mends:?}"
        );
        (repaired, mends)
    }

    #[test]
    fn an_empty_value_falls_back_to_what_warlock_measured() {
        let request = request();
        let expected = Expected::of(&request);
        let mut fill = good();
        fill.purpose = "   ".to_owned();
        fill.directories.insert("src".to_owned(), String::new());
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(
            repaired.purpose,
            fallback::purpose("engine", &expected, &Described::default())
        );
        assert_eq!(
            repaired.directories["src"],
            fallback::directory("src", &expected, &Described::default())
        );
        assert_eq!(
            mends,
            [
                Mend {
                    field: "purpose".to_owned(),
                    done: Mended::Supplied
                },
                Mend {
                    field: "directories[\"src\"]".to_owned(),
                    done: Mended::Supplied
                },
            ]
        );
    }

    #[test]
    fn a_value_under_the_floor_falls_back_to_what_warlock_measured() {
        let request = request();
        let mut fill = good();
        fill.directories
            .insert("src".to_owned(), "duplicate".to_owned());
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(
            repaired.directories["src"],
            fallback::directory("src", &Expected::of(&request), &Described::default())
        );
        assert_eq!(mends.first().map(|mend| mend.done), Some(Mended::Supplied));
        assert_eq!(mends.len(), 1, "{mends:?}");
    }

    #[test]
    fn a_value_over_more_than_one_line_keeps_the_first() {
        let mut fill = good();
        fill.directories.insert(
            "src".to_owned(),
            "\nthe source tree, in one line\nand a second the cap would have allowed".to_owned(),
        );
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(repaired.directories["src"], "the source tree, in one line");
        assert_eq!(
            mends,
            [Mend {
                field: "directories[\"src\"]".to_owned(),
                done: Mended::FirstLine
            }]
        );
        assert_eq!(
            mends[0].to_string(),
            "directories[\"src\"] ran to more than one line and keeps its first"
        );
    }

    #[test]
    fn a_value_over_its_cap_is_cut_to_it_counting_characters() {
        let mut fill = good();
        fill.directories
            .insert("src".to_owned(), "x".repeat(ENTRY_CHARS + 20));
        fill.purpose = "é".repeat(PURPOSE_CHARS + 1);
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(repaired.directories["src"].chars().count(), ENTRY_CHARS);
        // Characters, and a character boundary: the purpose is multibyte, so a
        // byte cut would either panic or land inside an `é`.
        assert_eq!(repaired.purpose.chars().count(), PURPOSE_CHARS);
        assert_eq!(repaired.purpose.len(), PURPOSE_CHARS * 2);
        assert_eq!(
            mends,
            [
                Mend {
                    field: "purpose".to_owned(),
                    done: Mended::Cut {
                        from: PURPOSE_CHARS + 1,
                        to: PURPOSE_CHARS
                    }
                },
                Mend {
                    field: "directories[\"src\"]".to_owned(),
                    done: Mended::Cut {
                        from: ENTRY_CHARS + 20,
                        to: ENTRY_CHARS
                    }
                },
            ]
        );
        assert_eq!(
            mends[1].to_string(),
            "directories[\"src\"] was 300 characters and was cut to 280",
            "the line a run reports, from the brief"
        );
    }

    #[test]
    fn a_list_over_its_cap_keeps_its_first_entries() {
        let mut fill = good();
        fill.structure = (0..LIST_CAP + 2)
            .map(|index| {
                Entry::naming(
                    format!("an entry long enough to count, the {index}th"),
                    "lib.rs",
                )
            })
            .collect();
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(repaired.structure.len(), LIST_CAP);
        assert_eq!(repaired.structure, fill.structure[..LIST_CAP]);
        assert_eq!(
            mends,
            [Mend {
                field: "structure".to_owned(),
                done: Mended::Shortened {
                    from: LIST_CAP + 2,
                    to: LIST_CAP
                }
            }]
        );
    }

    #[test]
    fn a_value_naming_the_tool_is_dropped_and_falls_to_the_next_rule() {
        let request = request();
        let expected = Expected::of(&request);
        let mut fill = good();
        fill.directories.insert(
            "src".to_owned(),
            "the source of the warlock engine, and long enough besides".to_owned(),
        );
        fill.structure = vec![Entry::naming(
            "warlock-style grants, one per module and then some",
            "lib.rs",
        )];
        // The purpose is the one slot with no next rule under it: a document
        // without one is not a document, so it falls straight to the fallback.
        fill.purpose = "A toy freshness ledger belonging to Warlock.".to_owned();
        let (repaired, mends) = mend_of(&fill);
        assert_eq!(
            repaired.directories["src"],
            fallback::directory("src", &expected, &Described::default())
        );
        assert_eq!(
            repaired.purpose,
            fallback::purpose("engine", &expected, &Described::default())
        );
        assert!(
            repaired.structure.is_empty(),
            "a list entry has no fallback"
        );
        assert_eq!(
            mends,
            [
                Mend {
                    field: "purpose".to_owned(),
                    done: Mended::Supplied
                },
                Mend {
                    field: "directories[\"src\"]".to_owned(),
                    done: Mended::Dropped
                },
                Mend {
                    field: "structure[0]".to_owned(),
                    done: Mended::Dropped
                },
                Mend {
                    field: "directories[\"src\"]".to_owned(),
                    done: Mended::Supplied
                },
            ],
            "the drop and the fall are two records, in the order they happened"
        );
    }

    #[test]
    fn an_entry_the_request_never_asked_for_is_gone_before_the_first_check() {
        let mut fill = good();
        fill.directories.insert(
            "target".to_owned(),
            "the build directory, unasked for".to_owned(),
        );
        // Not a mend: nothing was repaired, an answer to a question nobody
        // asked was thrown away. Were it left in, `keyed`'s debug assertion
        // would fire on the mend's own first `check`.
        let (repaired, mends) = mend_of(&fill);
        assert!(!repaired.directories.contains_key("target"));
        assert_eq!(
            repaired.directories,
            good().directories,
            "the asked-for entries stay"
        );
        assert_eq!(mends, []);
    }

    #[test]
    fn the_mend_settles_inside_its_bound_and_two_rules_cannot_spin() {
        const {
            assert!(
                MEND_PASSES >= 3,
                "the longest chain is drop, fall back, check clean"
            );
        }
        let request = request();
        let expected = Expected::of(&request);

        // The longest chain a rule here can start: a value both over the cap
        // and naming the tool. The drop takes it, the fallback answers the gap
        // the drop left, and the third look finds nothing — the two rules do
        // not hand the slot back and forth.
        let mut fill = good();
        fill.directories
            .insert("src".to_owned(), format!("warlock{}", "x".repeat(400)));
        let (repaired, mends, passes) = mended(&fill, &expected, &declared());
        assert_eq!(check(&repaired, &expected, &Described::default()), []);
        assert_eq!(
            mends.iter().map(|mend| mend.done).collect::<Vec<_>>(),
            [Mended::Dropped, Mended::Supplied],
            "twice over the same slot and then done: {mends:?}"
        );
        assert_eq!(passes, 2);
        assert!(
            passes < MEND_PASSES,
            "the bound is a stop, not a schedule: {passes} of {MEND_PASSES}"
        );

        // Every rule at once, on every slot, still settles inside the bound.
        let mut fill = good();
        fill.purpose = "Warlock's own\nledger.".to_owned();
        fill.directories.insert("src".to_owned(), String::new());
        fill.structure = vec![Entry::of(String::new()); LIST_CAP + 3];
        fill.structure.push(Entry::naming(
            "a claim about a name that is not here",
            "missing",
        ));
        let (repaired, _, passes) = mended(&fill, &expected, &declared());
        assert_eq!(check(&repaired, &expected, &Described::default()), []);
        assert!(passes <= MEND_PASSES, "{passes}");
    }

    fn variant(defect: &Defect) -> &'static str {
        match defect {
            Defect::NotJson { .. } => "NotJson",
            Defect::Missing { .. } => "Missing",
            Defect::Empty { .. } => "Empty",
            Defect::Multiline { .. } => "Multiline",
            Defect::TooShort { .. } => "TooShort",
            Defect::TooLong { .. } => "TooLong",
            Defect::TooMany { .. } => "TooMany",
            Defect::UnknownTarget { .. } => "UnknownTarget",
            Defect::ToolNamed { .. } => "ToolNamed",
        }
    }

    type Mutation = (&'static str, fn(&mut Fill));

    // One per repairable defect, and a few that collide on purpose: two
    // mutations over the same slot are how a repair comes to answer a slot
    // another repair already moved.
    fn mutations() -> [Mutation; 11] {
        [
            ("Missing", |fill| {
                fill.directories.remove("src");
            }),
            ("Empty", |fill| fill.purpose = "  ".to_owned()),
            ("Empty", |fill| {
                fill.structure.push(Entry::naming("   ", "lib.rs"));
            }),
            ("Multiline", |fill| {
                fill.directories.insert(
                    "src".to_owned(),
                    "the source, and long enough\nand a second line".to_owned(),
                );
            }),
            ("TooShort", |fill| {
                fill.purpose = "duplicate".to_owned();
            }),
            ("TooLong", |fill| {
                fill.structure
                    .push(Entry::naming("x".repeat(ENTRY_CHARS + 40), "lib.rs"));
            }),
            ("TooLong", |fill| {
                fill.purpose = "é".repeat(PURPOSE_CHARS + 9);
            }),
            ("TooMany", |fill| {
                fill.structure =
                    vec![Entry::naming("an entry long enough to count", "lib.rs"); LIST_CAP + 2];
            }),
            ("UnknownTarget", |fill| {
                fill.structure.push(Entry::naming(
                    "`load_tree` walks the repository from the crate root.",
                    "load_tree",
                ));
            }),
            ("ToolNamed", |fill| {
                fill.structure.push(Entry::naming(
                    "the crate root of the warlock engine, and long enough",
                    "lib.rs",
                ));
            }),
            ("ToolNamed", |fill| {
                fill.directories.insert(
                    "src".to_owned(),
                    "warlock's own source, in one line".to_owned(),
                );
            }),
        ]
    }

    #[test]
    fn a_mended_fill_is_never_defective_whatever_was_wrong_with_it() {
        let mutations = mutations();
        let request = request();
        let expected = Expected::of(&request);
        let described = declared();
        let mut covered: BTreeSet<&'static str> = BTreeSet::new();
        // A fixed seed and a plain congruential generator: this crate takes no
        // dependency for a coin toss, and a property test that cannot be
        // reproduced from its own source is not much of one.
        let mut state: u64 = 0x5eed_1234_5678_9abc;
        for _ in 0..512 {
            let mut fill = good();
            let mut applied: Vec<&str> = Vec::new();
            for (name, mutate) in &mutations {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                if (state >> 60) & 1 == 1 {
                    mutate(&mut fill);
                    applied.push(name);
                }
            }
            let defects = check(&fill, &expected, &Described::default());
            for defect in &defects {
                covered.insert(variant(defect));
            }
            let (repaired, mends, passes) = mended(&fill, &expected, &described);
            assert_eq!(
                check(&repaired, &expected, &Described::default()),
                [],
                "{applied:?} left {mends:?} and still a defect"
            );
            assert!(passes <= MEND_PASSES, "{applied:?} took {passes} passes");
            assert_eq!(
                mends.is_empty(),
                defects.is_empty(),
                "{applied:?}: a defect is a mend and nothing else is"
            );
        }
        assert_eq!(
            covered,
            BTreeSet::from([
                "Missing",
                "Empty",
                "Multiline",
                "TooShort",
                "TooLong",
                "TooMany",
                "UnknownTarget",
                "ToolNamed",
            ]),
            "every repairable defect was generated, and `NotJson` cannot be: \
             the mend is handed a fill, not an answer"
        );
    }

    #[test]
    fn a_multibyte_name_is_cut_on_a_character_boundary() {
        let name = format!("{}.rs", "é".repeat(400));
        let symbol = "🜁_très_long_identifiant_déclaré".repeat(20);
        let request = Request::new("describe", "/repo/x")
            .with_files([File::present(name.clone(), *b"pub fn draw() {}\n")]);
        let expected = Expected::of(&request);
        let described = Described {
            declared: [(name.clone(), vec![symbol.clone(), symbol])]
                .into_iter()
                .collect(),
            ..Described::default()
        };

        // No panic, and the count is characters rather than bytes.
        let line = fallback::file(&name, &expected, &described);
        holds_the_shape(&line);
        assert_eq!(line.chars().count(), ENTRY_CHARS);
        assert!(line.len() > ENTRY_CHARS, "multibyte: {} bytes", line.len());

        holds_the_shape(&fallback::purpose(&name, &expected, &Described::default()));
        holds_the_shape(&fallback::directory(
            &name,
            &expected,
            &Described::default(),
        ));
    }
}
