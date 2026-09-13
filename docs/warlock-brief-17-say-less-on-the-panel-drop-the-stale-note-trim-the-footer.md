# Say less on the panel: drop the stale note, trim the footer, colour the conversation

Three small things on warlock's screen currently make it harder to read.

When `/write` puts a brief on disk, the conversation card prints a line under the `wrote …` confirmation, like `. is now stale` or `docs is now stale`. `stale_line` in `crates/warlock-tui/src/writing.rs` produces it. To the reader it looks like a warning that something went wrong, but nothing did, and it tells them nothing the tree's colours don't already show. It can also be false. When the brief lands in a directory `.warlockignore` excludes, such as `docs/`, nothing goes stale, yet the line still claims something has.

The footer, built by `keys_line` from `KEYS` in `crates/warlock-tui/src/ui.rs`, lists nearly every key warlock has. That is paging, ends, focus, send and newline, leave, command, the mouse key and quit, as well as the tree keys. It is too crowded to scan, and on narrower terminals the key someone actually wants is the one that gets dropped.

On the conversation card, what the operator typed and what came back are both drawn in the same colour. A long exchange reads as one block, and you have to find where each turn starts to tell who said what.

Left alone, these make the panel noisier than the ledger it exists to show. The stale note keeps making successful writes look like failures.

## Outcome

The operator writes a brief with `/write`. The card shows `wrote docs/red-brief-17-….md — 6.1 KB` and nothing more, and the tree shows by its colours whatever changed.

The footer in the tree view reads `k/j: row    space: fold    o: pacts    f: files    p: pact    r: refresh    s: scope    q/Esc/Ctrl-C: quit`, and on a narrow terminal the quit key is still there. While a pact is running, the footer looks exactly as it does today.

In a conversation, the operator's messages appear in the terminal's ordinary text colour. Everything else on the card appears in a muted mauve, #af87af: the model's answers, its thinking and tool lines, turn endings, and warlock's own notes on that card. Scrolling back through a long thread, the operator's turns stand out at a glance.

## Success criteria

**The stale note**

- After a successful `/write`, the conversation card shows no line of the form `<module> is now stale`, whatever directory the file lands in.
- The `wrote <path> — <size>` line is still shown after a successful write.
- No test in the repository still expects a `… is now stale` line.

**The footer**

- In the tree view, the footer holds exactly `k/j: row`, `space: fold`, `o: pacts`, `f: files`, `p: pact`, `r: refresh`, `s: scope` and `q/Esc/Ctrl-C: quit`, in that order.
- `PgUp/PgDn`, `g/G: ends`, `Tab: focus`, `Enter/Alt+Enter: send/newline`, `Esc: leave, draft kept`, `/: command`, `m: mouse on`/`m: mouse off`, `v: view` and `e: edit` no longer appear on that footer.
- Every key removed from the footer still does what it did before.
- The quit key survives at every terminal width, as it does today.
- The footer shown while a pact is running is unchanged.

**The conversation colour**

- The conversation card draws every row that is not text the operator submitted in pinned 256-colour index 139 (#af87af).
- The operator's submitted messages on the conversation card are drawn in the terminal's default foreground, not a pinned colour.
- The account card and the tree are drawn exactly as before.
- A test asserts that index 139 is none of the three node-state colours, the focus colour or the guide colour.

## Constraints

- On-screen colours are pinned `Color::Indexed` values, as `colour.rs` requires. No named or truecolor values.
- Colour on the tree still means node state and nothing else. The new colour is used only on the conversation card.
- There is no fourth node state and no fourth tree colour.
- Nothing about how writing a brief affects the ledger changes: hashes, grants and staleness behave exactly as before. Only the line that reported it goes.
- No key binding is added, removed or remapped. The footer change only affects what is displayed.
- No new dependencies.
- Comments follow the repository's rule: any comment block touched while making these changes is rewritten whole against the code or deleted, never appended to.

## Out of scope

- **A pacted directory that `.warlockignore` later excludes stays stale forever.** This is a real bug, but it is a separate change. `pact_intent` refuses ignored rows before checking whether they are pacted, so not even un-pacting is possible. The agreed fix is automatic, which means watching `.warlockignore` and resolving its changes as they happen. That is engine and pacting work with its own tests and its own scope question, so it gets its own brief.
- **A footer that changes with focus or situation.** `Esc: leave, draft kept` looked like context-aware help, but it isn't, and nobody wants that feature right now. This change only removes entries.
- **Colouring the account card.** None of its text is typed by the operator, so the same rule would turn the whole card mauve and lose what the colour is for.
- **Changing the footer shown during a pact run.** It is already short and wasn't part of the complaint.
- **Documenting the keys dropped from the footer.** The operator expects to keep adjusting the footer, and where the full key list lives is not decided here.

## Scope

### 1. Remove the stale note from `/write`

depends_on: []

The line goes entirely, rather than being corrected to report staleness accurately under `.warlockignore`. The tree's colours already say what is stale, so even an accurate line would repeat them, and removing it avoids the false case without teaching the write path about ignore rules. If working out which pact a written file staled exists only to produce this line, that logic goes with it. The tests that expect the line are updated to expect only the `wrote …` line.

### 2. Cut the main footer to the tree keys and quit

depends_on: []

The footer keeps the seven keys the operator uses most, plus quit. Many tools don't list every control, and the full list was too crowded to read. Quit stays despite the cut, because someone who can't see how to leave has no way to find out, and the existing guarantee that quit outlives every width is kept. The drop order used on narrow terminals shrinks to cover only the keys that remain. The running-pact footer is left alone.

### 3. Draw everything on the conversation card except the operator's messages in #af87af

depends_on: []

Pinned index 139 joins the other pinned colours so the reader's theme can't shift it, and it is tested to be distinct from every other colour already in use. The line is drawn by who wrote the text rather than by the kind of row. The operator's submitted messages keep the terminal default foreground, and every other row on the conversation card takes the new colour, including warlock's notes there. The model's answers, activity lines and endings take it too. A pinned white for the operator's text was considered, but it would vanish on a light theme. The operator said "white", and the terminal default was proposed in its place without an explicit confirmation, so that choice is still open.
