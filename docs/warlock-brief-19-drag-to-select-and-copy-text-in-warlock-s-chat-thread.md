# Drag to select and copy text in warlock's chat thread

The chat card is the one place in warlock that shows text found nowhere else on disk: a model's answer lives in the thread card (`thread.rs`, drawn by `ui.rs`) and is gone when the session ends. Today the only way to copy from it is `m`, which turns off mouse capture (`ToggleMouseCapture` in `input.rs`, `report_mouse` in `terminal.rs`) so the terminal can select for you. The terminal selects screen cells, not text. A drag across an answer also grabs the tree pane beside it, the indent and `SAID_MARKER`/`NOTE_MARKER` glyphs `ui.rs` draws, and a hard line break wherever `wrap.rs` wrapped a line. It cannot reach anything scrolled out of view. Meanwhile, with capture on, `mouse_action` throws drag and release events away. Copying one paragraph out of a long answer means cleaning up the paste by hand, retyping it or taking a screenshot. The longer the chat gets, the more of what it produces stays stuck on the screen.

## Outcome

A reader asks warlock a question and gets a long answer in the thread card. They press the left button at the start of the paragraph they want, drag to its end, and a highlight follows the pointer across only the panel's text. The paragraph runs past the bottom of the panel, so they drag below the edge and hold. The card scrolls on its own, and the highlight keeps growing until they stop at the paragraph's last word. Meanwhile the answer below is still streaming in, and the card stops jumping to the newest text for as long as the button is held. They let go. The footer says the text was copied and how many characters. The highlight stays on the paragraph. They switch to their editor and paste: the paragraph as the model wrote it, with no marker glyphs, no indent, no line breaks where the panel wrapped and nothing from the tree pane. Back in warlock, a single click in the panel clears the highlight and focuses the panel, and their clipboard is untouched.

On a machine where the clipboard can't be reached, the same release puts a one-line failure in the footer instead, and nobody is left thinking they copied something.

## Success criteria

**Clipboard**

- `arboard` is a workspace dependency, used by `warlock-tui`, with a comment in the manifest saying why it is worth a crate, like every other dependency there.
- One clipboard handle is opened per session and kept until warlock exits. No copy opens and drops a handle of its own.
- A successful copy puts a footer line saying it was copied, with the character count.
- A failed copy puts the error in the footer as a single line through `Error::one_line`, and nothing claims success.

**Selection model**

- A selection is stored as a start and end position in the thread's text, not as screen rows or cells.
- A panel cell under the pointer maps to a position in the thread's text through the same wrapping `wrap.rs` does for display, and a test checks that for wrapped, indented and marker-prefixed rows.
- The copied text is each entry's exact text, in order, joined by one blank line between entries, with no marker glyphs, no indent and no wrap breaks. A test checks a selection inside one answer and one spanning a message, an answer and a note.
- A selection gives the same copied text before and after the panel is re-wrapped at a different width.

**Drag, highlight and copy on release**

- A left-button drag that starts on a thread-card line draws a highlight over exactly the selected text while the button is held.
- Releasing after a drag copies the selection and leaves the highlight in place.
- A press and release with no drag between them does what a press in the panel does today, and copies nothing.
- The highlight clears on the next press in the panel and when the panel swaps cards, and on nothing else.
- A drag starting in the tree, the composer, the footer or on the account or document card does nothing new.

**Auto-scroll**

- With the button held and the pointer above or below the thread card, the card scrolls in that direction on the loop's existing tick, even if the pointer isn't moving, and the selection extends with it.
- The number of rows scrolled per tick grows with how far past the edge the pointer is.
- Scrolling stops at the top and bottom of the thread and when the button is released.

**Streaming**

- While the button is held during a turn in flight, the thread card does not follow new text, and the selected text stays the same text as the answer grows.
- When the button is released, following new text resumes as it behaves today.

## Constraints

- `arboard` is the only new dependency, and the clipboard is reached only through it. No OSC 52 and no platform-specific clipboard code.
- `POLL_INTERVAL` is not changed. It also sets how fresh the footer clock and a run's progress line are, so scroll speed comes from rows per tick, not from a faster tick.
- `ui::draw` still renders a frame from its arguments alone. The selection reaches it as an argument, not as state the renderer keeps.
- The hit test stays the single source of layout for the mouse. Mapping a cell to a text position must agree with the frame that was drawn, not keep its own copy of the layout arithmetic.
- Existing key and mouse meanings do not move: `q` and Ctrl-C quit; Esc quits when nothing is running and cancels when something is; a press focuses the pane it lands in; a second click on the selected tree row folds it; the wheel scrolls the pane it is over; `m` toggles mouse capture, and with capture off the terminal's own selection works as it does today.
- Nothing about the copy is changed from what the thread holds. No labels, markers or reformatting are added to the copied text.
- The comment rules in `CLAUDE.md` apply: no module headers or narration. Comments only where their absence would let a correct-looking edit break something, such as why the clipboard handle lives for the whole session.
- `.warlock/pacts.toml` puts the `warlock-team` scope on the repository root, so every part of this change is under `warlock-team`. Which sigils this machine holds was not checked, and whoever does the work should confirm with `warlock config` before starting.

## Out of scope

- **Selection on the account card and the document card.** The document card shows a file that can be opened in an editor with `e`, and the account card is a pact run's progress log. The chat thread is the only card whose text exists nowhere else.
- **Double-click for a word, triple-click for a line, shift-click to extend.** Each one ties the mouse more tightly to wrapping and markers, and a second click already folds a tree row, so the gestures would start colliding. Drag covers copying a paragraph, which is what this change is for.
- **Selecting across panes.** Dragging from the panel into the tree, composer or footer would copy text that is either a file path or already the reader's own typing.
- **An OSC 52 fallback and copying over SSH.** OSC 52 support varies across macOS, Windows and Linux terminals, and working over SSH is not a goal. A second path would be upkeep for a case nobody asked for.
- **A copy key.** Ctrl-C already quits. Copying on release needs no key and matches the terminals and CLIs readers already use.
- **Esc to clear the highlight.** Esc quits when nothing is running and cancels a run when one is. Reusing it would let clearing a highlight quit warlock.
- **Changing what `m` does.** Handing the pointer back to the terminal stays the way to select anywhere else on the screen.
- **Claiming warlock as a whole runs on macOS and Windows.** This change picks a clipboard that works on all three, but whether the rest of warlock builds and runs there has not been checked, and this brief doesn't promise it.

## Scope

### 1. Reach the clipboard through one session-long handle

depends_on: []

Adds `arboard` and opens one handle for the whole session. Holding it for the whole session is deliberate: on X11 the copied text can vanish as soon as the handle that set it is dropped, and a handle opened per copy would pass a quick test while losing the clipboard in real use. Success and failure both land in the footer, failure through `Error::one_line`, because a copy that fails silently is worse than no copy. Still open: whether `arboard` needs a feature enabled to reach the Wayland clipboard directly rather than through XWayland, and whether it links system libraries that need packaging on NixOS. Both must be checked before this lands. It also stays open that on X11 the copied text may not outlive warlock itself unless a clipboard manager holds it.

### 2. Model a selection as positions in the thread's text

depends_on: []

Defines a selection as two positions in the thread's entries, a mapping from a thread-card cell to a position that goes back through the display wrapping, and the function that builds the copied text: exact entry text, one blank line between entries. Storing text positions rather than screen positions is what lets a selection survive re-wrapping, scrolling and new text arriving, and what makes slices 4 and 5 possible without redoing this one. It is pure code, testable without a terminal, and lands with nothing on screen using it.

### 3. Drag to highlight and copy on release in the thread card

depends_on: [1, 2]

Stops dropping drag and release events when the press began on a thread-card line. It starts a selection on the press, extends it on drag, draws the highlight, and copies on release. A press with no drag keeps today's meaning and copies nothing, so a stray click never overwrites the clipboard. The highlight stays after release so the reader can see what was copied, and clears only on the next press in the panel or a card swap. Only the thread card takes part. Everywhere else keeps its current mouse behaviour.

### 4. Scroll the thread card while a drag is held past its edge

depends_on: [3]

Uses the event loop's existing 100 ms tick to scroll the thread card while the button is held with the pointer above or below it, extending the selection as it goes. The tick fires whether or not the pointer moves, which is what a stationary pointer past the edge needs. Speed comes from scrolling more rows per tick the further past the edge the pointer is, because the tick itself isn't changing. Still open: the exact rows-per-tick curve.

### 5. Hold the thread still under a selection while a turn streams

depends_on: [3]

Pauses the thread card's following of new text for as long as the button is held, and resumes it on release. Because the selection is pinned to text positions from slice 2, new text arriving below doesn't move what is selected. This was chosen over refusing drags during a turn, which would stop a reader copying an earlier answer while a later one arrives.
