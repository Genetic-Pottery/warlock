use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::replace_atomically;

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
(one line per file, with its size and the names it declares),
`## Directories` (one line per child that has a document), and, where there
is anything to say, `## Structure`. Warlock lays that out itself from a
model's answer, and checks every name a structure line leans on against the
directory before writing it. Start at the nearest document above the code
you are about to touch, follow the directory and file lines down, and open
the file they name.

Two habits follow from that, and they matter more than anything else here:

- **Verify before you rely.** Confirm a claim against the source before you
  build on it, quote it in a review, or repeat it to someone. A document is
  the fastest way to find the evidence; it is not the evidence.
- **Silence is not absence.** If a document does not mention something, that
  is not a finding. It may be missing because the code changed, because the
  pass had no room for it, or because nobody thought it worth a line — and a
  document covering a large directory necessarily leaves things out. Never
  conclude that a thing does not exist because no document says it does. Go
  and look. This holds for every search that comes back empty and not only
  for a document: a grep, a guessed path, a listing of somewhere the thing
  was never kept. Report what you did not find as not found rather than as
  not there, and say how you looked, so somebody who knows the answer can
  tell you where to look instead.

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

**Ask warlock what is held rather than going to look for it.** `warlock check
<path>` names the scope covering that path, the sigils this machine holds, and
whether the one opens the other — the whole question, in three lines, for the
directory you are about to touch. It runs headless and reads the machine's own
store, wherever that is: the store is warlock's business and not a path to go
searching for, and a sigil that is held is held whether or not you found the
file it sits in. A search that missed it is a search that missed it, and not a
finding that nothing is held.

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
        let after_begin = start + BEGIN.len();
        let finish = existing[after_begin..]
            .find(END)
            .map_or(existing.len(), |at| after_begin + at + END.len());
        return format!("{}{section}{}", &existing[..start], &existing[finish..]);
    }

    // A file that does not end in a newline is a file whose last line would
    // otherwise have the marker glued onto it.
    let separator = if existing.is_empty() {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{existing}{separator}{section}\n")
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

    let existing = existing
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| Error::NotText {
                path: target.clone(),
            })
        })
        .transpose()?;
    let text = splice(&existing.unwrap_or_default());

    // The target, not whichever path the failure landed on: the caller asked
    // for `CLAUDE.md` and how it gets written is this function's business.
    if let Err((_, source)) = replace_atomically(root, FILE, text.as_bytes(), None) {
        return Err(Error::Write {
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
    Created { path: PathBuf },
    Updated { path: PathBuf },
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
#[path = "tests/claude_md.rs"]
mod tests;
