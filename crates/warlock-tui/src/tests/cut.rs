use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;
use warlock_engine::drafting::Draft;
use warlock_engine::{CutRecord, Filed, FiledRecord, filed_path};
use warlock_tui::{LinearError, Posts};

use super::{Cut, Filing, Slice, cut};
use crate::error::Error;
use crate::status_for;

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const BRIEF_PATH: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/cut-a-project-1a2b3c";

const TITLE: &str = "Read the file";

// A Linear that answers from memory, in the order it was given, keeping what it
// was asked so the order and the exact `input` of every create are assertions
// rather than readings. `RefCell` rather than the `Mutex` the client's own
// stand-in uses, because nothing here crosses a thread.
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

    fn creates(&self) -> Vec<Value> {
        self.asked
            .borrow()
            .iter()
            .filter(|(document, _)| document.contains("issueCreate"))
            .map(|(_, variables)| variables["input"].clone())
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

// The seam a slice the record already names may never reach: it is answered off
// this machine's own file, so being posted to at all is the failure the test is
// about — which is why it is a panic and not a flag.
#[derive(Debug, Clone, Copy)]
struct Unreachable;

impl Posts for Unreachable {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        panic!("a request was sent: {document}");
    }
}

// A `Posting` with one document barred. The refusals below all happen while the
// slice is still nothing on the board, and the thing that would make them
// worthless is an issue existing anyway — so the create is a panic here, and
// the questions before it are answered normally.
#[derive(Debug)]
struct NoCreate(Posting);

impl Posts for NoCreate {
    fn post(&self, document: &str, variables: Value) -> Result<Value, LinearError> {
        assert!(
            !document.contains("issueCreate"),
            "an issue was created: {document}"
        );

        self.0.post(document, variables)
    }
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// A repository that has filed one brief and cut nothing out of it, which is the
// state a first cut starts from.
fn a_repository() -> TempDir {
    let repo = a_dir();
    saving(repo.path(), &Filed::with_records([a_record(repo.path())]));
    repo
}

// The same repository, with this slice already cut into the issues named.
fn cut_already(issues: &[&str]) -> TempDir {
    let repo = a_dir();
    let mut record = a_record(repo.path());
    record.push_cut(CutRecord::new(
        TITLE,
        issues.iter().copied(),
        "2026-09-21T09:00:00Z",
    ));
    saving(repo.path(), &Filed::with_records([record]));
    repo
}

fn a_record(root: &Path) -> FiledRecord {
    FiledRecord::new(
        root,
        root.join(BRIEF_PATH),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-21T09:14:00Z",
    )
    .expect("a path inside the repository")
}

fn saving(root: &Path, filed: &Filed) {
    filed.save(root).expect("a record file that saves");
}

fn a_draft(title: &str, body: &str) -> Draft {
    Draft {
        title: title.to_owned(),
        body: body.to_owned(),
        blocked_by: Vec::new(),
        blocks: Vec::new(),
    }
}

fn filing() -> Filing<'static> {
    Filing {
        brief: BRIEF_PATH,
        project: PROJECT_ID,
        team: TEAM,
        label: LABEL,
    }
}

fn team_found() -> Value {
    json!({ "teams": { "nodes": [{ "id": "team-1" }] } })
}

fn no_team() -> Value {
    json!({ "teams": { "nodes": [] } })
}

fn states(names: &[(&str, &str)]) -> Value {
    let nodes: Vec<Value> = names
        .iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect();

    json!({ "workflowStates": { "nodes": nodes } })
}

fn backlog() -> Value {
    states(&[("state-todo", "Todo"), ("state-backlog", "Backlog")])
}

fn label_found() -> Value {
    json!({ "issueLabels": { "nodes": [{ "id": "label-held" }] } })
}

fn issue(number: u32) -> Value {
    json!({
        "issueCreate": {
            "issue": {
                "id": format!("issue-{number}"),
                "identifier": format!("WAR-{number}"),
                "url": format!("https://linear.app/acme/issue/WAR-{number}"),
            },
        },
    })
}

// The answers a cut of two drafts gets, in the order it asks for them.
fn a_whole_cut() -> Posting {
    Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
    ])
}

fn two_drafts() -> Vec<Draft> {
    vec![
        a_draft("Read the file off disk", "The bytes, whole."),
        a_draft("Fold its title", "Lowercased, collapsed."),
    ]
}

// The whole operation, less the environment: the repository root is this test's
// temporary directory and the seam is whatever was handed in.
fn cut_into(
    repo: &Path,
    linear: &impl Posts,
    title: &str,
    drafts: &[Draft],
) -> (Result<Cut, Error>, String) {
    let mut out = Vec::new();
    let outcome = cut(linear, repo, filing(), Slice { title, drafts }, &mut out);

    (
        outcome,
        String::from_utf8(out).expect("warlock writes its own text"),
    )
}

// What every refusal here promises, checked in one place: the ordinary exit
// status rather than the boundary's, and one line to print.
fn refusal(outcome: Result<Cut, Error>) -> Error {
    let outcome = outcome.map(drop);

    assert_eq!(status_for(&outcome), 1, "a cut refusal is the ordinary 1");
    assert_ne!(
        status_for(&outcome),
        3,
        "a cut refusal took the boundary's status"
    );

    let error = outcome.expect_err("a refusal");
    let message = error.to_string();
    assert!(!message.contains('\n'), "`main` prints one line: {message}");
    error
}

fn said(error: &Error) -> String {
    error.to_string()
}

fn filed_now(root: &Path) -> Filed {
    Filed::load(root).expect("a record file that loads")
}

fn cuts_of(root: &Path) -> Vec<CutRecord> {
    filed_now(root)
        .record(BRIEF_PATH)
        .expect("the brief is filed")
        .cuts()
        .to_vec()
}

fn filed(outcome: Result<Cut, Error>) -> Vec<String> {
    match outcome.expect("a slice that files") {
        Cut::Filed(issues) => issues
            .iter()
            .map(|issue| issue.identifier().to_owned())
            .collect(),
        Cut::Already(issues) => panic!("nothing was sent: {issues:?}"),
    }
}

#[test]
fn a_slice_becomes_one_issue_per_draft_and_a_cut_record() {
    let repo = a_repository();
    let linear = a_whole_cut();

    let (outcome, printed) = cut_into(repo.path(), &linear, TITLE, &two_drafts());

    assert_eq!(filed(outcome), ["WAR-125", "WAR-126"]);
    assert_eq!(
        linear.documents().len(),
        5,
        "one request per operation and no retry"
    );
    assert_eq!(linear.creates().len(), 2, "one `issueCreate` per draft");
    assert!(printed.contains(TITLE), "{printed}");
    assert!(printed.contains("`WAR-125`, `WAR-126`"), "{printed}");

    let cuts = cuts_of(repo.path());
    assert_eq!(cuts.len(), 1, "{cuts:?}");
    assert_eq!(cuts[0].title(), TITLE);
    assert_eq!(cuts[0].key(), "read the file");
    assert_eq!(cuts[0].issues(), ["WAR-125", "WAR-126"]);
    assert!(
        !cuts[0].cut_at().is_empty(),
        "a cut is recorded with a time"
    );
}

#[test]
fn each_issue_carries_the_scope_records_team_the_project_and_the_backlog_state() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");

    assert_eq!(
        linear.creates(),
        [
            json!({
                "title": "Read the file off disk",
                "description": "The bytes, whole.",
                "teamId": "team-1",
                "projectId": PROJECT_ID,
                "labelIds": ["label-held"],
                "stateId": "state-backlog",
            }),
            json!({
                "title": "Fold its title",
                "description": "Lowercased, collapsed.",
                "teamId": "team-1",
                "projectId": PROJECT_ID,
                "labelIds": ["label-held"],
                "stateId": "state-backlog",
            }),
        ],
    );
}

#[test]
fn the_label_is_resolved_as_an_issue_label_and_not_as_a_project_label() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");

    let documents = linear.documents().join("\n");
    assert!(documents.contains("issueLabels"), "{documents}");
    assert!(
        !documents.contains("projectLabels"),
        "a project label's id is not usable by an issue: {documents}"
    );
}

#[test]
fn a_label_the_team_does_not_have_yet_is_created_on_it() {
    let repo = a_repository();
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(json!({ "issueLabels": { "nodes": [] } })),
        Ok(json!({ "issueLabelCreate": { "issueLabel": { "id": "label-made" } } })),
        Ok(issue(125)),
    ]);

    let outcome = cut_into(repo.path(), &linear, TITLE, &two_drafts()[..1]).0;

    assert_eq!(filed(outcome), ["WAR-125"]);
    assert_eq!(linear.creates()[0]["labelIds"], json!(["label-made"]));
}

#[test]
fn a_slice_the_record_already_names_sends_nothing() {
    let repo = cut_already(&["WAR-125", "WAR-126"]);
    let before = fs::read(filed_path(repo.path())).expect("the record that was saved");

    let (outcome, printed) = cut_into(repo.path(), &Unreachable, TITLE, &two_drafts());

    let Cut::Already(issues) = outcome.expect("an already cut slice is an answer") else {
        panic!("a slice with a cut record was filed again");
    };
    assert_eq!(issues, ["WAR-125", "WAR-126"]);
    assert!(printed.contains("already cut"), "{printed}");
    assert!(printed.contains("`WAR-125`, `WAR-126`"), "{printed}");
    assert_eq!(
        fs::read(filed_path(repo.path())).expect("the record again"),
        before,
        "a slice that sent nothing rewrote its record"
    );
}

#[test]
fn an_already_cut_slice_is_matched_on_the_folded_key() {
    let repo = cut_already(&["WAR-125"]);

    let (outcome, printed) = cut_into(
        repo.path(),
        &Unreachable,
        "  READ   the File  ",
        &two_drafts(),
    );

    let Cut::Already(issues) = outcome.expect("the same slice, spelled twice") else {
        panic!("a retitled spelling of a cut slice was filed again");
    };
    assert_eq!(issues, ["WAR-125"]);
    assert!(printed.contains("already cut"), "{printed}");
}

#[test]
fn a_team_with_no_backlog_state_is_refused_naming_the_team() {
    let repo = a_repository();
    let linear = NoCreate(Posting::answering([
        Ok(team_found()),
        Ok(states(&[("state-todo", "Todo"), ("state-done", "Done")])),
    ]));

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(
        matches!(&error, Error::NoBacklog { team } if team == TEAM),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains(TEAM), "{message}");
    assert!(message.contains("Backlog"), "{message}");
    assert!(cuts_of(repo.path()).is_empty(), "a refusal recorded a cut");
}

#[test]
fn the_backlog_state_is_asked_for_before_the_label_and_before_any_create() {
    let repo = a_repository();
    let linear = NoCreate(Posting::answering([
        Ok(team_found()),
        Ok(states(&[("state-todo", "Todo")])),
    ]));

    refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    let documents = linear.0.documents();
    assert_eq!(documents.len(), 2, "{documents:?}");
    assert!(documents[1].contains("workflowStates"), "{documents:?}");
}

#[test]
fn an_unknown_team_is_refused_before_anything_is_created() {
    let repo = a_repository();
    let linear = NoCreate(Posting::answering([Ok(no_team())]));

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(
        matches!(&error, Error::UnknownTeam { team, .. } if team == TEAM),
        "{error:?}"
    );
    assert!(cuts_of(repo.path()).is_empty(), "a refusal recorded a cut");
}

#[test]
fn a_brief_no_record_names_is_refused_with_nothing_sent() {
    let repo = a_dir();

    let error = refusal(cut_into(repo.path(), &Unreachable, TITLE, &two_drafts()).0);

    assert!(
        matches!(&error, Error::NoRecord { path } if path == BRIEF_PATH),
        "{error:?}"
    );
}

#[test]
fn a_create_that_fails_partway_records_nothing() {
    let repo = a_repository();
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Err(LinearError::Refused {
            message: "Entity not found".to_owned(),
        }),
    ]);

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(matches!(&error, Error::Linear { .. }), "{error:?}");
    assert!(
        said(&error).contains("Entity not found"),
        "{}",
        said(&error)
    );
    assert!(
        cuts_of(repo.path()).is_empty(),
        "a half filed slice was recorded as cut"
    );
}

#[cfg(unix)]
#[test]
fn the_identifiers_come_back_when_the_record_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = a_repository();
    let linear = a_whole_cut();
    let records = repo.path().join(".warlock");
    fs::set_permissions(&records, fs::Permissions::from_mode(0o555))
        .expect("chmods the record directory read-only");

    let (outcome, printed) = cut_into(repo.path(), &linear, TITLE, &two_drafts());

    // Back to writable before anything can fail, so the temporary directory can
    // still be removed.
    fs::set_permissions(&records, fs::Permissions::from_mode(0o755)).expect("chmods it back");

    let error = refusal(outcome);
    assert!(
        matches!(&error, Error::Uncut { issues, .. } if issues == &["WAR-125", "WAR-126"]),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("`WAR-125`, `WAR-126`"), "{message}");
    assert!(
        printed.contains("`WAR-125`, `WAR-126`"),
        "the issues exist and were not named: {printed}"
    );
    assert!(
        cuts_of(repo.path()).is_empty(),
        "the record the save failed on was written anyway"
    );
}
