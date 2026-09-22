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
use warlock_tui::{App, Line, LinearError, Posts};

use super::{ALREADY_PULLING, Pulls};
use crate::error::{Error, one_line};
use crate::pushing::Opens;
use crate::stubs::{Gate, Reading};

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
fn press<O: Opens>(app: &mut App, pulls: &mut Pulls<O>, repo: &Path) {
    pulls.press(app, &a_manifest(), repo, BRIEF, now());
}

// Rounds until the fetch has reported, drained and never blocked on: the loop
// draws and then drains, so a test that waited on the channel would be a test
// of something the panel does not do.
fn landing<O: Opens>(app: &mut App, pulls: &mut Pulls<O>) -> usize {
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
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()));
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
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()));
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
    let mut pulls = Pulls::with_client(Unreachable, Some(home.path().to_path_buf()));
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
    let mut pulls = Pulls::with_client(Unreachable, None);
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
    let mut pulls = Pulls::with_client(linear.clone(), Some(home.path().to_path_buf()));
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
