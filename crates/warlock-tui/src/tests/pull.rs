use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;
use warlock_engine::{Filed, FiledRecord, filed_path};
use warlock_tui::{FetchedProject, LinearError, Posts};

use super::planned;
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const BRIEF_PATH: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/pull-a-brief-1a2b3c";

const NAME: &str = "Cut a planned project into tickets";

const CONTENT: &str = "# Cut a planned project into tickets\n\n## Scope\n";

// A Linear that answers from memory, in the order it was given. `RefCell`
// rather than the `Mutex` the client's own stand-in uses, because nothing here
// crosses a thread.
#[derive(Debug)]
struct Posting {
    answers: RefCell<VecDeque<Result<Value, LinearError>>>,
    asked: RefCell<Vec<(String, Value)>>,
}

impl Posting {
    fn answering(answers: impl IntoIterator<Item = Result<Value, LinearError>>) -> Self {
        Self {
            answers: RefCell::new(answers.into_iter().collect()),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn documents(&self) -> Vec<String> {
        self.asked
            .borrow()
            .iter()
            .map(|(document, _)| document.clone())
            .collect()
    }

    fn variables(&self) -> Vec<Value> {
        self.asked
            .borrow()
            .iter()
            .map(|(_, variables)| variables.clone())
            .collect()
    }
}

impl Posts for Posting {
    fn post(&self, document: &str, variables: Value) -> Result<Value, LinearError> {
        self.asked
            .borrow_mut()
            .push((document.to_owned(), variables));

        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("the stand-in was asked more times than it was answered"))
    }
}

// The seam a refusal may never reach. A brief nothing records is refused with
// the file on this machine and nothing else, so being posted to at all is the
// failure the test is about — which is why it is a panic and not a flag.
#[derive(Debug, Clone, Copy)]
struct Unreachable;

impl Posts for Unreachable {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        panic!("a request was sent: {document}");
    }
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// A repository that has filed one brief, which is the finished state a pull
// starts from.
fn a_repository() -> TempDir {
    let repo = a_dir();
    recording(repo.path(), BRIEF_PATH);
    repo
}

fn recording(root: &Path, path: &str) {
    Filed::with_records([FiledRecord::new(
        root,
        root.join(path),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-21T09:14:00Z",
    )
    .expect("a path inside the repository")])
    .save(root)
    .expect("a record file that saves");
}

fn a_project(status: Option<&str>) -> Value {
    json!({
        "project": {
            "name": NAME,
            "content": CONTENT,
            "url": URL,
            "status": status.map(|name| json!({ "name": name })),
        },
    })
}

// The whole operation, less the environment: the repository root is this test's
// temporary directory and the seam is whatever was handed in.
fn pull_from(repo: &Path, linear: &impl Posts, path: &str) -> Result<FetchedProject, Error> {
    planned(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        linear,
        Path::new(path),
    )
}

// What every refusal here promises, checked in one place: the ordinary exit
// status rather than the boundary's, and one line to print.
fn refusal(outcome: Result<FetchedProject, Error>) -> Error {
    let outcome = outcome.map(drop);

    assert_eq!(status_for(&outcome), 1, "a pull refusal is the ordinary 1");
    assert_ne!(
        status_for(&outcome),
        3,
        "a pull refusal took the boundary's status"
    );

    let error = outcome.expect_err("a refusal");
    let message = error.to_string();
    assert!(!message.contains('\n'), "`main` prints one line: {message}");
    error
}

fn said(error: &Error) -> String {
    error.to_string()
}

#[test]
fn a_recorded_brief_is_fetched_by_the_id_that_record_holds() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    let project = pull_from(repo.path(), &linear, BRIEF_PATH).expect("a planned project is read");

    assert_eq!(project.name(), NAME);
    assert_eq!(project.content(), CONTENT);
    assert_eq!(project.url(), URL);
    assert_eq!(project.status(), Some("Planned"));
    // The id out of `.warlock/filed.toml` and no other selector, in one request.
    assert_eq!(linear.variables(), [json!({ "id": PROJECT_ID })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_brief_named_from_a_subdirectory_resolves_to_the_same_record() {
    // The spelling a push records is `Standing::spelled`'s, so a path handed in
    // with the root's own separators and a `./` in front of it is the same
    // record rather than a second one.
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    pull_from(repo.path(), &linear, "./docs/brief.md").expect("the same brief, spelled twice");

    assert_eq!(linear.variables(), [json!({ "id": PROJECT_ID })]);
}

#[test]
fn a_brief_no_record_names_is_refused_with_the_path_and_the_command_that_files_it() {
    let repo = a_repository();

    let error = refusal(pull_from(repo.path(), &Unreachable, "docs/other.md"));

    assert!(
        matches!(&error, Error::NoRecord { path } if path == "docs/other.md"),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("docs/other.md"), "{message}");
    assert!(message.contains("warlock push"), "{message}");
}

#[test]
fn a_repository_that_has_filed_nothing_at_all_is_that_same_refusal() {
    // No `.warlock/filed.toml` on disk, which `records` reads as an empty one:
    // a repository that has never pushed records no project, and that is an
    // answer rather than a failure to reach one.
    let repo = a_dir();

    let error = refusal(pull_from(repo.path(), &Unreachable, BRIEF_PATH));

    assert!(matches!(error, Error::NoRecord { .. }), "{error:?}");
    assert!(!filed_path(repo.path()).exists());
}

#[test]
fn a_project_id_the_api_does_not_know_names_the_id_and_the_file_it_is_written_in() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(json!({ "project": null }))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert!(
        matches!(&error, Error::UnknownProject { id, .. } if id == PROJECT_ID),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains(PROJECT_ID), "{message}");
    assert!(
        message.contains(".warlock/filed.toml"),
        "the file the id is written in is not named: {message}"
    );
}

#[test]
fn a_project_that_is_not_planned_names_the_status_it_found_and_the_one_it_wanted() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Backlog")))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert!(
        matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some("Backlog")),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("Backlog"), "{message}");
    assert!(message.contains("Planned"), "{message}");
}

#[test]
fn a_project_with_no_status_at_all_is_refused_and_says_it_has_none() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(None))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert!(
        matches!(&error, Error::NotPlanned { status: None, .. }),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("no status"), "{message}");
    assert!(message.contains("Planned"), "{message}");
}

#[test]
fn the_status_is_trimmed_and_case_folded_and_planned_is_the_only_spelling_accepted() {
    let repo = a_repository();

    for accepted in ["Planned", "planned", " Planned ", "PLANNED", "\tplanned\n"] {
        let linear = Posting::answering([Ok(a_project(Some(accepted)))]);

        pull_from(repo.path(), &linear, BRIEF_PATH)
            .unwrap_or_else(|error| panic!("`{accepted}` is `Planned`: {error:?}"));
    }

    for refused in [
        "Backlog",
        "In Progress",
        "Plan",
        "Planned later",
        "",
        "unplanned",
    ] {
        let linear = Posting::answering([Ok(a_project(Some(refused)))]);

        let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

        assert!(
            matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some(refused)),
            "`{refused}` was read as `Planned`: {error:?}"
        );
    }
}

#[test]
fn a_status_that_is_not_planned_stops_the_sequence_where_it_stands() {
    // One answer in the stand-in and one request made: anything read or sent
    // after the status said `Backlog` would be a second `post`, which this
    // stand-in panics on.
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Backlog")))]);

    refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert_eq!(linear.documents().len(), 1, "{:?}", linear.documents());
}

#[test]
fn nothing_on_this_path_writes_to_the_board_or_to_the_record_file() {
    let repo = a_repository();
    let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    pull_from(repo.path(), &linear, BRIEF_PATH).expect("a planned project is read");

    for asked in linear.documents() {
        assert!(
            !asked.contains("mutation"),
            "a mutation was issued: {asked}"
        );
        assert!(!asked.contains("Create"), "a mutation was issued: {asked}");
        assert!(!asked.contains("Update"), "a mutation was issued: {asked}");
    }
    assert_eq!(
        fs::read_to_string(filed_path(repo.path())).expect("a record file"),
        before,
        "a read rewrote the record file"
    );
}

#[test]
fn a_record_file_that_will_not_read_is_a_failure_rather_than_an_empty_one() {
    // The reading half of `push.rs`'s rule: an unreadable record file is a
    // repository whose filings are unknown, not one with none, and answering
    // "nothing is filed" here would send somebody back to `warlock push` for a
    // brief that already has a project.
    let repo = a_repository();
    fs::write(filed_path(repo.path()), "version = 2\nnot toml {{{").expect("a broken record file");

    let error = refusal(pull_from(repo.path(), &Unreachable, BRIEF_PATH));

    assert!(matches!(error, Error::Filed { .. }), "{error:?}");
}
