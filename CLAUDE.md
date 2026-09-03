## About the repo

Everything in docs/ is from the past even if it was written today.
This means its not a document of current state or facts about the system.
Its a general direction about decisions made in the past and iterated on up to the present.
Ideas can be referenced from here but they should not be viewed as currently corrects or as a source of truth.

<!-- warlock:begin -->

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
being crossed is the whole of the job here.

<!-- warlock:end -->
