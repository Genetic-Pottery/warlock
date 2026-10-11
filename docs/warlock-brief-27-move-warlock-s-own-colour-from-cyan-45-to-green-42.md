# Move warlock's own colour from cyan 45 to green 42

`FOCUS_COLOUR` in `crates/warlock-tui/src/colour.rs:41` is `Color::Indexed(45)`, a cyan, and `SYSTEM_COLOUR` on line 49 is defined as that same constant. Between them they carry every pixel warlock uses to speak as itself: the focused pane border (`ui.rs:696`, `ui.rs:1719`), the background of the lit workflow step (`WORKFLOW_LIT`, `ui.rs:1003`), the run header (`ui.rs:798`), and warlock's own lines in the conversation (`Voice::Warlock`, `ui.rs:1081`). The cyan is not wanted; green 42 is. That is the whole of the problem — nothing is broken, the colour is simply the wrong one.

Leaving it alone costs only the look of the screen, but two things make the change less trivial than swapping a number. First, the comment block at `colour.rs:32–49` justifies the value by hue: it opens "Cyan because it is none of the three above" and argues the pin from cyan's distance from the three node states. After the change that reasoning is false, and a false comment in this repository is evidence fed to the pass that writes `.warlock.md` documents, so it would propagate. Second, no test pins `FOCUS_COLOUR` to a number. `GUIDE_COLOUR` and `CONVERSATION_COLOUR` each have an exact-value test in `crates/warlock-tui/src/tests/colour.rs`; the focus colour has only the pairwise-distinctness assertions, which pass for 45 and 42 alike. So the change would land with nothing holding it in place, and a later edit returning the constant to cyan would go green through the suite.

## Outcome

Someone runs `cargo run` in a repository, and the border of the focused pane is green instead of cyan. They press the key that moves focus and the green border moves with it, still bold, still the only bold border. The lit step of the workflow bar is black text on a green field. The run header and warlock's own notes in the conversation are green, while the model's replies stay mauve and what the operator typed stays the terminal's default. Nothing else on the screen moves: fresh directories are still 71, stale still 214, unpacted still 245, and the indent guides still 240.

Someone reading `crates/warlock-tui/src/colour.rs` finds a comment above `FOCUS_COLOUR` that no longer claims anything about hue — it says the value is pinned and indexed like every other colour in the file, and that what it must stay apart from is the three node states and the conversation mauve, which the tests enforce. Someone who changes the constant back to 45 and runs `cargo test` sees a named test fail.

## Success criteria

**The constant**

- `FOCUS_COLOUR` in `crates/warlock-tui/src/colour.rs` is `Color::Indexed(42)`.
- `SYSTEM_COLOUR` is still defined as `FOCUS_COLOUR` rather than as a second literal.
- No other colour constant in the file changes value: `colour_for` still returns 245, 214 and 71; `GUIDE_COLOUR` is still 240; `CONVERSATION_COLOUR` is still 139.
- No literal `45` for a colour remains in `crates/warlock-tui/src`.

**The comment**

- The block above `FOCUS_COLOUR` makes no claim about cyan or about which hues the node states occupy.
- It states what survives the change: the value is pinned and indexed for the same reason the rest of the file is, and it is held distinct from the three node states and from `CONVERSATION_COLOUR` by the tests in `tests/colour.rs`.
- The note that `SYSTEM_COLOUR` shares the value, and that focus is therefore told apart by weight and position rather than by hue, is still present and still true.
- The block is rewritten as a whole rather than edited in place sentence by sentence.

**The test**

- `crates/warlock-tui/src/tests/colour.rs` contains a test asserting `FOCUS_COLOUR == Color::Indexed(42)`, named in the style of its neighbours (`the_focus_colour_is_a_flat_indexed_green`).
- Setting `FOCUS_COLOUR` back to `Color::Indexed(45)` makes that test, and only that test, fail.
- Every existing test in `tests/colour.rs` still passes unchanged: the focus, guide, conversation and system colours are each still distinct from all three node states and from each other, and `the_system_colour_is_the_focus_colour` still holds.

**The suite**

- `cargo fmt` leaves no diff.
- `cargo clippy --all-targets -- -D warnings` exits zero.
- `cargo test` exits zero with no test file other than `tests/colour.rs` modified.

## Constraints

Colours in this crate are indexed and never named. `Color::Green` or any other named variant is not available to this work; the reason is in the module header at `colour.rs:1–6` and it is unaffected by the change.

`SYSTEM_COLOUR` stays an alias of `FOCUS_COLOUR`. The one-voice decision recorded in the comment at `colour.rs:43–48` and enforced by `the_system_colour_is_the_focus_colour` is not reopened here; warlock's voice and the focused border move together or not at all.

The node state colours are not touched. `colour_for` keeps its exhaustive match with no catch-all arm, and 245, 214 and 71 keep their current values.

The call sites in `ui.rs` are not touched. `FOCUS_COLOUR` and `SYSTEM_COLOUR` are referenced symbolically everywhere, including in `tests/ui.rs`, and that is the property that makes this a one-literal change; the work may not inline the new value at any call site, in `WORKFLOW_LIT` least of all.

The rewritten comment may not be longer than the reasoning it carries. The repo's comment rule applies: no restatement of what the constant is, and no appending to the existing block.

## Out of scope

**Finding a different green.** 42 was chosen by taste, not derived. The work does not evaluate alternatives, sample terminals, or check 42 against light backgrounds — if it reads badly in use, that is a later change with its own reasoning, not a judgement call for whoever cuts this slice.

**Arguing the collision with fresh-green 71.** Green now means both "this directory is fresh" and "this is warlock talking," distinguished by brightness rather than hue. That was weighed and accepted: the comment is not to defend it, and no new test is to encode it. The existing distinctness assertions remain the only guard.

**Updating `crates/warlock-tui/src/.warlock.md`.** Its line for `colour.rs` names the constants without quoting their values, so it does not go false; and if it did, the directory turning yellow is warlock reporting on itself, which is the product working. Hand-editing a document to keep it green defeats the ledger.

**Pinning the remaining unpinned values.** `WORKFLOW_DIM`'s 245 and 236 in `ui.rs:1008` have no exact-value tests either. Adding them is unrelated to this change and would make a one-slice brief into a test-coverage pass.

## Scope

### 1. Repoint FOCUS_COLOUR to green 42, rewrite its reason, and pin it with a test

depends_on: []

What exists now: `crates/warlock-tui/src/colour.rs` holds five pinned colours. `colour_for(NodeState)` returns 245, 214 and 71 for unpacted, stale and fresh. `FOCUS_COLOUR` on line 41 is `Color::Indexed(45)`, preceded by a comment block at lines 32–40 that argues for cyan on the grounds that it is none of the three state colours, and notes that the constant is no longer worn by the border alone. `SYSTEM_COLOUR` on line 49 is `FOCUS_COLOUR`, preceded by the one-voice comment at lines 43–48. `crates/warlock-tui/src/tests/colour.rs` pins the three state values, pins `GUIDE_COLOUR` to 240 and `CONVERSATION_COLOUR` to 139, asserts those two and `SYSTEM_COLOUR` are distinct from every state and from each other, asserts `SYSTEM_COLOUR == FOCUS_COLOUR`, and has no exact-value test for `FOCUS_COLOUR`. In `ui.rs`, `FOCUS_COLOUR` is read at lines 696, 1005 and 1719, and `SYSTEM_COLOUR` at 798 and 1081; `tests/ui.rs` asserts against both symbolically at lines 3574, 3720, 5361, 5371, 5415, 5419, 5516, 5519, 5756, 5759, 5763, 5857 and 5877.

What changes: line 41 becomes `Color::Indexed(42)`. The comment block above it is deleted and written again from nothing. The hue argument goes out — it was the thing the change falsified, and the alternative we rejected was keeping it and bending it into a claim about green being distinguishable from 71, which would have been a sentence written to defend a preference. What goes in is the part that is still true: the value is pinned and indexed for the reason the module header already gives, and what it must stay apart from is the three node states and `CONVERSATION_COLOUR`, which `tests/colour.rs` holds it to. The observation that `SYSTEM_COLOUR` shares the value, so focus is read by boldness and position rather than by colour, is still true and stays — it is the kind of comment the repo's rule keeps, because an edit that gave the border its own hue to "fix" focus visibility would be plausible and wrong. The one-voice block above `SYSTEM_COLOUR` is left alone; it names no colour and the change does not reach it.

Then a test goes into `tests/colour.rs` next to `the_guide_colour_is_a_flat_indexed_gray` and `the_conversation_colour_is_a_flat_indexed_mauve`, asserting `FOCUS_COLOUR == Color::Indexed(42)`. Without it the brief has no criterion anybody could mark done: every other test in the file passes for 45 and 42 equally, so the suite would be silent about whether the work happened. With it, a later return to cyan fails by name.

What a later edit must not break: the symbolic references. The reason this is one literal and not a sweep is that no call site and no UI test spells the number out, and inlining `Color::Indexed(42)` anywhere in `ui.rs` — `WORKFLOW_LIT` is the tempting one, since it is a `const` composed of literals already — would split the constant in two and make the next colour change a hunt. `WORKFLOW_LIT` also keeps `Color::Black` as its foreground: 42 is bright enough to carry black text, and an edit that changes the foreground to compensate for the new background is undoing a decision this slice made deliberately. `colour_for` keeps its exhaustive match; adding a catch-all arm while in the file would silently draw a future fourth state in a fallback colour.
