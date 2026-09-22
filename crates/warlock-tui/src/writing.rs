//! The path `/write` proposes, and the one function in warlock that puts a
//! document somebody asked for on disk.
//!
//! [`write_submit`] writes the path that was on screen when Enter was pressed,
//! never the proposal, which is what makes every rule in the first half cheap:
//! none of them refuses anything and none returns a [`Result`]. [`unfenced`]
//! runs before the title is looked for and is the same function the write
//! itself uses, so the proposed path and the written bytes cannot disagree.
//! Every digit run in every name counts towards the number, not only names of
//! warlock's own shape, because `docs/` holds briefs written by another hand.
//! [`missing_sections`] refuses and writes nothing; it never repairs, because a
//! brief with a section missing reads perfectly well and nobody finds out for
//! days. An existing target, or a disk that will not take the file, is news for
//! the footer and not a reason to tear the screen down.

use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use warlock_engine::{from_manifest_path, to_manifest_path};
use warlock_tui::{
    App, Edited, ScopeField, ScopePrompt, TemplateError, brief_template, missing_sections, size,
};

use crate::error::{Error, one_line};

pub(crate) const WRITE_HEADING: &str = "Write the brief to";

const BRIEF_PREFIX: &str = "warlock-brief";

const UNTITLED: &str = "untitled";

const SLUG_MAX: usize = 60;

const NO_PATH: &str = "type a path for the document, or press Esc to write nothing";

const NOTHING_TO_WRITE: &str =
    "there is no answer on the conversation to write, so nothing was written";

// The three rules in the order they depend on each other: the reply is
// unwrapped, the slug comes off the unwrapped document's first `# ` line, and
// the number comes off the names in the directory the file would land in.
// Relative rather than absolute because it is going into a field somebody reads
// and edits, and because it is the spelling the thread names the file by
// afterwards.
//
// Reads `repo_root/directory` and nothing else, creates nothing, and is as happy
// with a directory that is not there as with an empty one.
pub(crate) fn proposed_path(repo_root: &Path, directory: &str, reply: &str) -> String {
    let number = spelled(next_number(&repo_root.join(directory)));
    let slug = slug_of(unfenced(reply));
    format!("{directory}/{BRIEF_PREFIX}-{number}-{slug}.md")
}

// Nothing presses a key to get here — the prompt opens because a `/write` turn
// answered — so this takes the reply rather than the app, which keeps the path
// on screen about the very turn that just landed. `directory` comes the same
// way, off the loop, so nothing is read here to find it and a window that opens
// over a finished document cannot fail to open.
pub(crate) fn write_opened(repo_root: &Path, directory: &str, reply: &str) -> ScopePrompt {
    ScopePrompt::open(WRITE_HEADING, proposed_path(repo_root, directory, reply))
}

// What this window hands back: the prompt the loop holds next, and — on the one
// path that put bytes on a disk — the document that landed. Every other path
// carries no path at all, which is what makes "the brief this session wrote" a
// fact about a write that happened rather than about a window that was open.
//
// The spelling is the manifest-relative one, the same string the line on the
// thread names, because that is the form `from_manifest_path` reads back and
// the form the ledger everywhere else in warlock stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wrote {
    pub(crate) prompt: ScopePrompt,
    pub(crate) written: Option<String>,
}

impl Wrote {
    // Every path but the successful write, which is most of this file: a prompt
    // and nothing written.
    const fn only(prompt: ScopePrompt) -> Self {
        Self {
            prompt,
            written: None,
        }
    }
}

pub(crate) fn write_edit(
    app: &mut App,
    repo_root: &Path,
    prompt: &ScopePrompt,
    edited: Edited,
    now: Instant,
) -> Wrote {
    match edited {
        Edited::Open(field) => Wrote::only(ScopePrompt::Open(field)),
        Edited::Close => Wrote::only(ScopePrompt::Closed),
        Edited::Submit => match prompt.field() {
            Some(field) => write_submit(app, repo_root, field, now),
            None => Wrote::only(ScopePrompt::Closed),
        },
    }
}

// `scope_submit`'s shape, and deliberately the same one: it runs on the event
// loop's own thread between two frames, spawns nothing, and hands back the
// prompt the loop holds next.
//
// `field.directory()` is not consulted at all: in this window it carries the
// heading the prompt is drawn under, and the answer is the line the reader
// typed.
//
// Judge, then write, and every refusal happens before a byte moves. A path that
// climbs out of the repository and a path already taken both reopen the field
// with the rule under it and the typed text exactly where it was.
pub(crate) fn write_submit(
    app: &mut App,
    repo_root: &Path,
    field: &ScopeField,
    now: Instant,
) -> Wrote {
    let typed = field.text().trim();
    if typed.is_empty() {
        return refused(field, NO_PATH);
    }
    // The one spelling of the path, produced before anything is done with it:
    // the bytes go to it and the line names it, so both are the same string and
    // cannot come to disagree.
    let stored = match to_manifest_path(repo_root, typed) {
        Ok(stored) => stored,
        // The engine's own wording about a path that is not inside the root,
        // flattened as every other manifest failure in this binary is — on the
        // rule line rather than the footer, because it is the field's text that
        // is wrong and the field is still up.
        Err(source) => return refused(field, Error::Manifest { source }.to_string()),
    };
    let path = from_manifest_path(repo_root, &stored);
    if path.exists() {
        return refused(field, taken_rule(&stored));
    }

    let Some(document) = document_on(app) else {
        return closed_saying(app, NOTHING_TO_WRITE);
    };
    // The shape the document is held to, read now rather than remembered. Both
    // of the next two answers come down the prompt and go to the footer rather
    // than back into the field, on `NOTHING_TO_WRITE`'s rule: what is wrong is
    // the document and not the path, and the field cannot be the place a reader
    // fixes it — the keyboard is theirs again only once the window is down.
    let shape = match brief_template(repo_root) {
        Ok(shape) => shape,
        Err(source) => return closed_saying(app, unreadable_shape_line(&source)),
    };
    let missing = missing_sections(&shape, &document);
    if !missing.is_empty() {
        return closed_saying(app, missing_line(&missing));
    }
    if let Err(error) = put(&path, document.as_bytes()) {
        return closed_saying(app, failure_line(&stored, &error));
    }

    // The size is the bytes just handed to the disk rather than a `stat` of what
    // came back: it is the same number, and asking the filesystem again would be
    // a second way for this line to fail after the write succeeded.
    let bytes = u64::try_from(document.len()).unwrap_or(u64::MAX);
    app.panel_mut().note(wrote_line(&stored, bytes), now);
    // The same string the line was just worded from, handed on rather than
    // spelled a second time: what the session remembers `/write` wrote and what
    // the reader was told it wrote cannot come to disagree.
    Wrote {
        prompt: ScopePrompt::Closed,
        written: Some(stored),
    }
}

fn refused(field: &ScopeField, rule: impl Into<String>) -> Wrote {
    Wrote::only(ScopePrompt::Open(field.clone().refused(rule)))
}

fn closed_saying(app: &mut App, line: impl Into<String>) -> Wrote {
    app.set_message(line);
    Wrote::only(ScopePrompt::Closed)
}

fn document(reply: &str) -> String {
    let body = unfenced(reply);
    if body.ends_with('\n') {
        return body.to_owned();
    }
    format!("{body}\n")
}

fn document_on(app: &App) -> Option<String> {
    let thread = app.panel().thread()?;
    let reply = thread.turns().last().copied()?.answer()?;
    Some(document(reply))
}

fn put(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

fn wrote_line(stored: &str, bytes: u64) -> String {
    format!("wrote {stored} — {}", size(bytes))
}

fn taken_rule(stored: &str) -> String {
    format!("{stored} already exists — nothing was written; change the path or press Esc")
}

fn missing_line(missing: &[&str]) -> String {
    let named: Vec<String> = missing
        .iter()
        .map(|section| format!("## {section}"))
        .collect();
    let sections = warlock_tui::and_listed(&named);
    format!("the document is missing {sections}, so nothing was written")
}

fn unreadable_shape_line(error: &TemplateError) -> String {
    format!(
        "could not read the brief shape, so nothing was written: {}",
        one_line(&error.to_string())
    )
}

fn failure_line(stored: &str, error: &io::Error) -> String {
    format!("could not write {stored}: {}", one_line(&error.to_string()))
}

// Deliberately narrow: all three of these have to hold, or the reply is handed
// back untouched — the first line of the trimmed reply opens a fence, the last
// closes it, and no line between them closes it. The third is what tells a
// document wrapped in a fence from a document containing fenced examples, which
// is far more common in the briefs this writes, and getting it wrong would
// silently eat the first and last lines of somebody's document.
//
// Both fence characters markdown defines are understood, and a closing fence
// must be at least as long as the one that opened it, so a document whose own
// examples are three backticks long survives being handed back inside a
// four-backtick fence.
//
// A slice of the input rather than a new `String`: whoever writes the file
// writes these bytes, and copying them would be a second place for them to
// change.
pub(crate) fn unfenced(reply: &str) -> &str {
    let block = reply.trim();
    let Some((first, rest)) = block.split_once('\n') else {
        return reply;
    };
    let Some(fence) = opening_fence(first) else {
        return reply;
    };
    // A block with nothing in it is `rest` being the closing fence alone, with
    // no newline left to split on.
    let (body, last) = rest.rsplit_once('\n').unwrap_or(("", rest));
    if !closes(last, fence) || body.lines().any(|line| closes(line, fence)) {
        return reply;
    }
    body
}

// The first `# ` line and not the longest, the outermost or the one the model
// meant: a document's title is its first heading, and anything cleverer would
// mean parsing markdown to propose a filename. The prefix must be `# ` exactly,
// so an indented heading, a `#Title` with no space and a `## Section` are all
// not it — and a reply with only those is `untitled`, which is a field the
// reader can type into rather than a guess they have to undo.
pub(crate) fn slug_of(document: &str) -> String {
    let title = document
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or_default();
    let slug = slugged(title);
    if slug.is_empty() {
        return UNTITLED.to_owned();
    }
    slug
}

// A directory that cannot be read proposes the same number as an empty one on
// purpose. Refusing to propose a path costs the reader the prompt, and saying so
// on a line says nothing they can act on — the directory is about to be created
// by the write, and the field in front of them is where a number they disagree
// with gets fixed.
//
// Every entry counts, files and directories alike, because "the highest number
// already there" is a fact about the names in front of a reader rather than
// about which of them warlock recognises.
fn next_number(directory: &Path) -> u32 {
    let Ok(entries) = fs::read_dir(directory) else {
        return 1;
    };
    let highest = entries
        .flatten()
        .filter_map(|entry| highest_number_in(&entry.file_name().to_string_lossy()))
        .max()
        .unwrap_or(0);
    // Saturating rather than wrapping, so a directory holding a name with four
    // billion in it proposes an absurd number instead of proposing `01` and
    // colliding with the first brief ever written there.
    highest.saturating_add(1)
}

fn spelled(number: u32) -> String {
    format!("{number:02}")
}

fn highest_number_in(name: &str) -> Option<u32> {
    name.split(|character: char| !character.is_ascii_digit())
        .filter_map(|run| run.parse::<u32>().ok())
        .max()
}

fn slugged(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    capped(slug.trim_matches('-')).to_string()
}

// Three cases, and the third is why the cap is "about". A slug that fits comes
// back whole; one that does not is cut back to the last hyphen inside the cap;
// and one whose first word is itself longer than the cap is cut after that word,
// however long it is, because there is nowhere to break it and half a word is
// not a name.
fn capped(slug: &str) -> &str {
    let Some((cut, _)) = slug.char_indices().nth(SLUG_MAX) else {
        return slug;
    };
    // The character just past the cap is the separator itself: the last word
    // ends exactly on the cap, so everything up to it fits.
    if slug[cut..].starts_with('-') {
        return &slug[..cut];
    }
    if let Some(hyphen) = slug[..cut].rfind('-') {
        return &slug[..hyphen];
    }
    match slug[cut..].find('-') {
        Some(end) => &slug[..cut + end],
        None => slug,
    }
}

#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    width: usize,
}

// The markdown rule, less the indentation it allows: three or more backticks or
// tildes, then an info string, which for a backtick fence may not itself contain
// a backtick — that last is what keeps a first line merely quoting some
// backticks at each other from being read as a fence somebody opened.
fn opening_fence(line: &str) -> Option<Fence> {
    let line = line.trim_end();
    let marker = line
        .chars()
        .next()
        .filter(|character| matches!(character, '`' | '~'))?;
    let width = line
        .chars()
        .take_while(|&character| character == marker)
        .count();
    if width < 3 {
        return None;
    }
    let info = line.trim_start_matches(marker);
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some(Fence { marker, width })
}

fn closes(line: &str, fence: Fence) -> bool {
    let line = line.trim();
    line.chars().count() >= fence.width && line.chars().all(|character| character == fence.marker)
}

// Every rule is a function of a string and at most one directory of the test's
// own, so the whole suite runs with no terminal, no network, no `claude` and
// nothing written anywhere but a temporary directory.
#[cfg(test)]
#[path = "tests/writing.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/writing_writes.rs"]
mod writes;
