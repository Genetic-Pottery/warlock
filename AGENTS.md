# AGENTS.md

Rough draft. A later ticket cleans this up and `warlock init` will generate it.

## Read the `WARLOCK.md` files first

Every directory under warlock's management has a `WARLOCK.md` beside it,
written by a model and committed with the code. It describes that directory:
its files, and what the directories under it are for.

**They are the fast way into this codebase.** Read the ones covering the area
you are working in before you start opening source files. A parent's document
is written from its children's documents, so reading top-down gets you the
shape of the thing in a few files instead of a few dozen.

One caveat: a document can be behind the code it describes. See the colours
below. If a document and the source disagree, the source is right — and say so
in your work, because that gap is the thing warlock exists to make visible.

## What warlock is

A TUI where the project tree is rendered as the AI understands it: one document
per directory, each coloured by whether that understanding is still true.

- **Gray** — unpacted. Outside warlock's management.
- **Yellow** — pacted and stale. Content at or below it changed since the last
  grant. Stale is *mechanical*: the hash moved, so it is stale, immediately.
- **Green** — pacted and fresh. A model pass read the directory and granted it.
  Fresh is *earned* and can only be granted, never assumed.

There is deliberately no fourth state. Unjudged is stale.

**Pacting** is how a directory comes under management. You point at one and
pact it, and every directory beneath it is pacted too — each gets its own
`WARLOCK.md`, its own manifest entry and its own hash. Un-pacting is the
inverse and leaves the documents on disk.

**The hash is the trigger, not the judge.** `subtree_hash` digests every byte
of every file at or below a directory. It is recorded at grant time; when it
stops matching, something happened there and the document is owed a look. It
never decides whether the document is *wrong* — that is a model pass's job, and
whether a change warrants a doc update was always a judgement call. Warlock is
not trying to be correct about that. It is trying to make it visible.

The three sources of truth: the code, the `WARLOCK.md` documents, and
`.warlock/pacts.toml` (the manifest — one entry per pacted directory, holding
its granted hash).

## Working in this repo

Two crates. The dependency edge runs **TUI → engine, and never back**.

- `crates/warlock-engine` — the tree, the hash, the manifest, the pact. Pure
  domain logic. It opens no sockets and spawns no subprocesses, and its tests
  run with no terminal, no network and no `claude` binary present. Keep it that
  way.
- `crates/warlock-tui` — the terminal front end, and the only place a model is
  actually invoked (`claude.rs`).

The gate, which CI runs and which must be clean before you hand work back:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Standing rules:

- **No `unsafe`.** Denied at the workspace root.
- **No new dependency** without it being the point of the ticket. Whatever you
  need is probably already in the workspace table.
- **No configuration.** Caps, prompts, budgets and walk rules are constants in
  code, reviewed in a diff. A decision that can be changed without a diff is a
  decision nobody reviewed.
- **Prompts are code.** They live beside the code that sends them. No template
  files, no overrides.
- Stable Rust 2024, `rust-version` in `Cargo.toml`. There is no
  `rust-toolchain.toml` and it would be inert here.
- Public items need docs (`missing_docs` is warned, clippy is `-D warnings`).
  The house style is doc comments that say *why*, not what — match the density
  of the file you are editing.

## Briefs

`docs/red-brief-NN-*.md` are the project briefs, in order. Each one states its
outcome, success criteria, constraints, what is deliberately out of scope, and
its slices. `docs/warlock-design-doc.md` is the standing design; the briefs
cite it by section.

If you are implementing a ticket, its brief is the spec. Read it, and read the
"Out of scope" section too — a lot of what looks like an obvious improvement is
a decision that was already made against.
