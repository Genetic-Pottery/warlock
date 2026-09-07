//! Raw mode, the alternate screen, bracketed paste and mouse reporting are
//! restored on every way out — a quit, an error returned to `main`, a panic,
//! and the suspension a child process runs inside. `take_terminal` and
//! `restore_terminal` are the only two spellings of that in the crate, and
//! keeping it that way is the point of the module: a session taken one way and
//! resumed another is one where `m` silently un-presses itself.

use std::io::{self, Stdout};
use std::panic;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::Size;
use ratatui::{Frame, Terminal};

// A trait, with `TerminalGuard` on one side and a test's in-memory recorder on
// the other, so a `Session` can be built and driven round after round on a
// machine with no terminal attached. What a frame *contains* is deliberately
// not here: that is `draw` over an `App`, already testable against a
// `TestBackend`.
pub(crate) trait Screen {
    // Asked once a round rather than cached: a terminal is resized by somebody
    // else, so the size a frame is cut by has to be the size the screen has at
    // the moment it is cut.
    fn size(&self) -> io::Result<Size>;

    fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> io::Result<()>;

    // `mouse` is passed in rather than assumed: `m` may have turned reporting
    // off, and the event loop's flag is the only record of that.
    //
    // `body` gets a value out rather than a `?`, and that is the interface
    // rather than an implementation detail — with no `?` between them there is
    // no road from the teardown to the setup that skips the setup. A caller
    // with something to say about how the child went says it afterwards, with
    // the screen back.
    fn suspended<T, F: FnOnce() -> T>(&mut self, mouse: bool, body: F) -> io::Result<T>;

    // Here rather than at the keystroke, because this module is meant to be the
    // only place that knows what warlock does to a terminal: a key writing its
    // own escape sequence to stdout stays invisible until a test runs it and
    // the sequence lands in the test's output. The caller moves the flag it
    // keeps only after this returns, which holds what warlock believes about
    // the terminal down to what it last successfully told it.
    fn report_mouse(&mut self, on: bool) -> io::Result<()>;
}

// Setup and teardown paired by ownership rather than by remembering a teardown
// call on each of the several ways out of the event loop. The panic path is
// covered separately, by `install_panic_hook`, because a panic unwinding
// through here would run the hook first and print its message onto the
// alternate screen, which then disappears with it.
pub(crate) struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    // The guard never exists on a failure part-way through, so it never drops,
    // so this undoes its own work before returning: every step of
    // `restore_terminal` is attempted whether or not it was needed, so whichever
    // step of `take_terminal` failed, what it did get done is undone.
    //
    // The `true` is because a session always starts with the pointer reported —
    // `m` cannot have been pressed yet — which is why `suspended` takes the flag
    // and this does not.
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
}

impl Screen for TerminalGuard {
    fn size(&self) -> io::Result<Size> {
        self.terminal.size()
    }

    fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> io::Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }

    fn suspended<T, F: FnOnce() -> T>(&mut self, mouse: bool, body: F) -> io::Result<T> {
        restore_terminal();
        let said = body();
        if let Err(error) = take_terminal(mouse) {
            restore_terminal();
            return Err(error);
        }
        // Ratatui draws by diffing against the frame it last drew, and what the
        // child left on the screen is not that frame. Resizing to the size the
        // terminal has right now clears the viewport and throws the remembered
        // frame away, so the next `draw` writes every cell. `Terminal::clear`
        // does the same and one thing more — it asks the terminal where its
        // cursor is — and that is a write and a read with a timeout on it,
        // asked of a terminal a foreign program has just been typing on.
        let area = self.terminal.size()?.into();
        self.terminal.resize(area)?;
        Ok(said)
    }

    fn report_mouse(&mut self, on: bool) -> io::Result<()> {
        if on {
            execute!(io::stdout(), EnableMouseCapture)
        } else {
            execute!(io::stdout(), DisableMouseCapture)
        }
    }
}

// Bracketed paste is unconditional, so it rides in the same `execute!` as the
// screen: a terminal that supports it delivers a pasted block as one event
// instead of a stream of keys whose first newline reads as Enter, and one that
// does not ignores the sequence. Reporting is a statement of its own only
// because it is conditional; nothing is lost by that, since both callers undo a
// part-done setup with `restore_terminal`.
fn take_terminal(mouse: bool) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
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

// Best effort: every step is attempted even if an earlier one fails, and none
// of them report anything. This runs while unwinding from a panic and while
// returning an error, and in both cases there is a more interesting message on
// its way to the user that a complaint about an escape sequence would bury.
fn restore_terminal() {
    let _ = disable_raw_mode();
    // The setup undone in reverse: reporting and bracketing both off before the
    // screen they were turned on for goes away, and the cursor shown last
    // because drawing a frame hid it. Each omission hands back a shell the
    // reader cannot diagnose — an invisible caret, escape sequences whenever
    // the pointer moves, or every paste wrapped — with `reset` as the only cure.
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    );
}

// Order matters twice over. This must be installed before raw mode is entered,
// so a panic during setup is covered too; and it must restore *before*
// delegating, so the panic message lands on the normal screen where it can be
// scrolled back to instead of on the alternate screen that is about to vanish.
// Chaining rather than replacing keeps the default hook's message and
// backtrace.
pub(crate) fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}
