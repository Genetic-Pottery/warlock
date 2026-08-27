//! Terminal front end for warlock.
//!
//! This binary is the thin, impure shell around the pure parts in
//! `warlock_tui`: it owns the terminal's lifecycle, the working directory it
//! was invoked from, and the event loop, and nothing else. It asks the engine
//! to load the tree for that directory and knows nothing about how one is
//! built; what a frame looks like is [`warlock_tui::draw`]'s business and how
//! the selection moves is [`App`]'s.
//!
//! This file is the loop itself; each of the loop's concerns lives in a
//! sibling module. What a keystroke or a click means is [`input`]'s, running a
//! pact on a worker thread and applying what it says is [`pacting`]'s, where
//! the tree came from and when it is re-read is [`session`]'s, the terminal's
//! setup and restoration is [`terminal`]'s, and the one-line errors `main`
//! prints are [`error`]'s. The paragraphs below describe how the loop drives
//! all of them, and each module's own doc says why it is shaped as it is.
//!
//! The one rule this shell exists to keep is that the terminal is restored on
//! every way out: a normal quit, an error returned up to `main`, and a panic.
//! Raw mode left switched on after exit means a shell that no longer echoes
//! what the user types, and that is not something they should have to know how
//! to fix. A pact runs on a thread of its own now, so a panic *there* is one of
//! those ways out too — and it is covered by the same process-wide hook, which
//! is why the hook is installed before anything else happens.
//!
//! The one long keystroke is the pact key, and it is the reason the loop below
//! is shaped the way it is. A subtree pact is minutes of model passes, so
//! pressing the key spawns a worker thread ([`pacting::spawn_pact`]) and hands back a
//! [`Receiver`] of what that worker has to say: which directory it is on, and,
//! once, how the whole thing went. The loop polls for a keystroke with a short
//! timeout instead of blocking on one, drains that channel every frame, and
//! draws — so the tree still scrolls, the footer's progress line still advances,
//! and the run lands on screen the moment the worker is done with it. Nothing
//! here waits on the worker: the manifest, the tree and the message are updated
//! from the events it sends, and the thread is never joined.
//!
//! The refresh key is the second long keystroke and is not a second anything
//! else: `r` asks the engine to describe only the stale directories under the
//! selected one, and it does so through the same [`start_run`], the same
//! channel, the same account and the same [`Running`] — which is what makes one
//! run at a time a fact rather than a rule. The two keys refuse each other by
//! that alone: whichever run is in flight, the other key's press finds a `Some`
//! here and says so on the line the reader is already watching.
//!
//! Those events are also what fills the panel. The press that really starts a
//! run opens an [`warlock_tui::Account`] on the app — one pact, one account, so
//! the next run clears the last one — each directory the worker names opens a
//! section of it, and everything a pass is seen doing lands under the section it
//! belongs to. Both halves are [`apply_progress`], which is handed the instant
//! it is called at rather than reading a clock, and so is the draw above it: the
//! newest line of the live section counts up against that instant, which is what
//! makes a pass that thinks for a minute look like something is happening. The
//! loop's existing hundred-millisecond round is the whole of the tick — there is
//! no timer, no second thread and no redraw on a schedule of its own.
//!
//! What a run leaves behind is on disk rather than in the rows, and that is the
//! other thing the shape of the loop is for. A pact writes a `WARLOCK.md` beside
//! every directory it descends through, and those are rows the tree on screen
//! has never had, so the moment a run ends the view is one load out of date. One
//! rule covers it: [`apply_progress`] does its own arm's work first — the
//! outcome applied, the manifest saved — and then [`reload_tree`] re-reads the
//! tree from disk and re-seats the view on top of it, carrying the selection,
//! the collapsed directories, the filters and the window across by path. The
//! same single call ends all four ways a run can finish and an un-pact besides,
//! it runs here on the loop's thread and never on the worker's, and a load that
//! fails this late keeps the tree already drawn instead of ending the loop.
//!
//! The third and last reason to reload is that the disk moved without anybody
//! here pressing a key, and it is what [`Watched`] is for. A watcher started
//! beside the first load reports every path that changes under the tree's root
//! and at `.warlock/pacts.toml`; the loop drains it once a round, holds each
//! path against the directories the last successful load produced — the walk is
//! the whole filter, so a `cargo build` writing into a directory no walk
//! produced costs one comparison a path and nothing else — and asks a
//! [`WatchPolicy`] whether the tree is owed a reload yet. When it is, the same
//! [`reload_tree`] runs, on this thread, and the tree it hands back becomes the
//! filter the next round holds paths against. So a file saved in another window
//! turns its directory yellow with no keystroke instead of waiting for a
//! relaunch. Two things this reason gives way to: a pact in flight, whose
//! documents set off events the run's own end-of-run reload already answers, so
//! the trigger is remembered and nothing is read twice; and warlock itself,
//! since a watcher that will not start is one line on the footer rather than a
//! way out of [`run`] — warlock with no live updates is warlock as it was.
//!
//! A run that takes minutes has to be stoppable, and there are two ways to stop
//! one, which this file keeps apart on purpose. Esc *cancels*: the descent ends
//! between directories, the `claude` running right now is killed, and the worker
//! still finishes — it hashes and grants what it did write and saves the
//! manifest, so the record on disk is what actually completed. `q` and Ctrl-C
//! *quit*: the same handle kills the same child, but nothing waits for the
//! worker to tidy up, so the manifest is simply never rewritten. Either way the
//! documents already written are whole, because each of them is written beside
//! its directory and renamed over (WAR-21.01), and the manifest is written once
//! by a rename too — so there is no half-state for an abandoned worker to leave.
//! Both roads run through one [`Cancel`], which the run's [`Running`] owns and
//! drops through, which is why every way out of the loop — a quit, an error, a
//! `?` in the middle of a frame — takes the child with it.
//!
//! The loop answers a pointer as well as a keyboard, and it is the same
//! arrangement twice over. [`TerminalGuard`] asks the terminal to report its
//! mouse in the same breath as it takes the alternate screen, so the reporting
//! is switched off by the same [`restore_terminal`] every way out already runs
//! through; and an event that arrives is turned into an intention by
//! [`mouse_action`], which is [`action_for`] for the pointer — a function of the
//! event, the size this round measured and the app, with no terminal in it. The
//! wheel drives whichever pane the pointer is over rather than whichever pane
//! has the keys, a left click selects a row and takes the keys with it, and
//! everything else a mouse can send is read and dropped, so a pointer swept
//! across the screen changes nothing and costs no more than the round it
//! arrived in.
//!
//! The reader can hand the pointer back. `m` turns the terminal's reporting off
//! and on for the rest of the session ([`Action::ToggleMouseCapture`]); with it
//! off the terminal keeps its own text selection and no `Event::Mouse` arrives
//! at all, so nothing here needs a second gate. Whether it is on is this
//! thread's to know — the app is handed a copy of it every round so the footer
//! can name the key by what the next press does — and every way out still goes
//! through [`restore_terminal`], which turns reporting off whichever state the
//! toggle was left in.

use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;
use warlock_tui::{ClaudeAgent, Focus, QuitConfirm, draw, panel_height, tree_height};

mod error;
mod input;
mod pacting;
mod session;
mod terminal;

use error::Error;
use input::{Action, MouseAction, action_for, mouse_action};
use pacting::{Running, Work, apply_progress, pact_press, refresh_press, start_run};
use session::{Watched, load_app, load_manifest, note};
use terminal::{TerminalGuard, install_panic_hook};

/// How long the loop waits for a keystroke before going round again.
///
/// The number is a compromise between two things that are both cheap. A pact in
/// flight says where it has got to over a channel, and nothing but the top of
/// the loop reads that channel, so this is also how long a progress line can be
/// out of date: a tenth of a second is under the threshold at which a person
/// reads a screen as lagging. Ten wakeups a second with nothing to do is a
/// rounding error next to a terminal that repaints on every keystroke, and the
/// draw either side of it writes nothing when nothing has changed, because
/// ratatui diffs each frame against the last.
///
/// Polled rather than blocked on even when no pact is running, so there is one
/// loop rather than two: a second, blocking path taken only while idle would be
/// a second place the frame is drawn and the keys are handled, for a saving of a
/// few timer wakeups.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    // Before anything touches the terminal: a panic during setup has to leave
    // the terminal usable too.
    install_panic_hook();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // `run` has returned, so the guard inside it has already dropped and
        // the terminal is back to normal; only now is it worth printing
        // anything, because on the alternate screen nobody would ever see it.
        Err(error) => {
            eprintln!("warlock: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Load the tree, set the terminal up, run the event loop, and put the
/// terminal back.
///
/// The load happens *before* the guard is entered, on purpose. Both orders
/// restore the terminal correctly — the guard's `Drop` covers every `?` after
/// it, and it does not exist before it — so the choice is about what the user
/// sees on the failing path: loading first means a repository that will not
/// load never switches the screen at all, instead of flashing the alternate
/// screen up and tearing it down again around a message that is printed after
/// it is gone. It also keeps every filesystem error out of raw mode.
///
/// After the guard is entered, every `?` returns through its `Drop`, which is
/// the whole reason the guard exists: there is no error path out of this
/// function that skips restoration. The pact key is the one keystroke that
/// writes to disk, and it is deliberately not one of those paths — a pact that
/// goes wrong is news for the footer, not a reason to tear the screen down
/// (see [`apply_toggle`]).
///
/// The loop draws, waits [`POLL_INTERVAL`] for a keystroke, and then applies
/// whatever a pact in flight has said since the last time round. That order is
/// the one that matters: a progress event drained at the bottom of the loop is
/// on screen at the top of the next one, a few milliseconds later, and a
/// keystroke pressed during a pact is handled by exactly the same match as a
/// keystroke pressed with nothing running. Quitting while a pact runs returns
/// from here without joining the worker — see [`pacting::spawn_pact`] for why that is
/// safe — and the guard restores the terminal on the way out as it always did.
///
/// The round has a third thing in it now: what the disk did while the loop was
/// waiting on a keystroke. [`Watched`] is drained and asked once a round, after
/// a run that ended has had its own reload, and a watcher that could not be
/// started is a line put on the footer here — once, before the loop — and never
/// an error out of this function.
fn run() -> Result<(), Error> {
    let (mut app, scope, tree) = load_app()?;
    // Loaded before the terminal is touched, for the same reason the tree is:
    // a manifest that will not parse should say so on the normal screen. This
    // is a second read of the file the loader already parsed, which is cheap
    // and keeps the front end from reaching into the loader's internals for a
    // value it needs to keep and edit.
    let mut manifest = load_manifest(&scope.repo_root)?;
    // The one thing in this binary that runs a model, built once because it is
    // a command line and a timeout rather than a connection: nothing is spawned
    // until a pact actually asks for a pass.
    let agent = ClaudeAgent::new();
    // Asked for once, over the tree the load just produced, and kept for as
    // long as warlock runs — dropping it stops the watch. Whether it was
    // granted is a fact for the footer and nothing more, which is why this is
    // not a `?`: warlock with no live updates is warlock as it was.
    let mut watched = Watched::start(&scope, &tree);
    // Said here rather than in the loop, so it is one line said once and not a
    // line re-set ten times a second. It gives way to anything the app already
    // has to say, exactly as the reload's own line does — see [`note`].
    if let Some(line) = watched.off_note() {
        note(&mut app, line);
    }
    let mut guard = TerminalGuard::enter()?;
    // Whether the terminal is reporting its mouse, and the only record of it:
    // the guard has just asked it to, and `m` is the one thing that changes the
    // answer. It lives here rather than on the app because it is a fact about a
    // terminal — the app is handed a copy of it every frame for the footer's
    // sake, and copies of the app are taken and put back around a pact, which a
    // source of truth must survive.
    let mut mouse_captured = true;
    // The pact running somewhere else, when one is: everything this thread
    // needs to keep about a run it is not performing. `None` is the ordinary
    // state, and it is what the pact key checks before starting anything.
    let mut pact: Option<Running> = None;

    loop {
        // Told before it is drawn, and every frame rather than on resize: the
        // scroll offset is only right if it was computed against the height
        // this frame gives the tree, and `tree_height` is the same layout the
        // frame is cut by. A terminal resized between frames is handled by that
        // alone — the next frame measures again, and the next frame is at most
        // one `POLL_INTERVAL` away.
        let size = guard.terminal.size()?;
        app.set_viewport_height(tree_height(size));
        // The panel's window is measured the same way and for the same reason,
        // off the same size: `panel_height` and `tree_height` are two answers
        // from the one layout, so both panes are scrolled by the height this
        // frame is about to give them.
        app.set_panel_height(panel_height(size));
        // And told what the terminal is doing with the pointer, for the same
        // reason and in the same place: the footer names the `m` key by what the
        // next press of it will do, and the app cannot see a terminal. Every
        // frame rather than at the keystroke, so a view restored from the copy
        // taken before a pact — which is a copy of a flag that may have been
        // toggled since — is put right before it is drawn.
        app.set_mouse_captured(mouse_captured);
        // The instant this frame is being drawn at, read once and handed to the
        // renderer: the panel's newest clock counts up against it, so a frame
        // drawn with no event waiting still shows a run that is moving. See
        // `draw`.
        //
        // The gate on the way out is handed in beside the app because the app
        // has never heard of it (see `QuitConfirm`). This loop does not open it
        // yet, so what is drawn here is the frame warlock has always drawn.
        guard
            .terminal
            .draw(|frame| draw(frame, &app, Instant::now(), QuitConfirm::Closed))?;

        // Waited on rather than blocked on. Nothing is drawn while this thread
        // sits here, so the wait has to end whether or not anybody presses
        // anything: a pact reports its progress over a channel that only the
        // bottom of this loop reads, and a progress line that waits for a
        // keystroke to appear is worse than none at all.
        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                // The mode is passed in rather than read out of the app
                // because there is exactly one key it changes the meaning of —
                // Esc, which cancels a run when there is one and quits when
                // there is not. See [`action_for`].
                Event::Key(key) => match action_for(key, pact.is_some()) {
                    // Returning is the whole of quitting, and it is enough even
                    // with a pact in flight. `pact` drops on the way out, which
                    // cancels the run and kills the `claude` it was waiting on
                    // (see [`Running`]); the guard drops after it and puts the
                    // terminal back. Nothing joins the worker: it is left to be
                    // ended by the process, having written whole documents or none,
                    // and the manifest it never got to rewrite still says what it
                    // said before.
                    Some(Action::Quit) => return Ok(()),
                    // Esc with a run in flight. The handle does both halves at once
                    // — it latches, so the descent stops at the next directory
                    // instead of starting a pass for it, and it kills the `claude`
                    // running right now, so that stop happens in milliseconds
                    // rather than at the end of a five-minute pass.
                    //
                    // The pact is deliberately *not* taken down here. The worker is
                    // still going to hash what it wrote, save the manifest and
                    // report, and all of that arrives at the bottom of this loop
                    // like any other outcome; forgetting about it now would leave
                    // the footer's progress line up for a run nobody was listening
                    // to any more.
                    Some(Action::CancelPact) => {
                        if let Some(running) = pact.as_ref() {
                            running.cancel.cancel();
                        }
                    }
                    // Nothing but a bit of view state moves here, and deliberately
                    // so: focus decides which border the next frame lights and which
                    // pane a movement key is about, and both of those questions are
                    // answered where they are asked — by the renderer reading
                    // `App::focus`, and by the app's own movement methods, which
                    // move the tree's selection or scroll the panel's window
                    // depending on the pane being driven (WAR-26.02). There is
                    // nothing for this arm to gate a second time, and no message: a
                    // key that changes what the *next* key means has nothing to
                    // report.
                    Some(Action::ToggleFocus) => app.toggle_focus(),
                    Some(Action::SelectPrevious) => app.select_previous(),
                    Some(Action::SelectNext) => app.select_next(),
                    // No height is passed: the app was told the viewport's height
                    // at the top of this loop, so a page is whatever the frame just
                    // drawn could show.
                    Some(Action::SelectPageUp) => app.select_page_up(),
                    Some(Action::SelectPageDown) => app.select_page_down(),
                    Some(Action::SelectFirst) => app.select_first(),
                    Some(Action::SelectLast) => app.select_last(),
                    // Nothing else happens here on purpose. What is collapsed is
                    // the front end's view of the tree and never touches disk (§8),
                    // so there is no manifest to write; the tree has not changed,
                    // so there is nothing to re-read. The app moves the selection
                    // and the scroll offset back into range itself, and the next
                    // frame — the top of this same loop — draws the shorter or
                    // longer list.
                    Some(Action::ToggleCollapsed) => app.toggle_collapsed(),
                    // Nothing else happens here either, and for the same reasons as
                    // collapsing: which rows are worth looking at is the front end's
                    // view of the tree and is never written down (§5), so there is
                    // no manifest to save, and the tree itself has not changed, so
                    // there is nothing to re-read. The app re-flows its rows and
                    // puts the selection and the scroll offset back in range; the
                    // next frame draws whatever is left.
                    Some(Action::TogglePactedOnly) => app.toggle_pacted_only(),
                    // Nothing else here either, for the third time and for the same
                    // reasons as the two arms above: whether the files inside a
                    // module are on screen is the front end's view of the tree and
                    // is never written down (§5), so there is no manifest to save,
                    // and the files were read by the load that built these rows, so
                    // there is nothing to re-read. The app re-flows its rows and
                    // keeps the selection and the scroll offset in range; the next
                    // frame draws the longer or shorter list.
                    Some(Action::ToggleFiles) => app.toggle_files(),
                    // The one keystroke that writes anything, and the one that
                    // takes longer than a frame — so it is the one that is not done
                    // here. The subtree pact goes to a worker thread and this arm
                    // keeps the receipt: what the app looked like before the toggle
                    // painted it, and which subtree was painted. Everything the run
                    // produces arrives at the bottom of this loop, one directory at
                    // a time and finally as an outcome, and until it does the loop
                    // goes round as usual — drawing, scrolling, filtering.
                    //
                    // `None` needs nothing done about it, and it covers two cases
                    // that are alike in exactly this way: both have already said
                    // their piece on the app, and the next frame draws it. A refused
                    // toggle put its sentence in `App::message`; a press while a
                    // pact is in flight started nothing — a second pact over a tree
                    // the first one is still writing to would be two runs racing for
                    // the same documents and the same manifest — and said so by
                    // setting the flag that words `App::pact_line` as already
                    // running. Neither is this arm's to explain — see `pact_press`.
                    Some(Action::TogglePact) => {
                        // Copied before the toggle paints anything, because the
                        // toggle is no longer its own undo: it puts a whole subtree
                        // into one state, and the states it painted over were not
                        // all the same one. The copy is a list of rows and a tally,
                        // and it is taken once per press of one key.
                        let before = app.clone();
                        // The instant the key was pressed is the instant the run
                        // starts, and the account counts its clocks from it: a run
                        // is as old as the keystroke that asked for it, not as old
                        // as the first thing the model got round to saying.
                        if let Some(toggle) = pact_press(&mut app, pact.is_some(), Instant::now()) {
                            // The worker, the channel and the say-when, in the one
                            // value this loop keeps about a run it is not doing —
                            // see [`start_run`], which is where both keys start
                            // theirs.
                            pact = Some(start_run(
                                Work::Pact(toggle),
                                before,
                                &manifest,
                                &scope.repo_root,
                                &agent,
                            ));
                        }
                    }
                    // The same arm again with one word changed, and that is the
                    // point of it: a refresh is a run like a pact — one worker,
                    // one channel, one account, one say-when — over the stale
                    // directories of the subtree rather than all of them, and
                    // which those are is the engine's judgement and not this
                    // loop's. So the copy is taken the same way, the press
                    // decides the same three things ([`refresh_press`]), and
                    // what comes back fills the very same `Option<Running>`,
                    // which is what makes the two keys refuse each other: the
                    // in-flight check both of them go through is `pact.is_some()`
                    // here, whichever run is the one in flight.
                    //
                    // `None` needs nothing done about it here either, for
                    // `TogglePact`'s two reasons: a row the app turned down has
                    // its sentence in `App::message` already, and a press while
                    // a run is going said so on that run's progress line.
                    Some(Action::Refresh) => {
                        let before = app.clone();
                        if let Some(directory) =
                            refresh_press(&mut app, pact.is_some(), Instant::now())
                        {
                            pact = Some(start_run(
                                Work::Refresh(directory),
                                before,
                                &manifest,
                                &scope.repo_root,
                                &agent,
                            ));
                        }
                    }
                    // The one key that answers to the terminal rather than to
                    // the app. The sequence is written first and the flag moved
                    // only if it went out, so what this thread believes about
                    // the terminal is what it last successfully told it; a write
                    // that fails takes the whole loop down through the guard,
                    // which turns capture off on the way past whatever state it
                    // was left in.
                    //
                    // Nothing else happens: no focus moves, no row is selected,
                    // nothing is redrawn here — the top of the loop draws every
                    // round, and it is where the footer picks the new wording up.
                    // With capture off the terminal keeps the pointer to itself,
                    // so `Event::Mouse` simply stops arriving and the mouse
                    // handler needs no gate of its own.
                    Some(Action::ToggleMouseCapture) => {
                        if mouse_captured {
                            execute!(io::stdout(), DisableMouseCapture)?;
                        } else {
                            execute!(io::stdout(), EnableMouseCapture)?;
                        }
                        mouse_captured = !mouse_captured;
                    }
                    // A key nothing is bound to, or one whose press has already
                    // been answered where it was decided.
                    None => {}
                },
                // The pointer, answered in the same shape and for the same
                // reasons. [`mouse_action`] is handed the event, the size this
                // round measured at the top — the one the hit test has to
                // agree with, because it is the size the frame above was drawn
                // at — and the app, since which row a click lands on depends
                // on where the tree's window is. Nothing here reads the
                // terminal and nothing draws: the round is the redraw, which is
                // why a pointer swept across the screen costs nothing.
                Event::Mouse(mouse) => match mouse_action(mouse, size, &app) {
                    // The wheel over the tree column, whichever pane the keys
                    // are pointed at: the selection moves and the window
                    // follows it, exactly as it does for a movement key.
                    Some(MouseAction::SelectNextBy(rows)) => app.select_next_by(rows),
                    Some(MouseAction::SelectPreviousBy(rows)) => app.select_previous_by(rows),
                    // The panel's half of the same wheel. What the follow rule
                    // makes of it is the app's business and is not restated
                    // here: a window scrolled back stops following the newest
                    // line, and one scrolled to the end starts again.
                    Some(MouseAction::ScrollPanelDown(lines)) => app.scroll_panel_down(lines),
                    Some(MouseAction::ScrollPanelUp(lines)) => app.scroll_panel_up(lines),
                    // A click names a row, and a click in a pane also says
                    // which pane the keys are about from now on: the reader has
                    // just pointed at it, and leaving the keys driving the
                    // other pane would send the next `j` somewhere they are not
                    // looking.
                    Some(MouseAction::SelectRow(index)) => {
                        app.set_focus(Focus::Tree);
                        app.select_row(index);
                    }
                    // A click on the row that is already selected, which is
                    // space by another road — so it goes through the very
                    // method space goes through, and a row with nothing under
                    // it collapses nothing here exactly as it would there.
                    Some(MouseAction::ToggleCollapsed) => {
                        app.set_focus(Focus::Tree);
                        app.toggle_collapsed();
                    }
                    // A click inside a pane with nothing under it: the tree's
                    // header, the space below its last row, a line of the
                    // panel. Taking the focus is the whole of what it does.
                    Some(MouseAction::Focus(focus)) => app.set_focus(focus),
                    None => {}
                },
                // Resizes, focus changes and pasted text: read and dropped. The
                // frame is measured again at the top of every round, so a
                // resize needs nothing done about it here, and the other two
                // mean nothing to a program with nowhere to paste into.
                _ => {}
            }
        }

        // The clock is read here rather than inside the two calls below for the
        // same reason it is read before the draw: this file owns the clock and
        // everything under it is a function of the instant it is handed. What
        // that instant means is when the events being drained now landed on
        // screen, which is within one `POLL_INTERVAL` of when they were sent.
        let now = Instant::now();

        // Read before the drain, because the drain is what ends a run: a pact
        // that is `Some` here and `None` below finished in this round, and its
        // own reload has already read the tree.
        let running = pact.is_some();
        // Every frame, whether or not a key was pressed: this is the only place
        // anything the worker says reaches the screen, and it has to keep up
        // with a thread that is not waiting for it. The scope the app was
        // loaded at is handed to it because the end of a run re-reads the tree
        // — see [`reload_tree`].
        let reloaded = apply_progress(&mut pact, &mut app, &mut manifest, &scope, now);
        if running && pact.is_none() {
            // The run's documents are exactly the sort of thing the watcher
            // reports, so the events it set off are already sitting in the
            // policy. They are answered by the reload that just happened rather
            // than by one of their own, and the tree it read becomes the filter.
            watched.caught_up(reloaded.as_ref(), now);
        }
        // And then what everything else did to the disk. Nothing is read again
        // while a pact is in flight — the trigger keeps until the run's own
        // reload above — and nothing at all is read when the disk has been
        // still, which is almost every round.
        watched.round(&mut app, &scope, pact.is_some(), now);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_binary_is_named_warlock() {
        assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
    }
}
