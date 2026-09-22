//! Every test here drives [`Pulls`] over a temporary repository and a temporary
//! home, through the seam the event loop uses: a stand-in Linear handed in as
//! the [`Opens`] the value was built with. Nothing below reads the sigils, the
//! binding or the key store of the machine the suite runs on, and the one key
//! any of it stores is not one.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;
use warlock_engine::{
    CutRecord, Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, save_key, save_key_binding,
    save_sigils,
};
use warlock_tui::{App, Converses, Line, LinearError, Posts};

use super::{ALREADY_PULLING, Pulls};
use crate::error::{Error, one_line};
use crate::pushing::Opens;
use crate::stubs::{Answering, Gate, Reading, Scripted};

// Not a key, and named so that nothing reading this file mistakes it for one:
// it is stored only so that a bound name resolves and the worker has something
// to build a client from, which is the one line that sees it.
const NOT_A_KEY: &str = "not-a-real-key-value";

// A name no real key store would be holding, so a pull that reached the
// machine's own home could not pass for one that reached this test's.
const KEY_NAME: &str = "this-tests-own-name";

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

// The manifest's own spelling of the brief, which is what a filed record is
// keyed by and so what a `/pull` carries.
const BRIEF: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/pull-a-brief-1a2b3c";

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

// A description with no scope block in it at all, which is the project somebody
// filed and never edited.
const NO_SCOPE: &str = "Nothing cuts a planned project into tickets.\n\n\
                        ## Out of scope\n\nEverything.\n";

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
// own text is beside the point here — what a pull reads is the project, and the
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

// A client the worker may never reach: the refusals below are decided before a
// key is read, so being posted to at all is the failure the test is about.
#[derive(Debug, Clone, Copy)]
struct Unreachable;

impl Opens for Unreachable {
    type Client = Self;

    fn open(&self, _key: &str) -> Self {
        panic!("the key was read and a client was built");
    }
}

impl Posts for Unreachable {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        panic!("a request was sent: {document}");
    }
}

// A `/pull` pressed at the value the loop holds. The home is the value's own,
// handed to it when it was built, which is what keeps every test here off the
// machine's.
fn press<O: Opens, A: Converses>(app: &mut App, pulls: &mut Pulls<O, A>, repo: &Path) {
    pulls.press(app, &a_manifest(), repo, BRIEF, now());
}

// Rounds until the fetch has reported, drained and never blocked on: the loop
// draws and then drains, so a test that waited on the channel would be a test
// of something the panel does not do.
fn landing<O: Opens, A: Converses>(app: &mut App, pulls: &mut Pulls<O, A>) -> usize {
    let waited = Instant::now();
    let mut rounds = 0;
    while pulls.fetching() && waited.elapsed() < AT_MOST {
        pulls.keep_up(app, now());
        rounds += 1;
    }
    assert!(!pulls.fetching(), "the pull never reported");
    rounds
}

// The engine's own sentence for a refusal, flattened as the thread takes it:
// these tests assert the line *is* that sentence rather than restating it.
fn refusal(error: &Error) -> String {
    one_line(&error.to_string())
}

// The workspace every test here reads back: one project, planned, with three
// slices under its scope block.
fn a_project() -> Reading {
    Reading::holding(NAME, Some(STATUS), SLICED)
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
fn asked_over(
    linear: Reading,
    agent: Scripted,
) -> (App, Pulls<Reading, Scripted>, TempDir, TempDir) {
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let mut pulls = Pulls::with_client(linear, Some(home.path().to_path_buf()), agent);
    let mut app = App::default();
    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);
    (app, pulls, repo, home)
}

#[test]
fn a_pull_reports_the_project_its_status_and_how_many_slices_are_left() {
    // The whole sequence with nothing in its way: the board resolved, the
    // record read, the project fetched, the gate passed, the scope block
    // parsed. One request, and the line names what the reader is about to be
    // asked about.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let linear = Reading::holding(NAME, Some(STATUS), SLICED);
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()), unasked());
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    let notes = notes(&app);
    let said = notes.last().expect("the pull said nothing");
    assert!(said.contains(NAME), "{said:?} does not name the project");
    assert!(
        said.contains(STATUS),
        "{said:?} does not say the board's own spelling of the status"
    );
    assert!(said.contains("3 slices"), "{said:?} does not count slices");
    assert_eq!(linear.requests(), 1, "one pull is one request");
    assert!(
        !format!("{pulls:?}").contains(NOT_A_KEY),
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
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), SLICED),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    let said = notes(&app).last().cloned().expect("the pull said nothing");
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
    let linear = Reading::holding(NAME, Some("Backlog"), SLICED);
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()), unasked());
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    assert_eq!(
        notes(&app).last(),
        Some(&refusal(&Error::NotPlanned {
            path: BRIEF.to_owned(),
            status: Some("Backlog".to_owned()),
        })),
        "the line is not the engine's own sentence",
    );
    assert_eq!(linear.requests(), 1, "the gate sent a second request");
    assert!(!pulls.fetching(), "a refusal left a pull running");
}

#[test]
fn a_project_in_no_status_at_all_is_refused_in_the_same_words() {
    // A workspace whose board has no statuses answers `null`, which is not a
    // broken answer and is not `Planned` either.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, None, SLICED),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    assert_eq!(
        notes(&app).last(),
        Some(&refusal(&Error::NotPlanned {
            path: BRIEF.to_owned(),
            status: None,
        })),
    );
}

#[test]
fn a_brief_no_record_claims_is_refused_before_a_key_is_read() {
    // `.warlock/filed.toml` is what turns a path into a project id, so a brief
    // nothing filed is answered on this machine: no client is built, which the
    // seam here asserts by panicking if one is.
    let repo = a_repository();
    let home = a_home(repo.path());
    let mut pulls = Pulls::with_client(Unreachable, Some(home.path().to_path_buf()), unasked());
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    assert_eq!(
        notes(&app).last(),
        Some(&refusal(&Error::NoRecord {
            path: BRIEF.to_owned(),
        })),
    );
}

#[test]
fn a_description_with_no_scope_block_is_the_parsers_own_line() {
    // The project is planned and the request was answered; what came back is
    // not a brief. The sentence is the scope parser's, because it is the one
    // that knows what it was looking for.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), NO_SCOPE),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    let said = notes(&app).last().cloned().expect("the pull said nothing");
    assert!(!said.contains('\n'), "{said:?} is more than one line");
    assert_ne!(said.as_str(), NAME, "a project with no scope was reported");
}

#[test]
fn a_project_with_every_slice_cut_is_refused_rather_than_reported() {
    // Nothing to do is not a run of nothing: the record says all three are
    // filed, so the pull says so and stops.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[FIRST, SECOND, THIRD]);
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), SLICED),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    landing(&mut app, &mut pulls);

    assert_eq!(
        notes(&app).last(),
        Some(&refusal(&Error::AllCut {
            path: BRIEF.to_owned(),
        })),
    );
}

#[test]
fn a_machine_with_no_home_is_refused_before_anything_is_read() {
    // The one refusal this value answers itself, in `Standing::home`'s own
    // words: with no home there are no sigils, no binding and no key store, so
    // there is no board to resolve and nothing to read the record for.
    let repo = a_repository();
    filed(repo.path(), &[]);
    let mut pulls = Pulls::with_client(Unreachable, None, unasked());
    let mut app = App::default();

    pulls.press(&mut app, &a_manifest(), repo.path(), BRIEF, now());

    assert_eq!(notes(&app), vec![refusal(&Error::NoHome)]);
    assert!(!pulls.fetching(), "a refusal started a pull");
}

#[test]
fn a_second_pull_with_one_in_flight_is_one_line_and_reads_nothing() {
    // One pull at a time, and the say-no is the run itself. The first is held
    // open at the gate, so the second arrives while it is genuinely in flight;
    // what it costs is a line.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let linear = Reading::holding(NAME, Some(STATUS), SLICED).held_at(&gate);
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()), unasked());
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    press(&mut app, &mut pulls, repo.path());

    assert_eq!(
        notes(&app).last(),
        Some(&ALREADY_PULLING.to_owned()),
        "the second pull did not say no",
    );
    gate.open();
    landing(&mut app, &mut pulls);
    assert_eq!(linear.requests(), 1, "the second pull sent something");
}

#[test]
fn the_rounds_go_on_while_the_request_is_in_flight() {
    // The whole reason the fetch is on a worker: a drain that blocked would
    // freeze the panel for as long as Linear took to answer. The gate holds the
    // request open, the rounds are counted, and the pull reports afterwards.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());
    let held = notes(&app).len();
    for _ in 0..3 {
        pulls.keep_up(&mut app, now());
        assert!(pulls.fetching(), "the drain answered a held request");
    }

    assert_eq!(
        notes(&app).len(),
        held,
        "a round with nothing to report said something",
    );
    gate.open();
    landing(&mut app, &mut pulls);
    let said = notes(&app).last().cloned().expect("the pull said nothing");
    assert!(said.contains(NAME), "{said:?} does not name the project");
}

#[test]
fn a_pull_says_which_document_it_is_reading_before_the_board_answers() {
    // The answer is a request away and the reader has just typed the command,
    // so the thread names the document at once — by the document, because the
    // project has no name on this side of the request.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut pulls, repo.path());

    let said = notes(&app).last().cloned().expect("the pull said nothing");
    assert!(said.contains(BRIEF), "{said:?} does not name the document");
    assert!(pulls.fetching(), "the pull is not running");
    gate.open();
    landing(&mut app, &mut pulls);
}

#[test]
fn dropping_the_session_ends_the_pull_without_waiting_for_it() {
    // What quitting does: the guard goes with the value, the worker is
    // cancelled, and nothing on this side waits for the request it is sitting
    // in. A pull that had to be joined would hold the terminal until Linear
    // answered.
    let repo = a_repository();
    let home = a_home(repo.path());
    filed(repo.path(), &[]);
    let gate = Gate::shut();
    let mut pulls = Pulls::with_client(
        Reading::holding(NAME, Some(STATUS), SLICED).held_at(&gate),
        Some(home.path().to_path_buf()),
        unasked(),
    );
    let mut app = App::default();
    press(&mut app, &mut pulls, repo.path());

    let dropped = Instant::now();
    drop(pulls);

    assert!(
        dropped.elapsed() < AT_MOST,
        "dropping the value waited for the request"
    );
    // Opened afterwards so the worker finishes and the suite leaves no thread
    // parked: a gate nobody opens is a thread held until the binary exits.
    gate.open();
}

// The question between the fetch and the run, asked of the value the loop
// holds: the session answers it through `Pulls` and not through the dialog, so
// these drive the same three calls `Session::pull_answered` makes.
mod asking {
    use tempfile::TempDir;
    use warlock_tui::Answer;

    use super::{
        Answering, App, BRIEF, FIRST, Instant, KEY_NAME, NAME, NOT_A_KEY, Pulls, Reading, SLICED,
        STATUS, Scripted, TEAM, a_home, a_project, a_repository, asked_over, filed, landing, notes,
        now, press, refusal, unasked,
    };
    use crate::error::Error;

    fn asked() -> (App, Pulls<Reading, Scripted>, TempDir, TempDir) {
        asked_over(a_project(), unasked())
    }

    #[test]
    fn a_landed_fetch_puts_the_question_up_carrying_what_it_read() {
        // The five facts come off the one request rather than out of a second
        // one, which is why they are parked on the question at all.
        let (_app, pulls, _repo, _home) = asked();

        let cutting = pulls.confirm().cutting().expect("the question is not up");
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
    fn a_refused_pull_asks_nothing() {
        // There is nothing to confirm about a project the gate turned away, so
        // the refusal is the whole of what the round does.
        let repo = a_repository();
        let home = a_home(repo.path());
        filed(repo.path(), &[]);
        let mut pulls = Pulls::with_client(
            Reading::holding(NAME, Some("Backlog"), SLICED),
            Some(home.path().to_path_buf()),
            unasked(),
        );
        let mut app = App::default();

        press(&mut app, &mut pulls, repo.path());
        landing(&mut app, &mut pulls);

        assert!(
            !pulls.confirm().is_open(),
            "a refused pull asked to be confirmed"
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
        let (_app, mut pulls, _repo, _home) = asked();

        pulls.lit(Answer::Yes);

        let cutting = pulls.confirm().cutting().expect("the question came down");
        assert_eq!(cutting.answer(), Answer::Yes);
        assert_eq!(cutting.project(), NAME);
        assert_eq!(cutting.slices(), 3);
    }

    #[test]
    fn a_no_takes_the_question_down_and_leaves_the_session_where_it_was() {
        // What a No costs: the window, and nothing else. No second request, no
        // line on the thread and nothing written.
        let (mut app, mut pulls, repo, _home) = asked();
        let said = notes(&app);

        pulls.cancelled();

        assert!(!pulls.confirm().is_open(), "the question is still up");
        assert_eq!(notes(&app), said, "a No said something");
        assert!(!pulls.fetching(), "a No started a pull");
        // And the value is back where a session with no pull in it sits, so the
        // next `/pull` is allowed.
        press(&mut app, &mut pulls, repo.path());
        landing(&mut app, &mut pulls);
        assert!(pulls.confirm().is_open(), "the next pull could not ask");
    }

    #[test]
    fn a_yes_takes_the_question_down_and_starts_the_run() {
        // The window down, the project named and the first slice already being
        // drafted, all on the round the question was answered: a Yes that only
        // said something would leave the run to start on whatever round the
        // loop next came by.
        let (mut app, mut pulls, _repo, _home) =
            asked_over(a_project(), Scripted::saying([Answering::drafts(FIRST)]));
        let said = notes(&app).len();

        pulls.cut(&mut app, Instant::now());

        assert!(
            !pulls.confirm().is_open(),
            "the question is up over its own run"
        );
        assert!(pulls.drafting(), "a Yes started no run");
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
        let mut pulls = Pulls::with_client(
            Reading::holding(NAME, Some(STATUS), SLICED),
            Some(home.path().to_path_buf()),
            unasked(),
        );
        let mut app = App::default();

        pulls.lit(Answer::Yes);
        pulls.cancelled();
        pulls.cut(&mut app, now());

        assert!(!pulls.confirm().is_open());
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
        AT_MOST, Answering, App, FIRST, Gate, Instant, NAME, Pulls, Reading, SECOND, Scripted,
        THIRD, a_home, a_project, a_repository, asked_over, filed, landing, notes, now, press,
    };

    // The three `[n/total]` prefixes a run over this project says, in the order
    // it says them: the fraction is the place in the cut order and the position
    // is where the slice sits in the document, which is `pull.rs`'s register and
    // not a second one.
    const RUNNING: [&str; 3] = [
        "[1/3] slice 1 `Read the project back` — drafting",
        "[2/3] slice 2 `Parse the scope block` — drafting",
        "[3/3] slice 3 `File the drafts` — drafting",
    ];

    // Rounds until the run has finished, drained and never blocked on: the loop
    // draws and then drains, so a test that waited on a channel would be a test
    // of something the panel does not do.
    fn through(app: &mut App, pulls: &mut Pulls<Reading, Scripted>) {
        let waited = Instant::now();
        while pulls.drafting() && waited.elapsed() < AT_MOST {
            pulls.keep_up(app, now());
        }
        assert!(!pulls.drafting(), "the run never finished");
    }

    // A confirmed question and the run it starts, over a scripted model.
    fn cut(linear: Reading, agent: Scripted) -> (App, Pulls<Reading, Scripted>, TempDir, TempDir) {
        let (mut app, mut pulls, repo, home) = asked_over(linear, agent);
        pulls.cut(&mut app, now());
        (app, pulls, repo, home)
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
        let (mut app, mut pulls, _repo, _home) = cut(linear.clone(), agent.clone());

        through(&mut app, &mut pulls);

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
        let mut pulls =
            Pulls::with_client(a_project(), Some(home.path().to_path_buf()), agent.clone());
        let mut app = App::default();
        press(&mut app, &mut pulls, repo.path());
        landing(&mut app, &mut pulls);

        pulls.cut(&mut app, now());
        through(&mut app, &mut pulls);

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
        let (mut app, mut pulls, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::drafts(FIRST),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut pulls);

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
        let (mut app, mut pulls, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::says(a_stray_reference(FIRST)),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut pulls);

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
        let (mut app, mut pulls, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::missing(),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut pulls);

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
    fn a_question_is_said_in_the_words_it_was_asked_and_the_slice_is_left() {
        // Prose with rounds left is a question, and there is nothing here to
        // answer one with yet — so it is said and the slice is left uncut, which
        // is what the relay replaces.
        const ASKED: &str = "Which of the two records does this slice write?";
        let (mut app, mut pulls, _repo, _home) = cut(
            a_project(),
            Scripted::saying([
                Answering::says(ASKED),
                Answering::drafts(SECOND),
                Answering::drafts(THIRD),
            ]),
        );

        through(&mut app, &mut pulls);

        let said = notes(&app);
        let asked = about(&said, FIRST);
        assert!(
            asked.iter().any(|line| line.contains(ASKED)),
            "the question is not on the thread in the words it was asked: {said:?}"
        );
        assert!(
            !asked.iter().any(|line| line.contains("drafted `")),
            "a slice that asked a question was drafted anyway: {said:?}"
        );
        assert!(
            about(&said, SECOND)
                .iter()
                .any(|line| line.contains("drafted `")),
            "the run stopped at the question: {said:?}"
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
        let (mut app, mut pulls, _repo, _home) = cut(a_project(), agent);
        let held = notes(&app).len();

        for _ in 0..3 {
            pulls.keep_up(&mut app, now());
            assert!(pulls.drafting(), "the drain answered a held turn");
        }

        assert_eq!(
            notes(&app).len(),
            held,
            "a round with nothing to report said something"
        );
        gate.open();
        through(&mut app, &mut pulls);
    }

    #[test]
    fn a_second_pull_while_a_slice_is_drafting_reads_nothing() {
        // One pull at a time, and the run is its own say-no: two sets of
        // sessions would be two runs cutting one project.
        let gate = Gate::shut();
        let linear = a_project();
        let (mut app, mut pulls, repo, _home) = cut(
            linear.clone(),
            Scripted::saying([Answering::drafts(FIRST)]).held_at(&gate),
        );

        press(&mut app, &mut pulls, repo.path());

        assert_eq!(
            notes(&app).last(),
            Some(&super::ALREADY_PULLING.to_owned()),
            "the second pull did not say no"
        );
        assert_eq!(linear.requests(), 1, "the second pull read the board");
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
        let (_app, pulls, _repo, _home) = cut(a_project(), agent.clone());
        assert!(pulls.drafting(), "there is no run to cancel");

        let dropped = Instant::now();
        drop(pulls);

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
        let (mut app, mut pulls, _repo, _home) = cut(
            a_project(),
            Scripted::saying([Answering::drafts(FIRST), Answering::drafts(SECOND)]),
        );

        through(&mut app, &mut pulls);

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
