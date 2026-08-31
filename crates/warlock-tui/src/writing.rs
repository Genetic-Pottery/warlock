//! Where a brief goes, and the writing of it: the reply unwrapped, its title
//! slugged, the next number in the directory, and then the bytes on disk.
//!
//! Two halves, and the seam between them is the field. [`proposed_path`] is the
//! arithmetic behind the path the `/write` prompt opens holding —
//! `docs/warlock-brief-13-scopes-and-sigils.md` — and it writes nothing;
//! [`write_submit`] is what Enter in that field comes to, and it is the one
//! thing in warlock that puts a document somebody asked for on disk. Everything
//! between the two is the reader's: the field is editable, so the path that is
//! written is the path on screen when Enter was pressed and never the proposal
//! unless they left it alone.
//!
//! ## The proposal is only a proposal
//!
//! Every rule below is a guess at what somebody would have typed, and the field
//! it lands in is editable, which is what makes the guesses cheap. So none of
//! them refuses anything: a reply with no title still gets a path, a title made
//! entirely of punctuation still gets a path, and a directory holding a file
//! with `2026` in its name still gets a path. The reader reads the line and
//! changes it or presses Enter. Nothing in that half can fail, which is why none
//! of it returns a [`Result`].
//!
//! ## The directory is handed in
//!
//! Where a brief goes is a parameter of both halves rather than a constant of
//! this module: the event loop holds it as a local, settled once when brief mode
//! is entered and held for the life of that mode, and hands it down with the
//! reply. Nothing in here reads a config file, and nothing in here can fail for
//! want of one — by the time `/write` asks for a path the directory is a string
//! somebody else has already got hold of. It is joined onto the repository root
//! exactly as the built-in `docs/` was, and the proposal is still spelled
//! relative to that root.
//!
//! ## The unwrap comes before the title
//!
//! A model asked for a document sometimes hands back the document inside one
//! fenced block, which is why the written file is unwrapped before it is
//! written. The heading is looked for *after* that same unwrap, so a fenced
//! reply's title still supplies the slug rather than the slug being `untitled`
//! for a document whose first line is a fence. [`unfenced`] is that one
//! transformation, and it is the same function the write itself will use, so the
//! path proposed and the bytes written can never disagree about what the
//! document is.
//!
//! It unwraps only a reply that is *entirely* one block: an opening fence on the
//! first line, a closing fence on the last, and no closing fence in between. A
//! document that merely *contains* code blocks is left exactly as it stands,
//! because that is a document, and stripping its first and last lines would be
//! warlock editing prose it promised to copy.
//!
//! ## The slug is the title, folded, and no cleverer than that
//!
//! Lower-cased, runs of non-alphanumerics collapsed to a single hyphen, leading
//! and trailing hyphens trimmed. Nothing before a colon is stripped:
//! `# Scopes: the boundary problem` becomes `scopes-the-boundary-problem`,
//! because a title's first word is usually the subject and a filename that
//! throws it away is a filename nobody can find. There is no stop-word list, no
//! transliteration and no dictionary — every one of those would be warlock
//! having opinions about English in a string the reader is looking at and can
//! retype.
//!
//! The cap is *about* sixty characters and is broken at a hyphen, so the slug
//! ends on a whole word rather than mid-word. A single word longer than the cap
//! has nowhere to break and is kept whole, which is why the cap is "about": a
//! name cut into nonsense is worse than a name a few characters over.
//!
//! A reply with no `# ` line, and a title that leaves nothing behind once it is
//! folded, both come out as [`UNTITLED`]. A literal word rather than an empty
//! string, because `docs/warlock-brief-13-.md` reads as a bug and
//! `docs/warlock-brief-13-untitled.md` reads as an invitation to type.
//!
//! ## The number is one above the highest one there, wherever it is in the name
//!
//! Every digit run in every name in the directory counts, rather than only those
//! matching warlock's own filename shape. `docs/` today holds
//! `red-brief-12-…`, written before warlock existed and by another hand, and a
//! rule that only counted `warlock-brief-NN-…` would propose `01` into a
//! directory whose next brief is plainly the thirteenth. The cost is that a name
//! carrying a year proposes a number nobody expected — and that is a line in an
//! editable field, on screen, one keystroke from being fixed.
//!
//! Two digits with a leading zero below ten, so the directory sorts the way it
//! reads. Past ninety-nine the number simply gets wider — `100`, not `00` and
//! not a hundred pretending to be `99` — because the padding exists to make
//! `01` sort before `10`, and a wider number keeps that true for as long as the
//! widths agree.
//!
//! # The write
//!
//! ## Everything is spelled from the repository root
//!
//! Decided once, here: whatever is typed into the field resolves against the
//! repository root — the directory `.warlock/` and the manifest live under —
//! and not against the tree's root, which may be a subdirectory somebody
//! started warlock in, and not against the process's working directory, which
//! nothing on screen names. That is one rule for three things at once: where
//! the bytes land, how the written path is spelled on the line that announces
//! it, and which entries of `.warlock/pacts.toml` count as above it. A path
//! that climbs out of the repository is refused rather than followed, because
//! the artifact belongs to the repository the ledger is about.
//!
//! ## Two transformations, and warlock is not a markdown editor
//!
//! The reply is written as it stands but for exactly two things: [`unfenced`]
//! takes off a fence wrapped around the whole document, and a single trailing
//! newline is ensured so the file ends the way a text file does. Nothing else is
//! parsed, reformatted, re-indented, spell-checked or inspected — not the
//! headings, not the links, not the width of a line — because the document is
//! the model's and the reader's, and every further rule would be warlock editing
//! prose it promised to copy.
//!
//! The bytes come off the card. The reply is on the conversation as an ordinary
//! answer, and [`write_submit`] writes the newest turn's answer, so what lands
//! in the file is what the reader was looking at when they pressed Enter —
//! there is no second copy of the document anywhere for the two to disagree
//! about, and nothing is kept on disk between the answer and the write.
//!
//! ## A path that exists is refused, never overwritten
//!
//! There is one destructive thing a write can do, and this is it. So a target
//! that exists at all — file, directory or anything else — writes nothing and
//! comes back through [`ScopeField::refused`] with the rule under the field and
//! the typed path still in it, one keystroke from being changed. No `.bak`, no
//! suffix warlock invents, no prompt asking a second time.
//!
//! ## Two lines, both facts
//!
//! A write that lands says the path and the size, in the panel's own spelling
//! through [`size`], and then — when one exists — names the nearest directory
//! above the new file that has a pact in the manifest, because that directory's
//! document now describes a subtree with a file in it that the document has
//! never seen. Both are facts rather than narration: the first is what is now on
//! disk, the second is what the ledger now says. A file written where nothing
//! above it is pacted says only the first line, since there is no ledger entry
//! to have gone stale.
//!
//! ## A failed write is a line, not an error
//!
//! [`scope_submit`](crate::scoping::scope_submit)'s rule exactly: a disk that
//! will not take the file puts its reason on
//! [`App::message`](warlock_tui::App::message) and the prompt comes down.
//! Nothing here returns out of the event loop, because a write that did not
//! happen is news for the footer and not a reason to tear the screen down.

use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use warlock_engine::{Manifest, PactEntry, from_manifest_path, to_manifest_path};
use warlock_tui::{App, Edited, ScopeField, ScopePrompt, size};

use crate::error::{Error, one_line};

/// What the write prompt's window is headed with.
///
/// Carried in [`ScopeField::directory`] — which this window never reads as a
/// directory, only prints — because that is the one string the field already
/// has for saying what it is asking about. The scope prompt puts a module path
/// there and the window heads itself "Scope for <module>"; this one puts the
/// whole heading there, because what a path prompt is about is not a directory
/// but the question itself.
pub(crate) const WRITE_HEADING: &str = "Write the brief to";

/// What every brief's filename begins with, before its number and its slug.
const BRIEF_PREFIX: &str = "warlock-brief";

/// The slug for a document that gave nothing to make one out of.
const UNTITLED: &str = "untitled";

/// About how many characters of the title survive into the slug, before the
/// last whole word that fits.
const SLUG_MAX: usize = 60;

/// How the manifest spells the repository root.
///
/// The engine's own `ROOT_MODULE`, which is not exported, written down again
/// here because the ancestor walk below has to be able to arrive at it. One
/// character, fixed by the file format, and if it ever moved the manifest would
/// stop parsing long before this line was noticed.
const ROOT_MODULE: &str = ".";

/// What the field says when Enter is pressed on nothing.
///
/// Not a write and not a close: an empty field is a reader who has cleared the
/// line and is about to type, so the prompt stays up over the rule rather than
/// answering a question they have not finished asking.
const NO_PATH: &str = "type a path for the document, or press Esc to write nothing";

/// What the footer says when there is no reply to write.
///
/// Unreachable in the loop — the prompt opens over an answer that has landed —
/// and answered anyway, because a window whose Enter did nothing at all is the
/// one outcome a reader cannot tell from success.
const NOTHING_TO_WRITE: &str =
    "there is no answer on the conversation to write, so nothing was written";

/// The path the write prompt opens holding, spelled relative to the repository
/// root: `<directory>/warlock-brief-NN-slug.md`.
///
/// The three rules in one line, in the order they depend on each other: the
/// reply is unwrapped, the slug comes off the unwrapped document's first `# `
/// line, and the number comes off the names in the directory the file would
/// land in. Relative rather than absolute because it is going into a field
/// somebody reads and edits, and because it is the spelling the thread names
/// the written file by afterwards.
///
/// `directory` is the caller's — see the module docs — and it is the one answer
/// to two questions: where the file is proposed to go, and which directory's
/// names the number is counted from. Reads `repo_root/directory` and nothing
/// else, creates nothing, and is as happy with a directory that is not there as
/// with an empty one.
pub(crate) fn proposed_path(repo_root: &Path, directory: &str, reply: &str) -> String {
    let number = spelled(next_number(&repo_root.join(directory)));
    let slug = slug_of(unfenced(reply));
    format!("{directory}/{BRIEF_PREFIX}-{number}-{slug}.md")
}

/// The write prompt as it opens over `reply`: up, headed [`WRITE_HEADING`], and
/// holding [`proposed_path`]'s guess at where the document goes.
///
/// The counterpart of [`scope_press`](crate::scoping::scope_press) for a window
/// no keystroke opens. Nothing presses a key to get here — the prompt opens
/// because a `/write` turn answered, and the answer is what it opens over — so
/// this takes the reply rather than the app: the document handed back by the
/// drain is what the proposal is made of, which is what keeps the path on screen
/// about the very turn that just landed.
///
/// `directory` comes the same way, off the loop, and is the value the mode was
/// entered holding: nothing is read here to find it, which is what makes a
/// window that opens over a finished document a window that cannot fail to open.
///
/// It refuses nothing and cannot fail. The path is a proposal in an editable
/// field (see the module docs), so a reply with no title, a directory full of
/// years, and a repository with no output directory at all each open a window
/// with a line in it and no complaint anywhere.
pub(crate) fn write_opened(repo_root: &Path, directory: &str, reply: &str) -> ScopePrompt {
    ScopePrompt::open(WRITE_HEADING, proposed_path(repo_root, directory, reply))
}

/// What one keystroke *inside* the write prompt comes to: the prompt the event
/// loop holds from here on.
///
/// [`scope_edit`](crate::scoping::scope_edit)'s twin, over the other window and
/// with the other submit at the end of it — the same three answers from the same
/// [`edit_for`](warlock_tui::edit_for), because it is the same field type and
/// the same editor.
///
/// Typing moves the field and nothing else. Esc takes the window down and
/// writes nothing: the app was never told the question was asked, so the reply
/// is still on the card, the register is still whatever it was, and there is
/// nothing to put back. Enter is [`write_submit`] and only [`write_submit`],
/// which either writes the file and closes, or reopens this window over the rule
/// the path broke.
///
/// A closed prompt cannot submit: [`press_for`](crate::input::press_for) only
/// consults `edit_for` while one is up, so the `None` road below is unreachable
/// rather than silent. It answers with a closed prompt for the reason
/// `scope_edit` does — a submit that found no field to write is not a window
/// anybody can still be typing into.
pub(crate) fn write_edit(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    prompt: &ScopePrompt,
    edited: Edited,
    now: Instant,
) -> ScopePrompt {
    match edited {
        Edited::Open(field) => ScopePrompt::Open(field),
        Edited::Close => ScopePrompt::Closed,
        Edited::Submit => match prompt.field() {
            Some(field) => write_submit(app, manifest, repo_root, field, now),
            None => ScopePrompt::Closed,
        },
    }
}

/// What Enter in the write prompt comes to: the document on disk and two lines
/// on the conversation, or the prompt still up over the reason it is not.
///
/// [`scope_submit`](crate::scoping::scope_submit)'s shape, and deliberately the
/// same one — this runs on the event loop's own thread between two frames,
/// spawns nothing, and hands back the prompt the loop holds next:
/// [`ScopePrompt::Closed`] for a submit that was answered one way or another,
/// and an open prompt over the same text for one that was refused.
///
/// The path is what is in the field, trimmed of the whitespace an editable line
/// collects, and it resolves against `repo_root` — see the module docs, where
/// that is decided once. `field.directory()` is not consulted at all: in this
/// window it carries the heading the prompt is drawn under, and the answer is
/// the line the reader typed.
///
/// The order is judge, then write, and every refusal happens before a byte
/// moves. A path that climbs out of the repository, and a path that is already
/// taken, both reopen the field with the rule under it and the typed text
/// exactly where it was. Only then is the reply taken off the card, the parent
/// directory made if it is missing, and the bytes written — after which the
/// thread gains the line naming what was written and, if a pact sits above it,
/// the line naming what that made stale.
///
/// `now` is the caller's instant, as every line on the conversation is timed by
/// the loop's clock rather than by one this function reads for itself.
pub(crate) fn write_submit(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    field: &ScopeField,
    now: Instant,
) -> ScopePrompt {
    let typed = field.text().trim();
    if typed.is_empty() {
        return refused(field, NO_PATH);
    }
    // The one spelling of the path, produced before anything is done with it:
    // the bytes go to it, the line names it, and the ancestor walk climbs it,
    // so all three are the same string and cannot come to disagree.
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
        app.set_message(NOTHING_TO_WRITE);
        return ScopePrompt::Closed;
    };
    if let Err(error) = put(&path, document.as_bytes()) {
        app.set_message(failure_line(&stored, &error));
        return ScopePrompt::Closed;
    }

    // The size is the bytes just handed to the disk rather than a `stat` of what
    // came back: it is the same number, and asking the filesystem again would be
    // a second way for this line to fail after the write succeeded.
    let bytes = u64::try_from(document.len()).unwrap_or(u64::MAX);
    app.note(wrote_line(&stored, bytes), now);
    if let Some(module) = pacted_above(manifest, &stored) {
        app.note(stale_line(module), now);
    }
    ScopePrompt::Closed
}

/// The prompt still up over `rule`, with the text and the cursor exactly where
/// they were.
///
/// The one road back to the field, so every refusal in [`write_submit`] leaves
/// the window in the same state: the reader's own line, one keystroke from being
/// fixed, and the reason for the last Enter under it.
fn refused(field: &ScopeField, rule: impl Into<String>) -> ScopePrompt {
    ScopePrompt::Open(field.clone().refused(rule))
}

/// The bytes to write for `reply`: the fence off, and a trailing newline on.
///
/// The whole of what warlock does to a document, in one function so that the
/// promise in the module docs is a thing that can be read rather than a
/// discipline spread over a write. [`unfenced`] is the same call
/// [`proposed_path`] made, so the document the slug came from is the document
/// that lands.
///
/// The newline is *ensured*, not normalised: a reply already ending in one is
/// copied byte for byte, and a reply ending in three keeps all three. Trimming
/// them back would be a third transformation, and the one this makes exists only
/// so the file ends the way a text file ends.
fn document(reply: &str) -> String {
    let body = unfenced(reply);
    if body.ends_with('\n') {
        return body.to_owned();
    }
    format!("{body}\n")
}

/// The document to write, taken off the conversation's newest turn, or `None`
/// when there is no answer there to write.
///
/// The reply is on the card as an ordinary answer and this is warlock reading
/// it back off there — the newest turn, which is the `/write` turn whose answer
/// the prompt opened over, since nothing can be asked while the field holds the
/// keyboard. One copy of the document, on the card the reader is looking at.
fn document_on(app: &App) -> Option<String> {
    let thread = app.thread()?;
    let reply = thread.turns().last().copied()?.answer()?;
    Some(document(reply))
}

/// `bytes` at `path`, with the directory above it made first if it is not there.
///
/// The whole of the disk work, and the only two calls in warlock that create a
/// directory and a file for a document somebody asked for. `create_dir_all` is
/// happy with a directory that already exists, so there is nothing to check
/// first, and the write refuses nothing — [`write_submit`] has already settled
/// that this path is free.
fn put(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

/// The nearest directory at or above the one `stored` lands in that has an entry
/// in the manifest, or `None` when nothing above it is pacted.
///
/// The walk starts at the file's own directory rather than at the file, because
/// a pact is on a directory and the file itself has just been created. Segments
/// are cut at `/` exactly as `warlock_engine::scope`'s own ancestor walk cuts
/// them, so the ancestors of `docs/adr/x.md` are `docs/adr`, `docs` and `.`, and
/// a `docs-old` entry is never among them however much of a prefix it looks
/// like.
///
/// Nearest wins and the walk stops there: the directory that has to be described
/// again is the one whose document is closest to the new file, and naming every
/// pact above it would be a list of work nobody asked for.
///
/// The name comes back off the entry rather than out of the walk, which is what
/// makes the line say the module exactly as `.warlock/pacts.toml` spells it —
/// including `.` for a repository pacted at its root.
fn pacted_above<'manifest>(manifest: &'manifest Manifest, stored: &str) -> Option<&'manifest str> {
    at_or_above(directory_of(stored))
        .find_map(|module| manifest.entry(module).map(PactEntry::module))
}

/// The stored path of the directory `stored` sits in: `docs` for
/// `docs/x.md`, and the root for a file written beside it.
fn directory_of(stored: &str) -> &str {
    match stored.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => ROOT_MODULE,
    }
}

/// The stored path `stored` and every stored path above it, nearest first,
/// ending at [`ROOT_MODULE`].
///
/// `warlock_engine::scope`'s own walk, which is private to that module: the
/// engine decides what a scope covers, and this decides which pact a written
/// file staled. Copying eight lines is the cost of not opening a door in the
/// engine for the TUI to reach through — and it is the shape rather than the
/// judgement that is shared, so nothing about scopes, sigils or manifests is
/// duplicated by it.
fn at_or_above(stored: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(stored);
    std::iter::from_fn(move || {
        let current = next?;
        next = match current.rsplit_once('/') {
            // A path with a parent segment: `docs` above `docs/adr`.
            Some((parent, _)) => Some(parent),
            // A single segment sits directly under the root, and the root sits
            // under nothing.
            None if current == ROOT_MODULE => None,
            None => Some(ROOT_MODULE),
        };
        Some(current)
    })
}

/// The line the conversation gains when a document lands: what was written and
/// how big it is.
///
/// The path as the field spelled it, relative to the repository root, because
/// that is the path the reader typed and the one they will go looking for. The
/// size through [`size`], which is the account's own spelling, shared rather
/// than restated — see its doc comment.
fn wrote_line(stored: &str, bytes: u64) -> String {
    format!("wrote {stored} — {}", size(bytes))
}

/// The second line: the pact the new file has just made stale.
///
/// Present tense and no advice. The ledger says a directory's document no longer
/// describes it, and which key puts that right is the footer's business and the
/// reader's — a sentence telling them to press `r` here would be warlock
/// narrating rather than stating.
fn stale_line(module: &str) -> String {
    format!("{module} is now stale")
}

/// The rule under the field when the path is already taken.
///
/// It names the path rather than saying "that file exists", because the field
/// may hold a path several directories deep and the reader is looking at what
/// they typed rather than at what is on disk. Overwriting is the one destructive
/// thing this key could do, so it is refused rather than confirmed.
fn taken_rule(stored: &str) -> String {
    format!("{stored} already exists — nothing was written; change the path or press Esc")
}

/// The footer line when the disk would not take the file.
///
/// One line, naming the path warlock tried and the reason it came back with, in
/// the operating system's own words flattened the way every other non-fatal
/// failure in this binary is.
fn failure_line(stored: &str, error: &io::Error) -> String {
    format!("could not write {stored}: {}", one_line(&error.to_string()))
}

/// The document inside a reply that is entirely one fenced code block, or the
/// reply exactly as it stands.
///
/// The one transformation warlock makes to a reply besides ensuring a trailing
/// newline, and it is deliberately narrow. All three of these have to hold, or
/// the reply is handed back untouched: the first line of the trimmed reply
/// opens a fence, the last line closes it, and no line between them closes it.
/// The third is what tells a document wrapped in a fence from a document
/// containing fenced examples — the second kind is far more common in the
/// briefs this writes — and getting it wrong would silently eat the first and
/// last lines of somebody's document.
///
/// Both of the fence characters markdown defines are understood, since which one a
/// model reaches for is not warlock's decision to have a preference about, and a
/// closing fence must be at least as long as the one that opened it, so a
/// document whose own examples are three backticks long survives being handed
/// back inside a four-backtick fence.
///
/// A slice of the input rather than a new [`String`]: whoever writes the file
/// writes these bytes, and copying them would be a second place for them to
/// change.
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

/// The slug for a document: its first `# ` line folded into a filename, or
/// [`UNTITLED`] when there is nothing to fold.
///
/// The first such line and not the longest, the outermost or the one the model
/// meant — a document's title is its first heading, and looking for anything
/// cleverer would mean parsing markdown to propose a filename. The line must
/// begin with `# ` exactly: an indented heading, a `#Title` with no space and a
/// `## Section` are all not it, and a reply that has only those is `untitled`,
/// which is a field the reader can type into rather than a guess they have to
/// undo.
pub(crate) fn slug_of(document: &str) -> String {
    let title = document
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or_default();
    let slug = slugged(title);
    if slug.is_empty() {
        return UNTITLED.to_string();
    }
    slug
}

/// One above the highest number appearing anywhere in any name in `directory` —
/// or `1` for a directory that is empty, unreadable or not there at all.
///
/// A directory that cannot be read proposes the same number as an empty one on
/// purpose. The alternatives are refusing to propose a path, which costs the
/// reader the prompt, or saying so on a line, which says nothing they can act on
/// — the directory is about to be created by the write, and the field in front
/// of them is where a number they disagree with gets fixed.
///
/// Every entry counts, files and directories alike, because "the highest number
/// already there" is a fact about the names in front of a reader rather than
/// about which of them warlock recognises.
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

/// A number as a filename spells it: `01`, `13`, `100`.
///
/// Two digits so the directory sorts the way it reads, and no ceiling, because
/// the padding is there to keep `01` ahead of `10` rather than to promise a
/// width. A hundredth brief is `100`.
fn spelled(number: u32) -> String {
    format!("{number:02}")
}

/// The highest number any run of digits in `name` spells, or `None` for a name
/// with no digits in it.
///
/// A run too long to be a [`u32`] is not a number anybody meant, and is ignored
/// rather than clamped: a checksum in a filename should not decide what the next
/// brief is called.
fn highest_number_in(name: &str) -> Option<u32> {
    name.split(|character: char| !character.is_ascii_digit())
        .filter_map(|run| run.parse::<u32>().ok())
        .max()
}

/// `title` as a filename spells it: lower-cased, runs of non-alphanumerics
/// collapsed to one hyphen, hyphens trimmed off both ends, and capped.
///
/// [`char::is_alphanumeric`] rather than an ASCII test, so a title in a script
/// warlock has never heard of keeps its own letters instead of folding away to
/// nothing. What is left is empty only when the title held no letter or digit at
/// all, and [`slug_of`] answers that with [`UNTITLED`].
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

/// `slug` cut to about [`SLUG_MAX`] characters at a hyphen.
///
/// Three cases, and the third is the reason the cap is "about". A slug that fits
/// is returned whole; a slug that does not is cut back to the last hyphen inside
/// the cap; and a slug whose first word is itself longer than the cap is cut
/// after that word, however long it is, because there is nowhere to break it and
/// half a word is not a name.
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

/// A fence as the line that opened it defines it: which character it is made of,
/// and how many of them the fence that closes it needs.
#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    width: usize,
}

/// The fence `line` opens, or `None` for a line that opens none.
///
/// The markdown rule, less the indentation it allows: three or more backticks or
/// tildes, then an info string, which for a backtick fence may not itself
/// contain a backtick — that last is what keeps a first line that merely quotes
/// some backticks at each other from being read as a fence somebody opened.
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

/// Whether `line` closes `fence`: that character and nothing else, at least as
/// many of them as opened it.
fn closes(line: &str, fence: Fence) -> bool {
    let line = line.trim();
    line.chars().count() >= fence.width && line.chars().all(|character| character == fence.marker)
}

/// What a `/write` proposes to call the file: the fence taken off, the title
/// folded, the number counted, and the whole path assembled out of the three.
///
/// Every rule is a function of a string and at most one directory of the test's
/// own, so the whole suite runs with no terminal, no network, no `claude` and
/// nothing written anywhere but a temporary directory that goes away with the
/// test that made it.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{SLUG_MAX, next_number, proposed_path, slug_of, unfenced};

    /// A repository of this test's own, removed when the test that made it
    /// ends.
    fn a_repo() -> TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The default output directory, which is the caller's answer everywhere
    /// below but the one test about a repository that keeps its briefs
    /// elsewhere.
    const DOCS: &str = "docs";

    /// `directory` under `root` holding an empty file of each of `names`, which
    /// is all the numbering ever looks at.
    fn holding(root: &Path, directory: &str, names: &[&str]) {
        let output = root.join(directory);
        fs::create_dir_all(&output).expect("makes the output directory");
        for name in names {
            fs::write(output.join(name), "").expect("writes a file into it");
        }
    }

    /// [`holding`] over the default directory, which is where all but one of
    /// these tests put their names.
    fn docs_holding(root: &Path, names: &[&str]) {
        holding(root, DOCS, names);
    }

    /// The whole of a reply that is one document with one title line.
    fn titled(title: &str) -> String {
        format!("{title}\n\nSome prose about it.\n")
    }

    #[test]
    fn a_title_is_lowercased_and_hyphenated_at_every_run_of_anything_else() {
        for (title, slug) in [
            ("# Freshness", "freshness"),
            ("# The Brief Workflow", "the-brief-workflow"),
            // Runs collapse to one hyphen however long they are, and whatever
            // they are made of.
            ("# Read   what  it wrote", "read-what-it-wrote"),
            ("# Scopes & sigils", "scopes-sigils"),
            ("# back  --  to  green", "back-to-green"),
            // Both ends trimmed, so no filename starts or ends in a separator.
            ("# --Big files!--", "big-files"),
            ("#    At work   ", "at-work"),
            // Digits are alphanumeric and stay where they are.
            ("# 12 factor apps", "12-factor-apps"),
            // A script warlock has never heard of keeps its own letters rather
            // than folding away to nothing.
            ("# Дом and home", "дом-and-home"),
        ] {
            assert_eq!(slug_of(&titled(title)), slug, "the slug for {title:?}");
        }
    }

    #[test]
    fn nothing_before_a_colon_is_stripped() {
        // The rule that is easiest to get wrong by being helpful: a title's
        // first word is usually its subject, and a filename that throws it away
        // is a filename nobody can find again.
        assert_eq!(
            slug_of(&titled("# Scopes: the boundary problem")),
            "scopes-the-boundary-problem"
        );
    }

    #[test]
    fn the_first_hash_space_line_is_the_title_and_nothing_else_is() {
        // A heading is a line beginning with `# ` exactly. A deeper heading, an
        // indented one and a hash with no space after it are all not one, so
        // the first *real* heading wins however far down it is.
        let reply = "## Not this\n   # Nor this\n#NorThis\n\n# The real title\n\n# A later one\n";

        assert_eq!(slug_of(reply), "the-real-title");
    }

    #[test]
    fn a_title_capped_at_about_sixty_characters_breaks_at_a_hyphen() {
        let slug = slug_of(&titled(
            "# The quick brown fox jumps over the lazy dog and keeps on running for a while",
        ));

        // Cut back to the last whole word inside the cap: never `…-run`.
        assert_eq!(
            slug,
            "the-quick-brown-fox-jumps-over-the-lazy-dog-and-keeps-on"
        );
        assert!(slug.chars().count() <= SLUG_MAX, "{slug} is over the cap");
        assert!(!slug.ends_with('-'), "{slug} ends on a separator");
    }

    #[test]
    fn a_slug_that_fits_is_left_exactly_as_it_is() {
        // Sixty characters on the nose, and the word after it: the first is
        // never touched, and the second is cut at the separator that is already
        // there rather than one character earlier.
        let sixty = "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee fffff";
        let folded = "aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-fffff";
        assert_eq!(folded.chars().count(), SLUG_MAX);

        assert_eq!(slug_of(&titled(&format!("# {sixty}"))), folded);
        assert_eq!(slug_of(&titled(&format!("# {sixty} ggggg"))), folded);
    }

    #[test]
    fn one_word_longer_than_the_cap_is_kept_whole_rather_than_cut_in_half() {
        // "About sixty" is the whole of the promise: there is nowhere to break
        // this, and half a word is not a name.
        let long = "a".repeat(SLUG_MAX + 20);

        assert_eq!(slug_of(&titled(&format!("# {long}"))), long);
        assert_eq!(slug_of(&titled(&format!("# {long} and more"))), long);
    }

    #[test]
    fn a_reply_that_is_entirely_one_fenced_block_is_unwrapped() {
        for reply in [
            "```\n# Freshness\n\nProse.\n```",
            // An info string, which is what a model actually writes.
            "```markdown\n# Freshness\n\nProse.\n```\n",
            // Whitespace either side of the whole thing, and a closing fence
            // longer than the one that opened it.
            "\n\n~~~\n# Freshness\n\nProse.\n~~~~\n\n",
        ] {
            assert_eq!(unfenced(reply), "# Freshness\n\nProse.", "{reply:?}");
        }
    }

    #[test]
    fn a_document_that_merely_contains_fenced_blocks_is_left_exactly_as_it_is() {
        for reply in [
            // The ordinary case: prose with an example in it.
            "# Freshness\n\n```rust\nlet x = 1;\n```\n\nMore prose.\n",
            // Two blocks and nothing else, which opens and closes twice.
            "```\none\n```\n\n```\ntwo\n```\n",
            // A block that is never closed.
            "```\n# Freshness\n\nProse.\n",
            // A fence made of a different character than the one that opened
            // it, and a closing fence shorter than the opening one.
            "````\n# Freshness\n```\n",
            "~~~\n# Freshness\n```\n",
            // Not a fence at all.
            "# Freshness\n\nProse.\n",
            "`code` and ```more```\n# Freshness\n```\n",
            // One line, so there is no last line for a fence to close on.
            "# Freshness",
        ] {
            assert_eq!(unfenced(reply), reply, "{reply:?} was interfered with");
        }
    }

    #[test]
    fn the_title_is_looked_for_after_the_unwrap() {
        // The whole reason the two are ordered: a fenced reply's first line is
        // a fence, and a slug taken before the unwrap would be `untitled` for a
        // document that plainly has a title.
        let repo = a_repo();

        assert_eq!(
            proposed_path(
                repo.path(),
                DOCS,
                "```markdown\n# Scopes and sigils\n\nProse.\n```\n"
            ),
            "docs/warlock-brief-01-scopes-and-sigils.md"
        );
    }

    #[test]
    fn a_reply_with_no_heading_is_untitled() {
        let repo = a_repo();

        // The literal word, in the whole path, so the field opens on something
        // that reads as an invitation to type rather than as a bug.
        assert_eq!(
            proposed_path(repo.path(), DOCS, "Prose with no title anywhere in it.\n"),
            "docs/warlock-brief-01-untitled.md"
        );
    }

    #[test]
    fn a_title_that_folds_away_to_nothing_is_untitled_too() {
        let repo = a_repo();

        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# ---")),
            "docs/warlock-brief-01-untitled.md"
        );
    }

    #[test]
    fn the_number_is_one_above_the_highest_in_any_name_in_the_directory() {
        let repo = a_repo();
        // What `docs/` actually holds: documents written by another hand, in
        // another shape, which a rule that only counted warlock's own filenames
        // would ignore.
        docs_holding(
            repo.path(),
            &[
                "red-brief-01-foundations.md",
                "red-brief-12-the-brief-workflow.md",
                "red-brief-09-scopes-and-sigils.md",
                "warlock-design-doc.md",
            ],
        );

        assert_eq!(next_number(&repo.path().join("docs")), 13);
        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# Scopes and sigils")),
            "docs/warlock-brief-13-scopes-and-sigils.md"
        );
    }

    #[test]
    fn an_empty_or_absent_directory_proposes_01() {
        // Absent: nothing has ever been written here, which is every repository
        // the first time.
        let repo = a_repo();
        assert_eq!(next_number(&repo.path().join("docs")), 1);
        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# Foundations")),
            "docs/warlock-brief-01-foundations.md"
        );

        // Empty: the directory is there and holds nothing, which is the same
        // answer.
        docs_holding(repo.path(), &[]);
        assert_eq!(next_number(&repo.path().join("docs")), 1);
        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# Foundations")),
            "docs/warlock-brief-01-foundations.md"
        );
    }

    #[test]
    fn a_number_counts_wherever_in_the_name_it_sits() {
        let repo = a_repo();
        // Named for what this costs as much as for what it buys: a year in a
        // filename is a number in a filename, and the answer lands in an
        // editable field rather than in a file.
        docs_holding(repo.path(), &["notes-2026-08-31.md", "red-brief-03-x.md"]);

        assert_eq!(next_number(&repo.path().join("docs")), 2027);
    }

    #[test]
    fn a_directory_is_a_name_in_the_directory_too() {
        let repo = a_repo();
        docs_holding(repo.path(), &["red-brief-02-x.md"]);
        fs::create_dir(repo.path().join("docs/07-archive")).expect("makes a subdirectory");

        assert_eq!(next_number(&repo.path().join("docs")), 8);
    }

    #[test]
    fn nothing_outside_the_output_directory_is_counted() {
        let repo = a_repo();
        docs_holding(repo.path(), &["red-brief-04-x.md"]);
        // Beside `docs/`, not in it.
        fs::write(repo.path().join("release-99.md"), "").expect("writes a file at the root");

        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# At work")),
            "docs/warlock-brief-05-at-work.md"
        );
    }

    #[test]
    fn the_directory_handed_in_is_both_the_path_and_what_the_number_is_counted_in() {
        // A repository that keeps its briefs somewhere else: the whole of what
        // the caller's directory changes, in one assertion. It is the first
        // segment of the proposal, and it is the directory whose names the
        // number comes off — so `plans/` holding an eighth brief proposes the
        // ninth there, and the four documents sitting in `docs/` count for
        // nothing because nothing is proposed into `docs/`.
        let repo = a_repo();
        docs_holding(repo.path(), &["red-brief-12-the-brief-workflow.md"]);
        holding(repo.path(), "plans", &["warlock-brief-08-x.md"]);

        assert_eq!(
            proposed_path(repo.path(), "plans", &titled("# Scopes and sigils")),
            "plans/warlock-brief-09-scopes-and-sigils.md"
        );
        // And a directory that is not there at all is the empty one's answer,
        // wherever it is: nothing about the default is special.
        assert_eq!(
            proposed_path(repo.path(), "notes/briefs", &titled("# Foundations")),
            "notes/briefs/warlock-brief-01-foundations.md"
        );
    }

    #[test]
    fn past_ninety_nine_the_number_gets_wider_rather_than_wrapping() {
        // The padding exists to keep `01` ahead of `10`; it is not a promise
        // about width, and a hundredth brief is `100` rather than `00`.
        let repo = a_repo();
        docs_holding(repo.path(), &["warlock-brief-99-x.md"]);

        assert_eq!(
            proposed_path(repo.path(), DOCS, &titled("# Keeping up")),
            "docs/warlock-brief-100-keeping-up.md"
        );
    }

    #[test]
    fn a_run_of_digits_too_long_to_be_a_number_is_ignored() {
        // A checksum in a filename is not somebody's numbering, and clamping to
        // the largest number there is would propose the same path forever
        // after.
        let repo = a_repo();
        docs_holding(
            repo.path(),
            &["sha-99999999999999999999999999.md", "red-brief-06-x.md"],
        );

        assert_eq!(next_number(&repo.path().join("docs")), 7);
    }

    #[test]
    fn proposing_a_path_writes_nothing_and_creates_nothing() {
        // The whole module in one assertion: it reads one directory and answers
        // with a string. Nothing here makes the directory it names, and the
        // repository is exactly as empty afterwards as it was before.
        let repo = a_repo();

        let path = proposed_path(repo.path(), DOCS, &titled("# Foundations"));

        assert_eq!(path, "docs/warlock-brief-01-foundations.md");
        assert!(!repo.path().join("docs").exists(), "the directory was made");
        assert_eq!(
            fs::read_dir(repo.path())
                .expect("the repository reads back")
                .count(),
            0,
            "something was written into the repository"
        );
    }
}

/// What Enter in the write prompt actually does: the bytes that end up on disk,
/// the two lines that end up on the conversation, what a refusal leaves behind,
/// and what — deliberately — is never written at all.
///
/// The whole path is driven over a repository of the test's own under the
/// temporary directory, with the field built by hand as the loop would be
/// holding it. No terminal, no network, no `claude` and no worker thread: a
/// write is a function of an app, a manifest, a root and a field, which is what
/// makes every rule below one assertion.
#[cfg(test)]
mod writes {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use tempfile::TempDir;
    use warlock_engine::{Manifest, Node, NodeState, PactEntry, Tree, to_manifest_path};
    use warlock_tui::{App, Line, ScopeField, ScopePrompt};

    use super::{NO_PATH, write_submit};

    /// The path the field holds in most of the tests below: the proposal, left
    /// exactly as it opened.
    const BRIEF: &str = "docs/warlock-brief-13-scopes-and-sigils.md";

    /// The line the last keystroke left on the footer, which a write that lands
    /// is not allowed to spend.
    const LAST_KEY: &str = "something the last key said";

    /// A repository of this test's own, removed when the test that made it
    /// ends.
    fn a_repo() -> TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The instant every line below is timed by: one clock, handed in, so the
    /// whole of this suite runs in whatever time it takes and nothing depends on
    /// how long that was.
    fn now() -> Instant {
        Instant::now()
    }

    /// The app the loop is holding when the prompt is up: a tree over `root`,
    /// a `/write` turn, and `reply` landed on it as an ordinary answer.
    ///
    /// The reply goes on the card and nowhere else, because the card is where
    /// the write reads it back from — which is the whole of the arrangement the
    /// module docs describe.
    fn app_answering(root: &Path, reply: &str) -> App {
        let mut app = App::from_tree(&Tree::new(Node::new(
            root,
            None::<PathBuf>,
            NodeState::Unpacted,
        )));
        app.start_turn("/write", now());
        app.answer_turn(reply, now());
        app.set_message(LAST_KEY);
        app
    }

    /// The field as the window would be by the time Enter is pressed: the
    /// heading it is drawn under, and the path typed into it.
    fn field(path: &str) -> ScopeField {
        ScopeField::new("Write to", path)
    }

    /// A granted entry for `module`, as a pacted directory has one.
    fn entry(module: &str) -> PactEntry {
        let document = if module == "." {
            "WARLOCK.md".to_owned()
        } else {
            format!("{module}/WARLOCK.md")
        };
        PactEntry::new(".", module, document)
            .expect("a relative module path is inside the root")
            .with_grant("d0f5a1", "2026-08-19T07:32:00Z")
    }

    /// The manifest the loop holds, pacting each of `modules`.
    fn pacts(modules: &[&str]) -> Manifest {
        Manifest::with_entries(modules.iter().map(|module| entry(module)))
    }

    /// Warlock's own lines on the conversation, in order — which is what a
    /// write says for itself, as against the turn it was asked in.
    fn notes(app: &App) -> Vec<String> {
        app.thread()
            .expect("the conversation is there")
            .lines(now())
            .into_iter()
            .filter_map(|line| match line {
                Line::Note { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Everything under `root`, spelled the way the manifest spells a path and
    /// sorted, so "nothing but the artifact was written" is one assertion about
    /// the whole repository.
    fn everything_under(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut directories = vec![root.to_path_buf()];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(&directory)
                .expect("a directory of this test's own reads back")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    directories.push(path.clone());
                }
                found.push(to_manifest_path(root, &path).expect("inside the repository"));
            }
        }
        found.sort();
        found
    }

    /// A document of exactly `bytes` bytes, title and trailing newline included,
    /// so a test can assert the size the line spells it with.
    fn document_of(bytes: usize) -> String {
        let opening = "# Scopes and sigils\n\n";
        let closing = "\n";
        let prose = bytes - opening.len() - closing.len();
        format!("{opening}{}{closing}", "x".repeat(prose))
    }

    #[test]
    fn enter_writes_the_document_and_says_what_landed_and_what_it_staled() {
        let repo = a_repo();
        // 1832 bytes on the nose, so the line's size is a fact rather than a
        // range: the panel's own spelling, one decimal under ten kilobytes.
        let reply = document_of(1832);
        let mut app = app_answering(repo.path(), &reply);

        let prompt = write_submit(
            &mut app,
            &pacts(&["docs"]),
            repo.path(),
            &field(BRIEF),
            now(),
        );

        assert_eq!(prompt, ScopePrompt::Closed, "the window is answered");
        assert_eq!(
            fs::read_to_string(repo.path().join(BRIEF)).expect("the artifact reads back"),
            reply,
            "the bytes on disk are not the reply on the card"
        );
        assert_eq!(
            notes(&app),
            [
                format!("wrote {BRIEF} — 1.8 KB"),
                "docs is now stale".to_owned(),
            ]
        );
        // The two lines are the whole of what a write says. The footer is the
        // last keystroke's, and nothing about the run state moved.
        assert_eq!(app.message(), Some(LAST_KEY));
        assert!(!app.is_pacting());
        assert_eq!(app.pact_line(), None);
    }

    #[test]
    fn the_size_is_the_panels_own_spelling_at_every_scale() {
        for (bytes, said) in [
            (934, "934 bytes"),
            (1832, "1.8 KB"),
            (14_540, "14 KB"),
            (412_672, "403 KB"),
        ] {
            let repo = a_repo();
            let mut app = app_answering(repo.path(), &document_of(bytes));

            write_submit(
                &mut app,
                &Manifest::new(),
                repo.path(),
                &field(BRIEF),
                now(),
            );

            assert_eq!(notes(&app), [format!("wrote {BRIEF} — {said}")]);
        }
    }

    #[test]
    fn the_document_is_written_verbatim_but_for_the_fence_and_the_last_newline() {
        for (reply, written) in [
            // Byte for byte, trailing newline and all.
            ("# Freshness\n\nProse.\n", "# Freshness\n\nProse.\n"),
            // The one newline ensured, and nothing else added.
            ("# Freshness\n\nProse.", "# Freshness\n\nProse.\n"),
            // Already ending in three: not normalised, because trimming would
            // be a third transformation nobody asked for.
            ("# Freshness\n\n\n\n", "# Freshness\n\n\n\n"),
            // A reply that is entirely one fence comes out of it.
            (
                "```markdown\n# Freshness\n\nProse.\n```\n",
                "# Freshness\n\nProse.\n",
            ),
            // A document that merely contains one is left exactly as it is,
            // indentation, blank lines, trailing spaces and all.
            (
                "# Freshness\n\n```rust\nlet x = 1;\n```\n\n  indented   \n",
                "# Freshness\n\n```rust\nlet x = 1;\n```\n\n  indented   \n",
            ),
        ] {
            let repo = a_repo();
            let mut app = app_answering(repo.path(), reply);

            write_submit(
                &mut app,
                &Manifest::new(),
                repo.path(),
                &field(BRIEF),
                now(),
            );

            assert_eq!(
                fs::read_to_string(repo.path().join(BRIEF)).expect("the artifact reads back"),
                written,
                "{reply:?} was interfered with"
            );
        }
    }

    #[test]
    fn nothing_but_the_artifact_is_written() {
        let repo = a_repo();
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

        write_submit(
            &mut app,
            &pacts(&["docs", "."]),
            repo.path(),
            &field(BRIEF),
            now(),
        );

        // The output directory and the file in it, and that is the whole of the
        // repository: no transcript, no draft, and nothing warlock authored
        // under `.warlock/` — the manifest was read and never written.
        assert_eq!(everything_under(repo.path()), ["docs", BRIEF]);
    }

    #[test]
    fn the_parent_directory_is_made_when_it_is_not_there() {
        let repo = a_repo();
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");
        let deep = "docs/briefs/2026/warlock-brief-01-freshness.md";

        let prompt = write_submit(&mut app, &Manifest::new(), repo.path(), &field(deep), now());

        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(repo.path().join(deep).is_file(), "{deep} is not a file");
    }

    #[test]
    fn a_path_that_already_exists_writes_nothing_and_reopens_the_field() {
        let repo = a_repo();
        fs::create_dir_all(repo.path().join("docs")).expect("makes the output directory");
        fs::write(repo.path().join(BRIEF), "what was already there\n").expect("writes it first");
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");
        let before = app.clone();
        let typed = field(BRIEF);

        let prompt = write_submit(&mut app, &pacts(&["docs"]), repo.path(), &typed, now());

        // The rule under the field and the typed path still in it, one keystroke
        // from being changed — and the file that was there is byte for byte the
        // file that is there.
        let still_up = prompt.field().expect("the window came down");
        assert_eq!(still_up.text(), BRIEF);
        assert!(
            still_up
                .rule()
                .is_some_and(|rule| rule.contains(BRIEF) && rule.contains("already exists")),
            "the refusal said {:?}",
            still_up.rule()
        );
        assert_eq!(
            fs::read_to_string(repo.path().join(BRIEF)).expect("the file reads back"),
            "what was already there\n",
            "the write went over a file that was already there"
        );
        assert_eq!(app, before, "a refusal moved the view");
        assert!(notes(&app).is_empty(), "a refusal said something happened");
    }

    #[test]
    fn a_directory_in_the_way_is_a_path_that_already_exists() {
        // Not a file, and still nothing warlock may write over. `exists` is
        // asked about the path rather than about its kind.
        let repo = a_repo();
        fs::create_dir_all(repo.path().join(BRIEF)).expect("makes a directory of that name");
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

        let prompt = write_submit(
            &mut app,
            &Manifest::new(),
            repo.path(),
            &field(BRIEF),
            now(),
        );

        assert!(prompt.is_open(), "a directory was written over");
        assert!(notes(&app).is_empty());
    }

    #[test]
    fn the_stale_line_names_the_nearest_pacted_ancestor_and_only_it() {
        for (modules, said) in [
            // The directory the file landed in, when it is the one with a pact.
            (&["docs", "."][..], Some("docs is now stale")),
            // The root, when nothing nearer is pacted: an outer pact still
            // describes a subtree the new file is in.
            (&["."][..], Some(". is now stale")),
            // A pact beside it is not a pact above it, however much of a prefix
            // it looks like.
            (&["docs-old", "crates"][..], None),
            // And a repository with no pacts at all has no ledger to stale.
            (&[][..], None),
        ] {
            let repo = a_repo();
            let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

            write_submit(&mut app, &pacts(modules), repo.path(), &field(BRIEF), now());

            let lines = notes(&app);
            assert!(
                lines[0].starts_with(&format!("wrote {BRIEF} — ")),
                "{modules:?} said {lines:?}"
            );
            assert_eq!(
                lines.get(1).map(String::as_str),
                said,
                "{modules:?} said {lines:?}"
            );
        }
    }

    #[test]
    fn a_file_written_beside_the_root_is_staled_by_the_root_pact() {
        // No `/` in the path at all, which is the one case the ancestor walk
        // has to reach the root from directly.
        let repo = a_repo();
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

        write_submit(
            &mut app,
            &pacts(&["."]),
            repo.path(),
            &field("brief.md"),
            now(),
        );

        assert_eq!(
            notes(&app).get(1).map(String::as_str),
            Some(". is now stale")
        );
    }

    #[test]
    fn an_empty_field_writes_nothing_and_stays_up() {
        let repo = a_repo();
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

        for typed in ["", "   "] {
            let prompt = write_submit(
                &mut app,
                &Manifest::new(),
                repo.path(),
                &field(typed),
                now(),
            );

            assert_eq!(
                prompt.field().and_then(ScopeField::rule),
                Some(NO_PATH),
                "{typed:?} was answered as a path"
            );
            assert_eq!(everything_under(repo.path()), Vec::<String>::new());
        }
    }

    #[test]
    fn a_path_that_climbs_out_of_the_repository_is_refused() {
        let repo = a_repo();
        let outside = repo.path().join("outside");
        let root = repo.path().join("repo");
        fs::create_dir_all(&outside).expect("makes a directory beside the repository");
        fs::create_dir_all(&root).expect("makes the repository");
        let mut app = app_answering(&root, "# Freshness\n\nProse.\n");

        let prompt = write_submit(
            &mut app,
            &Manifest::new(),
            &root,
            &field("../outside/brief.md"),
            now(),
        );

        // The artifact belongs to the repository the ledger is about, so the
        // field stays up over the engine's own sentence and nothing lands
        // anywhere.
        assert!(
            prompt
                .field()
                .and_then(ScopeField::rule)
                .is_some_and(|rule| !rule.is_empty()),
            "a path outside the repository was allowed through"
        );
        assert_eq!(everything_under(&outside), Vec::<String>::new());
        assert_eq!(everything_under(&root), Vec::<String>::new());
    }

    #[test]
    fn a_write_that_will_not_happen_puts_its_reason_on_the_message_line() {
        // A file where the output directory has to be: the parent cannot be
        // made, which is the cheapest real version of a disk that will not take
        // the write.
        let repo = a_repo();
        fs::write(repo.path().join("docs"), "not a directory\n")
            .expect("writes a file in the way of the output directory");
        let mut app = app_answering(repo.path(), "# Freshness\n\nProse.\n");

        let prompt = write_submit(&mut app, &pacts(&["."]), repo.path(), &field(BRIEF), now());

        // A line on the footer and the window down off it — never an error out
        // of the event loop, and never a line on the conversation claiming a
        // file that is not there.
        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(
            app.message()
                .is_some_and(|line| line.contains(BRIEF) && line != LAST_KEY),
            "a write that failed said {:?}",
            app.message()
        );
        assert!(notes(&app).is_empty(), "it said something happened anyway");
        assert_eq!(
            fs::read_to_string(repo.path().join("docs")).expect("the file in the way reads back"),
            "not a directory\n"
        );
    }

    #[test]
    fn a_conversation_with_no_answer_on_it_writes_nothing_and_says_so() {
        // The road the loop cannot take — the prompt opens over an answer that
        // has landed — answered rather than left to be discovered.
        let repo = a_repo();
        let mut app = App::from_tree(&Tree::new(Node::new(
            repo.path(),
            None::<PathBuf>,
            NodeState::Unpacted,
        )));

        let prompt = write_submit(
            &mut app,
            &Manifest::new(),
            repo.path(),
            &field(BRIEF),
            now(),
        );

        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(app.message().is_some_and(|line| !line.is_empty()));
        assert_eq!(everything_under(repo.path()), Vec::<String>::new());
    }

    /// The event loop's own round over this window, one key at a time.
    ///
    /// `scoping.rs`'s counterpart for the other prompt, and driven exactly as it
    /// drives that one: the window is opened the way the loop opens it, every
    /// key goes through [`edit_for`] as `press_for` would send it, and what
    /// comes back goes through [`write_edit`], which is the loop's arm. Nothing
    /// here is a terminal, a `claude` or a worker thread — what is asserted is
    /// the path from a turn's answer to the bytes on disk, over a repository of
    /// the test's own.
    mod rounds {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use warlock_engine::DEFAULT_BRIEF_DIRECTORY;
        use warlock_tui::{Mode, ScopePrompt, edit_for};

        use super::super::{WRITE_HEADING, write_edit, write_opened};
        use super::{
            App, Instant, Manifest, TempDir, a_repo, app_answering, everything_under, fs, notes,
            now, pacts,
        };

        /// The reply the `/write` turn answered with in every round below: one
        /// document, with a title the slug is plainly made of.
        const REPLY: &str = "# Scopes and sigils\n\nA boundary somebody drew.\n";

        /// Where the loop is holding briefs when it opens the window below: the
        /// engine's default, which is what a session that has entered brief mode
        /// and read nothing else is carrying. The value is handed down, never
        /// looked up here — which is what these rounds are driving.
        const DIRECTORY: &str = DEFAULT_BRIEF_DIRECTORY;

        /// Where that reply proposes to go in an empty repository: nothing is in
        /// `docs/` yet, so the number is the first one.
        const PROPOSED: &str = "docs/warlock-brief-01-scopes-and-sigils.md";

        /// A plain press of `code`, as crossterm reports one.
        fn press(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        /// One round of the loop with `code` pressed into `prompt`: the key
        /// answered by [`edit_for`], and the answer applied by [`write_edit`].
        ///
        /// The loop's two lines and nothing else, so a test below cannot swallow
        /// a key through a kinder version of the loop written beside it.
        fn round(
            app: &mut App,
            manifest: &Manifest,
            repo: &TempDir,
            prompt: &ScopePrompt,
            code: KeyCode,
            now: Instant,
        ) -> ScopePrompt {
            let edited = {
                let field = prompt.field().expect("the window is still up");
                edit_for(press(code), field)
            };
            write_edit(app, manifest, repo.path(), prompt, edited, now)
        }

        #[test]
        fn the_window_opens_over_the_answer_holding_the_path_it_proposes() {
            // What the drain hands the loop, turned into the window the reader
            // sees: the heading it is drawn under, and the proposal in the
            // field, ready for an Enter that changes nothing about it.
            let repo = a_repo();

            assert_eq!(
                write_opened(repo.path(), DIRECTORY, REPLY),
                ScopePrompt::open(WRITE_HEADING, PROPOSED)
            );
            assert_eq!(
                everything_under(repo.path()),
                Vec::<String>::new(),
                "opening a window wrote something"
            );
        }

        #[test]
        fn the_whole_path_from_the_answer_to_the_file_is_one_key_at_a_time() {
            // The loop's arms in the order a reader presses them: the window
            // opens on the proposal, three Backspaces and four characters make
            // it something else, and Enter writes the document at the path on
            // screen rather than at the one warlock guessed.
            let repo = a_repo();
            let mut app = app_answering(repo.path(), REPLY);
            let manifest = pacts(&["docs"]);
            let mut prompt = write_opened(repo.path(), DIRECTORY, REPLY);

            for code in [
                KeyCode::Backspace,
                KeyCode::Backspace,
                KeyCode::Backspace,
                KeyCode::Char('.'),
                KeyCode::Char('t'),
                KeyCode::Char('x'),
                KeyCode::Char('t'),
                KeyCode::Enter,
            ] {
                prompt = round(&mut app, &manifest, &repo, &prompt, code, now());
            }

            let written = "docs/warlock-brief-01-scopes-and-sigils.txt";
            assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
            assert_eq!(
                fs::read_to_string(repo.path().join(written)).expect("the artifact reads back"),
                REPLY,
                "the bytes on disk are not the reply on the card"
            );
            assert_eq!(
                notes(&app),
                vec![
                    format!("wrote {written} — 47 bytes"),
                    "docs is now stale".to_owned(),
                ]
            );
        }

        #[test]
        fn esc_writes_nothing_and_leaves_the_reply_and_the_register_alone() {
            // The abandonment, and the whole of what it is allowed to touch:
            // the window. The reply is still on the card, the mode is still
            // brief, the footer still says what the last key said, and the
            // repository is as empty as it was.
            let repo = a_repo();
            let mut app = app_answering(repo.path(), REPLY);
            app.set_mode(Mode::Brief);
            let before = app.clone();
            let prompt = write_opened(repo.path(), DIRECTORY, REPLY);

            let prompt = round(
                &mut app,
                &pacts(&["docs"]),
                &repo,
                &prompt,
                KeyCode::Esc,
                now(),
            );

            assert_eq!(prompt, ScopePrompt::Closed);
            assert_eq!(app, before, "Esc moved something on the app");
            assert_eq!(app.mode(), Mode::Brief);
            assert_eq!(everything_under(repo.path()), Vec::<String>::new());
        }

        #[test]
        fn a_refused_path_reopens_the_field_and_the_next_enter_writes() {
            // A path that is taken, typed out of the way and written: the
            // refusal keeps the window up over the very text that earned it, so
            // the fix is a keystroke rather than a second `/write`.
            //
            // The file in the way appears after the window opened, which is the
            // situation the check is for: the proposal is a guess made once, at
            // the answer, and what is on disk when Enter is pressed is a
            // different question with a different answer.
            let repo = a_repo();
            let mut app = app_answering(repo.path(), REPLY);
            let manifest = pacts(&["docs"]);
            let mut prompt = write_opened(repo.path(), DIRECTORY, REPLY);
            fs::create_dir_all(repo.path().join("docs")).expect("makes the output directory");
            fs::write(repo.path().join(PROPOSED), "somebody else's brief\n")
                .expect("writes the file in the way");

            prompt = round(&mut app, &manifest, &repo, &prompt, KeyCode::Enter, now());

            let field = prompt
                .field()
                .expect("a refusal left the window up")
                .clone();
            assert_eq!(field.text(), PROPOSED, "the typed path was taken away");
            assert!(field.rule().is_some(), "the window says nothing about why");
            assert_eq!(
                fs::read_to_string(repo.path().join(PROPOSED)).expect("the file is still there"),
                "somebody else's brief\n",
                "the refused Enter wrote over the file in the way"
            );

            for code in [
                KeyCode::Backspace,
                KeyCode::Backspace,
                KeyCode::Backspace,
                KeyCode::Char('-'),
                KeyCode::Char('2'),
                KeyCode::Char('.'),
                KeyCode::Char('m'),
                KeyCode::Char('d'),
                KeyCode::Enter,
            ] {
                prompt = round(&mut app, &manifest, &repo, &prompt, code, now());
            }

            assert_eq!(prompt, ScopePrompt::Closed);
            assert_eq!(
                fs::read_to_string(
                    repo.path()
                        .join("docs/warlock-brief-01-scopes-and-sigils-2.md")
                )
                .expect("the second path reads back"),
                REPLY
            );
        }

        #[test]
        fn a_submit_from_a_window_that_is_not_up_writes_nothing() {
            // The road `press_for` cannot take — it consults `edit_for` only
            // while a window is up — answered rather than left to be
            // discovered.
            let repo = a_repo();
            let mut app = app_answering(repo.path(), REPLY);

            let prompt = write_edit(
                &mut app,
                &pacts(&["docs"]),
                repo.path(),
                &ScopePrompt::Closed,
                warlock_tui::Edited::Submit,
                now(),
            );

            assert_eq!(prompt, ScopePrompt::Closed);
            assert_eq!(everything_under(repo.path()), Vec::<String>::new());
        }
    }

    /// The whole of `/write`, from the word typed at the composer to the bytes
    /// on disk, in one test.
    ///
    /// Every other test in this file is about one joint of it. This one is about
    /// the joints being joined: the composer's own `apply_compose` starts the
    /// turn, the turn is really run — on a worker thread, through the very agent
    /// the loop holds — the loop's own drain hands the answer back, the prompt
    /// opens over it through `write_opened`, and Enter goes through `edit_for`
    /// into `write_edit` exactly as the event loop's arm does. Nothing in the
    /// middle is stood in for.
    ///
    /// What *is* stood in for is the model, and it is `/bin/sh` printing one
    /// result line — `chatting.rs`'s arrangement, for its reason, which is why
    /// this module is Unix-only. So the whole path runs with no terminal, no
    /// network and no `claude`, over a repository of the test's own that goes
    /// away with the test.
    #[cfg(unix)]
    mod whole {
        use std::path::Path;
        use std::thread;
        use std::time::Duration;

        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use warlock_engine::DEFAULT_BRIEF_DIRECTORY;
        use warlock_tui::{ChatAgent, Composed, Composer, Line, Mode, ScopePrompt, edit_for};

        use super::super::{WRITE_HEADING, write_edit, write_opened};
        use super::{
            App, Instant, Node, NodeState, PathBuf, Tree, a_repo, everything_under, fs, notes, now,
            pacts,
        };
        use crate::apply_compose;
        use crate::chatting::Chat;

        /// How long the rounds below go on before giving up on a turn that is
        /// never going to end. `chatting.rs`'s number and its reason: it is only
        /// ever reached when something is already wrong, and every wait ends the
        /// moment the turn does.
        const AT_MOST: Duration = Duration::from_secs(5);

        /// What the stand-in answers with: one document, with the title the slug
        /// is plainly made of and the trailing newline a document has.
        const DOCUMENT: &str =
            "# Scopes and sigils\n\nA boundary somebody drew, and the reason it is there.\n";

        /// Where that document proposes to go in a repository that has never had
        /// a brief written into it.
        const PROPOSED: &str = "docs/warlock-brief-01-scopes-and-sigils.md";

        /// A `claude` that is a shell script: it answers with `document` on the
        /// one line the seam takes an answer off, and exits.
        ///
        /// The result line is the only thing this stand-in says, so the turn has
        /// no work lines and lands as one answer — which is all this test is
        /// about. What a turn's *work* looks like on the card is `chatting.rs`'s
        /// own suite.
        fn answering_with(document: &str) -> ChatAgent {
            let result = format!(
                r#"{{"type":"result","subtype":"success","result":"{}"}}"#,
                escaped(document)
            );
            ChatAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c".to_owned(), format!("printf '%s\\n' '{result}'")])
        }

        /// `document` as a JSON string spells it, so the newlines in it survive
        /// the one line the stand-in prints.
        ///
        /// `printf '%s\n'` does not interpret its argument, so what the shell is
        /// handed is what `serde_json` reads — and what it reads back out is the
        /// document byte for byte.
        fn escaped(document: &str) -> String {
            document
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        }

        /// The loop's bottom end, round after round, until the turn has ended.
        ///
        /// [`Chat::keep_up`] and the one line above it in the event loop's own
        /// `keep_up`, and nothing else: a `/write` answer comes back from the
        /// drain and becomes the window, and every other round comes back with
        /// nothing and leaves the window closed. Nothing here joins a thread,
        /// receives from a channel or waits on a child.
        ///
        /// `directory` comes in the way the loop's own bottom end has it: a
        /// value settled turns ago, carried down to the one line that proposes a
        /// path, so no round of this loop reads a file or can fail.
        fn rounds_until_answered(
            chat: &mut Chat,
            app: &mut App,
            repo_root: &Path,
            directory: &str,
            now: Instant,
        ) -> ScopePrompt {
            let mut prompt = ScopePrompt::default();
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < AT_MOST {
                if let Some(document) = chat.keep_up(app, now) {
                    prompt = write_opened(repo_root, directory, &document);
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");
            prompt
        }

        #[test]
        fn a_write_typed_at_the_composer_ends_as_a_document_on_disk() {
            let repo = a_repo();
            let base = now();
            let mut app = App::from_tree(&Tree::new(Node::new(
                repo.path(),
                None::<PathBuf>,
                NodeState::Unpacted,
            )));
            // The register the command is only allowed in, and the pact the
            // written file is about to make stale.
            app.set_mode(Mode::Brief);
            let manifest = pacts(&["docs"]);
            let mut chat = Chat::with_agent(answering_with(DOCUMENT));
            let mut composer = Composer::new("/write");
            // The loop's own local, as a session that entered brief mode is
            // holding it: the default, settled at `/brief` and carried from here
            // to the window without being looked at again.
            let mut brief_directory = DEFAULT_BRIEF_DIRECTORY.to_owned();

            // The reader's Enter at the foot of the panel, through the very
            // function the loop's composer arm calls.
            apply_compose(
                &mut app,
                &mut composer,
                Composed::Submit,
                &mut chat,
                repo.path(),
                &mut brief_directory,
                base,
            );

            assert!(chat.answering(), "the command started no turn");
            assert_eq!(composer.draft(), "", "the field kept the submitted word");

            // Then the rounds, until the answer lands and the window opens over
            // it — pre-filled, headed, and complaining about nothing.
            let prompt =
                rounds_until_answered(&mut chat, &mut app, repo.path(), &brief_directory, base);

            let field = prompt
                .field()
                .expect("the path prompt opened over the answer")
                .clone();
            assert_eq!(field.text(), PROPOSED);
            assert_eq!(field.directory(), WRITE_HEADING);
            assert_eq!(field.rule(), None);
            assert_eq!(
                everything_under(repo.path()),
                Vec::<String>::new(),
                "a proposal wrote something"
            );
            // The card shows the word that was typed and the document that came
            // back — never the paragraph warlock sent.
            let rows = app.thread().expect("the conversation is there").lines(base);
            assert_eq!(
                rows.first(),
                Some(&Line::Said {
                    text: "/write".to_owned()
                })
            );
            assert!(
                rows.iter().any(|line| *line
                    == Line::Text {
                        text: "# Scopes and sigils".to_owned()
                    }),
                "the answer is not on the card: {rows:?}"
            );

            // And Enter in that window, through `edit_for` as `press_for` sends
            // it and `write_edit` as the loop's arm applies it.
            let edited = edit_for(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &field);
            let prompt = write_edit(&mut app, &manifest, repo.path(), &prompt, edited, base);

            assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
            let written =
                fs::read_to_string(repo.path().join(PROPOSED)).expect("the artifact reads back");
            assert_eq!(written, DOCUMENT, "the bytes are not the document answered");
            assert_eq!(
                notes(&app),
                [
                    format!("wrote {PROPOSED} — {} bytes", written.len()),
                    "docs is now stale".to_owned(),
                ]
            );
            // The whole repository, after the whole path: the output directory
            // and the one file in it. No transcript, no draft of the brief, and
            // nothing warlock authored under `.warlock/` — the turn wrote
            // nothing at all, and the write wrote the artifact.
            assert_eq!(everything_under(repo.path()), ["docs", PROPOSED]);
        }
    }
}
