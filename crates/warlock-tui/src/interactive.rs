use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Size;
use warlock_engine::Manifest;

use crate::app::{App, Focus, Working};
use crate::chatting::Chat;
use crate::claude::{ChatAgent, Converses};
use crate::clipboard::{self, Clip, Clipboard};
use crate::composer::{Composed, Composer, paste_for};
use crate::confirm::QuitConfirm;
use crate::confirm::Reviewed;
use crate::cutting::Cutter;
use crate::editing::{NO_EDITOR, edit_press, editor, run_editor};
use crate::error::{Error, one_line};
use crate::git::{Forge, Gh, Git, Repository};
use crate::help::HelpSheet;
use crate::input::{
    Action, Drag, MouseAction, Pressed, Scroll, drag_after, mouse_action, press_for,
};
use crate::linear::{Opener as LinearOpener, Opens};
use crate::modal::{Modal, Modals};
use crate::planned::edited_drafts;
use crate::prompt::{KeyEdited, KeyPrompt, RecordPrompt, ScopePrompt};
use crate::puller::{Claudes, Puller, Raises};
use crate::pushing::Pushes;
use crate::resume::resume_press;
use crate::scoping::{Windows, key_edit, record_edit, scope_edit, scope_press};
use crate::screen::Screen;
use crate::selection::{Cell, Position, copied_text, position_at};
use crate::session::{Scope, Watched, load_app, reload, start_watching};
use crate::settings::Said;
use crate::submission::{ToPush, Wanted};
use crate::ui::{Reach, Shape, draw, help_most_scroll, panel_width};
use crate::viewing::view_press;

/// How long the loop waits for a keystroke before going round anyway.
///
/// The ceiling on how stale the footer's clock and the thread's clocks can get,
/// and the reason an idle warlock redraws ten times a second rather than
/// sleeping: the worker threads have no way to wake this one.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long the footer keeps what it was told.
///
/// The footer says what the last thing that happened was, and nothing on it says
/// when: `copied 104 characters` an hour after the copy reads as a copy that
/// just happened. So a message is dropped once it is older than this, and the
/// footer says nothing rather than something out of its time. The highlight a
/// copy leaves on the conversation is what stays.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(10);

/// The interactive session as the event loop sees it: the rounds it drives,
/// with every seam but the screen already chosen.
pub struct Interactive<S: Screen>(Session<Live<S>>);

impl<S: Screen> fmt::Debug for Interactive<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Interactive")
            .finish_non_exhaustive()
    }
}

impl<S: Screen> Interactive<S> {
    /// The order is the point. The manifest, the tree and the watch are all set
    /// up before `enter` is called, so anything that can fail says so on the
    /// ordinary screen; from `enter` onwards the alternate screen is up and the
    /// only way back is dropping the screen it returned.
    pub fn open(enter: impl FnOnce() -> io::Result<S>) -> Result<Self, Error> {
        let (mut app, scope, tree, manifest) = load_app()?;
        // Asked for once, over the tree the load just produced, and kept for as
        // long as warlock runs — dropping it stops the watch. Whether it was
        // granted is a fact for the footer and nothing more, which is why this is
        // not a `?` and why the line about it is put up in there rather than here:
        // warlock with no live updates is warlock as it was. See [`start_watching`].
        let watched = start_watching(&mut app, &scope, &tree);

        // The terminal, taken last, so everything above that can fail says so on
        // the ordinary screen. From here on the alternate screen is up, and
        // dropping the screen is what puts it back — with the session, and on a
        // panic through the hook installed in `main`.
        let parts: Parts<Live<S>> = Parts {
            screen: enter()?,
            // Opened here and nowhere else, and dropped with the session: a
            // handle that does not outlive the copies made through it loses the
            // text it put on an X11 selection. See `mod@clipboard`.
            clipboard: Clipboard::open(),
            // Built once, and cheap to build: an agent is a command line and a
            // timeout, so no `claude` exists until a key asks for a turn.
            chat: Chat::new(scope.repo_root.clone()),
            // Built once, and cheaper still: the seam is a unit value and no
            // socket exists until a confirmed dialog asks for one. The home under which the sigils, the binding and the
            // key store sit is read here as well, once for the session.
            pushes: Pushes::new(),
            // The same two facts, read a second time rather than shared with the
            // value above: a cut opens its own client on its own worker, and a
            // `Cutter` that borrowed a `Pushes`'s home would tie the two together
            // for nothing but the four bytes it saves.
            cutter: Cutter::new(),
            // The same two facts a third time, and `git` and `gh` in this checkout:
            // nothing is run and no socket is opened until a `/pull` is confirmed.
            puller: Puller::new(&scope.repo_root),
        };
        Ok(Self(Session::new(app, scope, manifest, watched, parts)))
    }

    pub fn size(&self) -> io::Result<Size> {
        self.0.size()
    }

    pub fn draw(&mut self, size: Size) -> io::Result<()> {
        self.0.draw(size)
    }

    pub fn press(&mut self, key: KeyEvent, now: Instant) -> Result<bool, Error> {
        self.0.press(key, now)
    }

    pub fn point(&mut self, mouse: MouseEvent, size: Size, now: Instant) {
        self.0.point(mouse, size, now);
    }

    pub fn paste(&mut self, text: &str) {
        self.0.paste(text);
    }

    pub fn drag_scroll(&mut self) {
        self.0.drag_scroll();
    }

    pub fn keep_up(&mut self) {
        self.0.keep_up();
    }
}

// Every window the session can have up, gathered in one place so that the keys,
// the pointer and the frame are all asked about the same `Modals` and take the
// one `current` picks. A free function rather than a method for `draw`'s sake:
// it borrows these fields while the screen is borrowed mutably. One argument
// per owner of a window, which is past clippy's count; bundling two of them into
// a pair would be a type that exists only for the lint.
#[allow(clippy::too_many_arguments)]
fn modals<'a, C, O, A, R, F, M>(
    quit: QuitConfirm,
    help: HelpSheet,
    scope: &'a ScopePrompt,
    record: &'a RecordPrompt,
    key: &'a KeyPrompt,
    pushes: &'a Pushes<O>,
    cutter: &'a Cutter<O, A>,
    puller: &'a Puller<O, R, F, M>,
    chat: &'a Chat<C>,
) -> Modals<'a>
where
    C: Converses,
    O: Opens,
    A: Converses,
    R: Repository + Clone + Send + 'static,
    F: Forge + Clone + Send + 'static,
    M: Raises + Clone + Send + 'static,
{
    Modals {
        quit,
        help,
        push: &pushes.window().confirm,
        cut: cutter.confirm(),
        pull: puller.confirm(),
        // The two windows the run itself puts up, read off it rather than
        // copied: they are states of the cut in flight, and a session holding a
        // copy of either would be a second answer to what a slice is waiting for.
        review: cutter.reviewing(),
        carry: cutter.carrying(),
        write_brief: chat.write_offer(),
        scope,
        record,
        key,
        write: chat.write_prompt(),
    }
}

/// The impure things a session is built over — the screen, the conversation's
/// model, the clipboard, the Linear a push files to, the model a cut drafts a
/// slice with and what a pull runs — named once, so a test can press keys at a whole
/// session with no terminal attached, no `claude` installed, no display and no
/// socket. `warlock` itself only ever uses [`Live`].
///
/// Associated types rather than parameters on [`Session`], so another seam is a
/// line here and in each implementation rather than a parameter threaded
/// through every signature that names a session.
///
/// The two models are two types because they are two conversations: the panel's
/// has heard the reader's talk and answers in prose, and a slice's has heard
/// none of it and answers in JSON. A test drives each with the stand-in its own
/// path needs.
trait Seams {
    type Screen: Screen;
    type Talk: Converses;
    type Clip: Clip;
    type Board: Opens;
    type Draft: Converses;
    type Repo: Repository + Clone + Send + 'static;
    type Forge: Forge + Clone + Send + 'static;
    type Raise: Raises + Clone + Send + 'static;
}

struct Live<S>(PhantomData<S>);

impl<S: Screen> Seams for Live<S> {
    type Screen = S;
    type Talk = ChatAgent;
    type Clip = Clipboard;
    type Board = LinearOpener;
    type Draft = ChatAgent;
    type Repo = Git;
    type Forge = Gh;
    type Raise = Claudes;
}

/// What a caller hands [`Session::new`]: the one value of each seam. Everything
/// else a session holds starts closed, empty or unheld, and is the session's own
/// to set up, so a field added there reaches no caller.
struct Parts<K: Seams> {
    screen: K::Screen,
    clipboard: K::Clip,
    chat: Chat<K::Talk>,
    pushes: Pushes<K::Board>,
    cutter: Cutter<K::Board, K::Draft>,
    puller: Puller<K::Board, K::Repo, K::Forge, K::Raise>,
}

/// Everything one interactive session holds, and the seam the tests drive.
struct Session<K: Seams> {
    app: App,
    screen: K::Screen,
    scope: Scope,
    manifest: Manifest,
    chat: Chat<K::Talk>,
    /// The session's one clipboard handle, opened in [`Interactive::open`] and
    /// held for as long as the session is, because a copy does not outlive the
    /// handle that made it.
    clipboard: K::Clip,
    confirm: QuitConfirm,
    help: HelpSheet,
    /// Where a `/push` gets its client from, the home it resolves the board
    /// under, the window asking about it and the one request it may have in
    /// flight — which is its own say-no to a second. See [`Pushes`].
    pushes: Pushes<K::Board>,
    /// The same for a `/draft`, and in the other order: a cut has nothing to
    /// put a window up about until the board has answered, so the request comes
    /// first and the reading is what it has to say. Its own [`Option`] is its
    /// own say-no to a second cut. See [`Cutter`].
    cutter: Cutter<K::Board, K::Draft>,
    /// The same for a `/pull`, whose run is a say-no to much more than a second
    /// pull: while it is underway a session is editing this working tree, so
    /// `s`, `/draft`, `/resume` and the composer are all refused with the line
    /// [`Puller::in_flight`] words. See [`Puller`].
    puller: Puller<K::Board, K::Repo, K::Forge, K::Raise>,
    prompt: ScopePrompt,
    /// The second window the `s` key puts up, over a scope name no `[[scope]]`
    /// record claims. Never up at the same time as [`Session::prompt`]: one goes
    /// down as the other comes up, in the one
    /// [`scoping::Windows`](crate::scoping::Windows) a submit hands back.
    record: RecordPrompt,
    /// The window that binds one of this machine's stored keys to a scope, and
    /// the session's one copy of it: the record form's submit puts it up over
    /// the scope it just recorded — a created scope nothing on this machine
    /// could push into is the thing it is there to prevent — and a `/push` of a
    /// scope bound to nothing puts it up in front of the push, which waits in
    /// [`Pushes`] for the answer. Never up at the same time as either window
    /// above, for [`Session::record`]'s reason and out of the same value. See
    /// [`KeyPrompt`].
    key: KeyPrompt,
    /// The left button held down over the conversation, if it is: the one piece
    /// of a gesture that outlives the event carrying it, because the rounds
    /// between one drag event and the next are what [`Session::drag_scroll`]
    /// runs on. `None` is no button held, or one held after a press that landed
    /// anywhere else.
    drag: Option<Drag>,
    /// Which file is on the document card, which the app is never told:
    /// `App::show_document` takes lines and never a path. The edit key is what
    /// asks, so that it re-reads the card only when the file it just handed to
    /// an editor is the one on it. A press that read nothing leaves this as it
    /// was rather than clearing it.
    document: Option<PathBuf>,
    /// Which saying is on the footer and the instant it landed, for
    /// [`Session::forget_stale_message`]. Held here rather than on the app
    /// because the app is told every instant it works with and keeps no clock of
    /// its own, and a message's age is the one thing about it that moves without
    /// anybody touching it.
    said: Option<(u64, Instant)>,
    mouse_captured: bool,
    watched: Watched,
}

impl<K: Seams> Session<K> {
    fn new(app: App, scope: Scope, manifest: Manifest, watched: Watched, parts: Parts<K>) -> Self {
        let Parts {
            screen,
            clipboard,
            chat,
            pushes,
            cutter,
            puller,
        } = parts;
        Self {
            app,
            screen,
            scope,
            manifest,
            chat,
            clipboard,
            confirm: QuitConfirm::default(),
            help: HelpSheet::default(),
            pushes,
            cutter,
            puller,
            prompt: ScopePrompt::default(),
            record: RecordPrompt::default(),
            key: KeyPrompt::default(),
            drag: None,
            document: None,
            said: None,
            // The terminal has just been asked to report its pointer, and `m` is
            // the one thing that changes the answer.
            mouse_captured: true,
            watched,
        }
    }

    fn size(&self) -> io::Result<Size> {
        self.screen.size()
    }

    fn modal(&self) -> Option<Modal<'_>> {
        modals(
            self.confirm,
            self.help,
            &self.prompt,
            &self.record,
            &self.key,
            &self.pushes,
            &self.cutter,
            &self.puller,
            &self.chat,
        )
        .current()
    }

    fn reread_config(&mut self) {
        if let Some(held) = self.scope.settings.changed() {
            self.scope.chrome.set_held(held);
        }
    }

    /// Everything that has to be told the frame's dimensions before anything
    /// reads them. The loop draws and *then* waits for a key, which is what
    /// lets the composer's row-wise keys and the panel's wrapping both work off
    /// a width nothing has measured yet at startup.
    fn draw(&mut self, size: Size) -> io::Result<()> {
        self.reread_config();
        self.app.set_mouse_captured(self.mouse_captured);
        self.app.set_working(if self.puller.busy() {
            Some(Working::Pull)
        } else if self.cutter.running() || self.cutter.confirm().cutting().is_some() {
            Some(Working::Draft)
        } else if self.pushes.sending() || self.pushes.window().confirm.filing().is_some() {
            Some(Working::Push)
        } else {
            None
        });
        // One reading of the panel's inside width, told to both the things that
        // are about to use it: the app wraps the document it holds at it, and
        // the composer is drawn in the same column, so it is the width the
        // field's own rows are folded at and therefore the width its row-wise
        // keys move over. Told here, before the key that reads it, because the
        // loop draws and then waits.
        let width = panel_width(size);
        self.chat.set_composer_width(width);
        if self.help.scroll().is_some() {
            self.help = self.help.clamped(help_most_scroll(size));
        }
        // And the other thing the field is told once a round: which slice of a
        // cut, if any, the next submission answers for. Told here rather than
        // at the two edges of a question, so a field cannot be left labelled for
        // a question that is over — see [`Cutter::answering`].
        self.chat.set_composer_answering(self.cutter.answering());
        let cut = Shape::of(
            size,
            &self.app,
            Some(self.chat.composer()),
            self.scope.chrome.sigils(),
        )
        .cut();
        self.app.set_viewport_height(cut.tree_height());
        let panel = self.app.panel_mut();
        panel.set_height(cut.panel_height());
        panel.set_width(width);

        let modal = modals(
            self.confirm,
            self.help,
            &self.prompt,
            &self.record,
            &self.key,
            &self.pushes,
            &self.cutter,
            &self.puller,
            &self.chat,
        )
        .current();
        let (app, chrome, field) = (&self.app, &self.scope.chrome, self.chat.composer());
        self.screen.draw(|frame| {
            // The terminal can be resized between `size` being read and this
            // frame being handed out — a window manager settling a new window
            // does it at startup — and ratatui's frame is the newer size. The
            // frame is cut again at that size rather than drawn with a cut
            // that no longer fits it; the heights told to the app above stay a
            // round behind, and the next round, a poll interval away, measures
            // again.
            let cut = if frame.area() == cut.screen() {
                cut
            } else {
                Shape::of(frame.area().as_size(), app, Some(field), chrome.sigils()).cut()
            };
            draw(frame, &cut, app, chrome, Instant::now(), modal, Some(field));
        })
    }

    /// `now` is the instant the event arrived, for the reason a keystroke's is
    /// read once by the caller: a clock is part of a row's prefix, so the cell
    /// under the pointer is worked out against the same instant the rest of this
    /// round is.
    ///
    /// The copy is done here rather than in [`apply_mouse`] because the
    /// clipboard is the session's, held open for as long as it runs — see
    /// [`Clipboard`].
    fn point(&mut self, mouse: MouseEvent, size: Size, now: Instant) {
        // Read here rather than inside `apply_mouse` because every window and
        // the draft the frame was drawn with are the session's: what the event
        // means is decided against them, and what it then does is the app's
        // alone.
        let shape = Shape::of(
            size,
            &self.app,
            Some(self.chat.composer()),
            self.scope.chrome.sigils(),
        );
        let action = mouse_action(mouse, shape, &self.app, self.modal());
        // Before the action is acted on, because this is bookkeeping about the
        // gesture rather than part of it: what the app does with a press or a
        // drag is the same whether or not the button stays down afterwards.
        self.drag = drag_after(self.drag, mouse.kind, action);
        // Derived from the drag rather than kept beside it: `drag_after` already
        // says exactly "the left button is still down after a press on a line of
        // the conversation", which is the whole of what suspends following. Said
        // on every pointer event, so the release that ends the drag — wherever
        // the pointer was when the button came up — is the one that lets the card
        // go back to the newest line, and a press that landed anywhere else never
        // holds it at all.
        self.app.panel_mut().hold_thread(self.drag.is_some());
        if let Some(text) = apply_mouse(&mut self.app, action, now) {
            self.copy(&text);
        }
    }

    /// The conversation scrolled under a drag held past its edge, once per
    /// round, with the highlight taken along.
    ///
    /// Called off the loop's tick rather than off an event because a pointer
    /// held still past the card sends nothing: the last drag event said which
    /// edge it went past and by how far, and every round after it reads that
    /// again. Does nothing at all on a round with no button held, one held after
    /// a press that landed elsewhere, or a pointer still level with the rows.
    fn drag_scroll(&mut self) {
        let Some(past) = self.drag.and_then(|drag| drag.past) else {
            return;
        };
        // A key pressed with the button still down can put another card in the
        // panel, and the drag outlives it: without this the held pointer would
        // go on scrolling, now through the document, which a drag has never
        // been able to move.
        if !self.app.panel().showing_thread() {
            return;
        }
        let (rows, column, downwards) = match past {
            Reach::Above { rows, column } => (rows, column, false),
            Reach::Below { rows, column } => (rows, column, true),
            // Never recorded: a pointer level with the rows has a cell under it,
            // and `mouse_action` hands that over as the cell rather than as a
            // reach past anything.
            Reach::Inside { .. } => return,
        };

        let before = self.app.panel().scroll_offset();
        let by = rows_per_tick(rows);
        if downwards {
            self.app.scroll_panel_down(by);
        } else {
            self.app.scroll_panel_up(by);
        }

        let (scroll, height, width) = {
            let panel = self.app.panel();
            (panel.scroll_offset(), panel.height(), panel.width())
        };
        if scroll == before {
            // Either end of the thread, where the panel clamped and nothing new
            // came into view. The highlight stops growing with it: there is no
            // newly revealed row for the pointer to be pulling towards, and
            // extending anyway would drag the far end across the same edge row
            // once a tick for as long as the button is held.
            return;
        }

        // The row the scroll just uncovered — the bottom one going down, the top
        // one going up — at the column the pointer left the card by. Measured
        // from the panel's own scroll and width, as a cell under the pointer is,
        // so the endpoint maps back through the wrapping this frame was drawn
        // with.
        let cell = Cell {
            column: usize::from(column),
            row: if downwards {
                height.saturating_sub(1)
            } else {
                0
            },
            scroll,
            width,
        };
        extend(&mut self.app, cell, Instant::now());
    }

    /// One keystroke. `Ok(false)` is the session being over and the only thing
    /// that ends the event loop in `main.rs`.
    ///
    /// `now` is the instant the event arrived, read once by the caller: a turn
    /// is clocked from the keystroke that asked for them rather
    /// than from the first thing the model got round to saying.
    ///
    /// Every arm below is one key. Nothing here re-gates what the module it
    /// dispatches to already refuses, which is why most arms have no error case
    /// — a refusal is a line on the footer and the loop goes round again.
    // Over the pedantic line count on purpose: splitting by key group would put
    // half the keyboard behind a name somebody has to guess at, where here every
    // arm is one key a reader can find.
    #[allow(clippy::too_many_lines)]
    fn press(&mut self, key: KeyEvent, now: Instant) -> Result<bool, Error> {
        let typing = self.typing();
        let pulling = self.puller.in_flight();
        let pressed = press_for(key, self.modal(), typing, self.chat.answering());

        match pressed {
            Pressed::Leave => return Ok(false),
            Pressed::Confirm(next) => self.confirm = next,
            Pressed::Help(next) => self.help = next,
            Pressed::Push(answered) => self.pushes.answer(&mut self.app, answered, now),
            Pressed::Cut(answered) => self.cutter.confirmed(&mut self.app, answered, now),
            // Here rather than in the cut because it hands the screen to
            // `$EDITOR` the way `e` on a file does.
            Pressed::Review(Reviewed::Edit) => {
                if let Some((slice, drafts)) = self.cutter.editing() {
                    let edited = match editor() {
                        None => Err(NO_EDITOR.to_owned()),
                        Some(editor) => self.screen.suspended(self.mouse_captured, || {
                            edited_drafts(&slice, &drafts, |path| run_editor(&editor, path))
                        })?,
                    };
                    self.cutter.edited(&mut self.app, edited, now);
                }
            }
            Pressed::Review(answered) => self.cutter.reviewed(&mut self.app, answered, now),
            Pressed::Scroll(scroll) => {
                let page = self.app.panel().page();
                match scroll {
                    Scroll::LineUp => self.app.scroll_panel_up(1),
                    Scroll::LineDown => self.app.scroll_panel_down(1),
                    Scroll::PageUp => self.app.scroll_panel_up(page),
                    Scroll::PageDown => self.app.scroll_panel_down(page),
                }
            }
            Pressed::Carry(answered) => self.cutter.carried(&mut self.app, answered, now),
            Pressed::WriteBrief(answered) => self.chat.offered(&mut self.app, answered, now),
            Pressed::Pull(answered) => self.puller.answered(&mut self.app, answered, now),
            // The turn is deliberately *not* taken down here: the worker still has
            // one thing to say, and it says it at the bottom of the loop like any
            // other ending. That is
            // what puts the cancelled line under whatever work had already arrived
            // and gives the field the keyboard back — a turn forgotten here would
            // leave the composer muted for the rest of the session.
            Pressed::CancelTurn => self.chat.stop(),
            Pressed::Act(Action::ToggleFocus) => self.app.toggle_focus(),
            Pressed::Act(Action::SelectPrevious) => self.app.select_previous(),
            Pressed::Act(Action::SelectNext) => self.app.select_next(),
            Pressed::Act(Action::SelectPageUp) => self.app.select_page_up(),
            Pressed::Act(Action::SelectPageDown) => self.app.select_page_down(),
            Pressed::Act(Action::SelectFirst) => self.app.select_first(),
            Pressed::Act(Action::SelectLast) => self.app.select_last(),
            Pressed::Act(Action::ToggleCollapsed) => self.app.toggle_collapsed(),
            Pressed::Act(Action::ToggleScopedOnly) => self.app.toggle_scoped_only(),
            Pressed::Act(Action::ToggleFiles) => self.app.toggle_files(),
            // A press that read nothing leaves the card holding what it held, so
            // a `None` keeps the file remembered before rather than clearing it:
            // the edit key re-reads the card only when the file it edited is
            // this one.
            Pressed::Act(Action::ViewFile) => {
                if let Some(read) = view_press(&mut self.app) {
                    self.document = Some(read);
                }
            }
            // `mouse_captured` is handed over because it is this thread's only
            // record of what `m` last did: resuming without it would switch
            // reporting back on behind a reader who turned it off.
            Pressed::Act(Action::EditFile) => {
                edit_press(
                    &mut self.app,
                    &mut self.screen,
                    &self.scope,
                    &mut self.manifest,
                    self.document.as_deref(),
                    self.mouse_captured,
                )?;
            }
            Pressed::Act(Action::SwapCard) => self.app.swap_card(),
            // The sequence is written first and the flag moved only if it went
            // out, so what this thread believes about the terminal is what it
            // last successfully told it; a write that fails takes the whole loop
            // down through the guard, which turns capture off on the way past
            // whatever state it was left in.
            Pressed::Act(Action::ToggleMouseCapture) => {
                self.screen.report_mouse(!self.mouse_captured)?;
                self.mouse_captured = !self.mouse_captured;
            }
            Pressed::Act(Action::OpenScope) => {
                self.prompt = scope_press(
                    &mut self.app,
                    &self.manifest,
                    &self.scope.repo_root,
                    self.scope.chrome.sigils(),
                    pulling.as_deref(),
                    now,
                );
                // Both of the later windows put down rather than left as they
                // were, though `press_for` only lets this key through with all
                // of them already down: a record window that outlived the press
                // it belonged to would be three values filed under a scope name
                // from an earlier one, and a key window that did would bind a
                // key to it.
                self.record = RecordPrompt::Closed;
                self.key = KeyPrompt::Closed;
            }
            Pressed::Scope(edited) => {
                let before = self.manifest.clone();
                let windows = scope_edit(
                    &mut self.app,
                    &mut self.manifest,
                    &self.scope.repo_root,
                    &self.prompt,
                    edited,
                );
                self.apply_windows(windows);
                self.recolour(&before, now);
            }
            // The home is handed over from the settings this session was loaded
            // under, as the key arm below hands it over: a submit that saves a
            // record goes on to ask which key the scope is pushed with, and the
            // store it lists is this machine's. A session with no home saves the
            // record and asks nothing.
            Pressed::Record(edited) => {
                let before = self.manifest.clone();
                let windows = record_edit(
                    &mut self.app,
                    &mut self.manifest,
                    &self.scope.repo_root,
                    self.scope.settings.home(),
                    &self.record,
                    edited,
                );
                self.apply_windows(windows);
                self.recolour(&before, now);
            }
            // No `recolour` and no reload beside it: a binding is a fact about
            // this machine written into this checkout's config, so no row of
            // the tree changes colour and the manifest is not opened at all.
            //
            // The one caller of the pair in [`mod@crate::scoping`] that writes a
            // binding, reached by the keys pressed at the window a record
            // submit put up. The home and the root are the session's own,
            // resolved once in `load_app` — and the file a binding lands in is
            // the very file those settings read the sigils out of.
            Pressed::Key(edited) => {
                // Which of the window's three roads this press is on, read
                // before it is answered because answering takes it. It is also
                // what happens to a `/push` parked behind the window: a submit
                // has written the binding that push was waiting on, an Esc puts
                // it down, and a move of the highlight leaves it waiting.
                let (wrote, abandoned) = (
                    matches!(edited, KeyEdited::Submit),
                    matches!(edited, KeyEdited::Close),
                );
                self.key = match self.scope.settings.home() {
                    Some(home) => key_edit(
                        &mut self.app,
                        home,
                        &self.scope.repo_root,
                        &self.key,
                        edited,
                    ),
                    // A machine with no home has no config to write a binding
                    // into and no store for this window to have listed, so a
                    // window could not have come up over one: the window goes
                    // down and nothing is written.
                    None => KeyPrompt::Closed,
                };
                // The push the window was opened in front of, if it was opened
                // in front of one: on to `prepare` over the scope and the brief
                // it parked, which is the road a `/push` with a key bound takes
                // on the round it is typed. Nothing at all happens here when
                // the window came up over a scope just recorded by the `s` key,
                // which parks no push.
                if wrote {
                    self.pushes
                        .bound(&mut self.app, &self.manifest, &self.scope.repo_root, now);
                } else if abandoned {
                    self.pushes.put_down();
                }
            }
            Pressed::Write(edited) => {
                self.chat.write(&mut self.app, edited, now);
            }
            Pressed::Compose(outcome) => self.composed(outcome, now),
            // `press_for` turns quit and help into their own answers, so neither
            // arrives as an action.
            Pressed::Nothing | Pressed::Act(Action::Quit | Action::Help) => {}
        }

        Ok(true)
    }

    // Destructured rather than read field by field, so a fourth window added to
    // [`Windows`] fails to compile here instead of being silently left down:
    // every one of them is put up or down together, which is what keeps a submit
    // from being applied by halves.
    fn apply_windows(&mut self, windows: Windows) {
        let Windows { scope, record, key } = windows;
        self.prompt = scope;
        self.record = record;
        self.key = key;
    }

    /// A keystroke at the foot of the panel's column, and the one place it is
    /// decided which conversation a submitted draft belongs to.
    ///
    /// While a slice of a cut is waiting on an answer, an Enter is that answer:
    /// the text is taken out of the field and put to the session that asked, and
    /// no turn of the chat starts, no command in it is recognised and nothing
    /// about the register changes. Everything else — a character more or less,
    /// the keyboard handed back — is the conversation's as it always was, so the
    /// draft being edited is the same value whichever of the two will get it.
    ///
    /// The question is asked of the cut rather than answered by a flag kept
    /// here: a second record of "somebody is being asked something" would be one
    /// more thing to clear on every way a question can end, and the cut already
    /// knows (see [`Cutter::relaying`]).
    ///
    /// What a chat submission can hand back is a [`Wanted`]: a command about a
    /// board, a queue or this machine's settings, none of which the conversation
    /// holds. A `/push` is answered on this thread — no socket is opened by any of it —
    /// and what comes back is the window the reader is now looking at. A `/draft`
    /// has nothing to put up until a project has been read back, so it goes
    /// straight onto a worker and what comes back arrives at the bottom of a
    /// later round. See [`Pushes::press`] and [`Cutter::press`].
    fn composed(&mut self, outcome: Composed, now: Instant) {
        if self.cutter.relaying() && matches!(outcome, Composed::Submit) {
            // Taken whole and unread: what the slice asked is not warlock's
            // question, so what is sent back is not warlock's to word.
            let answer = self.chat.taken();
            self.cutter.answered(&mut self.app, &answer, now);
            return;
        }
        // A run of a ticket underway takes the Enter, and nothing else: the draft
        // stays in the field for after, and every editing key still works on it.
        // There is nothing to say to a model while another one is editing this
        // tree, and a command typed here — a `/draft`, a `/resume` — would be a
        // second writer of it. Asked before the conversation sees the draft, so
        // no turn is opened and no command in it is recognised.
        if self.puller.pulling() && matches!(outcome, Composed::Submit) {
            if let Some(line) = self.puller.in_flight() {
                self.app.panel_mut().note(locked(&line), now);
            }
            return;
        }

        let pulling = self.puller.in_flight();
        let Some(wanted) = self.chat.compose(&mut self.app, outcome, now) else {
            return;
        };
        let root = &self.scope.repo_root;
        match wanted {
            // The key window, over a scope this checkout binds to nothing:
            // `prepare` is what resolves the key, so the binding is asked for
            // ahead of it and the push waits in the flow for the answer. Every
            // other road hands back `KeyPrompt::Closed`, so this is the one copy
            // of that window either way — the flow holds none.
            Wanted::Push(ToPush { scope, path }) => {
                self.key =
                    self.pushes
                        .press(&mut self.app, &self.manifest, root, &scope, &path, now);
            }
            Wanted::Draft(drafting) => {
                self.cutter.press(
                    &mut self.app,
                    &self.manifest,
                    root,
                    &drafting,
                    pulling.as_deref(),
                    now,
                );
            }
            Wanted::Pull(Some(taking)) => {
                self.puller
                    .press(&mut self.app, &self.manifest, root, &taking, now);
            }
            // Answered from a file under the home and nothing else: no board is
            // opened, no `git` runs and no session is raised.
            Wanted::Pull(None) => {
                let said = self.scope.settings.pull_line();
                self.say(said, now);
            }
            Wanted::Branch(branching) => {
                let said = self
                    .scope
                    .settings
                    .branch(&branching, self.scope.chrome.remote_head());
                self.settled(said, now);
            }
            Wanted::Sigil(sigiling) => {
                let said = self.scope.settings.sigil(&sigiling);
                self.settled(said, now);
            }
            Wanted::Resume(ticket) => {
                resume_press(
                    &mut self.app,
                    &mut self.chat,
                    self.puller.home(),
                    root,
                    &ticket,
                    pulling.as_deref(),
                    now,
                );
            }
        }
    }

    // After a write the chrome is read again from the file rather than left to
    // the next round's stat, so the box and the boundary checks move with the
    // line even where the file's clock is too coarse to see the write. A pull
    // underway has already been refused above with every other submit, which
    // matters: its base was read when it started.
    fn settled(&mut self, said: Result<Said, Error>, now: Instant) {
        if matches!(said, Ok(Said::Wrote(_))) {
            self.scope.chrome.set_held(self.scope.settings.snapshot());
        }
        self.say(said.map(|said| said.line().to_owned()), now);
    }

    // The composer is offered on exactly the condition that lights its border,
    // which is the keyboard being pointed at it: with the keys anywhere else this
    // is `None`, there is no draft to type into, and every letter is the command
    // it has always been.
    fn typing(&self) -> Option<&Composer> {
        (self.app.focus() == Focus::Composer).then(|| self.chat.composer())
    }

    fn say(&mut self, said: Result<String, Error>, now: Instant) {
        let line = said.unwrap_or_else(one_line);
        self.app.panel_mut().note(line, now);
    }

    /// A block the terminal handed over whole. Nothing is returned and nothing
    /// can be: a paste never ends the session and never starts a turn, whatever
    /// newlines are in it.
    fn paste(&mut self, text: &str) {
        let Some(typing) = self.typing() else {
            return;
        };

        let pasted = paste_for(text, typing);
        self.chat.paste(pasted);
    }

    /// Text onto the system clipboard, and a line on the footer either way.
    ///
    /// Reached by the release that ends a drag over the conversation, and by
    /// nothing else: what decides the text is the selection that drag built.
    fn copy(&mut self, text: &str) {
        clipboard::copy(&mut self.clipboard, &mut self.app, text);
    }

    /// Takes `now` rather than reading the clock, so a test can hand it an
    /// instant ten seconds on instead of sitting there for ten seconds.
    ///
    /// The count and not the words: the same sentence said again is a new
    /// saying, and comparing the text would leave a second copy of the same
    /// length wearing out what was left of the first one's ten seconds.
    fn forget_stale_message(&mut self, now: Instant) {
        if self.app.message().is_none() {
            self.said = None;
            return;
        }
        let said = self.app.said();
        match self.said {
            Some((seen, at)) if seen == said => {
                if now.duration_since(at) >= MESSAGE_LIFETIME {
                    self.app.forget_message();
                    self.said = None;
                }
            }
            _ => self.said = Some((said, now)),
        }
    }

    // A saved scope changes which rows are covered, and the rows are only
    // coloured by a load: waiting for the watcher would leave the row grey for a
    // quiet period, and for good with the watcher off. The reload is handed to
    // the watcher as caught up, so the save's own file event costs no second walk.
    fn recolour(&mut self, before: &Manifest, now: Instant) {
        if self.manifest == *before {
            return;
        }
        let tree = reload(&mut self.app, &self.scope, &mut self.manifest);
        self.watched.caught_up(tree.as_ref(), now);
    }

    fn keep_up(&mut self) {
        let now = Instant::now();
        self.forget_stale_message(now);
        self.watched
            .round(&mut self.app, &self.scope, &mut self.manifest, now);
        // And the conversation's own bottom end. Nothing comes back: a ready
        // brief's question and a write turn's path prompt are the
        // conversation's windows, so they are opened in there rather than here
        // out of values this loop would otherwise have to be handed. See
        // [`Chat::keep_up`].
        self.chat.keep_up(&mut self.app, now);
        // And a brief on its way to the board, which says one thing and is over:
        // the project's address or one line about why there is none. Drained like
        // the rest, so the frames keep coming while it is in flight.
        self.pushes.keep_up(&mut self.app, now);
        // And a project being read back off the board, and then cut: what the
        // fetch found, a line about why there is nothing to cut, or whatever the
        // slice being drafted has come to. Nothing here writes, so a cut that
        // never reports has left the board exactly as it was.
        //
        // What it hands back is warlock's attempt at a question a slice asked,
        // shown as a ghost and not written into the field as a draft — see
        // [`Chat::suggest`].
        if let Some(proposal) = self.cutter.keep_up(&mut self.app, now) {
            self.chat.suggest(&proposal);
        }
        // And a ticket being chosen, asked about, or worked: the thread's work
        // lines and milestones, drained so the frames keep coming while a run
        // that takes an hour runs. See [`Puller::keep_up`].
        self.puller.keep_up(&mut self.app, now);
    }
}

// The composer's Enter turned down for a pull, in the sentence every keystroke
// that races one shares, plus the one way to stop it: the panel has no cancel key
// for a run, and quitting is what drops it and kills the session in flight.
fn locked(pulling: &str) -> String {
    format!("{pulling}; nothing was sent, and quitting warlock stops the pull")
}

/// What a pointer event already read off the frame comes to on the app.
///
/// Free rather than a method because none of it reads the terminal, spawns
/// anything or draws: the round is the redraw, which is why a pointer swept
/// across the screen costs nothing.
///
/// `now` is the instant the event arrived, because a clock is part of a row's
/// prefix and a row with a wider prefix breaks somewhere else — the cell under
/// the pointer has to be worked out against the same instant everything else
/// this round is.
///
/// What comes back is text a release asked for, which the caller puts on the
/// clipboard it holds: the one thing a pointer event can want that is not on
/// the app.
fn apply_mouse(app: &mut App, action: Option<MouseAction>, now: Instant) -> Option<String> {
    match action {
        // The wheel over the tree column, whichever pane the keys are pointed
        // at: the selection moves and the window follows it, exactly as it does
        // for a movement key.
        Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
        Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
        // The panel's half of the same wheel. What the follow rule makes of it
        // is the app's business and is not restated here: a window scrolled
        // back stops following the newest line, and one scrolled to the end
        // starts again.
        Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
        Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
        // A click names a row, and a click in a pane also says which pane the
        // keys are about from now on: the reader has just pointed at it, and
        // leaving the keys driving the other pane would send the next `j`
        // somewhere they are not looking.
        Some(MouseAction::SelectRow(index)) => {
            app.set_focus(Focus::Tree);
            app.select_row(index);
        }
        // A click on the row that is already selected, which is space by
        // another road — so it goes through the very method space goes through,
        // and a row with nothing under it collapses nothing here exactly as it
        // would there.
        Some(MouseAction::ToggleCollapsed) => {
            app.set_focus(Focus::Tree);
            app.toggle_collapsed();
        }
        // A click inside a pane with nothing under it: the tree's header, the
        // space below its last row, a line of the panel. Taking the focus is
        // the whole of what it does.
        Some(MouseAction::Focus(focus)) => app.set_focus(focus),
        // A press on the conversation is the focus arm above plus the anchor a
        // drag extends from. Whatever was highlighted comes down first and
        // comes down either way, cell with text under it or not: the reader has
        // just pointed somewhere else, and a highlight left standing is one the
        // next release would copy.
        Some(MouseAction::StartSelection(cell)) => {
            app.set_focus(Focus::Panel);
            app.clear_selection();
            if let Some(at) = position_under(app, cell, now) {
                app.start_selection(at);
            }
        }
        // A drag moves the far end and nothing else. A cell with no text under
        // it leaves the selection where the last one put it rather than
        // collapsing it: a pointer dragged off the end of the card is still
        // that drag.
        Some(MouseAction::ExtendSelection(cell)) => extend(app, cell, now),
        // The release ends the drag where the button came up, and what it hands
        // back is what the selection covers — nothing at all for a press nobody
        // dragged from, since the two ends are one position and `copied_text`
        // of that is empty. An empty copy is not a copy: the clipboard keeps
        // what it had and the footer is left alone, so nothing tells a reader
        // something went that did not.
        //
        // The highlight stays up. It is the only thing on screen saying what
        // went, and taking it down in the same frame as the footer says how much
        // would leave the reader to take warlock's word for it.
        Some(MouseAction::EndSelection(cell)) => {
            extend(app, cell, now);
            return copied(app);
        }
        // The release that ends a drag held past the card's edge. The far end is
        // left where the last cell under the pointer put it — there is no cell
        // out here to move it to — and what the highlight covers is handed back
        // exactly as the arm above hands it back, so a button let go past the
        // edge still copies rather than dropping the gesture on the floor.
        Some(MouseAction::EndPastEdge(_)) => return copied(app),
        // A drag held past that edge does nothing to the app *here*, which is
        // why it sits with the events that mean nothing at all: what it asks for
        // is a scroll a round at a time, and a round is not an event. The reach
        // it carries is kept on the session by [`Session::point`] and read by
        // [`Session::drag_scroll`] for as long as the button is down.
        Some(MouseAction::ExtendPastEdge(_)) | None => {}
    }

    None
}

/// How many rows one tick scrolls the card by, for a pointer `past` rows past
/// its edge.
///
/// The tick is [`POLL_INTERVAL`], ten a second, so a row per tick is ten rows a
/// second: slow enough to stop on the line you meant, which is what the first
/// row past the edge has to be. The ceiling is five, or fifty rows a second —
/// about a screenful a half-second, fast enough to cross a long answer and slow
/// enough to see what is going past. Between them a row of speed per three rows
/// of reach, so the whole range is available inside a few rows of pointer travel
/// rather than needing the reader to drag off the bottom of the terminal.
fn rows_per_tick(past: u16) -> usize {
    const FASTEST: usize = 5;
    const ROWS_PER_STEP: usize = 3;

    usize::from(past).div_ceil(ROWS_PER_STEP).clamp(1, FASTEST)
}

fn extend(app: &mut App, cell: Cell, now: Instant) {
    if let Some(to) = position_under(app, cell, now) {
        app.extend_selection(to);
    }
}

/// Where in the conversation's own text a cell of the card is, which is `None`
/// for a cell past its last row — a place a reader can put the pointer and not
/// a position in anything.
fn position_under(app: &App, cell: Cell, now: Instant) -> Option<Position> {
    position_at(app.panel().thread()?, cell, now)
}

fn copied(app: &App) -> Option<String> {
    let text = selected_text(app);
    (!text.is_empty()).then_some(text)
}

/// What the highlight covers, as the thread's own stored text: no marker, no
/// indent and no break where the panel wrapped it (see [`copied_text`]).
fn selected_text(app: &App) -> String {
    let (Some(selection), Some(thread)) = (app.selection(), app.panel().thread()) else {
        return String::new();
    };

    copied_text(&thread.pieces(), selection)
}

#[cfg(test)]
#[path = "tests/interactive.rs"]
mod tests;
