# A cursor in the composer, and pasting that does not submit

The chat bar at the foot of warlock's panel — `Composer` in `crates/warlock-tui/src/composer.rs` — is an append-only buffer. It holds one `String` and one `bool`, and it has no cursor field at all: printable characters push onto the end, Backspace pops the last character, Alt+Enter pushes a newline, Enter submits, Esc leaves. Left, Right, Up, Down, Home, End, Delete and Insert are byte-for-byte no-ops, asserted as such by `the_movement_and_editing_keys_this_field_does_not_have_do_nothing`. So a typo four words back costs every character typed since it, and a question worth rephrasing is a question retyped.

Bracketed paste is not enabled either. `take_terminal` in `terminal.rs` does `enable_raw_mode`, `EnterAlternateScreen` and conditionally `EnableMouseCapture`, and the event loop in `main.rs` matches `Event::Key` and `Event::Mouse` with `_ => {}` for everything else. Pasted text therefore arrives as individual key events, and the first `\n` in it arrives as `Enter`. Pasting a three-line block into the composer submits line one as a message; the turn starts, the field is muted, and the remaining lines are swallowed by the mute gate and lost. The reader gets a truncated question asked on their behalf, an answer to something they did not mean, and no trace of the rest of what they pasted. Copying a snippet out of a browser or another model — the case the composer most exists for — is the case that breaks.

There is a third fact underneath both, and it is what makes this more than a keybinding. `wrapped()` in `wrap.rs` discards the space it breaks at: line 217 trims the end of the row above, line 220 trims the start of what remains, and its own doc concedes that the rows "join back up to the line, give or take the spaces broken at." The panel only draws, so it does not care. A cursor does: bytes that vanished at a word break cannot be counted through, and a space the reader typed has no cell on screen for the cursor to sit in or for Backspace to be aimed at.

Left alone, the composer stays a field you can only type forwards into, and every multi-line paste is a live chance to send half a message you did not finish writing.

## Outcome

A reader types a question into the chat bar, presses Left four times and watches the terminal cursor walk back through the words. They type a missing word in the middle; the text to the right of it shifts along and the cursor stays put after what they typed. Backspace takes the character before the cursor rather than the last one in the draft. Home goes to the start of the row, End to its end.

They paste a paragraph copied from a web page. The whole paragraph lands in the draft at the cursor, newlines and all; the field grows to six rows and stops, scrolling within itself; nothing is sent and no turn starts. They press Up and the cursor rises one row as drawn — through the visual rows of the wrapped paragraph, not over it — and the window scrolls up with it so the cursor is still on screen. Down brings them back. Enter, once, sends the whole thing.

A reader with a turn in flight pastes; the muted field takes none of it, exactly as it takes no keys. A reader looking at the tree with focus off the composer pastes; nothing happens anywhere.

## Success criteria

**Bracketed paste**

- `Event::Paste` is delivered to warlock: bracketed paste is turned on when the terminal is taken and turned off when it is restored, alongside the existing alternate-screen and mouse-capture handling.
- Pasting a block containing newlines leaves every line of it in the draft, separated by `\n`, with no turn started and nothing sent.
- A paste arriving while the composer is muted changes the draft by nothing.
- A paste arriving while focus is anywhere but the composer changes nothing anywhere: no draft, no tree selection, no mode.
- Pasted text enters through a function of its own rather than through `compose_for`, and that function can return only "still typing" — never submit, never leave.

**The cursor**

- `Composer` carries the cursor as a byte offset into the draft, and that offset is always on a character boundary.
- Left and Right move it one character, not one byte, and stop at the two ends of the draft rather than wrapping or going out of range.
- Home and End move it to the start and end of the display row it is on.
- Up and Down move it one display row — the rows as drawn, so a wrapped paragraph is stepped through visually — and hold the column where the destination row is long enough. Up on the first row and Down on the last leave the cursor where it is.
- A key that moves the cursor changes no byte of the draft.

**Editing at the cursor**

- A printable character is inserted at the cursor and the cursor ends after it.
- Backspace removes the character before the cursor and does nothing at the very start of the draft. Delete removes the character after the cursor and does nothing at the very end.
- Alt+Enter inserts a newline at the cursor like any other character.
- Enter submits the whole draft regardless of where the cursor sits, and the field that comes back is empty with its cursor at zero.
- Every one of the above holds for a draft containing multi-byte characters: no operation can leave the buffer holding part of a character.

**A wrapper the composer can address**

- The composer wraps with a function that discards nothing: concatenating its rows reproduces the draft byte for byte.
- It breaks at the same places `wrapped()` does, with the break character kept on the upper row rather than dropped.
- `wrapped()` itself is unchanged, and every row the panel draws is what it was.
- `Composer::height` and `Composer::window` are computed with the composer's wrapper, so the height asked of the layout and the rows drawn agree as they do now.
- Columns are measured with `display_width`, not character count, so a wide character occupies the cells it draws in.

**Drawing the cursor where the character is**

- There is a map from a byte offset in the draft to the row and column it is drawn at, and an inverse map from a row and column back to a byte offset. Round-tripping an offset through both returns that offset.
- The terminal cursor is placed at the mapped cell, so it is visibly between the characters that Backspace and Delete would take.
- `Composer::window` returns a run of rows containing the cursor's row, computed from the draft and the cursor rather than stored, so nothing has to be reset when a long draft is cut short.
- Moving the cursor above or below the visible rows scrolls the window to it.
- A width of zero still wraps nothing and still comes back one row, as it does today.

## Constraints

- `wrapped()` and everything the panel draws with it are not to be modified. The composer's wrapper is an addition beside it, and the panel's break rule stays the panel's.
- No new dependency. Neither the movement nor the paste path may reach for a clipboard crate, a line-editor crate or a grapheme-segmentation crate; the cursor moves by `char` boundaries, as `Composer`'s existing Backspace already does, and display width comes from the `display_width` already in `ui.rs`.
- `compose_for` stays a pure function of a key event and a composer, taking no `App`, reading no terminal and drawing nothing. The paste entry point is held to the same rule.
- The composer is still the only copy of the draft, and it still lives on `Chat` rather than on `App`, so a pact that restores the app cannot swallow half a sentence.
- Muting is still derived from the turn in flight and set in one place; nothing added here sets or clears it.
- Every key the tree binds is still swallowed while the composer has focus. Adding cursor keys must not open a route for a keystroke to reach the app underneath.
- Ctrl-C is still answered by the event loop before the composer is consulted, and a character carrying a chord modifier is still not text.
- Only `KeyEventKind::Press` acts.
- Enter and Alt+Enter keep their present meanings, and Shift+Enter still means Enter.
- The work is inside the `warlock-team` scope inherited from the repository root; `crates/warlock-tui/src` is owed a pass afterwards, and the operator's sigils were not confirmed in this conversation.

## Out of scope

- **Mouse click to position the cursor.** Deferred to its own brief. The inverse map it needs is built here and will be sitting there for it, so that brief is small; what it still owes is the hit test from a screen position to the composer's own rows, which is work about the frame rather than about the buffer.
- **Copying out of warlock, and a clipboard crate.** Bracketed paste covers reading in, over ssh and through every paste route the terminal has, without a dependency. Copying out genuinely needs a crate — and with it X11/Wayland system dependencies and, on X11, a background thread to own the selection in a program built on the loop never blocking. That cost is worth paying when something needs it, in the brief that needs it.
- **Word-wise movement and deletion.** Ctrl+Left, Ctrl+Right and Alt+Backspace are the obvious next want, and they are a second set of rules — what a word boundary is, what happens at punctuation — for a field where the draft is a few sentences long. Left and Right cover the editing this change is for.
- **The scope and write-path prompt.** `prompt.rs` is a second append-only field with the same doctrine, used by the `s` key and by the window that edits the path a brief is written to, where mid-text editing arguably bites harder. It is left alone deliberately: the expensive part of this change — a window that follows the cursor, and a wrapper bytes can be counted through — does not exist for a single-line field, and giving that work a second consumer before it is proven would make both harder to get right.
- **Selection, undo and draft history.** None of them are implied by a cursor, each is a state machine of its own, and the field is six rows.
- **Terminals without bracketed paste.** They ignore the escape sequence and go on streaming keys, so the mid-paste submit survives there. Nothing is added to detect that or to work around it.

## Scope

### 1. Bracketed paste, inserting at the insertion point
depends_on: []

Turn bracketed paste on when the terminal is taken and off when it is restored, add the paste arm to the event loop, and give the composer a paste entry point separate from `compose_for` that inserts the pasted string at the insertion point and can do nothing else.

This lands first and on its own because it is where the data loss is. Before the cursor exists the insertion point is the end of the draft, which is where typing already goes, so the slice is complete and correct as it stands and is not rewritten when the cursor arrives — the same call site simply gains an insertion point that can be somewhere other than the end. It decides that pasting is a different kind of event from typing rather than a fast sequence of keystrokes, which is what makes "a paste cannot submit" a property of the event type instead of a check somebody has to keep remembering. It also decides that the mute and focus gates apply to pastes, which the key path's gates do not cover because they only ever see key events.

### 2. A wrapper the composer can count through
depends_on: []

Add a wrapping function beside `wrapped()` that breaks where `wrapped()` breaks but keeps the break character, so its rows concatenate back to the input exactly, and move `Composer::height` and `Composer::window` onto it.

Independent of the paste, and first among the cursor work, because everything after it is arithmetic over these rows. It decides that the composer stops sharing the panel's wrapper — sharing was free while nothing had to be addressable, and stops being free the moment a byte offset has to correspond to a cell. It also decides where the cost of that divergence is paid: the panel is untouched, and the field wraps a hair differently from the prose above it, which is the visible price of the space a reader typed having somewhere to be.

### 3. The cursor, and the keys that move it
depends_on: [2]

Add the byte offset to `Composer`, build the map from an offset to a row and column and its inverse over slice 2's rows, and answer Left, Right, Up, Down, Home and End with movement and no change to the draft.

Separated from the editing below because movement is where every hard case is — character boundaries, wide characters, the column held across a short row, the two ends — and all of it is assertable on a value with no terminal anywhere near it. It decides that the cursor is a byte offset rather than a row and column, which is what keeps it meaningful when the terminal is resized and the draft reflows underneath it. Building the inverse map here rather than in the mouse brief is the deliberate choice that Up and Down should move by display row: line-wise movement would need only the forward map, and would feel broken the first time somebody pastes a paragraph and presses Up.

### 4. Editing at the cursor
depends_on: [3]

Insert typed characters and newlines at the cursor, make Backspace take the character before it and Delete the character after, and leave the cursor after what was inserted or where what was deleted had been.

Small once slice 3 exists, and last because it is the one that invalidates the existing tests asserting these keys do nothing. It decides what Enter means with a cursor mid-draft: the whole draft is submitted regardless of where the cursor sits, because the field holds one message and the cursor is about writing it rather than about choosing part of it.

### 5. The cursor on screen, and the window that follows it
depends_on: [3]

Place the terminal cursor at the mapped cell, and make `Composer::window` return the rows around the cursor's row instead of the tail.

Last, and the only slice that touches the drawing. It decides that the window stays derived — computed from the draft and the cursor every time it is asked for, not stored as a scroll offset — which is what preserves the property the tail window had: there is nothing to reset when a long draft is cut short, and no second piece of state to disagree with where the cursor actually is.
