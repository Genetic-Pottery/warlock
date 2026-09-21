use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tempfile::TempDir;
use warlock_engine::{Node, NodeState, Tree, manifest_path, to_manifest_path};
use warlock_tui::{App, Line, ScopeField, ScopePrompt};

use super::NO_PATH;

// The prompt half of what a write answers with, which is what every test below
// but `what_was_written_is_remembered_…` is about. The path it hands back is
// asserted there, against the same function.
fn write_submit(app: &mut App, root: &Path, field: &ScopeField, now: Instant) -> ScopePrompt {
    super::write_submit(app, root, field, now).prompt
}

const BRIEF: &str = "docs/warlock-brief-13-scopes-and-sigils.md";

const LAST_KEY: &str = "something the last key said";

fn a_repo() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn now() -> Instant {
    Instant::now()
}

// The reply goes on the card and nowhere else, because the card is where the
// write reads it back from.
fn app_answering(root: &Path, reply: &str) -> App {
    let mut app = App::from_tree(&Tree::new(Node::new(
        root,
        None::<PathBuf>,
        NodeState::Unpacted,
    )));
    app.panel_mut().start_turn("/write", now());
    app.panel_mut().answer_turn(reply, now());
    app.set_message(LAST_KEY);
    app
}

fn field(path: &str) -> ScopeField {
    ScopeField::new("Write to", path)
}

fn notes(app: &App) -> Vec<String> {
    app.panel()
        .thread()
        .expect("the conversation is there")
        .lines(now())
        .into_iter()
        .filter_map(|line| match line {
            Line::Note { text } => Some(text),
            _ => None,
        })
        .collect()
}

fn everything_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .expect("a directory of this test's own reads back")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path.clone());
            }
            found.push(to_manifest_path(root, &path).expect("inside the repository"));
        }
    }
    found.sort();
    found
}

// Carries every section the built-in shape asks for, because a document that
// does not is refused before it reaches the disk. The padding is the problem
// prose, which is where a brief's unheaded text belongs anyway.
fn document_of(bytes: usize) -> String {
    let opening = "# Scopes and sigils\n\n";
    let shape = "\n\n## Outcome\n\n## Success criteria\n\n## Constraints\n\n\
                     ## Out of scope\n\n## Scope\n";
    let prose = bytes - opening.len() - shape.len();
    format!("{opening}{}{shape}", "x".repeat(prose))
}

const WHOLE: &str = "# Freshness\n\nProse.\n\n## Outcome\n\n## Success criteria\n\n\
                         ## Constraints\n\n## Out of scope\n\n## Scope\n";

// For the tests whose subject is not the shape — what the bytes come to,
// where they land, what the lines say — so adding five headings to every
// literal reply below does not bury what each one is about. The check itself
// has its own tests, against the built-in shape.
fn shapeless(root: &Path) {
    let template = manifest_path(root).with_file_name("brief-template.md");
    fs::create_dir_all(template.parent().expect("the template sits in a directory"))
        .expect("makes .warlock");
    fs::write(template, "").expect("writes an empty template");
}

#[test]
fn what_was_written_is_handed_back_and_a_second_write_replaces_it() {
    // The one thing a write tells the session about itself: the path of the
    // file, in the spelling the line on the thread used, so a later `/push`
    // files the document the reader just watched land. A path typed as
    // something else — a leading `./`, the absolute path — comes back
    // manifest-relative, because that is the spelling the ledger stores and
    // the one `from_manifest_path` reads back.
    let repo = a_repo();
    let second = "docs/warlock-brief-14-sigils.md";
    let mut app = app_answering(repo.path(), WHOLE);

    let wrote = super::write_submit(&mut app, repo.path(), &field(&format!("./{BRIEF}")), now());

    assert_eq!(wrote.prompt, ScopePrompt::Closed);
    assert_eq!(wrote.written.as_deref(), Some(BRIEF));
    assert_eq!(
        notes(&app),
        [format!("wrote {BRIEF} — {} bytes", WHOLE.len())],
        "the line names a path the session does not remember",
    );

    // A refusal in between leaves nothing behind for a caller to mistake for
    // a write: the file is already there, so this one writes nothing.
    let refused = super::write_submit(&mut app, repo.path(), &field(BRIEF), now());
    assert!(refused.prompt.is_open());
    assert_eq!(refused.written, None);

    // And the second document is the one a session keeping only the most
    // recent would remember.
    let again = super::write_submit(&mut app, repo.path(), &field(second), now());
    assert_eq!(again.written.as_deref(), Some(second));
}

#[test]
fn enter_writes_the_document_and_says_what_landed() {
    let repo = a_repo();
    // 1832 bytes on the nose, so the line's size is a fact rather than a
    // range: the panel's own spelling, one decimal under ten kilobytes.
    let reply = document_of(1832);
    let mut app = app_answering(repo.path(), &reply);

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(prompt, ScopePrompt::Closed, "the window is answered");
    assert_eq!(
        fs::read_to_string(repo.path().join(BRIEF)).expect("the artifact reads back"),
        reply,
        "the bytes on disk are not the reply on the card"
    );
    assert_eq!(notes(&app), [format!("wrote {BRIEF} — 1.8 KB")]);
    // That one line is the whole of what a write says. The footer is the
    // last keystroke's, and nothing about the run state moved.
    assert_eq!(app.message(), Some(LAST_KEY));
    assert!(!app.is_pacting());
    assert_eq!(app.pact_line(), None);
}

#[test]
fn the_size_is_the_panels_own_spelling_at_every_scale() {
    for (bytes, said) in [
        (934, "934 bytes"),
        (1832, "1.8 KB"),
        (14_540, "14 KB"),
        (412_672, "403 KB"),
    ] {
        let repo = a_repo();
        let mut app = app_answering(repo.path(), &document_of(bytes));

        write_submit(&mut app, repo.path(), &field(BRIEF), now());

        assert_eq!(notes(&app), [format!("wrote {BRIEF} — {said}")]);
    }
}

#[test]
fn the_document_is_written_verbatim_but_for_the_fence_and_the_last_newline() {
    for (reply, written) in [
        // Byte for byte, trailing newline and all.
        ("# Freshness\n\nProse.\n", "# Freshness\n\nProse.\n"),
        // The one newline ensured, and nothing else added.
        ("# Freshness\n\nProse.", "# Freshness\n\nProse.\n"),
        // Already ending in three: not normalised, because trimming would
        // be a third transformation nobody asked for.
        ("# Freshness\n\n\n\n", "# Freshness\n\n\n\n"),
        // A reply that is entirely one fence comes out of it.
        (
            "```markdown\n# Freshness\n\nProse.\n```\n",
            "# Freshness\n\nProse.\n",
        ),
        // A document that merely contains one is left exactly as it is,
        // indentation, blank lines, trailing spaces and all.
        (
            "# Freshness\n\n```rust\nlet x = 1;\n```\n\n  indented   \n",
            "# Freshness\n\n```rust\nlet x = 1;\n```\n\n  indented   \n",
        ),
    ] {
        let repo = a_repo();
        // The subject here is the bytes, so the shape is asked for nothing
        // and each reply below stays the literal it is meant to be.
        shapeless(repo.path());
        let mut app = app_answering(repo.path(), reply);

        write_submit(&mut app, repo.path(), &field(BRIEF), now());

        assert_eq!(
            fs::read_to_string(repo.path().join(BRIEF)).expect("the artifact reads back"),
            written,
            "{reply:?} was interfered with"
        );
    }
}

#[test]
fn nothing_but_the_artifact_is_written() {
    let repo = a_repo();
    let mut app = app_answering(repo.path(), WHOLE);

    write_submit(&mut app, repo.path(), &field(BRIEF), now());

    // The output directory and the file in it, and that is the whole of the
    // repository: no transcript, no draft, and nothing warlock authored
    // under `.warlock/` — the ledger is never touched by a write.
    assert_eq!(everything_under(repo.path()), ["docs", BRIEF]);
}

#[test]
fn the_parent_directory_is_made_when_it_is_not_there() {
    let repo = a_repo();
    let mut app = app_answering(repo.path(), WHOLE);
    let deep = "docs/briefs/2026/warlock-brief-01-freshness.md";

    let prompt = write_submit(&mut app, repo.path(), &field(deep), now());

    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(repo.path().join(deep).is_file(), "{deep} is not a file");
}

#[test]
fn a_path_that_already_exists_writes_nothing_and_reopens_the_field() {
    let repo = a_repo();
    fs::create_dir_all(repo.path().join("docs")).expect("makes the output directory");
    fs::write(repo.path().join(BRIEF), "what was already there\n").expect("writes it first");
    let mut app = app_answering(repo.path(), WHOLE);
    let before = app.clone();
    let typed = field(BRIEF);

    let prompt = write_submit(&mut app, repo.path(), &typed, now());

    // The rule under the field and the typed path still in it, one keystroke
    // from being changed — and the file that was there is byte for byte the
    // file that is there.
    let still_up = prompt.field().expect("the window came down");
    assert_eq!(still_up.text(), BRIEF);
    assert!(
        still_up
            .rule()
            .is_some_and(|rule| rule.contains(BRIEF) && rule.contains("already exists")),
        "the refusal said {:?}",
        still_up.rule()
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(BRIEF)).expect("the file reads back"),
        "what was already there\n",
        "the write went over a file that was already there"
    );
    assert_eq!(app, before, "a refusal moved the view");
    assert!(notes(&app).is_empty(), "a refusal said something happened");
}

#[test]
fn a_directory_in_the_way_is_a_path_that_already_exists() {
    // Not a file, and still nothing warlock may write over. `exists` is
    // asked about the path rather than about its kind.
    let repo = a_repo();
    fs::create_dir_all(repo.path().join(BRIEF)).expect("makes a directory of that name");
    let mut app = app_answering(repo.path(), WHOLE);

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert!(prompt.is_open(), "a directory was written over");
    assert!(notes(&app).is_empty());
}

#[test]
fn an_empty_field_writes_nothing_and_stays_up() {
    let repo = a_repo();
    let mut app = app_answering(repo.path(), WHOLE);

    for typed in ["", "   "] {
        let prompt = write_submit(&mut app, repo.path(), &field(typed), now());

        assert_eq!(
            prompt.field().and_then(ScopeField::rule),
            Some(NO_PATH),
            "{typed:?} was answered as a path"
        );
        assert_eq!(everything_under(repo.path()), Vec::<String>::new());
    }
}

#[test]
fn a_path_that_climbs_out_of_the_repository_is_refused() {
    let repo = a_repo();
    let outside = repo.path().join("outside");
    let root = repo.path().join("repo");
    fs::create_dir_all(&outside).expect("makes a directory beside the repository");
    fs::create_dir_all(&root).expect("makes the repository");
    let mut app = app_answering(&root, WHOLE);

    let prompt = write_submit(&mut app, &root, &field("../outside/brief.md"), now());

    // The artifact belongs to the repository the ledger is about, so the
    // field stays up over the engine's own sentence and nothing lands
    // anywhere.
    assert!(
        prompt
            .field()
            .and_then(ScopeField::rule)
            .is_some_and(|rule| !rule.is_empty()),
        "a path outside the repository was allowed through"
    );
    assert_eq!(everything_under(&outside), Vec::<String>::new());
    assert_eq!(everything_under(&root), Vec::<String>::new());
}

#[test]
fn a_document_missing_a_section_is_refused_and_nothing_reaches_the_disk() {
    let repo = a_repo();
    // Every section but the last: the failure that reads perfectly well and
    // is only noticed days later, by somebody looking for the slices.
    let dropped = "# Freshness\n\nProse.\n\n## Outcome\n\n## Success criteria\n\n\
                       ## Constraints\n\n## Out of scope\n";
    let mut app = app_answering(repo.path(), dropped);

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    // The window comes down rather than staying up over the field: what is
    // wrong is the document, and the field is not where a reader fixes one.
    assert_eq!(prompt, ScopePrompt::Closed, "the window stayed up");
    assert_eq!(
        app.message(),
        Some("the document is missing ## Scope, so nothing was written"),
    );
    // Nothing on the conversation and nothing on disk. A refusal that had
    // written the file and then complained would be the worst of both.
    assert_eq!(notes(&app), Vec::<String>::new());
    assert!(
        !repo.path().join(BRIEF).exists(),
        "the document was written"
    );
    assert_eq!(everything_under(repo.path()), Vec::<String>::new());
}

#[test]
fn several_dropped_sections_are_all_named_in_the_shapes_own_order() {
    let repo = a_repo();
    let mut app = app_answering(
        repo.path(),
        "# Freshness\n\nProse.\n\n## Success criteria\n\n## Out of scope\n",
    );

    write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(
        app.message(),
        Some(
            "the document is missing ## Outcome, ## Constraints and ## Scope, \
                 so nothing was written"
        ),
    );
}

#[test]
fn the_shape_a_document_is_held_to_is_the_repositorys_own() {
    // Read at the write rather than remembered from when brief mode was
    // entered, so a template edited mid-conversation is the one that counts.
    let repo = a_repo();
    let template = manifest_path(repo.path()).with_file_name("brief-template.md");
    fs::create_dir_all(template.parent().expect("a directory")).expect("makes .warlock");
    fs::write(&template, "# A title\n\n## Rollout\n").expect("writes a template");

    // A document with warlock's own five sections and not this repository's.
    let mut app = app_answering(repo.path(), WHOLE);
    write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(
        app.message(),
        Some("the document is missing ## Rollout, so nothing was written"),
    );
    assert!(!repo.path().join(BRIEF).exists());

    // And the same document, once the shape asks for what it has.
    fs::write(&template, "# A title\n\n## Outcome\n").expect("rewrites the template");
    let mut app = app_answering(repo.path(), WHOLE);
    write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(
        notes(&app),
        [format!("wrote {BRIEF} — {} bytes", WHOLE.len())]
    );
}

#[test]
fn a_shape_that_cannot_be_read_refuses_rather_than_letting_the_write_through() {
    // `template.rs` never quietly puts the built-in shape in place of a file
    // somebody wrote, and neither does this: a check that silently stopped
    // checking is worse than one that says it could not.
    let repo = a_repo();
    let template = manifest_path(repo.path()).with_file_name("brief-template.md");
    fs::create_dir_all(&template).expect("makes a directory where the file goes");
    let mut app = app_answering(repo.path(), WHOLE);

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(prompt, ScopePrompt::Closed);
    let said = app.message().expect("a refusal says why");
    assert!(
        said.starts_with("could not read the brief shape, so nothing was written: ")
            && said.contains("brief-template.md"),
        "{said:?} does not name the file or what it cost",
    );
    assert!(
        !repo.path().join(BRIEF).exists(),
        "the document was written"
    );
}

#[test]
fn a_write_that_will_not_happen_puts_its_reason_on_the_message_line() {
    // A file where the output directory has to be: the parent cannot be
    // made, which is the cheapest real version of a disk that will not take
    // the write.
    let repo = a_repo();
    fs::write(repo.path().join("docs"), "not a directory\n")
        .expect("writes a file in the way of the output directory");
    let mut app = app_answering(repo.path(), WHOLE);

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    // A line on the footer and the window down off it — never an error out
    // of the event loop, and never a line on the conversation claiming a
    // file that is not there.
    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(
        app.message()
            .is_some_and(|line| line.contains(BRIEF) && line != LAST_KEY),
        "a write that failed said {:?}",
        app.message()
    );
    assert!(notes(&app).is_empty(), "it said something happened anyway");
    assert_eq!(
        fs::read_to_string(repo.path().join("docs")).expect("the file in the way reads back"),
        "not a directory\n"
    );
}

#[test]
fn a_conversation_with_no_answer_on_it_writes_nothing_and_says_so() {
    // The road the loop cannot take — the prompt opens over an answer that
    // has landed — answered rather than left to be discovered.
    let repo = a_repo();
    let mut app = App::from_tree(&Tree::new(Node::new(
        repo.path(),
        None::<PathBuf>,
        NodeState::Unpacted,
    )));

    let prompt = write_submit(&mut app, repo.path(), &field(BRIEF), now());

    assert_eq!(prompt, ScopePrompt::Closed);
    assert!(app.message().is_some_and(|line| !line.is_empty()));
    assert_eq!(everything_under(repo.path()), Vec::<String>::new());
}

// `scoping.rs`'s counterpart for the other prompt, driven exactly as it
// drives that one: the window is opened the way the loop opens it, every key
// goes through `edit_for` as `press_for` would send it, and what comes back
// goes through `write_edit`, which is the loop's arm.
mod rounds {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use warlock_engine::DEFAULT_BRIEF_DIRECTORY;
    use warlock_tui::{Mode, ScopePrompt, edit_for};

    use super::super::{WRITE_HEADING, write_edit, write_opened};
    use super::{App, Instant, TempDir, a_repo, app_answering, everything_under, fs, notes, now};

    const REPLY: &str = "# Scopes and sigils\n\nA boundary somebody drew.\n\n\
                             ## Outcome\n\n## Success criteria\n\n## Constraints\n\n\
                             ## Out of scope\n\n## Scope\n";

    const DIRECTORY: &str = DEFAULT_BRIEF_DIRECTORY;

    const PROPOSED: &str = "docs/warlock-brief-01-scopes-and-sigils.md";

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn round(
        app: &mut App,
        repo: &TempDir,
        prompt: &ScopePrompt,
        code: KeyCode,
        now: Instant,
    ) -> ScopePrompt {
        let edited = {
            let field = prompt.field().expect("the window is still up");
            edit_for(press(code), field)
        };
        write_edit(app, repo.path(), prompt, edited, now).prompt
    }

    #[test]
    fn the_window_opens_over_the_answer_holding_the_path_it_proposes() {
        // What the drain hands the loop, turned into the window the reader
        // sees: the heading it is drawn under, and the proposal in the
        // field, ready for an Enter that changes nothing about it.
        let repo = a_repo();

        assert_eq!(
            write_opened(repo.path(), DIRECTORY, REPLY),
            ScopePrompt::open(WRITE_HEADING, PROPOSED)
        );
        assert_eq!(
            everything_under(repo.path()),
            Vec::<String>::new(),
            "opening a window wrote something"
        );
    }

    #[test]
    fn the_whole_path_from_the_answer_to_the_file_is_one_key_at_a_time() {
        // The loop's arms in the order a reader presses them: the window
        // opens on the proposal, three Backspaces and four characters make
        // it something else, and Enter writes the document at the path on
        // screen rather than at the one warlock guessed.
        let repo = a_repo();
        let mut app = app_answering(repo.path(), REPLY);
        let mut prompt = write_opened(repo.path(), DIRECTORY, REPLY);

        for code in [
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Char('.'),
            KeyCode::Char('t'),
            KeyCode::Char('x'),
            KeyCode::Char('t'),
            KeyCode::Enter,
        ] {
            prompt = round(&mut app, &repo, &prompt, code, now());
        }

        let written = "docs/warlock-brief-01-scopes-and-sigils.txt";
        assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
        assert_eq!(
            fs::read_to_string(repo.path().join(written)).expect("the artifact reads back"),
            REPLY,
            "the bytes on disk are not the reply on the card"
        );
        assert_eq!(
            notes(&app),
            vec![format!("wrote {written} — {} bytes", REPLY.len())]
        );
    }

    #[test]
    fn esc_writes_nothing_and_leaves_the_reply_and_the_register_alone() {
        // The abandonment, and the whole of what it is allowed to touch:
        // the window. The reply is still on the card, the mode is still
        // brief, the footer still says what the last key said, and the
        // repository is as empty as it was.
        let repo = a_repo();
        let mut app = app_answering(repo.path(), REPLY);
        app.panel_mut().set_mode(Mode::Brief);
        let before = app.clone();
        let prompt = write_opened(repo.path(), DIRECTORY, REPLY);

        let prompt = round(&mut app, &repo, &prompt, KeyCode::Esc, now());

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(app, before, "Esc moved something on the app");
        assert_eq!(app.panel().mode(), Mode::Brief);
        assert_eq!(everything_under(repo.path()), Vec::<String>::new());
    }

    #[test]
    fn a_refused_path_reopens_the_field_and_the_next_enter_writes() {
        // A path that is taken, typed out of the way and written: the
        // refusal keeps the window up over the very text that earned it, so
        // the fix is a keystroke rather than a second `/write`.
        //
        // The file in the way appears after the window opened, which is the
        // situation the check is for: the proposal is a guess made once, at
        // the answer, and what is on disk when Enter is pressed is a
        // different question with a different answer.
        let repo = a_repo();
        let mut app = app_answering(repo.path(), REPLY);
        let mut prompt = write_opened(repo.path(), DIRECTORY, REPLY);
        fs::create_dir_all(repo.path().join("docs")).expect("makes the output directory");
        fs::write(repo.path().join(PROPOSED), "somebody else's brief\n")
            .expect("writes the file in the way");

        prompt = round(&mut app, &repo, &prompt, KeyCode::Enter, now());

        let field = prompt
            .field()
            .expect("a refusal left the window up")
            .clone();
        assert_eq!(field.text(), PROPOSED, "the typed path was taken away");
        assert!(field.rule().is_some(), "the window says nothing about why");
        assert_eq!(
            fs::read_to_string(repo.path().join(PROPOSED)).expect("the file is still there"),
            "somebody else's brief\n",
            "the refused Enter wrote over the file in the way"
        );

        for code in [
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Char('-'),
            KeyCode::Char('2'),
            KeyCode::Char('.'),
            KeyCode::Char('m'),
            KeyCode::Char('d'),
            KeyCode::Enter,
        ] {
            prompt = round(&mut app, &repo, &prompt, code, now());
        }

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(
            fs::read_to_string(
                repo.path()
                    .join("docs/warlock-brief-01-scopes-and-sigils-2.md")
            )
            .expect("the second path reads back"),
            REPLY
        );
    }

    #[test]
    fn a_submit_from_a_window_that_is_not_up_writes_nothing() {
        // The road `press_for` cannot take — it consults `edit_for` only
        // while a window is up — answered rather than left to be
        // discovered.
        let repo = a_repo();
        let mut app = app_answering(repo.path(), REPLY);

        let prompt = write_edit(
            &mut app,
            repo.path(),
            &ScopePrompt::Closed,
            warlock_tui::Edited::Submit,
            now(),
        )
        .prompt;

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(everything_under(repo.path()), Vec::<String>::new());
    }
}

// Every other test in this file is about one joint. This one is about the
// joints being joined: `apply_compose` starts the turn, the turn is really
// run on a worker thread through the very agent the loop holds, the loop's
// own drain hands the answer back, the prompt opens through `write_opened`,
// and Enter goes through `edit_for` into `write_edit`. Nothing in the middle
// is stood in for.
//
// What *is* stood in for is the model, and it is `/bin/sh` printing one
// result line — `chatting.rs`'s arrangement, which is why this module is
// Unix-only.
mod whole {

    use std::thread;
    use std::time::Duration;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use warlock_tui::{Composed, Composer, Line, Mode, ScopePrompt, edit_for};

    use super::super::WRITE_HEADING;
    use super::{
        App, Instant, Node, NodeState, PathBuf, Tree, a_repo, everything_under, fs, notes, now,
    };
    use warlock_tui::Converses;

    use crate::chatting::Chat;
    use crate::stubs::Saying;

    const AT_MOST: Duration = Duration::from_secs(5);

    const DOCUMENT: &str = "# Scopes and sigils\n\nA boundary somebody drew, and the reason \
                                it is there.\n\n## Outcome\n\n## Success criteria\n\n\
                                ## Constraints\n\n## Out of scope\n\n## Scope\n";

    const PROPOSED: &str = "docs/warlock-brief-01-scopes-and-sigils.md";

    // `Chat::keep_up` and nothing else, which is what the event loop's own
    // `keep_up` calls: nothing here waits on the worker, joins a thread or
    // receives from a channel.
    //
    // Nothing is opened here. This used to build the window itself, calling
    // `write_opened` over whatever the drain handed up — a second spelling
    // of a line the event loop also had, living in a test. The conversation
    // owns the window now, so what opened is read off it afterwards.
    fn rounds_until_answered<C: Converses>(chat: &mut Chat<C>, app: &mut App, now: Instant) {
        let waited = Instant::now();
        while chat.answering() && waited.elapsed() < AT_MOST {
            chat.keep_up(app, now);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!chat.answering(), "the turn never ended");
    }

    #[test]
    fn a_write_typed_at_the_composer_ends_as_a_document_on_disk() {
        let repo = a_repo();
        let base = now();
        let mut app = App::from_tree(&Tree::new(Node::new(
            repo.path(),
            None::<PathBuf>,
            NodeState::Unpacted,
        )));
        // The register the command is only allowed in. Nothing else is
        // arranged: the write path is handed the app, the root and the
        // field, and the ledger is no part of it.
        app.panel_mut().set_mode(Mode::Brief);
        // The conversation, rooted in that repository: where a brief goes
        // is its own now, settled at `/brief` and read at `/write` without
        // being looked at again.
        let mut chat = Chat::with_agent(repo.path(), Saying::answering(DOCUMENT));

        // The reader typing the word and pressing Enter at the foot of the
        // panel, through the very method the loop's composer arm calls.
        chat.compose(&mut app, Composed::Typing(Composer::new("/write")), base);
        chat.compose(&mut app, Composed::Submit, base);

        assert!(chat.answering(), "the command started no turn");
        assert_eq!(
            chat.composer().draft(),
            "",
            "the field kept the submitted word"
        );

        // Then the rounds, until the answer lands and the window opens over
        // it — pre-filled, headed, and complaining about nothing.
        rounds_until_answered(&mut chat, &mut app, base);

        let field = chat
            .write_prompt()
            .field()
            .expect("the path prompt opened over the answer")
            .clone();
        assert_eq!(field.text(), PROPOSED);
        assert_eq!(field.directory(), WRITE_HEADING);
        assert_eq!(field.rule(), None);
        assert_eq!(
            everything_under(repo.path()),
            Vec::<String>::new(),
            "a proposal wrote something"
        );
        // The card shows the word that was typed and the document that came
        // back — never the paragraph warlock sent.
        let rows = app
            .panel()
            .thread()
            .expect("the conversation is there")
            .lines(base);
        assert_eq!(
            rows.first(),
            Some(&Line::Said {
                text: "/write".to_owned()
            })
        );
        assert!(
            rows.iter().any(|line| *line
                == Line::Text {
                    text: "# Scopes and sigils".to_owned()
                }),
            "the answer is not on the card: {rows:?}"
        );

        // And Enter in that window, through `edit_for` as `press_for` sends
        // it and `write_edit` as the loop's arm applies it.
        let edited = edit_for(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &field);
        chat.write(&mut app, edited, base);

        assert_eq!(
            *chat.write_prompt(),
            ScopePrompt::Closed,
            "Enter left the window up"
        );
        let written =
            fs::read_to_string(repo.path().join(PROPOSED)).expect("the artifact reads back");
        assert_eq!(written, DOCUMENT, "the bytes are not the document answered");
        assert_eq!(
            notes(&app),
            [format!("wrote {PROPOSED} — {} bytes", written.len())]
        );
        // The whole repository, after the whole path: the output directory
        // and the one file in it. No transcript, no draft of the brief, and
        // nothing warlock authored under `.warlock/` — the turn wrote
        // nothing at all, and the write wrote the artifact.
        assert_eq!(everything_under(repo.path()), ["docs", PROPOSED]);
    }
}
