# Red brief: Warlock at work

Make the pact visible while it happens, and make the tree tell the truth when
it ends. The screen becomes mostly a live account of what the model is doing —
which file it read, what it grepped for, how long it has been thinking — and a
finished pact puts the documents it wrote into the tree without a relaunch.

Read `docs/warlock-design-doc.md` before drafting, sections 5 and 11 in
particular. Section 5 fixes the tree, its three colours and the fact that
viewing is the primary mode; section 11 fixes the invocation and calls context
scoping the differentiator. Nothing in this project changes either. What
changes is that the invocation stops being opaque.

This is a project about **one panel and one reload**. It deliberately does not
build the file viewer that panel could obviously become, and it deliberately
does not build the filesystem watcher the reload could obviously trigger. Both
are named in *Out of scope* with the reason. The panel shows the model working
and nothing else; the reload happens when a pact ends and at no other time.

The behaviour this fixes is real and was observed. Pacting `crates/warlock-engine`
in this repository wrote two documents, granted two hashes and left the user
watching an unchanged yellow row, concluding the tool was broken — and the
documents it had just written were not in the tree until warlock was restarted.
Both halves of that are this project.

## Outcome

Press the pact key on a directory. The left panel — most of the screen — fills
with what the model is doing, one line per action, sectioned by directory as
the walk descends, with a clock running on the newest line so a long silence
reads as thinking rather than as a hang. When the run ends the panel says what
each directory produced and what the whole pact cost, and stays up to be read.

The tree, now a narrow column on the right, keeps its colours, its keys and its
behaviour. When the pact ends it reloads from disk, so every `WARLOCK.md` the
run wrote is a row where it was written — with the selection, the collapsed
directories, the filter and the scroll position exactly where they were left.

## Success criteria

- Pressing the pact key fills the panel within a second or two of the first
  model action, and the panel keeps moving for the whole run.
- Every action the model takes appears as one line: the tool and the one
  argument that says what the call was about, or `thinking`. Nothing in the
  panel is a wall of tool output or a paragraph of model prose.
- A stretch of thinking is visibly a stretch of thinking: the newest line's
  elapsed clock advances while nothing else arrives.
- After the run the whole account is still there and can be scrolled through,
  per directory, with nothing discarded. Starting a new pact clears it and
  starts again.
- Each directory's section ends with what happened to it — the document and its
  size, or why it was refused — and the run ends with what it cost.
- When a pact ends, every `WARLOCK.md` it wrote is a row in the tree with no
  keystroke and no relaunch, and the selection, collapsed set, pacted-only
  filter and scroll position are unchanged.
- `Tab` moves focus between the panes, the focused pane is visibly the focused
  one, and the movement keys drive whichever pane has focus.
- Cancelling still works, still kills the running `claude`, and leaves the
  account of what happened up on screen with the cancel recorded in it.
- A repository with no `claude` on `PATH` still launches, still draws, and still
  says so on the footer when the pact key is pressed.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary present.
- `warlock-tui`'s tests still run with no `claude` on `PATH`.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 04 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, **no HTTP
  client and no Anthropic SDK**, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **The engine does not change.** `Agent`, `AgentRequest`, `AgentResponse`,
  `AgentError`, `pact_subtree`, its observer port and the manifest are all as
  they are. Streaming is a fact about how the binary runs `claude`, and the
  engine has no opinion about it. If a slice finds itself editing
  `warlock-engine`, something has been put on the wrong side of the seam.
- **One new dependency is sanctioned, narrowly.** `warlock-tui` may take a JSON
  parser to read the stream. It goes in that crate only, with a line in the
  workspace table saying what it is for. The engine's dependency list does not
  move.
- **No free-text prompt in the panel, ever.** The panel shows work, not
  conversation. The moment there is somewhere to type a message into it,
  warlock is a chat client with a file tree, which is the one thing it must not
  become. This is not a "later" — it is a rule about what the panel is.
- **The panel shows facts, not prose.** Tool uses and thinking, one line each.
  Tool *results* are never read: that is where all the volume is, and a
  megabyte of `Bash` output on screen is neither useful nor survivable.
  `thinking` is the whole line, never its content.
- **Movement, collapse and filter keys keep working during a pact.** WAR-21.05
  decided this deliberately and it is not being revisited. The cursor is not
  frozen and the selection never follows the run.
- **`NodeState` still has exactly three variants.** "Currently being pacted" is
  not a fourth colour, and neither is "streaming".
- The cancel path, the five-minute invocation timeout and the
  `MINIMUM_DOCUMENT_BYTES` rule all survive the change to streaming unchanged,
  including the part where a cancelled pass does not wait on output a surviving
  grandchild is still holding.
- Warlock still does not parse `WARLOCK.md`. It cares that one exists and what
  its bytes hash to.

## Out of scope

**Reading a file's contents in the panel, `WARLOCK.md` included.** This is the
fourth brief in a row to say it and the first where it is genuinely tempting,
because the panel is right there and mostly empty at rest. It stays out. The
questions it opens — what the panel shows when a file is selected mid-run,
whether selecting a file interrupts a live account, what happens to the
document when the directory holding it is being rewritten — are a project's
worth of decisions, and answering them badly while the streaming seam is still
moving is how both get done poorly. **The panel is blank until the first pact
of the session.** No placeholder document, no welcome text, no ASCII art.

**Editing anything.** Section 5 says editing is possible but not the star of
the show; it is not this project's star either.

**Talking to the model.** If it happens at all it is an instruction attached to
a pact, scoped to a directory — not a thread, not a session, not a message box.

**A filesystem watcher.** Files created outside warlock still need a relaunch
after this project. Slice 4 builds the machinery a watcher would trigger, and
building the trigger is the next project's job.

**Flashing the pacting row**, and answering a second pact key press mid-run
with a message rather than silence. Both are tree behaviour and belong with the
rest of it in the next project. The panel is a louder in-flight signal than a
flashing row would be, which is why this one can ship without it.

**Mouse support, indent guides, file-type colour.** Next project, after the
panes have stopped moving — hit-testing a layout that is still being rebuilt
means doing it twice.

**The refresh pass** is still not this project. A directory edited after its
pact still has no way back to green except being pacted again, exactly as brief
04 left it.

Also out, as before: blessing and the gate, pact expansion proposals and
footprint prediction, Linear and Red/Forman integration, the journal at
`~/.warlock/`, the pre-push guard, the CI validator, the paid crate, and
publishing anything.

## Scope

### 1. Two panes, and which one the keys drive

The tree moves to a narrow column on the right and everything it does today
comes with it: the header, the rows, the three colours, the viewport, collapse,
the pacted-only filter, the file toggle. The rest of the width becomes one
panel, and in this slice it is empty — no content, no placeholder, nothing to
draw but its border.

The footer stays full width along the bottom. It is about the run and the
program, not about either pane, and splitting it would give the tally and the
message line a home nobody asked for.

`Tab` moves focus between the panes. The focused pane owns the movement keys
and shows it — a lit border is enough, and it is the only new piece of state
the user has to hold. Nothing else about any key changes: `p` still pacts the
tree's selection whichever pane has focus, `q`, `Esc` and `Ctrl-C` still mean
what they mean.

Pick the split, and pick it as a proportion with a floor rather than a fixed
column count: the tree holds indented paths and stops being readable below
something like thirty columns, and the panel is the point of the project so it
should get the majority. Say in the diff what happens on a terminal too narrow
to honour both, because someone will run this in a split pane.

This slice changes no behaviour beyond layout and focus. A pact before it and a
pact after it do the same things and produce the same files.

### 2. Stream the model pass

depends_on: []

`ClaudeAgent` runs `claude --print` and reads stdout to EOF, so nothing about a
pass is observable until it is over. That is the whole reason a pact looks like
a hang.

It moves to `--print --output-format stream-json --verbose`, which emits one
JSON object per line as the pass happens, and reads those lines as they arrive
rather than draining the pipe and parsing at the end.

Two kinds of event are kept and everything else is ignored:

- a `tool_use` content block becomes one activity carrying the tool's name and
  **one** detail argument, chosen by a whitelist per tool — `file_path` for
  `Read`, `pattern` for `Grep` and `Glob`, `command` for `Bash`, and so on. A
  tool not on the list renders as its bare name. This is `forman.spawn`'s
  `describe_activity` model and it is worth copying rather than reinventing:
  the whitelist is what stops an arbitrary input dict being dumped on someone's
  screen.
- a `thinking` block becomes a bare `thinking`, with no content.

**Tool results are never parsed.** Neither is the model's own text, as
activity. The final result line carries the pass's cost, and that is kept.

Activities are reported through a port on `ClaudeAgent` shaped exactly like
`Cancel`: attached by the caller with a `with_` constructor, doing nothing when
nobody attached one, `Send` because it crosses to the pact worker's thread. The
worker already owns a channel to the event loop; this rides it rather than
inventing a second route.

The document is still whatever the pass finally said, and it must be
byte-identical to what `--print` returns for the same pass today. Decide
whether that comes from the result line's own field or from accumulating the
assistant's text blocks, and say why in the diff. Everything that already
guards it holds: the minimum document length, the timeout, the cancel that
kills the child mid-pass and does not then wait on its output, and the atomic
write.

**An unparseable line is not a failed pass.** A line that is not JSON, or is
JSON of a shape this code does not know, is skipped. Losing a document because
the CLI grew a new event type is a far worse failure than missing one line of
activity, and new event types are certain.

The tests keep their standing property — they run with no `claude` on `PATH` —
which means the shell stand-ins now print a canned event stream instead of a
document. That is a better test seam than it sounds: a stand-in that emits a
tool use, a thinking block and a final text becomes the fixture for every
rendering decision in slice 3.

### 3. Show the pass in the panel

depends_on: [1, 2]

The panel is blank until the first pact of the session. A pact fills it. The
next pact clears it and starts again — one pact, one account of it, left up
afterwards to be read. Not a log across runs: a log is something you have to
search to find the current thing in, and warlock is not a log viewer.

Each directory the pact reaches opens a section, so a subtree pact reads as a
sequence of passes rather than one undifferentiated stream. This is what makes
"wait, what did it say about *that* one" answerable, and it mirrors the walk the
tree is doing beside it.

Under each section, one line per activity, prefixed with the elapsed time it
happened at. The newest line's clock **ticks while it is the newest**. This is
the single most important detail in the slice: a stretch of thinking is one
event followed by a minute of nothing, and without a moving clock that is
indistinguishable from the hang this whole project exists to stop looking like.
Forman learned this in its own progress line and the comment there says so.

Each section ends with what happened to that directory — the document written
and its size, or the reason it was refused — and the run ends with what the
whole pact cost. A cancelled run says it was cancelled, in the section it was
cancelled in, and everything above it stays.

The panel follows the newest line while a run is live and **stops following the
moment the user scrolls up**, showing how far behind they are and offering one
key back to live. Without break-on-scroll, scrolling back during a run is
impossible: the next event yanks the view to the bottom. Scrolling is the
focused pane's movement keys from slice 1, so this adds no keys of its own
beyond the one that returns to live.

Every line is one line. A path or a command longer than the pane is truncated
with an ellipsis, never wrapped into a paragraph — the panel's readability
comes from one action per row, and a wrapped `Bash` command destroys it.

### 4. Reload the tree when a pact ends, and keep the user's place

depends_on: []

A finished pact recolours rows in place and never re-reads the disk. `load_tree`
is called once, at startup, and that is why a document warlock just wrote is
invisible until warlock is restarted.

The tree now reloads from disk when a run ends. Two things about *when* matter
and both are ordering hazards worth stating: the reload happens **after** the
outcome has been applied and the manifest saved, or the new tree is decided
against a manifest that does not yet know what was granted and the whole
subtree comes back gray; and it happens on the event loop's thread like every
other frame, not on the worker.

The work is not the reload. It is keeping the view across it: the selected
node, the set of collapsed directories, the pacted-only filter and the scroll
position all survive being re-seated on a newly loaded tree. A tree that
collapses to the root and throws the selection to the top every time a pact
ends is worse than one that does not update at all, because the user loses
their place as a reward for finishing a run.

Selection and collapse are held by path, not by index — indices move when rows
appear, and rows appearing is the entire point. A path that is no longer in the
tree resolves to the nearest surviving ancestor rather than to the top.

Build this as **"re-seat this view on a new tree"** rather than as "reload after
a pact". Same work either way, but the first is what the next project's
filesystem watcher calls, and the second is what someone writes if they have
only been told about pacts.
