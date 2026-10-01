# Rename the scope record's `team` to `team_key` and fix the push refusal

A `[[scope]]` record in `.warlock/pacts.toml` carries a field spelled `team`, and a person reading it has no way to know what belongs there. The value is a Linear team key — `WAR` — because `linear.rs:271 team_id` resolves it with `teams(filter: { key: { eq: $key } })`. The word `team` invites a team name instead, and the repository's own examples confirm the misreading: the doctests at `route.rs:24` and `filing.rs:28` both use `"Data Plane"`, a value no workspace would ever resolve. The same word spreads through `Destination::team` at `filing.rs:249`, `FiledRecord::team` at `filed.rs:345`, the `--team` flag in `main.rs`, `RecordFields::team` and `RecordField::Team` in `rescope.rs`, and the prompt and refusal sentences a person meets when creating a scope. Left alone, every new surface that touches a board copies the bad word, and somebody eventually types a team name into a field that wants a key and finds out when a push fails against a live workspace.

Separately, `warlock push` gives wrong advice to a repository that records no scope at all. `filing.rs:154 no_candidate` asks `held.is_empty()` first, so a machine holding no sigil is told "this machine holds no sigil, so nothing says which board to file to: hold one with `warlock config`". Holding a sigil would change nothing: `resolve_filing` builds its candidates from `manifest.scopes()` alone, so with no `[[scope]]` record there is nowhere for a project to go whatever this machine holds. The person follows the instruction, records a sigil, pushes again, and fails again — this time with a different sentence. The fix the error should have named is in the repository, not on the machine.

## Outcome

Somebody opens `.warlock/pacts.toml`, sees `team = "WAR"` under a `[[scope]]`, and finds the field is read in the code as `team_key` and asked for on the command line as `--team-key`. The TUI's scope prompt asks for a team key. `warlock scope add` with the flag missing says so using the new flag name. Nothing in any existing checkout had to be edited for this: `.warlock/pacts.toml` and `.warlock/filed.toml` still hold the key `team`, and warlock reads them exactly as before.

In a repository with no `[[scope]]` record, `warlock push docs/brief.md` prints one sentence naming `.warlock/pacts.toml` and saying it records no scope, so there is nowhere to file. It does not mention sigils. Once a record exists, the next push is the one that talks about what this machine holds.

## Success criteria

**The refusal when a repository records no scope**

- `resolve_filing` against a manifest with an empty `scopes()` returns a variant naming the manifest path and the absent `[[scope]]` record, whether or not the machine holds a sigil.
- That sentence names no sigil and does not mention `warlock config`.
- `Error::Unsigiled` still fires when the repository records at least one scope and the machine holds nothing.
- `Unmatched` and `Unrecorded` are unchanged in both condition and wording.

**The record field**

- `ScopeRecord`, `Destination` and `FiledRecord` each spell the field `team_key`, with accessors and constructor parameters to match.
- `ScopeRecord` and `FiledRecord` carry `#[serde(rename = "team")]` on that field, and a `.warlock/pacts.toml` or `.warlock/filed.toml` written before this change round-trips byte-identically after it.
- A comment on each renamed serde field records that the TOML key is committed bytes and cannot move.
- Nothing in `linear.rs` is renamed.

**The flag**

- `warlock scope add` takes `--team-key`; `--team` is not accepted and has no alias.
- `RecordFields::team` and `RecordField::Team` are spelled for the key, and the `NeedsRecord` and `BlankRecord` refusals name `--team-key`.
- `HEADLESS-CLI.md` uses the new flag wherever it documents the command.

**The prose and the examples**

- The TUI scope prompt asks for a team key.
- The doctests in `route.rs` and `filing.rs` use a team key shaped like one Linear would answer to, not a team name.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass.

## Constraints

- The TOML keys `team` in `.warlock/pacts.toml` and `.warlock/filed.toml` do not change. They are committed, hand-edited bytes under `#[serde(deny_unknown_fields)]`, and moving them would refuse every manifest in existence on read.
- `linear.rs` keeps the word `team` throughout. Its parameters are a mix: `team_id` takes a key, while `workflow_state` filters `team: { id: { eq: $team } }` and holds an id. A sweep through that file would rename ids into key-sounding names, and the module speaks Linear's vocabulary, which is `team`.
- The record's values stay unjudged and unnormalised. What a team key, a review state or a label may contain is the tracker's business, as `manifest.rs:497` and `rescope.rs:137` already decide.
- `no_candidate` keeps one sentence per absence. It does not grow a variant that names two fixes at once, and the existing three sentences keep their wording.
- The staged ordering in `resolve_filing` stands: board refusals are decided before the key store is touched, for the reason given at `filing.rs:93`.
- No key value reaches an error, a `Debug` or a printed line. `Target`'s redacting `Debug` and `Destination`'s name-only key stay as they are.

## Out of scope

- A migration or a dual-read that accepts `team_key` as a TOML key. There is one spelling on disk and adding a second is how a format acquires two ways to say one thing; the serde rename already buys everything a migration would.
- A `--team` alias for the new flag. One person runs this tool, the old flag has no callers to protect, and an alias is a second name to document and keep working forever.
- Renaming `scope` itself, or any other field of `ScopeRecord`. `review_state` and `label` say what they hold; only `team` was ambiguous.
- A `warlock scope` subcommand that lists or validates records against a live workspace. Catching a bad team key before a push is worth doing and is a different change, with a network call and a cache behind it.
- Any change to what `warlock check` prints beyond the renamed word. The three-line answer is settled.
- The explicit document about scope records. That is being written by hand, separately.

## Scope

### 1. The push refusal names the repository before the machine

depends_on: []

Lands first and alone, because it shares no reasoning with the rename and nothing in it waits on a renamed field. `no_candidate` gains a test on `manifest.scopes().is_empty()` ahead of `held.is_empty()`, returning a new variant that names the manifest path. The condition is `scopes()` and not "some pact carries a scope": `resolve_filing` draws its candidates from the records alone, so a repository with records and no scoped pact can still file and must not be told otherwise. The sentence says nothing about sigils, because a person facing both absences has only one fix that moves them forward, and the next push after they add a record is where the sigil comes up.

### 2. The three record types spell the field `team_key`

depends_on: []

The field, its accessors and its constructor parameters move together across `ScopeRecord`, `Destination` and `FiledRecord`. All three hold the same value, and renaming one would leave a `Destination::team()` feeding a `team_key` into the board, which reads as two different things. `#[serde(rename = "team")]` keeps the committed bytes where they are, and a comment carries the reason, since nothing in the code otherwise explains why the field and the key disagree. This slice is also where the fixtures stop lying: `"Data Plane"` becomes a value shaped like a Linear team key.

### 3. The flag becomes `--team-key`

depends_on: [2]

After the field, so the plumbing from `main.rs` through `RecordFields` and `RecordField` moves once rather than twice. The flag, its value name, its doc comment and the `NeedsRecord` and `BlankRecord` refusals that quote it all change together — a refusal naming a flag that no longer exists is worse than the vague word it replaced. `HEADLESS-CLI.md` is part of this slice and not a follow-up, because a document naming the removed flag is a broken instruction for the one caller that reads it.

### 4. The prompt and the help text say team key

depends_on: [2]

The surfaces a person reads without reading code: the TUI scope prompt's label and whatever `--help` text describes the field. Split from slice 3 because it lands through the panel rather than the CLI and can be judged on its own; it depends on slice 2 only so that the words and the field agree in a single commit range rather than disagreeing in the middle of one.
