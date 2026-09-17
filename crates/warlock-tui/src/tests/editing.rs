use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use warlock_engine::{Node, NodeState, Tree};
use warlock_tui::App;

use super::{Editor, NO_EDITOR, came_back, edit_target, editor_command, run_editor};

// The line the last keystroke left on the footer, so "the refusal did not go
// through the message line" is an assertion about a line that is really there.
const LAST_KEY: &str = "something the last key said";

// A program no machine has, for the spawn that fails.
const NOT_A_PROGRAM: &str = "warlock-test-no-such-editor-4c19be";

// One documented directory holding a file each test presses on, and one
// directory with no document at all, which is the other half of what a directory
// row can be refused with.
//
//     <root>                          pacted, stale, documented
//     └── crates                      unpacted, no document
//         └── crates/engine           pacted, fresh, documented
//             WARLOCK.md
fn tree(root: &Path) -> Tree {
    let engine = root.join("crates/engine");
    Tree::new(
        Node::new(root, root.join("WARLOCK.md"), NodeState::PactedStale).with_children([
            Node::new(root.join("crates"), None::<PathBuf>, NodeState::Unpacted).with_children([
                Node::new(&engine, engine.join("WARLOCK.md"), NodeState::PactedFresh)
                    .with_files([engine.join("WARLOCK.md")]),
            ]),
        ]),
    )
}

// Nothing under it is ever opened: every test here stops before the file would
// be read, so the path only has to be a path.
fn root() -> PathBuf {
    PathBuf::from("/repo")
}

// The app the event loop would hold, with the row for `path` selected and a line
// on the footer from the keystroke before this one.
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

fn split(value: &str) -> Option<Editor> {
    editor_command(Some(OsStr::new(value)))
}

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

// Asked of the very method `e` asks, which is the assertion that there is one
// wording and not two.
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

// Unix only, because `/bin/sh` is where a program that exits how it is told to
// can be had without shipping a fixture. Not how warlock spells anything: the
// child is built by the very `run_editor` that builds a real one, and the shell
// here is the *editor* rather than a way of running one.
#[cfg(unix)]
mod outcomes {
    use std::path::Path;

    use super::{Editor, NOT_A_PROGRAM, editor, run_editor};

    fn file() -> &'static Path {
        Path::new("/repo/crates/engine/WARLOCK.md")
    }

    // A `/bin/sh` that does `script` and nothing else.
    fn stand_in(script: &str) -> Editor {
        editor("/bin/sh", &["-c", script])
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
        let line =
            run_editor(&editor(NOT_A_PROGRAM, &[]), file()).expect("a spawn that failed says so");

        assert!(line.contains(NOT_A_PROGRAM), "{line}");
        assert!(line.contains("$EDITOR"), "{line}");
        assert!(!line.contains('\n'), "the footer is one line: {line}");
    }
}

// Driven over a repository of the test's own, because `came_back` is the half
// that touches disk: the files are really written, the manifest really saved,
// the subtree really hashed and the tree really walked again. The edit is made
// by this thread rather than by a child, which stands in for nothing — what
// these tests are about is the state of the disk when warlock takes the terminal
// back, and by then the editor has exited.
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

    // Room for more lines than any file here has, so what a test reads off the panel
    // is the whole card. One test narrows it, because parking a window needs one to
    // park in.
    const PANEL: u16 = 400;

    const DOCUMENT: &str = "# The engine\n\nIt walks the tree and writes what it finds.\n";

    // The same document as an editor left it: different bytes, so the subtree hashes
    // differently and the lines on a card differ too.
    const REWRITTEN: &str = "# The engine\n\nRewritten in somebody else's editor.\n";

    // A plain file beside it that nothing below ever edits — the other file a
    // document card can be holding when `e` is pressed.
    const NOTES: &str = "one\ntwo\nthree\n";

    // Nothing here reads it — freshness is the hash — and it is written because a
    // grant is a hash *and* a time.
    const GRANTED_AT: &str = "2026-08-19T07:32:00Z";

    // One documented directory holding the document and one plain file, and a
    // manifest granting that directory over the bytes just written, so the directory
    // loads fresh and an edit under it has something to cost.
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
        Manifest::with_entries([
            PactEntry::new(repo.path(), &engine, engine.join("WARLOCK.md"))
                .expect("a module inside the root")
                .with_grant(hash, GRANTED_AT),
        ])
        .save(repo.path())
        .expect("a manifest that writes");
        repo
    }

    fn loaded(repo: &TempDir) -> (App, Scope) {
        let Loaded { tree, problems, .. } =
            load_tree(repo.path()).expect("a scratch repository with a `.git/` loads");
        assert!(problems.is_empty(), "the fixture does not read cleanly");
        let repo_root =
            repository_root(tree.root_path()).expect("the load found a repository root");
        let mut app = App::from_tree(&tree);
        app.panel_mut().set_height(PANEL);
        let scope = Scope {
            chrome: Chrome::of(&repo_root, tree.root_path()),
            root: tree.root_path().to_path_buf(),
            repo_root,
        };
        (app, scope)
    }

    fn state_of(app: &App, path: &Path) -> Option<NodeState> {
        app.rows()
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.state)
    }

    fn shown(app: &App, now: Instant) -> Vec<String> {
        app.panel()
            .window(now)
            .into_iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text,
            })
            .collect()
    }

    fn is_document(app: &App, now: Instant) -> bool {
        matches!(app.panel().window(now).first(), Some(Line::Text { .. }))
    }

    fn lines_of(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn with_an_account(app: &mut App, at: Instant) {
        app.start_account(at);
        app.panel_mut()
            .account_mut()
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
        came_back(&mut app, &scope, &mut Manifest::new(), &edited, None);

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
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

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
        // Round to the run, past the conversation the field is drawn under.
        app.swap_card();
        app.swap_card();
        let account = shown(&app, now);
        assert!(!is_document(&app, now), "the fixture is not on the run");

        fs::write(&edited, REWRITTEN).expect("the document rewrites");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

        // The panel is exactly where the reader left it: a file being
        // saved in an editor is not a reason to take a run off the screen.
        assert!(!is_document(&app, now), "the re-read flipped the panel");
        assert_eq!(shown(&app, now), account);
        // And the new lines are on the card behind it, waiting for the
        // swap the reader will ask for themselves — the file is the very
        // next card round from the run.
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
        app.panel_mut().set_height(2);
        app.show_document(lines_of(NOTES), false);
        app.scroll_panel_down(1);
        let before = shown(&app, now);
        let parked = app.panel().scroll_offset();

        fs::write(&edited, REWRITTEN).expect("the document rewrites");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&notes),
        );

        assert!(is_document(&app, now), "the re-read flipped the panel");
        assert_eq!(
            shown(&app, now),
            before,
            "a file this press never touched was read again"
        );
        assert_eq!(
            app.panel().scroll_offset(),
            parked,
            "the reader's line moved"
        );
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
        came_back(&mut app, &scope, &mut Manifest::new(), &edited, None);

        assert!(
            !app.panel().has_document(),
            "a card nobody asked for was filled"
        );
        assert!(!app.panel().has_content(), "the panel drew something");
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
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

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
