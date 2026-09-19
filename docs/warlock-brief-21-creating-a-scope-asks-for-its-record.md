# Creating a scope asks for its record

In this repository, `warlock scope add` and the `s` key's popup both write a scope name and nothing else. The team, review state and label that the scope routes work to have no way in. They live in a separate `[[scope]]` table in `.warlock/pacts.toml`, and the only way to write one is by hand. A comment in `crates/warlock-tui/src/check.rs` says so: no command writes a record. So a scope made from warlock is a scope with no record, and `warlock check` answers that it "has no `[[scope]]` record, so there is nothing to route to". Nothing points the operator at the missing half until they ask. Left alone, every new scope needs a hand-written TOML table whose four keys and spelling have to be looked up in tests or a brief. A scope made in a hurry stays unrouted until someone notices.

## Outcome

In a repository with no scopes, the operator presses `s` on a pacted directory, types `data-plane`, and presses Enter. A second popup asks for the team, the review state and the label. They fill in `Data Plane`, `In Review` and `area/data-plane` and submit. `.warlock/pacts.toml` now holds the scope on that pact and one `[[scope]]` record for `data-plane`. `warlock check` on that directory prints the full route: work there is filed to `Data Plane`, as `In Review`, labelled `area/data-plane`.

They press `s` on a second directory and type `data-plane` again. The first popup closes and nothing more is asked, because the record already exists.

At a shell, `warlock scope add crates/storage billing` fails and names `--team`, `--review-state` and `--label`, because `billing` has no record. Run again with all three flags, it writes the scope and the record together. `warlock scope add crates/storage data-plane` succeeds with no flags. The same command with `--team` added fails, because `data-plane` already has a record.

## Success criteria

**TUI**

- Pressing `s` and submitting a name that has no `[[scope]]` record opens a second popup asking for team, review state and label.
- Submitting a name that already has a record writes the scope as the `s` key does today, and no second popup opens.
- Submitting the second popup saves the pact's scope and the new `[[scope]]` record in a single save of `.warlock/pacts.toml`.
- Pressing Esc on the second popup leaves `.warlock/pacts.toml` byte-identical to what was read, and the pact's scope unchanged.
- A blank or whitespace-only value in any of the three fields is refused under that field, and the popup stays open with what was typed.
- An empty name in the first popup still clears the directory's scope and opens no second popup.
- The existing refusals (a run in flight, a closed scope, a row that cannot be scoped) still happen before either popup opens.

**CLI**

- `warlock scope add <path> <name>` fails without writing when `<name>` has no record and any of `--team`, `--review-state` or `--label` is missing. The error names the missing flags.
- `warlock scope add <path> <name> --team … --review-state … --label …` for a name with no record writes the scope and the record in one save.
- `warlock scope add <path> <name>` for a name that already has a record writes the scope as it does today.
- `warlock scope add` fails without writing when any of the three flags is passed for a name that already has a record.
- A blank or whitespace-only flag value is refused without writing.
- The existing boundary refusal (exit 3, file untouched) is still asked before any of the above.
- `HEADLESS-CLI.md` documents the three flags and both new refusals.

**Records**

- Team, review state and label are stored exactly as typed, with no trimming, folding or other judgement past "not blank".
- The record's name is the same folded string that is written onto the pact.
- Every `[[scope]]` record and `[[pact]]` row the write did not create or change is byte-identical after the save.
- After either path, `warlock check` on the scoped directory prints the full route.

## Constraints

- No change to `warlock-engine`. The existing `ScopeRecord::new`, `Manifest::with_scopes` and `Manifest::rebuilt_with` are what the write uses. The engine keeps judging only the record's name, through `validate_scope`.
- The rule is "not blank" and nothing more. What a team slug, review state or label may contain stays the tracker's business, as the comment on `ScopeRecord` says.
- A scope and its record are created together in one save or not at all. Warlock never writes half of one.
- An existing record is never overwritten, merged or deleted by either path.
- The CLI never prompts. `scope add` stays a command that runs with no terminal, as `HEADLESS-CLI.md` describes it.
- The boundary is asked first, exactly as today, in both the TUI and the CLI.
- No new key in the TUI. The second popup is reached only through `s`.

## Out of scope

- **Editing or deleting a record from warlock.** A record is shared by every directory with that scope name, so editing it from one row would quietly reroute others. A team or label is set about once, and the manifest is already the place to change it.
- **Interactive prompting on the CLI.** `scope add` is documented as running headless and people script it. Prompting would make it block in a pipeline, and a refusal that names the flags gives the same guidance.
- **Validating values beyond "not blank".** A known list of review states or a label format would mean warlock learning each tracker's rules. A mistake gets fixed in the file.
- **Offering a record for scope names already in use without one.** A scope written by hand or before this change with no record stays legal and unrouted. This change only asks when a name is being newly written through `s` or `scope add`.
- **A TUI path to make a scope with no record.** Esc writes nothing, so the TUI and CLI agree that a new scope arrives with its record. Hand-editing the file remains the way to make an unrouted one.

## Scope

### 1. A single write that sets a scope and creates its record

depends_on: []

This adds the rebuild that puts a scope on one pact and adds one new `[[scope]]` record in the same manifest. It sits beside the existing scope-only rebuild and is shared by the TUI and the CLI, so the rules are written once: records it did not create are carried untouched, and an existing record with that name is never replaced. Callers decide whether a record is needed by checking whether the manifest already holds one under the folded name. Having one write rather than two sequential saves is what makes "Esc writes nothing" and "the CLI never half-writes" hold.

### 2. The CLI flags and refusals

depends_on: [1]

`warlock scope add` gains `--team`, `--review-state` and `--label`. For a name with no record, all three are required and must not be blank. For a name that already has a record, any of them is an error rather than being ignored, so a `--team` that would have done nothing is caught. The boundary is still asked first. The exit status for the two new refusals was not decided. `HEADLESS-CLI.md`'s section on writing is updated to describe the flags and both refusals.

### 3. The TUI's second popup

depends_on: [1]

After the `s` popup's name passes `validate_scope`, and only when the manifest has no record for it, a second popup opens for team, review state and label. It refuses a blank field the way the first popup refuses a bad name: under the field, keeping what was typed. Submitting saves through slice 1. Esc closes it and writes nothing. A name that already has a record, and an empty name that clears the scope, take today's path unchanged. The field order and wording of the popup were not decided.
