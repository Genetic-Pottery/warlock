//! The view key, from the keystroke to the lines in the panel.
//!
//! One press, one read, one thing on screen: [`view_press`] asks the app which
//! file the selection is on, reads it, hands the lines over and says which file
//! they came out of — the one thing about the panel the app is deliberately not
//! told, since a path on it would be a path something later had to open. It is
//! the
//! smallest of the three key modules — [`mod@crate::pacting`] runs a thread and
//! [`mod@crate::scoping`] writes a file — because reading is the smallest thing
//! a key here can ask for.
//!
//! ## Nothing is written, and nothing is a run
//!
//! Warlock has never shown a file, and the reason to start is section 6: the
//! judgement about a document is the reader's, and a colour is not a document.
//! What that costs is one capped read on the event loop's own thread. There is
//! no worker, no channel, no account, no progress line and no reload — a file
//! read is over inside a frame, and re-reading the tree afterwards would walk a
//! repository to arrive at the tree already on screen, since nothing about it
//! changed.
//!
//! It is not a write either, in any sense: [`view_file`] opens the file, reads
//! at most one byte past the cap, and closes it. The workspace's writers are
//! still the pact, the refresh, the manifest, the scope key and `warlock init`,
//! and this module is not among them.
//!
//! ## The app words the row, this file does the filesystem
//!
//! [`scope_press`](crate::scoping::scope_press)'s shape exactly.
//! [`App::view_target`] decides what the press means over the row the selection
//! is on and words every row-level refusal itself — a directory with a document
//! names that document, a directory without one names `p` — and a `None` from it
//! is a press that has already been answered. Only the reading is here, because
//! [`App`] opens nothing: what crosses back into it is the file's lines, never
//! its path.
//!
//! ## A run in flight is not a reason to refuse
//!
//! Unlike `p`, `r` and `s`, this key does not consult the run. Those three
//! refuse mid-run because a second run would race the first for the same
//! documents and the same manifest; a read races nothing, writes nothing and is
//! done before the next frame. So `v` means the same thing during a pact as
//! outside one, which is also what [`action_for`](crate::input::action_for)
//! says about it. The panel is a shared surface, but a shared surface with two
//! cards on it: a document shows over a running account rather than taking the
//! slot from it, so the run goes on writing its own card behind the file. The
//! key that swaps the two back and forth is not here.
//!
//! ## A failed read is a line, not an end
//!
//! None of the three ways a read can fail is fatal and none of them touches the
//! panel: an unreadable file, a path that has vanished since the walk listed it
//! and a file that is not text each put one line on
//! [`App::message`](warlock_tui::App::message) — where the pact and scope keys'
//! refusals already go — and leave whatever the panel was holding exactly as it
//! was. Blanking the panel on the way in would mean a failed read taking down
//! the document a reader was looking at, so nothing is cleared until there are
//! lines to put there.

use std::path::PathBuf;

use warlock_engine::{Viewed, view_file};
use warlock_tui::App;

use crate::error::one_line;

/// What one press of the view key comes to: the selected file's lines in the
/// panel, or one line in the footer saying why not.
///
/// Answers nothing itself about which row can be read — that is
/// [`App::view_target`]'s, which has already put its sentence on
/// [`App::message`](warlock_tui::App::message) by the time `None` gets back here
/// — and decides nothing about how much of a file there is to show, which is
/// [`view_file`]'s and the one cap it reads under. What is left is the two
/// steps between them: open the file the app named, and hand over what came
/// back.
///
/// The read happens on every press. A second `v` on the same row opens the file
/// again, so a document a pass has just rewritten under a reader is one
/// keystroke away from being the one on screen — there is no cache to go stale
/// and no path kept to be opened later.
///
/// The cut is passed on as the fact it is. [`Viewed::cut`] says the cap stopped
/// the read short and [`App::show_document`](warlock_tui::App::show_document)
/// says so in the panel's own words: the engine adds no line to the text, and
/// this function invents none.
///
/// Splitting the text into lines is [`str::lines`] and nothing more elaborate:
/// one line of the file is one row of the panel, `\r\n` and `\n` alike, and a
/// file's trailing newline is not a blank row at the end of it.
///
/// Takes no run and no manifest: see the module docs for why a read is neither
/// refused mid-run nor followed by a reload.
///
/// What comes back is the file that is now on the document card, and `None` for
/// a press that put nothing there — a row that was refused, or a read that
/// failed and left the card holding whatever it held before. It is handed back
/// because somebody has to know which file the panel is showing and it is not
/// going to be [`App`]: a path kept there would be a path something later had to
/// open, and [`App::show_document`] takes lines for exactly that reason. The
/// loop keeps it, and the edit key asks for it — a file rewritten by `$EDITOR`
/// is re-read only when it is the one on the card (see
/// [`edit_press`](crate::editing::edit_press)).
///
/// Nothing else about the press changed for it. A refusal is still worded by
/// [`App::view_target`], a failed read still leaves one line on the footer and
/// the panel exactly as it was, and neither of those is a file the caller should
/// start remembering: `None` means the card is holding what it was already
/// holding.
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

/// What one press of the view key actually does: what ends up in the panel,
/// what ends up on the footer, and what — deliberately — does not move at all.
///
/// Driven over a repository of the test's own under the temporary directory,
/// because this is the half of the key that touches disk: the files are really
/// written, really read and really checked afterwards. No terminal, no network,
/// no `claude` and no worker thread.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use tempfile::TempDir;
    use warlock_engine::{Node, NodeState, PER_FILE_BYTE_CAP, Tree, Viewed, view_file};
    use warlock_tui::{App, Line};

    use super::view_press;

    /// The line the last keystroke left on the footer, which a successful read
    /// is not allowed to spend: showing a file says nothing, exactly as opening
    /// the scope window says nothing.
    const LAST_KEY: &str = "something the last key said";

    /// A panel with room for far more lines than any file below has, so that
    /// what a test reads back is the whole document rather than a screenful of
    /// it.
    const PANEL: u16 = 400;

    /// The document the tests read, written into every repository below.
    const DOCUMENT: &str = "# The engine\n\nIt walks the tree and writes what it finds.\n";

    /// A repository of this test's own, with the files the tree below lists
    /// really written into it, removed when the test that made it ends.
    ///
    /// One directory that is documented and holds files, one that is not, and
    /// one file the tree lists that was never written — which is what a path
    /// that vanished between the walk and the keystroke looks like.
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

    /// A file comfortably past [`PER_FILE_BYTE_CAP`], in lines long enough that
    /// the whole of it still fits in a panel: a thousand characters and a
    /// newline apiece, numbered so that no two lines are the same.
    fn over_the_cap() -> String {
        let cap = usize::try_from(PER_FILE_BYTE_CAP).expect("the cap is a few kilobytes");
        let mut text = String::new();
        for line in 0.. {
            let number = format!("{line:04} ");
            text.push_str(&number);
            text.push_str(&"x".repeat(1000 - number.len()));
            text.push('\n');
            if text.len() > cap + 4096 {
                break;
            }
        }
        text
    }

    /// The tree the app is built over, rooted at `root` and listing the files
    /// [`a_repo`] wrote, plus one it did not.
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

    /// The app the event loop would hold for `root`, with the files shown, the
    /// row for `path` selected, a panel with room to read, and a line on the
    /// footer from the keystroke before this one.
    fn app_on(root: &Path, path: &Path) -> App {
        let mut app = App::from_tree(&tree(root));
        app.toggle_files();
        app.set_panel_height(PANEL);
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

    /// The app with `path` under `root` selected, by the file's name inside
    /// `crates/engine`.
    fn app_on_file(root: &Path, file: &str) -> App {
        app_on(root, &root.join("crates/engine").join(file))
    }

    /// What the panel is showing, as the rows it draws as.
    ///
    /// A document draws as text and nothing else — no clock, no heading, no
    /// summary — so anything else here is the panel showing the wrong card.
    fn panel_text(app: &App) -> Vec<String> {
        app.panel_lines(Instant::now())
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
        assert!(app.has_document(), "nothing was read");
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
        assert_eq!(app.panel_scroll_offset(), 0);
        assert!(!app.panel_follows());
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
        assert!(!app.has_panel_content(), "a directory drew something");
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
        assert!(!app.has_panel_content(), "a directory drew something");
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

    /// An app that has already read a document into the panel and is now on the
    /// row for `file`, with a line on the footer from the keystroke that moved
    /// there.
    ///
    /// What the three failure tests below need is a panel with something really
    /// in it, so that "the panel is exactly as it was" is an assertion about a
    /// document a reader would have lost rather than about emptiness.
    fn app_holding_a_document_on(root: &Path, file: &Path) -> App {
        let mut app = app_on_file(root, "WARLOCK.md");
        assert!(view_press(&mut app).is_some(), "the fixture read nothing");
        assert!(app.has_document(), "the fixture read nothing");
        select(&mut app, file);
        // After the selection, because moving the selection is what takes a
        // message down.
        app.set_message(LAST_KEY);
        app
    }

    /// The assertions the three failures share: one line on the footer naming
    /// the file, the panel byte for byte as it was, and nothing else moved.
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
        assert!(!app.has_panel_content(), "the failed read drew something");

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

    /// Move the selection to the row for `path`, as the movement keys would.
    fn select(app: &mut App, path: &Path) {
        app.select_first();
        while app.selected_row().expect("the fixture has rows").path != path {
            let before = app.selected();
            app.select_next();
            assert_ne!(app.selected(), before, "no row for {}", path.display());
        }
    }
}
