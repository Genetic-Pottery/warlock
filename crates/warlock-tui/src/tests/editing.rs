use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::Size;
use warlock_engine::{Manifest, Node, Tree};

use super::{Editor, NO_EDITOR, came_back, edit_press, editor_command, run_editor};
use crate::app::{App, Chrome};
use crate::screen::Screen;
use crate::session::Scope;
use crate::settings::Settings;

// The line the last keystroke left on the footer, so "the refusal did not go
// through the message line" is an assertion about a line that is really there.
const LAST_KEY: &str = "something the last key said";

// A program no machine has, for the spawn that fails.
const NOT_A_PROGRAM: &str = "warlock-test-no-such-editor-4c19be";

//     <root>                          unscoped
//     └── crates                      scoped `platform`
//         └── crates/engine           covered by `platform`
//             lib.rs
fn tree(root: &Path) -> Tree {
    let engine = root.join("crates/engine");
    Tree::new(
        Node::new(root).with_children([Node::new(root.join("crates"))
            .with_scope(Some("platform".to_owned()))
            .with_children([Node::new(&engine)
                .with_covering(Some("platform".to_owned()))
                .with_files([engine.join("lib.rs")])])]),
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

// A screen that records whether it was given up and draws nothing: a refused
// press must never reach `suspended`.
#[derive(Default)]
struct Untouched {
    suspensions: usize,
}

impl Screen for Untouched {
    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(80, 24))
    }

    fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, _render: F) -> io::Result<()> {
        Ok(())
    }

    fn suspended<T, F: FnOnce() -> T>(&mut self, _mouse: bool, body: F) -> io::Result<T> {
        self.suspensions += 1;
        Ok(body())
    }

    fn report_mouse(&mut self, _on: bool) -> io::Result<()> {
        Ok(())
    }
}

fn scope() -> Scope {
    Scope {
        chrome: Chrome::of(root(), root()),
        root: root(),
        settings: Settings::at(None, root()),
        repo_root: root(),
    }
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
fn a_directory_is_refused_in_the_words_v_uses_and_the_screen_is_never_given_up() {
    let directory = root().join("crates/engine");
    let mut app = app_on(&directory);
    let mut screen = Untouched::default();

    edit_press(
        &mut app,
        &mut screen,
        &scope(),
        &mut Manifest::new(),
        None,
        false,
    )
    .expect("a refusal never touches the terminal");

    assert_eq!(screen.suspensions, 0, "a directory was opened");
    let message = app.message().expect("a directory row is refused");
    assert_eq!(
        message,
        what_v_says(&directory),
        "the edit key spelled a refusal of its own"
    );
    assert!(message.contains("is a directory"), "{message}");
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
        Path::new("/repo/crates/engine/lib.rs")
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
// that touches disk: the files are really written and the tree really walked
// again. The edit is made by this thread rather than by a child, which stands in
// for nothing — what these tests are about is the state of the disk when warlock
// takes the terminal back, and by then the editor has exited.
mod back {
    use std::fs;
    use std::time::Instant;

    use tempfile::TempDir;
    use warlock_engine::{Loaded, Manifest, load_tree, repository_root};

    use super::came_back;
    use crate::account::Line;
    use crate::app::{App, Chrome};
    use crate::panel::Showing;
    use crate::session::Scope;
    use crate::settings::Settings;

    // Room for more lines than any file here has, so what a test reads off the panel
    // is the whole card. One test narrows it, because parking a window needs one to
    // park in.
    const PANEL: u16 = 400;

    const SOURCE: &str = "//! The engine\n\npub fn walk() {}\n";

    // The same file as an editor left it: different bytes, so the lines on a card
    // differ too.
    const REWRITTEN: &str = "//! The engine\n\nRewritten in somebody else's editor.\n";

    // A plain file beside it that nothing below ever edits — the other file a
    // document card can be holding when `e` is pressed.
    const NOTES: &str = "one\ntwo\nthree\n";

    fn a_repo() -> TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let engine = repo.path().join("crates/engine");
        fs::create_dir_all(&engine).expect("the fixture's directories");
        fs::write(engine.join("lib.rs"), SOURCE).expect("the source file");
        fs::write(engine.join("notes.txt"), NOTES).expect("a plain file");
        // A load walks up looking for a `.git/` and refuses without one.
        // Nothing inside it is ever read: the walk skips hidden
        // directories, this one and `.warlock/` alike.
        fs::create_dir_all(repo.path().join(".git")).expect("the repository marker");
        fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("a HEAD");
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
            settings: Settings::at(None, &repo_root),
            repo_root,
        };
        (app, scope)
    }

    fn shown(app: &App, now: Instant) -> Vec<String> {
        app.panel()
            .window(now)
            .into_iter()
            .map(|line| match line {
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text,
            })
            .collect()
    }

    fn is_document(app: &App) -> bool {
        app.panel().showing() == Showing::Document
    }

    fn lines_of(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_file_the_editor_wrote_beside_the_edited_one_is_on_the_tree_with_no_further_keystroke() {
        let repo = a_repo();
        let (mut app, scope) = loaded(&repo);
        app.toggle_files();
        let engine = repo.path().join("crates/engine");
        let edited = engine.join("lib.rs");
        let written = engine.join("lib.rs.orig");
        assert!(
            app.rows().iter().all(|row| row.path != written),
            "the fixture already lists the new file"
        );

        // What the editor did while warlock had no screen.
        fs::write(&written, SOURCE).expect("a backup the editor left");
        came_back(&mut app, &scope, &mut Manifest::new(), &edited, None);

        // The tree was read again on the way in.
        assert!(
            app.rows().iter().any(|row| row.path == written),
            "the reload missed what the editor wrote"
        );
    }

    #[test]
    fn the_card_holding_the_edited_file_is_read_again_and_goes_on_showing() {
        let repo = a_repo();
        let (mut app, scope) = loaded(&repo);
        let now = Instant::now();
        let edited = repo.path().join("crates/engine/lib.rs");
        // A line on the thread behind the document, so that "the document is
        // still showing" is a claim about two cards rather than about the only
        // one with anything on it.
        app.panel_mut().note("a note on the thread", now);
        app.show_document(lines_of(SOURCE), false);
        assert!(is_document(&app), "the fixture is not on the document");

        fs::write(&edited, REWRITTEN).expect("the file rewrites");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

        assert!(
            is_document(&app),
            "the re-read took the reader off the document"
        );
        assert_eq!(
            shown(&app, now),
            lines_of(REWRITTEN),
            "the card is still holding what the editor opened"
        );
    }

    #[test]
    fn a_re_read_under_the_thread_fills_the_card_behind_it_and_leaves_the_thread_showing() {
        let repo = a_repo();
        let (mut app, scope) = loaded(&repo);
        let now = Instant::now();
        let edited = repo.path().join("crates/engine/lib.rs");
        app.panel_mut().note("a note on the thread", now);
        app.show_document(lines_of(SOURCE), false);
        app.swap_card();
        let thread = shown(&app, now);
        assert!(!is_document(&app), "the fixture is not on the thread");

        fs::write(&edited, REWRITTEN).expect("the file rewrites");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

        // The panel is exactly where the reader left it: a file being saved in
        // an editor is not a reason to take the thread off the screen.
        assert!(!is_document(&app), "the re-read flipped the panel");
        assert_eq!(shown(&app, now), thread);
        // And the new lines are on the card behind it, waiting for the swap
        // the reader will ask for themselves.
        app.swap_card();
        assert_eq!(shown(&app, now), lines_of(REWRITTEN));
    }

    #[test]
    fn a_card_holding_another_file_is_left_exactly_as_it_was() {
        let repo = a_repo();
        let (mut app, scope) = loaded(&repo);
        let now = Instant::now();
        let edited = repo.path().join("crates/engine/lib.rs");
        let notes = repo.path().join("crates/engine/notes.txt");
        // A panel small enough to have a window to park, and a reader who
        // has parked it: a card read again for no reason would put them
        // back at the top of a file nobody changed.
        app.panel_mut().set_height(2);
        app.show_document(lines_of(NOTES), false);
        app.scroll_panel_down(1);
        let before = shown(&app, now);
        let parked = app.panel().scroll_offset();

        fs::write(&edited, REWRITTEN).expect("the file rewrites");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&notes),
        );

        assert!(is_document(&app), "the re-read flipped the panel");
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
        let edited = repo.path().join("crates/engine/lib.rs");

        fs::write(&edited, REWRITTEN).expect("the file rewrites");
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
        let edited = repo.path().join("crates/engine/lib.rs");
        app.show_document(lines_of(SOURCE), false);

        // An editor that took the file with it, which is the same failure
        // a `v` over a vanished path meets.
        fs::remove_file(&edited).expect("the file goes");
        came_back(
            &mut app,
            &scope,
            &mut Manifest::new(),
            &edited,
            Some(&edited),
        );

        assert_eq!(
            shown(&app, now),
            lines_of(SOURCE),
            "the card that could not be read was emptied"
        );
        let message = app.message().expect("a read that failed says so");
        assert!(message.contains("lib.rs"), "{message}");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
    }
}
