use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::{temp_file_name, write_and_sync};

// `CLAUDE.md` and not `AGENTS.md`, which this was, for one mechanical reason:
// `CLAUDE.md` is loaded at the start of every session and `AGENTS.md` is not
// read at all. Orientation nothing loads is not orientation. The name is a fact
// about what reads the file rather than a preference, so that is the thing to
// re-check before changing it again.
const FILE: &str = "CLAUDE.md";

const BEGIN: &str = "<!-- warlock:begin -->";

const END: &str = "<!-- warlock:end -->";

// The obvious next section — how to author a good `WARLOCK.md` — is missing on
// purpose. A pacting pass runs headless with its own system prompt and its whole
// request on stdin: it never reads this file. Guidance here would be a second
// prompt no pass obeys, free to drift from the real one in `pact` with nothing
// to catch the drift.
//
// Hard wrapped, unlike the prompts, and these source lines are the lines that
// land on disk: this is a file somebody commits and reviews, so a changed
// sentence should be a changed line and not a reflowed paragraph.
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

**They are maps, not specifications.** A `WARLOCK.md` is written to get you
to the right file quickly, and it is not the last word on anything it
describes. Use it to find the surface area a question touches — which
directory, which file, which function — and then open that file and check.
Every document says this at the top, in the line warlock stamps on it.

**Every document has the same shape.** A heading, a purpose, then `## Files`
(one line per file, with its size), `## Directories` (one line per child
that has a document), and, where there is anything to say, `## Structure`,
`## Rules` and `## Where to look`: routes from a question to the file, and
the name in it, to open. Warlock lays that out itself from a model's
answer, and checks the answer against the directory before writing it, so
a line cannot name a file or a symbol that is not there. Start at the
nearest document above the code you are about to touch, follow `## Where
to look` and the file lines down, and open the file they name.

Two habits follow from that, and they matter more than anything else here:

- **Verify before you rely.** Confirm a claim against the source before you
  build on it, quote it in a review, or repeat it to someone. A document is
  the fastest way to find the evidence; it is not the evidence.
- **Silence is not absence.** If a document does not mention something, that
  is not a finding. It may be missing because the code changed, because the
  pass had no room for it, or because nobody thought it worth a line — and a
  document covering a large directory necessarily leaves things out. Never
  conclude that a thing does not exist because no document says it does. Go
  and look.

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
evaluate. `warlock config` prints what is held. A sigil is what opens a scope, so
holding none opens none: an operator who has recorded nothing is refused by every
scoped directory, exactly as one holding the wrong sigil is. The permissive
default sits on the directory instead — a pacted directory with no scope above it
is open to anyone.

**A scope is a term of the pact, not a thing beside it.** An unpacted
directory cannot carry one, and un-pacting a directory takes its scope away
with the rest of the pact. So a boundary is changed by changing a pact, and
there is nowhere else to look for one.

**Warlock's own mutating keys refuse across a closed boundary.** `p` (in both
directions), `r` and `s` are turned down on a directory whose covering scope
this machine does not hold, with a footer line naming the scope wanted. It is a
guardrail rather than a guarantee — the sigil file is machine-local and
self-asserted, so anyone can grant themselves one — and it exists to stop a
fumbled keystroke, not a determined person. The un-pact direction is what it is
really for: un-pacting drops the scope along with the pact, so a mis-aimed `p`
costs a full model pass to undo and does not bring the boundary back with it.
`v` and `e` stay open, and an unscoped directory stays open to everyone.

**You are not warlock, and this does not gate your edits.** The keys above
refuse; you do not. **Before planning or making a change, look at what the
directories it would touch are scoped to, and at what the operator holds.** When
the work crosses a scope they do not hold, say so plainly, and then either narrow
the work to what they hold or name the sigil the work wants. Never refuse the
work, never block it, and never quietly edit around the boundary: saying what is
being crossed is the whole of the job here.";

fn section() -> String {
    format!("{BEGIN}\n\n{BODY}\n\n{END}")
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Written {
    Created {
        path: PathBuf,
    },
    Updated {
        path: PathBuf,
    },
}

impl Written {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Created { path } | Self::Updated { path } => path,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    NotText {
        path: PathBuf,
    },
    Write {
        path: PathBuf,
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

    #[test]
    fn this_repositorys_own_claude_md_holds_exactly_what_warlock_writes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("a workspace member sits two levels under its root");
        let ours =
            std::fs::read_to_string(root.join(FILE)).expect("this repository has a CLAUDE.md");

        let start = ours.find(BEGIN).expect("with warlock's opening marker");
        let end = ours.find(END).expect("and its closing one");
        let between = ours[start + BEGIN.len()..end].trim();

        assert_eq!(
            between,
            BODY.trim(),
            "the block in this repository's CLAUDE.md and the one `warlock init` \
             writes have drifted apart; change both or neither"
        );
    }

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
            assert!(
                flat(BODY).contains(&flat(phrase)),
                "the body should mention {phrase:?}"
            );
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
            assert!(
                flat(BODY).contains(&flat(phrase)),
                "the body should mention {phrase:?}"
            );
        }
    }

    fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
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
            // Was "unrestricted", from the sentence "Holding nothing means
            // unrestricted, not shut out" — which `scope_opens_to` contradicts:
            // an empty `held` falls through to the membership test and matches
            // nothing. The permissive default lives on the directory instead.
            "holding none opens none",
            "no scope above it\nis open to anyone",
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
            assert!(
                flat(BODY).contains(&flat(phrase)),
                "the body should mention {phrase:?}"
            );
        }
    }

    #[test]
    fn the_body_describes_the_refusal_it_has_without_promising_one_it_has_not() {
        // This test used to assert the opposite — that nothing in this
        // workspace matches a sigil against a scope, so the text must never say
        // anything is refused. That stopped being true the day
        // [`scope_opens_to`](crate::scope_opens_to) shipped and the TUI began
        // turning `p`, `r` and `s` down on it, and the assertion outlived the
        // fact by holding the stale claim in place with a green test. Both
        // halves below are the replacement, and the second half is what the
        // original was really protecting.
        for phrase in [
            "mutating keys refuse across a closed boundary",
            "`v` and `e` stay open",
        ] {
            assert!(
                flat(BODY).contains(&flat(phrase)),
                "the body should say what really is refused: {phrase:?}"
            );
        }

        // And it must still not sell that refusal as a wall. The sigil file is
        // machine-local and self-asserted, so anyone can grant themselves one; a
        // file promising a guarantee warlock does not build is worse than no
        // file, because the first person to sail through it learns the document
        // lies.
        for phrase in [
            "guardrail rather than a guarantee",
            "anyone can grant themselves one",
        ] {
            assert!(
                flat(BODY).contains(&flat(phrase)),
                "the body should say what the refusal is not: {phrase:?}"
            );
        }
        for phrase in ["warlock check", "not allowed", "permission", "exit code"] {
            assert!(
                !BODY.contains(phrase),
                "and should still promise no machinery that does not exist: {phrase:?}"
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
