![Warlock](assets/warlock-logo.png)

# warlock

(Currently under development)

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
coloured by whether that understanding is still true. That document is a
`WARLOCK.md`, one per directory, written by the AI and committed alongside the
code it describes. You can still open files and read them. That is no longer the
main event.

## How the work flows

A change does not start with a file. It starts with a conversation.

You talk the change through with the patron, and it pushes back: what exactly do
you want, which modules does this touch, what does done look like. What comes
out is a brief and a set of tickets, each carrying a predicted module footprint.
Later, you or someone else pulls a ticket and works it, with the boundary
already agreed and the context already written down. The tree goes yellow where
the code moved. A refresh sends the AI back over its own diff to update what it
understands, and yellow goes green. That is the loop.

What puts a module in that loop is a pact. You point at a directory and pact it,
and the pact is taken over the whole subtree beneath it: every directory in it
gets a `WARLOCK.md` and comes under Warlock's care in one act, rather than being
adopted one folder at a time. Un-pacting is the exact inverse and is safe to
reach for: it drops the pact over that subtree and Warlock stops tracking it,
but the documents stay on disk. Pacting something to see what it says, then
un-pacting it, costs you nothing but the run.

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
- **Your subscription, your leverage.** Warlock holds no credentials of its own
  and resells no inference. It drives the `claude` CLI you are already paying
  for, which has to be on your `PATH` and logged in before Warlock can do
  anything with a model; the account, the plan and the limits stay yours.
  Warlock's job is to spend that spend well by feeding it exactly the right
  slice of the project.

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
- **Not a squatter in your docs.** The only files Warlock writes are its own
  `WARLOCK.md` files and its state under `.warlock/`. Your project's `README.md`
  files are yours: Warlock never writes to one, and pacting a directory that has
  a `README.md` leaves it exactly as it was.

What it is: the same game you are already playing, given a shape. You were
already directing an AI. This makes that legible, structured, and durable enough
to survive handoff, onboarding, and the agent not being you.

The guiding principle is narrow on purpose:

> Warlock makes the right thing visible and easy. It never makes the wrong thing
> impossible.

It holds for your code, your tickets and your documents, and there is one
deliberate exception: warlock's own mutating keys refuse to work inside a
**scope** this machine does not hold a matching **sigil** for. That is a
guardrail against a fumbled keystroke rather than a guarantee about a person —
the sigil file is local and self-asserted, so anyone may grant themselves one —
and it exists because the un-pact direction of `p` is not the harmless undo it
looks like. See [Key binds](#key-binds) for which keys, and
[`warlock config`](#key-binds) for what this machine holds.

See [`docs/warlock-design-doc.md`](docs/warlock-design-doc.md) for the full model: pacts,
freshness, gating, and where the boundaries are drawn.

## Key binds

Warlock has three places the keyboard can be — the tree, the panel beside it,
and the composer under the panel — and `Tab` moves between them. Almost every
command is a single unshifted letter, which the composer is allowed to take back
the moment it holds the keyboard: while you are typing a draft, `p` is the
letter `p` and not the pact key.

### Moving around

| Key | What it does |
| --- | --- |
| `k` / `↑` | Up one row — the tree's selection, or the panel's window when the panel has focus |
| `j` / `↓` | Down one row |
| `PgUp` / `PgDn` | Up or down one screenful |
| `g` | Go to the first row, or the top of the panel |
| `G` | Go to the last row, or the bottom of the panel — this is what returns a scrolled-back panel to the newest line |
| `Space` | Fold the subtree under the selected row, or unfold it again |
| `Tab` | Move the focus on one place: tree → panel → composer → tree, skipping the composer when it is not on screen |
| `Shift-Tab` | Show the panel's other card: the document if the account is up, the account if the document is |

At the composer, the movement keys move nothing — neither the tree's selection
nor the panel's window is what you are pointed at.

### Changing what the tree shows

| Key | What it does |
| --- | --- |
| `o` | Show only what warlock is managing — pacted directories, nothing else |
| `f` | Show the files inside each directory as well as the directories |

### Doing something

| Key | What it does | Scoped |
| --- | --- | --- |
| `p` | Pact the selected directory, over its whole subtree — or un-pact it if it is already pacted, which is a manifest edit only and leaves the documents on disk | yes |
| `r` | Refresh: run a pass over just the stale directories of an already-pacted subtree | yes |
| `s` | Open the scope prompt on a pacted directory | yes |
| `v` | Read the selected file into the panel's document card | no |
| `e` | Open the selected file in `$EDITOR`, handing the terminal over and taking it back afterwards | no |
| `m` | Stop the terminal reporting its mouse, or ask it to start again — with capture off, dragging over the screen selects text the way it does in any other program | no |

`p`, `r`, `s` and `e` are refused while a run is already going: one run at a
time, and the terminal cannot be handed to an editor from under a pass still
drawing on it. `v` and `Shift-Tab` are never refused — they race nothing.

**The scoped column** is the boundary. A directory's scope is the one written on
the nearest pacted directory at or above it — nearest wins outright, so an outer
scope is a default for whatever said nothing below it rather than a second gate
to also satisfy — and a key marked *yes* is refused there unless this machine
holds a sigil matching it. The footer names the scope wanted. `warlock config`
prints and sets what this machine holds; a sigil of `*` opens everything.

The permissive default is on the **directory** and only there: a directory nobody
scoped is open to anyone, which is why a repository that has never drawn a
boundary is unaffected by any of this. It is not mirrored on the sigil side. A
sigil is what opens a scope, so a machine holding none opens none — running
warlock in a scoped repository without recording what you hold gets you refused,
in the same words as holding the wrong sigil. That is the onboarding this
vocabulary was designed around: you are handed your scopes the way you are handed
any other credential, and you record them once.

`s` is on that list because you must hold a boundary in order to redraw it —
otherwise the one key whose entire purpose is to move a scope would be the one
key no scope covered. `p` is on it in **both** directions, and the un-pact
direction is the reason the list exists: un-pacting drops the scope along with
the pact, so a mis-aimed `p` on somebody else's subtree costs a full model pass
to put back and does not restore the boundary when it returns.

### Leaving

| Key | What it does |
| --- | --- |
| `q` / `Esc` | Ask whether to quit |
| `Ctrl-C` | Quit outright, with no question |

Ctrl-C is a keystroke here rather than a signal: the terminal is in raw mode, so
warlock answers it itself, including taking a running `claude` with it.

### While a run is going

| Key | What it does |
| --- | --- |
| `Esc` | Cancel the run — *not* quit, for as long as there is a run to stop |
| `q` / `Ctrl-C` | Quit outright, with no confirmation |

Every other key means exactly what it means the rest of the time. The tree stays
usable while a pact or a refresh works.

### In the composer

Reached with `Tab`. While it holds the keyboard, ordinary characters go into the
draft.

| Key | What it does |
| --- | --- |
| `Enter` | Send the draft |
| `Alt-Enter` | Put a newline in the draft |
| `Backspace` | Delete the last character |
| `Esc` | Hand the keyboard back and keep the draft exactly as it was |
| `Tab` | Move the focus on — the one key the composer does not take |
| `Ctrl-C` | Cancel the turn if one is being answered; otherwise quit |

While a turn is being answered the field is muted and takes no keys at all — one
question at a time. Shift-Enter is read as `Enter`, because too many terminals
never report it as a keystroke of its own.

### At the quit question

| Key | What it does |
| --- | --- |
| `←` / `→` | Light Yes, or light No |
| `Enter` | Take whichever answer is lit |
| `y` | Quit |
| `n` / `Esc` | Stay |

The question opens with No lit, and `Esc` answers No rather than leaving — the
reflex second press is the whole reason the question is there.

### At the scope prompt

| Key | What it does |
| --- | --- |
| `Enter` | Write the scope, or clear it if the field is empty |
| `Backspace` | Delete the last character |
| `Esc` | Close the prompt without writing anything |

### Mouse

Live only while capture is on, which is the default and what `m` turns off.

| Action | What it does |
| --- | --- |
| Left click | Select the row and give that pane the keys; a click on the row already selected folds it |
| Wheel | Scroll whichever pane the pointer is over, three rows or lines a notch, whatever has the focus |

Drags, moves, the other buttons and the horizontal wheel are read and dropped,
and the pointer means nothing anywhere while the quit question or the scope
prompt is up.

## Troubleshooting

**Why are my borders not even?**

Your terminal is almost certainly falling back to a second font. Warlock draws
its panes with Unicode box-drawing characters, one per cell, and a font with no
glyphs for them hands the job to whichever font the terminal finds next — whose
glyphs are rarely exactly one cell wide or aligned on the same baseline. The
result is segments that sit slightly high, low or short, and corners that do not
meet. Nothing is wrong with the layout: every border is a single character in a
single cell, and it lines up the moment the font can draw it.

Use a monospace font with box-drawing coverage — most programming fonts have it
— or turn on your terminal's own line-drawing, which some emulators offer as a
setting for exactly this reason. If it is only *some* rows that are off, check
whether your terminal is configured to treat ambiguous-width characters as
double width.

## Contributing

Run these three checks before pushing. CI runs exactly the same three commands on
every push and pull request, so if they pass locally they pass there too:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
