# Scopes carry the team, the machine carries the key

A brief written in warlock's panel ends as a markdown file in the brief directory `briefs.rs` reads from `.warlock/briefs.toml`, and stops there. Everything after that is done by hand: somebody opens Linear, picks a team, picks the lane a finished ticket lands in, remembers which label the tooling watches for, and retypes the brief into an Epic. The tool that knows which part of the repository the work sits in — `.warlock/pacts.toml` already writes `scope = "control-plane"` on a pact, and `scope_covering` in `crates/warlock-engine/src/scope.rs` already answers which scope covers a path — is not consulted at the moment it would matter most.

The obvious fix is the one this repository cannot have. Forman keeps its API key, team key, review state and label in a single `~/.config/forman/.env`, and switching which set is live means editing that file: comment out one key, comment in another. Warlock runs across every repository on a machine. One flat file means a side project shares a company's credentials, and the way you change teams is a text edit you make while thinking about something else. The failure is silent and it is the expensive kind: the upload succeeds, the ticket lands on the wrong board, in someone else's workspace, and you find out from them.

There is also a second, quieter cost. A scope today is a string that gates warlock's own mutating keys and says nothing else. The fact a team already asserts by writing `control-plane` on a directory — that this part of the repository is ours, and our work is tracked over there — has nowhere to live, so every tool downstream of warlock has to be told it again.

## Outcome

Somebody clones a repository they have never worked in and runs `warlock check crates/engine`. It tells them what it tells them today — the scope covering that path, the sigils this machine holds, whether the one opens the other — and then two lines it could not print before: that work under `control-plane` is filed to team `CRT`, moves to `In Review` when it is done, and carries the `warlock` label; and that this checkout has no Linear key bound, so nothing can be filed yet. The last line names the two ways out: bind a key that already exists on this machine, or add a new one.

They run `warlock key list`. Two names come back, `acme` and `friends`, with no secret beside either. This is the friends repository, so they run `warlock key use friends`, and `warlock check crates/engine` now says the route and the key name together. They open their other checkout, the company one, in a second terminal and run the same command there: same machine, same binary, `acme`, because the binding is a fact about the checkout and not about the session. Nothing was commented in or out to get from one to the other.

They run `warlock check docs/notes`, a directory no scope covers, and get a refusal saying there is nothing to route to and that a scope is what would fix it. Then they run `warlock check crates/plane`, which is scoped `data-plane` — a team they are not on and hold no sigil for. It prints that team's route in full, says the scope does not open to them, and exits 0 anyway.

## Success criteria

**The scope record**

- `.warlock/pacts.toml` accepts `[[scope]]` tables holding a name, a team slug, a review state and a label, separate from the `[[pact]]` rows, which keep naming a scope with the string they write today.
- A `[[scope]]` name is judged by the existing `validate_scope`, and its bytes are never folded or normalised on the way through, exactly as `PactEntry`'s `scope` is not.
- Every existing writer of the manifest round-trips the new tables untouched: pacting, un-pacting, granting and `warlock scope` each leave a `[[scope]]` record they did not mean to change byte-identical. A test pacts and un-pacts a directory in a repository holding scope records and asserts the file back.
- A `[[scope]]` naming a scope no pact uses is kept, not pruned. A scope used by a pact with no record is legal and routes to nothing.
- A malformed or duplicated record is named as a failure with its file and key, and never read as absent.

**Named keys on the machine**

- `~/.warlock/keys.toml` maps names to Linear API keys, and is the only place a secret is written.
- A project's existing sigil config — `~/.warlock/<name>-<digest>/config.toml`, at `sigils_path` — gains a `key` naming one of them. An existing config with no `key` loads as it does today and reads as unbound.
- `warlock key add <name>`, `warlock key list`, `warlock key use <name>` and `warlock key forget <name>` exist and are documented in `HEADLESS-CLI.md` with the rest of the headless surface.
- `key list` prints names only. No command prints a key, and no `--json` envelope carries one.
- `keys.toml` is written through `write_and_sync` like every other file this crate saves, and created with owner-only permissions.
- The home directory is a parameter through the new module, never read from the environment inside it, following the rule written at the top of `sigils.rs`.

**Resolving a route**

- Given a path, warlock answers with the covering scope, that scope's record, the key name bound to this checkout and whether that name resolves to a key — through one engine function, not re-derived per caller, for the reason `check.rs` gives for leaning on `scope_covering`.
- No scope covering the path is a refusal naming the path and saying a scope is what is missing.
- A scope with no record, or a checkout with no key bound, or a bound name absent from `keys.toml`, are three distinct refusals, each naming which one it is.
- A scope this machine holds no sigil for resolves normally. Whether the sigil opens the scope comes back beside the route as a separate fact and never as a failure.

**`warlock check`**

- The prose gains the team slug, review state and label of the covering scope, and the bound key name, and stays one answer rather than four things printed as they are worked out.
- The `--json` envelope gains the same fields alongside `scope`, `sigils` and `opens`, and existing keys keep their spelling and meaning.
- A closed scope, an unbound checkout and an unscoped path each keep exit status 0 in `check`, which reports rather than gates.

## Constraints

- No Linear API client, no HTTP and no new network dependency. This change ends at configuration that can be read and printed.
- Warlock has one home, `~/.warlock`. Nothing is read from or written to `~/.config`.
- Credentials are never committed and never printed. A key reaches disk only in `keys.toml`, and an error that carries one is a bug.
- The scope vocabulary does not change: `validate_scope`, its 24-character ceiling and the single `RULES` line stay the one judgement, and a record's name is judged by it rather than by a second rule.
- `.warlock/pacts.toml` is committed and read by people. A save must not reorder, reformat or drop what it did not set out to change.
- `scope_covering` and `scope_opens_to` stay the only implementation of the boundary rule. Routing walks no manifest of its own.
- Existing command spellings, their `--json` envelopes and their exit statuses do not move.
- The comment rules in `CLAUDE.md` apply. A comment earns its place by naming a rejected alternative or a constraint the code cannot show — why the home is a parameter, why a record survives a save it was not part of.
- The repository root carries the `warlock-team` scope in `.warlock/pacts.toml`, so every part of this work is under it. Confirm what this machine holds with `warlock config` before starting.

## Out of scope

- **Uploading a brief to Linear.** It is the next brief and it is the point of this one, but an Epic created against an API whose routing is still being argued about is written twice. This brief ends where a route can be printed and checked.
- **Pulling tickets and moving their state.** That is one machine changing state other people can see, and it is where a hard block belongs rather than a warning. Deciding that here would settle it before the case exists.
- **Blocking work that crosses a scope this machine does not hold.** A manager scoping out work across several teams, or an engineer covering for one who is away, is ordinary and must not be bricked. Warlock says what is being crossed and continues.
- **Committing the key name in the repository.** A name out of somebody's personal configuration in shared history forces everyone onto that name and says nothing true about a fork with its own workspace. The binding stays on the machine.
- **More than one key per checkout.** A person is on one team in a project, whatever the number of teams working in it. Filing for another team is the warn case, not a second credential.
- **A default key for an unbound checkout.** A default is the exact mechanism that files an `acme` ticket from the friends repository. Refusing costs one command, once, per checkout.
- **Migrating existing sigil configs.** There is nothing to migrate: a config with no `key` is unbound, which is already the refusing state.

## Scope

### 1. The scope record in the manifest

depends_on: []

Add the `[[scope]]` table to the manifest type in `crates/warlock-engine/src/manifest.rs`, with its name judged by `validate_scope` and its three fields read as written. The decision this slice makes is that a scope becomes a record with an identity of its own rather than a field on a pact — which is what lets a scope exist before anything is pacted under it, and lets a record outlive the pact that used to name it. It also owns the round-trip: every existing writer already rewrites the whole file, so the tables have to survive a save nobody aimed at them, and a test that pacts, un-pacts and grants in a repository holding records is how that is held. The dropped-scope defect this repository has already seen once is the same shape of mistake.

### 2. Named keys on the machine

depends_on: []

A new module beside `sigils.rs` reading and writing `~/.warlock/keys.toml`, with the home as a parameter and `write_and_sync` doing the writing. The binding goes in the project's existing `config.toml` next to the sigils, because that file is already the answer to "what does this machine say about this checkout", and a second per-project file would be a second place to look. Absent key, absent file and unreadable file stay three different answers, following `load_sigils`.

### 3. Resolving a route

depends_on: [1, 2]

One engine function from a path to a route: the covering scope, its record, the key name, whether that name resolves. This slice decides that the refusals are distinct and named — unscoped, unrecorded, unbound, dangling — because a single "cannot route" would send somebody editing the wrong file. It also decides that the sigil question travels beside the route rather than gating it, which is what the warn-don't-block decision amounts to in code.

### 4. The `warlock key` command

depends_on: [2]

`add`, `list`, `use` and `forget`, in `crates/warlock-tui`, alongside the existing `check`, `scope` and `config`. The decision is that `list` prints names and never secrets, in prose and in `--json` alike, so no shell history, screen share or CI log ever holds one.

### 5. `warlock check` says where a ticket would go

depends_on: [3]

Extend `check.rs` with the route, in both renderings, keeping the single `Checked` value that makes the prose and the object two views of one answer. The decision here is that the existing subcommand grows rather than a `warlock route` appearing beside it: the question "what covers this path and does it open to me" and the question "where would its ticket go" are the same question asked by the same person at the same moment, and splitting them would give a script two commands to call and a chance to see them disagree.
