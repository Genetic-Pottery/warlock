# Key binds

Warlock has three places the keyboard can be — the tree, the panel beside it,
and the chat bar under the panel — and `Tab` moves between them.

## Moving around

| Key | What it does |
| --- | --- |
| `k` / `↑` | Up one row — the tree's selection, or the panel's window when the panel has focus |
| `j` / `↓` | Down one row |
| `PgUp` / `PgDn` | Up or down one screenful |
| `g` | Go to the first row, or the top of the panel |
| `G` | Go to the last row, or the bottom of the panel — this is what returns a scrolled-back panel to the newest line |
| `Space` | Fold the subtree under the selected row, or unfold it again |
| `Tab` | Move the focus on one place: tree → panel → composer → tree, skipping the composer when it is not on screen |
| `Shift-Tab` | Show the panel's other card: the document if the account is up, the account if the document is |

## Changing what the tree shows

| Key | What it does |
| --- | --- |
| `o` | Show only what warlock is managing — pacted directories, nothing else |
| `f` | Show the files inside each directory as well as the directories |

## Doing something

| Key | What it does | Scoped |
| --- | --- | --- |
| `p` | Pact the selected directory, over its whole subtree — or un-pact it if it is already pacted, which is a manifest edit only and leaves the documents on disk | yes |
| `r` | Refresh: run a pass over just the stale directories of an already-pacted subtree | yes |
| `s` | Open the scope prompt on a pacted directory | yes |
| `v` | Read the selected file into the panel's document card | no |
| `e` | Open the selected file in `$EDITOR`, handing the terminal over and taking it back afterwards | no |
| `m` | Stop the terminal reporting its mouse, or ask it to start again — with capture off, dragging over the screen selects text the way it does in any other program | no |

`p`, `r`, `s` and `e` are refused while a run is already going: one run at a
time, and the terminal cannot be handed to an editor from under a pass still
drawing on it. `v` and `Shift-Tab` are never refused — they race nothing.

**The scoped column** is the boundary. A directory's scope is the one written on
the nearest pacted directory at or above it — nearest wins outright, so an outer
scope is a default for whatever said nothing below it rather than a second gate
to also satisfy — and a key marked *yes* is refused there unless this machine
holds a sigil matching it. The footer names the scope wanted.
[`warlock config`](#the-headless-cli) prints and sets what this machine holds;
a sigil of `*` opens everything.

The permissive default is on the **directory** and only there: a directory nobody
scoped is open to anyone, which is why a repository that has never drawn a
boundary is unaffected by any of this. It is not mirrored on the sigil side. A
sigil is what opens a scope, so a machine holding none opens none — running
warlock in a scoped repository without recording what you hold gets you refused,
in the same words as holding the wrong sigil. That is the onboarding this
vocabulary was designed around: you are handed your scopes the way you are handed
any other credential, and you record them once.

`s` is on that list because you must hold a boundary in order to redraw it —
otherwise the one key whose entire purpose is to move a scope would be the one
key no scope covered. `p` is on it in **both** directions, and the un-pact
direction is the reason the list exists: un-pacting drops the scope along with
the pact, so a mis-aimed `p` on somebody else's subtree costs a full model pass
to put back and does not restore the boundary when it returns.

## Leaving

| Key | What it does |
| --- | --- |
| `q` / `Esc` | Ask whether to quit |
| `Ctrl-C` | Quit outright, with no question |

Ctrl-C is a keystroke here rather than a signal: the terminal is in raw mode, so
warlock answers it itself, including taking a running `claude` with it.

## While a run is going

| Key | What it does |
| --- | --- |
| `Esc` | Cancel the run — *not* quit, for as long as there is a run to stop |
| `q` / `Ctrl-C` | Quit outright, with no confirmation |

Every other key means exactly what it means the rest of the time. The tree stays
usable while a pact or a refresh works.

## In the chat bar

Reached with `Tab`. While it holds the keyboard, ordinary characters go into the
draft.

| Key | What it does |
| --- | --- |
| `Enter` | Send the draft |
| `Alt-Enter` | Put a newline in the draft |
| `Backspace` | Delete the last character |
| `Esc` | Hand the keyboard back and keep the draft exactly as it was |
| `Tab` | Move the focus on — the one key the composer does not take |
| `Ctrl-C` | Cancel the turn if one is being answered; otherwise quit |
| `/brief` | Enter brief mode: the same conversation is now converging on a document, sent the instruction the brief template describes |
| `/write` | Only in brief mode: ask for the document, then open an editable path prompt already holding a proposal like `docs/warlock-brief-13-scopes-and-sigils.md` |
| `/chat` | Leave brief mode, writing nothing — back to answering questions as they come |

## At the quit question

| Key | What it does |
| --- | --- |
| `←` / `→` | Light Yes, or light No |
| `Enter` | Take whichever answer is lit |
| `y` | Quit |
| `n` / `Esc` | Stay |

The question opens with No lit, and `Esc` answers No rather than leaving — the
reflex second press is the whole reason the question is there.

## At the scope prompt

| Key | What it does |
| --- | --- |
| `Enter` | Write the scope, or clear it if the field is empty |
| `Backspace` | Delete the last character |
| `Esc` | Close the prompt without writing anything |

## Mouse

Live only while capture is on, which is the default and what `m` turns off.

| Action | What it does |
| --- | --- |
| Left click | Select the row and give that pane the keys; a click on the row already selected folds it |
| Wheel | Scroll whichever pane the pointer is over, three rows or lines a notch, whatever has the focus |

Drags, moves, the other buttons and the horizontal wheel are read and dropped,
and the pointer means nothing anywhere while the quit question or the scope
prompt is up.