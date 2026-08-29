//! `CLAUDE.md`: what warlock leaves at a repository root for the agent that
//! reads it.
//!
//! A repository that uses warlock has `WARLOCK.md` files committed beside its
//! code and a three-colour freshness model, and an agent that wanders in knows
//! neither. So the fastest route into the codebase — read the documents, then
//! the source — goes unused, and a stale document gets quoted as if it were
//! current. This module writes the one file that fixes that: a short account of
//! what warlock is, what a `WARLOCK.md` is, what the colours mean, and the
//! instruction to read the documents first.
//!
//! **Warlock owns a block in that file, not the file.** A `CLAUDE.md` is where
//! a repository keeps its own standing instructions, and warlock is not the
//! only thing with something to say there, so everything here is bracketed by
//! [`BEGIN`] and [`END`] and [`splice`] is the only thing that touches it: an
//! absent block is added, a present block is replaced where it stands, and
//! every byte outside the two markers survives byte for byte. That
//! is also what makes a second run a no-op — the replacement is the same
//! bytes as the block already there — so `warlock init` is safe to run again
//! without a reader having to remember whether they ran it before.
//!
//! The write is the crate's one write idiom, [`temp_file_name`] +
//! [`write_and_sync`] + [`fs::rename`], for the reason `Manifest::save` and
//! `write_document` use it: the file being replaced is somebody's own, so it
//! must be the whole old file or the whole new one and never a prefix of
//! either, and no temporary may be left behind on either way out.
//!
//! Nothing else happens here. No `.warlock/`, no manifest, no
//! `.warlockignore`, no `WARLOCK.md`, and no model pass: this is scaffolding a
//! reader can run in a repository that has never been pacted, and it must not
//! quietly enrol them in anything.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::{temp_file_name, write_and_sync};

/// The file warlock writes its orientation into, at the repository root.
///
/// The name is not a warlock invention, which is the whole reason this module
/// splices rather than writes: the file may well already be there with
/// somebody else's words in it.
///
/// `CLAUDE.md` and not `AGENTS.md`, which this was, for one mechanical reason:
/// `CLAUDE.md` is loaded at the start of every session, and `AGENTS.md` is not
/// read at all. Orientation nothing loads is not orientation, and warlock is
/// not in a position to argue the point — a repository that wants the
/// cross-tool file too can keep one and reference this from it. That makes the
/// name a fact about what reads the file rather than a preference, and it is
/// the thing to check first if it ever needs changing again.
const FILE: &str = "CLAUDE.md";

/// The opening marker of warlock's block.
///
/// An HTML comment because that is the one thing every Markdown renderer
/// agrees to swallow: the marker is invisible to a human reading the rendered
/// file and plain to anything reading the source, which is what a delimiter
/// somebody else's file has to carry has to be.
const BEGIN: &str = "<!-- warlock:begin -->";

/// The closing marker of warlock's block. See [`BEGIN`].
const END: &str = "<!-- warlock:end -->";

/// Everything warlock has to say about itself to somebody else's agent, and
/// the only thing this crate ever writes into a `CLAUDE.md`.
///
/// Text is code here, exactly as the prompts in [`pact`](crate::pact) are code:
/// no template file, no configuration key, no per-project override. Changing
/// what warlock tells a reader's agents is a change to this string, reviewed in
/// a diff like everything else.
///
/// # Why there is no guidance here on *writing* a `WARLOCK.md`
///
/// The obvious next section — how to author a document, what a good one
/// contains — is deliberately absent, and the reason is mechanical rather than
/// stylistic. A pacting pass runs headless with no tools, its own system prompt
/// in place of the CLI's, and its whole request on stdin: it cannot read this
/// file, and is never asked to. Guidance here would therefore be a second
/// prompt that no pass ever obeys, free to drift away from the real one in
/// [`pact`](crate::pact) with nothing to catch the drift. The prompts stay
/// code, and this file stays orientation for the reader's own agents.
///
/// # What it has to cover
///
/// What warlock is; that a `WARLOCK.md` is a per-directory document written by
/// a model and committed beside the code; that the three colours are unpacted,
/// stale and fresh, with stale mechanical and fresh only ever granted; that the
/// subtree hash is the trigger and not a judgement; and the load-bearing
/// instruction, which is to read the documents first and to treat a stale one
/// as possibly behind the code it describes.
///
/// Then what editing one of those documents costs, because an agent that fixes
/// a `WARLOCK.md` in passing will otherwise be surprised by the consequence: a
/// document is an ordinary file in the directory it describes, so it is inside
/// that directory's [`subtree_hash`](crate::subtree_hash), and saving an edit
/// restales the directory there and then. The road back is a pass and nothing
/// else — `r` in warlock's tree — because fresh is only ever granted, and an
/// agent that knows this can say the directory is owed a pass instead of
/// treating the yellow row it just caused as a fault.
///
/// Then scopes and sigils, the vocabulary of who a piece of work belongs to: a
/// pacted directory may carry one scope, recorded in `.warlock/pacts.toml`
/// beside the grant and shown in the tree; a scope covers everything beneath it
/// until a nearer one overrides it, so an outer scope is a default rather than
/// a second gate, and a directory with no scope over it is open to anyone; the
/// operator may hold several sigils and any one of them opens a matching scope,
/// which makes access a membership test rather than an expression to evaluate;
/// `warlock config` prints what is held, and holding nothing means
/// unrestricted; a scope is a term of the pact, so an unpacted directory cannot
/// carry one and un-pacting removes one. Its own load-bearing instruction is to
/// look at the scopes of what a change would touch before making it and to say
/// plainly when the work crosses a scope the operator does not hold — narrowing
/// it or naming the sigil it wants, never refusing, blocking or editing around
/// it.
///
/// Nothing here claims that warlock enforces any of that, or that the agent
/// should: nothing in this workspace matches a sigil against a scope, and a
/// file that promises a wall warlock does not build is worse than no file,
/// because the first person to sail through it learns the document lies.
///
/// It opens at heading level two: warlock owns a section of a file whose title,
/// if it has one, belongs to whoever created it.
///
/// It is hard wrapped, unlike the prompts, and the source lines below are the
/// lines that land on disk. A prompt is one stream to a model and wraps
/// wherever the literal is convenient; this is a file somebody commits, reviews
/// and diffs, so a changed sentence should be a changed line rather than a
/// changed paragraph.
const BODY: &str = "\
## Warlock

Warlock is a freshness ledger for a codebase's own documentation. Every
directory under its management holds a `WARLOCK.md`: a document about that
one directory — its files, and what the directories below it are for —
written by a model pass and committed beside the code, like any other source
file.

**Read the `WARLOCK.md` files first.** They are the fast way into this
repository: read the ones covering the area you are about to work in before
you start opening source files. A parent's document is written from its
children's documents, so reading downwards from the top gives you the shape
of the project in a few files instead of a few dozen.

One caveat, and it is what the colours below exist for: **a document can be
behind the code it describes.** Where a document and the code disagree, the
code is right. Say so in your work, because that gap is the thing warlock
exists to make visible.

## What the colours mean

Warlock draws the project as a tree, one row per directory, each in one of
three colours:

- **Unpacted** — outside warlock's management. No document, no record, and
  nothing claimed about it either way.
- **Stale** — pacted, and something at or below it has changed since its
  document was last granted. Stale is *mechanical*: the hash moved, so it is
  stale, immediately, with no opinion involved.
- **Fresh** — pacted, and granted after a model pass read the directory.
  Fresh is only ever *granted*, never assumed, and a directory nobody has
  judged yet is stale rather than fresh.

There is deliberately no fourth colour.

**The hash is the trigger, not the judgement.** Warlock digests every byte at
and below a directory and records that digest at the moment a document is
granted. When the digest stops matching, something happened down there and
the document is owed a look. It never decides that a document is *wrong* —
whether a particular change warrants a documentation update was always a
judgement call, and warlock makes the change visible rather than pretending
to make that call for you.

**Editing a `WARLOCK.md` by hand makes its own directory stale.** The
document sits in the directory it describes, so its own bytes are part of
that directory's digest: the moment an edit is saved the hash stops
matching the one recorded when the document was granted, and the directory
is stale again. That is the ledger being honest rather than something going
wrong. The only road back to fresh is another model pass over that
directory — `r` in warlock's tree — because fresh is only ever granted, and
nothing records a hash without a pass having read the directory first. So
correct a document where it is wrong, and say that the directory it
describes is now owed a pass.

## Scopes and sigils

A pacted directory may carry one **scope**: a short label — `data-plane`,
`billing`, `web` — saying whose work that part of the repository is. Scopes
live in `.warlock/pacts.toml` beside the grant, and are shown in the tree
next to the directory they are written on.

A scope covers everything beneath it until a nearer one overrides it. The
nearest scope at or above a directory is the one that applies, on its own: an
outer scope is a *default* for everything below it that has said nothing, not
a second gate to also satisfy. A pacted directory with no scope above it is
open to anyone.

The other half of the vocabulary is the **sigil**, which is what the operator
holds. They may hold several, and any one of them opens a matching scope — so
whether a scope is open to them is a membership test, not an expression to
evaluate. `warlock config` prints what is held. Holding nothing means
unrestricted, not shut out.

**A scope is a term of the pact, not a thing beside it.** An unpacted
directory cannot carry one, and un-pacting a directory takes its scope away
with the rest of the pact. So a boundary is changed by changing a pact, and
there is nowhere else to look for one.

**Before planning or making a change, look at what the directories it would
touch are scoped to, and at what the operator holds.** When the work crosses
a scope they do not hold, say so plainly, and then either narrow the work to
what they hold or name the sigil the work wants. Never refuse the work, never
block it, and never quietly edit around the boundary: saying what is being
crossed is the whole of the job here.";

/// Warlock's block, markers and all, as it is written into a `CLAUDE.md`.
///
/// One place assembles it so that what is appended to a file without a block
/// and what replaces the block in a file that has one cannot differ — that
/// equality is exactly why running twice changes nothing.
///
/// A blank line inside each marker, because a Markdown heading on the line
/// straight after an HTML comment is not a heading to every renderer.
fn section() -> String {
    format!("{BEGIN}\n\n{BODY}\n\n{END}")
}

/// `existing`, with warlock's block added or brought up to date.
///
/// Pure, and the whole of the file-editing policy. Two cases:
///
/// * **No block.** The block is appended, after a blank line, with the file's
///   own text left exactly as it was in front of it. Empty text becomes the
///   block alone.
/// * **A block.** Everything from [`BEGIN`] to [`END`] inclusive is replaced,
///   in place. Not appended a second time: the point of the markers is that a
///   file gains warlock's section once, however many times `warlock init` is
///   run.
///
/// Every byte outside the markers is carried through untouched — leading text,
/// trailing text, whitespace, line endings and all. This is somebody else's
/// file and warlock is a guest in it.
///
/// Splicing twice is splicing once: the second call finds the block the first
/// one wrote and replaces it with [`section`] again, which is the same bytes,
/// and touches nothing else. Tests pin that from both starting points.
///
/// A [`BEGIN`] with no [`END`] after it is treated as a block that runs to the
/// end of the file, which is what a save interrupted half way through leaves.
/// Appending in that case would put a second [`BEGIN`] in the file, and the run
/// after *that* would take the first marker with the second's [`END`] and
/// swallow whatever the reader had written in between — a worse failure than
/// re-ending a section warlock itself wrote.
fn splice(existing: &str) -> String {
    let section = section();

    if let Some(start) = existing.find(BEGIN) {
        let rest = &existing[start + BEGIN.len()..];
        let finish = rest
            .find(END)
            .map_or(existing.len(), |at| start + BEGIN.len() + at + END.len());
        let mut spliced = String::with_capacity(existing.len() + section.len());
        spliced.push_str(&existing[..start]);
        spliced.push_str(&section);
        spliced.push_str(&existing[finish..]);
        return spliced;
    }

    let mut spliced = String::with_capacity(existing.len() + section.len() + 2);
    spliced.push_str(existing);
    if !spliced.is_empty() {
        // A file that does not end in a newline is a file whose last line would
        // otherwise have the marker glued onto it.
        if !spliced.ends_with('\n') {
            spliced.push('\n');
        }
        spliced.push('\n');
    }
    spliced.push_str(&section);
    spliced.push('\n');
    spliced
}

/// Write `<root>/CLAUDE.md`, creating it or bringing warlock's section in it up
/// to date.
///
/// `root` is the repository root — [`repository_root`](crate::repository_root)
/// is what finds it, and it is passed in rather than resolved here so that this
/// function writes where it is told and nowhere else, and so its tests need no
/// `.git/`.
///
/// Exactly one file is written. No `.warlock/`, no manifest, no
/// `.warlockignore`, no `WARLOCK.md`, and no model pass: a reader running this
/// in a repository that has never been pacted gets a `CLAUDE.md` and no
/// enrolment in anything.
///
/// What happens to a file that is already there is [`splice`]'s policy in full:
/// warlock's block is replaced where it stands and every other byte survives,
/// so a second run leaves the file byte-identical to the first.
///
/// The write goes through a temporary in the same directory and a rename over
/// the target, so a reader's own `CLAUDE.md` is never seen half-replaced and no
/// temporary is left behind on either the success or the failure path.
///
/// ```
/// use warlock_engine::{Written, write_claude_md};
///
/// let root = tempfile::tempdir()?;
/// let first = write_claude_md(root.path())?;
/// assert!(matches!(first, Written::Created { .. }));
/// assert_eq!(first.path(), root.path().join("CLAUDE.md"));
///
/// let text = std::fs::read_to_string(first.path())?;
/// assert!(text.contains("WARLOCK.md"));
///
/// // Again is a no-op on the bytes, and says so.
/// let second = write_claude_md(root.path())?;
/// assert!(matches!(second, Written::Updated { .. }));
/// assert_eq!(std::fs::read_to_string(second.path())?, text);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// * [`Error::Read`] if a `CLAUDE.md` is there but cannot be read. A missing
///   file is not an error; it is the create case.
/// * [`Error::NotText`] if it is there and is not UTF-8. Nothing is written:
///   bytes this crate cannot reason about are not bytes to rewrite blind.
/// * [`Error::Write`] if the temporary cannot be written or the rename fails,
///   naming `<root>/CLAUDE.md` rather than the temporary.
pub fn write_claude_md(root: impl AsRef<Path>) -> Result<Written, Error> {
    let root = root.as_ref();
    let target = root.join(FILE);

    let existing = match fs::read(&target) {
        Ok(bytes) => Some(bytes),
        // The ordinary first run, and the one case that is not a failure.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::Read {
                path: target,
                source,
            });
        }
    };
    let created = existing.is_none();

    let existing = match existing {
        Some(bytes) => Some(String::from_utf8(bytes).map_err(|_| Error::NotText {
            path: target.clone(),
        })?),
        None => None,
    };
    let text = splice(existing.as_deref().unwrap_or_default());

    // The same idiom, through the same two helpers, as `Manifest::save` and
    // `write_document`: written beside and renamed over. The temporary is a dot
    // file, so the moment it exists it is invisible to every walk in this crate
    // — no tree, no subtree hash, no request — and it is removed on both ways
    // out.
    let temp = root.join(temp_file_name(FILE));
    let write = write_and_sync(&temp, text.as_bytes()).and_then(|()| fs::rename(&temp, &target));
    if let Err(source) = write {
        // Best effort, and nothing to report if it fails: the caller is already
        // being told the file was not written.
        drop(fs::remove_file(&temp));
        return Err(Error::Write {
            // The target, not the temporary: the caller asked for `CLAUDE.md`
            // and how it gets written is this function's business.
            path: target,
            source,
        });
    }

    Ok(if created {
        Written::Created { path: target }
    } else {
        Written::Updated { path: target }
    })
}

/// What a write of `CLAUDE.md` did, and to which file.
///
/// The distinction is worth a type because it is the whole of what a front end
/// has to tell a reader afterwards: a file appeared, or a file they already had
/// gained warlock's section. Both carry the path, since the caller passed a
/// root and should not have to know the file name to name what happened.
///
/// No [`Display`](fmt::Display) impl on purpose: the wording of "created" or
/// "updated" is the front end's, and this is the fact underneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Written {
    /// There was no `CLAUDE.md`, and now there is one.
    Created {
        /// The file that was written.
        path: PathBuf,
    },
    /// There was one already, and warlock's section in it is now current.
    /// Everything outside warlock's markers is exactly as it was — which
    /// includes the case where the section was already current and the file is
    /// byte-identical.
    Updated {
        /// The file that was written.
        path: PathBuf,
    },
}

impl Written {
    /// The `CLAUDE.md` that was written, either way.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Created { path } | Self::Updated { path } => path,
        }
    }
}

/// Everything that can stop a `CLAUDE.md` being written, each one naming the
/// file.
///
/// Hand-rolled like the rest of this crate's errors, and every variant prints
/// as one line including its cause, because the front end that shows these has
/// one line to show them in.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A `CLAUDE.md` is there but could not be read. A missing file is not in
    /// here: that is the create case, not a failure.
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// A `CLAUDE.md` is there and is not UTF-8, so warlock's section cannot be
    /// spliced into it without guessing at bytes it cannot read. Nothing is
    /// written.
    NotText {
        /// The file that is not text.
        path: PathBuf,
    },
    /// The file could not be written: the temporary failed, or the rename over
    /// the target did. Nothing is left half-written and no temporary is left
    /// behind.
    Write {
        /// The file that was being written — the target, never the temporary.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "could not read `{}`: {source}", path.display())
            }
            Self::NotText { path } => write!(
                f,
                "`{}` is not UTF-8 text, so it was left alone",
                path.display()
            ),
            Self::Write { path, source } => {
                write!(f, "could not write `{}`: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::NotText { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{BEGIN, BODY, END, Error, FILE, Written, splice, write_claude_md};

    /// The entries of a directory, sorted, as strings: what a test asserting
    /// "one file and nothing else" compares against.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names = fs::read_dir(dir)
            .expect("lists the directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn the_body_says_the_things_it_exists_to_say() {
        // Not a spell check: these are the facts the ticket requires the text
        // to carry, and each is load bearing for somebody who has never met
        // warlock.
        for phrase in [
            "WARLOCK.md",
            "Read the `WARLOCK.md` files first",
            "behind the code it describes",
            "Unpacted",
            "Stale",
            "Fresh",
            "mechanical",
            "granted",
            "trigger, not the judgement",
        ] {
            assert!(BODY.contains(phrase), "the body should mention {phrase:?}");
        }
    }

    #[test]
    fn the_body_says_what_editing_a_document_costs() {
        // The one consequence an agent discovers the hard way if nobody writes
        // it down: it edits a `WARLOCK.md`, the directory goes stale under it,
        // and it has no idea whether that is a fault it caused or the system
        // working. Each phrase is one link of that chain — the edit, why the
        // hash moves, that it is not breakage, and that only a pass undoes it.
        for phrase in [
            "Editing a `WARLOCK.md` by hand makes its own directory stale",
            "sits in the directory it describes",
            "own bytes are part of\nthat directory's digest",
            "the moment an edit is saved",
            "ledger being honest",
            "road back to fresh is another model pass",
            "`r` in warlock's tree",
            "owed a pass",
        ] {
            assert!(BODY.contains(phrase), "the body should mention {phrase:?}");
        }
    }

    #[test]
    fn the_body_says_the_things_it_exists_to_say_about_scopes_and_sigils() {
        // Same standard as above, for the second half of the vocabulary: each
        // phrase is a fact an agent gets wrong by default if nobody says it.
        for phrase in [
            "## Scopes and sigils",
            "may carry one **scope**",
            "`.warlock/pacts.toml`",
            "shown in the tree",
            "covers everything beneath it until a nearer one overrides it",
            "*default*",
            "not\na second gate",
            "open to anyone",
            "**sigil**",
            "may hold several",
            "any one of them opens a matching scope",
            "membership test, not an expression to\nevaluate",
            "`warlock config` prints what is held",
            "unrestricted",
            "A scope is a term of the pact, not a thing beside it",
            "unpacted\ndirectory cannot carry one",
            "un-pacting a directory takes its scope away",
            "Before planning or making a change",
            "crosses\na scope they do not hold",
            "say so plainly",
            "name the sigil the work wants",
            "Never refuse the work, never\nblock it",
            "never quietly edit around the boundary",
        ] {
            assert!(BODY.contains(phrase), "the body should mention {phrase:?}");
        }
    }

    #[test]
    fn the_body_promises_no_enforcement() {
        // Nothing in this workspace matches a sigil against a scope, so the
        // text must not imply that anything does. A file that promises a wall
        // warlock does not build is worse than no file: the first person to
        // sail through it learns the document lies. (The instruction *to* the
        // agent says "never block" and "never refuse", which is why these are
        // claim-shaped phrases rather than bare words.)
        for phrase in [
            "enforc",
            "warlock check",
            "blocks",
            "blocked",
            "refuses",
            "refused",
            "will refuse",
            "will block",
            "checks that",
            "not allowed",
            "permission",
            "exit code",
            "CI",
        ] {
            assert!(
                !BODY.contains(phrase),
                "the body should promise no enforcement: {phrase:?}"
            );
        }
    }

    #[test]
    fn the_body_offers_no_guidance_on_writing_a_document() {
        // The reason is on the constant: a pacting pass cannot read this file,
        // so anything prompt-shaped here is a second prompt nothing obeys.
        for phrase in ["Write the WARLOCK.md", "level-one Markdown heading"] {
            assert!(
                !BODY.contains(phrase),
                "the body should not carry prompt text: {phrase:?}"
            );
        }
    }

    #[test]
    fn an_absent_section_is_added_to_an_empty_file() {
        let spliced = splice("");
        assert!(spliced.starts_with(BEGIN), "{spliced}");
        assert!(spliced.ends_with(&format!("{END}\n")), "{spliced}");
        assert!(spliced.contains(BODY));
    }

    #[test]
    fn an_absent_section_is_appended_and_keeps_every_byte_in_front_of_it() {
        let before = "# CLAUDE.md\n\nOur own house rules.\n";
        let spliced = splice(before);
        assert!(
            spliced.starts_with(before),
            "the reader's own text comes through byte for byte: {spliced:?}"
        );
        assert_eq!(
            &spliced[before.len()..],
            &format!("\n{BEGIN}\n\n{BODY}\n\n{END}\n")
        );
    }

    #[test]
    fn a_file_without_a_trailing_newline_does_not_get_the_marker_glued_on() {
        let spliced = splice("no newline at the end");
        assert!(
            spliced.starts_with("no newline at the end\n\n<!--"),
            "{spliced:?}"
        );
    }

    #[test]
    fn a_present_section_is_replaced_in_place_and_never_appended_twice() {
        let before = format!("# Ours\n\nBefore.\n\n{BEGIN}\nstale warlock text\n{END}\n\nAfter.\n");
        let spliced = splice(&before);

        assert_eq!(
            spliced.matches(BEGIN).count(),
            1,
            "one section, not two: {spliced}"
        );
        assert!(spliced.starts_with("# Ours\n\nBefore.\n\n"), "{spliced:?}");
        assert!(spliced.ends_with("\n\nAfter.\n"), "{spliced:?}");
        assert!(!spliced.contains("stale warlock text"));
        assert!(spliced.contains(BODY));
    }

    #[test]
    fn splicing_is_a_fixed_point_from_both_starting_points() {
        for before in ["", "# Ours\n\nBefore.\n"] {
            let once = splice(before);
            assert_eq!(splice(&once), once, "starting from {before:?}");
            assert_eq!(splice(&splice(&once)), once, "and again");
        }
    }

    #[test]
    fn an_unterminated_section_is_re_ended_rather_than_doubled() {
        // What a save interrupted half way through leaves. Appending would put
        // a second BEGIN in the file, and the run after that would take the
        // first marker with the second's END and swallow the text between them.
        let spliced = splice(&format!("# Ours\n\n{BEGIN}\nhalf a section"));
        assert_eq!(spliced.matches(BEGIN).count(), 1, "{spliced}");
        assert_eq!(spliced.matches(END).count(), 1, "{spliced}");
        assert!(spliced.starts_with("# Ours\n\n"), "{spliced:?}");
        assert!(!spliced.contains("half a section"), "{spliced}");
        assert_eq!(splice(&spliced), spliced, "and it settles down after that");
    }

    #[test]
    fn a_write_into_an_empty_repository_leaves_exactly_one_file() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir(repo.path().join(".git")).expect("a .git directory");

        let written = write_claude_md(repo.path()).expect("writes");

        assert_eq!(
            written,
            Written::Created {
                path: repo.path().join(FILE)
            }
        );
        assert_eq!(
            listing(repo.path()),
            [".git", FILE],
            "one file written, and no temporary left behind on the success path"
        );
        // Named one at a time as well as counted, because this is the promise:
        // init enrols nobody in anything.
        assert!(!repo.path().join(".warlock").exists(), "no bookkeeping");
        assert!(
            !repo.path().join(".warlock/pacts.toml").exists(),
            "no manifest"
        );
        assert!(
            !repo.path().join(".warlockignore").exists(),
            "no rules file"
        );
        assert!(!repo.path().join("WARLOCK.md").exists(), "no document");
    }

    #[test]
    fn an_existing_file_keeps_every_byte_outside_the_delimiters() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let before = "# CLAUDE.md\n\nRun the tests.\n\nAnd the linter.\n";
        fs::write(repo.path().join(FILE), before).expect("writes the file");

        let written = write_claude_md(repo.path()).expect("writes");
        assert!(matches!(written, Written::Updated { .. }), "{written:?}");

        let after = fs::read_to_string(written.path()).expect("reads it back");
        let (theirs, ours) = after.split_at(before.len());
        assert_eq!(theirs, before, "byte for byte");
        assert_eq!(ours, format!("\n{BEGIN}\n\n{BODY}\n\n{END}\n"));
        assert_eq!(listing(repo.path()), [FILE], "and no temporary behind");
    }

    /// The body as brief 08 shipped it: documents and the three colours, and
    /// nothing about scopes. Kept verbatim rather than derived from [`BODY`],
    /// because the point of the test below is that a file written by the *old*
    /// warlock is brought forward by the new one.
    const BRIEF_08_BODY: &str = "\
## Warlock

Warlock is a freshness ledger for a codebase's own documentation. Every
directory under its management holds a `WARLOCK.md`: a document about that
one directory — its files, and what the directories below it are for —
written by a model pass and committed beside the code, like any other source
file.

**Read the `WARLOCK.md` files first.** They are the fast way into this
repository: read the ones covering the area you are about to work in before
you start opening source files.

## What the colours mean

- **Unpacted** — outside warlock's management.
- **Stale** — pacted, and something at or below it has changed.
- **Fresh** — pacted, and granted after a model pass read the directory.

There is deliberately no fourth colour.";

    #[test]
    fn a_brief_08_block_gains_the_new_text_and_disturbs_nothing_around_it() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let before = "# CLAUDE.md\n\nRun the tests.\n";
        let after = "\n## Ours\n\nAnd the linter.\n";
        let old = format!("{before}\n{BEGIN}\n\n{BRIEF_08_BODY}\n\n{END}\n{after}");
        fs::write(repo.path().join(FILE), &old).expect("writes the file");

        let written = write_claude_md(repo.path()).expect("writes");
        assert!(matches!(written, Written::Updated { .. }), "{written:?}");
        let text = fs::read_to_string(written.path()).expect("reads it back");

        // The new section arrived.
        assert!(text.contains("## Scopes and sigils"), "{text}");
        assert!(
            text.contains("`warlock config` prints what is held"),
            "{text}"
        );
        assert!(text.contains(BODY), "the whole current body, not a patch");
        assert_eq!(text.matches(BEGIN).count(), 1, "still one section: {text}");

        // And every byte outside the markers is exactly as the reader left it.
        let start = text.find(BEGIN).expect("an opening marker");
        let finish = text.find(END).expect("a closing marker") + END.len();
        assert_eq!(&text[..start], &old[..old.find(BEGIN).expect("the same")]);
        assert_eq!(
            &text[finish..],
            &old[old.find(END).expect("the same") + END.len()..]
        );
        assert_eq!(listing(repo.path()), [FILE], "and no temporary behind");

        // A second run after the upgrade is still a no-op on the bytes.
        write_claude_md(repo.path()).expect("writes again");
        assert_eq!(
            fs::read_to_string(repo.path().join(FILE)).expect("reads it back"),
            text
        );
    }

    #[test]
    fn twice_is_byte_identical_to_once_from_both_starting_points() {
        for before in [None, Some("# CLAUDE.md\n\nOurs.\n")] {
            let repo = tempfile::tempdir().expect("a temporary directory");
            if let Some(text) = before {
                fs::write(repo.path().join(FILE), text).expect("writes the file");
            }

            let first = write_claude_md(repo.path()).expect("writes once");
            let once = fs::read(first.path()).expect("reads it back");

            let second = write_claude_md(repo.path()).expect("writes twice");
            assert!(
                matches!(second, Written::Updated { .. }),
                "the second run finds a file: {second:?}"
            );
            assert_eq!(
                fs::read(second.path()).expect("reads it back"),
                once,
                "starting from {before:?}"
            );
            assert_eq!(listing(repo.path()), [FILE]);
        }
    }

    #[test]
    fn a_file_that_is_not_text_is_left_alone() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let bytes = [0xff_u8, 0xfe, 0x00, 0x41];
        fs::write(repo.path().join(FILE), bytes).expect("writes the file");

        let error = write_claude_md(repo.path()).expect_err("bytes that are not text");
        assert!(matches!(error, Error::NotText { .. }), "{error:?}");
        assert!(error.to_string().contains(FILE), "{error}");
        assert_eq!(
            fs::read(repo.path().join(FILE)).expect("reads it back"),
            bytes,
            "untouched"
        );
        assert_eq!(listing(repo.path()), [FILE], "and no temporary behind");
    }

    #[test]
    fn a_directory_where_the_file_should_be_is_an_error_not_a_panic() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir(repo.path().join(FILE)).expect("a directory in its place");

        let error = write_claude_md(repo.path()).expect_err("a directory is not a file");
        assert!(
            matches!(error, Error::Read { .. } | Error::Write { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains(FILE), "{error}");
    }

    /// Chmod cannot deny root anything, so the test checks the fixture really
    /// is unwritable before asserting on it and steps aside when it is not.
    #[cfg(unix)]
    #[test]
    fn an_unwritable_directory_is_an_error_naming_the_file_and_leaves_nothing() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = tempfile::tempdir().expect("a temporary directory");
        let before = "# Ours\n\nKeep me.\n";
        fs::write(repo.path().join(FILE), before).expect("writes the file");
        // Readable and listable, so the existing file is still read, but
        // nothing new can be created in it — neither the temporary nor a rename
        // over the target.
        fs::set_permissions(repo.path(), fs::Permissions::from_mode(0o555)).expect("chmods");
        if fs::write(repo.path().join("probe"), "").is_ok() {
            // Running as root: no directory is unwritable, so there is nothing
            // here to assert against.
            fs::remove_file(repo.path().join("probe")).expect("removes the probe");
            fs::set_permissions(repo.path(), fs::Permissions::from_mode(0o755)).expect("chmods");
            return;
        }

        let error = write_claude_md(repo.path()).expect_err("an unwritable directory");
        match &error {
            Error::Write { path, .. } => {
                assert_eq!(
                    path,
                    &repo.path().join(FILE),
                    "the target, not the temporary"
                );
            }
            other => panic!("expected a write failure, got {other:?}"),
        }
        assert!(error.to_string().contains(FILE), "{error}");

        fs::set_permissions(repo.path(), fs::Permissions::from_mode(0o755)).expect("chmods back");
        assert_eq!(
            fs::read_to_string(repo.path().join(FILE)).expect("reads it back"),
            before,
            "the write is atomic, so a failure leaves the old file whole",
        );
        assert_eq!(
            listing(repo.path()),
            [FILE],
            "and no temporary behind on the failure path"
        );
    }
}
