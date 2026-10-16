# Key binds

Warlock has three places the keyboard can be — the tree, the panel beside it,
and the chat bar under the panel — and `Tab` moves between them.

To see a short version of this page inside warlock, press `?` from the tree or
the panel. The help sheet scrolls with `j`, `k`, `PgUp`, and `PgDn`, and `Esc`
or `?` closes it. In the chat bar, `?` is typed as text.

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
| `Shift-Tab` | Show the panel's other card: the document if the conversation is up, the conversation if the document is |

## Changing what the tree shows

| Key | What it does |
| --- | --- |
| `o` | Show only scoped directories, and the directories above them |
| `f` | Show the files inside each directory as well as the directories |

## Doing something

| Key | What it does | Scoped |
| --- | --- | --- |
| `s` | Open the scope prompt on the selected directory | yes |
| `v` | Read the selected file into the panel's document card | no |
| `e` | Open the selected file in `$EDITOR`, handing the terminal over and taking it back afterwards | no |
| `m` | Stop the terminal reporting its mouse, or ask it to start again — with capture off, dragging over the screen selects text the way it does in any other program | no |

`s` is refused while a pull is running, because a session is editing the tree and
a scope written under it would move the boundary that session is judged against.
`v` and `Shift-Tab` are never refused.

**The scoped column** is the boundary. A directory's scope is the one written on
the nearest scoped directory at or above it — nearest wins outright, so an outer
scope is a default for whatever said nothing below it rather than a second gate
to also satisfy — and a key marked *yes* is refused there unless that scope is
open on this machine. The footer names the scope wanted. The **AI can work in**
box under the tree lists the scopes open on this machine. To change it, use
`/open` and `/close` in the chat bar or
[`warlock open` and `warlock close`](#the-headless-cli); opening `*` opens
everything.

A directory no scope covers is closed to every AI session: no pull works there,
and `warlock check --gate` refuses it. Anyone may draw the first scope on such a
directory with `s`, because otherwise no repository could ever be set up. Once a
scope covers a directory, that scope must be open on this machine to redraw it
— a boundary anybody may retype is not a boundary.

Every scope starts closed, so a machine that has opened none lets AI work in
none. Running warlock in a scoped repository without opening your scopes gets
you refused, in the same words as working in a scope you have not opened. That
is the onboarding this vocabulary was designed around: you are handed your
scopes the way you are handed any other credential, and you open them once.

## Leaving

| Key | What it does |
| --- | --- |
| `q` / `Esc` / `Ctrl-C` | Ask whether to quit |

Ctrl-C asks rather than quits because it's the copy key in most other programs.
To copy text, select it with the mouse instead. Ctrl-C asks over any other
window, and pressing it again at the quit question leaves the question up.

## In the chat bar

Reached with `Tab`. While it holds the keyboard, ordinary characters go into the
draft.

The strip on the chat bar's top border shows the workflow: chat, brief, write,
push, draft, and pull. The current mode is lit. While a push, draft, or pull is
running, it is the only stage lit.

| Key | What it does |
| --- | --- |
| `Enter` | Send the draft |
| `Alt-Enter` | Put a newline in the draft |
| `Backspace` | Delete the last character |
| `Ctrl-U` | Delete back to the start of the line; press again at the start to join the line above, so holding it clears the draft |
| `→` / `End` | With the draft empty and a suggestion showing, take the suggestion into the draft to edit |
| `Esc` | Hand the keyboard back and keep the draft exactly as it was |
| `Tab` | Move the focus on — the one key the composer does not take |
| `Ctrl-C` | Cancel the turn if one is being answered; otherwise ask whether to quit |
| `/brief` | Enter brief mode: the same conversation is now converging on a document. Your next message carries the instruction the brief template describes. When the model has enough, a window asks **Write the brief?** Yes asks for the document, then opens an editable path prompt already holding a proposal like `docs/warlock-brief-13-open-and-closed-scopes.md`; writing the file returns to chat mode. No keeps talking; ask for the document in your own words and the question comes back |
| `/chat` | Leave brief mode, writing nothing — back to answering questions as they come |
| `/push <SCOPE> <PATH>` | File the brief at `PATH` to the scope's Linear team as a project, after a Yes/No question |
| `/draft <SCOPE>` | List the scope's planned projects, one per line, slug first |
| `/draft <SCOPE> <SLUG>` | Cut that project into Linear issues, one slice at a time |
| `/pull` | Name the scopes AI can work in on this machine |
| `/pull <SCOPE>` | Work the next ready ticket in the scope's queue to a pull request |
| `/pull <SCOPE> <TICKET>` | Work the named ticket, for example `WAR-123`, instead of the next one |
| `/resume <TICKET>` | Mark the sub-tasks a halted pull stopped on as pending again, and put `/pull <SCOPE> <TICKET>` in the chat bar to send |
| `/branch` | Name the branch pulls start from and open pull requests against |
| `/branch use <BRANCH>` | Make pulls start from and open pull requests against `BRANCH`, the same as `warlock branch use` |
| `/branch clear` | Go back to the remote's default branch, the same as `warlock branch clear` |
| `/key <SCOPE>` | Name the key the scope is bound to, the same as `warlock key <scope>` |
| `/key list` | Name the keys this machine stores, the same as `warlock key list` |
| `/open` | Name the scopes AI can work in on this machine for this repository |
| `/open <SCOPE>...` | Let AI work in these scopes as well, the same as `warlock open` |
| `/close <SCOPE>...` | Stop AI working in these scopes, the same as `warlock close` |

Commands are case-sensitive and take their arguments on the same line. A
mistyped command is refused with a line listing every command, and nothing is
sent to the model.

## Drafting a project

A draft turns a project pushed with `/push` into issues. To draft one, do the
following:

1. Send `/draft <SCOPE>`. The panel lists the scope's projects that are
   `Planned` and carry the scope's label, as `slug  name`. The slug is the hex
   at the end of the project's Linear URL.
2. Send `/draft <SCOPE> <SLUG>` with the slug of the project you want. The slug
   must match exactly.
3. Answer the question that opens, which names the project, its status, how
   many slices it has, and the team. Its keys are the
   [quit question's](#at-the-quit-question), and it opens with No lit.
4. Answer any questions the slices ask, and review each slice's drafts, as
   described in the next two sections.

Slices already cut or skipped by an earlier draft are passed over, so you can
run `/draft <SCOPE> <SLUG>` again to finish a project.

Esc doesn't cancel a running draft. To stop one, skip a slice and answer No
when asked whether to carry on, or quit warlock.

### When a slice asks a question

The chat bar takes the keyboard, and its border names the slice being answered.
Warlock's suggested answer shows dimmed in the empty chat bar. It's never part
of the draft:

- To send the suggestion, press `Enter` on the empty bar.
- To edit the suggestion, press `→` to take it into the draft.
- To answer in your own words, type over it.

### At the review window

Each slice's drafts print whole on the panel, and the review window opens with
Create lit. Scroll the panel with `↑`, `↓`, `PgUp`, and `PgDn` to read them.

| Key | What it does |
| --- | --- |
| `←` / `→` | Light the answer to the left or right |
| `Enter` | Take whichever answer is lit |
| `c` | Create: file the drafts as issues and go to the next slice |
| `e` | Edit: open the drafts in `$EDITOR`, then review what you saved |
| `f` | Feedback: type in the chat bar what the drafts should be instead, and review the redraft |
| `s` / `Esc` | Skip: file nothing for this slice; the next draft passes it over |

The window has no cancel. After a skip with slices still left, a question asks
whether to carry on to the next slice. It opens with No lit, and No stops the
run with the remaining slices left for another draft.

## Pulling a ticket

A pull takes a ticket that is assigned to you, carries the scope's label, and
has nothing blocking it. Warlock splits it into sub-tasks, commits each one on
the ticket's branch, pushes the branch, and opens a pull request.

To start a pull, send `/pull <SCOPE>` for the next ready ticket, or
`/pull <SCOPE> <TICKET>` for a specific one. A question names the ticket, scope,
team, and branch, with the quit question's keys and No lit. If you don't
remember your scopes, send `/pull` on its own to list them.

A pull asks nothing once it starts and has no cancel key. The chat bar refuses
input until it finishes. To stop a pull, quit warlock.

When a pull halts, the ticket stays in progress and its Linear comment lists
what finished and what didn't. To pick it up again, send `/resume <TICKET>`,
then send the `/pull` it leaves in the chat bar. The question that opens asks
whether to resume and names the sub-task it starts from.

The title on the file tree's top border names the branch a pull starts from
and opens its pull request against, for example `pulls into main`. It ends in
`(default)` when no branch is set and warlock is using the remote's default.

One draft and one pull run at a time. A second `/draft` or `/pull` sent while
one is running is refused, and nothing is read or changed.

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