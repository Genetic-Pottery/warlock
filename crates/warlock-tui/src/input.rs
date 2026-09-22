//! Events turned into intentions, with nothing attached to stdout:
//! [`action_for`] for a key, [`press_for`] for a key once the windows have had
//! their say, and [`mouse_action`] for a pointer event.
//!
//! Everything about the situation arrives as a parameter rather than being
//! looked up, which is what keeps each of the three a pure function and every
//! rule below one assertion. Two of those parameters are read in exactly one
//! arm each: `in_flight` re-reads Esc in [`action_for`] and `q` in
//! [`press_for`], and `answered` re-reads Ctrl-C — which [`press_for`] takes at
//! the top, before [`action_for`] is ever asked. Nothing here decides what a
//! window *is*; [`press_for`] only decides which of them is asked, and the
//! order it asks in is the precedence.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Size;
use warlock_tui::{
    Answered, App, Carry, CarryAnswered, Cell, Composed, Composer, Edited, Focus, Hit,
    PullAnswered, PullConfirm, PushAnswered, PushConfirm, QuitConfirm, Reach, RecordEdited,
    RecordPrompt, Review, Reviewed, ScopePrompt, answer_for, carry_answer_for, compose_for,
    edit_for, hit_test, panel_reach, pull_answer_for, push_answer_for, record_edit_for,
    review_answer_for,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Quit,
    CancelPact,
    ToggleFocus,
    SelectPrevious,
    SelectNext,
    SelectPageUp,
    SelectPageDown,
    SelectFirst,
    SelectLast,
    ToggleCollapsed,
    TogglePactedOnly,
    ToggleFiles,
    TogglePact,
    Refresh,
    OpenScope,
    ViewFile,
    EditFile,
    SwapCard,
    ToggleMouseCapture,
}

pub(crate) fn action_for(key: KeyEvent, in_flight: bool) -> Option<Action> {
    // Presses only. Crossterm reports releases and auto-repeats on Windows and
    // on terminals speaking the Kitty protocol, and not on the rest, so a
    // release acted on would move the selection twice per keystroke on some
    // machines and once on others — and would toggle `p`'s pact straight back
    // off again on the way up.
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
        // Before the quit arm below, and the only arm the mode touches: Esc
        // stops being a way out for as long as there is a run to stop, while
        // `q` and Ctrl-C — the keys a reader reaches for deliberately — go on
        // meaning quit.
        KeyCode::Esc if in_flight => Some(Action::CancelPact),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Tab => Some(Action::ToggleFocus),
        // Shift-Tab is a keystroke of its own: the terminal sends a code for it
        // that crossterm spells `BackTab`, so there is no shift riding along on
        // a `Tab` here to tell the two apart by.
        KeyCode::BackTab => Some(Action::SwapCard),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown => Some(Action::SelectPageDown),
        // The one pair told apart by case, and matched on the character rather
        // than on `SHIFT`: terminals disagree about whether the modifier rides
        // along with an upper-case letter, and about caps lock, so a `G` is a
        // `G` however it is reported.
        KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::Char('G') => Some(Action::SelectLast),
        // Crossterm has no `KeyCode::Space`; the space bar arrives as an
        // ordinary character.
        KeyCode::Char(' ') => Some(Action::ToggleCollapsed),
        // The letters are lower case only, `g`/`G` above excepted: `O`, `F`,
        // `P`, `R`, `S`, `V`, `E` and `M` are keystrokes of their own that mean
        // nothing here rather than second spellings of these, which leaves them
        // free for later bindings.
        KeyCode::Char('o') => Some(Action::TogglePactedOnly),
        KeyCode::Char('f') => Some(Action::ToggleFiles),
        KeyCode::Char('p') => Some(Action::TogglePact),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('s') => Some(Action::OpenScope),
        KeyCode::Char('v') => Some(Action::ViewFile),
        KeyCode::Char('e') => Some(Action::EditFile),
        KeyCode::Char('m') => Some(Action::ToggleMouseCapture),
        _ => None,
    }
}

// `Act` never carries `Action::Quit`: every way out is `Leave`, which is what
// makes "the gate cannot be bypassed" a property of this type rather than a
// rule the event loop is trusted to keep. `Scope` and `Write` are two variants
// over one `Edited` from one `edit_for` because the loop does different things
// with a submit from each, and an `Edited` arriving with no way to say which
// window it came from is exactly the confusion this type exists to prevent.
// `Record` is a third window and a `RecordEdited` of its own, so the same holds
// for it by its type.
//
// Unboxed for the reason `RecordEdited` itself is: this value is built by the
// gate, read by the arm that asked for it and dropped, once per keystroke.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pressed {
    Leave,
    CancelTurn,
    Confirm(QuitConfirm),
    // The push dialog's answer rather than its next state, because one of the
    // three is not a state at all: `Send` is a request going out and the window
    // coming down, and a `PushConfirm::Closed` here could not tell it from the
    // Esc that closes the same window with nothing sent.
    Push(PushAnswered),
    // The question a `/pull` asks once the board has answered, for the reason
    // `Push` carries an answer rather than a next state: `Cut` is a run
    // starting and the window coming down, which a `PullConfirm::Closed` could
    // not be told from the Esc that closes the same window having started
    // nothing.
    Pull(PullAnswered),
    // One slice's drafts answered about: three answers, so its own value rather
    // than the two-answer ones above — see `Reviewed`.
    Review(Reviewed),
    // And the two-answer question a skipped slice leaves behind, which is the
    // quit dialog's shape said in the run's words.
    Carry(CarryAnswered),
    // The field that comes up in front of that dialog when the machine can file
    // to more than one board. A fourth variant over `Edited` for `Scope` and
    // `Write`'s reason: the three are the same keystrokes and three different
    // things to do with a submit.
    Filing(Edited),
    Scope(Edited),
    Record(RecordEdited),
    Write(Edited),
    Compose(Composed),
    Act(Action),
    Nothing,
}

// Spelled here rather than reached for through `action_for`, because
// `press_for` has to answer Ctrl-C before it consults anything else: two
// spellings of the one keystroke that must always work is how it stops working
// in one of them.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

// Every kind of event, releases included, because what this decides is not what
// Tab does but which function is asked: a release handed past the composer
// comes to nothing in `action_for`, where every release already comes to
// nothing.
fn is_tab(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab
}

// The order of the tests below is the precedence, and it is the whole of what
// this function decides.
//
// Ctrl-C first, before the windows: raw mode is exactly the mode in which the
// terminal stops turning it into `SIGINT`, so if nothing here answers it
// nothing does, and routed through the question it would arrive at `answer_for`
// as an ordinary `c` with a modifier riding along — the last resort of a reader
// who wants out, swallowed by the dialog. `answered` is the one thing that
// changes what it means: with a turn out it stops that turn, with none it
// leaves. Esc is deliberately not this key, because a turn and a run are two
// things and the key that stops one must not stop the other.
//
// Then each window, and on those roads `action_for` is not called at all, which
// is the plain statement of "nothing leaks through to the tree underneath". The
// two windows the `s` key puts up are asked before the write prompt because
// either can be up at once with it — `s` opens one from the tree while a
// `/write` turn is still out, and the answer to that turn opens the other with
// no keystroke — and the `s` window is the one somebody is typing in now.
// Between those two themselves there is no precedence to have: the record
// window opens exactly as the scope window closes, so they are never both up.
//
// The push dialog is asked before those three and after the quit one. Before,
// because the three fields can come up underneath it with nobody asking — a
// `/write` turn still out answers into the write prompt on no keystroke at all
// — and the dialog is the window somebody is looking at and the one drawn on
// top; after, because the quit dialog is the gate on the way out and the two
// are never up together anyway (`q` reaches nothing while this is up).
//
// The scope field a `/push` puts up when the machine can file to more than one
// board is asked in the dialog's own place, for the dialog's own reason: it is
// the other half of the same question and the two are never up together — the
// submit that takes this one down is what puts that one up.
//
// The composer is asked after all of them, because a window is drawn over it: a
// key cannot be both typed into a field on the frame and answered by the dialog
// covering it. `composer` is `Some` only when the focus is on the field, which
// is the caller's line, not a lookup here.
#[expect(
    clippy::too_many_arguments,
    reason = "the whole of what a keystroke can mean, and the point of it is that \
              there is one place that decides: each window that can be over the app \
              is a parameter here rather than a gate of its own somewhere else"
)]
pub(crate) fn press_for(
    key: KeyEvent,
    confirm: QuitConfirm,
    push: &PushConfirm,
    pull: &PullConfirm,
    review: Option<&Review>,
    carry: Option<&Carry>,
    filing: &ScopePrompt,
    prompt: &ScopePrompt,
    record: &RecordPrompt,
    write: &ScopePrompt,
    composer: Option<&Composer>,
    in_flight: bool,
    answered: bool,
) -> Pressed {
    if is_ctrl_c(key) {
        return if answered {
            Pressed::CancelTurn
        } else {
            Pressed::Leave
        };
    }

    if let Some(highlighted) = confirm.highlighted() {
        return match answer_for(key, highlighted) {
            Answered::Open(answer) => Pressed::Confirm(QuitConfirm::Open(answer)),
            Answered::Close => Pressed::Confirm(QuitConfirm::Closed),
            Answered::Leave => Pressed::Leave,
        };
    }

    if let Some(asked) = push.filing() {
        return Pressed::Push(push_answer_for(key, asked.answer()));
    }

    // The push dialog's place in the order rather than a place of its own: the
    // two are the same kind of question asked at the same point in the same
    // command's shape, and a session never has both up — a `/pull` is typed
    // into the composer, which takes no keys while either is drawn over it.
    if let Some(asked) = pull.cutting() {
        return Pressed::Pull(pull_answer_for(key, asked.answer()));
    }

    // The two windows a confirmed pull puts up, in the dialog's own place and
    // for its reason: they are the same kind of question at the same point in
    // the same command's shape. They can never be up with it or with each other
    // — the dialog is answered and gone before a slice is drafted, and a slice
    // is being reviewed, or asking whether to carry on, or neither — so the
    // order between the three of them is a statement rather than a choice.
    if let Some(drafts) = review {
        return Pressed::Review(review_answer_for(key, drafts));
    }

    if let Some(asked) = carry {
        return Pressed::Carry(carry_answer_for(key, asked.answer()));
    }

    if let Some(field) = filing.field() {
        return Pressed::Filing(edit_for(key, field));
    }

    if let Some(field) = prompt.field() {
        return Pressed::Scope(edit_for(key, field));
    }

    if let Some(form) = record.form() {
        return Pressed::Record(record_edit_for(key, form));
    }

    if let Some(field) = write.field() {
        return Pressed::Write(edit_for(key, field));
    }

    // Tab goes past the field rather than into it: it is not text on any
    // terminal, and a field that ate it would be a field whose only exit is
    // Esc, which means something else — hand the keyboard back and leave the
    // draft where it is.
    if let Some(draft) = composer
        && !is_tab(key)
    {
        // A muted field takes no keys and hands none on either: a `p` that fell
        // through a dead field to `action_for` would arrive at the tree as the
        // pact key. Nothing is said about it — the dim border says it once,
        // rather than the footer saying it per keystroke.
        if draft.is_muted() {
            return Pressed::Nothing;
        }
        return Pressed::Compose(compose_for(key, draft));
    }

    match action_for(key, in_flight) {
        // The gate, in two arms, and the only use `in_flight` is put to here.
        // With a run out, `q` leaves outright rather than asking: the confirm
        // is for the reflex second Esc after a cancel, not for a reader who has
        // decided, and Esc during a run is the cancel anyway (see
        // `action_for`). Ctrl-C never reaches this arm — it is taken at the top
        // of this function, where `answered` decides what it means.
        Some(Action::Quit) if !in_flight => Pressed::Confirm(QuitConfirm::open()),
        Some(Action::Quit) => Pressed::Leave,
        Some(action) => Pressed::Act(action),
        None => Pressed::Nothing,
    }
}

// One number for both panes, so the gearing does not change under a hand that
// crosses from one to the other.
const WHEEL_NOTCH: usize = 3;

// Three of these are one gesture over the conversation — press, drag, release —
// each carrying the panel cell it landed on, measured once by `cell_under` from
// the hit test the frame was cut by rather than a second time from the screen.
//
// `StartSelection` is the whole of that press rather than a second action
// alongside `Focus(Focus::Panel)`: one event becomes one action here, so the arm
// that anchors a selection is the arm that has to take the keys as well, or a
// press on the thread card would stop focusing the panel.
//
// The last two are the same drag and the same release once the pointer has left
// the card's rows, where there is no cell to name: they carry which edge it went
// past and by how many rows, which is the whole of what a scroll off the tick
// needs. They are never `Reach::Inside` — `past_rows` refuses that, since a
// pointer still level with the rows is a drag sideways and means nothing new.
//
// Still no variant for a hover or for a button other than the left one. Those
// are read and dropped in `mouse_action`, and a name for one here would be an
// invitation to behaviour warlock has decided against — a highlight following a
// pointer nobody is dragging costs a redraw per move to say what the selection
// already says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseAction {
    SelectNextBy(usize),
    SelectPreviousBy(usize),
    ScrollPanelDown(usize),
    ScrollPanelUp(usize),
    SelectRow(usize),
    ToggleCollapsed,
    Focus(Focus),
    StartSelection(Cell),
    ExtendSelection(Cell),
    EndSelection(Cell),
    ExtendPastEdge(Reach),
    EndPastEdge(Reach),
}

// The left button still held after a press that landed on a line of the
// conversation, which is the only gesture anything scrolls off the tick for.
//
// `past` is where the *latest* drag event left the card, and it is remembered
// rather than acted on once because a pointer that has stopped moving sends no
// further events: the round after it has nothing else to read, so the last
// event to arrive has to go on meaning what it said until another one does.
// `None` is a pointer still level with the rows, where the drag is the plain
// cell-by-cell one and there is nothing to scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Drag {
    pub(crate) past: Option<Reach>,
}

// `size` is the size the round measured before it drew, and `composer` the
// draft that frame was drawn with, because the hit test has to agree with the
// frame the reader is pointing at: a second opinion about the layout — or one
// that had not been told the field took rows from the panel — answers
// `PanelLine` for a point on the composer and scrolls a window the pointer is
// not over.
//
// None of the nine windows has anything clickable in it, so while any is up
// every event is dropped, wheel and click alike: a click that reached the tree
// behind one would select a row the reader cannot see.
#[expect(
    clippy::too_many_arguments,
    reason = "`press_for`'s reason, for the pointer: the frame the gesture landed \
              on is what decides what it meant, and every window that could be \
              over it has to be asked here"
)]
pub(crate) fn mouse_action(
    mouse: MouseEvent,
    size: Size,
    app: &App,
    confirm: QuitConfirm,
    push: &PushConfirm,
    pull: &PullConfirm,
    review: Option<&Review>,
    carry: Option<&Carry>,
    filing: &ScopePrompt,
    prompt: &ScopePrompt,
    record: &RecordPrompt,
    write: &ScopePrompt,
    composer: Option<&Composer>,
) -> Option<MouseAction> {
    if confirm.is_open()
        || push.is_open()
        || pull.is_open()
        || review.is_some()
        || carry.is_some()
        || filing.is_open()
        || prompt.is_open()
        || record.is_open()
        || write.is_open()
    {
        return None;
    }

    let header = app.run_header();
    let hit = hit_test(mouse.column, mouse.row, size, composer, header.as_ref());
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
        // A click is answered on its press alone — the half a reader means, and
        // answering both would select a row or collapse it twice. The release
        // below is read for the drag it ends and for nothing else, which is why
        // it is asked about the conversation rather than handed to `click`.
        MouseEventKind::Down(MouseButton::Left) => click(hit, app),
        // A drag or a release with no cell under it is not nothing: the pointer
        // may have been dragged clean off the card, and a stationary pointer
        // past the edge sends no further events, so the direction it left by has
        // to come back with this event or not at all.
        MouseEventKind::Drag(MouseButton::Left) => cell_under(hit, app)
            .map(MouseAction::ExtendSelection)
            .or_else(|| past_rows(mouse, size, app, composer).map(MouseAction::ExtendPastEdge)),
        MouseEventKind::Up(MouseButton::Left) => cell_under(hit, app)
            .map(MouseAction::EndSelection)
            .or_else(|| past_rows(mouse, size, app, composer).map(MouseAction::EndPastEdge)),
        _ => None,
    }
}

// What one pointer event does to the drag being held, which is the whole of
// what the loop's tick has to go on: `held` is what the last event left, and
// what comes back is what this one leaves.
//
// `kind` is read where the actions cannot answer: they do not distinguish a
// press on the border, the footer or off the screen — or a release over a card
// that went up mid-drag — from an event nothing was read into at all. Everything
// a button going down or coming up *can* mean over the conversation says so as
// an action; the rest say nothing, and a drag left standing under one of them
// would go on scrolling and holding the card with nobody's hand on the button.
pub(crate) fn drag_after(
    held: Option<Drag>,
    kind: MouseEventKind,
    action: Option<MouseAction>,
) -> Option<Drag> {
    let pressed = matches!(kind, MouseEventKind::Down(_));
    let released = matches!(kind, MouseEventKind::Up(MouseButton::Left));
    match action {
        // The one press that starts one, level with the rows by definition.
        Some(MouseAction::StartSelection(_)) => Some(Drag::default()),
        // Back inside the card: there is a cell under the pointer again, so the
        // drag extends off the event and this stops asking for a scroll.
        Some(MouseAction::ExtendSelection(_)) => held.is_some().then(Drag::default),
        Some(MouseAction::ExtendPastEdge(reach)) => {
            held.is_some().then_some(Drag { past: Some(reach) })
        }
        // The button coming up, inside the card or past its edge, is the end of
        // the gesture either way.
        Some(MouseAction::EndSelection(_) | MouseAction::EndPastEdge(_)) => None,
        // And so is a release the panel answered nothing for. A card put up
        // mid-drag leaves no cell of the conversation under the pointer and no
        // edge of it to be past, so that release reads as nothing at all — and a
        // drag left standing under a button that is up would hold the
        // conversation still long after the hand came off it.
        _ if released => None,
        // A press on the tree, the composer, the panel's header, one of the
        // other two cards, the footer, the border, off the screen: whatever it
        // begins, it is not a drag over the conversation's text.
        _ if pressed => None,
        // The wheel and every other button, none of which lets go of the left
        // one.
        _ => held,
    }
}

// The wheel drives whichever pane the pointer is over, focus notwithstanding,
// and every part of a pane's inside answers for it — headers included, because
// a notch that did nothing on the one line naming the tree reads as a wheel
// that sticks. The composer answers nothing: it scrolls itself as somebody
// types, and scrolling the account because the pointer was resting on the field
// would move the half of the screen the reader is not pointing at.
fn wheel(hit: Hit, tree: MouseAction, panel: MouseAction) -> Option<MouseAction> {
    match hit {
        Hit::TreeHeader | Hit::TreeRow { .. } | Hit::TreeBelowRows => Some(tree),
        Hit::PanelHeader | Hit::PanelLine { .. } => Some(panel),
        Hit::Composer | Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

fn click(hit: Hit, app: &App) -> Option<MouseAction> {
    match hit {
        Hit::TreeRow { offset } => {
            // The hit counts from the top of the tree's window, so only the
            // app's scroll offset turns it into a row. The window can be taller
            // than the tree in it: an index past the last row is a click on the
            // blank part of a half-full pane, which is a click in the pane and
            // no more. A second click on the row already selected is the reader
            // asking for something other than the selection they have, and what
            // a file tree does then is open or close it — through the very
            // method space goes through, so a row with nothing under it does
            // nothing at all.
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
        // Over a conversation the press anchors a selection, and that action
        // takes the keys too: everywhere else in the panel — its header, either
        // of the other two cards, a thread card with nothing on it yet — there is
        // no text to anchor in, so the press is the plain focus it has always
        // been.
        Hit::PanelHeader | Hit::PanelLine { .. } => Some(cell_under(hit, app).map_or(
            MouseAction::Focus(Focus::Panel),
            MouseAction::StartSelection,
        )),
        // The composer is hit-tested only when it is on screen, so a press on it
        // is somebody pointing at the field they mean to type in.
        Hit::Composer => Some(MouseAction::Focus(Focus::Composer)),
        Hit::Footer | Hit::Border | Hit::Offscreen => None,
    }
}

// The cell of the conversation a pointer event landed on, or `None` for an event
// that landed on no conversation at all: somewhere other than a line of the
// panel, another card showing, or a thread card with nothing recorded on it yet
// — there is no text under the pointer in any of the three, and a gesture over
// them has to go on meaning exactly what it meant before this one existed.
//
// The scroll and width are the panel's own rather than the frame's, for the
// reason the highlight reads them there too: they are the numbers the rows under
// the pointer were wrapped and windowed by, and a cell measured against a second
// window names a character the reader is not pointing at.
fn cell_under(hit: Hit, app: &App) -> Option<Cell> {
    let Hit::PanelLine { offset, column } = hit else {
        return None;
    };
    if !selectable(app) {
        return None;
    }

    let panel = app.panel();
    Some(Cell {
        column: usize::from(column),
        row: usize::from(offset),
        scroll: panel.scroll_offset(),
        width: panel.width(),
    })
}

// Which edge of the conversation's rows a pointer event is past, or `None` for
// one that is still level with them.
//
// Gated on the very question `cell_under` is gated on, because it is the same
// refusal: the account and the document are read past, not copied out of, and a
// pointer dragged off the bottom of either has to go on meaning what it meant
// before a drag off the thread card meant anything.
fn past_rows(
    mouse: MouseEvent,
    size: Size,
    app: &App,
    composer: Option<&Composer>,
) -> Option<Reach> {
    if !selectable(app) {
        return None;
    }

    let header = app.run_header();
    match panel_reach(mouse.column, mouse.row, size, composer, header.as_ref()) {
        Reach::Inside { .. } => None,
        past => Some(past),
    }
}

fn selectable(app: &App) -> bool {
    let panel = app.panel();
    panel.showing_thread() && panel.has_thread()
}

#[cfg(test)]
#[path = "tests/input.rs"]
mod tests;
