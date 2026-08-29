//! The terminal's lifecycle: taken whole, given back whole, on every way out.
//!
//! The one rule this module keeps is the binary's founding one — raw mode, the
//! alternate screen and mouse reporting are restored on a normal quit, on an
//! error returned up to `main`, and on a panic. [`TerminalGuard`] covers the
//! first two by ownership, [`install_panic_hook`] covers the third, and both
//! run through the same [`restore_terminal`], so there is exactly one spelling
//! of what "put it back" means.
//!
//! There is a fourth way out now, and it is the only one warlock comes back
//! from: [`TerminalGuard::suspended`] gives the terminal up for as long as a
//! child process needs it and takes it again afterwards. It is not a second
//! lifecycle. Giving up is the same [`restore_terminal`] every other way out
//! runs through, and taking back is the same [`take_terminal`]
//! [`TerminalGuard::enter`] takes it with in the first place — so "put it back"
//! and "take it" are each spelled exactly once in this crate, and a child that
//! panics the process, or takes it down with it, is still covered by the hook
//! that was installed before any of this.

use std::io::{self, Stdout};
use std::panic;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// A terminal in raw mode on the alternate screen, reporting its mouse,
/// restored when dropped.
///
/// Setup and teardown are paired by ownership rather than by remembering to
/// call a teardown function on each of the several ways out of the event loop.
/// Dropping it is the only way to restore the terminal on the normal and error
/// paths; the panic path is covered separately by [`install_panic_hook`],
/// because a panic unwinding through here would run the hook first and print
/// its message onto the alternate screen, which then disappears with it.
///
/// Mouse capture is one of the things it owns, and it is owned here for exactly
/// that reason: a terminal left reporting its mouse after warlock is gone spits
/// escape sequences into the shell every time the pointer crosses the window,
/// which is the same class of mess as raw mode left switched on. Every way out
/// — `q`, Esc, Ctrl-C, an error out of [`run`], a panic — goes through
/// [`restore_terminal`], and that is where the reporting is switched off.
pub(crate) struct TerminalGuard {
    /// The ratatui terminal the event loop draws frames on.
    pub(crate) terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enter raw mode and the alternate screen, and ask the terminal to report
    /// its mouse.
    ///
    /// On failure part-way through, the guard never exists and so never drops,
    /// which is why this undoes its own work before returning the error: every
    /// step of [`restore_terminal`] is attempted whether or not it was ever
    /// needed, so whichever step of [`take_terminal`] failed, what it did get
    /// done is undone.
    ///
    /// A session always starts with the pointer reported — `m` is the one thing
    /// that changes that, and it cannot have been pressed yet — which is the
    /// whole of why this reads as a `true` and [`TerminalGuard::suspended`]
    /// does not.
    pub(crate) fn enter() -> io::Result<Self> {
        if let Err(error) = take_terminal(true) {
            restore_terminal();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore_terminal();
                Err(error)
            }
        }
    }

    /// Give the terminal back for as long as `body` runs, take it again
    /// afterwards, and hand over whatever `body` said.
    ///
    /// The one thing in warlock that hands the screen to somebody else and
    /// expects it back: `e` runs `$EDITOR` as a foreground child, and an editor
    /// wants a terminal in its own mode, on the ordinary screen, with the
    /// keyboard to itself. What that costs is spelled here rather than at the
    /// keystroke, so the key module can be about the editor and this one stays
    /// the only place that knows what warlock does to a terminal.
    ///
    /// Neither half is new. Giving up is [`restore_terminal`], which is what a
    /// quit, an error and a panic already run through, so a child inherits a
    /// terminal in exactly the state warlock would have left it in had the
    /// reader pressed `q`. Taking back is [`take_terminal`], which is what
    /// [`TerminalGuard::enter`] took it with — with one thing passed in rather
    /// than assumed: `mouse` says whether the terminal was reporting its
    /// pointer when it was given up. `m` may have turned reporting off, and the
    /// event loop's flag is the only record of that, so resuming without asking
    /// would quietly switch back on something the reader turned off.
    ///
    /// `body` gets a value out rather than a `?`, and that is deliberate: there
    /// is no road from the teardown above to the setup below that skips the
    /// setup, because there is no `?` between them. A caller with something to
    /// say about how the child went says it afterwards, with the screen back.
    ///
    /// The repaint is not decoration. Ratatui draws by diffing against the
    /// frame it last drew, and what the child left on the screen is not that
    /// frame — so without this the first frame back would repaint only the
    /// cells warlock thinks changed, onto an alternate screen the terminal has
    /// just re-created empty. Resizing to the size the terminal has right now
    /// is how that is said: it clears the viewport and throws the remembered
    /// frame away, so the next `draw` writes every cell. `Terminal::clear` does
    /// the same and one thing more — it asks the terminal where its cursor is
    /// and puts it back — and that question is a write and a read with a
    /// timeout on it, asked of a terminal a foreign program has just been
    /// typing on, whose answer warlock has nothing to do with. Not asking it is
    /// both cheaper and one fewer way for coming back to fail.
    ///
    /// Failing to take the terminal back is the one error here, and it leaves
    /// through [`restore_terminal`] as well: whatever the setup managed is
    /// undone before the error goes up, so the caller returning it prints on a
    /// terminal that works.
    pub(crate) fn suspended<T>(&mut self, mouse: bool, body: impl FnOnce() -> T) -> io::Result<T> {
        restore_terminal();
        let said = body();
        if let Err(error) = take_terminal(mouse) {
            restore_terminal();
            return Err(error);
        }
        let area = self.terminal.size()?.into();
        self.terminal.resize(area)?;
        Ok(said)
    }
}

/// Take the terminal: raw mode, the alternate screen, and the pointer reported
/// if `mouse` says it should be.
///
/// The one spelling of setup, as [`restore_terminal`] is the one spelling of
/// teardown, and for the same reason: a session that is taken one way and
/// resumed another is a session where `m` silently un-presses itself, or where
/// the second alternate screen is not the first one's twin.
///
/// Reporting is asked for in a statement of its own rather than in the same
/// [`execute!`] as the screen, because it is now conditional. Nothing is lost by
/// that: both callers undo the whole of a part-done setup with
/// [`restore_terminal`], every step of which is attempted whether or not it was
/// ever needed.
fn take_terminal(mouse: bool) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    if mouse {
        execute!(io::stdout(), EnableMouseCapture)?;
    }
    Ok(())
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Put the terminal back the way it was found, best effort.
///
/// Every step is attempted even if an earlier one fails, and none of them
/// report anything: this runs while unwinding from a panic and while returning
/// an error, and in both cases there is a more interesting message on its way
/// to the user that a complaint about a terminal escape sequence would only
/// bury.
fn restore_terminal() {
    let _ = disable_raw_mode();
    // The setup's steps undone in the order they were done in reverse: mouse
    // reporting off before the screen it was turned on for goes away, and the
    // cursor shown last. Drawing a frame hides the cursor, so leaving without
    // showing it again hands back a shell with an invisible caret; leaving
    // without turning capture off hands back one that prints escape sequences
    // whenever the pointer crosses it, which the reader has no way to guess the
    // cause of and `reset` as their only cure.
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    );
}

/// Install a panic hook that restores the terminal and then chains to whatever
/// hook was installed before.
///
/// Order matters twice over. It must run before raw mode is entered, so a
/// panic during setup is covered as well. And it must restore *before*
/// delegating, so the panic message lands on the normal screen where it can be
/// read and scrolled back to, instead of on the alternate screen that is about
/// to vanish. Chaining rather than replacing keeps the default hook's message
/// and backtrace, which are the entire point of a panic.
pub(crate) fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}
