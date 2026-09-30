# Give the headless CLI a voice: questions in draft, and a `warlock brief` session

The panel and the shell are not the same product, and the gap is that the shell cannot be spoken to. `warlock draft <PATH>` opens each slice's session with `Drafting::one_shot` (`crates/warlock-tui/src/planned.rs`, in `drafted`), which tells the model up front that it may not ask a question and holds it to zero rounds — the reason written in the doc comment is "there is nobody at a shell to put a question to". So a slice whose prose is ambiguous gets a ticket drafted on a guess, or gets reported unusable, and either way nobody was asked. The `Reply::Question` arm in that same file exists only to say the words "the session asked a question and there is nobody to answer it". The panel's `/draft` (`crates/warlock-tui/src/cutting.rs`) has the whole loop the shell is missing: the session asks, warlock proposes an answer, the reader accepts it or types over it, and after the drafts land there is a review where the reader accepts them, skips the slice, or types feedback and has the slice drafted again.

The second half of the gap is that `/brief` has no headless twin at all. Brief mode is a register on the panel (`crates/warlock-tui/src/chatting.rs`): it reads the repository's brief template and `briefs.toml`, sends warlock's own instruction paragraph, and keeps a conversation going until `/write` proposes a path and `crates/warlock-tui/src/writing.rs` puts the document on disk. None of that is reachable from a shell. To argue a brief, you must be in the TUI.

What it costs to leave alone: every brief in this repository starts in the panel whether or not the panel is where the work is, and every headless cut is drafted by a model that was forbidden to ask about the one slice it did not understand.

## Outcome

A reader at a local terminal runs `warlock draft docs/warlock-brief-24-headless-voice.md`, and on the third slice the run stops and asks:

```
warlock: [3/5] slice 3 `Read a line from stdin` — drafting
warlock: slice 3 `Read a line from stdin` asks: should the prompt echo what was typed when stdin is not a terminal?
warlock: warlock's answer: No — `key.rs` already reads `is_terminal` for this and the same rule applies.
>
```

Enter sends warlock's answer. Typing something else sends that instead. The run waits there for as long as it takes. When the drafts come back the run stops again and shows them:

```
warlock: slice 3 `Read a line from stdin` — 2 drafts
warlock:   1. Read a line from stdin at the drafting prompt
warlock:   2. Wait rather than fall back when nothing answers
> accept / skip / feedback
```

`accept` files them, `skip` leaves the slice uncut and moves on, and anything else is read as feedback and the slice is drafted again.

Separately, the same reader runs `warlock brief` and argues a change at the shell:

```
$ warlock brief
warlock: brief mode — this conversation is converging on a document. /write writes it.
> headless draft can't ask questions, and there's no brief command at the shell
> both are about the CLI having no way to talk back
>
warlock: Those are two changes sharing one mechanism. Before I take them as one …
> ...
> /write
docs/warlock-brief-24-headless-voice.md
> 
warlock: wrote docs/warlock-brief-24-headless-voice.md
$
```

Lines accumulate until an empty line sends them as one turn. `/write` prints the path warlock proposes; Enter accepts it and any other text replaces it. The file lands and the command exits.

## Success criteria

**Questions reach the shell in `warlock draft`**

- `planned.rs`'s `drafted` opens each slice's session with `Drafting::for_slice` rather than `Drafting::one_shot`.
- A `Reply::Question` prints the question and warlock's own proposed answer, then reads a line from stdin.
- An empty line sends warlock's proposed answer; a non-empty line sends that text instead.
- A session with no proposal to offer says so and reads a line anyway.
- The read blocks until a line arrives.
- A question relayed after the session's rounds are spent leaves the slice uncut with a line saying why.
- The stdin read is behind a seam a test can drive, so no test in the crate reads the developer's real terminal.

**Drafts are reviewed before they are filed**

- After a slice's drafts come back, the run prints them and reads a line before anything is sent to the board.
- `accept` files the drafts.
- `skip` leaves the slice uncut, says so, and the run goes on to the next slice.
- Any other text is sent to the session as feedback and the slice is drafted again, with the new drafts reviewed the same way.
- A skipped slice writes no cut record, so a later run reaches it again.
- The review runs before `Planned::filing`, so a skip costs no request.

**`warlock brief` holds a conversation and writes a document**

- `warlock brief` is a subcommand with no required argument.
- It reads the repository's brief template and `briefs.toml` before the first turn, and a file that will not read is a refusal with nothing sent.
- The first turn carries the same instruction paragraph `/brief` sends, built from the same template.
- Typed lines accumulate and an empty line sends them as one turn.
- The model's reply is printed in full.
- `/write` prints the path `writing.rs`'s proposal rule produces for the answer just given; Enter accepts it, other text replaces it.
- A path that already has a file is refused and the prompt comes back.
- A successful write prints the path written and the command exits zero.
- A reply missing a section of the template's shape is refused by the same `missing_sections` check `/write` uses, and nothing is written.
- Nothing but prose and `/write` is understood at the prompt.

**The documentation matches**

- `HEADLESS-CLI.md`'s section saying the drafting session is held to no rounds is rewritten to describe what it now does.
- `warlock brief` appears in the headless documentation with the blank-line rule and `/write` stated.

## Constraints

- The drafting session type is not changed. `Drafting::for_slice` exists with its own contract and round count; this work calls it and does not rewrite it.
- The panel's `/draft` and `/brief` keep their current behaviour. The two doors converge on the same session types, and neither is re-implemented for the other.
- A question, a proposal and a slice are named in the words `planned.rs::named` already produces, so a line about a slice reads the same whichever door printed it.
- Nothing about a brief is worded twice: the shape check is `missing_sections`, the path proposal is `writing.rs`'s, the instruction is `brief_instruction`, and the directory is `briefs.toml`'s.
- The brief conversation is one agent and one session for the whole run. Per-turn sessions would lose the argument.
- No key value is printed on any path this work adds, and the redacting `Debug` on `Planned`, `Filing` and `Announcement` stays.
- Every refusal a run can make before spending a request is still made before it. A review that skips a slice sends nothing.
- Stdin is read through a seam. No test in the crate may block on a real terminal, and the whole of both features must be drivable against a scripted stand-in with no `claude` on the machine.
- Naming follows the settled split: what a reader types and reads says *brief* and *draft*, and Rust identifiers for the cutting flow say *cut*. Nothing new in this flow is named `draft*` internally.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass.

## Out of scope

- **Resuming an interrupted `warlock brief`.** A Ctrl-C loses the conversation and you start over. Saving a session id and picking it back up is a second mechanism with its own store, and shipping it alongside a session that does not yet exist would be guessing at how it fails.
- **A fallback when nobody answers.** The prompt waits. Deciding what an unattended run should do — skip the slice, or take warlock's own answer — is a question about a use case that has not arrived, and inventing an answer now would bake it in.
- **`/push`, `/draft` and `/chat` inside `warlock brief`.** `warlock push` and `warlock draft` are already subcommands, so the shell composes them; adding them to the prompt would put the same verb in a third place to drift. And `warlock brief` cannot leave brief mode because it *is* brief mode.
- **Multi-line turns containing a blank line.** The blank line is the terminator, so a two-paragraph turn is not expressible. This is accepted for now on the grounds that it is cheap to change to a different terminator later if it bites.
- **A `--json` output for either command.** `warlock draft` already declines one on the grounds that the answer worth parsing is the record, and a conversation has no record to hand back.
- **Changing what the model is asked or how a draft is validated.** The contracts, the repair passes and `ATTEMPTS` are settled elsewhere and this work routes their answers to a person rather than altering them.

## Scope

### 1. A stdin prompt the headless verbs share
depends_on: []

One place that prints a line and reads a line back, behind a trait or a small type so a test hands in a scripted stand-in. It decides the seam: reading stdin directly in `planned.rs` would make every assertion about the asking a test that blocks on a terminal, and the crate's rule is that no test reaches the developer's real machine. It also decides how echo is handled when stdin is not a terminal, following `key.rs`, which already asks `is_terminal` for the same reason. Nothing uses it yet.

### 2. `warlock draft` relays the session's questions
depends_on: [1]

`drafted` opens with `Drafting::for_slice`, and the `Reply::Question` arm stops being an unreachable apology: it prints the question and warlock's own proposed answer, reads a line, and sends the line or the proposal. This slice decides that the proposal is warlock's and the decision is the reader's — the panel already works this way, and a prompt that only asked, with nothing offered, would make every question a research task. It also decides that the read blocks, which is what makes the spent-rounds case the only way a question leaves a slice uncut.

### 3. `warlock draft` reviews the drafts before filing
depends_on: [2]

Between the drafts coming back and `Planned::filing`, the run prints the drafts and reads a line: `accept`, `skip`, or feedback. This slice decides that feedback is the default reading of anything unrecognised, so a reader who types a sentence gets a redraft rather than a usage message, and that a skip writes no cut record, so the slice is still there for the next run. It sits after slice 2 because the feedback path sends another turn to the same session, which is only worth wiring once the session is one that takes turns.

### 4. A `warlock brief` session at the shell
depends_on: [1]

The subcommand: read the template and `briefs.toml`, refuse if either will not read, open one agent and one session, send the instruction paragraph, then loop — lines accumulate, an empty line sends the turn, the reply is printed. `/write` proposes a path through `writing.rs`'s rule, reads a line to accept or replace it, checks the reply's sections with `missing_sections`, writes, prints the path and exits. This slice decides that the session is one-purpose and cannot leave brief mode, which is what keeps the prompt's whole vocabulary to one word.

### 5. The headless documentation says what the CLI now does
depends_on: [2, 3, 4]

`HEADLESS-CLI.md`'s "held to no rounds" section is rewritten, and `warlock brief` is documented with the blank-line rule and `/write`. Last because it describes three behaviours that must exist first, and a document written against a plan rather than against a run is the kind of stale paragraph this repository spends money on.
