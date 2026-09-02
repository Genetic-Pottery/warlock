# Decision: un-pacting refuses on a descendant's closed scope

**Un-pacting must refuse when any entry at or below the target carries a scope
this machine's sigils do not open**, because an un-pact is the one act in
warlock that destroys a boundary, and a boundary that can be destroyed by
standing outside it and aiming at its parent is not a boundary.

This reverses the rule shipped today and preserved deliberately by brief 13.
It is a decision about the rule only: no source file is changed by the ticket
that lands this note.

## What the code does today

Three facts, and the decision follows from them.

`session::closed_scope` (`crates/warlock-tui/src/session.rs` ~line 372) asks
`scope_covering` about the **selected row and nothing else**, and
`Opened::new` (`crates/warlock-tui/src/edits.rs` ~line 206) asks
`scope_covering` and `scope_opens_to` once about the **target path and nothing
else**. Coverage walks *up* — nearest ancestor wins — so neither door has ever
looked down.

`at_or_below` in `crates/warlock-engine/src/pact.rs` (~line 1424) begins
`selected == ROOT_MODULE`, so `unpact_subtree(".")` drops **every entry there
is**. `unpact_subtree`'s own docs say the entry is a scope's only home and
dropping the entry is dropping the scope.

So today: on a machine holding no `data-plane` sigil, `p` on `crates` — or
`warlock unpact crates` — is refused only if `crates` itself is scoped shut. If
`crates` is unscoped and `crates/engine` is scoped `data-plane`, the boundary
goes. And because a repository root is usually unscoped, `warlock unpact .`
erases every scope in the repository from a machine that holds none of them.

## Why refuse

**The guard exists for exactly this and today it is stepped over by accident.**
The sigil block was added (2026-08-31) not as security but as protection against
hitting something by mistake — a speed bump. The hazard `pacting.rs` names in
`pact_press`'s third-refusal doc is that a fumbled `p` un-pacting-ward "does not
merely undo — it costs a full model pass to put back and does not bring the
boundary back with it". That hazard is not smaller one directory up; it is
strictly larger, because the subtree is bigger. A speed bump you drive around by
aiming at the parent is not one, and nobody aims at the parent *in order* to
evade it — they aim at the parent because that is the row the cursor was on.

**Un-pacting is the only warlock act with no diff to answer for.** A pact or a
refresh across a scope spends model time and rewrites `WARLOCK.md` files inside
somebody else's subtree, and every one of those files lands in a diff a reviewer
sees and `git checkout` undoes. An un-pact removes the record that the boundary
existed. There is nothing left to review, and the person who held the sigil
learns about it when their directory turns grey.

**Nearest-ancestor-wins is not an argument against this.** That rule (brief 09)
decides *which* boundary governs a path: an outer scope is a default for paths
nothing nearer covers, not a second gate to accumulate. It answers "whose is
this". The descendant question is a different one — "what does this act reach" —
and an unscoped ancestor answers the first question with "nobody has said", not
with "everything below is yours".

**The cost is small and lands in the right place.** What becomes harder is
un-pacting a subtree containing boundaries you do not hold, which is the act
this whole feature exists to make harder. The remedy is the remedy warlock
already has: un-pact the parts you hold, or hold the sigil.

## The rule binds the un-pact direction of `p`, and nothing else

`p` un-pacting-ward, and `warlock unpact`. Not `r`, not `s`, not `p`
pacting-ward.

The rule follows **destruction, not traversal**. `pact_subtree` and
`refresh_subtree` provably leave every scope as they found it — the engine's own
tests say so (`a_pact_over_a_parent_of_a_scoped_module_keeps_every_scope`,
`a_refresh_leaves_every_scope_exactly_as_it_found_it`) — and `s` writes one
scope onto the selected row's entry and reaches nothing below it at all. Only
`unpact_subtree` takes a boundary with it.

Extending the rule to `r` would also break the key. A refresh at the repository
root is the ordinary gesture — it is most of what `r` is for — and in a scoped
monorepo it would refuse for everyone who does not hold every sigil in the
repository. That would make the feature worst for the team that adopted it
hardest, which is the failure brief 09 was written to avoid. Refusing
`unpact .` costs nothing comparable: un-pacting a whole repository is rare and
drastic, and there is a per-subtree way to do it.

So the existing `closed_scope` — shared by `p`, `r` and `s` — **does not
change**. The descendant rule is a second, separate check, asked only where the
direction of the press is already known.

## The sub-questions, settled

**`warlock unpact .` in a repository whose root carries no scope but holds
scoped entries below.** It refuses, if any of those descendant scopes is closed
to this machine. If every descendant scope is open, or there are none, it
proceeds and drops everything exactly as it does today. The root being unscoped
buys nothing: it is the absence of a statement, not permission over the
statements below it. The same is true of `p` on the root row.

**A scope the target itself opens does not license the descendants.** If
`crates` is scoped `engine` and this machine holds `engine`, but `crates/engine`
is scoped `data-plane` and it does not, the un-pact is refused. Passing the
first check is not permission for the second; they are different questions.

**A descendant scope `validate_scope` would refuse does not block.** The
blocking set is exactly the set `scope_covering` would answer with: a scope that
does not validate is read as no scope everywhere in warlock (`valid_scope_on`,
`crates/warlock-engine/src/scope.rs` ~line 311), and one place reading it
differently would make the boundary two rules instead of one. Concretely: for
each entry at or below the target, take the scope written on it, drop it unless
`validate_scope` accepts it, and ask `scope_opens_to`.

This is deliberately *not* what `unpacted_line` does on the success path, which
names an invalid scope exactly as written because it is a word somebody put in
the file and omitting it would be warlock deciding it did not count. Saying a
word is there and refusing in its name are different acts: the first is a
report, the second is enforcement, and enforcement uses the engine's one answer
about what a scope is.

**What the refusal names, and in what order.** Every distinct blocking scope,
deduplicated, in the order first met walking the entries in manifest file order
— the same order `unpacted_line` already walks to name what it dropped. Not the
paths. The scopes are what a reader would have to hold to proceed and there are
few of them by design (a boundary is architecture); the paths carrying them are
unbounded, and `warlock check <path>` is the thing that locates them. Naming
one scope and stopping is rejected: it turns a refusal into whack-a-mole, where
each sigil obtained reveals the next.

## No override, either way

No `--force`, no flag, no environment variable, no config key that weakens this.
`warlock config` is the one road, as it is for every other boundary refusal —
an escape hatch flag ends up in a script and is never read again. The escape is
to hold the sigil, or to un-pact the parts you hold.

## The two doors must agree

`session::closed_scope` (`crates/warlock-tui/src/session.rs` ~line 372) and
`Opened::new` / `Opened::unpacted` (`crates/warlock-tui/src/edits.rs` ~lines 206
and 293) are the two doors onto one rule. **Neither may refuse where the other
permits.** A `p` on a pacted subtree and a `warlock unpact` of the same path
give the same answer, in the same words, on the same machine.

Where each side asks it is settled by what each door is shared with:

- In the TUI the descendant check goes on the un-pact path in `pacting.rs`, not
  inside `closed_scope`, because `closed_scope` is also `r`'s and `s`'s and they
  must not change.
- Headless, it goes in `Opened::unpacted`, not in `Opened::new`, because
  `opened` builds an `Opened` for `scope add` and `scope remove` too. Asking it
  from the same dropped-entry set the success line is built from keeps "what
  would have gone" and "what blocks" one list by construction. Nothing is saved
  when it refuses.

This does not weaken the ordering property in the `edits.rs` module docs — that
the boundary is asked before the spelling and before any look inside the
manifest, so a closed boundary cannot answer questions about a manifest a reader
may not be in. That property is about the *target*, and the target's own check
still comes first, unchanged. The descendant check runs only after the target
said yes, and what it discloses is scopes, which are committed and visible to
everyone who clones the repository. There is nothing there to leak.

## Prose that this decision makes wrong

Recorded here so the implementation does not have to find it. None of it is
touched by the ticket that lands this note:

- the "blast radius" section of `crates/warlock-tui/src/edits.rs` module docs
  (~lines 66–79), which says the wide radius is deliberate and unchanged;
- `pact_press`'s "third refusal" doc in `crates/warlock-tui/src/pacting.rs`
  (~lines 849–865);
- `unpact_subtree`'s docs in `crates/warlock-engine/src/pact.rs` (~line 1350);
- `docs/warlock-brief-13-a-headless-cli-for-warlock-s-own-operations.md`,
  lines 25 and 37, which promise the cheap writes mirror today's rule. Brief 13
  shipped that rule as stated and correctly; this note supersedes it, and the
  brief is a record of what was decided then, not a claim about now.

Out of scope of this decision and unchanged by it: the manifest format,
`refresh`'s filtering, and what `scope_covering` and `scope_opens_to` answer.
