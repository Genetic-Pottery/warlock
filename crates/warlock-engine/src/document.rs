use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::{File, Request};
use crate::languages;

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

    // The same text with each file's comments cut out, which is what a claim's
    // names are held against. `sent` itself is unchanged and still answers the
    // tool-naming guard below, where a comment naming warlock is a directory
    // naming warlock and the guard should stand down for it.
    fn sent_code(&self) -> impl Iterator<Item = String> + '_ {
        self.expected.files.iter().filter_map(|(path, (_, shown))| {
            let Shown::Text(text) = shown else {
                return None;
            };
            Some(
                languages::without_comments(Path::new(path), text)
                    .unwrap_or_else(|| (*text).to_owned()),
            )
        })
    }

    // A name warlock can find, asked of both witnesses because the request is
    // not always one of them.
    //
    // The request is the better witness where it has the file: containment
    // rather than a declaration on purpose — a claim may name a lint, a
    // manifest key or a constant that no language table declares. Containment
    // over its code and not its comments, though: a comment is the one place a
    // name can appear with nothing behind it, and an invented mechanism written
    // in one witnessed its own repetition into a document.
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
            || self.sent_code().any(|code| code.contains(name))
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

// Asked once per identifier of every file sent, and again over whole file
// texts, so it runs against megabytes on a repository that never says
// "warlock" and so never short-circuits. Lowercasing the haystack first would
// allocate a copy of every one of them.
fn names_tool(text: &str) -> bool {
    text.as_bytes()
        .windows(b"warlock".len())
        .any(|window| window.eq_ignore_ascii_case(b"warlock"))
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

pub(crate) fn turned_down(text: &mut String, rejected: &[Defect]) {
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
    let mut parsed = match parse::<Fill>(answer) {
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

You are given the file's name and size, and its code with the comments removed \
and with function bodies elided where that was needed to fit. Describe what the \
code does. Where the name and the code disagree, the code is right.";

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
    let evidence = Evidence::new(expected, described);
    if !evidence.mentions_tool() && names_tool(line) {
        defects.push(Defect::ToolNamed {
            field: field.clone(),
        });
    }

    // The names are checked here, against this one file, because this is the
    // only place a per-file line is ever looked at. `check` grew a loop over
    // `fill.files` for this and never ran it: `accept_synthesis` clears that map
    // before checking and merges the lines back in afterwards, so the loop sees
    // an empty map on every real pass and fires only from `mend`, which runs
    // only once an answer has already failed for something else. The comment
    // above `check` claimed this function did the job; it did not, and
    // `Decoder::decode()` reached engine/core's document while every test of the
    // check passed, because those tests build a `Fill` by hand and call `check`
    // directly.
    //
    // One file's evidence and not the directory's, which is the narrower rule
    // and the right one. A line naming a symbol some neighbour declares is
    // spending this file's characters routing a reader out of this file, which
    // is what the line exists not to do. It costs nothing in true lines: a file
    // that really does call `ledger::post()` has `post` in its own text, so the
    // name is witnessed and stands. What it refuses is a name with nothing
    // behind it here — which, once comments stop counting, means a name with
    // nothing behind it at all.
    for name in referenced(line) {
        if !evidence.knows(&name) {
            defects.push(Defect::UnknownTarget {
                field: field.clone(),
                name,
            });
        }
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

// Generic over the fill so the drafting road scrapes braces by the same rule:
// a model that wraps its object in a sentence, or in a code fence, is answering
// the same way on both roads, and a tolerance widened for one of them that did
// not hold for the other would refuse an answer the other accepts.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(answer: &str) -> Result<T, Defect> {
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

// The `files` loop below is reached from `mend` and never from
// `accept_synthesis`, which clears that map before calling this and merges the
// lines in afterwards. That is not a hole any more — `accept_file` checks each
// line's names as it accepts it, against the one file the line is about — but it
// was one for as long as this comment claimed otherwise, so do not read the loop
// as the guard on a fresh line. It guards a fill being repaired: lines carried
// forward off the page, and lines already standing when something else about the
// answer failed.
//
// Keying the lines is a separate matter and still not done here: a synthesis
// request sends no text, so `Expected::asked` is empty and keying against it
// would report every line as a slot nothing asked for.
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

    // The per-file lines went unchecked entirely until this: not their names,
    // and not even the tool-naming guard below, which `values` still does not
    // reach. `Decoder::decode()` arrived in one of them.
    for (path, line) in &fill.files {
        if !expected.files.contains_key(path.as_str()) {
            continue;
        }
        for name in referenced(line) {
            if !evidence.knows(&name) {
                defects.push(Defect::UnknownTarget {
                    field: format!("files[{path:?}]"),
                    name,
                });
            }
        }
    }

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

// The references in a prose line that are about code rather than about English.
// A structure entry carries its own `names` and is checked against those; a
// files line is bare prose, so what it leans on has to be recognised instead of
// read off.
//
// Two shapes only: a call, and a qualified path whose tail is capitalised.
// `decode()` and `Decoder::decode` are claims about code; `caps`, `postings` and
// `balance.rs` are not. A bare noun therefore goes unchecked, which is the cost
// — a lie written without parentheses is missed. Widening this to every
// identifier was the alternative and it is worse: `identifiers` puts every
// English word in the line through `knows`, and a document whose prose reads
// like prose loses most of its lines.
fn referenced(line: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut opened: Option<usize> = None;
    for (index, character) in line.char_indices() {
        if character.is_alphanumeric() || character == '_' || character == ':' || character == '.' {
            opened = opened.or(Some(index));
            continue;
        }
        if let Some(from) = opened.take() {
            note_reference(&line[from..index], character == '(', &mut found);
        }
    }
    if let Some(from) = opened {
        note_reference(&line[from..], false, &mut found);
    }
    found
}

fn note_reference(chunk: &str, called: bool, found: &mut BTreeSet<String>) {
    let name = chunk.trim_matches(|c| c == '.' || c == ':');
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        return;
    }
    if called || qualified(name) {
        found.insert(name.to_owned());
    }
}

// `::` always separates, but `.` only counts when what follows it is
// capitalised. Without that, `e.g` is a claim about an identifier `e` and a
// `.rs` filename is a claim about a module named after a language.
fn qualified(name: &str) -> bool {
    name.contains("::")
        || name
            .rsplit_once('.')
            .is_some_and(|(_, tail)| tail.chars().next().is_some_and(char::is_uppercase))
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

pub(crate) fn line(
    field: &str,
    value: &str,
    minimum: usize,
    cap: usize,
    defects: &mut Vec<Defect>,
) {
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
            let _ = write!(text, "- `{path}` ({})", human(*size));
            // A sent file whose line was dropped has nothing to say here, and
            // the `· declares` list below is then the whole entry. Writing the
            // dash anyway leaves it dangling in front of it.
            if !entry.is_empty() {
                let _ = write!(text, " — {entry}");
            }
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
        DECLARED_SHOWN, Described, ENTRY_CHARS, ENTRY_MINIMUM, Expected, flattened, human,
        names_tool,
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
}

// Whitespace of any kind collapses to one space: a name or a symbol that
// carried a newline would otherwise make a one-line value into two.
pub(crate) fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
    filled_files: BTreeSet<String>,
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
            // A files line falls to `fallback::file` rather than going: the
            // document renders one line per file whatever happens, so dropping
            // the key leaves an entry with nothing after its size, while the
            // fallback is the name, the size and the symbols `languages.rs`
            // found — every one of them measured, and none of them the thing
            // the line was wrong about.
            Slot::FileLine(path) => (self.filled_files.insert(path), Mended::Supplied),
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
            Slot::FileLine(path) => (self.filled_files.insert(path), Mended::Supplied),
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
            Slot::FileLine(path) => self.filled_files.contains(&path),
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
        for path in &self.filled_files {
            fill.files
                .insert(path.clone(), fallback::file(path, expected, described));
        }

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
    FileLine(String),
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
        "files" => {
            let Ok(key) = serde_json::from_str::<String>(inside) else {
                return Slot::Unknown;
            };
            Slot::FileLine(key)
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
        Slot::FileLine(path) => fill.files.get_mut(&path),
        Slot::List(_) | Slot::Unknown => None,
    }
}

#[cfg(test)]
#[path = "tests/document.rs"]
mod tests;
