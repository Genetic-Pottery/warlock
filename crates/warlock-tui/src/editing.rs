//! The `e` key: the selected file handed to `$EDITOR` as a foreground child,
//! and the screen taken back afterwards. Not one byte of the file passes
//! through warlock — the child opens it and the child saves it.
//!
//! Editing a `WARLOCK.md` restales the directory it describes, because the
//! document is an ordinary file in the walk
//! [`subtree_hash`](warlock_engine::subtree_hash) makes; [`came_back`] reloads
//! the tree so the row says so at once rather than at the next keystroke.
//!
//! `$EDITOR` is split on whitespace and never handed to `sh -c`. A shell in the
//! middle would make warlock's behaviour depend on which `/bin/sh` a machine
//! has, turn a path with a space in it into two arguments, and make a `$EDITOR`
//! set from a variable a way to run arbitrary commands. The price is quoting.
//! `VISUAL` is not consulted and there is no fallback editor, because dropping
//! somebody into a modal editor they cannot leave is worse than a line of
//! footer saying what to set.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, io};

use warlock_engine::{Manifest, Viewed, view_file};
use warlock_tui::App;

use crate::error::one_line;
use crate::session::{Scope, note, reload};
use crate::terminal::Screen;

const EDITOR_VAR: &str = "EDITOR";

// One sentence for four situations, because they are one situation: unset,
// empty, whitespace only, and a value this platform holds as bytes that are not
// text. It names the variable and one way to set it, because a reader who has
// never exported `EDITOR` needs the name of the thing to set rather than a fact
// about warlock.
const NO_EDITOR: &str = "`$EDITOR` names no editor to run, so nothing was opened: \
                         set it (for example `EDITOR=nvim`) and press `e` again";

// A pair rather than a `Command`, so the whole of what was read out of the
// environment can be compared and asserted about without spawning anything.
#[derive(Debug, PartialEq, Eq)]
struct Editor {
    program: String,
    args: Vec<String>,
}

// The three refusals in the order that costs the reader least: a run in flight
// first, because it is true of the session rather than of the row; then the row,
// in `App::view_target`'s words, which are `v`'s words; then `$EDITOR`, so a
// press already refused for its row is not also judged on a variable it never
// got as far as needing.
//
// The child is waited on inside the suspension, which makes the whole editing
// session one turn of the event loop: no frame is drawn behind it and no
// progress event is drained until it exits.
//
// The `io::Result` is the terminal and only the terminal. How the *editor* went
// is never an error here — both ways it can go wrong come back from
// `run_editor` as a line for the footer.
//
// `showing` is which file the panel's document card is holding, passed in rather
// than asked of the app because the app is never handed a path at all (see
// `App::show_document`).
pub(crate) fn edit_press(
    app: &mut App,
    screen: &mut impl Screen,
    scope: &Scope,
    manifest: &mut Manifest,
    showing: Option<&Path>,
    mouse: bool,
    in_flight: bool,
) -> io::Result<()> {
    let Some(path) = edit_target(app, in_flight) else {
        return Ok(());
    };
    // Read once, and only once the press is known to be worth reading it for.
    let Some(editor) = editor_command(env::var_os(EDITOR_VAR).as_deref()) else {
        app.set_message(NO_EDITOR);
        return Ok(());
    };

    // The one call in warlock that hands the screen to somebody else. Everything
    // the child has to say about how it went comes back as a value, because
    // there is deliberately no way to leave the closure with the terminal still
    // down — see `Screen::suspended`.
    if let Some(line) = screen.suspended(mouse, || run_editor(&editor, &path))? {
        app.set_message(line);
    }
    came_back(app, scope, manifest, &path, showing);
    Ok(())
}

// The tree is read again on every outcome. Telling the three apart to skip the
// walk was considered and left out: an editor that exited non-zero may still
// have saved before it did, and one directory walk is cheap on a keystroke that
// has just cost minutes of somebody's typing.
//
// The card is refilled only when `showing` is the very file that was edited. Any
// other file was not touched by this press, and re-reading it would put a reader
// who had parked the window back at the top of a file nobody changed.
// `refill_document` rather than `show_document`, so which card is on screen does
// not move: an edit is not the reader asking to look at something.
//
// The watcher is not told. The editor's write set off events that sat in
// `Watched`'s channel while this thread was blocked, so the loop may walk the
// repository once more a round later; that is a harmless price for this function
// not reaching into the watcher. Compare `watched.caught_up`, which is how a run
// discharges the same debt from inside the loop.
fn came_back(
    app: &mut App,
    scope: &Scope,
    manifest: &mut Manifest,
    edited: &Path,
    showing: Option<&Path>,
) {
    // The tree it read is not kept: nothing here filters on a walk, and the
    // watcher's own filter is caught up by the reload it does for itself.
    let _ = reload(app, scope, manifest);

    if showing != Some(edited) {
        return;
    }
    match view_file(edited) {
        Ok(Viewed { text, cut }) => app.panel_mut().refill_document(text.lines(), cut),
        // The engine's wording, flattened onto one line exactly as `view_press`
        // flattens it — the same failure, reached without a keystroke of its own.
        Err(error) => note(app, one_line(&error.to_string())),
    }
}

// The run is asked first, so a mid-run press over a directory says the thing
// that is true of the whole session rather than the thing that is true of the
// row. Neither refusal is worded here: `App::view_target` refuses a row that is
// not a file in the words `v` uses.
fn edit_target(app: &mut App, in_flight: bool) -> Option<PathBuf> {
    if in_flight {
        // The whole of the refusal: a bit of wording on a line that is already
        // on screen. Setting it again says the same thing, so a reader leaning
        // on the key changes nothing after the first press — and nothing is
        // suspended, so the run goes on drawing on a screen it still owns.
        app.set_pact_refused();
        return None;
    }
    app.view_target()
}

// `split_whitespace` is the whole of the parsing, which answers the awkward
// spellings by construction: a shell that exported `EDITOR=" code -w "` meant
// `code -w`.
//
// A value that is not UTF-8 comes back `None` and lands in the same footer line
// as an unset one. It cannot be split — whitespace is a fact about text — and
// running the whole thing unsplit would make `EDITOR="code -w"` behave
// differently depending on the encoding of the bytes around it.
//
// Takes the value rather than reading the variable, for the reason
// `claude::or_default` is written that way: setting a real environment variable
// is process-wide, racy against every other test on the runner and unsafe
// besides.
fn editor_command(value: Option<&OsStr>) -> Option<Editor> {
    let mut words = value?.to_str()?.split_whitespace();
    let program = words.next()?.to_owned();
    Some(Editor {
        program,
        args: words.map(str::to_owned).collect(),
    })
}

// Called with the terminal already given up, so every stream is inherited rather
// than piped. That is `Command`'s default and the only way an interactive editor
// works at all — worth saying next to `claude.rs`, where every stream is piped
// and read by a thread apiece.
//
// `status` waits, which is the point: the event loop is blocked for as long as
// somebody is typing. The path goes last and goes as a path, so nothing here has
// to think about a name containing a space, a quote or a newline.
//
// An editor that exited zero says nothing — a reader who has just come back from
// their own editor can see that they have. The other two name the program,
// because "it did not work" about a program the reader named in their own
// environment is not something they can act on.
fn run_editor(editor: &Editor, path: &Path) -> Option<String> {
    let program = &editor.program;
    match Command::new(program).args(&editor.args).arg(path).status() {
        Ok(status) if status.success() => None,
        // What the child said on its way out, in the terminal's own words —
        // `exit status: 1`, or the signal that ended it. Warlock has no way to
        // know whether the file was saved before that happened, so it says the
        // fact and not a guess.
        Ok(status) => Some(format!(
            "`{program}` exited with {status}, so it may not have saved what was open"
        )),
        // Never started at all: the usual reason is a `$EDITOR` naming something
        // that is not on this machine's `PATH`, so the line names the variable
        // as well as the program.
        Err(error) => Some(format!(
            "could not run `{program}`: {}; check `$EDITOR`",
            one_line(&error.to_string())
        )),
    }
}

// Nothing here suspends anything: the suspension is `TerminalGuard`'s and a test
// harness has no terminal to give up. What that leaves is the parts with
// decisions in them.
#[cfg(test)]
#[path = "tests/editing.rs"]
mod tests;
