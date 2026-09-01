# A headless CLI for warlock's own operations

Right now `warlock` has exactly two subcommands, `init` and `config`, dispatched by a handful of string comparisons in `crates/warlock-tui/src/main.rs`'s `intention_for` — everything else either opens the TUI (no arguments) or is refused. Every other operation the engine already knows how to do — granting a pact, refreshing a stale subtree, un-pacting, writing or clearing a scope, deciding whether a directory is fresh or stale, and judging whether a held sigil opens a directory's scope — is reachable only by launching the terminal UI and pressing a key. `crates/warlock-engine`'s domain logic is already headless by design (`Cargo.toml` forbids any TUI/terminal/HTTP dependency in that crate specifically so it stays usable without one), but nothing exposes that headlessness at the command line: a script, a CI job, or an agent working in this repository has no way to ask "is this directory stale" or "does this sigil open this scope" without parsing `.warlock/pacts.toml` by hand, and no way to grant a pact or write a scope without a human at a keyboard. The cost of leaving this alone is that every consumer of warlock other than a person watching the tree — including an AI agent drafting a plan against this repository — either goes around the engine's own rules by reading the manifest directly, or cannot use warlock's judgments at all.

Separately, and this is what decides the shape of the new commands rather than merely colouring them: the boundary is already enforced in the TUI. `scope_covering` and `scope_opens_to` in `crates/warlock-engine/src/scope.rs` are wrapped by `closed_scope` in `warlock-tui/src/session.rs`, and every mutating key asks it before acting — `pact_press` at `pacting.rs:882`, `refresh_press` at `pacting.rs:936`, and the scope prompt at `scoping.rs:113`, each of them ahead of the operation rather than after it, so `p`, `r` and `s` are turned down on a directory whose covering scope this machine does not hold. That landed in `8aa050e`. CLAUDE.md's account of the keys is therefore the correct one, and `crates/WARLOCK.md` and the root `WARLOCK.md` — both of which still say that no code anywhere checks a sigil against a scope — are stale on exactly this point and are owed a pass. The question this brief has to answer is consequently not whether the CLI invents a check, but what answer it gives with one it can borrow whole.

## Outcome

Someone at a shell, outside the TUI entirely, runs `warlock check crates/warlock-engine/src` and sees which scope covers that path, if any, and whether the sigils this machine holds open it. They run `warlock stale` and get back the list of pacted directories whose recorded hash no longer matches their contents, one per line. They run `warlock scope add crates/warlock-engine data-plane` and the manifest now says so, without opening the tree. They run `warlock pact crates/warlock-engine` from a terminal with no TUI at all and watch it print, directory by directory, the same account of a model pass the TUI's panel shows today, ending in the same grant a `p` keystroke would have produced. Every one of these, and `refresh` and `unpact` and `scope remove` beside them, is scriptable: exit status says whether it worked, and a `--json` flag on the read commands gives back structured output instead of a sentence.

## Success criteria

**A real argument parser replaces the hand-rolled matching in `main.rs`**
- `intention_for`'s string-comparison approach is gone; a parsing crate is added to `crates/warlock-tui/Cargo.toml` with a comment justifying it, per the workspace's existing rule that every dependency carries one.
- `-h`/`--help` at the top level and per-subcommand continue to work, and an unrecognized subcommand or a malformed argument still fails on stderr with a non-zero exit rather than silently doing nothing.
- `init`, `config`, and opening the TUI with no arguments all still behave exactly as they do today.

**Read-only commands answer without a subprocess or a model pass**
- `warlock stale [path]` and `warlock fresh [path]` each print the pacted directories at or below `path` (repository root if omitted) in the given state, one per line, computed the same way the tree's colouring is computed today.
- `warlock check <path>` prints the scope covering `path` (or that none does), the sigils this machine holds, and whether they open it — built on the existing, already-tested `scope_covering` and `scope_opens_to`, not a reimplementation of either.
- Every read command above accepts `--json` and emits a single parseable structure instead of prose.
- None of the read commands touch the terminal, spawn a process, or write to `.warlock/pacts.toml`.

**Cheap write commands work synchronously, without spawning `claude`**
- `warlock unpact <path>` performs the same manifest-only edit the TUI's `p` on an already-pacted subtree performs, leaving any documents on disk untouched.
- `warlock scope add <path> <scope>` and `warlock scope remove <path>` perform the same synchronous manifest write the TUI's `s` key performs, validated by the same `validate_scope` rules the engine already enforces.
- Each of these checks the covering scope against this machine's held sigils before writing and refuses when they disagree, printing a line naming the scope wanted and exiting non-zero without touching the manifest — the same answer the TUI's `s` and `p`-on-pacted give, in the same words, because it is the same act.

**Model-pass commands run headlessly and report progress on stdout**
- `warlock pact <path>` and `warlock refresh <path>` run the same engine operations (`pact_subtree`, `refresh_subtree`) the TUI's `p` and `r` keys run, through the same `Agent` seam, without opening a terminal UI.
- Progress is printed to stdout as each directory in the subtree is entered and completed, and the command's exit status reflects whether the run completed, was refused, or failed.
- The invocation shape is inherited, not chosen again: `ClaudeAgent::spawn` (`claude.rs:1355`) runs one `claude --print` per `AgentRequest` and `pact_subtree` issues one request per directory, so going through the same seam is already the per-directory shape brief 04 left to be decided and the pact implementation settled. Nothing here re-opens it, and nothing here may quietly change it — the same `Agent`, the same one process per directory, with only the reporting different.
- These two commands perform the same scope check as the cheap write commands, and give the same refusal, before spending a single model pass. A run that would write documents inside a boundary this machine does not hold does not start, exactly as `p` and `r` do not start it.

**Nothing about the TUI's existing behaviour changes**
- `warlock` with no arguments opens the tree exactly as it does today; every key still does what it did before this work.
- The TUI's own boundary refusal is untouched. `closed_scope` and its three callers keep refusing `p`, `r` and `s` in the same words and the same places, and the new commands join them rather than arguing with them — one rule, reached by two doors.

## Constraints

- The dependency edge stays TUI → engine, never back. Every new subcommand's domain behaviour — the scope check, the state queries, the pact and refresh operations — is engine logic called from `warlock-tui`'s binary, not logic written into `main.rs` or a new crate.
- `crates/warlock-engine` gains no terminal, TUI, HTTP, or Anthropic dependency. If anything in the engine needs to change to support headless invocation, it stays within the existing "no unsafe, minimal dependencies, headless and testable without a terminal" rules already governing that crate.
- `claude` remains the only subprocess warlock spawns of its own accord — `$EDITOR`, spawned by `e` in `editing.rs:354`, is the operator's own program and is not one of warlock's — and it remains spawned only from `warlock-tui`, out of `claude.rs`. The headless `pact`/`refresh` commands live in that same crate, not in the engine, and add no second spawning point beside it.
- The manifest format, `.warlock/pacts.toml`, does not change shape. Cheap write commands use the same `Manifest` save path the TUI already uses.
- **A closed boundary refuses a change and warns about a hypothesis, and which one a command is decides its answer.** Every command in this brief that writes — `unpact`, `scope add`, `scope remove`, `pact`, `refresh` — is a literal change to the repository, so every one of them refuses across a scope this machine does not hold, with no `--force` and no override flag beside it. The warning tier exists, but nothing here is in it: a brief, a plan, a set of tickets cut for three teams is a statement about changes that have not happened, and a tool that declined to write one down would be refusing to think. That distinction, and not which input device was used, is what sets the answer — which is why the CLI lands on the TUI's side despite being the scriptable one, and why the drafting feature named in *Out of scope* will land on the other side despite being the one an operator reaches for first.
- The refusal is a speed bump and is meant to be. Sigils are machine-local and self-asserted — `warlock config` writes them, `*` opens everything — so anyone blocked here can hold the scope a minute later. That is the point rather than a hole in it: covering somebody's week is a real thing that happens, and the tool's job is to make an operator say once, deliberately, that they are working inside a boundary that is not theirs, instead of waving them through eight times and never asking.
- `unsafe_code = "deny"`, `clippy::pedantic`, and the rest of the workspace lint table apply to any new code exactly as they apply today; nothing here is exempted.

## Out of scope

- **Any way past the refusal that is not `warlock config`.** No `--force`, no environment variable, no flag that turns the check off for a run. The escape hatch is holding the sigil, which is a thing an operator does once and on purpose; a flag is a thing that ends up in a script and is never read again.
- **Any change to what `refresh` re-grants.** `refresh_subtree` already filters the subtree to the directories that are not fresh and passes over those alone — its own doctest asserts "the changed directory, and not the one below it" — so a headless `refresh` inherits that behaviour exactly and invents nothing finer. (`crates/warlock-engine/WARLOCK.md` still describes this as future work not yet present; it is stale there, and that document is owed a pass too.)
- **`--version`.** Considered and declined for the same reason `main.rs`'s own comments decline it today: warlock isn't installed from a registry, and nobody is diagnosing a version skew yet.
- **Any brief-drafting or conversation-side scope warning** — the idea that started this work, of a brief itself calling out crossed scopes as it is written. This CLI is what such a feature is built on, and the shape it takes is worth recording here so the two are never argued separately: nothing new goes inside warlock for it. The agent drafting the brief runs `check` over the directories the work would touch, the same way any other consumer would, and says what came back. "This spans `control-plane` and `data-plane`, and you hold `control-plane`" is the whole of it.
- **Making that line a refusal.** It warns for a reason that has nothing to do with plans being cheap: the person who implements a ticket is very often not the person who wrote it. A brief spanning three teams is an ordinary brief, and what the line answers is whether the operator could implement all of it themselves — useful to know, and never a reason to stop writing it down. Each ticket is refused or allowed on its own, later, when somebody comes to work it, and that is the gate.
- **Machine-write output formats other than JSON**, such as YAML or a custom line format, for the `--json` flag's commands. One structured format is enough to be scriptable; a second one is a maintenance burden with no named consumer.
- **Revisiting the per-directory invocation shape.** One `claude --print` per directory is what the pact path already does, and the headless commands take it as it stands. Whether a longer-lived session covering a whole subtree would be cheaper is a real question about the pact path itself, asked of both entry points at once and measured rather than argued; it is not something a new front end gets to change on its way past.

## Scope

### 1. A real argument parser

depends_on: []

`intention_for`'s hand-rolled string comparison is replaced with a real parsing crate, wired to the exact behaviour `main.rs` has today: no arguments opens the TUI, `init` and `config` behave unchanged, `-h`/`--help` prints usage and exits zero, anything unrecognized prints on stderr and exits non-zero. No new subcommand does anything yet — this slice only proves the replacement is behaviour-identical for the two subcommands that already exist, so every later slice adds a subcommand to a parser already carrying the crate's justification comment rather than fighting the old matching to make room for one.

### 2. Read-only queries: `stale`, `fresh`, `check`

depends_on: [1]

Three subcommands, none of which write anything or spawn a process. `stale [path]` and `fresh [path]` walk the tree the way the TUI already does and print the pacted directories in the requested state at or below `path`, defaulting to the repository root. `check <path>` calls the existing `scope_covering` and `scope_opens_to` directly — no new matching logic, since both are already correct and tested — and reports the covering scope, the sigils this machine holds, and whether they open it. `--json` on all three is decided here, once, as a shared shape the later write commands' own reporting can follow rather than each inventing its own.

### 3. Cheap writes: `unpact`, `scope add`, `scope remove`

depends_on: [1, 2]

These reuse the manifest-editing logic already behind the TUI's `p`-on-pacted (un-pact) and `s` (scope write) keys, exposed as subcommands that read and write `.warlock/pacts.toml` synchronously with no worker and no subprocess. Each one runs slice 2's `check` logic against the target path before writing and refuses when the sigils held do not open the covering scope, leaving the manifest alone and exiting non-zero. It depends on slice 2 so the question is asked in one place rather than twice, and the answer it gives is the one `closed_scope` already gives ahead of `p`, `r` and `s` — this is the second check-then-refuse site in the codebase, deliberately reading like the first. The engine pair is called directly rather than through `closed_scope` itself, which is `App`-coupled: it takes the selected row, words the refusal onto the app's message line, and has nothing to say to a command with no app under it. What the two share is the rule and the wording, not the function.

### 4. Headless `pact` and `refresh`

depends_on: [1, 3]

The expensive tier. `pact <path>` and `refresh <path>` call `pact_subtree` and `refresh_subtree` through the same `Agent` seam the TUI uses, from a codepath that spawns `claude` without a terminal attached, printing each directory's progress to stdout as it is entered and completed instead of drawing it into a panel. The `Agent` seam is what makes this small: `ClaudeAgent` already spawns one `claude --print` per directory and reports through `Activities`, so what is new here is a printer where the panel used to be, not a second way of running a pass. It depends on slice 3 rather than only on slice 1 because it carries the same check-then-refuse step the cheap writes established, and should read as the same rule applied to a slower operation rather than a second rule invented beside it. The refusal matters more here than anywhere else in the brief: this is the only tier where crossing a boundary spends model time and rewrites somebody else's documents before anyone notices.
