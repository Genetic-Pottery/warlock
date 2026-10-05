use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, io};

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Size;
use ratatui::{Frame, Terminal};
use warlock_engine::{Loaded, Manifest, Node, NodeState, Tree, load_tree, repository_root};

use super::{Parts, Seams, Session};
use crate::app::{App, Chrome, Focus, Row};
use crate::chatting::Chat;
use crate::claude::Converses;
use crate::confirm::QuitConfirm;
use crate::cutting::Cutter;
use crate::linear::Opens;
use crate::pacting::Pact;
use crate::prompt::{RecordPrompt, ScopePrompt};
use crate::puller::{Claudes, Puller, Raises};
use crate::pushing::Pushes;
use crate::screen::Screen;
use crate::session::{Scope, Watched};
use crate::stubs::{Boarding, Checkout, Copying, Forging, Passing, Saying, Scripted};
use crate::ui::tree_height;

#[derive(Debug)]
struct FakeScreen {
    terminal: Terminal<TestBackend>,
    suspensions: Vec<bool>,
    reported: Vec<bool>,
}

impl FakeScreen {
    fn of(width: u16, height: u16) -> Self {
        Self {
            terminal: Terminal::new(TestBackend::new(width, height))
                .expect("a test backend never fails to start"),
            suspensions: Vec::new(),
            reported: Vec::new(),
        }
    }
}

impl Screen for FakeScreen {
    fn size(&self) -> io::Result<Size> {
        // A `TestBackend` cannot fail, so its error type is `Infallible`
        // and there is nothing here for warlock to handle.
        Ok(self.terminal.size().expect("a test backend never fails"))
    }

    fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> io::Result<()> {
        self.terminal
            .draw(render)
            .expect("a test backend never fails");
        Ok(())
    }

    fn suspended<T, F: FnOnce() -> T>(&mut self, mouse: bool, body: F) -> io::Result<T> {
        self.suspensions.push(mouse);
        Ok(body())
    }

    fn report_mouse(&mut self, on: bool) -> io::Result<()> {
        self.reported.push(on);
        Ok(())
    }
}

// The board, the drafting model and the sessions a pull raises are left open,
// because they are the whole difference between a session that files, one that
// cuts and one that works a ticket. The sessions default to the stand-in that
// raises none: every test but `pulling`'s below is driven with no home, so a
// `/pull` is refused long before one would be raised.
struct Stubbed<O, A, M = Claudes>(PhantomData<(O, A, M)>);

impl<O: Opens, A: Converses, M: Raises + Clone + Send + 'static> Seams for Stubbed<O, A, M> {
    type Screen = FakeScreen;
    type Pass = Passing;
    type Talk = Saying;
    type Clip = Copying;
    type Board = O;
    type Draft = A;
    type Repo = Checkout;
    type Forge = Forging;
    type Raise = M;
}

// The drafting model is a script with nothing in it: no test driven through
// this confirms a cut, so a turn being asked for at all is a session opened
// where none was meant to be.
type Driven = Session<Stubbed<Boarding, Scripted>>;

fn driving(app: App, scope: Scope, tree: &Tree) -> Driven {
    driving_over(
        app,
        scope,
        tree,
        // No home, so a `/push` in any test but the ones in `filing` below is
        // refused before a board is resolved and no test in this file can read
        // the sigils of the machine it runs on. The tests that do push replace
        // this whole value with one over a temporary home.
        Pushes::with_client(Boarding::filing(""), None),
        // And the same for a `/draft`, for the same reason: with no home there
        // is nothing for one to resolve a board under, so no test in this file
        // can read the machine's own sigils by typing the command.
        Cutter::with_client(
            Boarding::filing(""),
            None,
            Scripted::saying([]),
            Scripted::saying([]),
        ),
        no_pull(Boarding::filing("")),
    )
}

// A pull over stand-ins and with no home, for the reason the push and the cut
// above have none: a `/pull` typed in any test driven through this is refused
// before a board is opened, a `git` is run or a `claude` is raised.
fn no_pull<O: Opens>(board: O) -> Puller<O, Checkout, Forging, Claudes> {
    Puller::with_seams(
        board,
        Checkout::clean("main"),
        Forging::opening(""),
        Claudes,
        None,
    )
}

// The three values that decide which board a session reaches, which models it
// opens and which sessions a run of a ticket spends are parameters, because that
// is the whole difference between a session that files, one that cuts and one
// that pulls: everything else here is the same loop.
fn driving_over<O: Opens, A: Converses, M: Raises + Clone + Send + 'static>(
    app: App,
    scope: Scope,
    tree: &Tree,
    pushes: Pushes<O>,
    cutter: Cutter<O, A>,
    puller: Puller<O, Checkout, Forging, M>,
) -> Session<Stubbed<O, A, M>> {
    let watched = Watched::start(&scope, tree);
    let root = scope.repo_root.clone();
    let parts = Parts {
        screen: FakeScreen::of(80, 24),
        clipboard: Copying::taking(),
        pact: Pact::with_agent(Passing::filling()),
        chat: Chat::with_agent(root, Saying::answering(ANSWER)),
        pushes,
        cutter,
        puller,
    };
    Session::new(app, scope, Manifest::new(), watched, parts)
}

fn session(rows: Vec<Row>) -> Driven {
    let root = PathBuf::from("/warlock/no/such/repository");
    let scope = Scope {
        chrome: Chrome::of(&root, &root),
        root: root.clone(),
        repo_root: root.clone(),
    };
    // A one-node tree for the watcher to be started over. Nothing is there,
    // so no watcher is granted and `Watching` says why — which is exactly
    // the state a session runs in when the platform refuses one, and costs
    // these tests nothing.
    let tree = Tree::new(Node::new(&root, None::<PathBuf>, NodeState::Unpacted));
    driving(App::from_rows(rows), scope, &tree)
}

fn pressed(driven: &mut Driven, key: KeyEvent) -> bool {
    driven
        .press(key, Instant::now())
        .expect("no key pressed here writes to a terminal")
}

const ANSWER: &str = "The tree, the manifest and the pact.";

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn directory(path: &str) -> Row {
    Row::new(0, path, None, NodeState::Unpacted)
}

const AT_MOST: Duration = Duration::from_secs(5);

fn a_repository() -> tempfile::TempDir {
    let scratch = tempfile::tempdir().expect("a temporary directory");
    for (path, text) in [
        (".git/HEAD", "ref: refs/heads/main\n"),
        ("crates/engine/src/lib.rs", "//! Core engine.\n"),
    ] {
        let at = scratch.path().join(path);
        fs::create_dir_all(at.parent().expect("every path here has a parent"))
            .expect("a scratch directory is writable");
        fs::write(&at, text).expect("a scratch file is writable");
    }
    scratch
}

// `session` over a real scratch repository, built the way `run` builds one
// and, like it, with the tree read first.
fn session_over(root: &Path) -> Driven {
    let (app, scope, tree) = loading(root);
    driving(app, scope, &tree)
}

// The same, over whichever board, models and raised sessions the caller is
// driving.
fn session_reading<O: Opens, A: Converses, M: Raises + Clone + Send + 'static>(
    root: &Path,
    pushes: Pushes<O>,
    cutter: Cutter<O, A>,
    puller: Puller<O, Checkout, Forging, M>,
) -> Session<Stubbed<O, A, M>> {
    let (app, scope, tree) = loading(root);
    driving_over(app, scope, &tree, pushes, cutter, puller)
}

fn loading(root: &Path) -> (App, Scope, Tree) {
    let Loaded { tree, .. } = load_tree(root).expect("a scratch repository loads");
    let repo_root = repository_root(tree.root_path()).expect("the load found a repository");
    let scope = Scope {
        chrome: Chrome::of(&repo_root, tree.root_path()),
        root: tree.root_path().to_path_buf(),
        repo_root,
    };
    (App::from_tree(&tree), scope, tree)
}

fn rounds_until_settled(driven: &mut Driven) {
    let waited = Instant::now();
    while driven.pact.running() && waited.elapsed() < AT_MOST {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        driven.keep_up();
    }
    assert!(!driven.pact.running(), "the run never finished");
}

#[test]
fn a_round_tells_the_app_the_size_the_frame_is_being_cut_at() {
    let mut driven = session(vec![directory("/repo/crates")]);
    let size = driven.size().expect("the fake screen has a size");

    driven.draw(size).expect("the fake screen draws");

    assert_eq!(
        driven.app.viewport_height(),
        usize::from(tree_height(size)),
        "the app was told the height this frame gives the tree"
    );
}

#[test]
fn pressing_the_pact_key_descends_the_subtree_and_lands_its_documents() {
    let repo = a_repository();
    let mut driven = session_over(repo.path());

    assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
    assert!(driven.pact.running(), "the press started a run");

    rounds_until_settled(&mut driven);

    assert!(
        repo.path().join(".warlock.md").is_file(),
        "the root was never documented"
    );
    assert!(
        repo.path().join("crates/engine/src/.warlock.md").is_file(),
        "the descent stopped short of the deepest directory"
    );
    assert_eq!(
        driven.manifest.entries().len(),
        4,
        "every directory the walk produced should have been granted"
    );
}

#[test]
fn a_scope_written_outside_warlock_survives_the_next_run_after_a_reload() {
    // `git pull`, or `warlock scope add` in another terminal: the manifest on
    // disk moves while warlock is up. The reload that follows is what every
    // key after it has to act on, or the next save writes the old copy back
    // over the edit.
    let repo = a_repository();
    let mut driven = session_over(repo.path());
    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    let outside = Manifest::load(repo.path()).expect("the run saved a manifest");
    let outside = Manifest::with_entries(outside.entries().iter().map(|entry| {
        if entry.module() == "crates/engine/src" {
            entry.clone().with_scope("data-plane")
        } else {
            entry.clone()
        }
    }));
    outside.save(repo.path()).expect("saves");
    fs::write(
        repo.path().join("crates/engine/src/lib.rs"),
        "//! Core engine, revised.\n",
    )
    .expect("a scratch file is writable");
    crate::session::reload(&mut driven.app, &driven.scope, &mut driven.manifest);

    pressed(&mut driven, key(KeyCode::Char('r')));
    rounds_until_settled(&mut driven);

    let after = Manifest::load(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after
            .entry("crates/engine/src")
            .and_then(warlock_engine::PactEntry::scope),
        Some("data-plane"),
        "the refresh wrote warlock's stale copy back over the outside edit"
    );
}

#[test]
fn a_second_press_of_the_pact_key_takes_the_whole_subtree_back_out() {
    let repo = a_repository();
    let mut driven = session_over(repo.path());
    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    assert_eq!(
        driven.manifest.entries().len(),
        0,
        "un-pacting left entries behind"
    );
    assert!(
        repo.path().join(".warlock.md").is_file(),
        "un-pacting deleted a document, which it has never done"
    );
}

#[test]
fn the_quit_key_opens_the_question_rather_than_leaving() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(
        pressed(&mut driven, key(KeyCode::Char('q'))),
        "the session goes on"
    );
    assert_eq!(
        driven.confirm,
        QuitConfirm::open(),
        "and the question is up with No lit"
    );
}

#[test]
fn ctrl_c_leaves_without_asking() {
    let mut driven = session(vec![directory("/repo/crates")]);

    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!pressed(&mut driven, key), "the session is over");
    assert_eq!(
        driven.confirm,
        QuitConfirm::Closed,
        "and no question was ever asked"
    );
}

#[test]
fn answering_no_puts_the_question_down_and_stays() {
    let mut driven = session(vec![directory("/repo/crates")]);
    pressed(&mut driven, key(KeyCode::Char('q')));

    assert!(
        pressed(&mut driven, key(KeyCode::Enter)),
        "the session goes on"
    );
    assert_eq!(
        driven.confirm,
        QuitConfirm::Closed,
        "and the question is down"
    );
}

#[test]
fn the_focus_key_moves_the_keyboard_on() {
    let mut driven = session(vec![directory("/repo/crates")]);
    let before = driven.app.focus();

    assert!(pressed(&mut driven, key(KeyCode::Tab)));
    assert_ne!(driven.app.focus(), before, "the focus moved");
}

#[test]
fn the_mouse_key_flips_what_the_loop_is_holding() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
    assert!(!driven.mouse_captured, "reporting was turned off");

    assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
    assert!(driven.mouse_captured, "and back on again");

    assert_eq!(
        driven.screen.reported,
        [false, true],
        "and the terminal was told each time, through the screen rather \
             than past it"
    );
}

#[test]
fn an_edit_over_a_directory_never_asks_for_the_screen() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(pressed(&mut driven, key(KeyCode::Char('e'))));
    assert!(
        driven.screen.suspensions.is_empty(),
        "a row that is not a file is refused before any child is run"
    );
    assert!(
        driven.app.message().is_some(),
        "and the refusal is said rather than swallowed"
    );
}

#[test]
fn the_scope_prompt_swallows_the_pact_key() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.prompt = ScopePrompt::open("crates", "");

    assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
    let field = driven
        .prompt
        .field()
        .expect("the window is still up over the directory it opened on");
    assert_eq!(field.text(), "p", "the key was typed, not pressed");
}

#[test]
fn a_key_the_window_does_not_want_puts_it_down_and_starts_nothing() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.prompt = ScopePrompt::open("crates", "");

    assert!(pressed(&mut driven, key(KeyCode::Esc)));
    assert_eq!(
        driven.prompt,
        ScopePrompt::Closed,
        "Esc closes the window rather than quitting warlock"
    );
}

#[test]
fn the_record_window_swallows_the_scope_key_itself() {
    // The one key that could reopen a window over the very scope being
    // recorded, and it does not: with the second window up `s` is a letter in
    // the focused field, like every other binding the loop would otherwise
    // answer.
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.record = RecordPrompt::open("crates", "billing");

    assert!(pressed(&mut driven, key(KeyCode::Char('s'))));
    assert_eq!(
        driven.prompt,
        ScopePrompt::Closed,
        "the first window came back up over the name being recorded"
    );
    let form = driven
        .record
        .form()
        .expect("the window is still up over the scope it opened on");
    assert_eq!(form.focused().text(), "s", "the key was typed, not pressed");
}

#[test]
fn esc_puts_the_record_window_down_and_writes_nothing() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.record = RecordPrompt::open("crates", "billing");

    assert!(pressed(&mut driven, key(KeyCode::Esc)));
    assert_eq!(
        driven.record,
        RecordPrompt::Closed,
        "Esc closes the window rather than quitting warlock"
    );
    // The session's manifest is the empty one `driving` starts it on, and an
    // Esc that had written would have replaced it. The repository root these
    // tests run over does not exist, so a write would have failed loudly too.
    assert_eq!(driven.manifest, Manifest::new());
}

mod copying {
    use super::{Copying, directory, session};

    // Not "no clipboard here": what arboard hands over is some other
    // program's complaint, and the footer has one line to say it on.
    const REFUSED: &str = "no clipboard on this session\nnothing was listening";

    #[test]
    fn a_copy_that_lands_says_how_much_went() {
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");

        assert_eq!(
            driven.clipboard.copied(),
            ["crates/engine"],
            "the text never reached the clipboard"
        );
        assert_eq!(driven.app.message(), Some("copied 13 characters"));
    }

    #[test]
    fn one_character_is_counted_in_the_singular() {
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("p");

        assert_eq!(driven.app.message(), Some("copied 1 character"));
    }

    #[test]
    fn characters_are_counted_rather_than_the_bytes_utf_8_spells_them_with() {
        let mut driven = session(vec![directory("/repo/crates")]);

        // Five characters and seven bytes: a count of bytes would tell a
        // reader something about UTF-8 rather than about what they copied.
        let text = "péché";
        assert_ne!(text.len(), text.chars().count(), "this text is all ASCII");
        driven.copy(text);

        assert_eq!(driven.app.message(), Some("copied 5 characters"));
    }

    #[test]
    fn a_clipboard_that_refuses_says_so_on_one_line_and_claims_nothing() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.clipboard = Copying::refusing(REFUSED);

        driven.copy("crates/engine");

        let said = driven
            .app
            .message()
            .expect("a copy that did not happen is said rather than swallowed");
        assert!(
            !said.contains('\n'),
            "the footer has one line and this wrapped: {said}"
        );
        assert!(
            said.starts_with("nothing was copied"),
            "the footer claims something happened: {said}"
        );
        assert!(
            !said.contains("character"),
            "a failed copy counted characters onto the clipboard: {said}"
        );
        assert!(
            said.contains("no clipboard on this session"),
            "what the clipboard said was thrown away: {said}"
        );
        assert!(
            driven.clipboard.copied().is_empty(),
            "a refused copy left text on the clipboard anyway"
        );
    }
}

mod forgetting {
    use std::time::Instant;

    use super::super::MESSAGE_LIFETIME;
    use super::{directory, session};

    #[test]
    fn a_message_stays_up_for_its_lifetime_and_is_gone_after_it() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");
        driven.forget_stale_message(base);

        driven.forget_stale_message(base + MESSAGE_LIFETIME / 2);
        assert_eq!(
            driven.app.message(),
            Some("copied 13 characters"),
            "the footer dropped what it was told before its time"
        );

        driven.forget_stale_message(base + MESSAGE_LIFETIME);
        assert_eq!(
            driven.app.message(),
            None,
            "the footer is still claiming a copy that has passed"
        );
    }

    #[test]
    fn the_same_sentence_said_again_gets_its_own_lifetime() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");
        driven.forget_stale_message(base);

        // Most of the way through the first saying's life, the same text
        // again, so the footer's line is the same line to the character.
        // Nothing about the words says it is new, which is what the count
        // rather than a comparison of them is for.
        let again = base + MESSAGE_LIFETIME / 2;
        driven.copy("crates/engine");
        driven.forget_stale_message(again);

        driven.forget_stale_message(base + MESSAGE_LIFETIME);
        assert_eq!(
            driven.app.message(),
            Some("copied 13 characters"),
            "the second copy was timed from the first one's saying"
        );

        driven.forget_stale_message(again + MESSAGE_LIFETIME);
        assert_eq!(driven.app.message(), None);
    }

    #[test]
    fn a_footer_with_nothing_on_it_is_left_alone() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.forget_stale_message(base);
        driven.forget_stale_message(base + MESSAGE_LIFETIME);

        assert_eq!(driven.app.message(), None);
    }
}

mod dragging {
    use ratatui::crossterm::event::{
        KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Size;

    use super::{ANSWER, Copying, Driven, Focus, Instant, directory, key, pressed, session};

    const PRESS: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
    const RELEASE: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

    // Piece 0 of the conversation these tests drag over, so what a drag
    // across the word `does` copies is `does` — the question as it was
    // asked, not the row the panel drew it on.
    const ASKED: &str = "what does the engine do?";

    const REFUSED: &str = "no clipboard on this session\nnothing was listening";

    // A session with a turn in the conversation, drawn once: a pointer event
    // is read against the frame it landed on, and the frame is what tells
    // the panel its width and height.
    fn conversing(now: Instant) -> (Driven, Size) {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.panel_mut().start_turn(ASKED, now);
        driven.app.panel_mut().answer_turn(ANSWER, now);
        let size = redrawn(&mut driven);
        (driven, size)
    }

    fn redrawn(driven: &mut Driven) -> Size {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        size
    }

    // Where a word the frame drew is, as screen cells: the column its first
    // character landed on and the row it landed in. Read off the drawn frame
    // rather than worked out from the layout, so these tests point at the
    // cells a reader would point at.
    fn drawn_at(driven: &Driven, word: &str) -> (u16, u16) {
        found_at(driven, word).unwrap_or_else(|| panic!("the frame never drew {word:?}"))
    }

    // The same search, handing back the absence rather than panicking on it,
    // for the tests that turn on a line *not* being on screen yet.
    fn found_at(driven: &Driven, word: &str) -> Option<(u16, u16)> {
        let buffer = driven.screen.terminal.backend().buffer();
        let area = buffer.area;
        (0..area.height).find_map(|row| {
            let line: String = (0..area.width).map(|x| buffer[(x, row)].symbol()).collect();
            let byte = line.find(word)?;
            let column = line[..byte].chars().count();
            let column = u16::try_from(column).expect("a column of the frame");
            Some((column, row))
        })
    }

    fn point(driven: &mut Driven, kind: MouseEventKind, at: (u16, u16), size: Size, now: Instant) {
        let (column, row) = at;
        driven.point(
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            size,
            now,
        );
    }

    #[test]
    fn a_drag_across_the_conversation_copies_what_the_highlight_covers() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 2, row), size, now);
        // The space after the `s`, which is the character the drag stopped
        // on and the end of `does`.
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        assert_eq!(
            driven.clipboard.copied(),
            ["does"],
            "the release copied something other than what the drag covered"
        );
        assert_eq!(driven.app.message(), Some("copied 4 characters"));
        // The one thing on screen saying what went: a footer line over a
        // card with nothing highlighted would leave the reader taking
        // warlock's word for it.
        assert_eq!(
            crate::interactive::selected_text(&driven.app),
            "does",
            "the copy took the highlight down with it"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Panel,
            "the press no longer points the keys at the pane it landed in"
        );
    }

    #[test]
    fn a_drag_whose_copy_is_refused_says_so_and_claims_nothing() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        driven.clipboard = Copying::refusing(REFUSED);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        let said = driven
            .app
            .message()
            .expect("a copy that did not happen is said rather than swallowed");
        assert!(
            !said.contains('\n'),
            "the footer has one line and this wrapped: {said}"
        );
        assert!(
            !said.contains("character"),
            "the footer counted characters onto a clipboard that refused: {said}"
        );
        assert!(
            said.contains("no clipboard on this session"),
            "what the clipboard said was thrown away: {said}"
        );
        assert!(
            driven.clipboard.copied().is_empty(),
            "a refused copy left text on the clipboard anyway"
        );
        assert_eq!(
            crate::interactive::selected_text(&driven.app),
            "does",
            "the highlight came down over a copy that never happened"
        );
    }

    #[test]
    fn a_press_nobody_dragged_from_copies_nothing_and_says_nothing() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, RELEASE, (column, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a press with no drag after it copied the character under it"
        );
        assert_eq!(
            driven.app.message(),
            None,
            "a copy that never happened was reported anyway"
        );
        assert_eq!(
            crate::interactive::selected_text(&driven.app),
            String::new(),
            "a press with no drag after it highlighted text"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Panel,
            "a press in the panel stopped taking the keys"
        );
    }

    #[test]
    fn a_drag_over_the_tree_copies_nothing_and_selects_its_row() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "crates");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 3, row), size, now);
        point(&mut driven, RELEASE, (column + 3, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a drag down the tree column copied something"
        );
        assert_eq!(
            driven.app.message(),
            None,
            "a drag over the tree wrote a line on the footer"
        );
        assert_eq!(
            driven.app.selection(),
            None,
            "a drag over the tree highlighted the conversation"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Tree,
            "a press on a row stopped pointing the keys at the tree"
        );
    }

    #[test]
    fn a_drag_over_another_card_copies_nothing_and_says_nothing() {
        let now = Instant::now();
        // The size this one drags at is the one the frame with the document
        // on it was drawn at, below.
        let (mut driven, _) = conversing(now);
        // The word is in the document rather than the conversation, so the
        // cells the drag covers are cells of the card that is showing.
        driven.app.show_document(["what a document does"], false);
        assert!(
            !driven.app.panel().showing_thread(),
            "the document card never took the conversation's place"
        );
        let size = redrawn(&mut driven);
        let said_before = driven.app.message().map(str::to_owned);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a drag over the document card copied a line of it"
        );
        assert_eq!(
            driven.app.message().map(str::to_owned),
            said_before,
            "a drag over the document card wrote a line on the footer"
        );
        assert_eq!(
            driven.app.selection(),
            None,
            "a drag over the document card highlighted the conversation behind it"
        );
    }

    // Typed into the composer so there is a field on screen to press in, and
    // a word to find it by.
    const DRAFT: &str = "draft";

    // A conversation several screens tall, so there is somewhere for the
    // card to scroll, drawn once at the size the drags below land on.
    fn scrollback(now: Instant) -> (Driven, Size) {
        let mut driven = session(vec![directory("/repo/crates")]);
        for turn in 0..12 {
            let asked = format!("question {turn} about the engine");
            driven.app.panel_mut().start_turn(&asked, now);
            driven.app.panel_mut().answer_turn(ANSWER, now);
        }
        let size = redrawn(&mut driven);
        assert!(
            driven.app.panel().scroll_offset() > 0,
            "the conversation fits on the card: nothing here would scroll"
        );
        (driven, size)
    }

    // The card wound back to its first line, where a drag downwards has the
    // whole conversation below it.
    fn wound_back(driven: &mut Driven) -> Size {
        driven.app.scroll_panel_up(usize::MAX);
        redrawn(driven)
    }

    // A point below every row of the card: the footer, which is inside the
    // screen and outside the panel.
    fn below_the_card(size: Size, column: u16) -> (u16, u16) {
        (column, size.height - 1)
    }

    // A draft typed into the composer, so that the field is drawn and there
    // is somewhere in it to press. Typed rather than set, because the
    // composer only takes letters with the keys pointed at it and that is
    // the state a reader presses in it from.
    fn drafting(driven: &mut Driven) {
        driven.app.set_focus(Focus::Composer);
        for letter in DRAFT.chars() {
            assert!(
                pressed(driven, key(KeyCode::Char(letter))),
                "typing into the composer ended the session"
            );
        }
    }

    // The tick, driven by hand: `run`'s loop calls `drag_scroll` once a
    // round whether or not an event arrived, so a round with nothing in it
    // is `drag_scroll` on its own.
    mod past_the_edge {
        use ratatui::crossterm::event::MouseEventKind;

        use super::{
            DRAFT, DRAG, Driven, Focus, Instant, PRESS, RELEASE, below_the_card, drafting,
            drawn_at, found_at, point, scrollback, wound_back,
        };
        use crate::interactive::rows_per_tick;

        // A pointer moved with nothing held down, which is what a reader
        // whose hand is off the button sends as they cross the footer.
        const MOVED: MouseEventKind = MouseEventKind::Moved;

        fn covered(driven: &Driven) -> usize {
            crate::interactive::selected_text(&driven.app)
                .chars()
                .count()
        }

        #[test]
        fn one_row_past_the_edge_is_a_row_a_tick_and_far_past_it_is_several() {
            assert_eq!(
                rows_per_tick(1),
                1,
                "the row just past the edge is not the slow, aimable one"
            );
            assert_eq!(rows_per_tick(6), 2, "the middle of the curve moved");
            assert_eq!(
                rows_per_tick(40),
                5,
                "a pointer dragged to the bottom of the terminal is not at the ceiling"
            );
        }

        #[test]
        fn a_drag_held_below_the_card_keeps_scrolling_and_takes_the_highlight_with_it() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            let anchored = driven.app.panel().scroll_offset();
            assert_eq!(
                covered(&driven),
                0,
                "the drag past the edge highlighted text off its own event"
            );

            driven.drag_scroll();
            let after_one = driven.app.panel().scroll_offset();
            assert!(
                after_one > anchored,
                "the tick left the card where the drag did: {after_one}"
            );
            let after_one_covered = covered(&driven);
            assert!(
                after_one_covered > 0,
                "the card scrolled out from under the highlight"
            );

            // No further event: the pointer is being held still, which is
            // the whole reason this runs off the tick.
            driven.drag_scroll();
            assert!(
                driven.app.panel().scroll_offset() > after_one,
                "the scrolling stopped when the pointer did"
            );
            assert!(
                covered(&driven) > after_one_covered,
                "the highlight stopped growing while the card went on scrolling"
            );
        }

        #[test]
        fn the_copy_on_release_takes_in_what_the_ticks_scrolled_into_view() {
            // A turn far enough down the conversation that the frame the
            // press lands on has not drawn it: nothing but the ticks can
            // bring it inside the highlight, so finding it on the clipboard
            // is the scrolling and the copy proving each other.
            const LATER: &str = "question 11";

            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");
            let past = below_the_card(size, column);
            assert!(
                found_at(&driven, LATER).is_none(),
                "the whole conversation is on screen already: \
                     {LATER} needs no scrolling to reach"
            );

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            let anchored = driven.app.panel().scroll_offset();
            for _ in 0..200 {
                driven.drag_scroll();
            }
            assert!(
                driven.app.panel().scroll_offset() > anchored,
                "the ticks left the card where the drag did"
            );
            assert!(covered(&driven) > 0, "the ticks grew no highlight");
            point(&mut driven, RELEASE, past, size, now);

            let [copied] = driven.clipboard.copied() else {
                panic!(
                    "the release past the edge copied something other than once: {:?}",
                    driven.clipboard.copied()
                )
            };
            assert!(
                copied.starts_with("question 0"),
                "the copy began somewhere other than where the press did: {copied:?}"
            );
            assert!(
                copied.contains(LATER),
                "the copy stopped at the edge the drag left rather than at \
                     the line the ticks reached: {copied:?}"
            );
        }

        #[test]
        fn the_scrolling_stops_at_the_end_of_the_thread() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            for _ in 0..200 {
                driven.drag_scroll();
            }

            assert_eq!(
                driven.app.panel().lines_below(),
                0,
                "the ticks left the card short of the end of the conversation"
            );
            let end = driven.app.panel().scroll_offset();
            let held = covered(&driven);

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                end,
                "the card scrolled past the last line of the conversation"
            );
            assert_eq!(
                covered(&driven),
                held,
                "the highlight went on growing over a card that had stopped"
            );
        }

        #[test]
        fn a_drag_held_above_the_card_scrolls_the_other_way_and_stops_at_the_top() {
            let now = Instant::now();
            // Left where a conversation sits: at the newest line, with
            // everything else above it.
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "engine");
            let at_the_end = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            // Row zero is the top border of the panes, which is past every
            // row of the card.
            point(&mut driven, DRAG, (column, 0), size, now);
            driven.drag_scroll();

            assert!(
                driven.app.panel().scroll_offset() < at_the_end,
                "the tick scrolled the wrong way for a pointer above the card"
            );
            assert!(
                covered(&driven) > 0,
                "the highlight did not follow the card upwards"
            );

            for _ in 0..200 {
                driven.drag_scroll();
            }
            let top = driven.app.panel().scroll_offset();
            assert_eq!(top, 0, "the ticks stopped short of the first line");

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                0,
                "the card scrolled above its first line"
            );
        }

        #[test]
        fn the_release_that_ends_the_drag_ends_the_scrolling() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");
            let past = below_the_card(size, column);

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            driven.drag_scroll();
            point(&mut driven, RELEASE, past, size, now);
            let let_go = driven.app.panel().scroll_offset();
            let held = covered(&driven);

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                let_go,
                "the card went on scrolling after the button came up"
            );
            assert_eq!(
                covered(&driven),
                held,
                "the highlight went on growing after the button came up"
            );
            assert_eq!(
                driven.clipboard.copied().len(),
                1,
                "the release past the edge copied something other than once"
            );
        }

        #[test]
        fn a_round_with_no_button_held_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let where_it_was = driven.app.panel().scroll_offset();

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a card nobody is dragging over scrolled by itself"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a tick with no drag behind it highlighted something"
            );
        }

        #[test]
        fn a_press_on_the_tree_dragged_past_the_card_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (tree_column, tree_row) = drawn_at(&driven, "crates");
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (tree_column, tree_row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a drag that began in the tree scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag that began in the tree highlighted the conversation"
            );
        }

        #[test]
        fn a_press_on_the_composer_dragged_past_the_card_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            drafting(&mut driven);
            // Pointed away again, so that the press taking the keys back is
            // this test's proof that it landed in the field rather than on a
            // line of the conversation behind it.
            driven.app.set_focus(Focus::Tree);
            let size = wound_back(&mut driven);
            let field = drawn_at(&driven, DRAFT);
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, field, size, now);
            assert_eq!(
                driven.app.focus(),
                Focus::Composer,
                "the press landed somewhere other than the composer"
            );
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a drag that began in the composer scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag that began in the composer highlighted the conversation"
            );
        }

        #[test]
        fn a_pointer_past_the_edge_with_no_button_held_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, MOVED, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a pointer crossing the footer with nothing held down \
                     scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a pointer crossing the footer with nothing held down \
                     highlighted the conversation"
            );
        }

        #[test]
        fn a_card_put_up_while_the_button_is_held_is_not_scrolled_by_it() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            // The one way another card can take the conversation's place
            // without the button coming up first: a key pressed mid-drag.
            driven
                .app
                .show_document(std::iter::repeat_n("a line of the file", 200), false);
            let where_it_was = driven.app.panel().scroll_offset();

            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "the held drag scrolled the document that replaced the conversation"
            );
        }
    }

    // Following, held off for the length of a drag. A turn is long, so
    // copying an earlier answer while a later one arrives is an ordinary
    // thing to want, and a card that went on pulling itself to the newest
    // line would take the text out from under the pointer mid-gesture.
    mod pausing {
        use super::{
            DRAFT, DRAG, Driven, Instant, PRESS, RELEASE, below_the_card, drafting, drawn_at,
            found_at, point, redrawn, scrollback, wound_back,
        };
        use crate::claude::Activity;
        use crate::panel::Showing;

        // A word nothing in the conversation has until the appends below put
        // it there, short enough that the panel cannot wrap it: finding it on
        // the frame is the card having gone to the newest line and nothing
        // else.
        const NEWEST: &str = "ozymandias";

        // Everything a live turn puts into the conversation: the question, a
        // line of activity, one of warlock's own notes, the answer. The
        // question and the note go through `Card::accrue`, which is the one
        // path that sets following; the other two are written into the turn
        // already there and ride the flag it left.
        fn a_turn_arrives(driven: &mut Driven, now: Instant) {
            let panel = driven.app.panel_mut();
            panel.start_turn("question 12 about the engine", now);
            panel.record_turn(&Activity::Thinking, now);
            panel.note("warlock has something to say", now);
            panel.answer_turn(NEWEST, now);
        }

        // The conversation growing under a gesture that is not a drag held
        // over it, which has to move the card to the newest line exactly as
        // it does with no button down anywhere. `before` is where the
        // conversation's window was, read by the caller while the card was
        // showing: some of these gestures put another card in front of it,
        // and a turn arriving brings the conversation back.
        fn the_conversation_still_follows(driven: &mut Driven, before: usize, now: Instant) {
            a_turn_arrives(driven, now);

            let panel = driven.app.panel();
            assert!(
                panel.showing_thread(),
                "a turn arriving left another card in front of the conversation"
            );
            let after = panel.scroll_offset();
            assert!(
                after > before,
                "the conversation stayed where it was: {before} to {after}"
            );
            assert!(
                panel.follows(),
                "the card came out of the gesture not following"
            );
            assert_eq!(
                panel.lines_below(),
                0,
                "the card stopped short of the newest line"
            );
        }

        #[test]
        fn a_held_drag_keeps_the_card_still_while_the_conversation_grows() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // An earlier part of the conversation, with everything the turn
            // is about to say far below it.
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            let offset = driven.app.panel().scroll_offset();
            let rows = driven.app.panel().window(now);
            let covered = crate::interactive::selected_text(&driven.app);
            assert!(
                !covered.is_empty(),
                "the drag covered nothing: there is no highlight here to disturb"
            );

            a_turn_arrives(&mut driven, now);

            assert_eq!(
                driven.app.panel().scroll_offset(),
                offset,
                "the arriving turn pulled the held card to the newest line"
            );
            assert_eq!(
                driven.app.panel().window(now),
                rows,
                "the rows under the pointer changed while the button was held"
            );
            assert_eq!(
                crate::interactive::selected_text(&driven.app),
                covered,
                "what the highlight covers changed under the held drag"
            );
            redrawn(&mut driven);
            assert!(
                found_at(&driven, NEWEST).is_none(),
                "the frame drew the newest line over a drag held on an earlier one"
            );
        }

        #[test]
        fn the_release_hands_the_newest_line_back_mid_turn() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // A question out and unanswered, which is the state a reader
            // copies an earlier answer in. The card is following it, so the
            // drag below is over the newest screenful — an earlier part of
            // the conversation than the answer still to come, and a card
            // that was following when the button went down.
            driven
                .app
                .panel_mut()
                .start_turn("question 12 about the engine", now);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "question 12");
            let held = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            // A line of its own, unlike the turn's first activity line,
            // which takes the place of the one the log draws for a turn
            // that has heard nothing yet.
            driven
                .app
                .panel_mut()
                .note("warlock has something to say", now);

            assert_eq!(
                driven.app.panel().scroll_offset(),
                held,
                "the note pulled the held card down"
            );

            point(&mut driven, RELEASE, (column + 8, row), size, now);

            let released = driven.app.panel().scroll_offset();
            assert!(
                released > held,
                "the card came out of the drag still parked where it was held"
            );

            driven.app.panel_mut().answer_turn(NEWEST, now);

            assert!(
                driven.app.panel().scroll_offset() > released,
                "the answer left the card where the drag had it"
            );
            assert_eq!(
                driven.app.panel().lines_below(),
                0,
                "the card stopped short of the newest line"
            );
            redrawn(&mut driven);
            assert!(
                found_at(&driven, NEWEST).is_some(),
                "the answer that ended the turn was never drawn"
            );
        }

        #[test]
        fn a_release_past_the_card_hands_the_newest_line_back_too() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");
            let past = below_the_card(size, column);

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            // The button let go out here, where there is no cell of the card
            // under it: a release all the same, and the end of the hold.
            point(&mut driven, RELEASE, past, size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_card_put_up_mid_drag_does_not_leave_the_conversation_held() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            // A key pressed with the button still down, which is the one way
            // another card takes the conversation's place mid-drag.
            driven.app.show_document(["what a document does"], false);
            point(&mut driven, RELEASE, (column + 8, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_press_nobody_dragged_from_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, RELEASE, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_in_the_tree_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "crates");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 3, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_in_the_composer_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            drafting(&mut driven);
            let size = redrawn(&mut driven);
            let field = drawn_at(&driven, DRAFT);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, field, size, now);
            point(&mut driven, DRAG, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_on_the_footer_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, below_the_card(size, column), size, now);
            point(&mut driven, DRAG, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_over_the_document_card_leaves_the_conversation_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // Read while the conversation is still the card showing, which
            // is the one moment the panel answers for it.
            let where_it_was = driven.app.panel().scroll_offset();
            driven.app.show_document(["what a document does"], false);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "document");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_over_the_account_card_leaves_the_conversation_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let where_it_was = driven.app.panel().scroll_offset();
            driven.app.start_account(now);
            driven
                .app
                .panel_mut()
                .write_run(|account| account.open_section("crates/engine", now));
            // The conversation has content, so a run does not put its own
            // account up: the swap key is what a reader would press, and this
            // is that press without the keyboard.
            driven.app.panel_mut().show(Showing::Account);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "crates/engine");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }
    }
}

mod pasting {
    use super::{Focus, Instant, directory, session};
    use crate::chatting::Asked;

    #[test]
    fn a_paste_with_the_keyboard_off_the_composer_changes_nothing_anywhere() {
        let mut driven = session(vec![directory("/repo/crates"), directory("/repo/docs")]);
        // Where a session opens: the keys are commands and there is no
        // draft for anything to land in.
        assert_ne!(
            driven.app.focus(),
            Focus::Composer,
            "this test is about the keyboard being somewhere else"
        );
        let selected = driven.app.selected();
        let mode = driven.app.panel().mode();

        driven.paste("crates\ndocs\n");

        assert_eq!(
            driven.chat.composer().draft(),
            "",
            "a paste aimed at nothing was typed into the field anyway"
        );
        assert_eq!(
            driven.app.selected(),
            selected,
            "the pasted lines moved the tree's selection"
        );
        assert_eq!(
            driven.app.panel().mode(),
            mode,
            "the pasted lines changed register"
        );
        assert!(
            driven.app.message().is_none(),
            "a paste nobody can act on said something on the footer"
        );
        assert!(!driven.chat.answering(), "a paste started a turn");
    }

    #[test]
    fn a_paste_at_a_muted_field_leaves_the_draft_byte_for_byte() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.set_focus(Focus::Composer);
        driven.paste("half a question");

        // A question put out without going past the field, which is what
        // leaves a draft standing under the muting: a submit would have
        // emptied it on the way through, and `Chat::settle_field` — still
        // the one thing that sets the flag — is what `say` calls.
        driven.chat.say(
            &mut driven.app,
            "what is a pact?",
            "what is a pact?",
            Asked::Answer,
            Instant::now(),
        );
        assert!(
            driven.chat.composer().is_muted(),
            "a question is out and the field still types"
        );

        driven.paste("\nand the rest of it");

        assert_eq!(
            driven.chat.composer().draft(),
            "half a question",
            "the muted field took a paste"
        );
        assert!(
            driven.chat.composer().is_muted(),
            "a paste handed the keyboard back mid-turn"
        );
    }

    #[test]
    fn a_multi_line_paste_lands_whole_and_asks_nothing() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.set_focus(Focus::Composer);

        // The block that used to send line one and lose the other two.
        driven.paste("what is a pact?\nand what is a scope?\nand a sigil?");

        assert_eq!(
            driven.chat.composer().draft(),
            "what is a pact?\nand what is a scope?\nand a sigil?",
            "the pasted lines did not all reach the draft"
        );
        assert!(
            !driven.chat.answering(),
            "a newline in a paste started a turn"
        );
        assert!(
            driven.app.panel().thread().is_none(),
            "a paste sent something: there is a conversation and nobody asked for one"
        );
        assert!(
            !driven.chat.composer().is_muted(),
            "a paste muted the field"
        );
    }
}

// A `/push` driven the whole way through a session — `/push <scope> <path>`,
// Left, Enter — over a Linear that answers out of memory and a home this module
// made. Nothing here opens a socket, reads a real credential or looks at the
// machine's own sigils, binding or key store.
mod filing {
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    use ratatui::crossterm::event::KeyCode;
    use tempfile::TempDir;
    use warlock_engine::{
        Manifest, PactEntry, ScopeRecord, save_key, save_key_binding, save_sigils,
    };

    use super::{AT_MOST, Driven, key, pressed, session_over};
    use crate::account::Line;
    use crate::app::Focus;
    use crate::cutting::Cutter;
    use crate::pushing::{ALREADY_FILING, Pushes};
    use crate::stubs::{Boarding, Gate, Op, Scripted};

    // Not a key, and named so that nothing reading this file mistakes it for
    // one. It is stored so that a bound name resolves and the client is built
    // from something, and every test below asserts it is in nothing warlock
    // said or holds.
    const NOT_A_KEY: &str = "not-a-real-key-value";

    // A name no real key store would be holding, so a session that reached the
    // machine's own home could not pass for one that reached this home.
    const KEY_NAME: &str = "this-tests-own-name";

    const SCOPE: &str = "data-plane";

    // A Linear team *key*, which is what a `[[scope]]` record carries.
    const TEAM: &str = "WAR";

    const URL: &str = "https://linear.app/acme/project/push-a-brief-1a2b3c";

    // Linear's own words for a request it understood and would not do, which is
    // the failure the session has to survive with a line.
    const REFUSED: &str = "Entity not found";

    // Rounds taken with a request held open: more than one, so what is being
    // asserted is a loop going round rather than a single frame.
    const ROUNDS: usize = 3;

    // Where the brief sits, and so what a `/push` names.
    const PATH: &str = "docs/a-brief.md";

    // Every section the built-in shape asks for, so the document the push files
    // is one `brief_at` reads.
    const BRIEF: &str = "# Push a brief to the board\n\n\
                         Nothing turns a document on disk into a project.\n\n\
                         ## Outcome\n\n`/push` files it.\n\n\
                         ## Success criteria\n\n**The reader**\n\n- sees a URL\n\n\
                         ## Constraints\n\nNo new dependency.\n\n\
                         ## Out of scope\n\nPulling anything back.\n\n\
                         ## Scope\n\n### 1. Read the file\n\ndepends_on: []\n";

    fn a_manifest() -> Manifest {
        Manifest::with_entries([PactEntry::new(".", "docs", "docs/.warlock.md")
            .expect("a relative module path is inside the root")
            .with_scope(SCOPE)])
        .with_scopes([ScopeRecord::new(SCOPE, TEAM, "In Review", "warlock")])
    }

    // Enough of a repository for the load the session is built over, and the
    // brief a `/push` names.
    fn a_repository() -> TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let head = repo.path().join(".git/HEAD");
        fs::create_dir_all(head.parent().expect("`.git` is a directory"))
            .expect("a scratch directory is writable");
        fs::write(&head, "ref: refs/heads/main\n").expect("a scratch file is writable");
        let path = repo.path().join(PATH);
        fs::create_dir_all(path.parent().expect("a `docs` directory"))
            .expect("a scratch directory is writable");
        fs::write(&path, BRIEF).expect("a scratch file is writable");
        repo
    }

    // One sigil and one record of that name, so nothing is ambiguous and the
    // dialog comes straight up.
    fn a_home(root: &Path) -> TempDir {
        let home = tempfile::tempdir().expect("a temporary directory");
        save_sigils(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
        save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
        save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
        home
    }

    // The session `run` builds, with its impure things replaced: the manifest
    // that would have been loaded, and a Linear reached through a home of this
    // test's own.
    fn filing_session(repo: &Path, home: &Path, linear: &Boarding) -> Driven {
        let mut driven = session_over(repo);
        driven.manifest = a_manifest();
        driven.pushes = Pushes::with_client(linear.clone(), Some(home.to_path_buf()));
        // The same home for a `/draft`, so the command reaches the board through
        // this test's sigils rather than being refused for want of one. The
        // client is the same stand-in because the session opens both through
        // one seam; the cut below is refused before a run starts.
        driven.cutter = Cutter::with_client(
            linear.clone(),
            Some(home.to_path_buf()),
            Scripted::saying([]),
            Scripted::saying([]),
        );
        driven
    }

    fn notes(driven: &Driven) -> Vec<String> {
        driven
            .app
            .panel()
            .thread()
            .map(|thread| thread.lines(Instant::now()))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|line| match line {
                Line::Note { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    // A command typed into the composer the way a reader types one: the block
    // arrives whole, as a terminal with bracketed paste hands it over, and the
    // Enter after it is the submit.
    fn typing(driven: &mut Driven, command: &str) {
        driven.app.set_focus(Focus::Composer);
        assert!(
            !driven.chat.composer().is_muted(),
            "the field is muted: a turn is still out"
        );
        driven.paste(command);
        assert!(
            pressed(driven, key(KeyCode::Enter)),
            "typing {command} ended the session"
        );
    }

    // Rounds until the push is in, drawn every time: the loop draws and then
    // waits, and a test that only drained would be a test of half a round.
    fn landing(driven: &mut Driven) {
        let waited = Instant::now();
        while driven.pushes.sending() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert!(!driven.pushes.sending(), "the push never reported");
    }

    // One turn of `run`'s loop with no event in it: draw, then everything that
    // happened off this thread.
    fn round(driven: &mut Driven) {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        driven.keep_up();
    }

    // `/push` and the two keys that answer its dialog Yes: No is lit when it
    // opens, so Left is what moves onto Yes and Enter is what sends.
    fn confirmed(driven: &mut Driven) {
        typing(driven, &format!("/push {SCOPE} {PATH}"));
        assert!(
            driven.pushes.window().confirm.is_open(),
            "the dialog did not come up: {:?}",
            notes(driven)
        );
        assert!(pressed(driven, key(KeyCode::Left)));
        assert!(pressed(driven, key(KeyCode::Enter)));
    }

    fn said(driven: &Driven, text: &str) -> bool {
        notes(driven).iter().any(|note| note.contains(text))
    }

    #[test]
    fn a_cut_typed_into_the_composer_reaches_the_board_and_reports_on_the_thread() {
        // The routing rather than the fetch: a `/draft` is a different thing
        // from a `/push`, and the loop has to answer it somewhere else. This
        // board holds no project, so the cut is refused on the far side of a
        // worker thread — which is the half being asserted, because the line
        // only reaches the thread if the loop drains the cut every round.
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing(URL);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        typing(&mut driven, &format!("/draft {SCOPE} 9e41c07a2b13"));
        let waited = Instant::now();
        while driven.cutter.fetching() && waited.elapsed() < AT_MOST {
            round(&mut driven);
        }

        assert!(!driven.cutter.fetching(), "the draft never reported");
        assert!(
            said(&driven, "no project with the slug `9e41c07a2b13`"),
            "the thread does not carry the refusal: {:?}",
            notes(&driven)
        );
        assert_eq!(
            linear.ops(),
            [Op::Viewer, Op::FetchProject],
            "a draft of an unknown project sent more than the read"
        );
        assert!(
            !driven.pushes.window().confirm.is_open(),
            "a draft put the push dialog up"
        );
    }

    #[test]
    fn a_bare_push_or_draft_is_the_refusal_and_reads_nothing() {
        // Neither command guesses: a `/push` without a scope and a brief, or a
        // `/draft` without a scope, is the one line naming what they take, and
        // no home, key or request is touched.
        for command in ["/push", &format!("/push {SCOPE}"), "/draft"] {
            let repo = a_repository();
            let home = a_home(repo.path());
            let linear = Boarding::unreachable();
            let mut driven = filing_session(repo.path(), home.path(), &linear);

            typing(&mut driven, command);
            round(&mut driven);

            let said = notes(&driven);
            assert_eq!(said.len(), 1, "{command} said {said:?}");
            assert!(
                said[0].contains("a scope and a brief for /push"),
                "{command} was not answered with the commands: {said:?}"
            );
            assert!(!driven.cutter.fetching(), "{command} started a fetch");
            assert!(
                !driven.pushes.window().confirm.is_open(),
                "{command} put the dialog up"
            );
        }
    }

    #[test]
    fn a_confirmed_push_files_the_brief_on_a_worker_and_says_where_it_landed() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing(URL);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        confirmed(&mut driven);

        // Said on the round the request started, before any of it came back:
        // the reader has just answered a question and is looking at the
        // conversation.
        assert!(
            said(&driven, &format!("filing to {TEAM}")),
            "the thread does not say the push started: {:?}",
            notes(&driven)
        );
        landing(&mut driven);

        assert!(
            said(&driven, URL),
            "the thread does not carry the project's address: {:?}",
            notes(&driven)
        );
        assert_eq!(
            linear.ops(),
            [
                Op::Team,
                Op::ProjectNamed,
                Op::BacklogStatus,
                Op::CreateProject
            ]
        );
        assert!(
            !repo.path().join(".warlock/filed.toml").exists(),
            "a push wrote something on this machine"
        );
    }

    #[test]
    fn the_loop_goes_round_while_the_request_is_in_flight() {
        // The point of the worker, and the one thing a push that has already
        // landed cannot show: the first request is held open, and the rounds
        // the loop takes while it sits there draw frames and answer keys.
        let repo = a_repository();
        let home = a_home(repo.path());
        let gate = Gate::shut();
        let linear = Boarding::filing(URL).held_at(&gate);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        confirmed(&mut driven);
        for _ in 0..ROUNDS {
            round(&mut driven);
        }

        assert!(
            driven.pushes.sending(),
            "the held request reported anyway: {:?}",
            notes(&driven)
        );
        assert!(
            said(&driven, &format!("filing to {TEAM}")),
            "the thread does not say the push started: {:?}",
            notes(&driven)
        );
        assert!(
            !said(&driven, URL),
            "a request nobody has answered came back"
        );
        // And the keys still mean what they meant: a push in flight is not a
        // window and holds nothing.
        assert!(
            pressed(&mut driven, key(KeyCode::Esc)),
            "a push in flight ended the session"
        );

        gate.open();
        landing(&mut driven);

        assert!(
            said(&driven, URL),
            "the released push never said where it landed: {:?}",
            notes(&driven)
        );
    }

    #[test]
    fn a_linear_that_refuses_lands_one_line_and_the_session_goes_on() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::refusing(REFUSED);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        confirmed(&mut driven);
        let before = notes(&driven).len();
        landing(&mut driven);

        let notes = notes(&driven);
        assert_eq!(
            notes.len(),
            before + 1,
            "a refusal is one line on the thread: {notes:?}"
        );
        let line = notes.last().expect("a refusal said something");
        assert!(line.contains(REFUSED), "{line} is not Linear's own words");
        assert_eq!(line.lines().count(), 1, "{line} is more than one line");
        // The session is where it was: the dialog is down, no push is in flight,
        // and the keyboard still works.
        assert!(!driven.pushes.window().confirm.is_open());
        assert!(!driven.pushes.sending());
        assert!(
            pressed(&mut driven, key(KeyCode::Esc)),
            "a refused push ended the session"
        );
    }

    #[test]
    fn a_second_push_while_one_is_in_flight_sends_nothing_and_says_so() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing(URL);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        confirmed(&mut driven);
        // Not a round in between, so the run is still the session's however fast
        // the worker was: what ends one is the drain, and this is a `/push`
        // typed before it.
        typing(&mut driven, &format!("/push {SCOPE} {PATH}"));

        assert!(
            said(&driven, ALREADY_FILING),
            "the second `/push` said nothing: {:?}",
            notes(&driven)
        );
        assert!(
            !driven.pushes.window().confirm.is_open(),
            "the second `/push` put a dialog up"
        );
        landing(&mut driven);
        assert_eq!(
            linear.projects_created().len(),
            1,
            "the brief was filed twice"
        );
    }

    #[test]
    fn a_push_of_a_brief_the_team_already_holds_says_where_it_is_and_creates_nothing() {
        // Nothing on this machine remembers a push, so a project of the same
        // name in the team is the board's answer to a second one: its address
        // on one line, and no second project under one title, which nothing on
        // this side could take back.
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing(URL).already_holding(URL);
        let mut driven = filing_session(repo.path(), home.path(), &linear);

        confirmed(&mut driven);
        let before = notes(&driven).len();
        landing(&mut driven);

        let notes = notes(&driven);
        assert_eq!(notes.len(), before + 1, "{notes:?}");
        let line = notes.last().expect("the refusal said something");
        assert!(line.contains("already filed"), "{line}");
        assert!(line.contains(URL), "{line} does not carry the address");
        assert_eq!(line.lines().count(), 1, "{line} is more than one line");
        assert!(
            linear.projects_created().is_empty(),
            "a second project was created"
        );
        assert!(!driven.pushes.window().confirm.is_open());
        assert!(!driven.pushes.sending());
        assert!(
            pressed(&mut driven, key(KeyCode::Esc)),
            "a refused push ended the session"
        );
    }

    #[test]
    fn no_key_value_reaches_the_thread_or_anything_the_session_holds() {
        // The one thing on this path that must never be printed: it is in the
        // key store this home holds, the client was built from it, and it is in
        // nothing the reader or a failing assertion anywhere else in the suite
        // would see.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = filing_session(repo.path(), home.path(), &Boarding::filing(URL));

        confirmed(&mut driven);
        let in_flight = format!("{:?}", driven.pushes);
        landing(&mut driven);

        assert!(!in_flight.contains(NOT_A_KEY), "the run carries the key");
        assert!(
            !format!("{:?}", driven.pushes).contains(NOT_A_KEY),
            "the session's push state carries the key"
        );
        assert!(
            !format!("{:?}", driven.pushes.window()).contains(NOT_A_KEY),
            "the window carries the key"
        );
        for note in notes(&driven) {
            assert!(!note.contains(NOT_A_KEY), "{note} carries the key");
        }
    }
}

// A `/draft` driven the whole way through a session — the command, the dialog's
// Yes, a slice that asks something, and the keys that edit and send the answer —
// over a Linear that answers one project out of memory and a home this module
// made. Nothing here opens a socket, reads a real credential or looks at the
// machine's own sigils, binding or key store.
//
// What is asserted is the routing, which is the half `tests/cutting.rs` cannot
// see: that the field is the composer's own value, that an Enter in it while a
// slice is waiting reaches that slice instead of starting a turn of the
// conversation, and that every editing key works on what warlock put there.
mod cutting {
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    use ratatui::crossterm::event::KeyCode;
    use tempfile::TempDir;
    use warlock_engine::{
        Manifest, PactEntry, ScopeRecord, save_key, save_key_binding, save_sigils,
    };

    use super::{AT_MOST, Driven, key, session_reading};
    use crate::account::Line;
    use crate::app::Focus;
    use crate::cutting::Cutter;
    use crate::linear::Listing;
    use crate::pushing::Pushes;
    use crate::stubs::{Answering, Boarding, Call, Scripted};

    // Not a key, and named so that nothing reading this file mistakes it for
    // one: it is stored so that a bound name resolves and the client is built
    // from something.
    const NOT_A_KEY: &str = "not-a-real-key-value";

    const KEY_NAME: &str = "this-tests-own-name";

    const SCOPE: &str = "data-plane";

    const TEAM: &str = "WAR";

    // What the command names the project by: the tail of its URL.
    const SLUG: &str = "1a2b3c";

    const NAME: &str = "Cut a planned project into tickets";

    // Two slices, so that what the run does after the question is something the
    // thread can be read for.
    const SLICED: &str = "Nothing cuts a planned project into tickets.\n\n## Scope\n\n\
                          ### 1. Read the project back\n\ndepends_on: []\n\n\
                          What it resolves.\n\n\
                          ### 2. Parse the scope block\n\ndepends_on: [1]\n\n\
                          What it parses.\n";

    const FIRST: &str = "Read the project back";

    const SECOND: &str = "Parse the scope block";

    const ASKED: &str = "Which of the two records does this slice write?";

    // Warlock's attempt, which the field is to hold as an ordinary draft: long
    // enough that a cursor at its end and a cursor anywhere else are different
    // places.
    const PROPOSED: &str = "The cut record, and nothing else.";

    fn a_manifest() -> Manifest {
        Manifest::with_entries([PactEntry::new(".", "docs", "docs/.warlock.md")
            .expect("a relative module path is inside the root")
            .with_scope(SCOPE)])
        .with_scopes([ScopeRecord::new(SCOPE, TEAM, "In Review", "warlock")])
    }

    // A repository the session loads, and nothing about the brief: what a cut
    // reads is the project on the board.
    fn a_repository() -> TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        let head = repo.path().join(".git/HEAD");
        fs::create_dir_all(head.parent().expect("`.git` is a directory"))
            .expect("a scratch directory is writable");
        fs::write(&head, "ref: refs/heads/main\n").expect("a scratch file is writable");
        a_manifest()
            .save(repo.path())
            .expect("a manifest that saves");
        repo
    }

    // A home of this test's own: the sigils that pick the board, the binding and
    // the key store all sit under it.
    fn a_home(root: &Path) -> TempDir {
        let home = tempfile::tempdir().expect("a temporary directory");
        save_sigils(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
        save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
        save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
        home
    }

    // The session `run` builds, with its impure things replaced: the manifest
    // that would have been loaded, a board that answers one project out of
    // memory, and the two conversations a cut opens.
    fn cutting_session(repo: &Path, home: &Path, agent: Scripted, proposer: Scripted) -> Driven {
        session_on(
            repo,
            home,
            &Boarding::holding(NAME, Some("Planned"), SLICED),
            agent,
            proposer,
        )
    }

    fn session_on(
        repo: &Path,
        home: &Path,
        linear: &Boarding,
        agent: Scripted,
        proposer: Scripted,
    ) -> Driven {
        let mut driven = session_reading(
            repo,
            Pushes::with_client(linear.clone(), Some(home.to_path_buf())),
            Cutter::with_client(linear.clone(), Some(home.to_path_buf()), agent, proposer),
            super::no_pull(linear.clone()),
        );
        driven.manifest = a_manifest();
        driven
    }

    // The slice's own script: it asks, it is answered, it drafts, and the slice
    // behind it drafts first time.
    fn a_slice_that_asks() -> Scripted {
        Scripted::saying([
            Answering::says(ASKED),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
        ])
    }

    fn notes(driven: &Driven) -> Vec<String> {
        driven
            .app
            .panel()
            .thread()
            .map(|thread| thread.lines(Instant::now()))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|line| match line {
                Line::Note { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    fn pressed(driven: &mut Driven, code: KeyCode) -> bool {
        driven
            .press(key(code), Instant::now())
            .expect("no key pressed here writes to a terminal")
    }

    // One turn of `run`'s loop with no event in it: draw, then everything that
    // happened off this thread. The draw is what tells the field its width and
    // what it is answering for, so a test that only drained would be a test of
    // half a round.
    fn round(driven: &mut Driven) {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        driven.keep_up();
    }

    // The command typed the way a reader types one: the block arrives whole, as
    // a terminal with bracketed paste hands it over, and the Enter after it is
    // the submit.
    fn typing(driven: &mut Driven, command: &str) {
        driven.app.set_focus(Focus::Composer);
        driven.paste(command);
        assert!(
            pressed(driven, KeyCode::Enter),
            "typing {command} ended the session"
        );
    }

    // `/draft` and the rounds the fetch takes, up to the dialog it puts on the
    // screen and no further: which key is pressed at it is the caller's.
    fn fetched(driven: &mut Driven) {
        typing(driven, &format!("/draft {SCOPE} {SLUG}"));
        let waited = Instant::now();
        while driven.cutter.fetching() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert!(
            driven.cutter.confirm().is_open(),
            "the dialog did not come up: {:?}",
            notes(driven)
        );
    }

    // The same, with the two keys that answer the dialog Yes: No is lit when it
    // opens, so Left is what moves onto Yes.
    fn confirmed(driven: &mut Driven) {
        fetched(driven);
        assert!(pressed(driven, KeyCode::Left));
        assert!(pressed(driven, KeyCode::Enter));
    }

    // Rounds until warlock's attempt at the question is in the field, which is
    // two things arriving on two rounds: the question, and then the attempt.
    fn offered(driven: &mut Driven) {
        let waited = Instant::now();
        while driven.chat.composer().ghost().is_none() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert!(
            driven.chat.composer().ghost().is_some(),
            "nothing was ever suggested for the field: {:?}",
            notes(driven)
        );
    }

    // Rounds until the run is over, so the suite leaves no worker parked, with
    // the two windows a run puts up answered by the keys that answer them:
    // `s` files nothing for the slice, and the carry-on question behind it is a
    // Left onto Yes and an Enter, exactly as the dialog before the run was.
    //
    // Skip and not Create, because what these tests are about is the relay: the
    // drafts a slice settles on are `cutting.rs`'s own to be answered about.
    fn through(driven: &mut Driven) {
        let waited = Instant::now();
        while driven.cutter.drafting() && waited.elapsed() < AT_MOST {
            round(driven);
            if driven.cutter.reviewing().is_some() {
                assert!(pressed(driven, KeyCode::Char('s')));
            }
            if driven.cutter.carrying().is_some() {
                assert!(pressed(driven, KeyCode::Left));
                assert!(pressed(driven, KeyCode::Enter));
            }
        }
        assert!(!driven.cutter.drafting(), "the run never finished");
    }

    #[test]
    fn a_draft_with_only_a_scope_lists_the_planned_projects_one_line_each() {
        // No picker and no dialog: the slugs land on the thread as lines a
        // reader can copy, and the next `/draft` names one.
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing("").listing(Listing::new(&[
            ("9e41c07a2b13", "Draft from the board"),
            ("d1cb3521be71", "Give the headless CLI a voice"),
        ]));
        let mut driven = session_on(
            repo.path(),
            home.path(),
            &linear,
            Scripted::saying([]),
            Scripted::saying([]),
        );

        typing(&mut driven, &format!("/draft {SCOPE}"));
        let waited = Instant::now();
        while driven.cutter.fetching() && waited.elapsed() < AT_MOST {
            round(&mut driven);
        }

        // Compared word by word: the thread lays a line out for the screen, and
        // the run of spaces between slug and name is not what is under test.
        let said = notes(&driven);
        let listed: Vec<String> = said[said.len() - 2..]
            .iter()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(
            listed,
            [
                "9e41c07a2b13 Draft from the board",
                "d1cb3521be71 Give the headless CLI a voice",
            ],
            "{said:?}"
        );
        assert!(
            !driven.cutter.confirm().is_open(),
            "a listing put the dialog up"
        );
        assert_eq!(
            linear.calls(),
            [Call::PlannedProjects {
                team: TEAM.to_owned(),
                label: "warlock".to_owned(),
            }],
            "a listing read more than the list"
        );
    }

    #[test]
    fn a_draft_naming_a_slug_reads_that_project_and_asks_about_it() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::holding(NAME, Some("Planned"), SLICED);
        let mut driven = session_on(
            repo.path(),
            home.path(),
            &linear,
            Scripted::saying([]),
            Scripted::saying([]),
        );

        fetched(&mut driven);

        assert!(
            linear
                .calls()
                .contains(&Call::FetchProject(SLUG.to_owned())),
            "the slug typed is not the one read: {:?}",
            linear.calls()
        );
        assert_eq!(
            driven
                .cutter
                .confirm()
                .cutting()
                .map(crate::confirm::Cutting::project),
            Some(NAME)
        );
    }

    #[test]
    fn a_no_at_the_dialog_ends_the_cut_and_opens_no_session_at_all() {
        // The dialog's two No paths as a reader reaches them through the loop:
        // Esc, and the reflex Enter on the round it came up. Both are the same
        // answer, and what it costs is the window — nothing is drafted, neither
        // conversation is opened, and the session is where it was, so the next
        // `/draft` asks the same question again.
        for code in [KeyCode::Esc, KeyCode::Enter] {
            let repo = a_repository();
            let home = a_home(repo.path());
            let agent = a_slice_that_asks();
            let proposer = Scripted::saying([Answering::says(PROPOSED)]);
            let mut driven =
                cutting_session(repo.path(), home.path(), agent.clone(), proposer.clone());

            fetched(&mut driven);
            let said = notes(&driven).len();
            assert!(pressed(&mut driven, code), "{code:?} ended the session");

            assert!(
                !driven.cutter.confirm().is_open(),
                "{code:?} left the dialog up"
            );
            assert!(!driven.cutter.drafting(), "{code:?} started a run");
            // A round after it, because a run that started would start on the
            // loop's next pass rather than on the key.
            round(&mut driven);
            assert!(
                !driven.cutter.drafting(),
                "{code:?} started a run a beat later"
            );
            assert_eq!(agent.turns(), 0, "{code:?} opened a drafting session");
            assert_eq!(
                proposer.turns(),
                0,
                "{code:?} opened the other conversation"
            );
            assert_eq!(notes(&driven).len(), said, "{code:?} said something");
            // And the value is back where a session with no cut in it sits.
            fetched(&mut driven);
        }
    }

    #[test]
    fn warlocks_attempt_is_a_suggestion_and_never_part_of_the_draft() {
        // Shown dimmed in the empty field and not written into it, so nobody
        // has to delete it to type their own answer.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            a_slice_that_asks(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);

        assert_eq!(driven.chat.composer().draft(), "");
        assert_eq!(driven.chat.composer().ghost(), Some(PROPOSED));
        assert!(
            !driven.chat.answering(),
            "the suggestion started a turn of the conversation"
        );
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);
    }

    #[test]
    fn right_takes_the_suggestion_into_the_field_to_edit() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            a_slice_that_asks(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        assert!(pressed(&mut driven, KeyCode::Right));

        assert_eq!(driven.chat.composer().draft(), PROPOSED);
        assert_eq!(driven.chat.composer().cursor(), PROPOSED.len());
        assert!(pressed(&mut driven, KeyCode::Backspace));
        assert_eq!(
            driven.chat.composer().draft(),
            &PROPOSED[..PROPOSED.len() - 1]
        );
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);
    }

    #[test]
    fn an_enter_while_a_slice_waits_answers_it_rather_than_starting_a_turn() {
        // The whole of the routing: the field is the conversation's, and what
        // decides where a submission goes is the cut in flight and nothing on
        // the chat at all.
        let repo = a_repository();
        let home = a_home(repo.path());
        let agent = a_slice_that_asks();
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        assert!(pressed(&mut driven, KeyCode::Enter));

        assert!(
            !driven.chat.answering(),
            "the answer started a turn of the conversation"
        );
        assert_eq!(
            driven.chat.composer().draft(),
            "",
            "the field kept the answer that was sent"
        );
        assert_eq!(
            driven.chat.composer().ghost(),
            None,
            "the field kept the suggestion that was sent"
        );
        through(&mut driven);
        assert!(
            agent.said().iter().any(|turn| turn == PROPOSED),
            "the answer did not reach the session that asked: {:?}",
            agent.said()
        );
        let said = notes(&driven);
        let asked = driven.app.panel().thread().is_some_and(|thread| {
            thread
                .turns()
                .iter()
                .any(|turn| turn.answer() == Some(ASKED))
        });
        assert!(asked, "the question is not on the thread: {said:?}");
        assert!(
            said.iter()
                .any(|line| line.contains("was answered:") && line.contains(PROPOSED)),
            "what was sent is not on the thread: {said:?}"
        );
    }

    #[test]
    fn what_is_typed_over_a_suggestion_is_what_the_slice_is_told() {
        // Enter sends whatever the field holds. Warlock's attempt has no
        // standing over it: anything typed is what goes, with nothing to delete
        // first.
        let repo = a_repository();
        let home = a_home(repo.path());
        let agent = a_slice_that_asks();
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        assert!(pressed(&mut driven, KeyCode::Char('N')));
        assert!(pressed(&mut driven, KeyCode::Char('o')));
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);

        assert!(
            agent.said().iter().any(|turn| turn == "No"),
            "what was typed did not reach the session: {:?}",
            agent.said()
        );
        assert!(
            !agent.said().iter().any(|turn| turn == PROPOSED),
            "warlock's own draft was sent instead: {:?}",
            agent.said()
        );
    }

    #[test]
    fn the_field_says_which_slice_it_is_answering_for_and_stops_when_it_is_over() {
        // Told once a round by the draw, off the cut, so a field cannot be left
        // labelled for a question that is over.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            a_slice_that_asks(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        // `offered` stops on the round the attempt lands, and the label is told
        // by the draw at the top of a round. The loop always draws again before
        // anybody sees the field, so this is that draw and not a second beat.
        round(&mut driven);

        assert_eq!(
            driven.chat.composer().answering(),
            Some(format!("answering slice 1 `{FIRST}`").as_str())
        );
        assert!(pressed(&mut driven, KeyCode::Enter));
        round(&mut driven);
        assert_eq!(
            driven.chat.composer().answering(),
            None,
            "the field is still labelled for a question that is over"
        );
        through(&mut driven);
    }

    #[test]
    fn a_command_typed_into_the_field_while_a_slice_waits_is_the_answer() {
        // Nothing reads what is sent: the slice asked a question of its own, and
        // a `/chat` in the answer is the reader's word rather than a register to
        // change.
        let repo = a_repository();
        let home = a_home(repo.path());
        let agent = a_slice_that_asks();
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        driven.paste("/chat");
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);

        assert!(
            agent.said().iter().any(|turn| turn == "/chat"),
            "the command was not sent as the answer it was: {:?}",
            agent.said()
        );
        assert!(
            !driven.chat.answering(),
            "a command in an answer started a turn of the conversation"
        );
    }

    #[test]
    fn the_second_slice_is_drafted_once_the_first_has_been_answered() {
        // The run goes on where it left off: the slice that asked drafts on the
        // turn after the answer, and the slice behind it is reached.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            a_slice_that_asks(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);

        let said = notes(&driven);
        assert!(
            said.iter()
                .any(|line| line.contains(SECOND) && line.contains("drafted `")),
            "the run did not reach the second slice: {said:?}"
        );
        for note in &said {
            assert!(!note.contains(NOT_A_KEY), "{note} carries the key");
        }
    }

    #[test]
    fn no_key_value_reaches_the_thread_or_anything_the_session_holds() {
        // `filing`'s claim, made again for the other command and for the one
        // place only a cut has: the field. Warlock's attempt is put there by
        // the run, so the composer is on this path as much as the thread is,
        // and the value out of this home's key store is to be in neither — nor
        // in the `Debug` rendering a failing assertion elsewhere would print.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut driven = cutting_session(
            repo.path(),
            home.path(),
            a_slice_that_asks(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );

        confirmed(&mut driven);
        offered(&mut driven);
        // Read with a slice still waiting, so what is asserted is the run in
        // flight as well as the run that is over.
        let in_flight = format!("{:?}", driven.cutter);
        assert!(pressed(&mut driven, KeyCode::Enter));
        through(&mut driven);

        assert!(!in_flight.contains(NOT_A_KEY), "the run carries the key");
        assert!(
            !format!("{:?}", driven.cutter).contains(NOT_A_KEY),
            "the session's draft state carries the key"
        );
        assert!(
            !driven.chat.composer().draft().contains(NOT_A_KEY),
            "the field carries the key"
        );
        for note in notes(&driven) {
            assert!(!note.contains(NOT_A_KEY), "{note} carries the key");
        }
    }
}

/// The seventh command, driven end to end through the loop it really runs in: a
/// ticket taken, a run stopped, and everything that races a run in flight refused
/// while it is in flight.
///
/// Nothing here opens a socket, runs a `git`, raises a `claude` or reads the
/// sigils, the binding or the key store of the machine the suite runs on. The
/// board, the checkout, the forge and the three sessions one pull spends are all
/// stand-ins, and every test builds a temporary home of its own — a session built
/// with none is refused before anything under `~` is read, which is why `driving`
/// above gives one to nothing.
mod pulling {
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    use ratatui::crossterm::event::KeyCode;
    use tempfile::TempDir;
    use warlock_engine::{
        Manifest, PactEntry, PullRun, PullSubtask, RunStatus, ScopeRecord, SubtaskStatus, save_key,
        save_key_binding, save_sigils, state_path,
    };

    use super::{AT_MOST, Session, Stubbed, key, session_reading};
    use crate::account::Line;
    use crate::app::Focus;
    use crate::claude::Activity;
    use crate::cutting::Cutter;
    use crate::git::Dirty;
    use crate::linear::{Assignee, NamedIssue, Priority, Queue, QueuedIssue, StateType};
    use crate::prompt::ScopePrompt;
    use crate::puller::Puller;
    use crate::pushing::Pushes;
    use crate::stubs::{
        Boarding, Checkout, Forging, Op, Refreshing, Scripted, Sessions, Slicing, VIEWER, Written,
        said,
    };
    use crate::submission::submitted_for;

    // Not a key, and named so that nothing reading this file mistakes it for one:
    // it is stored only so that a bound name resolves.
    const NOT_A_KEY: &str = "not-a-real-key-value";

    const KEY_NAME: &str = "this-tests-own-name";

    const SCOPE: &str = "warlock-team";

    // A scope this repository records that the home below does not hold, which is
    // what a sub-task writing under it is a crossing of.
    const CLOSED: &str = "control-plane";

    const TEAM: &str = "WAR";

    const LABEL: &str = "warlock";

    const DEFAULT: &str = "main";

    const TICKET: &str = "WAR-140";

    const ISSUE: &str = "issue-140";

    const TITLE: &str = "Add `warlock pull <SCOPE>`";

    // A second ticket, halted on this machine and never pulled here: what a
    // `/resume` refused during a run is asserted to have left alone.
    const OTHER: &str = "WAR-141";

    const URL: &str = "https://github.com/team/repo/pull/12";

    const WROTE: &str = "crates/engine/src/lib.rs";

    // What every refusal during a run says it did not do, beside the one sentence
    // naming the pull. Each is the wording of the module that refuses, so a test
    // asserting on it is asserting that the keystroke reached that module.
    const NO_PASS: &str = "no pass was started";

    const NO_SCOPE: &str = "no scope was written";

    // And the composer's own, which is what a `/draft` or a `/resume` typed during
    // a run meets: the Enter is taken before the conversation reads the draft, so
    // the command is never recognised and neither module is reached at all.
    const UNSENT: &str = "nothing was sent, and quitting warlock stops the pull";

    // Two recorded scopes, one pacted directory under each, so a run that stays
    // inside its boundary and one that writes past it are both reachable from the
    // one manifest.
    fn a_manifest() -> Manifest {
        Manifest::with_entries([
            pacted("crates/engine", SCOPE),
            pacted("crates/control", CLOSED),
        ])
        .with_scopes([
            ScopeRecord::new(SCOPE, TEAM, "In Review", LABEL),
            ScopeRecord::new(CLOSED, "CTL", "In Review", LABEL),
        ])
    }

    fn pacted(directory: &str, scope: &str) -> PactEntry {
        PactEntry::new(".", directory, format!("{directory}/.warlock.md"))
            .expect("a relative module path is inside the root")
            .with_scope(scope)
    }

    // The checkout the session loads its tree from: a file under each pacted
    // directory, and the `.git` that makes it a repository to everything that
    // looks. No `git` is ever run in it — the repository seam is a stand-in.
    fn a_repository() -> TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        for (path, text) in [
            (".git/HEAD", "ref: refs/heads/main\n"),
            (WROTE, "//! Core engine.\n"),
            ("crates/control/src/lib.rs", "//! The control plane.\n"),
        ] {
            let at = repo.path().join(path);
            fs::create_dir_all(at.parent().expect("every path here has a parent"))
                .expect("a scratch directory is writable");
            fs::write(&at, text).expect("a scratch file is writable");
        }
        a_manifest()
            .save(repo.path())
            .expect("a manifest that saves");
        repo
    }

    // A home of this test's own: the sigils that decide which scopes may be
    // pulled, the binding, the key store and the run records all sit under it.
    fn a_home(root: &Path) -> TempDir {
        let home = tempfile::tempdir().expect("a temporary directory");
        save_sigils(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
        save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
        save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
        home
    }

    type Pulls = Session<Stubbed<Boarding, Scripted, Written>>;

    // The session `run` builds, with its impure things replaced: the board a pull
    // reads its queue from, the checkout it works in, the forge it opens a request
    // on, and the three sessions it spends.
    //
    // The push and the cut share the board and are built with no home, as they are
    // everywhere else in this file: what a `/draft` typed during a run does is the
    // refusal above every other rule it has, and a cut that got past it would need
    // a home to resolve a board under.
    fn pulling_session(
        repo: &Path,
        home: &Path,
        board: Boarding,
        checkout: Checkout,
        raises: Written,
    ) -> Pulls {
        let mut driven = session_reading(
            repo,
            Pushes::with_client(board.clone(), None),
            Cutter::with_client(
                board.clone(),
                None,
                Scripted::saying([]),
                Scripted::saying([]),
            ),
            Puller::with_seams(
                board,
                checkout,
                Forging::opening(URL),
                raises,
                Some(home.to_path_buf()),
            ),
        );
        driven.manifest = a_manifest();
        driven
    }

    // A run that works one sub-task and reports on the way: enough for the account
    // to have something under its headings, and short enough that a test which
    // drives one to the end is not driving two.
    fn working() -> Written {
        Written::of(
            Slicing::into_chain(TICKET, &["Read the queue"]),
            Sessions::answering([said("done", "the queue is read", None)]),
            Refreshing::quiet(),
        )
        .doing([
            Activity::Tool {
                name: "Read".to_owned(),
                detail: Some(WROTE.to_owned()),
            },
            Activity::Thinking,
            Activity::Writing { bytes: 512 },
            Activity::Cost { usd: 0.42 },
        ])
    }

    fn queue(issues: impl IntoIterator<Item = QueuedIssue>) -> Queue {
        Queue::new(issues.into_iter().collect())
    }

    fn ready() -> QueuedIssue {
        QueuedIssue::new(
            ISSUE,
            TICKET,
            TITLE,
            "Todo",
            StateType::new("unstarted"),
            Priority::Urgent,
            Vec::new(),
        )
    }

    // The board a `/pull <SCOPE>` reads, and — for the resume below — the one a
    // `/pull <SCOPE> <TICKET>` names.
    fn board() -> Boarding {
        Boarding::filing(URL).queueing(queue([ready()]))
    }

    fn naming() -> Boarding {
        Boarding::filing(URL).naming(NamedIssue::new(
            ready(),
            TEAM,
            vec![LABEL.to_owned()],
            Some(Assignee::new(VIEWER, "Ada")),
        ))
    }

    fn wrote(path: &str) -> Vec<Dirty> {
        vec![Dirty {
            code: " M".to_owned(),
            path: path.to_owned(),
            from: None,
        }]
    }

    // Clean for the look before the board is opened, and holding the session's
    // work for every look after it.
    fn checkout() -> Checkout {
        Checkout::clean(DEFAULT).trees([Vec::new(), wrote(WROTE)])
    }

    fn notes(driven: &Pulls) -> Vec<String> {
        driven
            .app
            .panel()
            .thread()
            .map(|thread| thread.lines(Instant::now()))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|line| match line {
                Line::Note { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    // How many of the run's phases the thread holds: a note with a work turn
    // directly under it.
    fn sections(driven: &Pulls) -> usize {
        let lines = driven
            .app
            .panel()
            .thread()
            .map(|thread| thread.lines(Instant::now()))
            .unwrap_or_default();
        lines
            .windows(2)
            .filter(|pair| matches!(pair, [Line::Note { .. }, Line::Clocked { .. }]))
            .count()
    }

    fn pressed(driven: &mut Pulls, code: KeyCode) -> bool {
        driven
            .press(key(code), Instant::now())
            .expect("no key pressed here writes to a terminal")
    }

    // One turn of `run`'s loop with no event in it: draw, then everything that
    // happened off this thread. The draw is what tells the cards their width, so a
    // test that only drained would be a test of half a round.
    fn round(driven: &mut Pulls) {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        driven.keep_up();
    }

    // The command typed the way a reader types one: the block arrives whole, as a
    // terminal with bracketed paste hands it over, and the Enter after it is the
    // submit.
    fn typing(driven: &mut Pulls, command: &str) {
        driven.app.set_focus(Focus::Composer);
        driven.paste(command);
        assert!(
            pressed(driven, KeyCode::Enter),
            "typing {command} ended the session"
        );
    }

    // `/pull` and the rounds the queue takes, up to the dialog and no further:
    // which key is pressed at it is the caller's.
    fn asked(driven: &mut Pulls, command: &str) {
        typing(driven, command);
        let waited = Instant::now();
        while driven.puller.choosing() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert!(
            driven.puller.confirm().is_open(),
            "the dialog did not come up: {:?}",
            notes(driven)
        );
    }

    // The same, answered Yes: No is lit when it opens, so Left is what moves onto
    // Yes. The keyboard is left on the tree, which is where every key the tests
    // below press belongs.
    fn started(driven: &mut Pulls) {
        asked(driven, &format!("/pull {SCOPE}"));
        assert!(pressed(driven, KeyCode::Left));
        assert!(pressed(driven, KeyCode::Enter));
        assert!(driven.puller.pulling(), "the Yes started no run");
        driven.app.set_focus(Focus::Tree);
    }

    // Rounds until the run is over, so the suite leaves no worker parked.
    fn through(driven: &mut Pulls) {
        let waited = Instant::now();
        while driven.puller.pulling() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert!(!driven.puller.pulling(), "the run never finished");
    }

    // A run in flight with a sub-task that answers nothing until it is stopped:
    // what every test about racing a pull, and the one about quitting, is driven
    // over. Rounds until the session has really been raised, so a test that asserts
    // about stopping one is not asserting about a run that had not started it.
    fn held(driven: &mut Pulls, raises: &Written) {
        started(driven);
        let waited = Instant::now();
        while raises.raised().is_empty() && waited.elapsed() < AT_MOST {
            round(driven);
        }
        assert_eq!(
            raises.raised().len(),
            1,
            "the run never raised the sub-task's session"
        );
    }

    // A halted run of the other ticket, written straight to the home: what a
    // `/resume` would release if it were allowed to read anything.
    fn halted(home: &Path, root: &Path) {
        let mut run = PullRun::new(
            OTHER,
            "Add `warlock resume <TICKET>`",
            SCOPE,
            "war-141/add-warlock-resume-ticket",
            "2026-09-28T09:00:00+00:00",
        );
        let mut subtask = PullSubtask::new(
            format!("{OTHER}.01"),
            "Read the record back",
            Vec::<String>::new(),
        );
        subtask.set_status(SubtaskStatus::Failed(
            "the attempt was cancelled".to_owned(),
        ));
        run.push_subtask(subtask);
        run.set_status(RunStatus::Halted);
        run.save(home, root).expect("a record that writes");
    }

    #[test]
    fn a_word_nobody_made_a_command_is_refused_in_the_sentence_naming_all_seven() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = Boarding::unopened();
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            linear.clone(),
            checkout(),
            Written::of(
                Slicing::into_chain(TICKET, &["Nothing this test lets a run reach"]),
                Sessions::answering([]),
                Refreshing::quiet(),
            ),
        );

        typing(&mut driven, "/pullll warlock-team");

        let refusal = submitted_for("/pullll")
            .refusal()
            .expect("a word nobody made a command is refused");
        assert_eq!(notes(&driven), vec![refusal.to_owned()]);
        for command in [
            "/brief", "/write", "/chat", "/push", "/draft", "/pull", "/resume",
        ] {
            assert!(
                refusal.contains(command),
                "{refusal:?} does not name {command}"
            );
        }
        assert!(!driven.puller.choosing(), "a refusal read a queue");
        assert!(!driven.puller.pulling(), "a refusal started a run");
        assert_eq!(linear.requests(), 0, "a refusal reached the board");
    }

    #[test]
    fn esc_and_an_immediate_enter_at_the_dialog_are_both_a_no_that_starts_nothing() {
        // The dialog's two No paths as a reader reaches them through the loop: Esc,
        // and the reflex Enter on the round it came up. Both are the same answer,
        // and what it costs is the window — no branch is cut, no ticket moves, no
        // session is raised, and the next `/pull` may choose the same ticket again.
        for code in [KeyCode::Esc, KeyCode::Enter] {
            let repo = a_repository();
            let home = a_home(repo.path());
            let raises = working();
            let repository = checkout();
            let mut driven = pulling_session(
                repo.path(),
                home.path(),
                board(),
                repository.clone(),
                raises.clone(),
            );

            asked(&mut driven, &format!("/pull {SCOPE}"));
            assert!(pressed(&mut driven, code));

            assert!(
                !driven.puller.confirm().is_open(),
                "{code:?} left the question up"
            );
            assert!(!driven.puller.pulling(), "{code:?} started a run");
            assert!(
                raises.raised().is_empty(),
                "{code:?} raised a session: {:?}",
                raises.raised()
            );
            assert_eq!(
                repository.commits(),
                Vec::<String>::new(),
                "{code:?} committed something"
            );
            assert!(
                !state_path(home.path(), repo.path(), TICKET).exists(),
                "{code:?} wrote a run record"
            );
        }
    }

    #[test]
    fn a_no_at_the_resuming_dialog_leaves_the_run_exactly_where_it_was() {
        let repo = a_repository();
        let home = a_home(repo.path());
        // The halt a `/resume` released, which is the run selection carries on from
        // rather than taking a fresh ticket for.
        let mut run = PullRun::new(
            TICKET,
            TITLE,
            SCOPE,
            "war-140/add-warlock-pull-scope",
            "2026-09-28T09:00:00+00:00",
        );
        run.push_subtask(PullSubtask::new(
            format!("{TICKET}.01"),
            "Read the queue",
            Vec::<String>::new(),
        ));
        run.set_status(RunStatus::Resumed);
        run.save(home.path(), repo.path())
            .expect("a record that writes");
        let before =
            fs::read(state_path(home.path(), repo.path(), TICKET)).expect("the record is on disk");
        let raises = working();
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            naming(),
            checkout(),
            raises.clone(),
        );

        asked(&mut driven, &format!("/pull {SCOPE} {TICKET}"));

        let undertaking = driven
            .puller
            .confirm()
            .undertaking()
            .expect("the dialog is up")
            .clone();
        assert_eq!(
            undertaking.resuming(),
            Some(format!("{TICKET}.01").as_str()),
            "the question does not name the sub-task the run carries on from"
        );
        assert!(pressed(&mut driven, KeyCode::Esc));

        assert!(!driven.puller.pulling(), "a No started a run");
        assert!(raises.raised().is_empty(), "a No raised a session");
        assert_eq!(
            fs::read(state_path(home.path(), repo.path(), TICKET))
                .expect("the record is still on disk"),
            before,
            "a No rewrote the run record"
        );
    }

    #[test]
    fn a_halt_lands_on_the_thread_as_one_line_and_the_panel_goes_on_running() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let repository = checkout();
        let raises = working().sessioning(Sessions::answering([said(
            "blocked",
            "the scope is somebody else's",
            Some("a decision only the human can make"),
        )]));
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            board(),
            repository.clone(),
            raises,
        );

        started(&mut driven);
        through(&mut driven);

        let said = notes(&driven).pop().expect("the halt said nothing");
        assert!(said.contains(TICKET), "{said:?} does not name the ticket");
        let run =
            PullRun::load(home.path(), repo.path(), TICKET).expect("the run wrote its record");
        assert_eq!(run.status(), RunStatus::Halted);
        assert_eq!(
            repository.commits(),
            Vec::<String>::new(),
            "a halted sub-task was committed"
        );
        // The panel goes on running: the loop goes round, the thread still holds
        // what the run did, and the next `/pull` is allowed.
        round(&mut driven);
        assert!(sections(&driven) > 0, "the halt took the run's phases down");
        assert!(
            driven.puller.in_flight().is_none(),
            "a halted run still holds the tree"
        );
    }

    #[test]
    fn a_crossing_names_the_sub_task_that_wrote_past_the_boundary() {
        let repo = a_repository();
        let home = a_home(repo.path());
        // The session left a file under the scope this machine does not hold,
        // which is what the check after it is about.
        let repository =
            Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/control/src/lib.rs")]);
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            board(),
            repository.clone(),
            working(),
        );

        started(&mut driven);
        through(&mut driven);

        let said = notes(&driven).pop().expect("the crossing said nothing");
        assert!(
            said.contains(&format!("{TICKET}.01")),
            "{said:?} does not name the sub-task that crossed"
        );
        assert_eq!(
            repository.commits(),
            Vec::<String>::new(),
            "a crossing was committed"
        );
        assert_eq!(
            PullRun::load(home.path(), repo.path(), TICKET)
                .expect("the run wrote its record")
                .status(),
            RunStatus::Halted
        );
    }

    #[test]
    fn the_composer_starts_no_turn_while_a_pull_is_in_flight() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let raises = working().waiting();
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            board(),
            checkout(),
            raises.clone(),
        );
        held(&mut driven, &raises);
        let before = notes(&driven).len();

        typing(&mut driven, "What does the tree hold?");

        let said = notes(&driven);
        assert_eq!(
            said.len(),
            before + 1,
            "the submit said something other than one line: {said:?}"
        );
        let locked = said.last().expect("the submit said nothing");
        assert!(locked.contains(TICKET), "{locked:?} does not name the pull");
        assert!(
            locked.contains("quitting warlock"),
            "{locked:?} does not say how to stop it"
        );
        assert!(!driven.chat.answering(), "the submit opened a turn");
        assert_eq!(
            driven.chat.composer().draft(),
            "What does the tree hold?",
            "the draft was sent rather than left in the field"
        );
        // The thread card is still a thread: the account is the run's output
        // window, and what a reader typed into is readable under it.
        round(&mut driven);
        assert!(
            driven.app.panel().thread().is_some(),
            "the thread card went"
        );
    }

    #[test]
    fn every_key_and_command_that_races_a_pull_is_refused_with_the_line_naming_it() {
        let repo = a_repository();
        let home = a_home(repo.path());
        halted(home.path(), repo.path());
        let record = state_path(home.path(), repo.path(), OTHER);
        let before = fs::read(&record).expect("the halted record is on disk");
        let raises = working().waiting();
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            board(),
            checkout(),
            raises.clone(),
        );
        held(&mut driven, &raises);
        let already = notes(&driven).len();

        // The three keys first, with the keyboard on the tree, and then the two
        // commands, which need it in the field: a `p` typed into the composer is
        // a letter and not the pact key.
        for code in [KeyCode::Char('p'), KeyCode::Char('r'), KeyCode::Char('s')] {
            assert!(pressed(&mut driven, code), "{code:?} ended the session");
        }
        typing(&mut driven, "/draft docs/brief.md");
        typing(&mut driven, &format!("/resume {OTHER}"));

        // Five keystrokes, five lines, and every one of them names the pull. The
        // three keys are refused by the module each would have started, in its
        // own words; the two commands never reach theirs, because the Enter that
        // carries them is taken by the locked composer before the conversation
        // reads the draft — which is the stronger promise of the two, since a
        // command that was never recognised cannot have done anything.
        let said: Vec<String> = notes(&driven).split_off(already);
        assert_eq!(said.len(), 5, "five refusals said {said:?}");
        for (line, what) in said
            .iter()
            .zip([NO_PASS, NO_PASS, NO_SCOPE, UNSENT, UNSENT])
        {
            assert!(line.contains(TICKET), "{line:?} does not name the pull");
            assert!(line.contains(what), "{line:?} does not say {what:?}");
        }
        assert!(!driven.chat.answering(), "a refused command opened a turn");
        assert!(!driven.pact.running(), "a refused key started a pass");
        assert_eq!(
            driven.prompt,
            ScopePrompt::Closed,
            "a refused `s` opened the window"
        );
        assert!(
            !driven.cutter.fetching(),
            "a refused `/draft` read the board"
        );
        assert_eq!(
            fs::read(&record).expect("the halted record is still on disk"),
            before,
            "a refused `/resume` rewrote a run record"
        );
    }

    #[test]
    fn quitting_with_a_pull_in_flight_stops_the_session_and_records_the_halt() {
        let repo = a_repository();
        let home = a_home(repo.path());
        let linear = board();
        let repository = checkout();
        let raises = working().waiting();
        let mut driven = pulling_session(
            repo.path(),
            home.path(),
            linear.clone(),
            repository.clone(),
            raises.clone(),
        );
        held(&mut driven, &raises);

        // The reader's own way out: the question, Left onto Yes, and the Enter
        // that ends the session. `run` returns on that answer, and returning is
        // what drops the pull — which is the whole of how a run is stopped.
        assert!(pressed(&mut driven, KeyCode::Char('q')));
        assert!(pressed(&mut driven, KeyCode::Left));
        assert!(!pressed(&mut driven, KeyCode::Enter), "the session goes on");
        drop(driven);

        let waited = Instant::now();
        let run = loop {
            if let Ok(run) = PullRun::load(home.path(), repo.path(), TICKET)
                && run.status() == RunStatus::Halted
            {
                break run;
            }
            assert!(
                waited.elapsed() < AT_MOST,
                "the stopped run never recorded its halt"
            );
        };
        let subtask = run
            .subtasks()
            .first()
            .expect("the run split into sub-tasks");
        assert!(
            matches!(subtask.status(), SubtaskStatus::Failed(why) if why.contains("cancelled")),
            "the sub-task in flight is {:?} rather than failed as cancelled",
            subtask.status()
        );
        assert_eq!(
            repository.commits(),
            Vec::<String>::new(),
            "a cancelled run committed the tree"
        );
        assert!(
            !linear.ops().contains(&Op::IssueComment),
            "a cancelled run commented on the ticket: {:?}",
            linear.ops()
        );
    }
}
