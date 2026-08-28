# Red brief: Scopes and sigils

Section 8 of the design doc says a pact is the boundary of what Warlock
manages, and handles work reaching an unpacted module as a pact expansion
proposal a human blesses. Read that against the greenfield path two paragraphs
above it, which says to pact everything on day one. Pact everything and there
are no unpacted modules; no unpacted modules means nothing ever crosses
anything; nothing crossing means all three of section 8's checkpoints pass
everything forever, and the diagnostic the section ends on — "auth and api have
crossed in nine of your last ten tickets" — can never fire. **The boundary
evaporates for the team that adopted hardest**, and it does it silently, by
being satisfied rather than by breaking.

One line is doing two jobs. *What Warlock documents* and *what this piece of
work may touch* are different sets that coincide only while coverage is partial,
and this project separates them. A directory can carry a **scope**: a string,
committed in the repository, visible to everyone who clones it. A person holds
**sigils**: strings on their own machine, entered once. A directory has at most
one scope and a person may hold many, and that asymmetry is the design rather
than an accident of it — a boundary is one answer to "whose is this", while a
person can be on two teams. Any one sigil that matches a directory's scope opens
it. A directory with no scope is open to anyone. A person with no sigils is
unrestricted, which is every warlock user there is today and every one who
ignores this feature tomorrow.

**A scope is a term of a pact, and it is not entered at pact time.** Those are
two different statements and the brief needs both. A pact is what says *the AI
works here*; a scope adds *and whose AI*, which is why a scope only ever sits on
a pacted directory and dies with the pact it qualifies. An unpacted directory
has already answered the first question with no, so there is nothing for a scope
to say about it.

But asking for the scope *at* pact time welds two lifecycles that should not be
welded. A pact is bought — a model pass per directory, time and money — while a
scope is typed, and a team that pacts its whole repository on day one, which is
the adoption story section 8 wants most, would then be able to draw a boundary
only by un-pacting and re-pacting in the right order, paying for a hundred model
passes to record a string. So the scope gets its own key. Pacting never asks
about it, and setting one costs nothing and starts nothing: pact everything on
Monday and draw the boundaries on Friday, in that order or any other, as often
as you like.

You onboard to a team the way you onboard to AWS. Somebody hands you your
sigils, you run `warlock config` and paste them in, and they live on your
machine because what you hold is *identity*. Scopes go the other way: committed,
because a boundary only some people can see is not a boundary, it is a rumour.
Reversing either home is the main way this gets built wrong.

Nothing here enforces anything. No check, no warning, no refusal, no hook, no
CI — not because enforcement is unwanted but because section 3 already fixed its
shape ("warn, never block") and the place to put it is a non-interactive entry
point that does not exist yet. What ships is the record and the vocabulary:
scopes typed, committed and drawn in the tree; sigils configured and printed;
and both explained in the `AGENTS.md` that brief 08 taught `warlock init` to
write.

That last clause is the difference between this brief and one that ships
nothing. A Claude Code session in a warlocked repository already reads
`AGENTS.md`. Tell it there what a scope is, that `.warlock/pacts.toml` records
them and `warlock config` prints what the operator holds, and the workflow this
is all for arrives before a line of code enforces it: you ask an agent to draft a
task, it looks at the scopes on the directories that task would touch, and it
says *this crosses into `data-plane` and you do not hold it — narrow the task or
go get the sigil*. The soft gate is a social gate. A social gate is made of
words in a file the reader's agent already opens, and its whole enforcement
mechanism is the PR reviewer, exactly as section 8 says it is for over-pacting.

Read `docs/warlock-design-doc.md` before drafting, sections 3, 4 and 8 in
particular. Section 8 gets the amendment above rather than a footnote — the
paragraph on crossings currently assumes unpacted modules are the only way to
cross, and that assumption is the bug. Section 4's three sources of truth has no
home for "who am I and what do I hold", which is operator config rather than a
fact about the system, the work, or why a call was made; it needs a paragraph
saying why that is deliberately not a fourth source rather than an oversight.

**The scope is the first fact in the manifest that a run does not own, and the
write path is not shaped for that.** `pact_subtree` builds a whole `PactEntry`
per directory and `rewrite` swaps it in over the existing one, which is
lossless today for the accidental reason that an entry holds exactly the four
things a run produces. Add a fifth that a person typed and the next refresh
erases it — and so does pacting a parent of a scoped module.

That is not a hazard to note and test around. A test catches it once; the shape
that caused it stays, and the next field added to that struct walks into the
same hole. So the first slice changes the shape, with no behaviour change and
before the scope field exists: **a run stops being able to build an entry.** It
produces an outcome — the module, the document, and the grant it earned or did
not — and `rewrite` applies outcomes to the manifest, writing the fields a run
owns and touching nothing else. It is enforced by the parameter type rather than
by care, so a scope cannot be dropped by a run in this brief or in four briefs'
time, and `rewrite`'s own doc comment — "everything else left exactly as it
was" — becomes true at field granularity as well as entry granularity, which is
what it has always read as promising.

The one write still allowed to lose a scope is the un-pact, which loses it the
way it already loses a grant, and for the reason above: the pact it was a term
of is the thing being torn up.

## Outcome

Press `s` on a pacted directory, green or yellow. A window asks for a scope and
says what one may look like, with an empty field. Type `control-plane`, press
Enter, and the directory carries it — immediately, with no run, no model pass
and nothing to wait for. Press `s` again and Enter on an empty field and it is
gone. Press it on a gray row and it refuses and names `p`, because a scope says
whose AI may work here and an unpacted directory has already said no AI works
here at all.

The tree shows it: `warlock-tui (tui-team)` on the row it was typed on, in the
colour that row already had. The directories underneath are covered by it and do
not repeat it, so a scoped module reads as one labelled row rather than forty.

Pact the whole repository first and scope it afterwards, module by module, as
the teams get decided. Nothing about a pact asks you about scopes and nothing
about it is slower, and a refresh over a scoped subtree leaves every scope in it
untouched — on the directories it re-described and on the ones it skipped alike.
Pacting a parent of a scoped module does not disturb the child either: you
answer for the row you pressed `s` on and for nothing else.

Un-pact a scoped module and the scope goes with it, along with the documents and
the grants, on that row and every row beneath it. That is the same act saying
the same thing twice: warlock no longer works here, so there is nothing left to
say about whose AI may.

Open `.warlock/pacts.toml`. The scope is in the entry for that module, committed
beside its document and its grant. Hand-edit it — delete a scope, add one to a
module nobody has scoped yet, review the diff in a PR — because this is a
committed text file like `.warlockignore` and the boundary map is exactly the
kind of change that should be argued about in review.

Run `warlock config`. It names the repository it resolved, prints the sigils
held for it, prints the rules a sigil follows, and reads one line. Type
`data-plane, control-plane` and that is what you hold, on this machine, for this
project, in a file no repository will ever contain. Run it again and it tells
you the same thing back.

Start a session in the repository and ask an agent to plan a piece of work. It
has read `AGENTS.md`, so it knows to look at what the directories it would touch
are scoped to and what you hold, and to say so when the two do not line up —
before there is a ticket, and with nothing anywhere in warlock stopping anybody
from carrying on.

## Success criteria

- **No code path lets a pact or refresh construct a whole `PactEntry`.** A run
  produces outcomes — module, document, and the grant it earned or did not —
  and the function that folds them into the manifest is typed to accept only
  those, so a run cannot supply a field it does not own. This is checked by the
  compiler, not by a reviewer.
- Applying an outcome to an existing entry writes every run-owned field and no
  other field. A grant the run did not earn is written as *absent*, so partial
  completion still leaves a directory pacted and unjudged exactly as it does
  today — the rule is "run-owned fields are always written, person-owned fields
  are never touched", not "keep whatever is not mentioned".
- The whole of slice 1 is behaviour-identical: the manifests it writes for every
  existing test are byte-for-byte what they were before it.
- A `PactEntry` carries at most one scope, an optional string beside the module,
  the document and the grant. An entry with none serialises byte-identically to
  today, so adopting the feature churns no line of an existing manifest and this
  repository's own file does not move until a scope is typed.
- The manifest schema stays at version 1, and a manifest written before this
  brief loads with no scopes and no complaint.
- **A scope exists only where a pact exists**, and the type says so rather than a
  rule enforcing it: there is nowhere in the manifest to write a scope for a
  module that has no entry.
- A refresh, a pact over a parent of an already-scoped module, a cancellation
  mid-run and a partial completion all leave every scope exactly as they found
  it, on the directories the run described and on the ones it skipped. There is
  a test comparing a manifest's scopes before a run against its scopes after,
  and it is a regression guard rather than the mechanism — the mechanism is that
  a run has no way to say anything about a scope.
- Pacting a parent never changes or drops a child's scope, and no run ever sets
  one.
- Un-pacting drops the scope on the directory and on every directory beneath it,
  with the entries it drops — the pact the scope was a term of is the thing
  being torn up. That is deliberate, and it is written in the module's own
  narrative rather than discovered.
- Setting or clearing a scope leaves every grant, hash and timestamp exactly as
  it found them, so a boundary is never bought or lost with a freshness change.
- One validator in the engine decides what a scope string may be, and the
  prompt, the hand-edited manifest and `warlock config` all reach it. The rules:
  1 to 24 characters; ASCII lowercase letters, digits, `-` and `_`; begins with
  a letter; does not end with a separator. Input is lower-cased before
  validation, so `Data-Plane` and `data-plane` are never two boundaries.
- `*` is rejected as a directory scope — blank already means open there, and two
  spellings of one meaning is how a vocabulary rots — and accepted as a sigil,
  where it means "may work anywhere".
- The engine answers "what scope covers this path" with the scope on the nearest
  ancestor at or above it that has one, and nothing else. A directory with its
  own scope answers itself; an outer scope is a default, not an extra gate. One
  function, tested against a nested case where an inner scope overrides an outer
  one, and against a path no scope covers.
- `Node` carries the scope written on its own directory, not an inherited one,
  defaulted on deserialisation, in the way `ignored` already is.
- `s` on a pacted directory opens the prompt. On an unpacted one it refuses and
  names `p`; on a file row it refuses the way `p` already does. Refusals go
  where the pact key's refusals go.
- The field opens holding the scope that directory already has, cursor at the
  end, so the prompt is an editor for a value rather than a question asked twice.
- Enter writes what is in the field and closes. An empty field clears the
  directory's scope. Esc closes and changes nothing.
- The field takes one scope. A comma, a space or anything else the validator
  rejects is a refused submit with the rule named, not a second scope quietly
  accepted.
- Invalid input refuses to submit, keeps what was typed, and says which rule it
  broke. The rules are on screen before anything is typed, not only after a
  mistake.
- Setting a scope writes the manifest and repaints, with no model pass, no
  worker and no progress line. It is not a run and nothing about it can be
  cancelled, because it is over before there is anything to cancel.
- `s` during a pact or refresh refuses and says so, for the same reason a second
  `p` does: one writer to the manifest at a time.
- `p` and `r` never ask about a scope, and never write one.
- The label is drawn after the directory name in parentheses, in the row's own
  colour, so no colour gains a second meaning and there is still no fourth. It
  is one word, because a directory has one scope.
- The label is the first thing dropped when a row is too narrow. A directory
  name is never truncated to make room for a scope.
- File rows carry no label, and neither do directories covered only by an
  ancestor's scope.
- `warlock config` resolves the repository root the way `warlock init` does and
  refuses outside one with a line.
- It prints the project it resolved, the sigils held, and the rules, then reads
  one line: that line replaces the set entirely, a blank line clears it, and EOF
  or Ctrl-C leaves it untouched. The screen says all three before the cursor,
  because a prompt whose blank answer destroys something has to say so first.
- Sigils live outside the repository at `~/.warlock/<project>/config.toml`,
  written with the same write-and-rename idiom as the manifest. Nothing commits
  them, offers to commit them, or writes them anywhere inside a repository.
- Holding nothing is the default and means unrestricted — what every existing
  user gets by changing nothing.
- The TUI states the sigils held for the repository once, as a fact, and does
  nothing else with them: no colour, no filter, no sort, no warning, no refusal.
- The keys line carries `s` and still fits, and the way out is still on it.
- `warlock init` writes the scopes and sigils section into `AGENTS.md`; running
  it twice still leaves the file identical to running it once, and a repository
  initialised before this brief gains the new text on the next run without a
  line outside the delimiters being touched.
- The design doc says what a scope is and what a sigil is, section 8 says what
  they fix, section 5 says a scope is a term of a pact rather than a second kind
  of thing attached to a directory, and section 4 says why holdings are
  deliberately not a fourth source of truth.
- `warlock-engine` still opens no sockets and spawns no subprocesses, and its
  tests still run with no terminal, no network and no `claude` binary. The home
  directory is reached through an explicit path the tests supply, so no test
  reads or writes the developer's real one.
- The gate — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` — stays clean.

## Constraints

- Everything from briefs 01 through 08 still holds: stable Rust 2024, system
  toolchain and no `rust-toolchain.toml`, Apache-2.0, no `unsafe`, no HTTP
  client, no Anthropic SDK, no mocking framework, no async runtime. The
  dependency edge still runs TUI to engine and never back.
- **No new dependency.** The prompt is a ratatui widget drawn the way the quit
  confirmation is drawn, the config file is `toml`, the home directory is
  `std::env::var`, and `warlock config` is a second string compared against
  `args().nth(1)`. Still not an argument parser, and not a line-editor crate:
  the field takes characters, backspace, and the two keys that end it.
- **A run writes the fields a run owns, and no others.** The document and the
  grant are derived by warlock and belong to the run; the scope was typed by a
  person and does not. That split is the manifest's shape from now on, enforced
  where a run hands its results over rather than remembered at each call site,
  and every field added to `PactEntry` after this has to land on one side of it
  or the other. Brief 08's deferred per-directory hash is run-owned and would
  land on the outcome; a scope is person-owned and would not.
- **A scope is a term of a pact, and never entered at pact time.** Both halves
  are load-bearing and they pull in opposite directions, which is why they are
  one constraint. A scope lives on a pacted directory, is stored in that
  directory's entry, and dies when the pact does — there is no scope on a gray
  row and no way to write one. And no code path *sets* a scope as part of a run:
  pacting does not ask, refreshing does not ask, and if a diff ever has a scope
  being chosen inside `pact_subtree`, the coupling this brief is arranged
  against has arrived through the front door.
- **Nothing is enforced, warned about, or refused.** No scope stops a pact, an
  edit, a refresh or a keystroke. Section 3 governs, and the enforcement half
  belongs with the non-interactive entry point that does not exist yet — putting
  a warning only in the TUI would be a warning shown where the editing is not
  happening.
- **Scopes are committed, sigils are local.** A scope in a machine-local file or
  a sigil in the repository is the failure mode this whole design is arranged
  against. There is no repository file listing who holds what, and there never
  will be.
- **Defaults are permissive in both directions.** No scope on a directory means
  open; no sigils configured means unrestricted. A user who never presses `s`
  and never runs `warlock config` sees nothing change.
- **A directory has at most one scope; a person may hold many sigils.** The
  multiplicity is on the holder's side and nowhere else. A boundary is one
  answer to "whose is this", and a directory offering two answers is a boundary
  that has not been drawn yet. Any one held sigil that matches the directory's
  scope opens it, so somebody on two teams works in both without either
  directory being ambiguous about which team it belongs to.
- **Nearest ancestor wins; scopes do not accumulate down the tree.** A directory
  under a scoped root and inside a scoped module is covered by the module, not
  by both. The alternative — needing every scope on the path, the way unix needs
  execute on every directory — makes scoping the root a tax on every piece of
  work in the repository forever, whose only stable outcome is everybody holding
  the root scope until it means nothing, at which point the outer scope is
  decoration. It also spoils the warning, which is the only thing any of this is
  for: "this crosses `data-plane`" names a boundary somebody can act on, while
  "you are missing `ai-team` and `platform` and `data-plane`" on a directory
  four deep is noise. A team that genuinely needs "both" spells it as one
  scope.
- **Still three colours, and no fourth.** A scope is a label in the row's own
  colour, exactly as `ignored` is a fact about a node rather than a state. It
  adds a word to a row, never a shade, a marker, or a meaning to a colour that
  already has one. A gray directory with a scope is an ordinary sight and not a
  contradiction.
- **Warlock never invents or infers a scope.** No guessing from directory names,
  no suggestion from a model pass, no default inherited into a new record. A
  scope exists because a person typed it into the prompt or wrote it in the
  manifest by hand, and it stops existing because a person cleared it or tore up
  the pact holding it. Those two are the only ways out.
- **The manifest stays at version 1**, and the rule for next time is worth
  writing down while this one is free: an optional field that an older build
  would reject only on a downgrade, against a population of zero published
  builds, does not earn a version bump and a two-version reader kept forever.
  The next schema change that an older build cannot read bumps the version.
- **One validator, one vocabulary.** The prompt, the manifest reader and
  `warlock config` do not each decide what a legal string is. Two validators
  drift, and the day they disagree is the day a scope typed in the TUI cannot be
  read back off disk.
- **Nothing new is fatal.** An unreadable config file, an unwritable one, an
  invalid scope in a hand-edited manifest: each is a line the reader can act on,
  and none of them is a way out of the event loop. Section 3 of the design doc
  still governs.
- **The keys line gains a key**, and brief 08 already documented it as wider
  than eighty columns before `r` went on it. The slice that adds `s` is the
  slice that has to say how the line still fits, and the way out stays on it.

## Out of scope

**Enforcement, in every form.** No matching of sigils against scopes at any
decision point, no crossing warning, no `warlock check`, no pre-commit hook, no
CI job, no exit code. The check has to live where the editing happens, and the
prerequisite for that is the non-interactive entry point still sitting in the
backlog behind the refresh pass. This brief exists so that when the check is
written there is something for it to read.

**Crossings as records, and the architecture diagnostic.** "Control plane and
data plane crossed in nine of your last ten tickets" is the payoff section 8
promises and it needs crossings to have happened and been recorded, which needs
matching, which is the paragraph above. Nothing here counts anything.

**Scopes that accumulate, and any way to say "needs both".** Covered in the
constraints and deliberately not built. If it turns out to be wanted, it is a
change to one resolver function and a paragraph in the design doc, and it should
be made on evidence of somebody wanting it rather than on the appeal of the
metaphor.

That is not the same thing as work needing two sigils, and the matcher this
brief is written for must not confuse them. A piece of work touching
`crates/control-plane` and `crates/data-plane` crosses **two boundaries** and
wants a sigil for each: two directories, two questions, each answered by any one
held sigil. Work touching a single directory that sits under a scoped root and
inside a scoped module crosses **one** boundary, answered by the nearest scope,
and that is the case accumulate would have charged twice for. The conjunction
lives in the footprint, never in the boundary — one directory never asks for two
sigils.

**Colouring, filtering or sorting the tree by what you hold.** A "only what I
can touch" filter is a plausible view and a poor first one: it needs matching,
and it hides rows on the basis of a soft gate that anybody may sidestep, which
is a hard wall painted to look like a view.

**Drawing the boundary map before adoption.** A scope needs a pact under it, so
a lead cannot lay out who owns what across a repository warlock does not manage
yet. That is the intended reading of what a pact is — it says the AI works here,
and a scope only qualifies *whose* — but it does mean the boundary arrives with
coverage rather than ahead of it, and on the scoped-adoption path of section 8
that is one module at a time. If a team ever needs the map first, the answer is
a committed file of its own with a path-matching format, and it should be built
when somebody asks rather than in anticipation.

**Setting a scope on a subtree in one press.** `s` scopes the directory it is
pressed on. Recursion is what pacting does because a pact is per directory and a
scope is not: one scope covers everything beneath it already, so a recursive
apply would write the same string forty times to say what one row says.

**Sigils arriving from anywhere but a person's hands.** No team file, no import,
no fetch, no `warlock config --from-url`, no reading them out of a repository.
You are handed strings and you paste them.

**`*` doing anything.** It is defined, validated and storable so the vocabulary
is settled while it is cheap, and it is matched by nothing, because nothing
matches yet.

**Per-file scopes.** A module is a directory (section 5), and a file has no
state of its own for the same reason it will have no boundary of its own.

**Ownership in git's sense.** A scope is not CODEOWNERS. No review routing, no
notifications, no claim that the named team wrote or must approve anything.

Also out, unchanged from brief 08: telling direct staleness from inherited
staleness; reconciliation, diffs and "still holds"; configurable staleness;
cache eviction; making the caps configurable; blessing and the gate; pact
expansion proposals and footprint prediction; Linear and Red/Forman integration;
the journal at `~/.warlock/<project>/<date>`; the pre-push guard; the CI
validator; `warlock status` and every other non-interactive entry point that
reports freshness; the paid crate; and publishing anything.

## Scope

### 1. A run may not build an entry

depends_on: []

A refactor with no behaviour change, landing before the field it protects
exists. Today phase two of `pact_subtree` constructs a `PactEntry` per directory
and `rewrite` swaps each one in over whatever sat there — and the doc comment on
`rewrite` already says "everything else left exactly as it was", which is true
of *entries* and false of *fields*. Nothing is broken by that yet, for the
accidental reason that an entry holds exactly the four things a run produces.
The scope is the first thing it will hold that a run does not, and the fix is to
make the accident into a rule.

A run's product becomes an outcome rather than an entry: the module, the
document it wrote, and the grant it earned — `Some(hash, timestamp)` when it
hashed cleanly, `None` when the directory was left pacted and unjudged. That is
everything phase two actually knows, and it is a smaller type than `PactEntry`
precisely because a run knows less than the manifest holds.

`rewrite` then takes outcomes instead of entries, which is where the enforcement
is. It is not that callers are careful; it is that the parameter will not accept
a whole entry, so there is no expression a future slice can write that drops a
person-owned field. For each covered module: an outcome with an existing entry
updates that entry's run-owned fields in place, an outcome with no existing
entry becomes a new one, and a covered module with no outcome loses its entry —
which is the un-pact and partial-pact honesty the function already provides, and
it is unchanged.

**Applying an outcome writes every run-owned field, including writing a grant to
absent.** This is the part to get right, and the reason the split is a type
rather than a "merge the fields that are set" helper: a partial pact must clear
the grant it did not earn, or a directory that failed to describe would keep
yesterday's hash and draw green. So the rule has two halves and both are
absolute — run-owned fields are always written, person-owned fields are never
touched.

`PactEntry::new` and `with_grant` stay as they are, since the tests and doctests
are full of them and a general constructor is not the problem; the guarantee
comes from what `rewrite` will accept, not from making entries hard to build.

The whole slice is invisible from outside: every existing test passes unchanged,
and the manifests written for each of them are byte-for-byte what they were.
That is the acceptance condition, and it is what makes this safe to land ahead
of a feature — if a manifest moves, the refactor did something it was not asked
to do.

### 2. The scope on the entry

depends_on: [1]

One optional field on `PactEntry`, beside the module, the document and the
grant. A scope qualifies a pact, so it is stored in the pact's own record, and
the invariant the whole feature rests on — no pact, no scope — is a property of
the type rather than a rule somebody has to remember to enforce. There is
simply nowhere to write a scope for a module that has no entry.

`deny_unknown_fields` is already on both manifest structs, which is what makes
the version note in the constraints a decision rather than an oversight: a build
from before this brief will refuse a manifest carrying scopes, and that is
acceptable exactly once, now, while the only manifests in the world are in this
repository. The field is `#[serde(default, skip_serializing_if)]` on the way
out, so an unscoped entry writes the bytes it writes today and this repository's
committed manifest does not move until a scope is typed.

**Surviving a run costs this slice nothing, and that is the point of the one
before it.** The field is person-owned, a run hands over outcomes, and an
outcome has nowhere to put a scope — so a refresh, a pact over a parent, a
cancellation and a partial completion all leave it alone because none of them
can express touching it. Write the tests anyway, across all four, as regression
guards: they are what fails loudly if somebody ever widens the outcome type, and
that is the failure worth catching early rather than the scope itself.

`unpact_subtree` is the one place allowed to lose a scope, and it loses it the
way it already loses a grant: entries at and below the un-pacted directory go,
and their scopes go with them. Nothing here special-cases that, because nothing
has to — the entry is the scope's home, so dropping the entry is dropping the
scope, and the semantics fall out of the storage decision rather than being
maintained on top of it. It still wants a line in the module's narrative,
because it is the difference between a boundary that quietly evaporated and one
somebody chose to tear up.

Validation is a free function over a string, returning the normalised scope or
the rule it broke. Lower-casing happens before the check rather than after,
because `Data-Plane` and `data-plane` must never be two boundaries and the
cheapest way to guarantee that is for only one of them to be representable. The
same function serves the sigil side, where the *set* is validated one string at
a time — that is the only place multiplicity lives, and the reason the value
here is a string rather than a one-element array is that a shape which can hold
two is a shape somebody eventually puts two in.

Coverage is one function and no stored state: the scope covering a path is the
one on the nearest ancestor at or above it that has any, and a directory with
its own scope answers itself. It returns at most one scope, because there is at
most one to return — the signature is where that invariant is cheapest to state
and hardest to lose. Nearest wins rather than accumulating, for the reasons in
the constraints, the short version being that an outer scope is a default and
not a second gate. Nothing in this brief calls the function except its tests. It
exists so the matcher has one home when it is written, rather than three callers
each walking up the tree their own way.

`Node` gets the scope written on its own directory as an `Option<String>`,
populated by the loader where `state` already is, `#[serde(default)]` for the
same reason `ignored` is: a tree written before the field existed still reads
back. Its own scope and not an inherited one, because the renderer draws the
owner and the resolver answers coverage, and a node holding both would let those
two drift.

### 3. The scope key

depends_on: [2]

`s`, on a pacted directory row, green or yellow. On a gray one it refuses and
names `p` — a scope says whose AI may work here, and an unpacted directory has
already answered that no AI works here at all — and on a file row it refuses the
way `p` already does. Refusals go where the pact key's refusals go, so nothing
here invents a second way of saying no. It also refuses while a pact or refresh
is running, for the reason a second `p` does: one writer to the manifest at a
time.

What happens on Enter is a manifest write and a repaint. No worker, no channel,
no progress line, no cancel handle, nothing in the footer counting anything: it
is not a run, and building it as one would be the coupling this brief exists to
avoid, arriving through the front door instead of the back. The whole action is
read the manifest, set or clear one entry's scope, write it, redraw — and the
grant, the hash and the document on that entry come out the other side
untouched, because a boundary is not a freshness event and must never buy or
cost one.

The window is the shape the quit confirmation established: a value of its own
held by the event loop rather than a field on `App`, and one pure function
saying what a key does to it. `App` does not learn about the prompt for the same
reason it does not learn about the confirmation — an `App` before the prompt
opens and after it is dismissed is equal because nothing touched it.

It differs from the confirmation in holding text, and that is the whole of the
new work: a buffer, a cursor at the end of it, characters, backspace, Enter and
Esc, and nothing else. No history, no completion, no selection, no left-arrow
editing unless it falls out for free. This is a field for one short string, not
an editor.

The field opens holding whatever that directory already has, cursor at the end.
This is an editor for a value rather than a question asked twice, and it is what
makes clearing legible: select the row, press `s`, see `tui-team` sitting there,
delete it, Enter. Esc changes nothing, which is the answer for the reader who
opened the prompt to look. Retyping over an existing scope replaces it, because
a directory has one and the field holds one.

Content: a line naming the directory being scoped, the field, and the rules
under it. The rules are on screen before anything is typed — the point of
stating a length cap and a character set is to stop the mistake, not to explain
it afterwards. On an invalid submit the text stays, the cursor stays, and the
line under the field says which rule was broken.

One scope goes in, so there is no separator to parse and nothing to split. A
reader who types `control-plane, data-plane` gets the character-set rule quoted
back at them rather than two scopes, and that refusal is the feature: the place
to hold two strings is the sigil set, and letting the directory side quietly
grow a list is how the asymmetry this design rests on gets lost.

Then the keys line, which brief 08 left wider than eighty columns with the way
out at the end of it, and which now gains a key rather than inheriting the
problem unchanged. Decide the wording here, and keep the way out on the line.

### 4. The label in the tree

depends_on: [2]

`warlock-tui (tui-team)`, after the name, in the row's own colour. The colour is
the constraint: three colours mean unpacted, stale and fresh, and a label drawn
in a fourth would be a second thing colour says. Parentheses do the
distinguishing, and if a scoped row needs to stand out further than that, it
does not — the reader is looking at a name they typed.

Only the owning row is labelled. Covered descendants are not, because a scoped
module of forty directories with the label repeated forty times is a tree that
has stopped being readable to say one fact forty times. Coverage is learned from
position, which is how every reader already reads `.gitignore` and CODEOWNERS.
File rows are never labelled: a file has no state of its own and it has no
boundary of its own.

A gray row never carries one, and that falls out rather than being special-cased:
a scope lives in a pact entry, an unpacted directory has no entry, so there is
nothing to draw. The label appears when a module is pacted and scoped and
disappears when either fact goes away.

Truncation order is the part worth deciding in the diff rather than discovering
in a narrow terminal. The name is the row; the label is an annotation on it. So
the label goes first, whole, rather than the two of them each losing half — a
row reading `warlock-t… (tui-te…)` has damaged the thing the tree is for in
order to preserve the thing it is not.

The label is one word, since a directory has one scope, which is what keeps the
annotation from ever competing with the name it annotates.

### 5. `warlock config` and the sigils it holds

depends_on: []

The second subcommand, in the shape brief 08 gave the first: one string against
`args().nth(1)`, the repository root resolved from `.git/` the way the tree
already resolves it, a refusal with a line outside a repository, and the
ordinary screen throughout — never the alternate one.

The file is `~/.warlock/<project>/config.toml`, holding `sigils = [...]`. The
home directory comes from `HOME`, falling back to `USERPROFILE`, and a line if
neither is set. `<project>` has to be derived from the repository, and the
proposal is the root directory's name plus a short digest of its canonical
absolute path, because the name alone collides across two checkouts and the path
alone is unreadable in a directory listing. The consequence — two clones of the
same repository in two directories hold sigils separately — is correct rather
than tolerated: a sigil is a fact about this checkout on this machine, and
identifying "the same repository" across clones needs git remotes, which is a
capability warlock does not have and does not want for this.

The interaction is print-then-read, and everything it needs to say is on screen
before the cursor. It names the repository it resolved, prints the sigils
currently held, prints the rules a sigil follows, and then says what the three
possible answers do: a line of sigils replaces the set, a blank line clears it,
Ctrl-C or EOF changes nothing. A prompt whose blank answer destroys something
has to say so above the cursor, not in a manual, and saying it there is what
buys the single entry point — there is no `warlock config clear`, no `warlock
config set`, and no second spelling of anything.

Validation is slice 1's function, with `*` allowed here and rejected there. The
asymmetry gets a comment where the exception lives: blank already means open on
the directory side, so `*` there would be a second spelling of a meaning that
already has one, while on this side it is the only way to say "anywhere".

The write is write-and-rename, like the manifest and the documents. The
directory is created if absent. Nothing here ever writes inside the repository,
and nothing offers to.

The TUI shows what is held, once, where it shows facts — the sigils for this
repository, or that none are held. That is the whole of the front end's
involvement: no colour, no filter, no sort, no warning, no refusal, and no
prompt box. If a reader wants to change them they run the subcommand, which is
the same shape as everything else warlock configures.

### 6. Telling the agent, and the doc

depends_on: [2, 5]

The `AGENTS.md` body gains a section on scopes and sigils, and this is the slice
that makes the brief worth shipping before any matcher exists. The text is a
constant in the crate, reviewed in a diff like the prompts, and `warlock init`
splices it into the existing delimited section — so a repository initialised
under brief 08 gains it on the next run with nothing outside the delimiters
touched, and running it twice still leaves the file identical to running it
once.

What it says: that a pacted directory may carry one scope, that scopes are
recorded in `.warlock/pacts.toml` beside the grant and shown in the tree, that a
scope covers everything beneath it until a nearer one overrides it, that an
outer scope is a default rather than a second gate, and that a pacted directory
with no scope is open to anyone. That the operator may hold several sigils and any one of them
opens a matching scope, so "do they have access" is a membership test rather
than an expression to evaluate. That a scope is a term of the pact and not a thing beside it — an unpacted
directory cannot carry one, and un-pacting removes one — because an agent that
gets this backwards will give the operator bad advice about how to fix a
boundary. That the operator
holds sigils, that `warlock config` prints them, and that holding nothing means
unrestricted. And the instruction that does the work: before planning or making
a change, look at what the directories it would touch are scoped to and what the
operator holds, and **say plainly when the work crosses a scope they do not
hold** — narrow it, or tell them which sigil the work wants. Never refuse, never
block, never edit around it: section 3 is as binding on the agent reading this
file as it is on warlock.

What it deliberately does not say is that the agent should enforce anything, or
that warlock will. A file that promises a wall warlock does not build is worse
than no file, because the first time somebody sails through it they learn the
document lies.

The design doc gets its amendments in the same diff. Section 8 gains the
paragraph this brief opens with — that crossings defined only against unpacted
modules cannot fire at full coverage, and that scopes separate what warlock
documents from what a piece of work may touch — plus what a scope and a sigil
are, that `*` is legal on both sides, that a directory has one scope while a
person may hold many and any one of them opens it, that nesting resolves to the
nearest ancestor rather than accumulating, that defaults are permissive both
ways, and that a crossing remains an event to warn about rather than an error to
block.

Section 5 gains two things: the label in the tree, with the sentence that it
takes the row's colour, because "there is deliberately no fourth colour" is a
promise that has to survive every feature that wants to say something on a row;
and the statement that a scope is a term of a pact. That second one belongs in
the section that defines what a module is, because section 5 is where "node,
directory, pact and `WARLOCK.md` are the same thing" is asserted — a scope
qualifies that pact rather than joining the list, which is exactly why it can
only exist where the pact does.

Section 4 gets the harder edit. The three sources of truth have no row for "who
am I and what do I hold", and the right fix is not a fourth row: the journal
owns why, the repo owns what the system is, Linear owns what should happen, and
none of those is a set of strings on one operator's machine that says nothing
about the software at all. It is operator config, it is excluded on purpose, and
the section should say so in a sentence so the next person to notice the gap
finds a decision instead of an omission.
