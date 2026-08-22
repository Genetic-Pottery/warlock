//! The terminal's lifecycle: taken whole, given back whole, on every way out.
//!
//! The one rule this module keeps is the binary's founding one — raw mode, the
//! alternate screen and mouse reporting are restored on a normal quit, on an
//! error returned up to `main`, and on a panic. [`TerminalGuard`] covers the
//! first two by ownership, [`install_panic_hook`] covers the third, and both
//! run through the same [`restore_terminal`], so there is exactly one spelling
//! of what "put it back" means.

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
    /// which is why this undoes its own work before returning the error — and
    /// why capture is turned on in the same [`execute!`] as the alternate
    /// screen: whichever of the two fails, the restoration below undoes both,
    /// because every step of it is attempted whether or not it was ever needed.
    pub(crate) fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
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
