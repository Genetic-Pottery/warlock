//! The edit key, from the keystroke to the child that is handed the screen.
//!
//! One press, one child, one terminal given up and taken back: [`edit_press`]
//! asks the app which file the selection is on, works out what `$EDITOR` names,
//! hands both to a foreground child through
//! [`TerminalGuard::suspended`](crate::terminal::TerminalGuard::suspended), and
//! waits. It is [`mod@crate::viewing`]'s other half — that module reads a file
//! into the panel, this one hands the same file to something that can change it
//! — and it is section 9's escape hatch given a keystroke: manual edits are not
//! fought, they are reconciled, and until now there was no key for making one.
//!
//! ## Warlock still writes nothing
//!
//! Not one byte of the file passes through here. The child opens it, the child
//! saves it, and warlock's own writers are still the pact, the refresh, the
//! manifest, the scope key and `warlock init`. What this module owns is an argv
//! and a suspension.
//!
//! ## What editing a document costs
//!
//! A `WARLOCK.md` is an ordinary file in its own directory's walk, so saving one
//! restales the very directory it describes: the subtree hash the pact was
//! granted against no longer matches, and the row goes yellow. That is not a bug
//! to route around — it is the ledger being honest — and the only road back to
//! green is `r` and a pass. There is no key that records a hash without
//! describing a directory, and there will not be one.
//!
//! ## The app words the row, this file does the process
//!
//! [`view_press`](crate::viewing::view_press)'s shape exactly, and deliberately
//! the same call: [`App::view_target`] decides what a press means over the row
//! the selection is on and words every row-level refusal itself — a directory
//! with a document names that document, a directory without one names `p` — so
//! `e` and `v` refuse a directory in one wording rather than two that have to be
//! kept in step. A `None` from it is a press that has already been answered.
//!
//! ## A run in flight is the one refusal `v` does not share
//!
//! A read races nothing, so `v` is never refused mid-run. A child does: it takes
//! the screen a pass is still drawing its account on, and it holds it for as long
//! as somebody is typing. So `e` consults the run exactly as
//! [`scope_press`](crate::scoping::scope_press) does, and says so through
//! [`App::set_pact_refused`](warlock_tui::App::set_pact_refused) — the channel a
//! second `p`, an `r` and an `s` mid-run already use, which is the progress line
//! the reader is watching rather than the message line the run has taken.
//!
//! ## `$EDITOR`, split on whitespace and nothing cleverer
//!
//! The value is split on whitespace: the first word is the program and the rest
//! are arguments handed over before the file. So `EDITOR="code -w"` runs
//! `code -w <file>` and `EDITOR="nvim -u NONE"` runs `nvim -u NONE <file>`, and
//! the argv is built explicitly with [`Command`], as `claude.rs` builds
//! `ClaudeAgent`'s.
//!
//! It is never handed to `sh -c`. A shell in the middle would mean warlock's
//! behaviour depending on which `/bin/sh` a machine has, a file path with a space
//! in it becoming two arguments, and a `$EDITOR` somebody set from a variable
//! being a way to run arbitrary commands. What is given up by refusing the shell
//! is quoting: an editor whose arguments contain spaces cannot be spelled in
//! `$EDITOR` here. That is the trade, made once, and no command-line parsing
//! crate is added to soften it.
//!
//! `VISUAL` is not consulted, and there is no fallback editor: a `vi` warlock
//! chose is an editor the reader did not, and dropping somebody who has never
//! used it into a modal editor they cannot leave is worse than one line of
//! footer saying what to set.
//!
//! ## Every outcome ends with the screen back
//!
//! Three of them — the editor exiting zero, exiting non-zero, and never starting
//! at all — and the terminal is taken back on all three, because the taking back
//! is the suspension's and not the outcome's. The two that went wrong each put
//! one line on [`App::message`](warlock_tui::App::message) naming the program
//! warlock tried to run, where every other non-fatal failure in this binary goes;
//! neither is fatal and neither leaves the event loop.
//!
//! ## And what warlock has to read again on the way back in
//!
//! Two things moved while the screen was somebody else's, and [`came_back`] is
//! both of them. The tree is read again through the same
//! [`reload_tree`](crate::session::reload_tree) a run ends with, because the
//! whole point of the section above is that a saved `WARLOCK.md` restales its
//! directory — a row that only went yellow at the reader's next keystroke would
//! be warlock knowing something and not saying it. And the document card is read
//! again *if it is holding the file that was just edited*, so that a reader who
//! pressed `v`, then `e`, is looking at what they saved rather than at what they
//! opened; a card holding any other file is left byte for byte as it was, because
//! nothing happened to that file.
//!
//! Which file the card is holding is not asked of the app, because the app does
//! not know: it is handed lines and never a path (see
//! [`App::show_document`](warlock_tui::App::show_document)). The loop keeps it,
//! from what [`view_press`](crate::viewing::view_press) hands back, and passes it
//! in here. And the re-read goes through
//! [`App::refill_document`](warlock_tui::App::refill_document) rather than
//! `show_document`, which is the same filling minus the one line that decides
//! what is on screen: an edit is not the reader asking to look at something, so
//! the panel stays on whichever card it was on.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, io};

use warlock_engine::{Viewed, view_file};
use warlock_tui::App;

use crate::error::one_line;
use crate::session::{Scope, note, reload_tree};
use crate::terminal::TerminalGuard;

/// The one environment variable this key reads.
///
/// Not `VISUAL`, and not both: see the module docs. Named here so the constant
/// and the sentence the footer says when it holds nothing runnable sit next to
/// each other.
const EDITOR_VAR: &str = "EDITOR";

/// What the footer says when `$EDITOR` names no program warlock can run.
///
/// One sentence for four situations, because they are one situation: unset,
/// empty, nothing but whitespace, and — the road [`editor_command`] documents —
/// a value this platform's environment holds as bytes that are not text. In all
/// four there is no first word to run, nothing is spawned, and the screen is
/// never given up.
///
/// It names the variable and one way to set it, because a reader who has never
/// exported `EDITOR` needs the name of the thing to set rather than a fact about
/// warlock.
const NO_EDITOR: &str = "`$EDITOR` names no editor to run, so nothing was opened: \
                         set it (for example `EDITOR=nvim`) and press `e` again";

/// The program `$EDITOR` names and the arguments it carries, in the order they
/// are handed to the child.
///
/// A pair rather than a `Command`, so that the whole of what was read out of the
/// environment can be looked at, compared and asserted about without spawning
/// anything.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The first word of `$EDITOR`: the program to run.
    program: String,
    /// Everything after it, in order, handed over before the file path.
    args: Vec<String>,
}

/// What one press of the edit key comes to: the selected file open in
/// `$EDITOR`, or one line in the footer saying why not.
///
/// The three refusals are asked in the order they cost the reader least. A run
/// in flight is first, because it is true of the session rather than of the row
/// and nothing should be resolved before it; the row is next, in
/// [`App::view_target`]'s words, which are `v`'s words; and `$EDITOR` is last,
/// read once, so a press that was going to be refused for its row is not also
/// judged on a variable it never got as far as needing.
///
/// Nothing is given up until all three have passed. The suspension is the last
/// thing that happens and the child is waited on inside it, which is what makes
/// the whole editing session one turn of the event loop: the loop is blocked for
/// the editor's entire life, no frame is drawn behind it, and no progress event
/// is drained until it exits.
///
/// The `io::Result` is the terminal and only the terminal — a screen that could
/// not be taken back is not something to put on a footer nobody can see, so it
/// goes up to `main` through the guard like every other terminal failure. How
/// the *editor* went is never an error here: both ways it can go wrong come back
/// from [`run_editor`] as a line for the footer.
///
/// `showing` is which file the panel's document card is holding, or `None` for a
/// session in which nothing has been read yet. It is passed in rather than asked
/// of the app for the reason the app is never handed a path at all — see the
/// module docs — and it is the whole of what decides whether the card is read
/// again on the way back.
///
/// The child's line is said *before* [`came_back`] reads anything, so that the
/// housekeeping in there gives way to it: [`note`] writes only onto a footer
/// nobody else has claimed, which is the same precedence a run and its own
/// reload have.
pub(crate) fn edit_press(
    app: &mut App,
    guard: &mut TerminalGuard,
    scope: &Scope,
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
    // down — see `TerminalGuard::suspended`.
    if let Some(line) = guard.suspended(mouse, || run_editor(&editor, &path))? {
        app.set_message(line);
    }
    came_back(app, scope, &path, showing);
    Ok(())
}

/// What the editor left behind, read again: the tree, and the document card when
/// it is the edited file that is on it.
///
/// The half of the key that happens after the screen is warlock's again, kept
/// out of [`edit_press`] so that it is a function of an app, a scope and two
/// paths — no guard, no terminal and no child — and so can be driven over a
/// repository of a test's own.
///
/// The tree first, and on every outcome. A `WARLOCK.md` that was saved has
/// restaled its own directory and the row is owed its colour now rather than at
/// the reader's next keystroke; a file that was not a document may have appeared,
/// gone or changed under a directory that hashes it. Telling the three outcomes
/// apart to skip the walk after one of them was considered and left out: an
/// editor that exited non-zero may still have saved before it did, an editor that
/// never started leaves a reload that reads back the tree already on screen, and
/// the saving is one directory walk on a keystroke that has just cost minutes.
/// One road back in beats a cheaper one that has to be kept true.
///
/// Then the card, and only when `showing` is the very file that was edited. Any
/// other file was not touched by this press, so re-reading it would be spending a
/// read to redraw the same lines — and, worse, would put a reader who had parked
/// the window somewhere back at the top of a file nobody changed.
///
/// Which card is on screen does not move either way: the re-read goes through
/// [`App::refill_document`](warlock_tui::App::refill_document), which fills the
/// card and stops there.
///
/// A re-read that fails — the editor deleted the file, saved something that is
/// not text, or wrote it unreadable — leaves the card holding what it held, the
/// way a failed `v` does, and says so through [`note`]: it is a fact about the
/// view, so it gives way to whatever the run or the child has already put on the
/// footer.
///
/// The watcher is not told about any of this. The editor's own write set off
/// events that are sitting in [`Watched`](crate::session::Watched)'s channel
/// while this thread was blocked, so the loop may well read the tree once more a
/// round or two later — the reload here answers those events in fact, though not
/// in the policy's bookkeeping, and a second walk of a repository is a harmless
/// price for this function not having to reach into the watcher. Compare
/// `watched.caught_up`, which is how a *run* discharges the same debt: a run ends
/// inside the loop, where the watcher is, and this ends inside a keystroke.
fn came_back(app: &mut App, scope: &Scope, edited: &Path, showing: Option<&Path>) {
    // The tree it read is not kept: nothing here filters on a walk, and the
    // watcher's own filter is caught up by the reload it does for itself.
    let _ = reload_tree(app, scope);

    if showing != Some(edited) {
        return;
    }
    match view_file(edited) {
        Ok(Viewed { text, cut }) => app.refill_document(text.lines(), cut),
        // The engine's wording, flattened onto one line exactly as `view_press`
        // flattens it — the same failure, reached without a keystroke of its own.
        Err(error) => note(app, one_line(&error.to_string())),
    }
}

/// Which file this press would edit, or `None` for a press that has already been
/// answered.
///
/// Both refusals, and neither of them worded here. A run in flight is refused
/// exactly as [`scope_press`](crate::scoping::scope_press) refuses one, on the
/// progress line the reader is already watching; a row that is not a file is
/// refused by [`App::view_target`], in the words `v` uses, which is the point of
/// asking it rather than spelling a second sentence about directories.
///
/// The run is asked first so that a mid-run press over a directory says the
/// thing that is true of the whole session rather than the thing that is true of
/// the row.
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

/// What `$EDITOR`'s value names, or `None` when it names nothing runnable.
///
/// [`str::split_whitespace`] is the whole of the parsing, which answers the
/// awkward spellings by construction: runs of spaces, tabs, and leading or
/// trailing padding all collapse, because a shell that exports `EDITOR=" code
/// -w "` meant `code -w`. The first word out of it is the program and the rest
/// are its arguments; a value with no words in it has no program in it, and
/// comes back `None`.
///
/// A value that is not UTF-8 comes back `None` too, and lands in the same
/// footer line as an unset one. It could not be split — whitespace is a fact
/// about text and this is a bag of bytes — and the alternatives are worse than
/// refusing: running the whole thing as one unsplit program would mean
/// `EDITOR="code -w"` behaving differently depending on the encoding of the
/// bytes around it, and a refusal of its own would be a fourth sentence for a
/// situation nobody outside a broken locale reaches. Taken as read: the reader
/// is told `$EDITOR` names no editor warlock can run, which is true of it.
///
/// Takes the value rather than reading the variable, for the reason
/// `claude::or_default` is written that way: setting a real environment variable
/// is process-wide, racy against every other test on the runner and unsafe
/// besides, so the decision worth pinning is tested as the pure function it is,
/// and [`edit_press`] is left as the one line that reads an environment nobody
/// else can see.
fn editor_command(value: Option<&OsStr>) -> Option<Editor> {
    let mut words = value?.to_str()?.split_whitespace();
    let program = words.next()?.to_owned();
    Some(Editor {
        program,
        args: words.map(str::to_owned).collect(),
    })
}

/// Run the editor on `path` and wait for it, and say in one line if that went
/// badly.
///
/// Called with the terminal already given up, so everything about the child is
/// inherited rather than piped: it reads the keyboard warlock was reading and
/// writes to the screen warlock was drawing on, which is the only way an
/// interactive editor works at all. That is [`Command`]'s default, and it is
/// worth saying out loud next to `claude.rs`, where every stream is piped and
/// read by a thread apiece.
///
/// [`Command::status`] rather than a spawn and a wait of our own: it is the same
/// thing spelled once, and the waiting is the point — the event loop is blocked
/// here for as long as somebody is typing, so warlock is not drawing frames
/// behind an editor or draining a channel it cannot show.
///
/// The path goes last, after whatever `$EDITOR` carried, and as a path rather
/// than as a string: nothing here has to think about a file whose name contains
/// a space, a quote or a newline, because nothing here is building a command
/// line for a shell to take apart again.
///
/// `None` is an editor that exited zero, and it says nothing: a reader who has
/// just come back from their own editor can see that they have. The other two
/// each name the program warlock tried to run, because "it did not work" about a
/// program the reader named in their own environment is not something they can
/// act on.
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

/// The parts of the edit key that can be asked about without giving a terminal
/// away: what `$EDITOR` splits into, which presses are refused and in whose
/// words, and what a child that went wrong leaves on the footer.
///
/// Nothing here suspends anything, because the suspension is
/// [`TerminalGuard`](crate::terminal::TerminalGuard)'s and a test harness has no
/// terminal to give up. What that leaves is exactly the parts with decisions in
/// them.
#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use warlock_engine::{Node, NodeState, Tree};
    use warlock_tui::App;

    use super::{Editor, NO_EDITOR, came_back, edit_target, editor_command, run_editor};

    /// The line the last keystroke left on the footer, so that "the refusal did
    /// not go through the message line" is an assertion about a line that is
    /// really there.
    const LAST_KEY: &str = "something the last key said";

    /// A program no machine has, for the spawn that fails.
    const NOT_A_PROGRAM: &str = "warlock-test-no-such-editor-4c19be";

    /// The tree the app is built over, rooted at `root`.
    ///
    /// One documented directory holding a file each of the tests below presses
    /// on, and one directory with no document at all, which is the other half of
    /// what a directory row can be refused with.
    ///
    /// ```text
    /// <root>                          pacted, stale, documented
    /// └── crates                      unpacted, no document
    ///     └── crates/engine           pacted, fresh, documented
    ///         WARLOCK.md
    /// ```
    fn tree(root: &Path) -> Tree {
        let engine = root.join("crates/engine");
        Tree::new(
            Node::new(root, root.join("WARLOCK.md"), NodeState::PactedStale).with_children([
                Node::new(root.join("crates"), None::<PathBuf>, NodeState::Unpacted).with_children(
                    [
                        Node::new(&engine, engine.join("WARLOCK.md"), NodeState::PactedFresh)
                            .with_files([engine.join("WARLOCK.md")]),
                    ],
                ),
            ]),
        )
    }

    /// Where the fixture's repository is. Nothing under it is ever opened: every
    /// test here stops before the file would be read, so the path only has to be
    /// a path.
    fn root() -> PathBuf {
        PathBuf::from("/repo")
    }

    /// The app the event loop would hold, with the files shown, the row for
    /// `path` selected, and a line on the footer from the keystroke before this
    /// one.
    fn app_on(path: &Path) -> App {
        let mut app = App::from_tree(&tree(&root()));
        app.toggle_files();
        while app.selected_row().expect("the fixture has rows").path != path {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {}", path.display());
        }
        // After the selection, because moving the selection is what takes a
        // message down.
        app.set_message(LAST_KEY);
        app
    }

    /// What `$EDITOR` holding `value` splits into.
    fn split(value: &str) -> Option<Editor> {
        editor_command(Some(OsStr::new(value)))
    }

    /// An editor of `program` with `args`, as the split should produce it.
    fn editor(program: &str, args: &[&str]) -> Editor {
        Editor {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    #[test]
    fn the_first_word_is_the_program_and_the_rest_are_arguments() {
        // The two spellings the ticket names, which are the two shapes a real
        // `$EDITOR` takes: a bare program, and a program with switches that have
        // to arrive before the file.
        assert_eq!(split("nvim"), Some(editor("nvim", &[])));
        assert_eq!(split("code -w"), Some(editor("code", &["-w"])));
        assert_eq!(split("nvim -u NONE"), Some(editor("nvim", &["-u", "NONE"])));
        // A path is a first word like any other.
        assert_eq!(
            split("/usr/bin/emacs -nw"),
            Some(editor("/usr/bin/emacs", &["-nw"]))
        );
    }

    #[test]
    fn padding_and_runs_of_whitespace_are_not_arguments() {
        // What a shell leaves behind when somebody exports a value with a stray
        // space in it: no empty argument, and no program that is the empty
        // string.
        assert_eq!(split("  code -w"), Some(editor("code", &["-w"])));
        assert_eq!(split("code -w   "), Some(editor("code", &["-w"])));
        assert_eq!(split("code    -w"), Some(editor("code", &["-w"])));
        assert_eq!(split("\tcode\t-w\n"), Some(editor("code", &["-w"])));
        assert_eq!(
            split(" nvim  -u   NONE "),
            Some(editor("nvim", &["-u", "NONE"]))
        );
    }

    #[test]
    fn unset_empty_and_whitespace_are_one_answer_and_it_is_no_editor() {
        // One situation with one answer: there is no first word, so there is no
        // program, so nothing is spawned and no fallback is invented.
        assert_eq!(editor_command(None), None, "unset named an editor");
        assert_eq!(split(""), None, "an empty value named an editor");
        assert_eq!(split("   "), None, "spaces named an editor");
        assert_eq!(split("\t\n "), None, "whitespace named an editor");
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_text_is_the_same_answer() {
        // Bytes that are not UTF-8, which only Unix can hold in an environment:
        // there is nothing to split on whitespace, so it lands in the one
        // refusal rather than being run unsplit. Documented on `editor_command`.
        use std::os::unix::ffi::OsStrExt;

        assert_eq!(editor_command(Some(OsStr::from_bytes(&[0xff, 0xfe]))), None);
    }

    #[test]
    fn the_refusal_names_the_variable_and_says_how_to_set_it() {
        assert!(NO_EDITOR.contains("$EDITOR"), "{NO_EDITOR}");
        assert!(NO_EDITOR.contains("EDITOR=nvim"), "{NO_EDITOR}");
        assert!(!NO_EDITOR.contains('\n'), "the footer is one line");
        // No fallback and no second variable, said as an assertion because both
        // are decisions rather than omissions.
        assert!(!NO_EDITOR.contains("VISUAL"), "{NO_EDITOR}");
        assert!(!NO_EDITOR.contains("`vi`"), "{NO_EDITOR}");
    }

    #[test]
    fn a_file_row_hands_back_that_file_and_says_nothing() {
        let path = root().join("crates/engine/WARLOCK.md");
        let mut app = app_on(&path);
        let before = app.clone();

        assert_eq!(edit_target(&mut app, false), Some(path));
        assert_eq!(app, before, "resolving the row moved something");
        assert_eq!(app.message(), Some(LAST_KEY));
    }

    #[test]
    fn a_press_while_a_run_is_in_flight_suspends_nothing_and_says_so_on_the_progress_line() {
        let mut app = app_on(&root().join("crates/engine/WARLOCK.md"));
        // The run the press is going to bounce off, as the event loop would have
        // told the app about it.
        app.set_pact_in_flight(root().join("crates/tui"), 3, 12);
        let before = app.clone();

        assert_eq!(edit_target(&mut app, true), None, "a run was interrupted");

        // Saying so is the whole of it: the same app with the flag set and
        // nothing else moved, exactly as a second `p` and an `s` leave it.
        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
        assert!(
            app.pact_line()
                .is_some_and(|line| line.ends_with("— already running")),
            "the refusal is worded onto the line the reader is watching: {:?}",
            app.pact_line()
        );
        assert_eq!(
            app.message(),
            Some(LAST_KEY),
            "the refusal did not go through the message"
        );
    }

    #[test]
    fn leaning_on_the_edit_key_during_a_run_changes_nothing_further() {
        let mut app = app_on(&root().join("crates/engine/WARLOCK.md"));
        app.set_pact_in_flight(root().join("crates/tui"), 3, 12);

        assert_eq!(edit_target(&mut app, true), None);
        let after_one = app.clone();

        assert_eq!(edit_target(&mut app, true), None);
        assert_eq!(app, after_one, "pressing again changed something");
    }

    #[test]
    fn a_run_in_flight_is_answered_before_the_row_is_even_looked_at() {
        // The selection is on a directory, which `v` would refuse in its own
        // words. Mid-run it is the run that is answered: what is true of the
        // whole session outranks what is true of the row.
        let mut app = app_on(&root().join("crates"));
        app.set_pact_in_flight(root().join("crates/tui"), 3, 12);

        assert_eq!(edit_target(&mut app, true), None);

        assert_eq!(app.message(), Some(LAST_KEY), "the row was worded as well");
    }

    /// What `v` says about the row `path` is on, asked of the very method `e`
    /// asks — which is the assertion that there is one wording and not two.
    fn what_v_says(path: &Path) -> String {
        let mut app = app_on(path);
        assert_eq!(app.view_target(), None, "this row is not a refusal");
        app.message()
            .expect("a directory row is refused")
            .to_owned()
    }

    #[test]
    fn a_documented_directory_is_refused_in_the_words_v_uses() {
        let directory = root().join("crates/engine");
        let mut app = app_on(&directory);

        assert_eq!(edit_target(&mut app, false), None, "a directory was opened");

        let message = app.message().expect("a directory row is refused");
        assert_eq!(
            message,
            what_v_says(&directory),
            "the edit key spelled a refusal of its own"
        );
        assert!(message.contains("is a directory"), "{message}");
        // One keystroke away from what the reader wanted, so the refusal names
        // the document row beneath the directory.
        assert!(message.contains("WARLOCK.md"), "{message}");
    }

    #[test]
    fn an_undocumented_directory_is_refused_in_the_same_words_and_names_the_pact_key() {
        let directory = root().join("crates");
        let mut app = app_on(&directory);

        assert_eq!(edit_target(&mut app, false), None, "a directory was opened");

        let message = app.message().expect("a directory row is refused");
        assert_eq!(
            message,
            what_v_says(&directory),
            "the edit key spelled a refusal of its own"
        );
        // Nothing to point at, so it points at the key that would make something
        // to point at.
        assert!(message.contains("press p to pact it"), "{message}");
    }

    /// Editors that really run, for the two outcomes that are not silence.
    ///
    /// Unix only, because `/bin/sh` is where a program that exits how it is told
    /// to can be had without shipping a fixture — and a stand-in like this is
    /// deliberately not how warlock spells anything: the child below is built by
    /// the very [`run_editor`] that builds a real one, argument by argument, and
    /// the shell here is the *editor*, not a way of running it.
    #[cfg(unix)]
    mod outcomes {
        use std::path::Path;

        use super::{Editor, NOT_A_PROGRAM, run_editor};

        /// The file the stand-in is pointed at, which none of them opens.
        fn file() -> &'static Path {
            Path::new("/repo/crates/engine/WARLOCK.md")
        }

        /// A `/bin/sh` that does `script` and nothing else.
        fn stand_in(script: &str) -> Editor {
            Editor {
                program: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), script.to_owned()],
            }
        }

        #[test]
        fn an_editor_that_exits_zero_says_nothing() {
            assert_eq!(run_editor(&stand_in("exit 0"), file()), None);
        }

        #[test]
        fn a_non_zero_exit_is_one_line_naming_the_program() {
            let line = run_editor(&stand_in("exit 3"), file()).expect("a bad exit says so");

            assert!(line.contains("/bin/sh"), "{line}");
            assert!(!line.contains('\n'), "the footer is one line: {line}");
        }

        #[test]
        fn an_editor_that_will_not_start_is_one_line_naming_the_program() {
            let editor = Editor {
                program: NOT_A_PROGRAM.to_owned(),
                args: Vec::new(),
            };

            let line = run_editor(&editor, file()).expect("a spawn that failed says so");

            assert!(line.contains(NOT_A_PROGRAM), "{line}");
            assert!(line.contains("$EDITOR"), "{line}");
            assert!(!line.contains('\n'), "the footer is one line: {line}");
        }
    }

    /// The other half of the key: what warlock reads again once the screen is
    /// its own.
    ///
    /// Driven over a repository of the test's own under the temporary
    /// directory, because [`came_back`] is the half that touches disk — the
    /// files are really written, the manifest is really saved, the subtree is
    /// really hashed and the tree is really walked again. The edit is made by
    /// this thread rather than by a child, which is not a stand-in for
    /// anything: what these tests are about is the state of the disk when
    /// warlock takes the terminal back, and by then the editor has exited.
    mod back {
        use std::fs;
        use std::path::Path;
        use std::time::Instant;

        use tempfile::TempDir;
        use warlock_engine::{
            Loaded, Manifest, NodeState, PactEntry, load_tree, repository_root, subtree_hash,
        };
        use warlock_tui::{App, Chrome, Line};

        use super::came_back;
        use crate::session::Scope;

        /// Room for more lines than any file here has, so that what a test
        /// reads off the panel is the whole card rather than a screenful of it.
        /// One test narrows it, because parking a window needs one to park in.
        const PANEL: u16 = 400;

        /// The document the fixture's pact was granted over.
        const DOCUMENT: &str = "# The engine\n\nIt walks the tree and writes what it finds.\n";

        /// The same document as an editor left it: different bytes, so the
        /// subtree hashes differently and the lines on a card differ too.
        const REWRITTEN: &str = "# The engine\n\nRewritten in somebody else's editor.\n";

        /// A plain file beside it that nothing below ever edits — the other
        /// file a document card can be holding when `e` is pressed.
        const NOTES: &str = "one\ntwo\nthree\n";

        /// When the fixture's pact was granted. Nothing here reads it —
        /// freshness is the hash — and it is written because a grant is a hash
        /// *and* a time.
        const GRANTED_AT: &str = "2026-08-19T07:32:00Z";

        /// A repository of this test's own, pacted and green, removed when the
        /// test that made it ends.
        ///
        /// One documented directory holding the document and one plain file,
        /// and a manifest granting that directory over the bytes just written
        /// — so the directory loads fresh, and an edit under it has something
        /// to cost.
        fn a_repo() -> TempDir {
            let repo = tempfile::tempdir().expect("a temporary directory");
            let engine = repo.path().join("crates/engine");
            fs::create_dir_all(&engine).expect("the fixture's directories");
            fs::write(engine.join("WARLOCK.md"), DOCUMENT).expect("the document");
            fs::write(engine.join("notes.txt"), NOTES).expect("a plain file");
            // A load walks up looking for a `.git/` and refuses without one.
            // Nothing inside it is ever read: the walk skips hidden
            // directories, this one and `.warlock/` alike.
            fs::create_dir_all(repo.path().join(".git")).expect("the repository marker");
            fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("a HEAD");

            let hash = subtree_hash(&engine).expect("a directory just written hashes");
            Manifest::with_entries([PactEntry::new(
                repo.path(),
                &engine,
                engine.join("WARLOCK.md"),
            )
            .expect("a module inside the root")
            .with_grant(hash, GRANTED_AT)])
            .save(repo.path())
            .expect("a manifest that writes");
            repo
        }

        /// The app and the [`Scope`] the event loop would hold for `repo`,
        /// built the way `load_app` builds them.
        fn loaded(repo: &TempDir) -> (App, Scope) {
            let Loaded { tree, problems } =
                load_tree(repo.path()).expect("a scratch repository with a `.git/` loads");
            assert!(problems.is_empty(), "the fixture does not read cleanly");
            let repo_root =
                repository_root(tree.root_path()).expect("the load found a repository root");
            let mut app = App::from_tree(&tree);
            app.set_panel_height(PANEL);
            let scope = Scope {
                chrome: Chrome::of(&repo_root, tree.root_path()),
                root: tree.root_path().to_path_buf(),
                repo_root,
            };
            (app, scope)
        }

        /// The colour the app is showing for the node at `path`, or `None`
        /// when no row stands for it.
        fn state_of(app: &App, path: &Path) -> Option<NodeState> {
            app.rows()
                .iter()
                .find(|row| row.path == path)
                .map(|row| row.state)
        }

        /// What the panel is drawing at `now`, whichever card is showing: a
        /// document's own text, or the account's headings and clocked lines.
        fn shown(app: &App, now: Instant) -> Vec<String> {
            app.panel_lines(now)
                .into_iter()
                .map(|line| match line {
                    Line::Directory { path } => path.display().to_string(),
                    Line::Clocked { clock, text } => format!("{clock} {text}"),
                    Line::Summary { text } | Line::Text { text } => text,
                })
                .collect()
        }

        /// Whether the card on screen is the document one.
        ///
        /// Asked of what is drawn rather than of a flag, because what the
        /// tests below are about is what the reader is looking at: a document
        /// draws as text and an account never does.
        fn is_document(app: &App, now: Instant) -> bool {
            matches!(app.panel_lines(now).first(), Some(Line::Text { .. }))
        }

        /// `text` as the lines a card holding it draws.
        fn lines_of(text: &str) -> Vec<String> {
            text.lines().map(str::to_owned).collect()
        }

        /// Give `app` the account of a run, opened at `at`.
        ///
        /// One section and no entries, which is the smallest account that
        /// draws anything — and drawing something is the point: a card that is
        /// showing and empty cannot be told from the other one.
        fn with_an_account(app: &mut App, at: Instant) {
            app.start_account(at);
            app.account_mut()
                .expect("the press that started the run opened one")
                .open_section("crates/engine", at);
        }

        #[test]
        fn an_edit_that_changed_a_file_leaves_the_directory_yellow_with_no_further_keystroke() {
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let engine = repo.path().join("crates/engine");
            let edited = engine.join("WARLOCK.md");
            assert_eq!(
                state_of(&app, &engine),
                Some(NodeState::PactedFresh),
                "the fixture does not start green"
            );

            // What the editor did while warlock had no screen.
            fs::write(&edited, REWRITTEN).expect("the document rewrites");
            came_back(&mut app, &scope, &edited, None);

            // The tree was read again on the way in: the subtree hashes
            // differently now, so the row says so without the reader having to
            // press anything for it.
            assert_eq!(state_of(&app, &engine), Some(NodeState::PactedStale));
        }

        #[test]
        fn the_card_holding_the_edited_file_is_read_again_and_goes_on_showing() {
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let now = Instant::now();
            let edited = repo.path().join("crates/engine/WARLOCK.md");
            // A run behind the document, so that "the document is still
            // showing" is a claim about two cards rather than about the only
            // one there is.
            with_an_account(&mut app, now);
            app.show_document(lines_of(DOCUMENT), false);
            assert!(is_document(&app, now), "the fixture is not on the document");

            fs::write(&edited, REWRITTEN).expect("the document rewrites");
            came_back(&mut app, &scope, &edited, Some(&edited));

            assert!(
                is_document(&app, now),
                "the re-read took the reader off the document"
            );
            assert_eq!(
                shown(&app, now),
                lines_of(REWRITTEN),
                "the card is still holding what the editor opened"
            );
        }

        #[test]
        fn a_re_read_under_the_account_fills_the_card_behind_it_and_leaves_the_run_showing() {
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let now = Instant::now();
            let edited = repo.path().join("crates/engine/WARLOCK.md");
            with_an_account(&mut app, now);
            app.show_document(lines_of(DOCUMENT), false);
            app.swap_card();
            let account = shown(&app, now);
            assert!(!is_document(&app, now), "the fixture is not on the run");

            fs::write(&edited, REWRITTEN).expect("the document rewrites");
            came_back(&mut app, &scope, &edited, Some(&edited));

            // The panel is exactly where the reader left it: a file being
            // saved in an editor is not a reason to take a run off the screen.
            assert!(!is_document(&app, now), "the re-read flipped the panel");
            assert_eq!(shown(&app, now), account);
            // And the new lines are on the card behind it, waiting for the
            // swap the reader will ask for themselves.
            app.swap_card();
            assert_eq!(shown(&app, now), lines_of(REWRITTEN));
        }

        #[test]
        fn a_card_holding_another_file_is_left_exactly_as_it_was() {
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let now = Instant::now();
            let edited = repo.path().join("crates/engine/WARLOCK.md");
            let notes = repo.path().join("crates/engine/notes.txt");
            // A panel small enough to have a window to park, and a reader who
            // has parked it: a card read again for no reason would put them
            // back at the top of a file nobody changed.
            app.set_panel_height(2);
            app.show_document(lines_of(NOTES), false);
            app.scroll_panel_down(1);
            let before = shown(&app, now);
            let parked = app.panel_scroll_offset();

            fs::write(&edited, REWRITTEN).expect("the document rewrites");
            came_back(&mut app, &scope, &edited, Some(&notes));

            assert!(is_document(&app, now), "the re-read flipped the panel");
            assert_eq!(
                shown(&app, now),
                before,
                "a file this press never touched was read again"
            );
            assert_eq!(app.panel_scroll_offset(), parked, "the reader's line moved");
        }

        #[test]
        fn a_card_holding_nothing_is_the_same_as_a_card_holding_another_file() {
            // The state a session is in until its first `v`: nothing has been
            // read, so there is nothing to read again, and the edit is the
            // tree's business alone.
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let now = Instant::now();
            let edited = repo.path().join("crates/engine/WARLOCK.md");

            fs::write(&edited, REWRITTEN).expect("the document rewrites");
            came_back(&mut app, &scope, &edited, None);

            assert!(!app.has_document(), "a card nobody asked for was filled");
            assert!(!app.has_panel_content(), "the panel drew something");
            assert!(shown(&app, now).is_empty());
        }

        #[test]
        fn a_re_read_that_will_not_read_leaves_the_card_holding_what_it_held_and_says_so() {
            let repo = a_repo();
            let (mut app, scope) = loaded(&repo);
            let now = Instant::now();
            let edited = repo.path().join("crates/engine/WARLOCK.md");
            app.show_document(lines_of(DOCUMENT), false);

            // An editor that took the file with it, which is the same failure
            // a `v` over a vanished path meets.
            fs::remove_file(&edited).expect("the document goes");
            came_back(&mut app, &scope, &edited, Some(&edited));

            assert_eq!(
                shown(&app, now),
                lines_of(DOCUMENT),
                "the card that could not be read was emptied"
            );
            let message = app.message().expect("a read that failed says so");
            assert!(message.contains("WARLOCK.md"), "{message}");
            assert!(!message.contains('\n'), "the footer is one line: {message}");
        }
    }
}
