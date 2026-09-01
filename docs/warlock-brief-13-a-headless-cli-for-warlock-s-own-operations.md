# A headless CLI for warlock's own operations

Right now `warlock` has exactly two subcommands, `init` and `config`, dispatched by a handful of string comparisons in `crates/warlock-tui/src/main.rs`'s `intention_for` — everything else either opens the TUI (no arguments) or is refused. Every other operation the engine already knows how to do — granting a pact, refreshing a stale subtree, un-pacting, writing or clearing a scope, deciding whether a directory is fresh or stale, and judging whether a held sigil opens a directory's scope — is reachable only by launching the terminal UI and pressing a key. `crates/warlock-engine`'s domain logic is already headless by design (`Cargo.toml` forbids any TUI/terminal/HTTP dependency in that crate specifically so it stays usable without one), but nothing exposes that headlessness at the command line: a script, a CI job, or an agent working in this repository has no way to ask "is this directory stale" or "does this sigil open this scope" without parsing `.warlock/pacts.toml` by hand, and no way to grant a pact or write a scope without a human at a keyboard. The cost of leaving this alone is that every consumer of warlock other than a person watching the tree — including an AI agent drafting a plan against this repository — either goes around the engine's own rules by reading the manifest directly, or cannot use warlock's judgments at all.

Separately, the scope/sigil matching itself — `scope_covering` and `scope_opens_to` in `crates/warlock-engine/src/scope.rs` — is implemented and tested, and is used exactly once, inside `warlock-tui/src/session.rs`. Nothing calls that pair before `p`, `r`, or `s` act, so none of warlock's mutating operations — in the TUI today, and in this new CLI unless the work says otherwise — actually check a held sigil against a directory's scope before writing. CLAUDE.md describes `p`, `r`, and `s` as already refusing across a closed boundary; `crates/WARLOCK.md` and `crates/warlock-tui/WARLOCK.md` both say plainly that no enforcement exists anywhere in the tree yet, and that is the account to trust.

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
- Each of these checks the covering scope against this machine's held sigils before writing, prints a warning naming the scope wanted when they disagree, and performs the write anyway — warn, never block, matching the posture CLAUDE.md already states for scoped work.

**Model-pass commands run headlessly and report progress on stdout**
- `warlock pact <path>` and `warlock refresh <path>` run the same engine operations (`pact_subtree`, `refresh_subtree`) the TUI's `p` and `r` keys run, through the same `Agent` seam, without opening a terminal UI.
- Progress is printed to stdout as each directory in the subtree is entered and completed, and the command's exit status reflects whether the run completed, was refused, or failed.
- The chosen invocation shape — a `claude` subprocess spawned fresh per directory, or one longer-lived session covering the whole subtree — is stated explicitly in the implementation and the reasoning recorded beside it, closing the question brief 04 left open rather than defaulting silently.
- These two commands perform the same scope check as the cheap write commands, with the same warn-and-proceed behavior, before starting the run.

**Nothing about the TUI's existing behaviour changes**
- `warlock` with no arguments opens the tree exactly as it does today; every key still does what it did before this work.
- No scope check is added to the TUI's own `p`, `r`, or `s` handling as part of this work — the gap CLAUDE.md describes and the engine's own documents deny exists is left exactly as wide as it is today, for the TUI specifically.

## Constraints

- The dependency edge stays TUI → engine, never back. Every new subcommand's domain behaviour — the scope check, the state queries, the pact and refresh operations — is engine logic called from `warlock-tui`'s binary, not logic written into `main.rs` or a new crate.
- `crates/warlock-engine` gains no terminal, TUI, HTTP, or Anthropic dependency. If anything in the engine needs to change to support headless invocation, it stays within the existing "no unsafe, minimal dependencies, headless and testable without a terminal" rules already governing that crate.
- `claude` remains the only subprocess warlock ever spawns, and it remains spawned only from `warlock-tui` — the headless `pact`/`refresh` commands live in the same crate as the TUI's own spawning code, not in the engine.
- The manifest format, `.warlock/pacts.toml`, does not change shape. Cheap write commands use the same `Manifest` save path the TUI already uses.
- No scope/sigil check becomes a hard failure anywhere in this work. Every check this brief adds warns and proceeds, consistent with CLAUDE.md's stated posture that warlock's guardrails stop a fumbled keystroke, not a determined person.
- `unsafe_code = "deny"`, `clippy::pedantic`, and the rest of the workspace lint table apply to any new code exactly as they apply today; nothing here is exempted.

## Out of scope

- **Enforcement on the TUI's own `p`, `r`, and `s` keys.** CLAUDE.md describes them as already refusing across a closed boundary; they do not. Wiring `scope_covering`/`scope_opens_to` into the TUI's own keystrokes is a real gap but a different piece of work from giving the CLI a check the TUI still lacks, and doing both at once risks neither landing cleanly.
- **A refresh pass that re-grants only what deserves it, for anything short of a full subtree.** `crates/warlock-engine/WARLOCK.md` already names this as future work; a headless `refresh` command runs the existing `refresh_subtree` operation and inherits its current scope, not a finer-grained one.
- **`--version`.** Considered and declined for the same reason `main.rs`'s own comments decline it today: warlock isn't installed from a registry, and nobody is diagnosing a version skew yet.
- **Any brief-drafting or conversation-side scope warning** — the idea that started this work, of a brief itself calling out crossed scopes as it is written. This CLI is what such a feature would eventually be built on, but building the primitive is not building the feature, and the feature is not part of this work.
- **Machine-write output formats other than JSON**, such as YAML or a custom line format, for the `--json` flag's commands. One structured format is enough to be scriptable; a second one is a maintenance burden with no named consumer.
- **Which invocation shape `pact`/`refresh` use is decided inside this brief's implementation, not here.** Both a per-directory subprocess and one longer session satisfy every success criterion above; the choice and its reasoning are implementation work this document deliberately leaves open rather than prejudging.

## Scope

### 1. A real argument parser

depends_on: []

`intention_for`'s hand-rolled string comparison is replaced with a real parsing crate, wired to the exact behaviour `main.rs` has today: no arguments opens the TUI, `init` and `config` behave unchanged, `-h`/`--help` prints usage and exits zero, anything unrecognized prints on stderr and exits non-zero. No new subcommand does anything yet — this slice only proves the replacement is behaviour-identical for the two subcommands that already exist, so every later slice adds a subcommand to a parser already carrying the crate's justification comment rather than fighting the old matching to make room for one.

### 2. Read-only queries: `stale`, `fresh`, `check`

depends_on: [1]

Three subcommands, none of which write anything or spawn a process. `stale [path]` and `fresh [path]` walk the tree the way the TUI already does and print the pacted directories in the requested state at or below `path`, defaulting to the repository root. `check <path>` calls the existing `scope_covering` and `scope_opens_to` directly — no new matching logic, since both are already correct and tested — and reports the covering scope, the sigils this machine holds, and whether they open it. `--json` on all three is decided here, once, as a shared shape the later write commands' own reporting can follow rather than each inventing its own.

### 3. Cheap writes: `unpact`, `scope add`, `scope remove`

depends_on: [1, 2]

These reuse the manifest-editing logic already behind the TUI's `p`-on-pacted (un-pact) and `s` (scope write) keys, exposed as subcommands that read and write `.warlock/pacts.toml` synchronously with no worker and no subprocess. Each one runs slice 2's `check` logic against the target path before writing and prints its warning when the sigils held do not open the covering scope, then performs the write regardless — the first place in the whole codebase where the scope check and a mutation sit next to each other, which is why it depends on the check existing first rather than duplicating it.

### 4. Headless `pact` and `refresh`

depends_on: [1, 3]

The expensive tier. `pact <path>` and `refresh <path>` call `pact_subtree` and `refresh_subtree` through the same `Agent` seam the TUI uses, from a codepath that spawns `claude` without a terminal attached, printing each directory's progress to stdout as it is entered and completed instead of drawing it into a panel. This slice is where the per-directory-subprocess-versus-one-session question left open since brief 04 gets answered, in the implementation and in a comment beside it, because it cannot be avoided here the way it could be avoided everywhere else. It depends on slice 3 rather than only on slice 1 because it carries the same scope-check-then-warn-then-proceed step the cheap writes established, and should read as the same rule applied to a slower operation rather than a second rule invented beside it.
