# AGENTS.md

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

<!-- warlock:end -->
