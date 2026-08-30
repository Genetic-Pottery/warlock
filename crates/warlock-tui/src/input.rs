//! What a keystroke or a mouse event asks the app to do.
//!
//! Pure translations, one per device, and no terminal in any of them.
//! [`action_for`] turns a key event and a situation — is a pact in flight —
//! into an [`Action`]; [`mouse_action`] turns a mouse event, the size the
//! frame was drawn at, the app and the gate on the way out into a
//! [`MouseAction`]. Naming the intent apart from the event that produced it is
//! what keeps both testable with nothing attached to stdout, and leaves the
//! event loop in `main.rs` reading as a list of consequences.
//!
//! [`press_for`] is the keyboard's second half and the newer one: it is where
//! the windows drawn over the frame are decided, so that the loop above has one
//! arm that returns, one that moves the question, one that types into the scope
//! prompt, one that types into the composer and one that hands the key on. Esc
//! and `q` no longer leave by themselves — with nothing running they open the
//! question instead — and while either window is up, or while the composer holds
//! the keyboard, every key goes to its own pure function,
//! [`answer_for`](warlock_tui::answer_for),
//! [`edit_for`](warlock_tui::edit_for) or
//! [`compose_for`](warlock_tui::compose_for), rather than to [`action_for`],
//! which is what keeps a stray `j` from moving a selection nobody can see behind
//! the window and a typed `p` from pacting a directory. Ctrl-C is answered before
//! any of them, and a run in flight suppresses the quit gate; both are argued for
//! on [`press_for`] itself.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Size;
use warlock_tui::{
    Answered, App, Composed, Composer, Edited, Focus, Hit, QuitConfirm, ScopePrompt, answer_for,
    compose_for, edit_for, hit_test,
};

/// What a keystroke asks the app to do.
///
/// Naming the intent separately from the key that produced it keeps
/// [`action_for`] a pure function of a key event, testable with no terminal
/// attached, and leaves the loop above reading as a list of consequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Leave the app.
    Quit,
    /// Stop the pact that is running, and stay.
    CancelPact,
    /// Move the keys one place round the cycle: the tree column, the panel
    /// beside it, and the composer under the panel take it in turns to be lit
    /// and to be what the movement keys drive.
    ///
    /// One action rather than a focus-the-tree, a focus-the-panel and a
    /// focus-the-composer, because there is one key: "go to the next one" is the
    /// whole of what a reader can mean by pressing it, and a set of actions
    /// would be three names for the same keystroke read three times. Which
    /// places the cycle can stop at is the app's, not this function's — the
    /// composer is skipped while the document card hides it, see
    /// [`App::toggle_focus`](crate::App::toggle_focus).
    ToggleFocus,
    /// Move the selection one row up.
    SelectPrevious,
    /// Move the selection one row down.
    SelectNext,
    /// Move the selection one screenful up.
    SelectPageUp,
    /// Move the selection one screenful down.
    SelectPageDown,
    /// Select the first row of the tree.
    SelectFirst,
    /// Select the last row of the tree.
    SelectLast,
    /// Hide the selected directory's descendants, or show them again if they
    /// are hidden already.
    ToggleCollapsed,
    /// Draw only the pacted nodes and the ancestors that reach them, or the
    /// whole tree again if that is what is on screen already.
    TogglePactedOnly,
    /// Draw the files inside each directory as well as the directories, or go
    /// back to directories alone if the files are on screen already.
    ToggleFiles,
    /// Pact the selected node, or unpact it if it is pacted already.
    TogglePact,
    /// Re-describe the stale directories under the selected node, and only
    /// those.
    ///
    /// A pact says "describe all of this"; this says "describe the part of it
    /// that has gone yellow". One edited file in a large repository leaves a
    /// handful of directories stale and the rest green, and getting back to
    /// green through [`Action::TogglePact`] would pay for a pass over every
    /// directory in the subtree to re-describe the few that need it.
    Refresh,
    /// Ask what scope the selected directory carries.
    ///
    /// The only action here that opens a window rather than changing the tree,
    /// and the only one whose whole answer is somewhere else: what the loop does
    /// with it is read the directory's scope out of the manifest and put the
    /// prompt up over it, and from that keystroke on the keys belong to
    /// [`edit_for`](warlock_tui::edit_for) rather than to this file.
    ///
    /// It is not a run. Nothing is spawned, no `claude` is started and no
    /// progress line appears — a scope is a fact somebody types, and the whole
    /// of writing one is a manifest saved on the loop's own thread.
    ///
    /// The two refusals a press can come to — a row that cannot be scoped, and a
    /// run already in flight — are the app's answer and the loop's, exactly as
    /// they are for [`Action::TogglePact`] and [`Action::Refresh`]. This
    /// function's business is that the key was pressed.
    OpenScope,
    /// Read the selected file and put its lines in the panel.
    ///
    /// The first action here that shows a file rather than describing one: every
    /// colour in the tree is a claim about a `WARLOCK.md`, and this is how the
    /// document behind a claim gets on screen without leaving warlock.
    ///
    /// It writes nothing and starts nothing. A capped read is over inside a
    /// frame, so there is no thread, no channel and no progress line — which is
    /// also why a run in flight is no reason to refuse it, and why this key,
    /// like every one but Esc, means the same thing during a pact as outside
    /// one.
    ///
    /// What a row that is not a file comes to is the app's answer, exactly as it
    /// is for [`Action::TogglePact`] and [`Action::OpenScope`]: a directory is
    /// refused in the terms its own row makes available. This function's
    /// business is that the key was pressed.
    ViewFile,
    /// Hand the selected file to `$EDITOR`, and take the terminal back when the
    /// editor is done with it.
    ///
    /// [`Action::ViewFile`]'s other half, and section 9's escape hatch given a
    /// keystroke: a reader who sees a `WARLOCK.md` that is wrong should not have
    /// to leave warlock, find the path again and come back. Warlock still writes
    /// no byte of it — the editor does, and the workspace's writers are still
    /// the pact, the refresh, the manifest, the scope key and `warlock init`.
    ///
    /// The only action here that gives the screen away. What the loop does with
    /// it is put the terminal back the way it found it, run the editor as a
    /// foreground child, wait for it, and re-enter raw mode, the alternate
    /// screen and mouse reporting afterwards — so it is also the only one whose
    /// answer is measured in whole minutes rather than in frames.
    ///
    /// And it has a cost worth saying out loud: a `WARLOCK.md` is an ordinary
    /// file in its own directory's walk, so saving one restales the very
    /// directory it describes, and the only road back to green is `r` and a
    /// pass.
    ///
    /// The refusals a press can come to are the app's answer and the loop's,
    /// exactly as they are for [`Action::TogglePact`] and [`Action::OpenScope`]:
    /// a row that is not a file is refused in the same words [`Action::ViewFile`]
    /// refuses it in, and a run in flight is refused where every other mid-run
    /// refusal is. This function's business is that the key was pressed, which
    /// is why the key means the same thing during a pact as outside one.
    EditFile,
    /// Show the panel's other card: the account if the document is up, the
    /// document if the account is.
    ///
    /// One action rather than a show-the-account and a show-the-document, for
    /// [`Action::ToggleFocus`]'s reason: the panel is one slot holding two
    /// cards, so "show the other one" is the whole of what a reader can mean by
    /// pressing the key, and a pair of actions would be two names for the same
    /// keystroke read twice.
    ///
    /// It is the only thing besides [`Action::ViewFile`] that decides which card
    /// is on screen, which is what makes a document survive a pact starting,
    /// finishing, failing or being cancelled underneath it. It reads nothing,
    /// writes nothing and starts nothing — the cards are already in hand — so a
    /// run in flight is no reason to refuse it, and it means the same thing
    /// during a pact as outside one.
    ///
    /// The one refusal it can come to — no document read yet this session, so
    /// there is no second card to swap to — is the app's answer, exactly as a
    /// directory row is for [`Action::ViewFile`]. This function's business is
    /// that the key was pressed.
    SwapCard,
    /// Stop the terminal reporting its mouse, or ask it to start again if it has
    /// been stopped.
    ///
    /// The one action here that is not about the app at all: what it changes is
    /// what the terminal sends, which is why the loop answers it with an escape
    /// sequence rather than with a method on [`App`]. With capture off the
    /// terminal keeps its own selection — dragging over the screen copies text,
    /// the way it does in any other program — and warlock hears no pointer at
    /// all until the next press.
    ToggleMouseCapture,
}

/// The action `key` asks for with a pact `in_flight` or without one, or `None`
/// for a key that means nothing here.
///
/// One key reads two ways, and it is Esc. With nothing running it quits, which
/// is what it has always done and what the footer has always said. With a pact
/// running it cancels *that* — because the run is the thing in front of the
/// reader, because stopping it is the only thing they can want from a key that
/// means "not this", and because quitting outright on the key nearest to hand
/// would be the one keystroke that costs minutes of somebody else's model time
/// by mistake. Quitting during a run is still one keystroke away, spelled `q` or
/// Ctrl-C, which say what they mean and are not what a hand reaches for to stop
/// something.
///
/// The mode is a parameter rather than something looked up, so this stays a pure
/// function of a key and a situation and both readings are one assertion each.
/// Nothing else in here consults it: every other key means exactly what it meant
/// before, mid-pact included, which is what keeps the tree usable while a run
/// works.
///
/// Only presses count. Crossterm reports key releases and auto-repeats on some
/// platforms (Windows, and on terminals that speak the Kitty keyboard
/// protocol) and not on others, so acting on anything but a press would move
/// the selection twice per keystroke on those platforms and once on the rest —
/// and, since `p` writes the manifest, would toggle a pact straight back off
/// again on the release of the key that turned it on.
///
/// Ctrl-C is a key event, not a signal: raw mode is exactly the mode in which
/// the terminal stops turning it into `SIGINT`, so if this function does not
/// handle it, nothing does — including during a pact, where it is one of the two
/// ways out that also has to take the running `claude` with it.
pub(crate) fn action_for(key: KeyEvent, in_flight: bool) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }

    match key.code {
        // `contains` rather than equality: shift or caps lock can ride along
        // (some terminals report the upper-case letter with it), and Ctrl-C is
        // still Ctrl-C.
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Quit)
        }
        // Before the quit arm below, and the only thing in here the mode
        // touches: `q` and Ctrl-C keep meaning quit while a pact runs, and Esc
        // stops being a way out for as long as there is a run to stop.
        KeyCode::Esc if in_flight => Some(Action::CancelPact),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        // Tab is the key every split-screen program moves focus with: it takes
        // no argument and asks no question, so it means the same thing whether
        // or not a pact is in flight, exactly like every key below it.
        KeyCode::Tab => Some(Action::ToggleFocus),
        // Shift-Tab is a different keystroke, and crossterm spells it
        // `BackTab` — the terminal sends its own code for it, so there is no
        // shift riding along on a `Tab` to match against and nothing here that
        // could confuse the two. It is the panel's key rather than focus's:
        // focus's cycle is short enough to get anywhere by pressing Tab again,
        // and with two cards in one slot there is exactly one other thing a
        // reader can be asking for. Like every key but Esc it reads the same
        // way with a run in flight
        // as without one — the cards are already in hand, so there is nothing
        // for a run to race — and what a session with no document read yet comes
        // to is the app's answer, exactly as a directory row is for `v`.
        KeyCode::BackTab => Some(Action::SwapCard),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown => Some(Action::SelectPageDown),
        // `g` and `G` are the pair every pager and vi-like editor has trained
        // hands for, and they are told apart by case alone: matching on the
        // character rather than on `SHIFT` keeps a terminal that reports the
        // upper-case letter without the modifier — or with it, or with caps
        // lock instead — landing on the same action, exactly as Ctrl-C above
        // does not care which of those it is handed.
        KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::Char('G') => Some(Action::SelectLast),
        // Space is the file-tree key everywhere, and crossterm spells it as an
        // ordinary character: there is no `KeyCode::Space`, so `Char(' ')` is
        // the whole of it. Nothing rides along that needs matching — a modifier
        // held with space is a different keystroke, not this one badly spelled.
        KeyCode::Char(' ') => Some(Action::ToggleCollapsed),
        // Lower case only, like `p` below: the upper-case letter is a
        // different keystroke and means nothing here, and a filter that also
        // answered to `O` would take a key that a later binding may want. The
        // mnemonic is "only": what stays on screen is the pacted nodes only.
        KeyCode::Char('o') => Some(Action::TogglePactedOnly),
        // Lower case only, like `o` above and `p` below. The mnemonic is
        // "files": what the key adds to the screen is the files inside each
        // module. It writes nothing and reads nothing — the files came with the
        // tree — so, unlike `p`, there is nothing here that a stray press could
        // cost anybody.
        KeyCode::Char('f') => Some(Action::ToggleFiles),
        // Lower case only, and with no confirmation: the mnemonic is the
        // product's own word (pact, §15), and the action is its own undo —
        // pressing it again removes what it wrote.
        KeyCode::Char('p') => Some(Action::TogglePact),
        // Lower case only, like `p` above: the mnemonic is "refresh", and `R`
        // is a different keystroke that means nothing here. Like every key but
        // Esc it reads the same way with a run in flight as without one —
        // what a refresh does about a run already working is the app's answer
        // to give, not this function's, exactly as a second `p` is.
        KeyCode::Char('r') => Some(Action::Refresh),
        // Lower case only, like `p` and `r` above: the mnemonic is "scope", and
        // `S` is a different keystroke that means nothing here. Like every key
        // but Esc it reads the same way with a run in flight as without one —
        // a run is a reason to refuse the prompt, and refusing is the loop's
        // answer to give, exactly as it is for a second `p`.
        KeyCode::Char('s') => Some(Action::OpenScope),
        // Lower case only, like the four above it: the mnemonic is "view", and
        // `V` is a different keystroke that means nothing here. Like every key
        // but Esc it reads the same way with a run in flight as without one, and
        // here there is nothing for the mode to change even in principle: a read
        // is not a run — it writes nothing, starts nothing and is over inside a
        // frame — so there is no second run for it to be refused as.
        KeyCode::Char('v') => Some(Action::ViewFile),
        // Lower case only, like the five above it: the mnemonic is "edit", and
        // `E` is a different keystroke that means nothing here. Like every key
        // but Esc it reads the same way with a run in flight as without one — a
        // run is a reason to refuse the editor, because the terminal cannot be
        // handed away from under a pass that is still drawing its account on it,
        // and refusing is the loop's answer to give, exactly as it is for a
        // second `p`.
        KeyCode::Char('e') => Some(Action::EditFile),
        // Lower case only, like the five above it. The mnemonic is "mouse",
        // and the key means the same thing whether or not a pact is in flight:
        // giving the terminal its own text selection back is exactly the thing a
        // reader wants during a long run, when there is output on screen worth
        // copying. It moves nothing, selects nothing and writes nothing.
        KeyCode::Char('m') => Some(Action::ToggleMouseCapture),
        _ => None,
    }
}

/// What a keystroke comes to once the gate on the way out has had it.
///
/// [`Action`] says what a key asks the *app* for; this says what it asks the
/// *loop* for, and the gap between the two is the whole of the gate. Leaving is
/// no longer something a key does to the app — it is one of three things that
/// can happen to a session — so it is named here, beside the question that now
/// stands in front of it, rather than left as an [`Action`] the loop has to
/// remember to treat differently.
///
/// Six variants, and the useful part is that they are exclusive: a keystroke
/// either ends the session, or moves the question, or goes into the scope
/// prompt, or goes into the composer, or reaches the app, or comes to nothing.
/// While either window is up the [`Pressed::Act`] road is unreachable, which is
/// the plain statement of "nothing leaks through to the tree underneath"; while
/// the composer holds the keyboard it is reachable by exactly one key, and that
/// key is Tab (see [`press_for`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pressed {
    /// Leave warlock now, by the path a quit has always taken: the loop returns,
    /// the run's handle drops and takes a running `claude` with it, and the
    /// terminal guard puts the screen back.
    Leave,
    /// The gate had the key: the confirmation is `.0` from here on, and nothing
    /// else happened. It covers opening the question, moving its highlight,
    /// closing it again, and the keys that leave it exactly where it was.
    Confirm(QuitConfirm),
    /// The scope prompt had the key, and `.0` is what it made of it: the field
    /// with one character more or less in it, the prompt abandoned, or the text
    /// offered up for the engine to judge.
    ///
    /// The prompt's own outcome is carried through rather than translated,
    /// because two of its three answers are the loop's to act on and none of
    /// them is a state this file can name better than
    /// [`edit_for`](warlock_tui::edit_for) already does. Like
    /// [`Pressed::Confirm`], it says the app was not consulted: while the prompt
    /// is up every key that is not Ctrl-C comes back through here.
    Scope(Edited),
    /// The composer had the key, and `.0` is what it made of it: the draft with
    /// one character more or less in it, the keyboard handed back, or the draft
    /// offered up.
    ///
    /// [`Pressed::Scope`]'s counterpart for the field that is not a window. The
    /// composer's own outcome is carried through rather than translated, for the
    /// reason the prompt's is: two of its three answers are the loop's to act on,
    /// and none of them is a state this file can name better than
    /// [`compose_for`](warlock_tui::compose_for) already does. Like the two arms
    /// above it, it says the app was not consulted — while the composer holds the
    /// keyboard every key but Ctrl-C and Tab comes back through here, which is
    /// what makes `p` the letter p.
    Compose(Composed),
    /// The app's key: do `.0`.
    ///
    /// Never [`Action::Quit`]. Every way out is [`Pressed::Leave`] above, which
    /// is what makes "the gate cannot be bypassed" a fact about this type rather
    /// than a rule the loop is trusted to keep.
    Act(Action),
    /// A key nothing is bound to, or one already answered where it was decided.
    Nothing,
}

/// Whether `key` is the keystroke every reader trusts to get them out.
///
/// Split out of [`action_for`]'s first arm because [`press_for`] has to answer
/// it *before* it consults anything else, and answering it in two places with
/// two different spellings is how the one keystroke that must always work stops
/// working in one of them. `contains` rather than equality for the reason the
/// arm below has it: shift or caps lock can ride along, and Ctrl-C is still
/// Ctrl-C.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Whether `key` is the keystroke that moves the keyboard on.
///
/// Split out for [`is_ctrl_c`]'s reason, one layer down: [`press_for`] has to
/// answer it *before* it offers the key to the composer, because a field that
/// swallowed Tab would be a field with no way out but Esc — and Esc is the key
/// that hands the keyboard back without moving it anywhere, which is a different
/// thing to want. The code alone, with no modifier compared, exactly as
/// [`action_for`] matches it: Shift-Tab is a keystroke of its own that crossterm
/// spells `BackTab`, so there is no shift riding along here to tell apart.
///
/// Every kind of event, presses included and releases with them, because what
/// this decides is not what Tab *does* but which function is asked: a release
/// handed on comes to nothing in [`action_for`], which is where every release
/// already comes to nothing.
fn is_tab(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab
}

/// What `key` comes to with the confirmation at `confirm`, the scope prompt at
/// `prompt`, the composer at `composer` and a run `in_flight`.
///
/// The gate itself, and it is a function of a key and four situations so that
/// every rule below is one assertion with no terminal attached. Five roads out
/// of it, in the order they are decided.
///
/// **Ctrl-C first, always.** It is a key event and not a signal — raw mode is
/// exactly the mode in which the terminal stops turning it into `SIGINT` — so if
/// nothing here answers it, nothing does. Routed through the question it would
/// arrive at [`answer_for`] as an ordinary `c` with a modifier riding along,
/// i.e. one of the keys that change nothing, and the last resort of a reader
/// who wants out would be the one keystroke the dialog swallowed. So it leaves
/// with the question up and with it closed, and during a run as well as outside
/// one, exactly as it always has.
///
/// **Then the question, if it is up.** Every other key goes to [`answer_for`]
/// and *only* to it: this is where "nothing reaches the app underneath" is
/// true, because [`action_for`] is not called at all on that road. The tree's
/// own bindings — `j`, `k`, `g`, `G`, space, `o`, `f`, `p`, `r`, `s`, `m`, Tab,
/// the page keys — are inert for as long as the question stands, without any of
/// them needing to know the question exists.
///
/// **Then the scope prompt, if that is up.** The same road again, to
/// [`edit_for`] and only to it, and for the same reason: while somebody is
/// typing a scope, the tree's bindings are letters going into a field or
/// keystrokes that mean nothing, and [`action_for`] is not consulted at all.
/// The two windows are asked in this order rather than the other because a key
/// answered twice is a key answered wrongly once; in practice they cannot both
/// be up, since `q` and Esc are text and an abandonment while the prompt has the
/// keyboard and so never reach the gate that opens the question.
///
/// **Then the composer, if it has the keyboard.** `composer` is `Some` only when
/// focus is on the field, which is the shape [`ScopePrompt::field`] already has:
/// the situation is offered rather than looked up, so "the composer is consulted
/// exactly when the reader is pointed at it" is the caller's one line and every
/// rule here is one assertion. The road is the same as the two above it, to
/// [`compose_for`](warlock_tui::compose_for) and only to it, and it is the whole
/// reason this file's single-letter bindings can go on being single letters:
/// while somebody is typing, `p`, `r`, `s`, `v`, `e`, `f`, `g`, `G`, `j` and `k`
/// are characters going into a draft and [`action_for`] is not consulted at all.
/// It is asked last of the three because a window is drawn *over* the composer:
/// a key cannot be both typed into a field on the frame and answered by the
/// dialog covering it.
///
/// One key is not the composer's, and it is Tab. It is the key every split-screen
/// program moves the keyboard with, it is not text on any terminal, and a field
/// that ate it would be a field whose only exit is Esc — so it goes past the
/// composer to [`action_for`]'s cycle, which is where the composer was arrived
/// at in the first place. Esc is the other way out and means something else: it
/// hands the keyboard back and leaves the draft where it is, which is why a run
/// in flight is *not* cancelled by an Esc typed at the composer — that Esc is
/// answered by the field the reader is in, exactly as it is while the scope
/// prompt is up, and the next one cancels the run.
///
/// **Then the keys, as they have always been read.** [`action_for`] answers,
/// and the one answer this function re-reads is [`Action::Quit`]: with nothing
/// running it opens the question instead of leaving, and with a run in flight it
/// leaves outright. That last part is the whole reason `in_flight` is here.
/// Esc during a run already means cancel (see [`action_for`]), and the press
/// after it is the reflex second Esc this gate exists for — but `q` and Ctrl-C
/// during a run are keys a reader reaches for deliberately, often to get out of
/// a run that is going nowhere, and a question in front of them would be a
/// question in front of somebody who has already decided. The gate is for the
/// twitch, not for the decision; the twitch only happens when there is a run to
/// have cancelled, and by then Esc means cancel anyway.
///
/// Nothing here changes what cancel means, what Ctrl-C does, or what any other
/// key is bound to: [`action_for`] is untouched and is still the one place a key
/// is turned into an [`Action`].
pub(crate) fn press_for(
    key: KeyEvent,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    composer: Option<&Composer>,
    in_flight: bool,
) -> Pressed {
    if is_ctrl_c(key) {
        return Pressed::Leave;
    }

    if let Some(highlighted) = confirm.highlighted() {
        return match answer_for(key, highlighted) {
            Answered::Open(answer) => Pressed::Confirm(QuitConfirm::Open(answer)),
            Answered::Close => Pressed::Confirm(QuitConfirm::Closed),
            Answered::Leave => Pressed::Leave,
        };
    }

    if let Some(field) = prompt.field() {
        return Pressed::Scope(edit_for(key, field));
    }

    if let Some(draft) = composer
        && !is_tab(key)
    {
        return Pressed::Compose(compose_for(key, draft));
    }

    match action_for(key, in_flight) {
        // The gate, in one arm: the key that used to leave now asks first.
        Some(Action::Quit) if !in_flight => Pressed::Confirm(QuitConfirm::open()),
        Some(Action::Quit) => Pressed::Leave,
        Some(action) => Pressed::Act(action),
        None => Pressed::Nothing,
    }
}

/// How far one notch of the wheel moves a pane: three rows of the tree, or
/// three lines of the panel.
///
/// One number for both panes, so the two answer at the same speed — the pointer
/// crosses from one to the other and a hand does not expect the gearing to
/// change under it. Three is what terminal programs have settled on: a row a
/// notch is a wheel that has to be spun to get anywhere, and a screenful a notch
/// is a wheel that loses the reader's place on the way.
const WHEEL_NOTCH: usize = 3;

/// What a mouse event asks the app to do.
///
/// [`Action`]'s counterpart for the pointer, and separate from the event for the
/// same reason: naming the intent apart from what produced it keeps
/// [`mouse_action`] a pure function of an event, a terminal size and the app —
/// testable with nothing attached to stdout — and leaves the loop above reading
/// as a list of consequences.
///
/// There is no variant for hovering, for dragging, or for a button other than
/// the left one. Those events are read and dropped ([`mouse_action`]), and a
/// name here for any of them would be an invitation to behaviour warlock has
/// decided against: a highlight that follows the pointer costs a redraw per
/// pointer move to say what the selection already says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseAction {
    /// Move the tree's selection `.0` rows down, whichever pane has the focus.
    SelectNextBy(usize),
    /// Move the tree's selection `.0` rows up, whichever pane has the focus.
    SelectPreviousBy(usize),
    /// Scroll the panel's window `.0` lines towards the newest line, whichever
    /// pane has the focus.
    ScrollPanelDown(usize),
    /// Scroll the panel's window `.0` lines back, whichever pane has the focus.
    ScrollPanelUp(usize),
    /// Select the row at `.0` in [`App::rows`](warlock_tui::App::rows), and give
    /// the tree the keys.
    SelectRow(usize),
    /// Expand or collapse the selected row, and give the tree the keys: what a
    /// click on the row that is already selected comes to, which is what space
    /// comes to.
    ToggleCollapsed,
    /// Give the named pane the keys, and do nothing else.
    Focus(Focus),
}

/// What `mouse` over a terminal of `size` asks `app` to do with the
/// confirmation at `confirm` and the scope prompt at `prompt`, or `None` for an
/// event that means nothing here.
///
/// [`action_for`]'s counterpart, and the same shape: everything in, one
/// intention out, no terminal read and nothing drawn. The size is the one the
/// round measured before it drew, so the hit test agrees with the frame the
/// reader is pointing at rather than with a second opinion about the layout; the
/// app is here because a screen point alone cannot say which row it landed on —
/// [`Hit::TreeRow`] counts from the top of the tree's window, and only the app
/// knows where that window is and how many rows are under it.
///
/// Two events count and the rest do not. The wheel drives whichever pane the
/// pointer is *over*, focus notwithstanding — that is the whole convention of a
/// pointer, and a wheel that scrolled the focused pane instead would scroll the
/// half of the screen the reader is not looking at. The left button selects and
/// focuses. Drags, moves, releases, the other buttons and the horizontal wheel
/// are read and dropped: they are out of scope by decision, not by omission,
/// and dropping them here is what keeps a pointer swept across the screen from
/// changing anything at all.
///
/// While either window is up the pointer means nothing anywhere: every event is
/// read and dropped, wheel and click alike. Neither has anything clickable in it
/// — the confirmation has no clickable Yes and no clickable No, and the scope
/// prompt has a field that is typed into and no buttons — both are answered from
/// the keyboard, like the keystrokes that opened them, and a click that landed
/// on the tree behind either would select a row the reader cannot see, under a
/// window that is about to close. The gate lives here rather than in the loop's
/// arm for the same reason [`press_for`]'s does: it is a decision, and decisions
/// are testable with nothing attached to stdout.
///
/// `composer` is the draft the frame was drawn with — `None` on a frame that had
/// no composer on it, which is every frame while the document card has the panel
/// — and it is here for the layout and nothing else: the rows the field takes
/// are rows the panel gave up, so a hit test that had not been told about the
/// draft would answer [`Hit::PanelLine`] for a point drawn on a field and scroll
/// a window the pointer is not over. See [`hit_test`].
pub(crate) fn mouse_action(
    mouse: MouseEvent,
    size: Size,
    app: &App,
    confirm: QuitConfirm,
    prompt: &ScopePrompt,
    composer: Option<&Composer>,
) -> Option<MouseAction> {
    if confirm.is_open() || prompt.is_open() {
        return None;
    }

    let hit = hit_test(mouse.column, mouse.row, size, composer);
    match mouse.kind {
        // Down the tree and down the account are the same direction, so one
        // notch reads the same way over either pane.
        MouseEventKind::ScrollDown => wheel(
            hit,
            MouseAction::SelectNextBy(WHEEL_NOTCH),
            MouseAction::ScrollPanelDown(WHEEL_NOTCH),
        ),
        MouseEventKind::ScrollUp => wheel(
            hit,
            MouseAction::SelectPreviousBy(WHEEL_NOTCH),
            MouseAction::ScrollPanelUp(WHEEL_NOTCH),
        ),
        // The press, not the release: it is the half of a click a reader means,
        // and answering both would do everything twice.
        MouseEventKind::Down(MouseButton::Left) => click(hit, app),
        _ => None,
    }
}

/// One notch of the wheel at `hit`: `tree` when the pointer is over the tree
/// column, `panel` when it is over the panel, and nothing anywhere else.
///
/// Which way the notch went is the caller's, because that is the only thing
/// that differs between the two directions; what this owns is the rule that the
/// pointer picks the pane. Every part of a pane's inside answers for that pane,
/// the tree's header included: a wheel is aimed at a column rather than at a
/// row, and a notch that did nothing because the pointer happened to be on the
/// one line naming the tree would read as a wheel that sticks.
///
/// The footer, the composer and the borders answer nothing, and they are the
/// whole of what does not: the footer is nobody's pane, a border is the line
/// between two of them rather than a place a reader means to scroll, and the
/// composer has nothing to scroll — it is a handful of rows showing the end of a
/// draft, and it scrolls itself as somebody types. A notch over it is emphatically
/// not a notch over the panel above it: scrolling the account because the pointer
/// was resting on the field would move the half of the screen the reader is not
/// pointing at, which is the one thing this function exists to prevent.
fn wheel(hit: Hit, tree: MouseAction, panel: MouseAction) -> Option<MouseAction> {
    match hit {
        Hit::TreeHeader | Hit::TreeRow { .. } | Hit::TreeBelowRows => Some(tree),
        Hit::PanelLine { .. } => Some(panel),
        Hit::Composer | Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

/// One press of the left button at `hit`, given where `app` has its window and
/// its selection.
///
/// A click inside a pane always gives that pane the keys, and on the tree it may
/// do one thing more. The window offset the hit carries is turned into a row of
/// [`App::rows`](warlock_tui::App::rows) by adding
/// [`App::scroll_offset`](warlock_tui::App::scroll_offset), which is the only
/// arithmetic in here, and an offset past the last row is a point on nothing:
/// the window can be taller than the tree in it, and a click on the blank part
/// of a half-full pane is a click in the pane and no more.
///
/// A click on the row that is already selected is the reader asking for
/// something other than the selection they already have, and the thing a file
/// tree does with a second click is open or close the row. So it goes through
/// [`App::toggle_collapsed`](warlock_tui::App::toggle_collapsed) — the very
/// method space goes through, so a directory opens and closes and a row with
/// nothing under it does nothing at all, without this file having to know which
/// is which.
fn click(hit: Hit, app: &App) -> Option<MouseAction> {
    match hit {
        Hit::TreeRow { offset } => {
            let index = app.scroll_offset().saturating_add(usize::from(offset));
            if index >= app.rows().len() {
                Some(MouseAction::Focus(Focus::Tree))
            } else if index == app.selected() {
                Some(MouseAction::ToggleCollapsed)
            } else {
                Some(MouseAction::SelectRow(index))
            }
        }
        Hit::TreeHeader | Hit::TreeBelowRows => Some(MouseAction::Focus(Focus::Tree)),
        Hit::PanelLine { .. } => Some(MouseAction::Focus(Focus::Panel)),
        // A click inside a pane gives that pane the keys, and the composer is a
        // pane: it is only ever hit-tested when it is on screen, so a press on
        // it is somebody pointing at the field they mean to type in.
        Hit::Composer => Some(MouseAction::Focus(Focus::Composer)),
        Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{Action, action_for};

    /// A plain press of `code`, as crossterm reports one with no modifiers.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_and_esc_quit_with_no_pact_running() {
        assert_eq!(
            action_for(press(KeyCode::Char('q')), false),
            Some(Action::Quit)
        );
        assert_eq!(action_for(press(KeyCode::Esc), false), Some(Action::Quit));
    }

    #[test]
    fn esc_cancels_the_pact_in_flight_while_q_and_ctrl_c_still_quit() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(
            action_for(press(KeyCode::Esc), true),
            Some(Action::CancelPact),
            "Esc during a pact stops the pact, not warlock"
        );
        assert_eq!(
            action_for(press(KeyCode::Char('q')), true),
            Some(Action::Quit),
            "and the ways out are still the ways out"
        );
        assert_eq!(action_for(ctrl_c, true), Some(Action::Quit));
    }

    #[test]
    fn esc_is_the_only_key_a_pact_in_flight_changes_the_meaning_of() {
        // Everything else the tree answers to keeps working while a run works,
        // which is the point of running it on a thread at all.
        let codes = [
            KeyCode::Char('q'),
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('k'),
            KeyCode::Char('j'),
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Char(' '),
            KeyCode::Char('o'),
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('s'),
            KeyCode::Char('v'),
            KeyCode::Char('e'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('x'),
        ];

        for code in codes {
            assert_eq!(
                action_for(press(code), true),
                action_for(press(code), false),
                "{code:?} means something different mid-pact"
            );
        }
    }

    #[test]
    fn ctrl_c_quits_but_a_bare_c_does_not() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(action_for(ctrl_c, false), Some(Action::Quit));
        assert_eq!(action_for(press(KeyCode::Char('c')), false), None);
    }

    #[test]
    fn ctrl_c_quits_with_caps_lock_or_shift_held() {
        // Some terminals report Ctrl-C as an upper-case `C` when shift or caps
        // lock is in play; it is still the key everyone reaches for to get out.
        let ctrl_shift_c = KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert_eq!(action_for(ctrl_shift_c, false), Some(Action::Quit));
    }

    #[test]
    fn tab_moves_the_keys_to_the_other_pane() {
        assert_eq!(
            action_for(press(KeyCode::Tab), false),
            Some(Action::ToggleFocus)
        );
    }

    #[test]
    fn tab_means_the_same_thing_during_a_pact() {
        // Esc is the one key a run in flight re-reads, and focus is nothing to
        // do with a run: the tree stays drivable while a pact works
        // (WAR-21.05), so the key that says which pane is being driven has to
        // work then too.
        assert_eq!(
            action_for(press(KeyCode::Tab), true),
            Some(Action::ToggleFocus)
        );
    }

    #[test]
    fn releases_and_repeats_of_tab_move_no_focus() {
        // The same rule as every other key, and with the same consequence: a
        // release acted on would put focus straight back where the press took
        // it from, so one keystroke would look like none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Tab,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of Tab should not move focus"
            );
        }
    }

    #[test]
    fn tab_is_the_only_key_that_moves_focus() {
        // Its neighbours on the keyboard and the keys it sits between in the
        // match arms above, plus the back-tab a terminal sends for Shift-Tab,
        // which is a keystroke of its own: it swaps the panel's card, and moving
        // focus is the one thing it must not be confused with.
        for code in [
            KeyCode::BackTab,
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Up,
            KeyCode::Char('p'),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleFocus),
                "{code:?} should not move focus"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::ToggleFocus),
                "{code:?} should not move focus mid-pact"
            );
        }
    }

    #[test]
    fn up_and_k_move_the_selection_up() {
        assert_eq!(
            action_for(press(KeyCode::Up), false),
            Some(Action::SelectPrevious)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('k')), false),
            Some(Action::SelectPrevious)
        );
    }

    #[test]
    fn down_and_j_move_the_selection_down() {
        assert_eq!(
            action_for(press(KeyCode::Down), false),
            Some(Action::SelectNext)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('j')), false),
            Some(Action::SelectNext)
        );
    }

    #[test]
    fn page_up_and_page_down_move_the_selection_by_a_screenful() {
        assert_eq!(
            action_for(press(KeyCode::PageUp), false),
            Some(Action::SelectPageUp)
        );
        assert_eq!(
            action_for(press(KeyCode::PageDown), false),
            Some(Action::SelectPageDown)
        );
    }

    #[test]
    fn lower_g_jumps_to_the_first_row_and_upper_g_to_the_last() {
        assert_eq!(
            action_for(press(KeyCode::Char('g')), false),
            Some(Action::SelectFirst)
        );
        assert_eq!(
            action_for(press(KeyCode::Char('G')), false),
            Some(Action::SelectLast)
        );
    }

    #[test]
    fn upper_g_still_jumps_to_the_last_row_with_shift_reported() {
        // Terminals disagree about whether the modifier rides along with the
        // upper-case letter; both spellings are the same keystroke.
        let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);

        assert_eq!(action_for(shift_g, false), Some(Action::SelectLast));
    }

    #[test]
    fn releases_and_repeats_of_the_new_movement_keys_move_nothing() {
        let codes = [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('g'),
            KeyCode::Char('G'),
        ];

        for code in codes {
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                let event = KeyEvent::new_with_kind_and_state(
                    code,
                    KeyModifiers::NONE,
                    kind,
                    KeyEventState::NONE,
                );

                assert_eq!(
                    action_for(event, false),
                    None,
                    "{kind:?} of {code:?} should not move anything"
                );
            }
        }
    }

    #[test]
    fn space_toggles_the_collapse_of_the_selected_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char(' ')), false),
            Some(Action::ToggleCollapsed)
        );
    }

    #[test]
    fn releases_and_repeats_of_space_collapse_nothing() {
        // The same rule as every other key: a release acted on would expand
        // again what the press had just collapsed, so one keystroke would look
        // like none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char(' '),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of space should not collapse anything"
            );
        }
    }

    #[test]
    fn space_is_the_only_key_that_collapses() {
        // Neighbours on the keyboard and in the match arms above, in case a
        // space ever gets typed into the wrong pattern.
        for code in [
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Char('s'),
            KeyCode::Char('p'),
            KeyCode::Char('g'),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleCollapsed),
                "{code:?} should not collapse anything"
            );
        }
    }

    #[test]
    fn o_toggles_the_pacted_only_filter() {
        assert_eq!(
            action_for(press(KeyCode::Char('o')), false),
            Some(Action::TogglePactedOnly)
        );
    }

    #[test]
    fn releases_and_repeats_of_o_filter_nothing() {
        // The same rule as space: a release acted on would restore the whole
        // tree the press had just narrowed, so one keystroke would look like
        // none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('o'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of o should not filter anything"
            );
        }
    }

    #[test]
    fn o_is_the_only_key_that_filters() {
        // Its neighbours on the keyboard, the key it sits next to in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('i'),
            KeyCode::Char('p'),
            KeyCode::Char('k'),
            KeyCode::Char('l'),
            KeyCode::Char('O'),
            KeyCode::Char('r'),
            KeyCode::Char(' '),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::TogglePactedOnly),
                "{code:?} should not filter anything"
            );
        }
    }

    #[test]
    fn f_toggles_the_files_inside_each_directory() {
        assert_eq!(
            action_for(press(KeyCode::Char('f')), false),
            Some(Action::ToggleFiles)
        );
    }

    #[test]
    fn releases_and_repeats_of_f_show_nothing() {
        // The same rule as space and `o`: a release acted on would hide again
        // the files the press had just shown, so one keystroke would look like
        // none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('f'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of f should not show anything"
            );
        }
    }

    #[test]
    fn f_is_the_only_key_that_shows_files() {
        // Its neighbours on the keyboard, the keys it sits between in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('d'),
            KeyCode::Char('g'),
            KeyCode::Char('r'),
            KeyCode::Char('o'),
            KeyCode::Char('p'),
            KeyCode::Char('F'),
            KeyCode::Char(' '),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleFiles),
                "{code:?} should not show any files"
            );
        }
    }

    #[test]
    fn p_toggles_the_pact_on_the_selected_node() {
        assert_eq!(
            action_for(press(KeyCode::Char('p')), false),
            Some(Action::TogglePact)
        );
    }

    #[test]
    fn releases_and_repeats_of_p_write_nothing() {
        // The same rule as for movement, and it matters more here: a release
        // acted on would undo the pact the press had just written, and a held
        // key would rewrite the manifest as fast as the terminal repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('p'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} should not write anything"
            );
        }
    }

    #[test]
    fn r_asks_for_a_refresh_with_a_run_in_flight_or_without_one() {
        // Like every key but Esc, `r` means one thing in both situations: what
        // a refresh does about a run already working is the app's answer to
        // give, and a second `p` is refused in exactly the same place.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::Char('r')), in_flight),
                Some(Action::Refresh),
                "r should ask for a refresh with a run in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn upper_r_asks_for_nothing() {
        // Lower case only, like `o`, `f`, `p` and `m`: the upper-case letter is
        // a different keystroke, and leaving it unbound keeps it free for a
        // later one.
        for in_flight in [false, true] {
            assert_eq!(action_for(press(KeyCode::Char('R')), in_flight), None);
        }
    }

    #[test]
    fn releases_and_repeats_of_r_start_nothing() {
        // The same rule as `p`, and it matters for the same reason: a release
        // acted on would ask for a second run on the heels of the one the press
        // started, and a held key would ask as fast as the terminal repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('r'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of r should not start anything"
            );
        }
    }

    #[test]
    fn r_is_the_only_key_that_refreshes() {
        // Its neighbours on the keyboard, the keys it sits beside in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('e'),
            KeyCode::Char('t'),
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Char('m'),
            KeyCode::Char('R'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::Refresh),
                "{code:?} should not refresh anything"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::Refresh),
                "{code:?} should not refresh anything mid-run"
            );
        }
    }

    #[test]
    fn s_asks_for_the_scope_prompt_with_a_run_in_flight_or_without_one() {
        // Like `p` and `r`, and like every key but Esc, `s` means one thing in
        // both situations: a run in flight is a reason to refuse the prompt,
        // and refusing is the loop's answer to give rather than this
        // function's.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::Char('s')), in_flight),
                Some(Action::OpenScope),
                "s should ask for the prompt with a run in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn upper_s_asks_for_nothing() {
        // Lower case only, like `o`, `f`, `p`, `r` and `m`: the upper-case
        // letter is a different keystroke, and leaving it unbound keeps it free
        // for a later one.
        for in_flight in [false, true] {
            assert_eq!(action_for(press(KeyCode::Char('S')), in_flight), None);
        }
    }

    #[test]
    fn releases_and_repeats_of_s_open_nothing() {
        // The same rule as `p` and `r`, and here it decides whether the prompt
        // can be typed into at all: acting on a release would reopen the prompt
        // on the release of the very key that opened it, and a held `s` would
        // reopen it — losing whatever had been typed — as fast as the terminal
        // repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('s'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of s should open nothing"
            );
        }
    }

    #[test]
    fn s_is_the_only_key_that_scopes() {
        // Its neighbours on the keyboard, the keys it sits between in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('d'),
            KeyCode::Char('w'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('m'),
            KeyCode::Char('S'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::OpenScope),
                "{code:?} should not ask for a scope"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::OpenScope),
                "{code:?} should not ask for a scope mid-run"
            );
        }
    }

    #[test]
    fn v_asks_to_read_the_selected_file_with_a_run_in_flight_or_without_one() {
        // Like `p`, `r` and `s`, and like every key but Esc, `v` means one
        // thing in both situations — and here the mode has nothing it could
        // change even in principle: a read is not a run, so there is no second
        // run for it to be refused as.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::Char('v')), in_flight),
                Some(Action::ViewFile),
                "v should ask for the file with a run in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn upper_v_asks_for_nothing() {
        // Lower case only, like `o`, `f`, `p`, `r`, `s` and `m`: the upper-case
        // letter is a different keystroke, and leaving it unbound keeps it free
        // for a later one.
        for in_flight in [false, true] {
            assert_eq!(action_for(press(KeyCode::Char('V')), in_flight), None);
        }
    }

    #[test]
    fn releases_and_repeats_of_v_read_nothing() {
        // The same rule as the keys above. Nothing is written by this one, so a
        // stray read costs no manifest — but a held `v` would re-read the file
        // from disk as fast as the terminal repeats, and throw the panel's
        // window back to the top of it every time.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('v'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of v should read nothing"
            );
        }
    }

    #[test]
    fn v_is_the_only_key_that_reads_a_file() {
        // Its neighbours on the keyboard, the keys it sits between in the match
        // arms above, and its upper-case self, which this binding does not
        // answer to.
        for code in [
            KeyCode::Char('c'),
            KeyCode::Char('b'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('s'),
            KeyCode::Char('m'),
            KeyCode::Char('V'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ViewFile),
                "{code:?} should not read a file"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::ViewFile),
                "{code:?} should not read a file mid-run"
            );
        }
    }

    #[test]
    fn e_asks_to_edit_the_selected_file_with_a_run_in_flight_or_without_one() {
        // Like `p`, `r`, `s` and `v`, and like every key but Esc, `e` means one
        // thing in both situations. A run in flight is a reason to refuse the
        // editor — the terminal cannot be handed to a child while a pass is
        // still drawing on it — but refusing is the loop's answer to give, in
        // the same place a second `p` is refused, and not this function's.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::Char('e')), in_flight),
                Some(Action::EditFile),
                "e should ask for the editor with a run in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn upper_e_asks_for_nothing() {
        // Lower case only, like `o`, `f`, `p`, `r`, `s`, `v` and `m`: the
        // upper-case letter is a different keystroke, and leaving it unbound
        // keeps it free for a later one.
        for in_flight in [false, true] {
            assert_eq!(action_for(press(KeyCode::Char('E')), in_flight), None);
        }
    }

    #[test]
    fn releases_and_repeats_of_e_start_nothing() {
        // The same rule as the keys above, and it matters here as much as it
        // does for `p`: a release acted on would hand the terminal to a second
        // editor the moment the first one was asked for, and a held `e` would
        // suspend warlock as fast as the terminal repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('e'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of e should start nothing"
            );
        }
    }

    #[test]
    fn e_is_the_only_key_that_edits_a_file() {
        // Its neighbours on the keyboard, the keys it sits beside in the match
        // arms above — `v` first, since viewing a file and editing one are the
        // two halves this binding must not blur — and its upper-case self,
        // which this binding does not answer to.
        for code in [
            KeyCode::Char('v'),
            KeyCode::Char('w'),
            KeyCode::Char('r'),
            KeyCode::Char('p'),
            KeyCode::Char('s'),
            KeyCode::Char('m'),
            KeyCode::Char('E'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::EditFile),
                "{code:?} should not edit a file"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::EditFile),
                "{code:?} should not edit a file mid-run"
            );
        }
    }

    #[test]
    fn shift_tab_swaps_the_panel_card_with_a_run_in_flight_or_without_one() {
        // Crossterm spells Shift-Tab `BackTab`, and like every key but Esc it
        // means one thing in both situations — here there is nothing the mode
        // could change even in principle: both cards are already in the app, so
        // a swap races nothing and there is no run for it to be refused as. A
        // run that could take a document off the screen is the whole thing this
        // binding exists to prevent.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::BackTab), in_flight),
                Some(Action::SwapCard),
                "Shift-Tab should swap the card with a run in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn releases_and_repeats_of_shift_tab_swap_nothing() {
        // The same rule as Tab, and with the same consequence: a release acted
        // on would swap straight back to the card the press had just left, so
        // one keystroke would look like none at all.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::BackTab,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of Shift-Tab should swap nothing"
            );
        }
    }

    #[test]
    fn shift_tab_is_the_only_key_that_swaps_the_card() {
        // Tab first, because the two are one shift apart and a terminal that
        // reported the modifier on an ordinary `Tab` is the accident worth
        // catching; then the keys it sits between in the match arms above and
        // `v`, which is the other key that decides what the panel shows.
        for code in [
            KeyCode::Tab,
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Char('v'),
            KeyCode::Char(' '),
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::SwapCard),
                "{code:?} should not swap the panel's card"
            );
            assert_ne!(
                action_for(press(code), true),
                Some(Action::SwapCard),
                "{code:?} should not swap the panel's card mid-run"
            );
        }
    }

    #[test]
    fn m_toggles_the_mouse_with_a_pact_in_flight_or_without_one() {
        // The one key here that is about the terminal rather than the tree, and
        // it reads the same way in both situations — like everything but Esc.
        // Mid-run is in fact when a reader most wants it: the panel is filling
        // up with output worth copying, and copying it means handing the pointer
        // back to the terminal for a moment.
        for in_flight in [false, true] {
            assert_eq!(
                action_for(press(KeyCode::Char('m')), in_flight),
                Some(Action::ToggleMouseCapture),
                "m should toggle capture with a pact in flight = {in_flight}"
            );
        }
    }

    #[test]
    fn the_mouse_key_neither_quits_nor_moves_anything() {
        // Said against every other action by name, because what the key must not
        // do is the interesting half of it: it does not leave, it does not stop a
        // run, it does not move the keys to the other pane and it does not touch
        // a row. One variant is all it can come to, and the list below is the
        // rest of them.
        for in_flight in [false, true] {
            let action = action_for(press(KeyCode::Char('m')), in_flight);
            for other in [
                Action::Quit,
                Action::CancelPact,
                Action::ToggleFocus,
                Action::SelectPrevious,
                Action::SelectNext,
                Action::SelectPageUp,
                Action::SelectPageDown,
                Action::SelectFirst,
                Action::SelectLast,
                Action::ToggleCollapsed,
                Action::TogglePactedOnly,
                Action::ToggleFiles,
                Action::TogglePact,
                Action::Refresh,
                Action::OpenScope,
                Action::ViewFile,
                Action::EditFile,
                Action::SwapCard,
            ] {
                assert_ne!(action, Some(other), "m should not mean {other:?}");
            }
        }
    }

    #[test]
    fn m_is_the_only_key_that_touches_the_mouse() {
        // Its neighbours in the match arms above, the letter beside it on the
        // keyboard, and its upper-case self, which this binding does not answer
        // to any more than `o`, `f` and `p` answer to theirs.
        for code in [
            KeyCode::Char('n'),
            KeyCode::Char('o'),
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('M'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_ne!(
                action_for(press(code), false),
                Some(Action::ToggleMouseCapture),
                "{code:?} should not touch the mouse"
            );
        }
    }

    #[test]
    fn releases_and_repeats_of_m_toggle_nothing() {
        // The same rule as the keys above, and here it is the difference between
        // a working key and none: a release acted on would turn capture straight
        // back on after the press turned it off, and a held `m` would flip the
        // terminal's reporting as fast as it repeats.
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Char('m'),
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} of m should not toggle anything"
            );
        }
    }

    #[test]
    fn keys_with_no_meaning_here_are_ignored() {
        assert_eq!(action_for(press(KeyCode::Char('x')), false), None);
        assert_eq!(action_for(press(KeyCode::Enter), false), None);
        assert_eq!(action_for(press(KeyCode::Left), false), None);
    }

    #[test]
    fn releases_and_repeats_are_ignored_so_one_keystroke_moves_one_row() {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let event = KeyEvent::new_with_kind_and_state(
                KeyCode::Down,
                KeyModifiers::NONE,
                kind,
                KeyEventState::NONE,
            );

            assert_eq!(
                action_for(event, false),
                None,
                "{kind:?} should not move anything"
            );
        }
    }

    /// The gate on the way out: what Esc and `q` come to now that a question
    /// stands in front of them, and what the question does with everything else.
    ///
    /// Two layers again, as in [`pointer`] below. [`press_for`] is asked what a
    /// key *means*, which is the pure part and the only place a variant is
    /// named; [`round`] — the loop's key arms written out a second time — is
    /// asked what it *does*, so that "answering No changes nothing" is one
    /// comparison of an app against a copy of itself rather than a list of
    /// fields. No terminal is entered and no frame is drawn: the whole gate is a
    /// function of a key, a mode and a flag.
    ///
    /// The composer is decided here too, and after both windows — see
    /// [`press_for`] for why that order — so [`round_composing`] below is the
    /// loop's arms once more with the draft in them, and [`composing`] is where
    /// the rules that are about a field rather than about a way out are asserted.
    mod gate {
        use std::time::Instant;

        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        use ratatui::layout::Size;
        use warlock_engine::NodeState;
        use warlock_tui::{
            Answer, App, Composed, Composer, Edited, Focus, QuitConfirm, Row, ScopeField,
            ScopePrompt, edit_for, panel_height, tree_height,
        };

        use super::super::{Action, Pressed, action_for, press_for};

        /// The terminal these tests measure their app against: the same eighty
        /// by twenty-four every other test here uses.
        const SIZE: Size = Size {
            width: 80,
            height: 24,
        };

        /// The directory the scope prompt is opened over below, so a test that
        /// meant to assert about the text cannot pass by asserting about this.
        const DIRECTORY: &str = "crates/warlock-engine";

        /// Every key the tree answers to, plus a character bound to nothing:
        /// the list either window has to swallow whole, so that no keystroke
        /// reaches the app behind it.
        const INERT: [KeyCode; 19] = [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Char(' '),
            KeyCode::Char('o'),
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('s'),
            KeyCode::Char('v'),
            KeyCode::Char('m'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('x'),
        ];

        /// Whether a round of the loop ended the session: the loop's
        /// `return Ok(())` written down as a value, so a test can assert that
        /// warlock stayed as flatly as it asserts that it left.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Round {
            /// The loop went round again.
            Stayed,
            /// The loop returned, which is the whole of quitting.
            Left,
        }

        /// A plain press of `code`, as crossterm reports one with no modifiers.
        fn press(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        /// Ctrl-C, as crossterm reports it in raw mode: a key event like any
        /// other, which is exactly why the gate has to answer it first.
        fn ctrl_c() -> KeyEvent {
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        }

        /// A tree with more rows than the screen holds, told how big that
        /// screen is — which is what the top of the event loop does every round.
        ///
        /// Half the directories are pacted and half are not, so that the
        /// pacted-only filter has something to hide and something to keep: with
        /// it on there are still rows to select, collapse and scroll past, which
        /// is what lets [`app_in_use`] hold both filters off their defaults at
        /// once. Each file is in the state of the directory listing it, as the
        /// loader has it.
        fn app_on_screen() -> App {
            let mut rows = vec![
                Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale)
                    .with_child_count(12),
            ];
            for n in 0..12 {
                let directory = format!("/repo/d{n:02}");
                let state = if n % 2 == 0 {
                    NodeState::PactedFresh
                } else {
                    NodeState::Unpacted
                };
                rows.push(Row::new(1, directory.clone(), None, state));
                rows.push(Row::file(2, format!("{directory}/lib.rs"), state));
            }
            let mut app = App::from_rows(rows);
            app.set_viewport_height(tree_height(SIZE));
            app.set_panel_height(panel_height(SIZE, None));
            app
        }

        /// That app, moved off its defaults in every way the confirmation
        /// promises to leave alone.
        ///
        /// Selection, scroll offset, panel offset, focus, both filters, the
        /// collapsed set and the message: a copy of this is what the No answer
        /// is compared against, so each of them is somewhere it would not be if
        /// a key had leaked through. The counts come with the rows and are
        /// compared along with everything else, because the comparison is
        /// [`App`]'s own — every field, named or not.
        fn app_in_use() -> App {
            let mut app = app_on_screen();
            app.toggle_files();
            app.toggle_pacted_only();
            app.select_row(9);
            app.toggle_collapsed();
            app.select_previous();
            // A panel with more in it than its window holds, so that scrolling
            // it back is a real offset rather than a no-op: an app with no
            // account has exactly one place its window can be, and a field that
            // cannot move cannot catch a key that moved it.
            let started = Instant::now();
            app.start_account(started);
            if let Some(account) = app.account_mut() {
                for n in 0..40 {
                    account.open_section(format!("/repo/d{n:02}"), started);
                }
            }
            app.scroll_panel_up(5);
            app.set_focus(Focus::Panel);
            app.set_message("something worth keeping");
            app
        }

        /// One round of the event loop with `key` arriving in it and the gate at
        /// `confirm`: the answer worked out, then done, then said.
        ///
        /// The loop's key arms written out a second time, as [`pointer`]'s
        /// `round` is for the pointer, so the tests below are about an app and a
        /// mode rather than about the name of a variant. Nothing is in flight
        /// here, which is the only situation the question can be up in at all —
        /// the gate does not open during a run, and no key that reaches the app
        /// while it is up could start one.
        ///
        /// What it does with each answer, and which four arms panic, is
        /// [`round_composing`] below: this is that round with no window and an
        /// empty draft nobody is pointed at.
        fn round(app: &mut App, confirm: &mut QuitConfirm, key: KeyEvent) -> Round {
            round_under(app, confirm, &mut ScopePrompt::Closed, key)
        }

        /// The composer the loop offers [`press_for`] with the keys where `app`
        /// has them: `Some` only while the focus is on the field.
        ///
        /// The one line the event loop has, written once here so that every test
        /// below asks the question the loop asks. A test that handed the field
        /// over regardless would be asserting that a draft catches keystrokes
        /// aimed at the tree.
        fn offered<'a>(app: &App, composer: &'a Composer) -> Option<&'a Composer> {
            (app.focus() == Focus::Composer).then_some(composer)
        }

        /// The same round with the scope prompt at `prompt` as well: the other
        /// window the tree's bindings go inert under.
        ///
        /// A parameter rather than a second copy of the arms below, exactly as
        /// in [`pointer`]: a test that swallowed keys through a kinder version
        /// of the loop written beside it would be testing the version it wrote.
        /// The prompt's own answer is applied here the way the loop applies it
        /// — the field replaced, or the prompt taken down — and a submit does
        /// nothing to the app, because writing a manifest is not something an
        /// [`App`] hears about.
        fn round_under(
            app: &mut App,
            confirm: &mut QuitConfirm,
            prompt: &mut ScopePrompt,
            key: KeyEvent,
        ) -> Round {
            round_composing(app, confirm, prompt, &mut Composer::default(), key)
        }

        /// The same round again with the draft at `composer`: the whole of the
        /// loop's key handling, and the version the other two call.
        ///
        /// A parameter for [`round_under`]'s reason, and the composer is offered
        /// to [`press_for`] through [`offered`] rather than by the test saying
        /// so — which is the loop's own line, so a test cannot type into a field
        /// the app is not pointed at. The three arms it adds are the loop's:
        /// the draft replaced, the keyboard handed back to the panel, and a
        /// submit that does nothing whatever.
        ///
        /// The four arms the loop answers with a worker thread, a window or an
        /// escape sequence — the pact key, the refresh key, the scope key and
        /// the mouse key — panic rather than doing nothing quietly: a key that
        /// reached one of those from behind a window, or from a composer that
        /// was supposed to be typing it, is precisely the accident these tests
        /// exist to catch.
        fn round_composing(
            app: &mut App,
            confirm: &mut QuitConfirm,
            prompt: &mut ScopePrompt,
            composer: &mut Composer,
            key: KeyEvent,
        ) -> Round {
            match press_for(key, *confirm, prompt, offered(app, composer), false) {
                Pressed::Leave | Pressed::Act(Action::Quit) => return Round::Left,
                Pressed::Confirm(next) => *confirm = next,
                Pressed::Scope(Edited::Open(field)) => *prompt = ScopePrompt::Open(field),
                Pressed::Scope(Edited::Close) => *prompt = ScopePrompt::Closed,
                // What a submit comes to is a manifest saved on the loop's own
                // thread, and nothing an app can see: the prompt stays up until
                // the engine has judged the text, which is the next slice's.
                // All this arm can say is where the key came from, and it says
                // it rather than nothing so that a submit conjured out of a
                // closed prompt would be caught here.
                Pressed::Scope(Edited::Submit) => {
                    assert!(prompt.is_open(), "a submit came from a prompt that is up");
                }
                // The loop's three composer arms, and the reason the draft is a
                // local here exactly as it is there: nothing about it is ever
                // handed to the app.
                Pressed::Compose(Composed::Typing(next)) => *composer = next,
                Pressed::Compose(Composed::Leave) => app.set_focus(Focus::Panel),
                // Inert, as it is in the loop: this slice has no consumer for a
                // submitted draft, so nothing is started, nothing is written and
                // the footer is told nothing. What is asserted rather than done
                // is where the key came from — a submit conjured out of a blank
                // draft, or out of a composer nobody was pointed at, would be
                // caught here.
                Pressed::Compose(Composed::Submit) => {
                    assert_eq!(
                        app.focus(),
                        Focus::Composer,
                        "a submit came from a composer that has the keyboard"
                    );
                    assert!(
                        composer.is_submittable(),
                        "a submit came from a draft with something in it"
                    );
                }
                Pressed::Act(Action::ToggleFocus) => app.toggle_focus(),
                Pressed::Act(Action::SelectPrevious) => app.select_previous(),
                Pressed::Act(Action::SelectNext) => app.select_next(),
                Pressed::Act(Action::SelectPageUp) => app.select_page_up(),
                Pressed::Act(Action::SelectPageDown) => app.select_page_down(),
                Pressed::Act(Action::SelectFirst) => app.select_first(),
                Pressed::Act(Action::SelectLast) => app.select_last(),
                Pressed::Act(Action::ToggleCollapsed) => app.toggle_collapsed(),
                Pressed::Act(Action::TogglePactedOnly) => app.toggle_pacted_only(),
                Pressed::Act(Action::ToggleFiles) => app.toggle_files(),
                // With the plain arms rather than the panicking ones below: a
                // swap is answered by the app between two frames, like a
                // collapse or a filter, and it starts no worker, opens no window
                // and writes nothing to the terminal. What matters here is that
                // it is done at all — a Shift-Tab that leaked past either window
                // would change the card under it, which is exactly what the
                // `app == before` assertions are watching for.
                Pressed::Act(Action::SwapCard) => app.swap_card(),
                Pressed::Act(
                    action @ (Action::CancelPact
                    | Action::TogglePact
                    | Action::Refresh
                    | Action::OpenScope
                    | Action::ViewFile
                    | Action::EditFile
                    | Action::ToggleMouseCapture),
                ) => panic!("{action:?} reached the app"),
                Pressed::Nothing => {}
            }
            Round::Stayed
        }

        #[test]
        fn the_app_the_no_answer_is_compared_against_is_off_its_defaults() {
            // The teeth behind every `assert_eq!(app, before)` below. An app
            // sitting on its defaults would compare equal to one a leaked
            // keystroke had put back there, so each of the things the
            // confirmation promises to leave alone is somewhere a stray key
            // would move it away from — and the fixture is asserted rather than
            // assumed, because a later edit that flattened it would leave the
            // tests passing and testing nothing.
            let app = app_in_use();
            let fresh = app_on_screen();

            assert!(app.show_files(), "the file filter is on");
            assert!(app.pacted_only(), "and so is the pacted-only filter");
            assert_ne!(app.selected(), fresh.selected(), "the selection has moved");
            assert_ne!(
                app.panel_scroll_offset(),
                0,
                "the panel's window is off the top"
            );
            assert!(
                !app.panel_follows(),
                "and no longer following the newest line"
            );
            assert_eq!(app.focus(), Focus::Panel, "the panel has the keys");
            assert!(app.message().is_some(), "and there is a line worth keeping");
            assert_ne!(
                app.rows().len(),
                fresh.rows().len(),
                "something is collapsed or filtered out of the list"
            );
        }

        #[test]
        fn esc_and_q_ask_before_they_leave() {
            // The whole ticket in one assertion each: the key that used to end
            // the session now puts a question in front of it, with the safe
            // answer lit.
            for code in [KeyCode::Esc, KeyCode::Char('q')] {
                let mut app = app_in_use();
                let before = app.clone();
                let mut confirm = QuitConfirm::Closed;

                assert_eq!(
                    round(&mut app, &mut confirm, press(code)),
                    Round::Stayed,
                    "{code:?} should not leave on its own"
                );
                assert_eq!(confirm, QuitConfirm::Open(Answer::No));
                assert_eq!(app, before, "opening the question changed nothing");
            }
        }

        #[test]
        fn the_question_swallows_every_key_the_tree_answers_to() {
            // Asserted at both highlight positions, and in both layers: the key
            // comes to a mode and never to an `Action`, and the app behind the
            // dialog is the app that was there before it opened.
            for lit in [Answer::Yes, Answer::No] {
                let mut app = app_in_use();
                let before = app.clone();
                let mut confirm = QuitConfirm::Open(lit);

                for code in INERT {
                    assert_eq!(
                        press_for(
                            press(code),
                            QuitConfirm::Open(lit),
                            &ScopePrompt::Closed,
                            None,
                            false
                        ),
                        Pressed::Confirm(QuitConfirm::Open(lit)),
                        "{code:?} should reach neither the app nor the way out with {lit:?} lit"
                    );
                    assert_eq!(round(&mut app, &mut confirm, press(code)), Round::Stayed);
                }

                assert_eq!(
                    confirm,
                    QuitConfirm::Open(lit),
                    "the highlight did not move"
                );
                assert_eq!(app, before, "nothing reached the tree underneath");
            }
        }

        #[test]
        fn answering_yes_leaves_by_the_road_a_quit_already_takes() {
            // Both spellings of Yes, and the same value Ctrl-C comes to: one
            // road out of the loop means one `return Ok(())`, so the terminal
            // guard restores the screen and a running `claude` is taken down by
            // the run's own drop, exactly as before this gate existed.
            for key in [press(KeyCode::Char('y')), press(KeyCode::Enter)] {
                let mut app = app_in_use();
                let mut confirm = QuitConfirm::Open(Answer::Yes);

                assert_eq!(
                    press_for(key, confirm, &ScopePrompt::Closed, None, false),
                    Pressed::Leave
                );
                assert_eq!(
                    press_for(key, confirm, &ScopePrompt::Closed, None, false),
                    press_for(ctrl_c(), confirm, &ScopePrompt::Closed, None, false)
                );
                assert_eq!(round(&mut app, &mut confirm, key), Round::Left);
            }
        }

        #[test]
        fn answering_no_closes_the_question_and_leaves_the_app_untouched() {
            // The three ways of saying No — the key, the key that opened the
            // question, and Enter on the answer that is lit when it opens — each
            // with the highlight walked over to Yes and back first, so the app
            // is compared after a handful of keystrokes rather than after one.
            for code in [KeyCode::Char('n'), KeyCode::Esc, KeyCode::Enter] {
                let mut app = app_in_use();
                let before = app.clone();
                let mut confirm = QuitConfirm::Closed;

                assert_eq!(
                    round(&mut app, &mut confirm, press(KeyCode::Esc)),
                    Round::Stayed
                );
                assert_eq!(
                    round(&mut app, &mut confirm, press(KeyCode::Left)),
                    Round::Stayed
                );
                assert_eq!(
                    round(&mut app, &mut confirm, press(KeyCode::Right)),
                    Round::Stayed
                );
                assert_eq!(
                    round(&mut app, &mut confirm, press(code)),
                    Round::Stayed,
                    "{code:?} should answer No"
                );

                assert_eq!(confirm, QuitConfirm::Closed, "the question came down");
                assert_eq!(app, before, "and took nothing with it");
            }
        }

        #[test]
        fn the_reflex_second_esc_closes_the_question_rather_than_the_session() {
            // The accident this gate exists for, spelled out: two presses of the
            // key nearest to hand leave warlock exactly where it was.
            let mut app = app_in_use();
            let before = app.clone();
            let mut confirm = QuitConfirm::Closed;

            for _ in 0..4 {
                assert_eq!(
                    round(&mut app, &mut confirm, press(KeyCode::Esc)),
                    Round::Stayed
                );
            }

            assert_eq!(confirm, QuitConfirm::Closed, "an even number of presses");
            assert_eq!(app, before);
        }

        #[test]
        fn ctrl_c_leaves_at_once_with_the_question_up_or_down() {
            // Answered before the mode is consulted, which is what keeps it out
            // of `answer_for`'s "every other key" arm: through there it would be
            // an ordinary `c` with a modifier riding along, and the one
            // keystroke every reader trusts would be the one the dialog ate.
            //
            // Pinned with the composer holding the keyboard as well as without
            // it, because the field is the third thing that could have eaten the
            // key: through `compose_for` it is a chord rather than text, i.e.
            // one of the keys that change nothing, so a gate that consulted the
            // draft first would swallow it in silence.
            let draft = Composer::new("web");
            for confirm in [
                QuitConfirm::Closed,
                QuitConfirm::Open(Answer::No),
                QuitConfirm::Open(Answer::Yes),
            ] {
                for composer in [None, Some(&draft)] {
                    for in_flight in [false, true] {
                        assert_eq!(
                            press_for(ctrl_c(), confirm, &ScopePrompt::Closed, composer, in_flight),
                            Pressed::Leave,
                            "Ctrl-C should leave with {confirm:?}, {composer:?} and a run in \
                             flight = {in_flight}"
                        );
                    }
                }
            }

            let mut app = app_in_use();
            let mut confirm = QuitConfirm::open();
            assert_eq!(round(&mut app, &mut confirm, ctrl_c()), Round::Left);
        }

        #[test]
        fn a_run_in_flight_puts_no_question_in_front_of_anybody() {
            // Esc still cancels the run and `q` still leaves, pinned at both
            // settings of the flag: the gate is for the twitch that follows a
            // cancel, and during a run Esc already means cancel.
            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    None,
                    true
                ),
                Pressed::Act(Action::CancelPact),
            );
            assert_eq!(
                press_for(
                    press(KeyCode::Char('q')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    None,
                    true
                ),
                Pressed::Leave,
            );

            // And the same two keys with nothing running, which is the only
            // difference the flag makes here.
            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::open()),
            );
            assert_eq!(
                press_for(
                    press(KeyCode::Char('q')),
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    None,
                    false
                ),
                Pressed::Confirm(QuitConfirm::open()),
            );
        }

        #[test]
        fn every_other_key_still_means_what_it_always_meant() {
            // The gate is one question in front of two keys and nothing else:
            // with it closed, every binding reaches the app as before, at both
            // settings of the flag.
            for in_flight in [false, true] {
                for code in INERT {
                    assert_eq!(
                        press_for(
                            press(code),
                            QuitConfirm::Closed,
                            &ScopePrompt::Closed,
                            None,
                            in_flight
                        ),
                        action_for(press(code), in_flight).map_or(Pressed::Nothing, Pressed::Act),
                        "{code:?} should read as it always has, in flight = {in_flight}"
                    );
                }
            }
        }

        #[test]
        fn releases_and_repeats_neither_open_the_question_nor_answer_it() {
            // The same rule the two key functions already keep, and here it is
            // the difference between a gate and no gate: acting on a release
            // would answer the question with the release of the very key that
            // opened it.
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('y')] {
                    let key = KeyEvent::new_with_kind_and_state(
                        code,
                        KeyModifiers::NONE,
                        kind,
                        KeyEventState::NONE,
                    );

                    assert_eq!(
                        press_for(key, QuitConfirm::Closed, &ScopePrompt::Closed, None, false),
                        Pressed::Nothing,
                        "{kind:?} of {code:?} should open nothing"
                    );
                    assert_eq!(
                        press_for(key, QuitConfirm::open(), &ScopePrompt::Closed, None, false),
                        Pressed::Confirm(QuitConfirm::open()),
                        "{kind:?} of {code:?} should answer nothing"
                    );
                }
            }
        }

        #[test]
        fn the_scope_prompt_swallows_every_key_the_tree_answers_to() {
            // The confirmation's rule, said again for the other window: while
            // somebody is typing a scope, `j`, `k`, `g`, `G`, space, `o`, `f`,
            // `p`, `r`, `s`, `m`, Tab and the page keys are letters going into a
            // field or keystrokes that mean nothing, and `action_for` is not
            // consulted at all. Both layers: the key comes back as the prompt's
            // own answer and never as an `Action`, and the app behind the window
            // is the app that was there before it opened.
            //
            // Asserted on an empty field and on one already holding a scope,
            // because what is in the field is nothing to do with what the gate
            // does with a key.
            for text in ["", "data-plane"] {
                let mut app = app_in_use();
                let before = app.clone();
                let field = ScopeField::new(DIRECTORY, text);
                let mut prompt = ScopePrompt::Open(field.clone());
                let mut confirm = QuitConfirm::Closed;

                for code in INERT {
                    let key = press(code);
                    let pressed = press_for(key, QuitConfirm::Closed, &prompt, None, false);

                    assert_eq!(
                        pressed,
                        Pressed::Scope(edit_for(key, prompt.field().expect("the prompt is up"))),
                        "{code:?} should go to the prompt and nowhere else"
                    );
                    assert!(
                        matches!(pressed, Pressed::Scope(_)),
                        "{code:?} reached something other than the prompt: {pressed:?}"
                    );

                    assert_eq!(
                        round_under(&mut app, &mut confirm, &mut prompt, key),
                        Round::Stayed
                    );
                }

                assert!(prompt.is_open(), "the prompt is still up");
                assert_eq!(confirm, QuitConfirm::Closed, "and no question was opened");
                assert_eq!(app, before, "nothing reached the tree underneath");
            }
        }

        #[test]
        fn esc_and_q_belong_to_the_prompt_while_it_is_up() {
            // The order the gate decides in, where it is visible: the prompt is
            // asked before `action_for`, so `q` is a character somebody typed
            // rather than a way out, and Esc takes the prompt down rather than
            // putting a question in front of a session nobody asked to end.
            let field = ScopeField::new(DIRECTORY, "web");
            let prompt = ScopePrompt::Open(field.clone());

            assert_eq!(
                press_for(
                    press(KeyCode::Char('q')),
                    QuitConfirm::Closed,
                    &prompt,
                    None,
                    false
                ),
                Pressed::Scope(Edited::Open(ScopeField::new(DIRECTORY, "webq"))),
                "q is a letter while the field has the keyboard"
            );
            assert_eq!(
                press_for(
                    press(KeyCode::Esc),
                    QuitConfirm::Closed,
                    &prompt,
                    None,
                    false
                ),
                Pressed::Scope(Edited::Close),
                "Esc abandons the prompt rather than opening the question"
            );

            // And through the loop's arms: the prompt comes down, the question
            // does not go up, and the app never heard either keystroke.
            let mut app = app_in_use();
            let before = app.clone();
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::Open(field);

            assert_eq!(
                round_under(&mut app, &mut confirm, &mut prompt, press(KeyCode::Esc)),
                Round::Stayed
            );
            assert_eq!(prompt, ScopePrompt::Closed, "the prompt came down");
            assert_eq!(confirm, QuitConfirm::Closed, "and nothing took its place");
            assert_eq!(app, before);
        }

        #[test]
        fn ctrl_c_leaves_at_once_with_the_scope_prompt_up() {
            // Answered before either window is consulted, and for the reason it
            // is answered before the question: through `edit_for` it is a `c`
            // wearing a modifier, i.e. one of the keys that change nothing, and
            // the last resort of a reader who wants out would be the one
            // keystroke the field swallowed. Pinned with an empty field, with
            // something typed, and at both settings of the run flag.
            for prompt in [
                ScopePrompt::open(DIRECTORY, ""),
                ScopePrompt::open(DIRECTORY, "data-plane"),
            ] {
                for in_flight in [false, true] {
                    assert_eq!(
                        press_for(ctrl_c(), QuitConfirm::Closed, &prompt, None, in_flight),
                        Pressed::Leave,
                        "Ctrl-C should leave with {prompt:?} up and a run in flight = {in_flight}"
                    );
                }
            }

            let mut app = app_in_use();
            let mut confirm = QuitConfirm::Closed;
            let mut prompt = ScopePrompt::open(DIRECTORY, "billing");
            assert_eq!(
                round_under(&mut app, &mut confirm, &mut prompt, ctrl_c()),
                Round::Left
            );
        }

        #[test]
        fn the_order_is_ctrl_c_the_question_the_prompt_the_composer_then_the_keys() {
            // The whole decision order in one test, each step asserted by taking
            // the situation above it away and pressing the same key again. `j`
            // is the key it is said with because it means something different to
            // every one of them: a letter to both fields, a key the question
            // ignores, and a movement to the app.
            let key = press(KeyCode::Char('j'));
            let draft = Composer::new("web");
            let prompt = ScopePrompt::open(DIRECTORY, "web");
            let question = QuitConfirm::open();

            // Ctrl-C, over all three at once. It is a key event and not a
            // signal, so if the gate does not answer it here nothing does.
            assert_eq!(
                press_for(ctrl_c(), question, &prompt, Some(&draft), false),
                Pressed::Leave
            );
            // Then the question, which is drawn over everything else on the
            // frame: a key cannot be both typed into a field and answered by the
            // dialog covering it.
            assert_eq!(
                press_for(key, question, &prompt, Some(&draft), false),
                Pressed::Confirm(question)
            );
            // Then the prompt, over the composer, for the same reason again.
            assert_eq!(
                press_for(key, QuitConfirm::Closed, &prompt, Some(&draft), false),
                Pressed::Scope(edit_for(key, prompt.field().expect("the prompt is up")))
            );
            // Then the composer, over the keys: this is where `j` stops being a
            // movement and becomes the letter j.
            assert_eq!(
                press_for(
                    key,
                    QuitConfirm::Closed,
                    &ScopePrompt::Closed,
                    Some(&draft),
                    false
                ),
                Pressed::Compose(Composed::Typing(Composer::new("webj")))
            );
            // And then the keys, as they have always been read.
            assert_eq!(
                press_for(key, QuitConfirm::Closed, &ScopePrompt::Closed, None, false),
                Pressed::Act(Action::SelectNext)
            );
        }

        /// The keyboard in the composer: what the tree's own bindings come to
        /// while somebody is typing, and what they come to again once the field
        /// has let go.
        ///
        /// The same two layers as the module above, and the same road: every
        /// test here asks [`press_for`] what a key *means* and then puts it
        /// through [`round_composing`], which is the loop's arms with the draft
        /// in them. Nothing is drawn and no terminal is entered — where the
        /// composer sits on the frame is `ui.rs`'s, what a key does to the draft
        /// itself is `composer.rs`'s, and what is asserted here is only which of
        /// the two functions a key reaches.
        ///
        /// The pairs are the point. Each key is asserted twice — once as a
        /// letter with the focus on the field, once as the command it has always
        /// been with the focus off it — because either half alone is a field that
        /// works or a tree that works, and the ticket is both at once.
        mod composing {
            use std::time::Instant;

            use super::{
                Action, App, Composed, Composer, Focus, INERT, KeyCode, Pressed, QuitConfirm,
                Round, ScopePrompt, app_in_use, app_on_screen, ctrl_c, offered, press, press_for,
                round_composing,
            };

            /// What is in the draft before each test types anything.
            ///
            /// Something rather than nothing, so a key that appended nothing is
            /// told apart from a key that replaced everything, and short enough
            /// that the expected draft can be read at a glance.
            const TYPED: &str = "web";

            /// The app with the keyboard in the composer: [`app_in_use`] with
            /// focus moved on one place, which is where Tab from the panel puts
            /// it.
            fn app_composing() -> App {
                let mut app = app_in_use();
                app.set_focus(Focus::Composer);
                assert_eq!(
                    app.focus(),
                    Focus::Composer,
                    "the composer can hold the keyboard with the account card up"
                );
                app
            }

            /// `code` pressed with the composer holding [`TYPED`]: it is the
            /// letter, it goes into the draft, and no [`Action`] comes of it.
            ///
            /// Both layers, because they are different claims. The first is that
            /// [`press_for`] answers with the composer's own outcome, which is
            /// the plain statement that `action_for` was not consulted; the
            /// second is that a round of the loop leaves the app it was holding
            /// byte for byte — and [`round_composing`] panics on the four keys
            /// that start a run or open a window, so a `p` that leaked through
            /// would not be a quiet failure.
            fn types(code: char) {
                let key = press(KeyCode::Char(code));
                let before = Composer::new(TYPED);
                let typed = Composer::new(format!("{TYPED}{code}"));

                assert_eq!(
                    press_for(
                        key,
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&before),
                        false
                    ),
                    Pressed::Compose(Composed::Typing(typed.clone())),
                    "{code} should be a letter while the composer has the keyboard"
                );

                let mut app = app_composing();
                let untouched = app.clone();
                let mut composer = before;
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert_eq!(
                    round_composing(&mut app, &mut confirm, &mut prompt, &mut composer, key),
                    Round::Stayed,
                    "{code} should not end the session"
                );
                assert_eq!(composer, typed, "{code} should have gone into the draft");
                assert_eq!(app, untouched, "{code} reached the app behind the composer");
                assert_eq!(confirm, QuitConfirm::Closed, "and opened no question");
                assert_eq!(prompt, ScopePrompt::Closed, "and no prompt");
            }

            /// `code` pressed with the keys anywhere but the composer: the
            /// action it has always meant, and a draft nobody typed into.
            ///
            /// Asserted at both of the other two places focus can be, because
            /// what makes the key a command is that the field does not have the
            /// keyboard rather than which pane does. The situation goes in
            /// through [`offered`], the loop's own line, so the test cannot
            /// arrange something the loop would not.
            fn acts(code: char, action: Action) {
                let key = press(KeyCode::Char(code));
                let composer = Composer::new(TYPED);

                for focus in [Focus::Tree, Focus::Panel] {
                    let mut app = app_in_use();
                    app.set_focus(focus);

                    assert_eq!(
                        press_for(
                            key,
                            QuitConfirm::Closed,
                            &ScopePrompt::Closed,
                            offered(&app, &composer),
                            false
                        ),
                        Pressed::Act(action),
                        "{code} should mean {action:?} again with the keys at {focus:?}"
                    );
                }

                assert_eq!(
                    composer,
                    Composer::new(TYPED),
                    "{code} should have typed nothing anywhere"
                );
            }

            #[test]
            fn p_is_the_letter_p_while_the_composer_has_the_keyboard() {
                // The key the whole arrangement is for: `p` writes a manifest,
                // so a letter that pacted a directory would be the one typo that
                // costs somebody minutes of model time.
                types('p');
            }

            #[test]
            fn p_pacts_again_once_the_composer_has_let_go() {
                acts('p', Action::TogglePact);
            }

            #[test]
            fn r_is_the_letter_r_while_the_composer_has_the_keyboard() {
                types('r');
            }

            #[test]
            fn r_refreshes_again_once_the_composer_has_let_go() {
                acts('r', Action::Refresh);
            }

            #[test]
            fn s_is_the_letter_s_while_the_composer_has_the_keyboard() {
                // And a window that opened over the field somebody is typing in
                // would take the keyboard off them mid-sentence.
                types('s');
            }

            #[test]
            fn s_scopes_again_once_the_composer_has_let_go() {
                acts('s', Action::OpenScope);
            }

            #[test]
            fn v_is_the_letter_v_while_the_composer_has_the_keyboard() {
                types('v');
            }

            #[test]
            fn v_reads_a_file_again_once_the_composer_has_let_go() {
                acts('v', Action::ViewFile);
            }

            #[test]
            fn e_is_the_letter_e_while_the_composer_has_the_keyboard() {
                // The worst of them to leak: `e` hands the terminal to an editor,
                // so a typed letter would take the screen away mid-draft.
                types('e');
            }

            #[test]
            fn e_edits_a_file_again_once_the_composer_has_let_go() {
                acts('e', Action::EditFile);
            }

            #[test]
            fn f_is_the_letter_f_while_the_composer_has_the_keyboard() {
                types('f');
            }

            #[test]
            fn f_shows_the_files_again_once_the_composer_has_let_go() {
                acts('f', Action::ToggleFiles);
            }

            #[test]
            fn g_is_the_letter_g_while_the_composer_has_the_keyboard() {
                types('g');
            }

            #[test]
            fn g_jumps_to_the_first_row_again_once_the_composer_has_let_go() {
                acts('g', Action::SelectFirst);
            }

            #[test]
            fn upper_g_is_the_letter_g_while_the_composer_has_the_keyboard() {
                // Its own test rather than a second case of `g`'s: the pair is
                // told apart by case alone, so a field that folded the letter
                // would be a field somebody cannot write a sentence in.
                types('G');
            }

            #[test]
            fn upper_g_jumps_to_the_last_row_again_once_the_composer_has_let_go() {
                acts('G', Action::SelectLast);
            }

            #[test]
            fn j_is_the_letter_j_while_the_composer_has_the_keyboard() {
                types('j');
            }

            #[test]
            fn j_moves_the_selection_down_again_once_the_composer_has_let_go() {
                acts('j', Action::SelectNext);
            }

            #[test]
            fn k_is_the_letter_k_while_the_composer_has_the_keyboard() {
                types('k');
            }

            #[test]
            fn k_moves_the_selection_up_again_once_the_composer_has_let_go() {
                acts('k', Action::SelectPrevious);
            }

            #[test]
            fn every_other_binding_the_tree_has_is_the_composers_too() {
                // The ten keys above one by one, and then the rest of the list in
                // a loop: space, `o`, `m`, Shift-Tab, the arrows and the page
                // keys are text or nothing while the field has the keyboard, and
                // none of them is an `Action`. Tab is the exception and has its
                // own test below.
                let mut app = app_composing();
                let untouched = app.clone();
                let mut composer = Composer::new(TYPED);
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                for code in INERT.into_iter().filter(|code| *code != KeyCode::Tab) {
                    let pressed = press_for(
                        press(code),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        false,
                    );

                    assert!(
                        matches!(pressed, Pressed::Compose(_)),
                        "{code:?} reached something other than the composer: {pressed:?}"
                    );
                    assert_eq!(
                        round_composing(
                            &mut app,
                            &mut confirm,
                            &mut prompt,
                            &mut composer,
                            press(code)
                        ),
                        Round::Stayed
                    );
                }

                assert_eq!(app, untouched, "nothing reached the app underneath");
                assert_eq!(confirm, QuitConfirm::Closed, "and no question was opened");
                assert_eq!(prompt, ScopePrompt::Closed, "and no prompt");
            }

            #[test]
            fn tab_still_moves_the_keyboard_on_rather_than_being_typed() {
                // The one key the composer does not get. It is not text on any
                // terminal, and a field that swallowed it would be a field whose
                // only way out is Esc — which means something else.
                let composer = Composer::new(TYPED);

                assert_eq!(
                    press_for(
                        press(KeyCode::Tab),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        false
                    ),
                    Pressed::Act(Action::ToggleFocus)
                );

                let mut app = app_composing();
                let mut composer = composer;
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Tab)
                    ),
                    Round::Stayed
                );
                assert_eq!(app.focus(), Focus::Tree, "the cycle went on round");
                assert_eq!(composer.draft(), TYPED, "and typed nothing on the way");
            }

            #[test]
            fn esc_hands_the_keyboard_back_and_leaves_the_draft_where_it_is() {
                let composer = Composer::new(TYPED);

                assert_eq!(
                    press_for(
                        press(KeyCode::Esc),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        false
                    ),
                    Pressed::Compose(Composed::Leave),
                    "Esc belongs to the field rather than to the gate on the way out"
                );

                let mut app = app_composing();
                let mut expected = app.clone();
                expected.set_focus(Focus::Panel);
                let mut composer = composer;
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Esc)
                    ),
                    Round::Stayed,
                    "Esc at the composer does not end the session"
                );
                assert_eq!(app, expected, "it moved the focus to the panel and no more");
                assert_eq!(composer.draft(), TYPED, "and threw nothing away");
                assert_eq!(confirm, QuitConfirm::Closed, "and asked nothing");
            }

            #[test]
            fn esc_at_the_composer_leaves_a_run_alone_and_the_next_one_cancels_it() {
                // Deliberate, and the same rule the scope prompt keeps: the Esc
                // pressed while a field has the keyboard is answered by that
                // field, and the press after it — with the keyboard back on the
                // panel — is the one that stops the run.
                let composer = Composer::new(TYPED);

                assert_eq!(
                    press_for(
                        press(KeyCode::Esc),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        true
                    ),
                    Pressed::Compose(Composed::Leave)
                );
                assert_eq!(
                    press_for(
                        press(KeyCode::Esc),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        None,
                        true
                    ),
                    Pressed::Act(Action::CancelPact)
                );
            }

            #[test]
            fn enter_offers_the_draft_up_and_the_loop_does_nothing_whatever_with_it() {
                // The submission has no consumer in this slice: nothing is
                // started, nothing is spawned and nothing is written. The round
                // panics on every arm that would do any of those, so "inert" is
                // asserted rather than described — and the app it was holding
                // comes out of the round unchanged, message and all.
                let composer = Composer::new("why nine passes");

                assert_eq!(
                    press_for(
                        press(KeyCode::Enter),
                        QuitConfirm::Closed,
                        &ScopePrompt::Closed,
                        Some(&composer),
                        false
                    ),
                    Pressed::Compose(Composed::Submit)
                );

                let mut app = app_composing();
                let untouched = app.clone();
                let mut composer = composer;
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert_eq!(
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Enter)
                    ),
                    Round::Stayed
                );
                assert_eq!(app, untouched, "a submit changed something");
                assert_eq!(
                    composer.draft(),
                    "why nine passes",
                    "and the draft is left for the consumer this slice does not have"
                );
                assert_eq!(prompt, ScopePrompt::Closed, "and opened no window");
            }

            #[test]
            fn an_empty_or_blank_submit_puts_no_message_on_the_footer() {
                // A submission with nothing in it is a keystroke, not a mistake:
                // it leaves the draft as it was and says nothing at all. Asserted
                // on an app with a clean footer, so a line put there would be the
                // only line there is.
                for draft in ["", " ", "  \t ", "\n", " \n \n "] {
                    let mut app = app_on_screen();
                    app.set_focus(Focus::Composer);
                    let untouched = app.clone();
                    let mut composer = Composer::new(draft);
                    let mut confirm = QuitConfirm::Closed;
                    let mut prompt = ScopePrompt::Closed;

                    assert!(app.message().is_none(), "the footer starts with nothing");
                    assert_eq!(
                        round_composing(
                            &mut app,
                            &mut confirm,
                            &mut prompt,
                            &mut composer,
                            press(KeyCode::Enter)
                        ),
                        Round::Stayed
                    );

                    assert_eq!(
                        app.message(),
                        None,
                        "submitting {draft:?} said something on the footer"
                    );
                    assert_eq!(app, untouched, "and changed something");
                    assert_eq!(composer, Composer::new(draft), "and moved the draft");
                }
            }

            #[test]
            fn ctrl_c_leaves_at_once_and_types_no_c_while_the_composer_has_it() {
                // The order the gate decides in, where it matters most: through
                // `compose_for` Ctrl-C is a chord rather than text, so a gate
                // that consulted the field first would answer the one keystroke
                // every reader trusts with nothing at all — and would not even
                // leave a `c` behind to show for it.
                for draft in ["", TYPED] {
                    let composer = Composer::new(draft);

                    for in_flight in [false, true] {
                        assert_eq!(
                            press_for(
                                ctrl_c(),
                                QuitConfirm::Closed,
                                &ScopePrompt::Closed,
                                Some(&composer),
                                in_flight
                            ),
                            Pressed::Leave,
                            "Ctrl-C should leave from {draft:?} with a run in flight = {in_flight}"
                        );
                    }
                }

                let mut app = app_composing();
                let mut composer = Composer::new(TYPED);
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                assert_eq!(
                    round_composing(&mut app, &mut confirm, &mut prompt, &mut composer, ctrl_c()),
                    Round::Left
                );
                assert_eq!(composer.draft(), TYPED, "and typed no c on the way out");
            }

            #[test]
            fn the_draft_survives_esc_the_focus_cycle_and_a_run_that_started_and_ended() {
                // Where the draft is kept, said as a fact about a session rather
                // than as a rule somebody follows: it is a local of the event
                // loop, so nothing that happens to the `App` can reach it. The
                // run is the case that decides it — a pact or a refresh that
                // recorded nothing puts the copy taken before it back over the
                // live app and keeps only the panel (`App::restore_from`), so a
                // draft stored there would be a draft a run swallowed half a
                // sentence into.
                let mut app = app_composing();
                let mut composer = Composer::default();
                let mut confirm = QuitConfirm::Closed;
                let mut prompt = ScopePrompt::Closed;

                for character in "why nine".chars() {
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Char(character)),
                    );
                }
                assert_eq!(composer.draft(), "why nine");

                // Esc: the keyboard goes back to the panel and the draft stays.
                round_composing(
                    &mut app,
                    &mut confirm,
                    &mut prompt,
                    &mut composer,
                    press(KeyCode::Esc),
                );
                assert_eq!(app.focus(), Focus::Panel);
                assert_eq!(composer.draft(), "why nine", "Esc threw the draft away");

                // A run that started and ended with nothing recorded, which is
                // the one move that replaces the whole app.
                let before = app.clone();
                app.start_account(Instant::now());
                app.restore_from(before);
                assert_eq!(
                    composer.draft(),
                    "why nine",
                    "a run that ended took the draft with it"
                );

                // And the focus all the way round the cycle: panel, composer,
                // tree, panel.
                for _ in 0..3 {
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Tab),
                    );
                }
                assert_eq!(app.focus(), Focus::Panel, "back where it started");
                assert_eq!(
                    composer.draft(),
                    "why nine",
                    "the focus cycle typed into the draft or emptied it"
                );

                // Typing carries on exactly where it left off.
                app.set_focus(Focus::Composer);
                for character in " passes".chars() {
                    round_composing(
                        &mut app,
                        &mut confirm,
                        &mut prompt,
                        &mut composer,
                        press(KeyCode::Char(character)),
                    );
                }
                assert_eq!(composer.draft(), "why nine passes");
            }
        }
    }

    /// What the pointer comes to: which move a notch of the wheel or a press of
    /// the left button at a named point on a named screen asks the app for.
    ///
    /// Every test here builds its own event, names its own [`Size`] and builds
    /// its app out of rows written down in this file. No terminal is entered, no
    /// frame is drawn and nothing is attached to stdout — which is the whole
    /// reason the pointer's answer is a function of an event, a size and an app
    /// rather than something the event loop does inline.
    ///
    /// Two layers are asserted, and they are different things. Most tests ask
    /// [`asks`] — that is [`mouse_action`] over the screen below, with the gate
    /// on the way out closed — what a point *means*, which is the pure part. The
    /// few that care what the reader would see also go through [`round`], which
    /// is the event loop's arms written out a second time, so
    /// that "three rows a notch, clamped" and "a click on the row already
    /// selected opens it" are asserted about an app rather than about a variant
    /// name. What each of those moves does on its own is `app.rs`'s to test, and
    /// is not restated here.
    mod pointer {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Size;
        use warlock_engine::NodeState;
        use warlock_tui::{App, Focus, QuitConfirm, Row, ScopePrompt, panel_height, tree_height};

        use super::super::{MouseAction, mouse_action};

        /// The terminal every test here points at, and the layout it comes to.
        ///
        /// Eighty by twenty-four is the terminal every other program's defaults
        /// assume, and it is wide enough that the tree takes its floor of thirty
        /// columns rather than its share — the even-split branch of the layout is
        /// `ui.rs`'s to test, and what is being tested here is what a point
        /// means, not where the panes are.
        ///
        /// ```text
        /// columns  0        panel        49 50       tree        79
        /// row  0   ┌───────────────────────┐┌───────────────────────┐
        /// row  1   │ panel line 0          ││ tree header           │
        /// row  2   │ panel line 1          ││ tree row 0            │
        ///  ...     │  ...                  ││  ...                  │
        /// row 19   │ panel line 18         ││ tree row 17           │
        /// row 20   └───────────────────────┘└───────────────────────┘
        /// rows 21-23                     the footer
        /// ```
        const SIZE: Size = Size {
            width: 80,
            height: 24,
        };

        /// A column inside the tree pane, well clear of either border.
        const IN_TREE: u16 = 65;

        /// A column inside the panel, likewise.
        const IN_PANEL: u16 = 10;

        /// The screen row the tree's first drawn row is on: the pane's top
        /// border, then its header.
        const FIRST_TREE_ROW: u16 = 2;

        /// The screen row the panel's first drawn line is on: the pane's top
        /// border and no header, because the panel has none.
        const FIRST_PANEL_LINE: u16 = 1;

        /// The one line inside the tree pane's border that names the tree.
        const TREE_HEADER: u16 = 1;

        /// A row of the footer — the middle of its three.
        const FOOTER: u16 = 22;

        /// How many rows of tree this screen has room for, which is what the
        /// event loop tells the app before it draws. Asserted rather than
        /// assumed, so a layout that ever changed shape says so here.
        fn viewport() -> usize {
            usize::from(tree_height(SIZE))
        }

        /// One notch of the wheel towards the newest line, at a point.
        fn wheel_down(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::ScrollDown, column, row)
        }

        /// One notch of the wheel back, at a point.
        fn wheel_up(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::ScrollUp, column, row)
        }

        /// The left button going down at a point, which is the half of a click
        /// warlock answers.
        fn left_click(column: u16, row: u16) -> MouseEvent {
            event(MouseEventKind::Down(MouseButton::Left), column, row)
        }

        /// A mouse event of `kind` at a point, as crossterm reports one with no
        /// modifiers held.
        fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }
        }

        /// Twenty-five rows: a root, then twenty-four directories with a file
        /// apiece.
        ///
        /// More rows than the screen above has room for, so a window that has
        /// been scrolled is a state these tests can get into; and each directory
        /// claims the one child it is given, because a row with no children is a
        /// row [`App::toggle_collapsed`] refuses, and a click on the selected
        /// row has to be tested against a row it does not refuse as well as
        /// against one it does.
        fn rows() -> Vec<Row> {
            let mut rows = vec![
                Row::new(0, "/repo", "/repo/WARLOCK.md", NodeState::PactedStale)
                    .with_child_count(24),
            ];
            for n in 0..24 {
                let directory = format!("/repo/d{n:02}");
                // No child count: the row under each of these is the file below,
                // and a file is not a child. What makes them collapsible is the
                // file toggle being on, which is `App::can_collapse`'s answer
                // and not the tree's.
                rows.push(Row::new(1, directory.clone(), None, NodeState::Unpacted));
                rows.push(Row::file(
                    2,
                    format!("{directory}/lib.rs"),
                    NodeState::Unpacted,
                ));
            }
            rows
        }

        /// Those rows, in an app told how big the screen above is — which is
        /// what the top of the event loop does every round, and what the hit
        /// test's answers have to agree with.
        ///
        /// The file rows are hidden, as the file toggle starts, so the drawn
        /// list is the root and its twenty-four directories.
        fn app_on_screen() -> App {
            let mut app = App::from_rows(rows());
            app.set_viewport_height(tree_height(SIZE));
            app.set_panel_height(panel_height(SIZE, None));
            app
        }

        /// What `mouse` over [`SIZE`] asks `app` for with both windows down,
        /// which is the situation every test here but the last two is about.
        ///
        /// Named so the question the pointer tests ask stays one line long now
        /// that the confirmation and the scope prompt are things a pointer event
        /// is read against.
        fn asks(mouse: MouseEvent, app: &App) -> Option<MouseAction> {
            mouse_action(
                mouse,
                SIZE,
                app,
                QuitConfirm::Closed,
                &ScopePrompt::Closed,
                None,
            )
        }

        /// One round of the event loop with `mouse` arriving in it and the gate
        /// at `confirm`: the answer worked out and then done, which is the
        /// loop's arms written out again.
        ///
        /// Here so that a test can assert about a selection and a focus rather
        /// than about the name of a variant. It is the pointer's whole road, and
        /// a change to the loop that this stopped matching would be a change one
        /// of the tests below is asserting the old shape of.
        fn round(app: &mut App, confirm: QuitConfirm, mouse: MouseEvent) {
            round_under(app, confirm, &ScopePrompt::Closed, mouse);
        }

        /// The same round with the scope prompt at `prompt` as well: the other
        /// window the pointer goes inert under.
        ///
        /// A parameter rather than a second copy of the arms below, so the two
        /// windows are asserted against the one road out of [`mouse_action`] —
        /// a test that dropped events through a second, kinder version of the
        /// loop would be testing the version it wrote.
        fn round_under(
            app: &mut App,
            confirm: QuitConfirm,
            prompt: &ScopePrompt,
            mouse: MouseEvent,
        ) {
            match mouse_action(mouse, SIZE, app, confirm, prompt, None) {
                Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
                Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
                Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
                Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
                Some(MouseAction::SelectRow(index)) => {
                    app.set_focus(Focus::Tree);
                    app.select_row(index);
                }
                Some(MouseAction::ToggleCollapsed) => {
                    app.set_focus(Focus::Tree);
                    app.toggle_collapsed();
                }
                Some(MouseAction::Focus(focus)) => app.set_focus(focus),
                None => {}
            }
        }

        #[test]
        fn the_screen_these_tests_point_at_is_the_one_they_describe() {
            // The table above is load-bearing: every point below is a literal
            // read off it, so a layout that moved would otherwise turn these
            // tests into assertions about somewhere else.
            assert_eq!(viewport(), 18, "eighteen rows of tree at 80x24");
            assert_eq!(
                usize::from(panel_height(SIZE, None)),
                19,
                "nineteen lines of panel: no header of its own"
            );
        }

        #[test]
        fn a_notch_over_the_tree_moves_the_selection_three_rows() {
            let app = app_on_screen();

            assert_eq!(
                asks(wheel_down(IN_TREE, FIRST_TREE_ROW + 4), &app),
                Some(MouseAction::SelectNextBy(3)),
            );
            assert_eq!(
                asks(wheel_up(IN_TREE, FIRST_TREE_ROW + 4), &app),
                Some(MouseAction::SelectPreviousBy(3)),
            );
            // Every part of the pane's inside answers for the pane, the header
            // included: a wheel is aimed at a column, and a notch that did
            // nothing because the pointer sat on the naming line would read as a
            // wheel that sticks.
            assert_eq!(
                asks(wheel_down(IN_TREE, TREE_HEADER), &app),
                Some(MouseAction::SelectNextBy(3)),
            );
        }

        #[test]
        fn three_notched_rows_are_three_pressed_ones_and_stop_at_the_ends() {
            let mut app = app_on_screen();
            let mut pressed = app.clone();
            for _ in 0..3 {
                pressed.select_next();
            }

            round(
                &mut app,
                QuitConfirm::Closed,
                wheel_down(IN_TREE, FIRST_TREE_ROW),
            );
            assert_eq!(app, pressed, "a notch is three presses of the movement key");

            // Clamped at both ends rather than wrapping or running off: the
            // wheel is spun past the end far more easily than a key is held
            // there.
            for _ in 0..20 {
                round(
                    &mut app,
                    QuitConfirm::Closed,
                    wheel_up(IN_TREE, FIRST_TREE_ROW),
                );
            }
            assert_eq!(app.selected(), 0, "stopped at the first row");
            for _ in 0..20 {
                round(
                    &mut app,
                    QuitConfirm::Closed,
                    wheel_down(IN_TREE, FIRST_TREE_ROW),
                );
            }
            assert_eq!(app.selected(), app.rows().len() - 1, "stopped at the last");
        }

        #[test]
        fn a_notch_over_the_panel_scrolls_it_three_lines() {
            let app = app_on_screen();

            assert_eq!(
                asks(wheel_down(IN_PANEL, FIRST_PANEL_LINE + 7), &app),
                Some(MouseAction::ScrollPanelDown(3)),
            );
            assert_eq!(
                asks(wheel_up(IN_PANEL, FIRST_PANEL_LINE), &app),
                Some(MouseAction::ScrollPanelUp(3)),
            );
        }

        #[test]
        fn the_wheel_drives_the_pane_it_is_over_and_moves_no_focus() {
            // The keys are pointed at the panel and the pointer at the tree,
            // which is the case the convention is for: the wheel scrolls what
            // the reader is looking at, and a wheel that scrolled the focused
            // pane instead would move the half of the screen they are not.
            let mut app = app_on_screen();
            app.set_focus(Focus::Panel);
            round(
                &mut app,
                QuitConfirm::Closed,
                wheel_down(IN_TREE, FIRST_TREE_ROW + 2),
            );

            assert_eq!(app.selected(), 3, "the tree moved under the pointer");
            assert_eq!(app.focus(), Focus::Panel, "the keys did not follow");

            // And the other way round: the tree has the keys, the pointer is
            // over the panel, and the notch is the panel's.
            let mut app = app_on_screen();
            let selected = app.selected();
            round(
                &mut app,
                QuitConfirm::Closed,
                wheel_up(IN_PANEL, FIRST_PANEL_LINE),
            );

            assert_eq!(app.focus(), Focus::Tree, "the keys did not follow");
            assert_eq!(app.selected(), selected, "the tree did not move");
        }

        #[test]
        fn a_notch_over_the_footer_or_a_border_does_nothing() {
            let app = app_on_screen();
            // The footer is nobody's pane; a border is the line between two of
            // them rather than a place a reader means to scroll. The columns are
            // the panel's left border, the two panes' shared edge and the tree's
            // right, and the rows are the panes' top and bottom.
            for (column, row) in [
                (IN_PANEL, FOOTER),
                (IN_TREE, FOOTER),
                (0, FIRST_PANEL_LINE),
                (49, FIRST_TREE_ROW),
                (50, FIRST_TREE_ROW),
                (79, FIRST_TREE_ROW),
                (IN_TREE, 0),
                (IN_PANEL, 20),
            ] {
                assert_eq!(
                    asks(wheel_down(column, row), &app),
                    None,
                    "a notch at {column},{row} should change nothing"
                );
                assert_eq!(
                    asks(wheel_up(column, row), &app),
                    None,
                    "a notch at {column},{row} should change nothing"
                );
            }
        }

        #[test]
        fn a_click_on_a_row_selects_it_and_takes_the_keys() {
            let mut app = app_on_screen();
            app.set_focus(Focus::Panel);

            assert_eq!(
                asks(left_click(IN_TREE, FIRST_TREE_ROW + 5), &app),
                Some(MouseAction::SelectRow(5)),
                "the sixth row of a window that has not scrolled"
            );

            round(
                &mut app,
                QuitConfirm::Closed,
                left_click(IN_TREE, FIRST_TREE_ROW + 5),
            );
            assert_eq!(app.selected(), 5);
            assert_eq!(app.focus(), Focus::Tree, "the reader pointed at the tree");
        }

        #[test]
        fn a_click_names_a_row_of_the_tree_and_not_of_the_window() {
            // The window is scrolled to the bottom, so the offset the hit test
            // hands over is short of the row by exactly where the window starts.
            let mut app = app_on_screen();
            app.select_last();
            let offset = app.scroll_offset();
            assert_eq!(
                offset,
                app.rows().len() - viewport(),
                "the window is at the end"
            );

            assert_eq!(
                asks(left_click(IN_TREE, FIRST_TREE_ROW + 3), &app),
                Some(MouseAction::SelectRow(offset + 3)),
            );
        }

        #[test]
        fn a_second_click_on_a_directory_row_opens_and_closes_it() {
            let mut app = app_on_screen();
            // Files shown, so the directory clicked has a row under it to hide.
            // Without them it holds nothing on screen and the collapse refuses,
            // which is what the test below this one is about.
            app.toggle_files();
            // The row under the pointer is selected first, by a click of its
            // own: the second click is the one that collapses, and it is the
            // same point twice.
            let point = left_click(IN_TREE, FIRST_TREE_ROW + 1);
            round(&mut app, QuitConfirm::Closed, point);
            let path = app.selected_row().expect("a row is selected").path.clone();
            assert!(!app.is_collapsed(&path), "nothing collapsed by selecting");

            assert_eq!(asks(point, &app), Some(MouseAction::ToggleCollapsed),);
            round(&mut app, QuitConfirm::Closed, point);
            assert!(app.is_collapsed(&path), "the second click closed it");

            // And back open, which is what space does on the third press too.
            round(&mut app, QuitConfirm::Closed, point);
            assert!(!app.is_collapsed(&path), "the third click opened it");
        }

        #[test]
        fn a_second_click_on_a_file_row_does_nothing_more() {
            // Files shown, so a file row can be pointed at. It is a row like any
            // other to the hit test — what refuses it is the collapse itself,
            // which is exactly what refuses space on the same row.
            let mut app = app_on_screen();
            app.toggle_files();
            let point = left_click(IN_TREE, FIRST_TREE_ROW + 2);
            round(&mut app, QuitConfirm::Closed, point);
            assert!(
                app.selected_row().expect("a row is selected").is_file(),
                "the third drawn row is a file"
            );

            let before = app.clone();
            round(&mut app, QuitConfirm::Closed, point);
            assert_eq!(app, before, "a file row has nothing to open");
        }

        #[test]
        fn a_click_in_the_panel_takes_the_keys_and_no_more() {
            let mut app = app_on_screen();
            let before = app.clone();

            assert_eq!(
                asks(left_click(IN_PANEL, FIRST_PANEL_LINE + 9), &app),
                Some(MouseAction::Focus(Focus::Panel)),
                "the panel has no selection, so focus is the whole of it"
            );

            round(
                &mut app,
                QuitConfirm::Closed,
                left_click(IN_PANEL, FIRST_PANEL_LINE + 9),
            );
            assert_eq!(app.focus(), Focus::Panel);
            assert_eq!(app.selected(), before.selected(), "the tree did not move");
            assert_eq!(
                app.panel_scroll_offset(),
                before.panel_scroll_offset(),
                "the panel's window did not move either"
            );
        }

        #[test]
        fn a_click_on_the_tree_header_takes_the_keys_and_no_more() {
            let mut app = app_on_screen();
            app.set_focus(Focus::Panel);
            let selected = app.selected();

            assert_eq!(
                asks(left_click(IN_TREE, TREE_HEADER), &app),
                Some(MouseAction::Focus(Focus::Tree)),
            );

            round(
                &mut app,
                QuitConfirm::Closed,
                left_click(IN_TREE, TREE_HEADER),
            );
            assert_eq!(app.focus(), Focus::Tree);
            assert_eq!(app.selected(), selected, "the selection did not move");
        }

        #[test]
        fn a_click_below_the_last_row_selects_nothing() {
            // A window taller than the tree in it: one row drawn and seventeen
            // rows of blank pane under it, which is a click in the pane and no
            // more. The app is asked rather than the layout, because only the
            // app knows how many rows it has.
            let mut app = App::from_rows(vec![Row::new(
                0,
                "/repo",
                "/repo/WARLOCK.md",
                NodeState::PactedStale,
            )]);
            app.set_viewport_height(tree_height(SIZE));
            app.set_focus(Focus::Panel);

            assert_eq!(
                asks(left_click(IN_TREE, FIRST_TREE_ROW + 6), &app),
                Some(MouseAction::Focus(Focus::Tree)),
            );

            let before = app.clone();
            round(
                &mut app,
                QuitConfirm::Closed,
                left_click(IN_TREE, FIRST_TREE_ROW + 6),
            );
            assert_eq!(app.focus(), Focus::Tree);
            assert_eq!(app.rows(), before.rows(), "nothing was opened or closed");
            assert_eq!(app.selected(), 0, "the one row stayed selected");
        }

        #[test]
        fn a_click_on_the_footer_or_a_border_does_nothing_at_all() {
            let app = app_on_screen();
            for (column, row) in [
                (IN_PANEL, FOOTER),
                (IN_TREE, FOOTER),
                (0, FIRST_PANEL_LINE),
                (49, FIRST_TREE_ROW),
                (50, FIRST_TREE_ROW),
                (79, FIRST_TREE_ROW),
                (IN_TREE, 0),
                (IN_PANEL, 20),
            ] {
                assert_eq!(
                    asks(left_click(column, row), &app),
                    None,
                    "a click at {column},{row} should change nothing"
                );
            }
        }

        #[test]
        fn everything_but_the_wheel_and_the_left_press_is_read_and_dropped() {
            let app = app_on_screen();
            // Out of scope by decision: hovering, dragging, the release half of
            // a click, the other two buttons and the horizontal wheel. Asked at
            // every kind of point, because dropping them is what keeps a pointer
            // swept across the screen from costing anything — a highlight that
            // followed it would cost a redraw per move to say what the selection
            // already says.
            for kind in [
                MouseEventKind::Moved,
                MouseEventKind::Drag(MouseButton::Left),
                MouseEventKind::Drag(MouseButton::Right),
                MouseEventKind::Up(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Right),
                MouseEventKind::Down(MouseButton::Right),
                MouseEventKind::Down(MouseButton::Middle),
                MouseEventKind::Up(MouseButton::Middle),
                MouseEventKind::ScrollLeft,
                MouseEventKind::ScrollRight,
            ] {
                for (column, row) in [
                    (IN_TREE, FIRST_TREE_ROW),
                    (IN_TREE, TREE_HEADER),
                    (IN_PANEL, FIRST_PANEL_LINE),
                    (IN_PANEL, FOOTER),
                    (50, FIRST_TREE_ROW),
                ] {
                    assert_eq!(
                        asks(event(kind, column, row), &app),
                        None,
                        "{kind:?} at {column},{row} should mean nothing"
                    );
                }
            }
        }

        #[test]
        fn the_pointer_is_read_and_dropped_while_the_confirmation_is_up() {
            // The dialog is answered from the keyboard and has no clickable Yes
            // or No, so a click that got through would land on a tree the
            // reader cannot see, behind a window that is about to close. Asked
            // over the whole pointer — both notches, a click on a row, a click
            // on the row already selected and a click in the panel — and then
            // asserted about the app itself, since "read and dropped" is a
            // claim about what did not move.
            let mut app = app_on_screen();
            app.toggle_files();
            // Selected and focused somewhere other than where it started, so a
            // leak has something to disturb: the panel has the keys and its
            // window has been scrolled back, and the tree's selection is a row
            // down the list rather than the first one.
            app.select_row(9);
            app.scroll_panel_down(4);
            app.set_focus(Focus::Panel);
            let before = app.clone();
            assert_eq!(
                app.scroll_offset(),
                0,
                "the tree's window has not moved, so drawn row nine is row nine"
            );

            for mouse in [
                wheel_down(IN_TREE, FIRST_TREE_ROW),
                wheel_up(IN_TREE, FIRST_TREE_ROW),
                wheel_down(IN_PANEL, FIRST_PANEL_LINE),
                wheel_up(IN_PANEL, FIRST_PANEL_LINE),
                left_click(IN_TREE, FIRST_TREE_ROW),
                left_click(IN_TREE, FIRST_TREE_ROW + 9),
                left_click(IN_TREE, TREE_HEADER),
                left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
            ] {
                assert_eq!(
                    mouse_action(
                        mouse,
                        SIZE,
                        &app,
                        QuitConfirm::open(),
                        &ScopePrompt::Closed,
                        None
                    ),
                    None,
                    "{mouse:?} should mean nothing while the question is up"
                );
                round(&mut app, QuitConfirm::open(), mouse);
            }

            assert_eq!(app, before, "the pointer moved nothing behind the dialog");
        }

        #[test]
        fn the_pointer_is_read_and_dropped_while_the_scope_prompt_is_up() {
            // The same rule as the confirmation above, for the same reasons:
            // the prompt is typed into and has no buttons, and a click that got
            // through would move a selection under a window the reader is in
            // the middle of answering. The whole pointer again — both notches,
            // a click on a row, a click on the row already selected, a click on
            // the header and a click in the panel — with the app asserted
            // afterwards, since "read and dropped" is a claim about what did
            // not move.
            let mut app = app_on_screen();
            app.toggle_files();
            app.select_row(9);
            app.scroll_panel_down(4);
            app.set_focus(Focus::Panel);
            let before = app.clone();
            assert_eq!(
                app.scroll_offset(),
                0,
                "the tree's window has not moved, so drawn row nine is row nine"
            );

            // Both an empty field and one with something typed into it: the
            // gate is the prompt being up, not what is in it.
            for prompt in [
                ScopePrompt::open("crates/warlock-engine", ""),
                ScopePrompt::open("crates/warlock-engine", "data-plane"),
            ] {
                for mouse in [
                    wheel_down(IN_TREE, FIRST_TREE_ROW),
                    wheel_up(IN_TREE, FIRST_TREE_ROW),
                    wheel_down(IN_PANEL, FIRST_PANEL_LINE),
                    wheel_up(IN_PANEL, FIRST_PANEL_LINE),
                    left_click(IN_TREE, FIRST_TREE_ROW),
                    left_click(IN_TREE, FIRST_TREE_ROW + 9),
                    left_click(IN_TREE, TREE_HEADER),
                    left_click(IN_PANEL, FIRST_PANEL_LINE + 3),
                ] {
                    assert_eq!(
                        mouse_action(mouse, SIZE, &app, QuitConfirm::Closed, &prompt, None),
                        None,
                        "{mouse:?} should mean nothing while the prompt is up"
                    );
                    round_under(&mut app, QuitConfirm::Closed, &prompt, mouse);
                }
            }

            assert_eq!(app, before, "the pointer moved nothing behind the prompt");
        }
    }
}
