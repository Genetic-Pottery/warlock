![Warlock](assets/warlock-logo.png)

# warlock

See your codebase the way your AI does. A TUI where documentation is the interface.

> **warlock** *(n.)* one who draws power from a pact with an entity greater than
> themselves. The patron is the model. The pact is the boundary. Invocations are
> agent runs.

## Why this exists

Editors and AI-enabled IDEs bolt AI on the side. The file tree is still the
interface, what they accelerate is how fast you type rather than how well the
model understands, and the model itself is a helper you summon into the margin
of a window built for typing.

Meanwhile most people are shipping AI-written code and sanding it down to look
hand-written. Stripping the em dashes. Not letting the model commit. A whole
industry using AI while performing restraint, which means using it poorly: no
structure, no shared context, no record. The pretending is the waste.

Warlock admits what the game is. The interface is not your filesystem, it is
your project rendered as the AI understands it: a tree of module documents,
coloured by whether that understanding is still true. You can still open files
and read them. That is no longer the main event.

## How the work flows

A change does not start with a file. It starts with a conversation.

You talk the change through with the patron, and it pushes back: what exactly do
you want, which modules does this touch, what does done look like. What comes
out is a brief and a set of tickets, each carrying a predicted module footprint.
Later, you or someone else pulls a ticket and works it, with the boundary
already agreed and the context already written down. The tree goes yellow where
the code moved. A refresh sends the AI back over its own diff to update what it
understands, and yellow goes green. That is the loop.

The conversation is not overhead in front of the work. It is the work. Eight
minutes of genuine back and forth, spent acting as your own PM and
pressure-testing what you actually want, routinely saves hours on the other end.
The alternative is the way most people use an agent today: fire a one-line
prompt, watch it confidently build the wrong shape, then wrestle it back turn
after turn, paying for the thinking you skipped in the most expensive currency
available. Warlock's job is to make sure those eight minutes happen, and that
nothing about them is lost afterwards.

## What it promises

- **Documentation that is actually current.** A stale doc is stale the moment
  the code under it changes, mechanically and immediately. Fresh has to be
  earned by an AI pass that reads the diff and either confirms the doc still
  holds or rewrites it until it does. There is no unjudged middle state,
  because unjudged is stale.
- **A record that survives you.** Every change lands with intent attached in
  the tracker of your choice, and the context lives in the repo. Someone clones
  it and their agent picks up where yours left off. No one's head, no one's
  subscription.
- **Process artifacts as a byproduct.** Most teams LARP process: the ticket
  exists, the doc exists, and both are one-sentence husks. Warlock's artifacts
  are real by construction, because the work runs through them rather than
  around them.
- **Your subscription, your leverage.** Warlock holds no credentials and
  resells no inference. It drives the `claude` CLI you are already paying for,
  and its job is to spend that spend well by feeding it exactly the right slice
  of the project.

## What it is not

- **Not autonomous.** Nothing merges because a model felt good about it. Every
  consequential step is a human decision, and the point of the framework is that
  the decision leaves a trace instead of evaporating into your Enter key.
- **Not a project generator.** It will not turn one sentence into a shipped
  feature, and it does not manufacture thought. A lazy one-liner inflated into a
  beautifully formatted page is still a lazy one-liner, now with better
  typography. The structure carries thinking; it cannot replace it.
- **Not a way to stop knowing your codebase.** Whether a document needs updating
  is a subjective call, and Warlock makes that judgment visible rather than
  pretending to settle it. The map is negotiable. The code still has to work.
- **Not an editor.** You can hand-edit anything, and the next refresh will
  notice and reconcile. But the tool is not optimised for living in a file, and
  anyone who wants to is not the customer.

What it is: the same game you are already playing, given a shape. You were
already directing an AI. This makes that legible, structured, and durable enough
to survive handoff, onboarding, and the agent not being you.

The guiding principle is narrow on purpose:

> Warlock makes the right thing visible and easy. It never makes the wrong thing
> impossible.

See [`docs/warlock-design-doc.md`](docs/warlock-design-doc.md) for the full model: pacts,
freshness, gating, and where the boundaries are drawn.

## What works today

Everything above is the destination. What the binary does right now is smaller,
and worth saying plainly:

- **It renders your actual repository.** `warlock` walks the directory you
  launched it in and draws every directory underneath it that git would not
  ignore — `.gitignore` at every level, hidden directories and your global
  excludes are all honoured, and Warlock's own `.warlock/` is skipped on top of
  that. That is the whole tree, not the documented part of it: every directory
  is a row, and every row is gray until you pact it.
- **A module is a directory with a `README.md` in it.** The file has to sit
  directly in the directory, that is the entire test, and no README is ever
  parsed — Warlock only cares that one exists. A directory without one is not a
  special kind of row; it is an ordinary undocumented directory, drawn like any
  other, which cannot be pacted because there is no document to pact.
- **The keys.** Up/down or `k`/`j` move the selection by a row, PgUp/PgDn by a
  screenful, and `g` and `G` jump to the first and last row. Space collapses the
  selected directory and expands it again. `f` shows the files inside each
  directory and hides them again; `o` narrows the tree to the pacted nodes and
  the ancestors that reach them, and shows the whole thing again. `p` pacts.
  `q`, `Esc` or Ctrl-C leave.
- **`p` toggles a pact on the selected node.** Gray becomes yellow on the next
  frame; press `p` again and it goes back to gray. One press, one node, no
  confirmation — the action is its own undo. Pressed on an undocumented
  directory or on a file, it changes nothing and puts one line in the footer
  saying which it was, rather than doing nothing quietly.
- **The pact is written down as you press the key.** It lands in
  `.warlock/pacts.toml` at the repository root — the nearest ancestor holding a
  `.git/` directory: one manifest per repository, committed to git, because a
  pact is a fact about the repository rather than about your checkout. Each
  pacted module gets one entry naming its directory, the README that documents
  it, and a granted hash — the last of which appears only once freshness has
  been granted. Every path in it is relative to the repository root, so two
  clones of the same commit hold the same file.
- **Files are visible, not readable.** `f` puts them on screen in the colour of
  the module holding them, and that is all it does: nothing opens a file, and no
  key tries to. A viewer pane is deliberately the next thing to build rather
  than a missing part of this one — the tree had to be worth moving around in
  before there was any point putting a file beside it.

**Green is not reachable through the product.** Freshness is granted by an AI
pass over the diff, and that pass does not exist yet: nothing here invokes
`claude`, and no code outside the tests writes a granted hash. A module you pact
is yellow from the moment you pact it and stays yellow, because a pact that has
never been judged is stale by definition. The only way to see green today is to
hand-write a matching `granted_hash` into the manifest yourself.

## Contributing

Run these three checks before pushing. CI runs exactly the same three commands on
every push and pull request, so if they pass locally they pass there too:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
