# Record a tracker on each scope and ask which one when a scope is made

Every scope in this repository files to Linear, and nothing in the repository says so. `ScopeRecord` in `crates/warlock-engine/src/manifest.rs` holds three values — `team` on the wire, read into a field called `team_key`, plus `review_state` and `label` — and the only board the code can open is Linear's: `Opens::open` in `crates/warlock-tui/src/linear.rs` takes a key and returns a `Linear<Client>`. `push.rs`, `cut.rs`, `planned.rs`, `pulling.rs`, `puller.rs`, `queue.rs` and `filing.rs` all read `record.team_key()` and hand it to a Linear team lookup, a Linear project filter or a Linear issue filter. The `s` key's record form in `crates/warlock-tui/src/prompt.rs` asks for exactly those three fields in that order, with no question ahead of them; `warlock scope add` takes `--team-key`, `--review-state` and `--label`.

So a team that files work to Jira cannot record a scope at all. There is no field for a Jira site, no field for a project key, and nowhere in the manifest where "which tracker is this?" could be answered — the record's shape *is* the answer, implicitly, in every file that reads it.

Leaving it alone costs more than a missing feature. The Jira board is the next piece of work, and with the record flat and Linear-shaped there is no list of the places that have to decide: a Jira record would reach `push.rs` carrying a project key in a field named `team_key`, resolve nothing, and fail at a Linear API call with a message about a team that does not exist. The refusal has to be structural before the second tracker is built, or the second tracker is built against a seam that cannot hold it.

## Outcome

Someone presses `s` on a directory, types a scope name nothing records yet, and gets a window they have not seen before: two rows, `Linear` and `Jira`, one of them lit. They move the highlight to `Jira` and press Enter. The record form opens with four fields — base URL, project key, review state, label — rather than Linear's three, and the base URL field takes `https://acme.atlassian.net` exactly as typed. Enter writes it, and the key window comes up over the new scope as it does today. `.warlock/scopes.toml` now reads:

```toml
version = 2

[[scope]]
name = "payments"
tracker = "jira"
base_url = "https://acme.atlassian.net"
project_key = "PAY"
review_state = "In Review"
label = "area/core"
```

The same record from a terminal:

```
$ warlock scope add crates/payments payments jira \
    --base-url https://acme.atlassian.net --project-key PAY \
    --review-state 'In Review' --label area/core
```

Asked about that path, `warlock check` answers without complaint, naming the tracker and the fields that tracker has:

```
$ warlock check crates/payments --json
{"command":"check","path":"crates/payments","scope":"payments","open":["payments"],"opens":true,"tracker":"jira","project_key":"PAY","base_url":"https://acme.atlassian.net","review_state":"In Review","label":"area/core","key":"payments","key_found":true}
```

A key binds to it as to any scope:

```
$ warlock key use payments payments-token
warlock: `payments` now uses the key `payments-token` in this checkout
```

And every verb that would open a board says what it cannot do, by name, having done nothing:

```
$ warlock push payments docs/brief.md
warlock: `payments` files to Jira, and warlock can only file to Linear, so nothing was sent
```

The same sentence arrives on the panel's message line when `p` is pressed over a Jira scope, and the push never starts.

## Success criteria

**The record carries a tracker**

- `ScopeRecord` holds `name`, `review_state`, `label` and a `Tracker` enum whose variants are `Linear { team_key }` and `Jira { base_url, project_key }`.
- `ScopeRecord::team_key` no longer exists; a caller that wants a team key goes through the Linear view from slice 2.
- `SCHEMA_VERSION` is `2`, and a manifest declaring `1` is refused with the existing unsupported-version sentence naming both numbers.
- `tracker` is a required key on every `[[scope]]` table; a record without it is refused by index and by the name it spelled.
- `tracker = "linear"` requires `team` and refuses `base_url` and `project_key`; `tracker = "jira"` requires `base_url` and `project_key` and refuses `team`. Each refusal names the key at fault.
- A `tracker` value that is neither `linear` nor `jira` is refused, naming the value.
- `team`, `base_url` and `project_key` are stored byte-for-byte as written: no trimming, no case folding, no scheme added, no trailing slash added or removed.
- `.warlock/scopes.toml` in this repository declares `version = 2` and `tracker = "linear"` on the `warlock-team` record, and `Manifest::load` reads it.

**Linear flows cannot be handed a Jira record**

- A `LinearScope<'_>` view exposes `team_key`, `review_state` and `label`, and is the only way to read a team key.
- Producing one from a `&ScopeRecord` yields a refusal for a Jira record, in the one sentence the outcome shows.
- `push.rs`, `cut.rs`, `planned.rs`, `pulling.rs`, `puller.rs`, `queue.rs` and `filing.rs` take `LinearScope<'_>` where they took `&ScopeRecord` for its team key, and the crate does not compile if one is handed a record directly.
- `warlock push`, `warlock draft` and `warlock pull`, and the panel presses that reach a board, all refuse a Jira scope before a key is read or a client is built, and the message names the scope and the tracker.
- `warlock key use` and `warlock scope add` succeed against a Jira scope.
- `warlock check` against a Jira scope exits as it does for a Linear one, reporting rather than refusing.

**The TUI asks which tracker**

- A submit on the scope window naming an unrecorded scope opens a tracker picker, not the record form.
- The picker lists `Linear` and `Jira`, moves a highlight, writes nothing on Esc, and opens the record form for the chosen tracker on Enter.
- The Linear record form has three fields in today's order; the Jira form has base URL, project key, review state, label, in that order.
- `Windows` carries the picker alongside the scope, record and key windows, and never has two of the four open at once.
- A blank field in either form is refused under the uppermost blank field, as today, with the message naming that field.
- Esc on the picker leaves no scope and no record written.

**The headless record flags follow the tracker**

- `warlock scope add <path> <scope> [tracker]` takes the tracker as a third positional.
- The tracker is required when nothing records the scope name yet, and refused when something already does — matching the condition the record flags already carry.
- A new name given with no tracker is refused in one sentence that names the tracker alongside the record values also needed.
- `--team-key` is accepted only after `linear`; `--base-url` and `--project-key` only after `jira`; `--review-state` and `--label` after either.
- A flag the named tracker has no use for is refused by name, and so is a missing one.
- `warlock check --json` carries `tracker` whenever a record covers the path, `team` only for a Linear record, and `base_url` and `project_key` only for a Jira one.
- `HEADLESS-CLI.md`'s `scope add` section and `check` field table describe both trackers, including the refusals above.

## Constraints

The manifest reader stays hand-rolled. `Manifest::read` already parses each `[[scope]]` as a `toml::Value` before building a record, and that is what lets a refusal name an index, a scope name and a key. A `#[serde(tag = "tracker")]` enum reached through `#[serde(flatten)]` is not available: flatten disables `deny_unknown_fields`, and the unexpected-key refusals above are success criteria.

Nothing in this change opens a socket, and no Jira request is written anywhere. The second brief owns the board.

The base URL is stored as typed. Nothing here parses it, normalises it, or judges whether it names Jira Cloud or a self-hosted instance — those shapes differ, and the code that builds a request from one belongs to the brief that makes requests.

Blankness remains the only judgement on a record value. A tracker name is checked against two words because the enum has two variants; a team key, a project key, a review state, a label and a base URL are refused when blank and accepted otherwise, which is the rule `HEADLESS-CLI.md` already states and the reason `ScopeRecord`'s fields are stored unfolded.

`key use` keeps its shape. A key is one opaque string under one name per scope, and Jira's two-part credential is that string's business: nothing here adds a field for an account, an email or a token kind, to the committed record or to the machine-side config.

The TUI's four windows chain as the three do now: each submit takes one down and may put the next one up in the same value, so no caller can apply half of a transition or leave the reader looking at two prompts.

## Out of scope

**The Jira board.** No `Board` implementation, no `Opens` that can return one, no change to the twenty methods on the trait. That is the second brief, and it starts from a decision already made rather than an open question: Jira's **v2 REST API**, against both Cloud and Server/Data Center. v2 keeps a description as text, so the markdown warlock writes into a brief and later parses `## Scope` back out of in `cut.rs` survives the round trip; v3 takes Atlassian Document Format and would put a converter warlock maintains between `push` and `cut`. v2 is also the one API both deployments serve, so the choice buys a single code path. Recording it here means the second brief does not relitigate it.

**Making `Opens` tracker-aware.** `Opens::open` still takes a key and returns a `Linear<Client>`. Widening it to take a record, and choosing between an enum board and `Box<dyn Board>`, only pays for itself when there is a second board to return. Doing it now means writing the seam against one implementation and guessing at the other.

**Showing the tracker in the panel.** `ui.rs` keeps its Linear-shaped push line. Every press that would reach a board over a Jira scope refuses, so there is no screen in this change where a Jira record's fields would be read out — the first one is in the second brief, and it should be designed against a working board.

**Asking an already-recorded, unbound scope about its tracker.** The picker appears only where the record form appears: on creation. A scope recorded by somebody else and merely pointed at a new directory is not re-asked, which is the rule `scope_submit` already follows and the one `scope add` follows by refusing a tracker for a name that has a record.

**Migrating a manifest from version 1.** There is no reader for the old shape and no rewriter: a `version = 1` file is refused with the sentence that names both versions, and the one such file in existence is this repository's, edited by hand in slice 1. With no installed users there is nobody whose committed file this strands, and a lenient reader that guessed `tracker = "linear"` would be a second spelling of a Linear record to maintain forever.

**Validating a review state or a label against the tracker.** Neither board is consulted, here or today. A review state no Jira workflow has is a refusal the second brief can make at the moment it moves an issue, where it can name the states that do exist.

## Scope

### 1. The record carries a tracker, and the manifest reads version 2

depends_on: []

`ScopeRecord` in `crates/warlock-engine/src/manifest.rs` is a four-field struct with `#[serde(deny_unknown_fields)]` and a `#[serde(rename = "team")]` on `team_key`, carrying a comment that explains why the TOML key could not follow the Rust field name: committed manifests already spell `team`. `Manifest::read` parses `raw.scope` as a `Vec<toml::Value>`, reads `name` out of each raw table first so a record that will not parse is still nameable, calls `try_into::<ScopeRecord>()`, runs `validate_scope` on the name, and refuses a repeated name. `SCHEMA_VERSION` is `1` and is checked twice — in `read` against `RawManifest::version`, and in `deserialize_version` for the derived path, which exists so `toml::from_str::<Manifest>` cannot accept a version `read` refuses.

This slice gives `ScopeRecord` the fields `name`, `review_state`, `label` and `tracker: Tracker`, where `Tracker` is an enum over `Linear { team_key: String }` and `Jira { base_url: String, project_key: String }`. `SCHEMA_VERSION` becomes `2`. The derive stops carrying the record's whole shape: `try_into::<ScopeRecord>()` gives way to reading the raw table's keys in `read`, because the per-key refusals this brief owes — a missing `tracker`, a `team` on a Jira record, a `base_url` on a Linear one, a tracker value that is neither word — are exactly what a derive collapses into one serde message. The two-pass reader already in that function is where this fits, and `ScopeFault` grows the variants those refusals need beside `Malformed` and `Refused`.

An enum rather than a flat struct with optional fields, and a view in slice 2 rather than a `team_key()` that answers for both: a Jira project key and a Linear team key are not the same thing, and a single accessor returning either is what would let a Jira record travel into `push.rs` and fail at an API call instead of at a gate. The cost is real — roughly twenty-five call sites across seven files read `team_key()` today — and slice 2 is where it is paid.

Serialisation writes `tracker` before the tracker's own fields and both before `review_state` and `label`, so a hand-edited file and a warlock-written one read the same way. `RecordFields` in `crates/warlock-tui/src/rescope.rs` becomes tracker-shaped in this slice rather than in 3 or 4, because both the TUI and the CLI write records through it and neither can be converted before it changes; its existing callers keep handing it a Linear shape until slices 3 and 4 let a person choose otherwise.

`.warlock/scopes.toml` in this repository is edited here: `version = 2`, and `tracker = "linear"` on the `warlock-team` record. Without that edit the slice does not land, because warlock cannot read its own repository.

A later edit must not make the reader lenient about a version it does not know, and must not reintroduce a `team_key` accessor on `ScopeRecord`. The first turns an old binary meeting a new file from a clear refusal into a rewrite that drops fields; the second quietly undoes slice 2.

### 2. A Linear view, and every board verb refuses a Jira scope

depends_on: [1]

With `team_key` gone from `ScopeRecord`, the flows that read it do not compile. `push.rs` resolves `destination.team_key()` to a team id and filters a project-by-name lookup with it; `cut.rs` and `planned.rs` do the same for issues and for planned projects; `pulling.rs` builds a branch name and a queue filter from it; `puller.rs` carries it into a run record; `queue.rs` compares it against a named issue's own team key; `filing.rs` copies it into a target struct; `check.rs` reports it.

This slice adds `LinearScope<'_>`, a borrowed view over a record exposing `team_key`, `review_state` and `label`, with one fallible constructor from `&ScopeRecord` that refuses a Jira record in the sentence the outcome shows. The flows above take the view where they took the record for its team key. `check.rs` is the exception and stays on the record: it reports what the record says, per tracker, and reporting is not reaching for a board.

The gate goes where a scope is resolved into work — the headless `push`, `draft` and `pull` paths in `main.rs`, and their panel counterparts in `pushing.rs`, `cutting.rs`, `pulling.rs` and `planned.rs` — so the refusal lands before a key is read or a client is built. Putting it at the gate rather than inside each flow is what makes the compiler the list of sites: a flow that wants a team key must name `LinearScope<'_>` in its signature, and the only way to hold one is through the constructor that can refuse. That list is what the second brief walks.

The rejected alternative was a runtime check at the three verbs with the record left flat. It is a fraction of the work, and it saves exactly the thing the second brief needs. It also leaves a fourth verb free to forget the check and discover Jira at a failed API call.

A later edit must not add a way to read a team key off a record without passing the gate, and must not let a refusal arrive after a mutation. The whole value of this slice is that a Jira scope reaches no socket; a gate moved below the first create is a brief filed to a board nobody meant.

### 3. The tracker picker, and a record form per tracker

depends_on: [1]

`scope_press` in `crates/warlock-tui/src/scoping.rs` opens the scope window. `scope_submit` calls `rescope` and, on `ScopeRefusal::NeedsRecord`, returns `Windows::recording(module, &scope)`, which puts up `RecordPrompt::open`. `RecordForm` in `crates/warlock-tui/src/prompt.rs` holds three `ScopeField`s and a `focus: RecordField` over the fixed variants `TeamKey`, `ReviewState` and `Label`, with `field` and `slot` matching on all three. `Windows` is a three-field struct — scope, record, key — documented as never having two open at once, and `record_submit` writes the record and then opens the key window over the scope it has just saved, in that order, so an Esc on the key window leaves a recorded, unbound scope rather than something half written.

This slice puts a picker between the scope window and the record form: a highlighted list of two rows, built the way `KeyPrompt` builds its list of key names — a highlight that moves, Esc that writes nothing, Enter that chooses. `Windows` grows a fourth field and `Modals::current`'s precedence covers it. `RecordForm` is constructed with a tracker and holds that tracker's fields, three for Linear and four for Jira; `RecordField` covers `TeamKey`, `BaseUrl`, `ProjectKey`, `ReviewState` and `Label`, and a form answers only for the ones it has.

A separate window rather than a tracker toggle as the form's first field: a toggle would have to decide what happens to text already typed when the tracker changes, and either answer — discard it, or keep it in fields the new tracker does not have — is a behaviour nobody can guess from the screen. Asking first means the question is answered before anything is typed, and the form stays a form.

The order of writes does not change: the record reaches disk, and only a save that landed opens the key window. A later edit must not let the picker write anything, and must not let two of the four windows stand at once — the first makes an Esc leave a scope half recorded, and the second gives the reader two prompts and the input loop no answer about which one a keystroke belongs to.

### 4. `scope add` takes the tracker, and `check` reports it

depends_on: [1]

`warlock scope add <path> <scope>` takes `--team-key`, `--review-state` and `--label`, required together when nothing records the name yet and refused when something already does, with refusals that name the missing or blank flag — `HEADLESS-CLI.md` prints all of them. `warlock check --json` carries `team`, `review_state` and `label`, each absent when no record covers the path, documented in that file's field table.

This slice makes the tracker a third positional: `warlock scope add <path> <scope> [tracker]`. It is required under the same condition the record flags already carry — a name nothing records — and refused for a name that has a record, so pointing a directory at somebody else's scope stays a two-argument command. `--team-key` is accepted only after `linear`, `--base-url` and `--project-key` only after `jira`, `--review-state` and `--label` after either; a flag the named tracker has no use for is refused by name, and so is a missing one. A new name given with no tracker is refused in one sentence that names the tracker alongside the record values it also needs, rather than refusing the tracker and then refusing the flags on the next attempt.

Positional rather than `--tracker`, because a value a command cannot run without is not an option: `key use <scope> <name>` settled that. A flag spelling would also make the tracker a third place the shape is named — after the file and the picker — and so a third place the three could disagree.

`check` gains `tracker` whenever a record covers the path, keeps `team` for a Linear record only, and adds `base_url` and `project_key` for a Jira one. It reports and does not refuse: a hook asking about a path under a Jira scope is asking who may work there, which has nothing to do with which board the work is filed to. `HEADLESS-CLI.md`'s `scope add` section and `check` field table are rewritten to match, including each refusal above.

A later edit must not make `check` refuse a Jira scope, and must not make the tracker optional for a new record by defaulting it. The first breaks the boundary hook for every path under a Jira scope; the second puts a Linear record on disk for somebody who meant Jira and said nothing.
