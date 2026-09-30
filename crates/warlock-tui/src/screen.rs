use std::io;

use ratatui::Frame;
use ratatui::layout::Size;

// A trait, with `TerminalGuard` on one side and a test's in-memory recorder on
// the other, so a `Session` can be built and driven round after round on a
// machine with no terminal attached. What a frame *contains* is deliberately
// not here: that is `draw` over an `App`, already testable against a
// `TestBackend`.
pub trait Screen {
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

    // A method here rather than a write at the keystroke, so the binary's
    // `terminal.rs` stays the only place that knows what warlock does to a
    // terminal: a key writing its own escape sequence to stdout stays invisible
    // until a test runs it and the sequence lands in the test's output. The
    // caller moves the flag it keeps only after this returns, which holds what
    // warlock believes about the terminal down to what it last successfully
    // told it.
    fn report_mouse(&mut self, on: bool) -> io::Result<()>;
}
