use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{Value, json};
use tempfile::TempDir;
use warlock_engine::{
    Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, filed_path, keys_path, manifest_path,
    route, save_key, save_key_binding, save_sigils, sigils_path,
};
use warlock_tui::{Brief, LinearError, Posts, brief_at};

use super::{Board, pushed, sent};
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;

// Not a key, and named so that nothing reading this file mistakes it for one.
// It is stored only so that a bound name resolves and a push can reach the
// client, and the tests below assert it is in nothing warlock printed.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "work";

const SCOPE: &str = "warlock-team";

// A Linear team *key* rather than a team name, which is what a `[[scope]]`
// record's `team` is and what the unknown-team refusal is worded about.
const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const BRIEF_PATH: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/push-a-brief-1a2b3c";

// Every section the built-in shape asks for, so a repository that has written
// no template of its own holds this document to something it satisfies.
const BRIEF: &str = "# Push a brief to the board\n\n\
                     Nothing turns a document on disk into a project.\n\n\
                     ## Outcome\n\n`warlock push docs/brief.md` files it.\n\n\
                     ## Success criteria\n\n**The reader**\n\n- sees a URL\n\n\
                     ## Constraints\n\nNo new dependency.\n\n\
                     ## Out of scope\n\nPulling anything back.\n\n\
                     ## Scope\n\n### 1. Read the file\n\ndepends_on: []\n";

const TITLE: &str = "Push a brief to the board";

// A Linear that answers from memory, held by handle: the closure `pushed` opens
// its client through has to hand one *over*, and the test keeps a copy to read
// afterwards. `RefCell` rather than the `Mutex` the client's own stand-in uses,
// because nothing here crosses a thread.
//
// It also looks at `.warlock/filed.toml` on every call, which is what makes
// "the record is appended after the create" an assertion rather than a reading:
// a record already on disk at the fourth request would be one written before
// the project it names existed.
#[derive(Debug, Clone)]
struct Posting {
    answers: Rc<RefCell<VecDeque<Result<Value, LinearError>>>>,
    asked: Rc<RefCell<Vec<(String, Value)>>>,
    recorded: Rc<RefCell<Vec<bool>>>,
    filed: PathBuf,
}

impl Posting {
    fn answering(
        root: &Path,
        answers: impl IntoIterator<Item = Result<Value, LinearError>>,
    ) -> Self {
        Self {
            answers: Rc::new(RefCell::new(answers.into_iter().collect())),
            asked: Rc::new(RefCell::new(Vec::new())),
            recorded: Rc::new(RefCell::new(Vec::new())),
            filed: filed_path(root),
        }
    }

    fn documents(&self) -> Vec<String> {
        self.asked
            .borrow()
            .iter()
            .map(|(document, _)| document.clone())
            .collect()
    }

    // The `input` of the last thing the stand-in was asked, which is the project
    // create in every test that gets that far.
    fn last_input(&self) -> Value {
        self.asked
            .borrow()
            .last()
            .expect("the stand-in was asked at least once")
            .1["input"]
            .clone()
    }

    fn recorded_when_asked(&self) -> Vec<bool> {
        self.recorded.borrow().clone()
    }
}

impl Posts for Posting {
    fn post(&self, document: &str, variables: Value) -> Result<Value, LinearError> {
        self.asked
            .borrow_mut()
            .push((document.to_owned(), variables));
        self.recorded.borrow_mut().push(self.filed.exists());

        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("the stand-in was asked more times than it was answered"))
    }
}

// The client a refusal may never build. Every refusal a push has happens before
// the socket is opened, so reaching this at all is the failure the test is
// about — which is why it is a panic and not a recorded flag.
#[derive(Debug, Clone, Copy)]
struct Unreachable;

impl Posts for Unreachable {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        panic!("a request was sent: {document}");
    }
}

fn no_socket(_key: &str) -> Unreachable {
    panic!("the key was read and a client was built");
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn a_manifest(scopes: impl IntoIterator<Item = ScopeRecord>) -> Manifest {
    Manifest::with_entries([PactEntry::new(".", "docs", "docs/WARLOCK.md")
        .expect("a relative module path is inside the root")
        .with_scope(SCOPE)])
    .with_scopes(scopes)
}

fn a_record(name: &str, team: &str) -> ScopeRecord {
    ScopeRecord::new(name, team, "In Review", LABEL)
}

// A repository holding one recorded scope and one brief: the ordinary finished
// setup, which each refusal below then takes one piece out of.
fn a_repository() -> TempDir {
    let repo = a_dir();
    saving(repo.path(), &a_manifest([a_record(SCOPE, TEAM)]));
    write_brief(repo.path(), BRIEF);
    repo
}

fn saving(root: &Path, manifest: &Manifest) {
    manifest.save(root).expect("a manifest that saves");
}

fn write_brief(root: &Path, document: &str) -> PathBuf {
    let path = root.join("docs").join("brief.md");
    fs::create_dir_all(path.parent().expect("a `docs` directory")).expect("a `docs` directory");
    fs::write(&path, document).expect("a brief file");
    path
}

// The home of a machine that holds the sigil, has bound a name and stores a key
// under it: everything a push needs, under a directory of this test's own.
fn a_home(root: &Path) -> TempDir {
    let home = a_dir();
    holding(home.path(), root, &[SCOPE]);
    bound(home.path(), root, KEY_NAME);
    keeping(home.path(), KEY_NAME);
    home
}

fn holding(home: &Path, root: &Path, sigils: &[&str]) {
    let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home, root, &sigils).expect("a config that writes");
}

fn bound(home: &Path, root: &Path, name: &str) {
    save_key_binding(home, root, name).expect("a binding that writes");
}

fn keeping(home: &Path, name: &str) {
    save_key(home, name, NOT_A_KEY).expect("a key store that writes");
}

// The whole subcommand, less the environment: the repository root and the home
// are this test's temporary directories, and the socket is whatever `open` is.
fn push_to<P: Posts, O: FnOnce(&str) -> P>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    dry_run: bool,
    open: O,
) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = pushed(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        home,
        Path::new(path),
        scope,
        dry_run,
        open,
        &mut out,
    );

    (
        outcome,
        String::from_utf8(out).expect("warlock writes its own text"),
    )
}

// A push that must refuse before the socket is opened, which is every refusal
// this command has. What comes back is the error, once the three things every
// one of them promises have been checked: nothing was sent, nothing was
// printed, nothing was recorded, and the status is the ordinary 1.
fn refusal(repo: &Path, home: &Path, path: &str, scope: Option<&str>) -> Error {
    let (outcome, printed) = push_to(repo, home, path, scope, false, no_socket);

    assert!(printed.is_empty(), "a refusal printed something: {printed}");
    assert!(
        !filed_path(repo).exists(),
        "a refusal wrote a record of a project nobody created"
    );
    assert_eq!(status_for(&outcome), 1, "a push refusal is the ordinary 1");
    assert_ne!(
        status_for(&outcome),
        3,
        "a push refusal took the boundary's status"
    );

    outcome.expect_err("a refusal")
}

fn said(error: &Error) -> String {
    let message = error.to_string();
    assert!(!message.contains('\n'), "`main` prints one line: {message}");
    message
}

fn team_found() -> Value {
    json!({ "teams": { "nodes": [{ "id": "team-1" }] } })
}

fn no_team() -> Value {
    json!({ "teams": { "nodes": [] } })
}

fn backlog() -> Value {
    json!({
        "projectStatuses": {
            "nodes": [
                { "id": "status-planned", "name": "Planned" },
                { "id": "status-backlog", "name": "Backlog" },
            ],
        },
    })
}

fn label_found() -> Value {
    json!({ "projectLabels": { "nodes": [{ "id": "label-held" }] } })
}

fn project_created() -> Value {
    json!({ "projectCreate": { "project": { "id": PROJECT_ID, "url": URL } } })
}

// The four answers a push that files something gets, in the order it asks for
// them.
fn a_whole_push(root: &Path) -> Posting {
    Posting::answering(
        root,
        [
            Ok(team_found()),
            Ok(backlog()),
            Ok(label_found()),
            Ok(project_created()),
        ],
    )
}

fn brief_here(root: &Path) -> Brief {
    brief_at(root, root.join(BRIEF_PATH)).expect("a document that is a brief")
}

fn board() -> Board<'static> {
    Board {
        scope: SCOPE,
        team: TEAM,
        label: LABEL,
    }
}

fn records_in(root: &Path) -> Filed {
    Filed::load(root).expect("a record file that reads")
}

#[test]
fn a_push_that_files_asks_for_the_team_the_status_and_the_label_before_it_creates_anything() {
    let repo = a_repository();
    let home = a_home(repo.path());
    let linear = a_whole_push(repo.path());
    let handed = linear.clone();

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |key| {
        // The value out of the key store reached the constructor, which is the
        // one line in the module that reads it.
        assert_eq!(key, NOT_A_KEY);
        handed
    });

    outcome.expect("a repository, a board and a brief are a push");
    let asked = linear.documents();
    assert_eq!(asked.len(), 4, "one request per operation: {asked:?}");
    assert!(asked[0].contains("teams("), "{asked:?}");
    assert!(asked[1].contains("projectStatuses"), "{asked:?}");
    assert!(asked[2].contains("projectLabels"), "{asked:?}");
    assert!(asked[3].contains("projectCreate("), "{asked:?}");
    // And the record is the step after all four, rather than a file written
    // beside a project that might never have existed.
    assert_eq!(
        linear.recorded_when_asked(),
        [false, false, false, false],
        "a record was on disk before the project that it names"
    );
    assert!(filed_path(repo.path()).exists());
    assert!(printed.contains(URL), "{printed}");
}

#[test]
fn the_create_carries_the_brief_the_team_and_the_status_that_were_resolved_for_it() {
    let repo = a_repository();
    let home = a_home(repo.path());
    let linear = a_whole_push(repo.path());
    let handed = linear.clone();

    push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |_| {
        handed
    })
    .0
    .expect("a push");

    let input = linear.last_input();
    assert_eq!(input["name"], json!(TITLE), "{input}");
    assert_eq!(
        input["content"],
        json!(brief_here(repo.path()).content()),
        "the body is the file's own bytes: {input}"
    );
    assert_eq!(input["teamIds"], json!(["team-1"]), "{input}");
    assert_eq!(input["statusId"], json!("status-backlog"), "{input}");
}

#[test]
fn a_push_that_files_says_the_team_the_label_the_name_and_the_url_and_records_them() {
    let repo = a_repository();
    let home = a_home(repo.path());
    let handed = a_whole_push(repo.path());

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |_| {
        handed
    });

    outcome.expect("a push");
    for said in [TEAM, LABEL, TITLE, URL] {
        assert!(printed.contains(said), "{said} is not in: {printed}");
    }

    let record = records_in(repo.path());
    let record = record
        .record(BRIEF_PATH)
        .expect("the brief that was filed is recorded under its repository path");
    // The id and the URL could only have come from the create, which is the
    // other half of the record being appended after it rather than before.
    assert_eq!(record.project_id(), PROJECT_ID);
    assert_eq!(record.url(), URL);
    assert_eq!(record.scope(), SCOPE);
    assert_eq!(record.team(), TEAM);
}

#[test]
fn a_dry_run_opens_no_socket_writes_no_record_and_says_what_would_go() {
    let repo = a_repository();
    let home = a_home(repo.path());

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, true, no_socket);

    outcome.expect("a dry run answers");
    assert!(
        !filed_path(repo.path()).exists(),
        "a dry run recorded a project nobody created"
    );
    // The board, the key by name, the project name and how much content would
    // go — everything somebody would check before typing the command for real.
    for said in [SCOPE, TEAM, KEY_NAME, TITLE, "bytes"] {
        assert!(printed.contains(said), "{said} is not in: {printed}");
    }
    let size = brief_here(repo.path()).content().len();
    assert!(printed.contains(&format!("{size} bytes")), "{printed}");
    assert!(
        printed.contains("nothing was sent"),
        "a dry run has to say it was one: {printed}"
    );
    assert_eq!(printed.lines().count(), 1, "one line: {printed:?}");
}

#[test]
fn a_brief_this_repository_has_already_filed_is_refused_with_the_url_it_got() {
    let repo = a_repository();
    let home = a_home(repo.path());
    Filed::with_records([FiledRecord::new(
        repo.path(),
        repo.path().join(BRIEF_PATH),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-20T07:32:00Z",
    )
    .expect("a path inside the repository")])
    .save(repo.path())
    .expect("a record file that saves");

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, no_socket);

    assert!(printed.is_empty(), "{printed}");
    assert_eq!(status_for(&outcome), 1);
    let error = outcome.expect_err("a brief is filed once");
    assert!(
        matches!(&error, Error::AlreadyFiled { path, url } if path == BRIEF_PATH && url == URL),
        "{error:?}"
    );
    // The address of the project it already made is the point of this refusal.
    assert!(said(&error).contains(URL));
}

#[test]
fn a_machine_with_no_board_to_file_to_is_refused_before_anything_is_read() {
    // The three ways there is no candidate, which are fixed in three different
    // places: the machine holds nothing, it holds a sigil this repository has
    // never recorded, and it holds one a pact carries with no `[[scope]]`
    // record behind it.
    let holds_nothing = a_repository();
    let nothing = a_dir();

    let unmatched = a_repository();
    let elsewhere = a_dir();
    holding(elsewhere.path(), unmatched.path(), &["billing"]);

    let unrecorded = a_dir();
    saving(unrecorded.path(), &a_manifest([]));
    write_brief(unrecorded.path(), BRIEF);
    let no_record = a_dir();
    holding(no_record.path(), unrecorded.path(), &[SCOPE]);

    for (repo, home, expected) in [
        (&holds_nothing, &nothing, "warlock config"),
        (&unmatched, &elsewhere, "billing"),
        (&unrecorded, &no_record, "[[scope]]"),
    ] {
        let error = refusal(repo.path(), home.path(), BRIEF_PATH, None);

        assert!(matches!(error, Error::Filing { .. }), "{error:?}");
        assert!(said(&error).contains(expected), "{}", said(&error));
    }
}

#[test]
fn a_machine_that_can_file_to_several_boards_names_them_all_and_asks_for_one() {
    let repo = a_repository();
    saving(
        repo.path(),
        &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
    );
    let home = a_dir();
    holding(home.path(), repo.path(), &[SCOPE, "web"]);
    bound(home.path(), repo.path(), KEY_NAME);
    keeping(home.path(), KEY_NAME);

    let error = refusal(repo.path(), home.path(), BRIEF_PATH, None);

    let message = said(&error);
    assert!(matches!(error, Error::Filing { .. }), "{error:?}");
    assert!(message.contains(SCOPE), "{message}");
    assert!(message.contains("web"), "{message}");
    assert!(message.contains("--scope"), "{message}");
}

#[test]
fn a_scope_that_is_a_candidate_is_honoured_and_one_that_is_not_names_the_candidates() {
    let repo = a_repository();
    saving(
        repo.path(),
        &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
    );
    let home = a_dir();
    holding(home.path(), repo.path(), &[SCOPE, "web"]);
    bound(home.path(), repo.path(), KEY_NAME);
    keeping(home.path(), KEY_NAME);

    // Honoured: the named candidate's team is the board the dry run reports,
    // and the other one is nowhere in it.
    let (outcome, printed) = push_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        Some("web"),
        true,
        no_socket,
    );
    outcome.expect("a named candidate is a board");
    assert!(printed.contains("WEB"), "{printed}");
    assert!(!printed.contains(TEAM), "{printed}");

    // And a name that is not one of them is refused with both of them named.
    let error = refusal(repo.path(), home.path(), BRIEF_PATH, Some("billing"));

    let message = said(&error);
    assert!(matches!(error, Error::Filing { .. }), "{error:?}");
    assert!(message.contains("billing"), "{message}");
    assert!(
        message.contains(SCOPE) && message.contains("web"),
        "{message}"
    );
}

#[test]
fn an_unbound_checkout_and_a_dangling_name_are_route_s_two_sentences_reached_through_it() {
    // Both are `route.rs`'s words rather than words of this module's, which is
    // what these assertions are: the sentence is asked of the engine's own
    // error rather than retyped here, so a re-wording in `push.rs` would fail
    // them.
    let repo = a_repository();

    let unbound = a_dir();
    holding(unbound.path(), repo.path(), &[SCOPE]);
    keeping(unbound.path(), KEY_NAME);

    let error = refusal(repo.path(), unbound.path(), BRIEF_PATH, None);
    assert_eq!(
        said(&error),
        route::Error::Unbound {
            path: sigils_path(unbound.path(), repo.path()),
        }
        .to_string()
    );

    let dangling = a_dir();
    holding(dangling.path(), repo.path(), &[SCOPE]);
    bound(dangling.path(), repo.path(), KEY_NAME);
    keeping(dangling.path(), "personal");

    let error = refusal(repo.path(), dangling.path(), BRIEF_PATH, None);
    assert_eq!(
        said(&error),
        route::Error::Dangling {
            key: KEY_NAME.to_owned(),
            path: keys_path(dangling.path()),
        }
        .to_string()
    );
}

#[test]
fn a_file_that_is_not_a_brief_is_refused_before_the_socket_is_opened() {
    // Absent, untitled, and missing a section of the repository's shape: three
    // documents, one refusal each, and no board told about any of them.
    let absent = a_repository();
    fs::remove_file(absent.path().join(BRIEF_PATH)).expect("the brief this test removes");

    let untitled = a_repository();
    write_brief(untitled.path(), &BRIEF.replacen("# Push", "#Push", 1));

    let sectionless = a_repository();
    write_brief(
        sectionless.path(),
        "# Push a brief to the board\n\nNo sections.\n",
    );

    for (repo, expected) in [
        (&absent, "nothing to push"),
        (&untitled, "title line"),
        (&sectionless, "## Outcome"),
    ] {
        let home = a_home(repo.path());
        let error = refusal(repo.path(), home.path(), BRIEF_PATH, None);

        assert!(matches!(error, Error::Brief { .. }), "{error:?}");
        assert!(said(&error).contains(expected), "{}", said(&error));
    }
}

#[test]
fn a_team_key_linear_does_not_know_names_the_value_and_the_file_it_is_written_in() {
    let repo = a_repository();
    let home = a_home(repo.path());
    let linear = Posting::answering(repo.path(), [Ok(no_team())]);
    let handed = linear.clone();

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |_| {
        handed
    });

    assert_eq!(status_for(&outcome), 1);
    let error = outcome.expect_err("a team the workspace does not have is a refusal");
    assert!(
        matches!(&error, Error::UnknownTeam { team, .. } if team == TEAM),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains(TEAM), "{message}");
    assert!(
        message.contains(&manifest_path(repo.path()).display().to_string()),
        "the file the key is written in is not named: {message}"
    );
    // Nothing was created and nothing was printed, because the refusal is the
    // answer to the first request.
    assert_eq!(linear.documents().len(), 1);
    assert!(printed.is_empty(), "{printed}");
    assert!(!filed_path(repo.path()).exists());
}

#[test]
fn a_label_that_will_not_resolve_stops_before_the_project_rather_than_after_it() {
    // The ticket's label bullet turned the other way up, and deliberately: the
    // label is `create_project`'s own first request (see `linear.rs`), so there
    // is no create that landed for a failing label to be missing from. What the
    // push can honestly report is that nothing was created, nothing was
    // recorded and there is no URL — which is what this asserts.
    let repo = a_repository();
    let home = a_home(repo.path());
    let linear = Posting::answering(
        repo.path(),
        [
            Ok(team_found()),
            Ok(backlog()),
            Err(LinearError::Refused {
                message: "Entity not found".to_owned(),
            }),
        ],
    );
    let handed = linear.clone();

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |_| {
        handed
    });

    assert_eq!(
        status_for(&outcome),
        1,
        "a push that filed nothing is not a 0"
    );
    let error = outcome.expect_err("a label that will not resolve is a refusal");
    assert!(matches!(error, Error::Linear { .. }), "{error:?}");

    let asked = linear.documents();
    assert_eq!(asked.len(), 3, "a project was created anyway: {asked:?}");
    assert!(asked[2].contains("projectLabels"), "{asked:?}");
    assert!(
        printed.is_empty(),
        "a URL was printed for a project that does not exist: {printed}"
    );
    assert!(
        !filed_path(repo.path()).exists(),
        "a project that was never created was recorded"
    );
}

#[test]
fn a_create_that_landed_with_a_record_that_will_not_save_still_prints_the_url() {
    // Driven at `sent` rather than at the whole command, because what has to
    // fail is the save and nothing above it: the root here is a path under a
    // file, so the record's directory cannot be made and every read before it
    // has already happened.
    let repo = a_repository();
    let wall = repo.path().join("docs").join("brief.md");
    let root = wall.join("inside");
    let brief = brief_here(repo.path());
    let linear = Posting::answering(
        &root,
        [
            Ok(team_found()),
            Ok(backlog()),
            Ok(label_found()),
            Ok(project_created()),
        ],
    );

    let mut out = Vec::new();
    let outcome = sent(
        &linear,
        &root,
        board(),
        &brief,
        &root.join(BRIEF_PATH),
        Filed::new(),
        &mut out,
    );
    let printed = String::from_utf8(out).expect("warlock writes its own text");

    assert_eq!(
        status_for(&outcome),
        1,
        "a project with no record is not a 0"
    );
    let error = outcome.expect_err("a record that cannot be saved is a failure");
    assert!(
        matches!(&error, Error::Unfiled { url, .. } if url == URL),
        "{error:?}"
    );
    // The URL twice over: printed as the project was created, and again in the
    // refusal, because this line is the last place that address appears.
    assert!(printed.contains(URL), "{printed}");
    let message = said(&error);
    assert!(message.contains(URL), "{message}");
    assert!(message.contains("could not record"), "{message}");
}

#[test]
fn no_line_and_no_error_on_this_path_carries_a_key_value() {
    // The whole command, over a machine that really does hold a key: a dry run,
    // a push that filed, and the refusals that are reached with the key store
    // already read. The name is printed and the value it stands for never is.
    let repo = a_repository();
    let home = a_home(repo.path());
    let handed = a_whole_push(repo.path());

    let (filed, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, |_| {
        handed
    });
    filed.expect("a push");

    let dry = a_repository();
    let dry_home = a_home(dry.path());
    let (outcome, dried) = push_to(
        dry.path(),
        dry_home.path(),
        BRIEF_PATH,
        None,
        true,
        no_socket,
    );
    outcome.expect("a dry run");

    // The same push again, which is the already-filed refusal, and a `--scope`
    // nothing answers to: both are raised with a target in hand.
    let again = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, no_socket).0;
    let unknown = push_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        Some("billing"),
        false,
        no_socket,
    )
    .0;

    for line in [printed, dried] {
        assert!(!line.contains(NOT_A_KEY), "a key value was printed: {line}");
    }
    for outcome in [again, unknown] {
        let error = outcome.expect_err("a refusal");
        let (shown, printed) = (error.to_string(), format!("{error:?}"));
        assert!(
            !shown.contains(NOT_A_KEY),
            "the key reached `Display`: {shown}"
        );
        assert!(
            !printed.contains(NOT_A_KEY),
            "the key reached `Debug`: {printed}"
        );
    }
    // And the file the push wrote, which is committed.
    let recorded = fs::read_to_string(filed_path(repo.path())).expect("a record file");
    assert!(!recorded.contains(NOT_A_KEY), "{recorded}");
    assert!(!recorded.contains(KEY_NAME), "{recorded}");
}

#[test]
fn a_record_file_that_will_not_read_is_a_failure_rather_than_an_empty_one() {
    // Pushing over records warlock could not read is how a brief that already
    // has a project gets a second one. A missing file is the empty answer and
    // is covered by every other test here.
    let repo = a_repository();
    let home = a_home(repo.path());
    fs::write(filed_path(repo.path()), "version = 1\nnot toml {{{").expect("a broken record file");

    let (outcome, printed) = push_to(repo.path(), home.path(), BRIEF_PATH, None, false, no_socket);

    assert_eq!(status_for(&outcome), 1);
    let error = outcome.expect_err("a record file that will not parse is a failure");
    assert!(matches!(error, Error::Filed { .. }), "{error:?}");
    assert!(printed.is_empty(), "{printed}");
}
