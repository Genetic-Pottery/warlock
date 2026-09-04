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

A change does not start with a file. It starts with a conversation — and that
conversation happens in Warlock, in the composer under the panel, rather than in
some other window you paste the result out of.

You talk the change through with warlock, and it pushes back: what exactly do
you want, which modules does this touch, what does done look like? Along the way
warlock warns you if the work you are planning crosses into another teams domain
but never fails there are many situations where you may want to create changes spanning multiple teams.

After settling on a feature to add to the product with warlock you can write the change document
to specified location.

[Not yet implemented]
1) A change document will be taken and turned into an epic / project in ticketing systems
2) Later an epic / project can be pulled and turned into tickets. Along the way warlock will warn if a drafted ticket crosses a boundry you do not have access to but will not hard fail
3) Finally tickets can be pulled and worked on if you have access to the directory the code change will happen in. At this phase warlock will hard fail if you do not have access.

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
[`warlock config`](#the-headless-cli) for what this machine holds.

See [`docs/warlock-design-doc.md`](docs/warlock-design-doc.md) for the full model: pacts,
freshness, gating, and where the boundaries are drawn.

## Key binds

Warlock has three places the keyboard can be — the tree, the panel beside it,
and the chat bar under the panel — and `Tab` moves between them.

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
holds a sigil matching it. The footer names the scope wanted.
[`warlock config`](#the-headless-cli) prints and sets what this machine holds;
a sigil of `*` opens everything.

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

### In the chat bar

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
| `/brief` | Enter brief mode: the same conversation is now converging on a document, sent the instruction the brief template describes |
| `/write` | Only in brief mode: ask for the document, then open an editable path prompt already holding a proposal like `docs/warlock-brief-13-scopes-and-sigils.md` |
| `/chat` | Leave brief mode, writing nothing — back to answering questions as they come |

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

## The headless CLI

Warlock with no subcommand opens the tree. With one, it does that one thing and
exits, and none of the subcommands goes near the terminal: no alternate screen,
no raw mode, nothing drawn. What each has to say is lines on stdout, so a
script, a CI job or an agent reads the answer through a pipe rather than into a
repaint.

| Command | What it does | What it spends |
| --- | --- | --- |
| `warlock init` | Write warlock's section of `CLAUDE.md` at the repository root | nothing |
| `warlock config` | Print the sigils this machine holds here, and read a line replacing them | nothing |
| `warlock stale [path]` | List the pacted directories at or below `path` that are stale | nothing |
| `warlock fresh [path]` | The same for the fresh ones | nothing |
| `warlock check <path>` | Say which scope covers `path`, what this machine holds, and whether the two meet | nothing |
| `warlock unpact <path>` | Drop the pact on a directory and every pact below it | one manifest write |
| `warlock scope add <path> <scope>` | Write a scope onto a pacted directory | one manifest write |
| `warlock scope remove <path>` | Clear the scope on a pacted directory | one manifest write |
| `warlock pact <path>` | Describe a directory and everything below it, a `WARLOCK.md` each | a model pass per directory |
| `warlock refresh <path>` | The same over only the directories that are not fresh | a model pass per stale directory |

The two listings take the repository root when the path is left off. Every other
path is required, and on `unpact` and `pact` that is the point rather than an
omission: the largest thing warlock can do to a repository must not also be the
thing an absent argument does by itself. `warlock pact .` is somebody having
said so.

`init` and `config` are the two that are about the checkout rather than the
ledger. `warlock init` writes warlock's section of the `CLAUDE.md` at the
repository root and says which file it wrote, and whether it was created or
updated. `warlock config` prints the repository, the file this machine keeps
sigils in and what is held now, then reads one line: the sigils on it replace
everything held for this repository, a blank line clears it, and Ctrl-C or EOF
changes nothing. It is the only road to a sigil, and a sigil is the only thing
that opens a scope.

### Asking

`stale`, `fresh` and `check` only read. None of them writes to
`.warlock/pacts.toml`, none spawns a process and none runs a model pass, so
asking costs no tokens, no minutes and no risk of a manifest left in a state
nobody asked for — which is what makes them safe to put in a CI job or an
agent's hands. What they read is what the tree itself reads: the same walk, the
same staleness rule, the same coverage, never a second opinion written on the
shell side.

The listings print one path a line and nothing else on the line, relative to the
repository root and spelled the way the manifest spells them:

```sh
$ warlock stale
.
crates
crates/warlock-engine
crates/warlock-engine/src

# nothing under the engine is behind its code
$ test -z "$(warlock stale crates/warlock-engine)"
```

`check` walks up from one path and answers in three lines — the scope covering
it, what this machine holds, and whether the two meet:

```sh
$ warlock check crates
nothing scopes `crates`
holding `data-plane`
an unscoped path is open to anyone, so this machine may work here
```

All three take `--json` and answer as one object on one line instead:

```sh
$ warlock stale --json
{"command":"stale","directories":[{"path":".","state":"stale"}]}

$ warlock check crates --json
{"command":"check","path":"crates","scope":null,"sigils":["data-plane"],"opens":true}
```

The verdict is a field and never a status. A closed scope is the answer to the
question rather than a failure to reach one, so `check` exits 0 either way,
which is what leaves the exit status free:
`warlock check <path> --json | jq -e '.opens'` spends `jq`'s status on the
verdict and warlock spends none of its own on saying no. The same goes for an
empty listing — nothing stale is an answer, and it is a 0.

### Writing

`unpact`, `scope add` and `scope remove` are `.warlock/pacts.toml` rewritten and
nothing else. No terminal, no process, no model pass, and every `WARLOCK.md`
left exactly where it was — un-pacting drops the record, not the documents.

All three ask the boundary first, before they look at whether the path has an
entry at all, so a command aimed inside a scope this machine does not hold
answers with the refusal and never with what the manifest holds. A refusal is
one line on stderr and **exit 3**, with the file byte-identical to what was
read. There is no `--force` and no environment variable past it:
`warlock config` is the one road, here exactly as it is in the panel.

An un-pact has a second refusal that is not that one. When the boundary over the
path itself is open but something *below* it carries a scope this machine does
not hold, what is being refused is the blast radius rather than the place — so
it is an ordinary **1**, and the sentence offers the road that needs no sigil:
un-pact the parts you hold.

### Running

`pact` and `refresh` are the two subcommands that spend anything: minutes, one
`claude --print` per directory, a `WARLOCK.md` written beside each of them, and
one manifest save at the end. They pass the same gate the cheap writes do, asked
before a single directory is walked — a boundary asked any later would be asked
after somebody's tokens were spent and somebody else's prose overwritten, and no
exit status puts that back.

Which directories a `refresh` describes is the engine's judgement, the same one
the `r` key gets: the ones that are not fresh, no wider and no narrower.

Progress is two lines a directory on stdout, and the denominator does not move
for the length of the run:

```
warlock: [3/6] documenting crates/warlock-engine/src
warlock: documented crates/warlock-engine/src
```

A directory whose pass failed never gets its second line. What it gets instead
is stderr, where every failing directory is named, one line each, and then
counted:

```
warlock: crates/warlock-tui/src — nothing was written for `/repo/crates/warlock-tui/src`: the model pass produced no answer: …
warlock: 1 of 6 directories failed — the manifest holds what the rest earned
```

That run exits **4**, and the manifest is saved either way: the grants the rest
of the subtree earned are on disk, so the thing to do about a 4 is re-run over
what failed rather than buy the whole descent again. The split between the two
streams is what makes it readable — `warlock pact . > run.log` puts the descent
in the file and leaves what went wrong on the terminal.

Ctrl-C is the only key a headless run has, and it is the panel's two answers in
order. The first press is Esc: the `claude` in flight is killed, so the stop
takes milliseconds rather than the rest of a five-minute pass, the descent ends
at the next directory rather than part way through one, and everything that
finished is hashed, granted and saved before the process leaves with **130**.
The second press is `q`: it exits at once, saving nothing and printing nothing.
Nothing is corrupted by taking it — every document and the manifest are written
beside and renamed over, so what is on disk is always a whole file.

### Exit statuses

| Status | What it means |
| --- | --- |
| `0` | Completed. The question was answered or the write happened, whatever the answer turned out to be — an empty listing and a scope closed to this machine included |
| `1` | Warlock could not do it: the repository will not resolve, the manifest will not parse or will not save, the path has no repository-relative spelling. The line on stderr is the thing to go and read |
| `2` | The command line was never a request. Clap's status and its wording, for a word warlock has no place for |
| `3` | Refused, with nothing spent: this machine's sigils do not open the scope covering the path. No byte moved, retrying changes nothing, and the road out is `warlock config` |
| `4` | Completed with failures: a run wrote the documents it could and saved the manifest, and the lines above the count name the directories that did not come out of it |
| `130` | Cancelled: somebody pressed Ctrl-C during a run, and what had finished by then is saved and granted. 128 plus SIGINT, so a shell, `make` and CI read it as interrupted without being told anything about warlock |

The three that are not 1 are not 1 because they want different things done about
them. A 3 says this checkout is outside that boundary, so stop and go and get
the sigil. A 4 says the work is partly on disk, so re-run over the part that is
not. A 130 says somebody decided to stop it, so nothing should retry it at all.
Telling those apart by their wording would be telling them apart by parsing
prose.

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
