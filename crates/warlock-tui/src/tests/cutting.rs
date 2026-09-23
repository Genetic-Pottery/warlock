//! Every test here drives [`Cutter`] over a temporary repository and a temporary
//! home, through the seam the event loop uses: a stand-in Linear handed in as
//! the [`Opens`] the value was built with. Nothing below reads the sigils, the
//! binding or the key store of the machine the suite runs on, and the one key
//! any of it stores is not one.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use warlock_engine::{
    CutRecord, Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, save_key, save_key_binding,
    save_sigils,
};
use warlock_tui::{App, Converses, Line, Opens};

use super::{ALREADY_CUTTING, Cutter};
use crate::error::{Error, one_line};
use crate::stubs::{Answering, Boarding, Gate, Scripted};

// Not a key, and named so that nothing reading this file mistakes it for one:
// it is stored only so that a bound name resolves and the worker has something
// to build a client from, which is the one line that sees it.
const NOT_A_KEY: &str = "not-a-real-key-value";

// A name no real key store would be holding, so a cut that reached the
// machine's own home could not pass for one that reached this test's.
const KEY_NAME: &str = "this-tests-own-name";

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

// The manifest's own spelling of the brief, which is what a filed record is
// keyed by and so what a `/draft` carries.
const BRIEF: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/draft-a-brief-1a2b3c";

const NAME: &str = "Cut a planned project into tickets";

// The board's own spelling, which is not `Planned`: the gate folds case, and
// the line reports what somebody sent to look would find written there.
const STATUS: &str = "planned";

// Three slices in document order, each waiting on the one before it: the
// description a project carries once somebody has edited the scope in Linear.
const SLICED: &str = "Nothing cuts a planned project into tickets.\n\n## Scope\n\n\
                      ### 1. Read the project back\n\ndepends_on: []\n\n\
                      What it resolves.\n\n\
                      ### 2. Parse the scope block\n\ndepends_on: [1]\n\n\
                      What it parses.\n\n\
                      ### 3. File the drafts\n\ndepends_on: [2]\n\n\
                      What it writes.\n";

const FIRST: &str = "Read the project back";

const SECOND: &str = "Parse the scope block";

const THIRD: &str = "File the drafts";

// Long enough that a worker which never reports fails the test rather than
// hanging the suite, and short enough that it is a failure rather than a wait.
const AT_MOST: Duration = Duration::from_secs(10);

fn now() -> Instant {
    Instant::now()
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn a_manifest() -> Manifest {
    Manifest::with_entries([PactEntry::new(".", "docs", "docs/WARLOCK.md")
        .expect("a relative module path is inside the root")
        .with_scope(SCOPE)])
    .with_scopes([ScopeRecord::new(SCOPE, TEAM, "In Review", LABEL)])
}

// A repository with the manifest saved and the brief on disk. The document's
// own text is beside the point here — what a cut reads is the project, and the
// file is what the record is keyed by.
fn a_repository() -> TempDir {
    let repo = a_dir();
    a_manifest()
        .save(repo.path())
        .expect("a manifest that saves");
    let path = repo.path().join(BRIEF);
    fs::create_dir_all(path.parent().expect("a `docs` directory")).expect("a `docs` directory");
    fs::write(&path, "# A brief\n").expect("a brief file");
    repo
}

// A home of this test's own: the sigils that pick the board, the binding and
// the key store all sit under it, and the value under test is only ever handed
// the one built here.
fn a_home(root: &Path) -> TempDir {
    let home = a_dir();
    save_sigils(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
    save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

// The record a `/push` of this brief left behind, with whichever slices have
// already been cut written under it.
fn filed(root: &Path, cuts: &[&str]) {
    let mut record = FiledRecord::new(
        root,
        root.join(BRIEF),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-20T07:32:00Z",
    )
    .expect("a path inside the repository");
    for (place, title) in cuts.iter().enumerate() {
        record.push_cut(CutRecord::new(
            *title,
            [format!("WAR-{}", place + 1)],
            "2026-09-21T09:00:00Z",
        ));
    }
    Filed::with_records([record])
        .save(root)
        .expect("a record file that saves");
}

fn notes(app: &App) -> Vec<String> {
    app.panel()
        .thread()
        .map(|thread| thread.lines(now()))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| match line {
            Line::Note { text } => Some(text),
            _ => None,
        })
        .collect()
}

// A `/draft` pressed at the value the loop holds. The home is the value's own,
// handed to it when it was built, which is what keeps every test here off the
// machine's.
fn press<O: Opens, A: Converses>(app: &mut App, cutter: &mut Cutter<O, A>, repo: &Path) {
    cutter.press(app, &a_manifest(), repo, BRIEF, now());
}

// Rounds until the fetch has reported, drained and never blocked on: the loop
// draws and then drains, so a test that waited on the channel would be a test
// of something the panel does not do.
fn landing<O: Opens, A: Converses>(app: &mut App, cutter: &mut Cutter<O, A>) -> usize {
    let waited = Instant::now();
    let mut rounds = 0;
    while cutter.fetching() && waited.elapsed() < AT_MOST {
        cutter.keep_up(app, now());
        rounds += 1;
    }
    assert!(!cutter.fetching(), "the draft never reported");
    rounds
}

// Rounds until the run is over, drained and never blocked on — the loop draws
// and then drains, so a test that waited on a channel would be a test of
// something the panel does not do — with the window each slice's drafts stop
// behind answered the way somebody at the panel would answer it: Skip, which
// sends nothing at all, and a Yes to the carry-on question behind it.
//
// Skip and not Create, because what a run drafts and what becomes of the drafts
// are two questions: every test that drives this one is about the first, and the
// second is `reviewing`'s own.
fn through<O: Opens, A: Converses>(app: &mut App, cutter: &mut Cutter<O, A>) {
    let waited = Instant::now();
    while cutter.drafting() && waited.elapsed() < AT_MOST {
        drop(cutter.keep_up(app, now()));
        if cutter.reviewing().is_some() {
            cutter.skip(app, now());
        }
        if cutter.carrying().is_some() {
            cutter.carry_on(app, now());
        }
    }
    assert!(!cutter.drafting(), "the run never finished");
}

// The engine's own sentence for a refusal, flattened as the thread takes it:
// these tests assert the line *is* that sentence rather than restating it.
fn refusal(error: &Error) -> String {
    one_line(&error.to_string())
}

// The workspace every test here reads back: one project, planned, with three
// slices under its scope block.
fn a_project() -> Boarding {
    Boarding::holding(NAME, Some(STATUS), SLICED)
}

// A model no slice may reach. Every test above the run is decided before a
// session is opened, so a turn being asked for at all is the failure — which is
// what an empty script panics over.
fn unasked() -> Scripted {
    Scripted::saying([])
}

// A fetch carried all the way to the question, which is the only way the dialog
// is ever up and so the only way a run ever starts. The two directories come
// back with it because they are the repository and the home the value is
// reading: dropped here, every path under test would be gone before the answer.
//
// Generic over the workspace for the one difference a run that files has: a
// create sends over the same seam the fetch read over, so the tests below that
// answer a window hand in a board that can answer both.
fn asked_over<O: Opens>(
    linear: O,
    agent: Scripted,
) -> (App, Cutter<O, Scripted>, TempDir, TempDir) {
    asked_proposing(linear, agent, unasked())
}

// The same, with the second conversation scripted as well: the one warlock's
// attempt at a question is asked in. Separate from the slice's own script
// because they are separate sessions, so a test that scripts one and leaves the
// other empty is a test that says which of the two was asked.
fn asked_proposing<O: Opens>(
    linear: O,
    agent: Scripted,
    proposer: Scripted,
) -> (App, Cutter<O, Scripted>, TempDir, TempDir) {
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let mut cutter = Cutter::with_client(linear, Some(home.path().to_path_buf()), agent, proposer);
    let mut app = App::default();
    press(&mut app, &mut cutter, repo.path());
    landing(&mut app, &mut cutter);
    (app, cutter, repo, home)
}

#[test]
fn a_cut_reports_the_project_its_status_and_how_many_slices_are_left() {
    // The whole sequence with nothing in its way: the board resolved, the
    // record read, the project fetched, the gate passed, the scope block
    // parsed. One request, and the line names what the reader is about to be
    // asked about.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let linear = Boarding::holding(NAME, Some(STATUS), SLICED);
    let mut cutter = Cutter::with_client(
        linear.clone(),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());
    landing(&mut app, &mut cutter);

    let notes = notes(&app);
    let said = notes.last().expect("the draft said nothing");
    assert!(said.contains(NAME), "{said:?} does not name the project");
    assert!(
        said.contains(STATUS),
        "{said:?} does not say the board's own spelling of the status"
    );
    assert!(said.contains("3 slices"), "{said:?} does not count slices");
    assert_eq!(linear.requests(), 1, "one draft is one request");
    assert!(
        !format!("{cutter:?}").contains(NOT_A_KEY),
        "the key value is in the value the session holds"
    );
    assert!(
        notes.iter().all(|line| !line.contains(NOT_A_KEY)),
        "the key value is on the thread"
    );
}

#[test]
fn slices_already_cut_are_left_out_of_what_is_still_to_cut() {
    // The count of what is left comes off `.warlock/filed.toml` rather than off
    // the block, so a brief two slices into being cut says so.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[FIRST, SECOND]);
    let mut cutter = Cutter::with_client(
        Boarding::holding(NAME, Some(STATUS), SLICED),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());
    landing(&mut app, &mut cutter);

    let said = notes(&app).last().cloned().expect("the draft said nothing");
    assert!(said.contains("3 slices"), "{said:?} miscounts the block");
    assert!(
        said.contains('1'),
        "{said:?} does not say that one slice is left"
    );
    assert!(!said.contains(THIRD), "{said:?} names a slice");
}

#[test]
fn a_project_that_is_not_planned_is_one_line_with_nothing_torn_down() {
    // The gate, asked of what the board answered with. The request was made —
    // there is no other way to know the status — and nothing else happened.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let linear = Boarding::holding(NAME, Some("Backlog"), SLICED);
    let mut cutter = Cutter::with_client(
        linear.clone(),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());
    landing(&mut app, &mut cutter);

    assert_eq!(
        notes(&app).last(),
        Some(&refusal(&Error::NotPlanned {
            path: BRIEF.to_owned(),
            status: Some("Backlog".to_owned()),
        })),
        "the line is not the engine's own sentence",
    );
    assert_eq!(linear.requests(), 1, "the gate sent a second request");
    assert!(!cutter.fetching(), "a refusal left a draft running");
}

#[test]
fn a_machine_with_no_home_is_refused_before_anything_is_read() {
    // The one refusal this value answers itself, in `Standing::home`'s own
    // words: with no home there are no sigils, no binding and no key store, so
    // there is no board to resolve and nothing to read the record for.
    let repo = a_repository();
    filed(repo.path(), &[]);
    let mut cutter = Cutter::with_client(Boarding::unopened(), None, unasked(), unasked());
    let mut app = App::default();

    cutter.press(&mut app, &a_manifest(), repo.path(), BRIEF, now());

    assert_eq!(notes(&app), vec![refusal(&Error::NoHome)]);
    assert!(!cutter.fetching(), "a refusal started a draft");
}

#[test]
fn a_second_cut_with_one_in_flight_is_one_line_and_reads_nothing() {
    // One cut at a time, and the say-no is the run itself. The first is held
    // open at the gate, so the second arrives while it is genuinely in flight;
    // what it costs is a line.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let linear = Boarding::holding(NAME, Some(STATUS), SLICED).held_at(&gate);
    let mut cutter = Cutter::with_client(
        linear.clone(),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());
    press(&mut app, &mut cutter, repo.path());

    assert_eq!(
        notes(&app).last(),
        Some(&ALREADY_CUTTING.to_owned()),
        "the second draft did not say no",
    );
    gate.open();
    landing(&mut app, &mut cutter);
    assert_eq!(linear.requests(), 1, "the second draft sent something");
}

#[test]
fn the_rounds_go_on_while_the_request_is_in_flight() {
    // The whole reason the fetch is on a worker: a drain that blocked would
    // freeze the panel for as long as Linear took to answer. The gate holds the
    // request open, the rounds are counted, and the cut reports afterwards.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut cutter = Cutter::with_client(
        Boarding::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());
    let held = notes(&app).len();
    for _ in 0..3 {
        cutter.keep_up(&mut app, now());
        assert!(cutter.fetching(), "the drain answered a held request");
    }

    assert_eq!(
        notes(&app).len(),
        held,
        "a round with nothing to report said something",
    );
    gate.open();
    landing(&mut app, &mut cutter);
    let said = notes(&app).last().cloned().expect("the draft said nothing");
    assert!(said.contains(NAME), "{said:?} does not name the project");
}

#[test]
fn a_cut_says_which_document_it_is_reading_before_the_board_answers() {
    // The answer is a request away and the reader has just typed the command,
    // so the thread names the document at once — by the document, because the
    // project has no name on this side of the request.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut cutter = Cutter::with_client(
        Boarding::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut cutter, repo.path());

    let said = notes(&app).last().cloned().expect("the draft said nothing");
    assert!(said.contains(BRIEF), "{said:?} does not name the document");
    assert!(cutter.fetching(), "the draft is not running");
    gate.open();
    landing(&mut app, &mut cutter);
}

#[test]
fn dropping_the_session_ends_the_cut_without_waiting_for_it() {
    // What quitting does: the guard goes with the value, the worker is
    // cancelled, and nothing on this side waits for the request it is sitting
    // in. A cut that had to be joined would hold the terminal until Linear
    // answered.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut cutter = Cutter::with_client(
        Boarding::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
        unasked(),
    );
    let mut app = App::default();
    press(&mut app, &mut cutter, repo.path());

    let dropped = Instant::now();
    drop(cutter);

    assert!(
        dropped.elapsed() < AT_MOST,
        "dropping the value waited for the request"
    );
    // Opened afterwards so the worker finishes and the suite leaves no thread
    // parked: a gate nobody opens is a thread held until the binary exits.
    gate.open();
}

// The question between the fetch and the run, asked of the value the loop
// holds: the session answers it through `Cutter` and not through the dialog, so
// these drive the same three calls `Session::cut_answered` makes.
mod asking {
    use tempfile::TempDir;
    use warlock_tui::Answer;

    use super::{
        Answering, App, BRIEF, Boarding, Cutter, FIRST, Instant, KEY_NAME, NAME, NOT_A_KEY, SLICED,
        STATUS, Scripted, TEAM, a_home, a_project, a_repository, asked_over, filed, landing, notes,
        now, press, refusal, unasked,
    };
    use crate::error::Error;

    fn asked() -> (App, Cutter<Boarding, Scripted>, TempDir, TempDir) {
        asked_over(a_project(), unasked())
    }

    #[test]
    fn a_landed_fetch_puts_the_question_up_carrying_what_it_read() {
        // The five facts come off the one request rather than out of a second
        // one, which is why they are parked on the question at all.
        let (_app, cutter, _repo, _home) = asked();

        let cutting = cutter.confirm().cutting().expect("the question is not up");
        assert_eq!(cutting.project(), NAME);
        assert_eq!(cutting.status(), STATUS);
        assert_eq!(cutting.slices(), 3);
        assert_eq!(cutting.team(), TEAM);
        // The key by name, and nowhere in the question for its bytes.
        assert_eq!(cutting.key(), KEY_NAME);
        assert_eq!(cutting.answer(), Answer::No);
        assert!(
            !format!("{cutting:?}").contains(NOT_A_KEY),
            "the key value is on the question"
        );
    }

    #[test]
    fn a_refused_cut_asks_nothing() {
        // There is nothing to confirm about a project the gate turned away, so
        // the refusal is the whole of what the round does.
        let repo = a_repository();
        let home = a_home(repo.path());
        filed(repo.path(), &[]);
        let mut cutter = Cutter::with_client(
            Boarding::holding(NAME, Some("Backlog"), SLICED),
            Some(home.path().to_path_buf()),
            unasked(),
            unasked(),
        );
        let mut app = App::default();

        press(&mut app, &mut cutter, repo.path());
        landing(&mut app, &mut cutter);

        assert!(
            !cutter.confirm().is_open(),
            "a refused draft asked to be confirmed"
        );
        assert_eq!(
            notes(&app).last(),
            Some(&refusal(&Error::NotPlanned {
                path: BRIEF.to_owned(),
                status: Some("Backlog".to_owned()),
            })),
        );
    }

    #[test]
    fn an_arrow_lights_the_other_answer_and_leaves_the_facts_where_they_were() {
        let (_app, mut cutter, _repo, _home) = asked();

        cutter.lit(Answer::Yes);

        let cutting = cutter.confirm().cutting().expect("the question came down");
        assert_eq!(cutting.answer(), Answer::Yes);
        assert_eq!(cutting.project(), NAME);
        assert_eq!(cutting.slices(), 3);
    }

    #[test]
    fn a_no_takes_the_question_down_and_leaves_the_session_where_it_was() {
        // What a No costs: the window, and nothing else. No second request, no
        // line on the thread and nothing written.
        let (mut app, mut cutter, repo, _home) = asked();
        let said = notes(&app);

        cutter.cancelled();

        assert!(!cutter.confirm().is_open(), "the question is still up");
        assert_eq!(notes(&app), said, "a No said something");
        assert!(!cutter.fetching(), "a No started a draft");
        // And the value is back where a session with no cut in it sits, so the
        // next `/draft` is allowed.
        press(&mut app, &mut cutter, repo.path());
        landing(&mut app, &mut cutter);
        assert!(cutter.confirm().is_open(), "the next draft could not ask");
    }

    #[test]
    fn a_yes_takes_the_question_down_and_starts_the_run() {
        // The window down, the project named and the first slice already being
        // drafted, all on the round the question was answered: a Yes that only
        // said something would leave the run to start on whatever round the
        // loop next came by.
        let (mut app, mut cutter, _repo, _home) =
            asked_over(a_project(), Scripted::saying([Answering::drafts(FIRST)]));
        let said = notes(&app).len();

        cutter.cut(&mut app, Instant::now());

        assert!(
            !cutter.confirm().is_open(),
            "the question is up over its own run"
        );
        assert!(cutter.drafting(), "a Yes started no run");
        let notes = notes(&app);
        assert_eq!(notes.len(), said + 2, "a Yes said nothing");
        assert!(
            notes[said].contains(NAME),
            "{:?} does not name the project",
            notes[said]
        );
        assert!(
            notes[said + 1].starts_with("[1/3] slice 1 "),
            "{:?} does not say which slice is running",
            notes[said + 1]
        );
    }

    #[test]
    fn an_answer_to_a_question_that_is_not_up_does_nothing_at_all() {
        // The three calls again with the dialog closed, which is where a key
        // that reached the wrong window would land.
        let repo = a_repository();
        let home = a_home(repo.path());
        let mut cutter = Cutter::with_client(
            Boarding::holding(NAME, Some(STATUS), SLICED),
            Some(home.path().to_path_buf()),
            unasked(),
            unasked(),
        );
        let mut app = App::default();

        cutter.lit(Answer::Yes);
        cutter.cancelled();
        cutter.cut(&mut app, now());

        assert!(!cutter.confirm().is_open());
        assert!(notes(&app).is_empty(), "a closed question said something");
    }
}

// The run a Yes starts, driven the way the loop drives it: rounds at the value
// the session holds, over a model that answers out of memory and a board that
// is only ever read. Nothing below files anything — that is the next slice of
// this work — so what is asserted is what lands on the thread and what the
// board was asked for.
mod cutting {
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use warlock_engine::drafting::stub_answer;

    use super::{
        AT_MOST, Answering, App, Boarding, Cutter, FIRST, Gate, Instant, NAME, SECOND, Scripted,
        THIRD, a_home, a_project, a_repository, asked_over, filed, landing, notes, now, press,
        through, unasked,
    };

    // The three `[n/total]` prefixes a run over this project says, in the order
    // it says them: the fraction is the place in the cut order and the position
    // is where the slice sits in the document, which is `planned.rs`'s register and
    // not a second one.
    const RUNNING: [&str; 3] = [
        "[1/3] slice 1 `Read the project back` — drafting",
        "[2/3] slice 2 `Parse the scope block` — drafting",
        "[3/3] slice 3 `File the drafts` — drafting",
    ];

    // A confirmed question and the run it starts, over a scripted model.
    fn cut(
        linear: Boarding,
        agent: Scripted,
    ) -> (App, Cutter<Boarding, Scripted>, TempDir, TempDir) {
        let (mut app, mut cutter, repo, home) = asked_over(linear, agent);
        cutter.cut(&mut app, now());
        (app, cutter, repo, home)
    }

    // The stub object with a reference pointing at a draft this slice does not
    // have, which is the repair `prune` makes to every fill and the one thing
    // `check` never reports: an answer the model could give, mended rather than
    // refused.
    fn a_stray_reference(slice: &str) -> String {
        let mut fill: Value =
            serde_json::from_str(&stub_answer(slice)).expect("the stub answer is JSON");
        fill["drafts"][0]["blocked_by"] = json!([7]);
        fill.to_string()
    }

    // Which of the run's lines is about a slice, by the prefix every line about
    // one carries.
    fn about(said: &[String], slice: &str) -> Vec<String> {
        said.iter()
            .filter(|line| line.contains(&format!("`{slice}`")))
            .cloned()
            .collect()
    }

    #[test]
    fn every_uncut_slice_is_drafted_in_cut_order_and_the_board_is_only_read() {
        // One session per slice, one at a time, in the order the slices are to
        // be cut — and the project is left exactly as it was found, because
        // nothing on this path sends a mutation at all.
        let linear = a_project();
        let agent = Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), agent.clone());

        through(&mut app, &mut cutter);

        let said = notes(&app);
        let running: Vec<&String> = said
            .iter()
            .filter(|line| line.ends_with("— drafting"))
            .collect();
        assert_eq!(running, RUNNING.iter().collect::<Vec<_>>());
        assert_eq!(agent.turns(), 3, "a slice was drafted twice or not at all");
        assert_eq!(linear.requests(), 1, "the run sent something to the board");
    }

    #[test]
    fn a_slice_a_record_already_claims_is_not_drafted_again() {
        // The run is over what is left to cut, so the fraction counts those and
        // the position still finds the slice in the document — the two differ
        // exactly here, which is the whole reason both are on the line.
        let repo = a_repository();
        let home = a_home(repo.path());
        filed(repo.path(), &[FIRST]);
        let agent = Scripted::saying([Answering::drafts(SECOND), Answering::drafts(THIRD)]);
        let mut cutter = Cutter::with_client(
            a_project(),
            Some(home.path().to_path_buf()),
            agent.clone(),
            unasked(),
        );
        let mut app = App::default();
        press(&mut app, &mut cutter, repo.path());
        landing(&mut app, &mut cutter);

        cutter.cut(&mut app, now());
        through(&mut app, &mut cutter);

        let said = notes(&app);
        let running: Vec<&String> = said
            .iter()
            .filter(|line| line.ends_with("— drafting"))
            .collect();
        assert_eq!(
            running,
            vec![
                &"[1/2] slice 2 `Parse the scope block` — drafting".to_owned(),
                &"[2/2] slice 3 `File the drafts` — drafting".to_owned(),
            ]
        );
        assert_eq!(agent.turns(), 2, "a slice already cut was drafted again");
    }

    #[test]
    fn the_titles_a_slice_was_drafted_into_land_on_the_thread() {
        // What the reader is about to be offered, said as it arrives: a run
        // that only reported progress would leave them watching a bar.
        let (mut app, mut cutter, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::drafts(FIRST),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut cutter);

        let said = notes(&app);
        let drafted = about(&said, FIRST);
        let line = drafted
            .iter()
            .find(|line| line.contains("drafted `"))
            .unwrap_or_else(|| panic!("no titles were said for the first slice: {drafted:?}"));
        // The stub's own two titles, which name the slice they stand in for.
        assert!(
            line.contains(&format!("`Stand in for {FIRST}`")),
            "{line:?} is missing a draft's title"
        );
        assert!(
            line.contains(&format!("`Follow on from {FIRST}`")),
            "{line:?} is missing a draft's title"
        );
    }

    #[test]
    fn every_repair_the_drafting_path_reported_lands_on_the_thread() {
        // A repaired draft is a ticket that was drafted, not one that was
        // missed, and a conversation read back tomorrow has to be able to tell
        // the two apart.
        let (mut app, mut cutter, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::says(a_stray_reference(FIRST)),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut cutter);

        let said = notes(&app);
        let mended: Vec<String> = about(&said, FIRST)
            .into_iter()
            .filter(|line| line.contains("blocked_by"))
            .collect();
        assert_eq!(mended.len(), 1, "the repair is not on the thread: {said:?}");
        assert!(
            mended[0].contains("outside this slice's drafts"),
            "{:?} is not the engine's own wording of the repair",
            mended[0]
        );
        // And the drafts themselves still arrived: a mend is not a refusal.
        assert_eq!(
            about(&said, FIRST)
                .iter()
                .filter(|line| line.contains("drafted `"))
                .count(),
            1
        );
    }

    #[test]
    fn a_failed_attempt_is_one_line_and_the_run_carries_on() {
        // A missing `claude` is not better the second time, so the slice is left
        // and the slices after it are other work: a run that stopped would leave
        // the reader typing the command again to reach them.
        let (mut app, mut cutter, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::missing(),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut cutter);

        let said = notes(&app);
        let failed: Vec<String> = about(&said, FIRST)
            .into_iter()
            .filter(|line| line.contains("was not drafted"))
            .collect();
        assert_eq!(failed.len(), 1, "a failure cost more than a line: {said:?}");
        assert!(
            about(&said, THIRD)
                .iter()
                .any(|line| line.contains("drafted `")),
            "the run did not reach the last slice: {said:?}"
        );
    }

    #[test]
    fn the_rounds_go_on_while_a_slice_is_drafting() {
        // The whole reason a turn is on a worker: a drain that blocked would
        // freeze the panel for as long as the model took to think, which for a
        // slice is minutes.
        let gate = Gate::shut();
        let agent = Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
        .held_at(&gate);
        let (mut app, mut cutter, _repo, _home) = cut(a_project(), agent);
        let held = notes(&app).len();

        for _ in 0..3 {
            cutter.keep_up(&mut app, now());
            assert!(cutter.drafting(), "the drain answered a held turn");
        }

        assert_eq!(
            notes(&app).len(),
            held,
            "a round with nothing to report said something"
        );
        gate.open();
        through(&mut app, &mut cutter);
    }

    #[test]
    fn a_second_cut_while_a_slice_is_drafting_reads_nothing() {
        // One cut at a time, and the run is its own say-no: two sets of
        // sessions would be two runs cutting one project.
        let gate = Gate::shut();
        let linear = a_project();
        let (mut app, mut cutter, repo, _home) = cut(
            linear.clone(),
            Scripted::saying([Answering::drafts(FIRST)]).held_at(&gate),
        );

        press(&mut app, &mut cutter, repo.path());

        assert_eq!(
            notes(&app).last(),
            Some(&super::ALREADY_CUTTING.to_owned()),
            "the second draft did not say no"
        );
        assert_eq!(linear.requests(), 1, "the second draft read the board");
        gate.open();
    }

    #[test]
    fn dropping_the_session_cancels_the_slice_in_flight_without_waiting() {
        // What quitting does: the guard goes with the value and the handle it
        // latches is the session's own, so the `claude` the turn is waiting on
        // is told to stop — and nothing on this side waits to find out that it
        // did.
        let gate = Gate::shut();
        let agent = Scripted::saying([Answering::drafts(FIRST)]).held_at(&gate);
        let (_app, cutter, _repo, _home) = cut(a_project(), agent.clone());
        assert!(cutter.drafting(), "there is no run to cancel");

        let dropped = Instant::now();
        drop(cutter);

        assert!(
            dropped.elapsed() < AT_MOST,
            "dropping the value waited for the turn"
        );
        assert!(
            agent.cancelled(),
            "the session in flight was not told to stop"
        );
        // Opened afterwards so the worker finishes and the suite leaves no
        // thread parked.
        gate.open();
    }

    #[test]
    fn a_worker_that_says_nothing_costs_its_slice_and_no_more() {
        // The one ending a session cannot word for itself, made here by leaving
        // the last slice off the script: the stand-in panics, which is a worker
        // that closed its channel with nothing on it. The panic it prints is the
        // point of the test and not a failure in it — what is asserted is that
        // the slice is reported rather than silently counted as drafted.
        let (mut app, mut cutter, _repo, _home) = cut(
            a_project(),
            Scripted::saying([Answering::drafts(FIRST), Answering::drafts(SECOND)]),
        );

        through(&mut app, &mut cutter);

        let said = notes(&app);
        assert!(
            about(&said, THIRD)
                .iter()
                .any(|line| line.contains("was not drafted")),
            "the slice whose worker said nothing was reported as drafted: {said:?}"
        );
        assert!(
            said.iter().any(|line| line.contains(NAME)),
            "the run never named the project"
        );
    }
}

// The relay: a slice that asks something, the question on the thread, warlock's
// attempt at it in whoever's hand the field is in, and whatever they send going
// back to the session that asked. Driven in rounds at the value the loop holds,
// exactly as the loop drives it, over two scripted conversations — the slice's
// and the one the attempt is made in — so which of the two was asked for what is
// something these tests can say.
mod relaying {
    use tempfile::TempDir;
    use warlock_tui::NOTHING_SETTLES_IT;

    use super::{
        AT_MOST, Answering, App, Boarding, Cutter, FIRST, Gate, Instant, SECOND, Scripted, THIRD,
        a_project, asked_proposing, notes, now, through,
    };

    // The question the first slice comes back with, and warlock's attempt at it:
    // prose with a round left is a question, and the model that answers it is a
    // second session, so the two strings can never be confused for each other.
    const ASKED: &str = "Which of the two records does this slice write?";

    const PROPOSED: &str = "The cut record. `filed.toml` is written once, by the filing path.";

    // What somebody types over the attempt, which is what the session has to
    // hear: warlock's draft has no standing over it at all.
    const TYPED: &str = "Neither — it writes the cut record and nothing else.";

    // A confirmed run whose first slice asks, with the whole of both scripts
    // written down: the slice asks, is answered, and drafts, and the two after
    // it draft first time.
    fn asking(proposer: Scripted) -> (App, Cutter<Boarding, Scripted>, TempDir, TempDir) {
        let (mut app, mut cutter, repo, home) = asked_proposing(
            a_project(),
            Scripted::saying([
                Answering::says(ASKED),
                Answering::drafts(FIRST),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
            proposer,
        );
        cutter.cut(&mut app, now());
        waiting(&mut app, &mut cutter);
        (app, cutter, repo, home)
    }

    // Rounds until the slice under way is waiting on an answer, drained and
    // never blocked on: the loop draws and then drains, so a test that waited on
    // a channel would be a test of something the panel does not do.
    fn waiting(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) {
        let waited = Instant::now();
        while !cutter.relaying() && waited.elapsed() < AT_MOST {
            round(app, cutter);
        }
        assert!(cutter.relaying(), "no question was ever put");
    }

    // Rounds until warlock's attempt has come to something, and whatever it came
    // to: a draft for the field, or `None` and a line on the thread instead.
    // Both are one round in the panel's life, which is why one helper waits for
    // either.
    fn attempted(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) -> Option<String> {
        let waited = Instant::now();
        let said = notes(app).len();
        while waited.elapsed() < AT_MOST {
            if let Some(draft) = cutter.keep_up(app, now()) {
                return Some(draft);
            }
            if notes(app).len() > said {
                return None;
            }
        }
        panic!("the attempt at the question never came to anything");
    }

    // One round of the loop's drain, with whatever it had for the field thrown
    // away: the tests that care about the draft ask for it through `attempted`.
    fn round(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) {
        drop(cutter.keep_up(app, now()));
    }

    fn answered(said: &[String]) -> Vec<String> {
        said.iter()
            .filter(|line| line.contains("was answered:"))
            .cloned()
            .collect()
    }

    #[test]
    fn a_question_lands_on_the_thread_in_the_words_it_was_asked_and_the_slice_waits() {
        // The question is the slice's, said as it was said: nothing summarises
        // it, and the run goes no further, because what this slice says next
        // depends on what it is told.
        let (app, cutter, _repo, _home) = asking(Scripted::saying([Answering::says(PROPOSED)]));

        let said = notes(&app);
        let asked: Vec<&String> = said.iter().filter(|line| line.contains(ASKED)).collect();
        assert_eq!(
            asked.len(),
            1,
            "the question is not on the thread: {said:?}"
        );
        assert!(
            asked[0].starts_with(&format!("slice 1 `{FIRST}` asked:")),
            "{:?} does not mark the slice as having asked",
            asked[0]
        );
        assert!(
            !said.iter().any(|line| line.contains(SECOND)),
            "the run walked past a question: {said:?}"
        );
        assert!(cutter.drafting(), "the run was taken down by a question");
    }

    #[test]
    fn the_panel_says_which_slice_the_field_is_answering_for() {
        // The one thing about the field that cannot be read off the field: a
        // draft that answers somewhere else looks exactly like one that answers
        // here. Named the way every line about this slice names it.
        let (mut app, mut cutter, _repo, _home) =
            asking(Scripted::saying([Answering::says(PROPOSED)]));

        assert_eq!(
            cutter.answering(),
            Some(format!("answering slice 1 `{FIRST}`"))
        );

        cutter.answered(&mut app, TYPED, now());

        assert_eq!(
            cutter.answering(),
            None,
            "the field is still labelled for a question that is over"
        );
    }

    #[test]
    fn warlocks_attempt_comes_back_for_the_field_and_is_not_said_on_the_thread() {
        // It is a draft nobody has sent. A thread that reported it would read
        // tomorrow as though warlock had answered the question itself.
        let (mut app, mut cutter, _repo, _home) =
            asking(Scripted::saying([Answering::says(PROPOSED)]));

        let offered = attempted(&mut app, &mut cutter);

        assert_eq!(offered.as_deref(), Some(PROPOSED));
        let said = notes(&app);
        assert!(
            !said.iter().any(|line| line.contains(PROPOSED)),
            "an unsent draft was put on the thread: {said:?}"
        );
        assert!(
            cutter.relaying(),
            "the attempt answered the question itself"
        );
    }

    #[test]
    fn the_attempt_is_made_in_the_other_conversation_and_asks_about_this_slice() {
        // Not the slice's own session — whose next turn is the answer — and not
        // the panel's chat: a third conversation, told the brief, the one slice
        // and the question, and told to say so when the three do not settle it.
        let proposer = Scripted::saying([Answering::says(PROPOSED)]);
        let (mut app, mut cutter, _repo, _home) = asking(proposer.clone());

        attempted(&mut app, &mut cutter);

        let asked = proposer.said();
        assert_eq!(asked.len(), 1, "the attempt was not one turn: {asked:?}");
        assert!(asked[0].contains(ASKED), "the question was not put to it");
        assert!(asked[0].contains(FIRST), "the slice was not put to it");
        assert!(
            asked[0].contains(NOTHING_SETTLES_IT),
            "it was never told what to say when nothing settles it"
        );
    }

    #[test]
    fn the_rounds_go_on_while_the_attempt_is_being_made() {
        // The whole reason the attempt is on a worker: a drain that blocked
        // would freeze the panel for as long as a second `claude` took to read a
        // repository, with a question up and nobody able to type an answer.
        let gate = Gate::shut();
        let (mut app, mut cutter, _repo, _home) =
            asking(Scripted::saying([Answering::says(PROPOSED)]).held_at(&gate));
        let held = notes(&app).len();

        for _ in 0..3 {
            round(&mut app, &mut cutter);
            assert!(cutter.relaying(), "a held attempt took the question down");
        }

        assert_eq!(
            notes(&app).len(),
            held,
            "a round with nothing to report said something"
        );
        gate.open();
        attempted(&mut app, &mut cutter);
    }

    #[test]
    fn what_was_sent_reaches_the_session_that_asked_and_the_run_carries_on() {
        // The answer goes to the session mid-question rather than to a fresh
        // one, which is the whole point of parking it: the slice drafts on the
        // turn after, and the two slices behind it are reached.
        let agent = Scripted::saying([
            Answering::says(ASKED),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let (mut app, mut cutter, _repo, _home) = asked_proposing(
            a_project(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );
        cutter.cut(&mut app, now());
        waiting(&mut app, &mut cutter);
        attempted(&mut app, &mut cutter);

        cutter.answered(&mut app, TYPED, now());
        through(&mut app, &mut cutter);

        assert!(
            agent.said().iter().any(|turn| turn == TYPED),
            "what was sent did not reach the session in the words it was sent: {:?}",
            agent.said()
        );
        let said = notes(&app);
        assert!(
            said.iter()
                .any(|line| line.contains(THIRD) && line.contains("drafted `")),
            "the run did not reach the last slice: {said:?}"
        );
    }

    #[test]
    fn a_question_and_what_was_sent_are_marked_apart_from_each_other() {
        // Read back tomorrow, the pair says who answered: the slice asked, and
        // the panel answered. Warlock's attempt and something typed over it land
        // identically, because what went to the session is what was in the
        // field.
        let (mut app, mut cutter, _repo, _home) =
            asking(Scripted::saying([Answering::says(PROPOSED)]));
        let offered = attempted(&mut app, &mut cutter).expect("an attempt for the field");

        cutter.answered(&mut app, &offered, now());

        let said = notes(&app);
        let sent = answered(&said);
        assert_eq!(
            sent.len(),
            1,
            "what was sent is not on the thread: {said:?}"
        );
        assert_eq!(
            sent[0],
            format!("slice 1 `{FIRST}` was answered: {PROPOSED}")
        );
        assert!(
            said.iter().any(|line| line.contains("asked:")),
            "the question it answers is not on the thread: {said:?}"
        );
        assert!(
            !cutter.relaying(),
            "the question is still waiting on an answer"
        );
    }

    #[test]
    fn a_draft_cleared_and_typed_over_is_what_is_sent() {
        // Whatever the field holds at the Enter, and nothing else: the attempt
        // is an offer, not a decision, and a relay that preferred its own words
        // would be warlock answering for somebody.
        let agent = Scripted::saying([
            Answering::says(ASKED),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let (mut app, mut cutter, _repo, _home) = asked_proposing(
            a_project(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]),
        );
        cutter.cut(&mut app, now());
        waiting(&mut app, &mut cutter);
        attempted(&mut app, &mut cutter);

        cutter.answered(&mut app, TYPED, now());
        through(&mut app, &mut cutter);

        assert!(
            !agent.said().iter().any(|turn| turn.contains(PROPOSED)),
            "warlock's own draft was sent over what was typed: {:?}",
            agent.said()
        );
        assert!(
            notes(&app).iter().any(|line| line.contains(TYPED)),
            "the thread does not say what was sent"
        );
    }

    #[test]
    fn a_question_nothing_settles_is_that_sentence_on_the_thread_and_an_empty_field() {
        // The one sentence, recognised by the session that asked for it and put
        // through untouched: a guess is the one kind of answer that reaches the
        // board looking like a decision somebody made.
        let (mut app, mut cutter, _repo, _home) =
            asking(Scripted::saying([Answering::says(NOTHING_SETTLES_IT)]));

        let offered = attempted(&mut app, &mut cutter);

        assert_eq!(offered, None, "a refusal was offered as a draft");
        let said = notes(&app);
        assert!(
            said.iter().any(|line| line.contains(NOTHING_SETTLES_IT)),
            "the sentence is not on the thread: {said:?}"
        );
        assert!(
            cutter.relaying(),
            "a question nothing settles stopped being a question"
        );
    }

    #[test]
    fn a_failed_attempt_is_one_line_and_the_question_is_still_somebodys_to_answer() {
        // A missing `claude` on the second conversation costs the question
        // nothing: the session that asked is still parked, the field is empty,
        // and an answer typed into it goes where it always would.
        let agent = Scripted::saying([
            Answering::says(ASKED),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let (mut app, mut cutter, _repo, _home) = asked_proposing(
            a_project(),
            agent.clone(),
            Scripted::saying([Answering::missing()]),
        );
        cutter.cut(&mut app, now());
        waiting(&mut app, &mut cutter);

        let offered = attempted(&mut app, &mut cutter);

        assert_eq!(offered, None, "a failed attempt filled the field");
        let said = notes(&app);
        let failed: Vec<&String> = said
            .iter()
            .filter(|line| line.contains("no answer was proposed"))
            .collect();
        assert_eq!(failed.len(), 1, "a failure cost more than a line: {said:?}");
        assert!(cutter.relaying(), "a failed attempt took the question down");

        cutter.answered(&mut app, TYPED, now());
        through(&mut app, &mut cutter);

        assert!(
            agent.said().iter().any(|turn| turn == TYPED),
            "the session was not alive to be answered: {:?}",
            agent.said()
        );
    }

    #[test]
    fn an_answer_sent_before_the_attempt_lands_is_the_answer() {
        // Somebody who knows the answer does not wait for warlock's: the
        // question is over the moment they send, and the attempt still out has
        // nowhere left to be put.
        let gate = Gate::shut();
        let agent = Scripted::saying([
            Answering::says(ASKED),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let (mut app, mut cutter, _repo, _home) = asked_proposing(
            a_project(),
            agent.clone(),
            Scripted::saying([Answering::says(PROPOSED)]).held_at(&gate),
        );
        cutter.cut(&mut app, now());
        waiting(&mut app, &mut cutter);

        cutter.answered(&mut app, TYPED, now());
        gate.open();
        through(&mut app, &mut cutter);

        assert!(
            !agent.said().iter().any(|turn| turn.contains(PROPOSED)),
            "an attempt that landed late was sent: {:?}",
            agent.said()
        );
        let said = notes(&app);
        assert!(
            !said.iter().any(|line| line.contains(PROPOSED)),
            "an attempt nobody was waiting for was said: {said:?}"
        );
    }
}

// The window between a slice's drafts and the board: the titles offered, the
// three answers, and what each of them costs. Driven at the value the loop
// holds, over a workspace that answers the writes as well as the read, so what
// a create sent and what a skip did not are both things these tests can say.
mod reviewing {
    use tempfile::TempDir;
    use warlock_engine::{CutRecord, Filed, filed_path};
    use warlock_tui::{Answer, Choice};

    use super::{
        AT_MOST, Answering, App, BRIEF, Cutter, FIRST, Instant, NOT_A_KEY, PROJECT_ID, SECOND,
        Scripted, THIRD, a_project, asked_over, fs, notes, now,
    };
    use crate::stubs::{Boarding, Op};

    // What somebody types about drafts they have just read, which is the one
    // thing a redrafting session hears.
    const FEEDBACK: &str = "Two tickets is one too many — say it in one.";

    // What the redraft comes back as, named so that a window offering it cannot
    // be mistaken for the window that was up before the feedback.
    const REDRAFTED: &str = "Read the project back, said again";

    // A confirmed run over a board that can be filed into as well as read.
    fn cut(
        linear: Boarding,
        agent: Scripted,
    ) -> (App, Cutter<Boarding, Scripted>, TempDir, TempDir) {
        let (mut app, mut cutter, repo, home) = asked_over(linear, agent);
        cutter.cut(&mut app, now());
        (app, cutter, repo, home)
    }

    // The whole of the project's three slices, each drafted first time: the
    // scripts below add to this only when a slice is to be redrafted.
    fn drafting_each() -> Scripted {
        Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
    }

    // The same three slices with a redraft of the first written into the script:
    // it drafts, the reader says what is wrong with what it drafted, and it
    // answers again in its own words.
    fn redrafting() -> Scripted {
        Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(REDRAFTED),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
    }

    // Rounds until the slice under way is waiting behind the window, drained and
    // never blocked on, for the reason every other helper here is.
    fn offered(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) {
        let waited = Instant::now();
        while cutter.reviewing().is_none() && waited.elapsed() < AT_MOST {
            drop(cutter.keep_up(app, now()));
        }
        assert!(
            cutter.reviewing().is_some(),
            "no drafts were ever offered: {:?}",
            notes(app)
        );
    }

    // Rounds until the answer that was given has come to whatever it comes to:
    // the next slice's window, or a run that is over. Both are endings, and a
    // helper per ending would be two ways of saying the same wait.
    fn settled(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) {
        let waited = Instant::now();
        while cutter.drafting() && cutter.reviewing().is_none() && waited.elapsed() < AT_MOST {
            drop(cutter.keep_up(app, now()));
        }
        assert!(
            !cutter.drafting() || cutter.reviewing().is_some(),
            "the answer never came to anything: {:?}",
            notes(app)
        );
    }

    // The window's three answers, taken off the value that is up rather than
    // pressed: which key means which is `confirm.rs`'s, asserted there.
    fn titles(cutter: &Cutter<Boarding, Scripted>) -> Vec<String> {
        cutter
            .reviewing()
            .expect("a window is up")
            .titles()
            .to_vec()
    }

    // Rounds until nothing of the cut is left in flight — the project's one
    // comment included — for the reason every other helper here drains.
    fn over(app: &mut App, cutter: &mut Cutter<Boarding, Scripted>) {
        let waited = Instant::now();
        while cutter.running() && waited.elapsed() < AT_MOST {
            drop(cutter.keep_up(app, now()));
        }
        assert!(
            !cutter.running(),
            "the draft never finished: {:?}",
            notes(app)
        );
    }

    // What the record beside the brief claims, which is the one thing that stops
    // the next run filing the same drafts again.
    fn cuts(root: &std::path::Path) -> Vec<CutRecord> {
        Filed::load(root)
            .expect("a record file that loads")
            .record(BRIEF)
            .expect("the brief is filed")
            .cuts()
            .to_vec()
    }

    // Which of the run's lines is about a slice, by the prefix every line about
    // one carries.
    fn about(said: &[String], slice: &str) -> Vec<String> {
        said.iter()
            .filter(|line| line.contains(&format!("`{slice}`")))
            .cloned()
            .collect()
    }

    #[test]
    fn a_slices_drafts_wait_behind_a_window_with_their_titles_on_the_thread() {
        // Nothing a run drafts becomes an issue on its own: the titles are said
        // as they arrive and then the slice stops, with the board still holding
        // the one request the fetch made.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), drafting_each());

        offered(&mut app, &mut cutter);

        let review = cutter.reviewing().expect("a window is up");
        assert_eq!(review.slice(), format!("slice 1 `{FIRST}`"));
        assert_eq!(
            review.titles(),
            [
                format!("Stand in for {FIRST}"),
                format!("Follow on from {FIRST}"),
            ]
        );
        // Skip is lit, so the round that put this up and an Enter straight
        // after it file nothing at all.
        assert_eq!(review.choice(), Choice::Skip);
        assert!(review.feedback(), "the one redraft is not offered");
        let said = notes(&app);
        assert!(
            about(&said, FIRST)
                .iter()
                .any(|line| line.contains("drafted `")),
            "the titles are not on the thread: {said:?}"
        );
        assert_eq!(linear.requests(), 1, "the drafts were filed unasked");
        assert!(
            !said.iter().any(|line| line.contains(SECOND)),
            "the run walked past a window: {said:?}"
        );
    }

    #[test]
    fn create_files_the_slice_and_reports_what_it_became_by_identifier() {
        // The one thing about an issue that must not be lost, said in the words
        // the cut record keeps it in — and the record itself is written by the
        // filing path, which is what stops the next run filing these again.
        let linear = a_project();
        let (mut app, mut cutter, repo, _home) = cut(linear.clone(), drafting_each());
        offered(&mut app, &mut cutter);

        cutter.create(&mut app, now());
        settled(&mut app, &mut cutter);

        let said = notes(&app);
        let filed: Vec<String> = about(&said, FIRST)
            .into_iter()
            .filter(|line| line.contains("cut into"))
            .collect();
        assert_eq!(
            filed,
            vec![format!("slice 1 `{FIRST}` — cut into `WAR-1`, `WAR-2`")],
            "the identifiers are not on the thread: {said:?}"
        );
        let cuts = cuts(repo.path());
        assert_eq!(cuts.len(), 1, "the cut record was not written: {cuts:?}");
        assert_eq!(cuts[0].issues(), ["WAR-1".to_owned(), "WAR-2".to_owned()]);
        assert_eq!(
            linear.issues_created().len(),
            2,
            "one issue per draft, and no more"
        );
        // And the run goes on: the window up now is the next slice's.
        assert_eq!(
            cutter.reviewing().map(|review| review.slice().to_owned()),
            Some(format!("slice 2 `{SECOND}`"))
        );
    }

    #[test]
    fn an_edge_linear_turned_down_is_its_own_line_beside_the_identifiers() {
        // The issues exist either way, and an issue with a missing edge is
        // something a person can fix on the board — if they are told.
        let linear = a_project().refuse(Op::Relation, "that relation already exists");
        let (mut app, mut cutter, _repo, _home) = cut(linear, drafting_each());
        offered(&mut app, &mut cutter);

        cutter.create(&mut app, now());
        settled(&mut app, &mut cutter);

        let said = notes(&app);
        let refused: Vec<String> = about(&said, FIRST)
            .into_iter()
            .filter(|line| line.contains("was not written as blocking"))
            .collect();
        assert_eq!(
            refused.len(),
            1,
            "the edge Linear turned down is not on the thread: {said:?}"
        );
        assert!(
            refused[0].contains("that relation already exists"),
            "{:?} does not carry Linear's own words",
            refused[0]
        );
        assert!(
            about(&said, FIRST)
                .iter()
                .any(|line| line.contains("cut into `WAR-1`, `WAR-2`")),
            "a refused edge took the issues down with it: {said:?}"
        );
    }

    #[test]
    fn a_create_that_came_to_nothing_is_one_line_and_the_run_carries_on() {
        // A team with nowhere to put an issue is refused while the slice is
        // still nothing on the board. One line, the session alive, the panel
        // usable, and the next slice offered.
        let linear = a_project().without_team();
        let (mut app, mut cutter, repo, _home) = cut(linear, drafting_each());
        offered(&mut app, &mut cutter);

        cutter.create(&mut app, now());
        settled(&mut app, &mut cutter);

        let said = notes(&app);
        let unfiled: Vec<String> = about(&said, FIRST)
            .into_iter()
            .filter(|line| line.contains("was not filed"))
            .collect();
        assert_eq!(
            unfiled.len(),
            1,
            "a refusal cost more than a line: {said:?}"
        );
        assert!(
            cuts(repo.path()).is_empty(),
            "a slice nothing was filed for was recorded as cut"
        );
        assert!(cutter.drafting(), "a refused create took the run down");
        assert_eq!(
            cutter.reviewing().map(|review| review.slice().to_owned()),
            Some(format!("slice 2 `{SECOND}`")),
            "the run did not reach the next slice: {said:?}"
        );
    }

    #[test]
    fn a_skip_records_nothing_and_asks_whether_to_carry_on() {
        // A skipped slice is one the next `/draft` offers again, which is the
        // whole difference between skipping drafts and filing them.
        let linear = a_project();
        let (mut app, mut cutter, repo, _home) = cut(linear.clone(), drafting_each());
        offered(&mut app, &mut cutter);

        cutter.skip(&mut app, now());

        let said = notes(&app);
        assert!(
            about(&said, FIRST)
                .iter()
                .any(|line| line.contains("was skipped; nothing was recorded for it")),
            "the skip is not on the thread: {said:?}"
        );
        assert_eq!(linear.requests(), 1, "a skip sent something to the board");
        assert!(cuts(repo.path()).is_empty(), "a skip wrote a cut record");
        let carry = cutter.carrying().expect("the carry-on question is up");
        assert_eq!(carry.left(), "2 slices");
        // No is lit, so the answer that is under the finger is the one that
        // leaves the rest of the project alone.
        assert_eq!(carry.answer(), Answer::No);
        assert!(
            cutter.reviewing().is_none(),
            "the window is up behind its own answer"
        );
    }

    #[test]
    fn a_no_to_the_carry_on_question_leaves_the_slices_after_it_unoffered() {
        // The run ends here and the slices behind it are untouched: nothing was
        // drafted for them, nothing was sent about them, and the next `/draft`
        // finds them exactly as this one did.
        let linear = a_project();
        let agent = drafting_each();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), agent.clone());
        offered(&mut app, &mut cutter);
        cutter.skip(&mut app, now());

        cutter.stop(&mut app, now());

        assert!(!cutter.drafting(), "a No left the run running");
        assert_eq!(agent.turns(), 1, "a slice past the No was drafted");
        let said = notes(&app);
        assert!(
            said.iter()
                .any(|line| line == "the run stopped; 2 slices left for another draft"),
            "the run did not say what it left: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line.contains(SECOND)),
            "a slice that was never offered was named: {said:?}"
        );
        assert_eq!(linear.requests(), 1, "a stopped run sent something");
    }

    #[test]
    fn a_yes_to_the_carry_on_question_moves_to_the_next_slice() {
        // The other answer, which is a run that goes on where it left off: the
        // slice after the skipped one is drafted and offered in its turn.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear, drafting_each());
        offered(&mut app, &mut cutter);
        cutter.skip(&mut app, now());

        cutter.carry_on(&mut app, now());
        offered(&mut app, &mut cutter);

        assert_eq!(
            cutter.reviewing().map(|review| review.slice().to_owned()),
            Some(format!("slice 2 `{SECOND}`"))
        );
        assert!(
            cutter.carrying().is_none(),
            "the question is up behind its own answer"
        );
    }

    #[test]
    fn the_last_slice_skipped_ends_the_run_without_asking() {
        // A question whose only answer is "there is nothing left" is one nobody
        // should have to press a key for.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear, drafting_each());
        for _ in 0..2 {
            offered(&mut app, &mut cutter);
            cutter.skip(&mut app, now());
            cutter.carry_on(&mut app, now());
        }
        offered(&mut app, &mut cutter);

        cutter.skip(&mut app, now());

        assert!(cutter.carrying().is_none(), "the last slice asked anyway");
        assert!(!cutter.drafting(), "the run outlived its last slice");
        let said = notes(&app);
        assert!(
            about(&said, THIRD)
                .iter()
                .any(|line| line.contains("was skipped")),
            "the last slice was not reported as skipped: {said:?}"
        );
    }

    #[test]
    fn feedback_redrafts_that_one_slice_and_offers_the_window_again() {
        // The field takes whatever the reader has to say, it goes to the session
        // that drafted these — so it is feedback about something that session
        // said — and what comes back is offered in its turn.
        let linear = a_project();
        let agent = redrafting();
        let (mut app, mut cutter, _repo, _home) = cut(linear, agent.clone());
        offered(&mut app, &mut cutter);

        cutter.feedback(&mut app, now());

        assert!(cutter.relaying(), "the field is not taking the feedback");
        assert_eq!(
            cutter.answering(),
            Some(format!("redrafting slice 1 `{FIRST}`"))
        );
        assert!(
            cutter.reviewing().is_none(),
            "the window is up while its own feedback is being typed"
        );
        cutter.answered(&mut app, FEEDBACK, now());
        offered(&mut app, &mut cutter);

        assert!(
            agent.said().iter().any(|turn| turn == FEEDBACK),
            "the feedback did not reach the session that drafted: {:?}",
            agent.said()
        );
        assert_eq!(
            titles(&cutter),
            [
                format!("Stand in for {REDRAFTED}"),
                format!("Follow on from {REDRAFTED}"),
            ],
            "the window is not about what came back"
        );
        let said = notes(&app);
        assert!(
            about(&said, FIRST)
                .iter()
                .any(|line| line.contains("is being redrafted:") && line.contains(FEEDBACK)),
            "the thread does not say what was asked for: {said:?}"
        );
    }

    #[test]
    fn a_second_feedback_is_not_offered_and_answers_nothing() {
        // One redraft each: the window that comes back from one is drawn with
        // two answers, and the key that would ask for another reaches a window
        // that does not have it.
        let linear = a_project();
        let agent = redrafting();
        let (mut app, mut cutter, _repo, _home) = cut(linear, agent.clone());
        offered(&mut app, &mut cutter);
        cutter.feedback(&mut app, now());
        cutter.answered(&mut app, FEEDBACK, now());
        offered(&mut app, &mut cutter);
        let said = notes(&app).len();

        assert!(
            !cutter
                .reviewing()
                .expect("the window is up again")
                .feedback(),
            "a second redraft is offered"
        );
        cutter.feedback(&mut app, now());

        assert!(
            cutter.reviewing().is_some(),
            "a second feedback took the window down"
        );
        assert!(!cutter.relaying(), "a second feedback asked for text");
        assert_eq!(agent.turns(), 2, "the slice was redrafted twice");
        assert_eq!(
            notes(&app).len(),
            said,
            "a key that reached the wrong window said something"
        );
    }

    #[test]
    fn the_run_carries_on_past_a_redraft_with_its_own_redraft_back() {
        // The redraft is spent by the slice that used it and not by the run: the
        // slice after a redrafted one is offered its own.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear, redrafting());
        offered(&mut app, &mut cutter);
        cutter.feedback(&mut app, now());
        cutter.answered(&mut app, FEEDBACK, now());
        offered(&mut app, &mut cutter);

        cutter.skip(&mut app, now());
        cutter.carry_on(&mut app, now());
        offered(&mut app, &mut cutter);

        assert!(
            cutter
                .reviewing()
                .expect("the next slice's window is up")
                .feedback(),
            "the next slice has no redraft of its own"
        );
    }

    #[test]
    fn an_answer_to_a_window_that_is_not_up_does_nothing_at_all() {
        // A key that arrived a round late, which the loop's one question cannot
        // rule out: every answer here is a no-op with nothing waiting, so a
        // stale press cannot file drafts nobody was looking at.
        let linear = a_project();
        let (mut app, mut cutter, repo, _home) = cut(linear.clone(), drafting_each());

        cutter.create(&mut app, now());
        cutter.skip(&mut app, now());
        cutter.feedback(&mut app, now());
        cutter.carry_on(&mut app, now());
        cutter.stop(&mut app, now());
        cutter.review_lit(Choice::Create);
        cutter.carry_lit(Answer::Yes);

        assert!(cutter.drafting(), "a stale answer took the run down");
        assert_eq!(linear.requests(), 1, "a stale answer sent something");
        assert!(
            cuts(repo.path()).is_empty(),
            "a stale answer wrote a record"
        );
    }

    #[test]
    fn an_arrow_lights_another_answer_and_leaves_the_drafts_where_they_were() {
        // Answering re-lights the window rather than building a second one: what
        // is being answered about rides along, because nothing would hand the
        // drafts over again.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear, drafting_each());
        offered(&mut app, &mut cutter);
        let offered_titles = titles(&cutter);

        cutter.review_lit(Choice::Create);

        let review = cutter.reviewing().expect("the window is still up");
        assert_eq!(review.choice(), Choice::Create);
        assert_eq!(review.titles(), offered_titles.as_slice());
        assert_eq!(review.slice(), format!("slice 1 `{FIRST}`"));

        cutter.skip(&mut app, now());
        cutter.carry_lit(Answer::Yes);

        let carry = cutter.carrying().expect("the question is still up");
        assert_eq!(carry.answer(), Answer::Yes);
        assert_eq!(carry.left(), "2 slices");
    }

    #[test]
    fn nothing_a_run_sends_is_a_mutation_of_the_project() {
        // The promise the whole command is written around: a cut reads the
        // project, files issues out of it and edges between them, says one
        // comment on it, and leaves it exactly as it found it — `Planned`, with
        // nobody assigned to anything.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), drafting_each());
        for _ in 0..3 {
            offered(&mut app, &mut cutter);
            cutter.create(&mut app, now());
            settled(&mut app, &mut cutter);
        }
        over(&mut app, &mut cutter);

        // A board has no operation that moves a status; what a run can still
        // get wrong is writing something other than an issue, an edge or that
        // one comment — a second project among them.
        let sent: Vec<Op> = linear
            .ops()
            .into_iter()
            .filter(|op| {
                matches!(
                    op,
                    Op::CreateProject | Op::CreateIssue | Op::Relation | Op::Comment
                )
            })
            .collect();
        assert!(!sent.is_empty(), "nothing was filed at all");
        for op in &sent {
            assert!(
                matches!(op, Op::CreateIssue | Op::Relation | Op::Comment),
                "a run wrote something that is not an issue, an edge or the comment: {op:?}"
            );
        }
    }

    #[test]
    fn a_run_that_created_issues_says_the_projects_one_comment_after_its_last_slice() {
        // The comment `warlock draft` says, said from the panel too: once, after
        // the last slice settles, naming what this run created. A comment that
        // was said is on the project and nowhere on the thread.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), drafting_each());
        for _ in 0..3 {
            offered(&mut app, &mut cutter);
            cutter.create(&mut app, now());
            settled(&mut app, &mut cutter);
        }
        let said = notes(&app).len();

        over(&mut app, &mut cutter);

        let comments = linear.comments();
        assert_eq!(comments.len(), 1, "{comments:?}");
        let (project, body) = &comments[0];
        assert_eq!(project, PROJECT_ID);
        for issue in ["WAR-1", "WAR-2", "WAR-3", "WAR-4", "WAR-5", "WAR-6"] {
            assert!(body.contains(issue), "{body}");
        }
        let commented = linear.positions_of(Op::Comment)[0];
        let last = *linear
            .positions_of(Op::CreateIssue)
            .last()
            .expect("issues were created");
        assert!(commented > last, "{commented} is not after {last}");
        assert_eq!(
            notes(&app).len(),
            said,
            "a comment that was said was reported"
        );
    }

    #[test]
    fn a_run_stopped_partway_still_says_what_it_did_create() {
        // A No to carrying on ends the run as surely as the last slice does, and
        // the issues filed before it are this run's to name.
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), drafting_each());
        offered(&mut app, &mut cutter);
        cutter.create(&mut app, now());
        settled(&mut app, &mut cutter);
        cutter.skip(&mut app, now());

        cutter.stop(&mut app, now());
        over(&mut app, &mut cutter);

        let comments = linear.comments();
        assert_eq!(comments.len(), 1, "{comments:?}");
        assert!(comments[0].1.contains("`WAR-1`, `WAR-2`"), "{comments:?}");
        assert!(!comments[0].1.contains("WAR-3"), "{comments:?}");
    }

    #[test]
    fn a_run_that_created_nothing_says_nothing_on_the_project() {
        let linear = a_project();
        let (mut app, mut cutter, _repo, _home) = cut(linear.clone(), drafting_each());
        for _ in 0..2 {
            offered(&mut app, &mut cutter);
            cutter.skip(&mut app, now());
            cutter.carry_on(&mut app, now());
        }
        offered(&mut app, &mut cutter);

        cutter.skip(&mut app, now());

        assert!(
            !cutter.running(),
            "a run that created nothing is still running"
        );
        assert!(linear.comments().is_empty(), "{:?}", linear.comments());
        assert_eq!(
            linear.requests(),
            1,
            "a run that created nothing sent something"
        );
    }

    #[test]
    fn a_comment_linear_turned_down_is_a_line_on_the_thread() {
        // The issues exist and are recorded by the time the comment is said, so
        // its refusal is a line and nothing is undone.
        let linear = a_project().refuse(Op::Comment, "the workspace would not");
        let (mut app, mut cutter, repo, _home) = cut(linear, drafting_each());
        for _ in 0..3 {
            offered(&mut app, &mut cutter);
            cutter.create(&mut app, now());
            settled(&mut app, &mut cutter);
        }

        over(&mut app, &mut cutter);

        let said = notes(&app);
        let last = said.last().expect("the run said something");
        assert!(
            last.contains("the project was not commented on"),
            "{said:?}"
        );
        assert!(last.contains("the workspace would not"), "{said:?}");
        assert_eq!(
            cuts(repo.path()).len(),
            3,
            "a refused comment undid the cut"
        );
    }

    #[test]
    fn no_key_value_reaches_the_thread_the_record_or_anything_the_run_holds() {
        // The cut's half of the claim `tests/pushing.rs` makes for a push, and
        // it is made here because this is the only module that drives a run the
        // whole way to a written record: the key store this home holds is what
        // the filing worker built its client from, so the value has been through
        // every step below. It is to be in none of them — not on a line
        // somebody reads, not in the record beside the brief, and not in the
        // `Debug` rendering that a failing assertion anywhere else in the suite
        // would print.
        let linear = a_project();
        let (mut app, mut cutter, repo, _home) = cut(linear.clone(), drafting_each());
        offered(&mut app, &mut cutter);
        // Read while the window is up, so what is asserted is the run in
        // flight as well as the run that is over.
        let in_flight = format!("{cutter:?}");

        for _ in 0..3 {
            offered(&mut app, &mut cutter);
            cutter.create(&mut app, now());
            settled(&mut app, &mut cutter);
        }

        assert!(!cutter.drafting(), "the run never finished");
        assert!(!in_flight.contains(NOT_A_KEY), "the run carries the key");
        assert!(
            !format!("{cutter:?}").contains(NOT_A_KEY),
            "the session's draft state carries the key"
        );
        let said = notes(&app);
        assert!(!said.is_empty(), "the run said nothing at all");
        for line in &said {
            assert!(!line.contains(NOT_A_KEY), "{line} carries the key");
        }
        assert!(
            !fs::read_to_string(filed_path(repo.path()))
                .expect("the record this run wrote")
                .contains(NOT_A_KEY),
            "the record carries the key"
        );
        // And nothing the run sent carried it either: the key opens the board
        // and is in no call made of it.
        let calls = format!("{:?}", linear.calls());
        assert!(
            !calls.contains(NOT_A_KEY),
            "a call carries the key: {calls}"
        );
    }
}
