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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, io};

use warlock_tui::App;

use crate::error::one_line;
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
pub(crate) fn edit_press(
    app: &mut App,
    guard: &mut TerminalGuard,
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
    Ok(())
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

    use super::{Editor, NO_EDITOR, edit_target, editor_command, run_editor};

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
}
