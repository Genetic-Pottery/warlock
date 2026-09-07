//! The view key. [`view_press`] is the two steps between
//! [`App::view_target`], which decides what the press means over the selected
//! row and words every refusal itself, and
//! [`App::show_document`](warlock_tui::App::show_document), which takes lines
//! and never a path.
//!
//! Two things here are choices rather than consequences. The press consults no
//! run and no manifest, so `v` means the same thing during a pact as outside
//! one: `p`, `r` and `s` refuse mid-run because a second run would race the
//! first for the same documents and the same manifest, and a read races
//! nothing. And nothing is cleared on the way in, so a read that fails costs
//! one line on the footer rather than the document a reader was looking at.

use std::path::PathBuf;

use warlock_engine::{Viewed, view_file};
use warlock_tui::App;

use crate::error::one_line;

// The file that is now on the document card, and `None` for a press that put
// nothing there — a row `App::view_target` refused, or a read that failed and
// left the card holding what it held before. It is handed back rather than kept
// on `App`: a path there would be a path something later had to open, which is
// why `App::show_document` takes lines. The loop keeps it so the edit key can
// ask whether the file `$EDITOR` rewrote is the one on the card.
pub(crate) fn view_press(app: &mut App) -> Option<PathBuf> {
    // Every row-level refusal leaves through here, having already said its
    // piece: there is one place that decides what this press means over a row,
    // and it is not this file.
    let path = app.view_target()?;

    match view_file(&path) {
        Ok(Viewed { text, cut }) => {
            app.show_document(text.lines(), cut);
            Some(path)
        }
        // The engine's own wording — it names the file and says which of the two
        // ways it went — flattened onto the footer's single line the way every
        // other non-fatal failure in this binary is. The panel is not touched:
        // a reader who could not read this file is still looking at whatever
        // they were looking at before they asked.
        Err(error) => {
            app.set_message(one_line(&error.to_string()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use tempfile::TempDir;
    use warlock_engine::{Node, NodeState, PER_FILE_BYTE_CAP, Tree, Viewed, view_file};
    use warlock_tui::{App, Line};

    use super::view_press;

    // The line the last keystroke left on the footer. A successful read is not
    // allowed to spend it: showing a file says nothing.
    const LAST_KEY: &str = "something the last key said";

    // Room for far more lines than any file below has, so what a test reads
    // back is the whole document rather than a screenful of it.
    const PANEL: u16 = 400;

    const DOCUMENT: &str = "# The engine\n\nIt walks the tree and writes what it finds.\n";

    // Writes every file `tree` lists except `gone.txt`, which is what a path
    // that vanished between the walk and the keystroke looks like.
    fn a_repo() -> TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let engine = repo.path().join("crates/engine");
        fs::create_dir_all(&engine).expect("the fixture's directories");
        fs::write(engine.join("WARLOCK.md"), DOCUMENT).expect("the document");
        fs::write(engine.join("notes.txt"), "one\ntwo\nthree\n").expect("a plain file");
        // Not text and not pretending to be: a PNG's first bytes, which stop
        // being UTF-8 at the second one.
        fs::write(
            engine.join("logo.png"),
            [0x89, b'P', b'N', b'G', 0x0d, 0xff, 0xfe],
        )
        .expect("some bytes that are not text");
        fs::write(engine.join("huge.txt"), over_the_cap()).expect("a file past the cap");
        repo
    }

    // The line width is a fraction of the cap rather than a fixed number of
    // characters: a cap raised in a one-line diff would otherwise turn this into
    // a file with more lines than `PANEL` has rows, and the tests below would
    // fail on their own scaffolding instead of on what they are about.
    fn over_the_cap() -> String {
        let cap = usize::try_from(PER_FILE_BYTE_CAP).expect("the cap fits in memory");
        let width = cap / 128;
        let mut text = String::new();
        for line in 0.. {
            let number = format!("{line:04} ");
            text.push_str(&number);
            text.push_str(&"x".repeat(width - number.len()));
            text.push('\n');
            if text.len() > cap + 4096 {
                break;
            }
        }
        text
    }

    /// The tree the app is built over, rooted at `root`.
    ///
    /// ```text
    /// <root>                          pacted, stale, documented
    /// └── crates                      unpacted, no document
    ///     └── crates/engine           pacted, fresh, documented
    ///         WARLOCK.md, notes.txt, logo.png, huge.txt, gone.txt
    /// ```
    fn tree(root: &Path) -> Tree {
        let engine = root.join("crates/engine");
        Tree::new(
            Node::new(root, root.join("WARLOCK.md"), NodeState::PactedStale).with_children([
                Node::new(root.join("crates"), None::<PathBuf>, NodeState::Unpacted).with_children(
                    [
                        Node::new(&engine, engine.join("WARLOCK.md"), NodeState::PactedFresh)
                            .with_files(
                                [
                                    "WARLOCK.md",
                                    "notes.txt",
                                    "logo.png",
                                    "huge.txt",
                                    "gone.txt",
                                ]
                                .map(|file| engine.join(file)),
                            ),
                    ],
                ),
            ]),
        )
    }

    // The app the event loop would hold for `root`: files shown, the row for
    // `path` selected, and a line on the footer from the keystroke before.
    fn app_on(root: &Path, path: &Path) -> App {
        let mut app = App::from_tree(&tree(root));
        app.toggle_files();
        app.panel_mut().set_height(PANEL);
        while app.selected_row().expect("the fixture has rows").path != path {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {}", path.display());
        }
        // After the selection, because moving the selection is what takes a
        // message down: every assertion below about the footer needs there to be
        // a line on it already.
        app.set_message(LAST_KEY);
        app
    }

    fn app_on_file(root: &Path, file: &str) -> App {
        app_on(root, &root.join("crates/engine").join(file))
    }

    // A document draws as text and nothing else — no clock, no heading, no
    // summary — so anything else here is the panel showing the wrong card.
    fn panel_text(app: &App) -> Vec<String> {
        app.panel()
            .window(Instant::now())
            .into_iter()
            .map(|line| match line {
                Line::Text { text } => text,
                other => panic!("a document draws as text, not as {other:?}"),
            })
            .collect()
    }

    #[test]
    fn v_on_a_file_row_puts_that_files_lines_in_the_panel() {
        let repo = a_repo();
        let mut app = app_on_file(repo.path(), "WARLOCK.md");

        let read = view_press(&mut app);

        // The file that is now on the card, said out loud: the app is never told
        // which file it is holding, so the press has to say.
        assert_eq!(read, Some(repo.path().join("crates/engine/WARLOCK.md")));
        assert!(app.panel().has_document(), "nothing was read");
        assert_eq!(
            panel_text(&app),
            [
                "# The engine",
                "",
                "It walks the tree and writes what it finds."
            ],
            "one row per line of the file, from its first"
        );
        // From the top and not following: a file is read from its first line.
        assert_eq!(app.panel().scroll_offset(), 0);
        assert!(!app.panel().follows());
        // A successful read says nothing: the line the last keystroke left is
        // still the line on the footer.
        assert_eq!(app.message(), Some(LAST_KEY));
    }

    #[test]
    fn a_file_that_is_not_a_document_is_read_the_same_way() {
        // Section 5 is not bent for this key: a `WARLOCK.md` is an ordinary file
        // row, and so an ordinary file is a readable one.
        let repo = a_repo();
        let mut app = app_on_file(repo.path(), "notes.txt");

        let read = view_press(&mut app);

        assert_eq!(read, Some(repo.path().join("crates/engine/notes.txt")));
        assert_eq!(panel_text(&app), ["one", "two", "three"]);
    }

    #[test]
    fn v_on_a_documented_directory_reads_nothing_and_names_the_document_row() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), &repo.path().join("crates/engine"));
        let mut before = app.clone();

        // Nothing was read, so nothing is named: the card is holding whatever it
        // held before this press.
        assert_eq!(view_press(&mut app), None);

        let message = app.message().expect("a directory row is refused");
        assert!(message.contains("is a directory"), "{message}");
        assert!(message.contains("WARLOCK.md"), "{message}");
        assert!(!app.panel().has_content(), "a directory drew something");
        // The message is the whole of what the press changed.
        before.set_message(message);
        assert_eq!(app, before, "refusing a directory moved something else");
    }

    #[test]
    fn v_on_an_undocumented_directory_reads_nothing_and_names_the_pact_key() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), &repo.path().join("crates"));
        let mut before = app.clone();

        assert_eq!(view_press(&mut app), None);

        let message = app.message().expect("a directory row is refused");
        assert!(message.contains("is a directory"), "{message}");
        // Nothing to point at, so it points at the key that would make
        // something to point at.
        assert!(message.contains("press p to pact it"), "{message}");
        assert!(!app.panel().has_content(), "a directory drew something");
        before.set_message(message);
        assert_eq!(app, before, "refusing a directory moved something else");
    }

    #[test]
    fn a_file_past_the_cap_is_shown_up_to_the_cap_and_says_it_was_cut() {
        let repo = a_repo();
        let mut app = app_on_file(repo.path(), "huge.txt");

        assert_eq!(
            view_press(&mut app),
            Some(repo.path().join("crates/engine/huge.txt")),
            "a read the cap cut short is still a file on the card"
        );

        let drawn = panel_text(&app);
        // Exactly the text the engine read under the one cap, split into rows,
        // and one line more: the sentence about the cut, which is the only line
        // in the panel the file did not write.
        let Viewed { text, cut } = view_file(repo.path().join("crates/engine/huge.txt"))
            .expect("the fixture's huge file reads");
        assert!(cut, "the fixture is meant to be past the cap");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            drawn.len(),
            lines.len() + 1,
            "one line more than the file's"
        );
        assert_eq!(drawn[..lines.len()], lines[..]);
        let last = drawn.last().expect("a cut read says so");
        assert!(last.contains("cut"), "{last}");
        // And what was shown is a prefix of the file, at the cap: what a reader
        // sees is what a model would have been sent under the same cap.
        let whole = fs::read_to_string(repo.path().join("crates/engine/huge.txt"))
            .expect("the fixture's huge file is text");
        assert!(whole.starts_with(&text), "the panel showed something else");
        assert!(text.len() <= usize::try_from(PER_FILE_BYTE_CAP).expect("a few kilobytes"));
    }

    // The panel really holds a document, so that "the panel is exactly as it
    // was" is an assertion about something a reader would have lost rather than
    // about emptiness.
    fn app_holding_a_document_on(root: &Path, file: &Path) -> App {
        let mut app = app_on_file(root, "WARLOCK.md");
        assert!(view_press(&mut app).is_some(), "the fixture read nothing");
        assert!(app.panel().has_document(), "the fixture read nothing");
        select(&mut app, file);
        // After the selection, because moving the selection is what takes a
        // message down.
        app.set_message(LAST_KEY);
        app
    }

    // The assertions the three failures share: one line on the footer naming the
    // file, the panel as it was, and nothing else moved.
    fn assert_failed_read(app: &App, before: &App, shown: &[String], names: &str) {
        let message = app.message().expect("a read that failed says so");
        assert!(message.contains(names), "{message}");
        assert_ne!(message, LAST_KEY, "the failure said nothing");
        assert_eq!(panel_text(app), shown, "the panel lost what it was holding");

        let mut expected = before.clone();
        expected.set_message(message);
        assert_eq!(app, &expected, "the failed read moved something else");
    }

    #[test]
    fn a_file_that_is_not_text_says_so_and_leaves_the_panel_exactly_as_it_was() {
        let repo = a_repo();
        let mut app =
            app_holding_a_document_on(repo.path(), &repo.path().join("crates/engine/logo.png"));
        let shown = panel_text(&app);
        let before = app.clone();

        // A read that failed names no file: the card is still holding the one it
        // was holding, so the caller has nothing new to remember.
        assert_eq!(view_press(&mut app), None);

        assert_failed_read(&app, &before, &shown, "logo.png");
        let message = app.message().expect("a file that is not text says so");
        assert!(message.contains("not text"), "{message}");
        // Nothing was decoded, so there is nothing to have decoded badly.
        assert!(
            !panel_text(&app)
                .iter()
                .any(|line| line.contains('\u{fffd}')),
            "a replacement character was drawn"
        );
    }

    #[test]
    fn a_path_that_has_vanished_says_so_and_leaves_the_panel_exactly_as_it_was() {
        // `gone.txt` is in the walk and not on disk, which is what a file
        // deleted in another window since the tree was loaded looks like.
        let repo = a_repo();
        let mut app =
            app_holding_a_document_on(repo.path(), &repo.path().join("crates/engine/gone.txt"));
        let shown = panel_text(&app);
        let before = app.clone();

        assert_eq!(view_press(&mut app), None);

        assert_failed_read(&app, &before, &shown, "gone.txt");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_says_so_and_leaves_the_panel_exactly_as_it_was() {
        // The permissions saying no, which is the other half of unreadable.
        // Unix only, because that is where a mode of `0o000` means anything —
        // and skipped rather than failed for the run that is `root`, to whom it
        // does not.
        use std::os::unix::fs::PermissionsExt;

        let repo = a_repo();
        let secret = repo.path().join("crates/engine/notes.txt");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000))
            .expect("the fixture's permissions");
        if fs::read(&secret).is_ok() {
            return;
        }
        let mut app = app_holding_a_document_on(repo.path(), &secret);
        let shown = panel_text(&app);
        let before = app.clone();

        assert_eq!(view_press(&mut app), None);

        assert_failed_read(&app, &before, &shown, "notes.txt");
    }

    #[test]
    fn the_next_keystroke_is_answered_normally_after_a_read_that_failed() {
        // None of the three failures is fatal: the loop goes round again, and
        // the very next press reads a file into the panel as though nothing had
        // happened.
        let repo = a_repo();
        let mut app = app_on_file(repo.path(), "logo.png");

        assert_eq!(view_press(&mut app), None);
        assert!(!app.panel().has_content(), "the failed read drew something");

        let notes = repo.path().join("crates/engine/notes.txt");
        select(&mut app, &notes);
        assert_eq!(view_press(&mut app), Some(notes));

        assert_eq!(panel_text(&app), ["one", "two", "three"]);
    }

    #[test]
    fn reading_a_file_writes_no_byte_of_it() {
        let repo = a_repo();
        let path = repo.path().join("crates/engine/WARLOCK.md");
        let before = fs::read(&path).expect("the fixture's document");
        let mut app = app_on_file(repo.path(), "WARLOCK.md");

        assert_eq!(view_press(&mut app), Some(path.clone()));
        assert_eq!(view_press(&mut app), Some(path.clone()));

        assert_eq!(
            fs::read(&path).expect("the document is still there"),
            before,
            "the view key wrote to the file it read"
        );
    }

    #[test]
    fn a_second_press_reads_the_file_again_from_disk() {
        let repo = a_repo();
        let path = repo.path().join("crates/engine/WARLOCK.md");
        let mut app = app_on_file(repo.path(), "WARLOCK.md");

        assert_eq!(view_press(&mut app), Some(path.clone()));
        assert_eq!(panel_text(&app)[0], "# The engine");

        // The file rewritten under the reader, as a pass would rewrite it.
        fs::write(&path, "# Rewritten\n\nBy something else.\n").expect("the document rewrites");
        assert_eq!(view_press(&mut app), Some(path.clone()));

        assert_eq!(
            panel_text(&app),
            ["# Rewritten", "", "By something else."],
            "the second press showed what the first one read"
        );
    }

    fn select(app: &mut App, path: &Path) {
        app.select_first();
        while app.selected_row().expect("the fixture has rows").path != path {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {}", path.display());
        }
    }
}
