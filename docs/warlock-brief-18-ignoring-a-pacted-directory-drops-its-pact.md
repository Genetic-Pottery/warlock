# Ignoring a pacted directory drops its pact

Adding a pacted directory to `.warlockignore` strands it yellow for good. The watcher already reloads when `.warlockignore` is saved, and `mark_excluded` in `crates/warlock-engine/src/load.rs` marks the directory as ignored. But `state_of` in the same file never reads that mark. It finds the directory's entry in `.warlock/pacts.toml` and hashes the directory. `subtree_hash` in `hash.rs` returns the empty digest for an ignored directory, which never matches the recorded grant, so the row is drawn `PactedStale`. Neither way out works. `pact_intent` in `crates/warlock-tui/src/app.rs` refuses `p` in both directions on an ignored row. `pactable_directories` in `pact.rs` returns nothing for an ignored directory, so `r` has nothing to write. Left alone, every directory anyone ever ignores after pacting it stays yellow in the tree and keeps a dead entry, and any scope on it, in the committed manifest. The only fix is editing `pacts.toml` by hand, and the tree exists so nobody has to do that.

## Outcome

The operator has `crates/warlock-tui/src` pacted and green in the tree. They open `crates/warlock-tui/.warlockignore` in another window, add `src/`, and save. Within one watcher quiet period the `crates/warlock-tui/src` row, and every row below it, turns gray. `crates/warlock-tui` turns yellow, because the files it was granted over have left its digest. Nothing appears in the footer. `git diff .warlock/pacts.toml` shows the entries for `crates/warlock-tui/src` and everything under it removed, scopes included. Pressing `r` on `crates/warlock-tui` brings it back to green.

If they remove `src/` from `.warlockignore` and save, `crates/warlock-tui/src` comes back gray, with no pact and no scope, and `crates/warlock-tui` goes yellow again. Getting it green means `p` and a full pass.

If they add the rule while a run is in flight, the rows turn gray on the next reload. The manifest entries are removed by the reload that follows the run.

## Success criteria

**The loader draws an ignored directory gray**
- `load_tree` returns `NodeState::Unpacted` for every directory marked ignored, including one that still has an entry in `pacts.toml`.
- That holds for every directory below an ignored one, as marked by `mark_excluded`, whether or not those directories have entries.
- `load_tree` does not call `subtree_hash` for an ignored directory that has an entry.
- A test loads a repository where a directory with a granted entry is later named in `.warlockignore`, and asserts the directory is `Unpacted` and its parent is `PactedStale`.

**The engine can remove ignored entries from a manifest**
- A public engine function takes a manifest and a repository root and returns a manifest with no entry whose directory, or any ancestor of it, is excluded by `.warlockignore`.
- Entries with a scope are removed exactly like entries without one, and no sigil is consulted.
- Entries for directories that are not excluded come back unchanged, grant and scope included.
- A malformed `.warlockignore` makes the function return an error, never a manifest with entries removed or kept on a guess.
- The function writes nothing to disk. Tests cover: an ignored entry with a scope, an entry under an ignored ancestor that has no entry of its own, a negated rule that keeps a directory in, and a manifest with nothing to remove coming back equal to the input.

**The TUI applies the cleanup without being asked**
- The startup load in `load_app` and every `reload_tree` run the cleanup, and save `pacts.toml` only when it removed at least one entry.
- No cleanup runs while a run is in flight. The `in_flight` gate in `Watched::round` is what ensures this, and a test pins it.
- A save made by the cleanup causes at most one further reload, which removes nothing and writes nothing.
- The cleanup sets no footer message.
- The existing `p` refusal on an ignored row is unchanged.

## Constraints

- `HASH_CONTEXT` and the bytes `subtree_hash` digests do not change. Moving them would make every pacted repository stale.
- `NodeState` keeps exactly three variants. Being ignored is not a colour, and an ignored row is drawn with the existing gray.
- Whether a path is excluded is decided only through `ignores::is_ignored` and the walks in `load.rs`. No second matcher and no skip list is added.
- `pacts.toml` is written only through `Manifest::save`, which stays atomic.
- `warlock check` stays read-only and writes nothing anywhere.
- The cleanup needs no sigil and does not check scopes. Editing `.warlockignore` is file editing, which is the operator's call, like editing the sigil file. Warlock's guardrails belong to its keys, not to files.
- No new dependency is added.
- Comments follow `CLAUDE.md`: none that restates the code, and a comment only where a plausible edit would otherwise break something.

## Out of scope

- **Cleanup from `warlock check`.** Its contract is that it writes nothing, and CI depends on that. Until warlock next launches it can report a scope from an entry that is now ignored. That is accepted, because the next launch fixes it.
- **Cleanup during shell runs, or inside `descend`.** In the TUI, reloads are already held back while a run is in flight, and a reload follows every run. So a run that saves entries for a directory ignored mid-run is corrected straight away. A shell run's leftovers wait for the next launch, which is the same trade as `check`.
- **Two warlock processes writing `pacts.toml` at once.** That race exists today for every pact, refresh, un-pact and scope edit. It deserves its own change, not a special case here.
- **A footer line naming what was removed.** Ignoring a directory has an obvious effect, the diff in `pacts.toml` is the record, and the footer is noisy enough already.
- **Keeping or restoring an entry when a rule is removed.** Removing a rule returns the directory to the state it would have had if it had never been pacted under that rule. That is the state pacting a parent already leaves around an ignored child. Keeping a grant hidden so it can come back later would be a fourth state in all but name.
- **Protecting against an accidental rule such as `*`.** The removed entries are committed bytes, and `git revert` brings them back.
- **Entries for directories that were deleted.** `is_ignored` treats a missing path as not excluded on purpose. Dead entries for deleted directories are a separate problem with separate risks.

## Scope

### 1. Draw ignored directories gray in the loader
depends_on: []

`state_of` consults the ignored mark before the manifest. An ignored directory is `Unpacted` whatever `pacts.toml` holds, and it is never hashed. This slice is what makes the change feel immediate: the row goes gray on the reload `.warlockignore` already triggers, even mid-run and even if a later save fails. It needs no writing at all. It closes off ever showing an ignored directory in colour, which the cleanup rules out anyway. The existing mark from `mark_excluded` is the source, so descendants of an ignored directory go gray along with it.

### 2. Add the manifest cleanup to the engine
depends_on: []

A pure function from manifest to manifest removes every entry at or below an excluded directory. It sits beside `unpact_subtree` and has the same semantics: removing an entry removes its scope. It checks each entry's directory and its ancestors, not just the entry itself, because `is_ignored` only answers for a directory's own name, and a pacted directory can sit under an ignored one that has no entry. It lives in the engine, not the TUI, so the rule is written once. It does not save, so the caller decides when writing is safe.

Open: whether it is built on `unpact_subtree` or filters entries directly.

Open: whether it covers the whole manifest or only entries under the directory warlock was loaded at, when warlock is started in a subdirectory.

### 3. Run the cleanup on every TUI load and reload
depends_on: [2]

`load_app` and `reload_tree` run the cleanup against the manifest on disk before loading the tree, and save only if something was removed, so the tree drawn reflects the saved manifest. The hook goes here because this path already never runs during a run: `Watched::round` skips reloads while a run is in flight, and a reload follows every run's single save. That removes the lost-update risk without a queue, a lock or a second place to wait. The save triggers the manifest watch, and the reload that follows finds nothing to remove, so it stops after one extra round. The cleanup says nothing in the footer.

Open: what the operator sees when the cleanup's save fails. It must not fail the load or block the draw. Whether it gets a footer note through `note`, or stays silent until the next reload tries again, was not decided.
