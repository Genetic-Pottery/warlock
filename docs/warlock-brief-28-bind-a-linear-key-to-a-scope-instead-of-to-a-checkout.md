# Bind a Linear key to a scope instead of to a checkout

Today a repository has one Linear key. `warlock key use <name>` writes a single `key = "work"` line into the machine-local `config.toml` that `crates/warlock-engine/src/sigils.rs` owns, and every door that needs a credential reads that one line: `bound_key` and `route_facts` in `crates/warlock-engine/src/route.rs`, and `resolve_filing` in `crates/warlock-engine/src/filing.rs`. The binding is a fact about the checkout, so a repository holding two scopes that file to boards in two different Linear workspaces cannot work — the two boards need different keys, and there is one place to put a key.

What that costs is not a warning but a wrong answer. Whichever workspace is bound wins; a push, draft or pull into the other scope fails at the API with an authentication error that names nothing about scopes, or, if the two workspaces happen to share a team key, succeeds against the wrong board and is found out later by somebody else. There is no spelling of `warlock key use` that fixes it, because the config has no room for a second key. A repository in that shape is stuck, and the only way out today is a second checkout at a second path so the derived project directory — and with it the binding — is a different one.

The keys themselves are not the problem and do not change. They stay in `~/.warlock/keys.toml` under names each person picks, machine-wide, reusable across repositories, never committed, read only by name everywhere except `resolve_filing`. What changes is the binding: the machine-local config stops holding one key for the checkout and starts holding one key name per scope name. The scope name is the only handle the two sides can share, because it is in the committed `.warlock/scopes.toml` and every member's key names and key values differ.

## Outcome

In a repository whose `scopes.toml` records `data-plane` against the `WAR` board and `web` against `WEB` in another workspace, somebody binds each:

```
$ warlock key use data-plane work
warlock: `data-plane` is bound to the key `work`, written to `/home/you/.warlock/warlock-3f91c2a8d4e0b117/config.toml`
$ warlock key use web side-project
warlock: `web` is bound to the key `side-project`, written to `/home/you/.warlock/warlock-3f91c2a8d4e0b117/config.toml`
```

The config now reads:

```toml
sigils = ["*"]

[keys]
data-plane = "work"
web = "side-project"
```

`warlock check` answers per scope, and the key line is about the scope covering the path rather than about the checkout:

```
$ warlock check crates/engine/src
crates/engine/src is covered by the scope `data-plane`
this machine holds `*`, which opens it
filing to `WAR` would use the key `work`, which this machine stores
$ warlock check apps/web/src
apps/web/src is covered by the scope `web`
this machine holds `*`, which opens it
filing to `WEB` would use the key `side-project`, which this machine stores
```

A scope nobody has bound refuses by name, and says the command that fixes it:

```
$ warlock push billing docs/brief.md
warlock: no key is bound to the scope `billing` in `/home/you/.warlock/warlock-3f91c2a8d4e0b117/config.toml`: bind one with `warlock key use billing <name>`
```

A typo is refused before anything is written, against the scopes the manifest records:

```
$ warlock key use data-palne work
warlock: `data-palne` has no `[[scope]]` record: the scopes recorded here are `data-plane`, `web`
```

Nobody types five bindings by hand for five scopes, because the panel asks. Pressing `s` over a directory and typing a scope nothing records opens the record form as it does today; answering it writes the scope and the `[[scope]]` record, and then a third window comes up listing this machine's key names:

```
┌ key for `billing` ───────────────────────┐
│ which stored key files to this board?    │
│                                          │
│   work                                   │
│ > side-project                           │
│   client-acme                            │
│                                          │
│ Enter binds · Esc leaves it unbound      │
└──────────────────────────────────────────┘
```

Enter writes the binding and the window goes down with nothing on the message line. Esc leaves the scope and its record written and the binding absent, which is the state the refusal above is about. On a machine with no keys stored the window has no rows and one sentence instead — `no keys are stored on this machine: run \`warlock key add <name>\`, then bind this scope with \`warlock key use billing <name>\`` — because a Linear key comes from a browser and nobody has one in hand at that moment.

The same window comes up in front of a push, draft or pull whose scope is unbound, so a member who clones the repository and opens the panel is asked once, where the work is, rather than reading a refusal and going to look up a command.

## Success criteria

**The config holds a map**

- `config.toml` serialises bindings as a `[keys]` table of scope name to key name; a config with no bindings writes no `[keys]` table at all.
- There is no `key` field on `sigils::Config`, and a config holding a top-level `key = "..."` line fails to parse rather than being read leniently.
- `save_key_binding(home, root, scope, name)` writes one scope's binding and leaves every other binding, the sigils and the base branch in the file untouched.
- A second `save_key_binding` for a scope already bound replaces that scope's key name and no other.
- `load_key_binding(home, root, scope)` returns `Ok(None)` for a scope with no binding in a config that exists, and `Err(Error::NotFound)` for a checkout with no config.
- Both the scope name and the key name are judged by `validate_scope`, and a name that fails it writes nothing.

**Every door asks per scope**

- `bound_key(home, root, scope)` refuses with `Error::Unbound` naming that scope, and with `Error::Dangling` when the scope's bound name is not in the key store.
- `resolve_route` resolves the key for the scope covering the path, so two paths under two scopes in one repository resolve to two different key names.
- `resolve_filing` resolves the key for the record it has already picked, and the board refusals (`Boardless`, `Nothing`, `Unknown`, `Several`) are still decided before the key store is touched.
- `route_facts` reports `key: None` and `stored: false` for a path no scope covers.
- `Error::Unbound`'s sentence names the scope and the `warlock key use <scope> <name>` that binds it; `Error::Dangling`'s names the scope, the key name and the `warlock key add` that stores it.

**The command takes a scope**

- `warlock key use <scope> <name>` takes both positionally, scope first, and neither is optional.
- `key use` refuses a scope with no `[[scope]]` record in `scopes.toml`, names the recorded scopes, and writes nothing.
- `key use` refuses a key name the machine's store does not hold, as it does today, and writes nothing.
- Both refusals are decided before the config is opened.
- `warlock key forget <name>` names every scope in this checkout that was bound to the forgotten name, and says nothing about scopes when none was.
- `warlock key list` takes no scope, resolves no repository, and prints names exactly as it does today, in text and in `--json`.
- `warlock check <path>` prints a key line about the scope covering the path, and `key` is null in `--json` when no scope covers it.

**The panel asks, and never takes a key**

- A key-binding window lists the key names this machine stores, one row each, and binds the highlighted row's name to the scope it was opened for.
- With no key stored, the window shows no rows and one sentence naming `warlock key add <name>` and the `warlock key use` that follows it.
- No key value is read, held, drawn or carried by anything the window touches; the window's state holds scope names and key names only.
- Esc leaves the scope unbound and writes nothing to the config.
- Writing a scope that needs a `[[scope]]` record opens the key window after the record form's submit has saved the scope and the record, and not before.
- A push, draft or pull whose scope has no binding opens the key window instead of its own confirm dialog, and the confirm dialog follows once the binding is written.
- `Modals::current` gives the key window a fixed place in the precedence, and a key window cannot take the keys from a push, cut or pull dialog already up.

## Constraints

Keys stay where they are: `~/.warlock/keys.toml`, names each person picks, machine-wide, reusable across repositories, never committed. This work does not move them, rename them, scope them to a repository or add a second store.

No key value reaches the repository, and no key name does either. The scope name is the only handle shared between the committed `scopes.toml` and the machine-local config, because every member's key names and values differ.

Only `resolve_filing` reads a key value. Everything else — `route.rs`, `check.rs`, `key.rs`, and everything the panel work adds — asks `load_key_names` and holds names, so no new code path can leak a value into an error, a `Debug` or a panic.

The home directory stays a parameter in `sigils.rs`, `keys.rs`, `route.rs` and `filing.rs`, resolved by the caller and never read from the environment inside those modules. The existing header comments give the reason and it still holds: a module that resolved `HOME` itself would let a test in this crate read the developer's real credentials.

`Config` keeps `deny_unknown_fields`. The file is short, hand-editable and has no version key, so a misspelled table name must fail rather than parse as a config holding nothing.

The boundary rule stays where it is. `scope_covering` decides which scope covers a path and `scope_opens_to` decides membership, each called once at the door that needs it. No part of this work writes a second comparison, and the sigil wildcard `*` keeps its current meaning for sigils and gains no meaning for keys.

There is no fallback binding and no wildcard key. A key binding that covered scopes added later is the thing this change exists to remove.

`warlock key list` and `warlock key add` resolve no repository and must keep working from a home directory or anywhere else outside a checkout.

The panel never takes a secret. No field, prompt or window added here reads a credential from the keyboard; `warlock key add` on a terminal stays the single place in the codebase that does, with the raw-mode read and its `Drop` restore.

No dependency is added. The TUI work uses the `prompt.rs`, `modal.rs` and `confirm.rs` machinery already in the crate.

## Out of scope

**Migrating a config that holds the old `key` line.** Under `deny_unknown_fields` such a file stops parsing entirely, which also hides the `sigils` beside it — and `held()` would then default over them on the next save. Accepting and ignoring the field was rejected: there are no users, and the only machines holding such a file are the developer's own, where the fix is deleting one line. A lenient read would be permanent code paying for a one-time hand edit.

**A command that prints the whole map.** `warlock key list` keeps standing in no repository and printing names only; a command whose output changes with the directory it runs in is worse than one with no bindings column. `warlock check <path>` is the per-scope answer, and the `[keys]` table is flat readable TOML whose path every refusal prints. The cost is accepted: until the panel chooser lands there is no one-line answer to "which scopes are bound here".

**Adding a key from inside the panel.** A Linear API key comes from a browser, so nobody has one in hand when the window opens; the inline form would save no trip and would cost a field holding a live credential in `App` state, drawn in a frame and carried through `Modal`. `push.rs` already needs a redacting `Key` wrapper to keep one value safe on one line, and a typed secret in a prompt is a second, larger place to get that wrong.

**A one-argument `warlock key use <name>` that binds every recorded scope.** It would make the common case one command, but a one-argument and a two-argument form under one name are two commands sharing a spelling, and the shortcut's whole value is binding scopes in bulk — which is one keystroke away from binding a scope to the wrong workspace's key. Five scopes is five bindings, and the panel is what makes that bearable.

**Binding a scope before its `[[scope]]` record exists.** `key use` refuses it. Creating a scope writes the record and then asks for the key, in that order, so there is no moment where a binding needs to exist ahead of a record.

**Reporting which scopes on this machine are bound to a key being forgotten, beyond this checkout.** `key forget` names the scopes in the checkout it is standing in and no others. Walking every project directory under the home to find the rest is the behaviour the existing comment already refuses, and it is not revisited here.

## Scope

### 1. Bind keys to scopes in the engine and in `warlock key use`

depends_on: []

`crates/warlock-engine/src/sigils.rs` holds `Config` with three fields — `sigils`, `key` and `branch` — and the pair `load_key_binding(home, root)` / `save_key_binding(home, root, name)` that read and write `key`. `crates/warlock-engine/src/route.rs` has `bound_key(home, root)`, which reads that one binding and refuses with `Error::Unbound { path }` or `Error::Dangling { key, path }`, and `route_facts`, which reads the binding and the store leniently and reports `key` and `stored` beside the scope. `crates/warlock-engine/src/filing.rs` calls `bound_key` at line 120, after it has already picked a record. `crates/warlock-tui/src/key.rs` has `key_use(name)` → `bound(home, root, name, out)`, which checks the name against the store before writing, and `key_forget` → `forgotten`, which reads the binding to report whether this checkout was unbound. `crates/warlock-tui/src/check.rs` builds its `key` and `key_found` fields from `route_facts` and words them in `key_line`.

Replace `Config`'s `key: Option<String>` with `keys: BTreeMap<String, String>`, serialised as a `[keys]` table and skipped when empty so a config with no bindings keeps writing the bytes it writes today. `load_key_binding` and `save_key_binding` each gain a scope parameter; the save reads the file through `held`, inserts or replaces one entry and writes the whole config back, which is how the sigils and the base branch beside it survive. Keep the existing asymmetry exactly: unbound is `Ok(None)` and a checkout with no config is `Err(NotFound)`, because those send a person to different places. Judge the scope name with `validate_scope` as the key name already is, so scopes, sigils and key names stay one character class.

`bound_key` and `route_facts` take the scope they are asking about. Neither needs reordering — `resolve_route` has the covering scope in hand before it reaches the key, and `resolve_filing` has picked its record before line 120 — which is why this shape was chosen over threading a map of bindings out to the callers and letting each one index it: a caller that indexes is a caller that can index with the wrong scope. `route_facts` reports `key: None` for a path no scope covers, which changes its current doctest: there is no checkout-wide key left to report beside an unscoped path. Reword `Error::Unbound` to name the scope and the `warlock key use <scope> <name>` that binds it, and `Error::Dangling` to name the scope alongside the key and the store path.

`key_use` becomes `key_use(scope, name)` with both arguments positional and scope first, matching `push <scope> <path>`, `draft <scope>` and `pull <scope>`. A flag was rejected because the scope is never optional. `bound` gains a second refusal before it writes: the scope must have a `[[scope]]` record in `scopes.toml`, read through `Standing::manifest()`, and a scope that has none is refused naming the recorded scopes, in the shape `filing::Error::Unknown` already uses. Both refusals are decided before the config is opened, for the reason the existing store check gives — a name written and then found wrong leaves a config that reads as a binding and a push that fails somewhere else. `forgotten` now reports every scope in this checkout bound to the forgotten name rather than a single yes or no, and says nothing about scopes when none was. `check.rs`'s `key_line` keeps its three cases and names the covering scope in the unbound one; `key` in the JSON envelope is null when no scope covers the path.

This slice ends at the headless refusal and must stand alone: with it merged, a two-workspace repository works for anybody who types commands, with no panel change at all. A later edit must not put a fallback or wildcard entry into the `[keys]` map, must not make `load_key_binding` answer for a scope other than the one it was asked about, and must not move the key lookup in `resolve_filing` ahead of the board refusals — a checkout that is both ambiguous and unbound is told about the board first, because `--scope` is the fix that is about this push.

### 2. A panel window that binds a scope to a stored key name

depends_on: [1]

`crates/warlock-tui/src/prompt.rs` holds `ScopeField`/`ScopePrompt` and `RecordForm`/`RecordPrompt`, the two windows the `s` key puts up. `crates/warlock-tui/src/modal.rs` enumerates every window in `Modal` and `Modals`, and `Modals::current` fixes which one takes the keys, swallows the pointer and is drawn. `crates/warlock-tui/src/ui.rs` draws whichever `current` returns. Both existing windows are text fields; there is no window in the crate that offers a list of choices.

Add a key-binding window: a prompt type in `prompt.rs` holding the scope it was opened for and the key names this machine stores, with a highlighted row, plus its `Modal` variant, its slot in `Modals`, its place in `Modals::current` and its drawing in `ui.rs`. It is built from `load_key_names`, never from `load_key`, so no key value exists anywhere in the window's state — which is what makes this safe to hold in `App` and draw in a frame, and it is the reason the alternative of a field somebody types a key into was refused outright. With no names stored the window has no rows and one sentence naming `warlock key add <name>` and the `warlock key use <scope> <name>` that follows it, because a Linear key comes from a browser and the person has to leave the panel either way. Enter on a row calls `save_key_binding` for the window's scope and that row's name; Esc writes nothing and leaves the scope unbound.

The window is built here with no caller, so this slice lands as a window that can be opened by a test and by nothing else. That is deliberate: it keeps the two wirings in slices 3 and 4 free of the window's own shape, and each of those slices is then one decision about when to ask.

A later edit must not give this window a field that reads a key value, and must not reach for `load_key` to decorate a row — the window shows names, and the whole key subsystem's never-print-a-secret rule costs nothing only as long as no value is held here. The precedence line in `Modals::current` is a statement about what a person is looking at, not an ordering of convenience; an edit that moves this window must say why in that comment and rewrite the whole block rather than appending to it.

### 3. Ask for the key when a scope is created

depends_on: [2]

`crates/warlock-tui/src/scoping.rs` runs the `s` key from press to saved `scopes.toml`. `scope_submit` either saves directly or, on `ScopeRefusal::NeedsRecord`, returns `Windows::recording(module, &scope)`, which closes the scope field and opens the record form. `record_submit` builds `RecordFields` from the form's three fields, calls `rescope` and saves the scope and the record together. `Windows` carries both windows in one value so that a submit can take one down and put the other up in a single return, with the comment explaining why two returns would let a caller apply half of it.

Extend that chain by one window: after `record_submit`'s save succeeds, open the key-binding window for the scope just recorded. The ordering is the decision — the record is written first and the key is asked second — which is what makes `key use`'s record check in slice 1 safe, since there is no moment in the panel where a binding would need to exist ahead of a record. A save that fails goes to the message line as it does today and opens no key window: asking which key files to a board that was never written would be a question about nothing.

Whether the key window should also open when `s` sets a directory to a scope that is already recorded but unbound is left open. Slice 4 covers the person who hits that case at a push, draft or pull, which is where it actually bites, so this slice asks only on creation.

Carry `Windows` forward rather than returning the new window separately, for the reason already written there. A later edit must not reorder the record save and the key ask, and must not make an abandoned key window roll back the scope or the record — Esc leaves a scope that exists and is unbound, which is a state slice 1's refusal is built to explain.

### 4. Ask for the key when a push, draft or pull needs an unbound scope

depends_on: [2]

`crates/warlock-tui/src/confirm.rs` holds `PushConfirm`, `CutConfirm` and `PullConfirm` with their `Filing`, `Cutting` and `Undertaking` payloads; each is opened from the composer by `/push`, `/draft` or `/pull` and each resolves a scope before it asks its question. All three sit above the scope and record fields in `Modals::current`, with the comment explaining that a question somebody is looking at outranks a field that can come up under it on no keystroke.

Before any of the three opens its dialog, ask `load_key_binding` for the scope it resolved; when there is no binding, open the key-binding window for that scope instead, and open the confirm dialog once a binding has been written. Esc on the key window puts the whole flow down with nothing started, which is the same outcome as Esc on the confirm dialog itself — a person who declines to bind has declined the push. This shape was chosen over letting the flow start and refusing at the engine: the refusal from slice 1 is correct and stays, but a member who clones the repository and opens the panel should be asked where the work is rather than sent to a shell to read a command.

Fix the key window's place in `Modals::current` for the three flows that now open it, and rewrite the precedence comment whole to say what it is: a key window opened by one of the three cannot take the keys from a dialog already up, because the dialog is what the person is looking at. Appending a sentence to that block would leave the lines above it unchecked against the new ordering, and the half that went wrong would read as authoritatively as the half that is right.

A later edit must not let a confirm dialog open on a scope with no binding — that path exists only to be answered by the engine refusal, and reaching it from the panel means somebody watches a push start and fail. It must also not read a key value in any of the three flows outside the one line in `push.rs` that already does it behind the redacting `Key` wrapper.
