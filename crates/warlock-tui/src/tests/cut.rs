use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;
use warlock_engine::drafting::Draft;
use warlock_engine::{CutRecord, Filed, FiledRecord, filed_path};
use warlock_tui::{LinearError, LinearIssue, Posts};

use super::{Cut, Filing, Slice, announce, cut};
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
        self.inputs("issueCreate")
    }

    fn relations(&self) -> Vec<Value> {
        self.inputs("issueRelationCreate")
    }

    fn comments(&self) -> Vec<Value> {
        self.inputs("commentCreate")
    }

    fn inputs(&self, mutation: &str) -> Vec<Value> {
        self.asked
            .borrow()
            .iter()
            .filter(|(document, _)| document.contains(mutation))
            .map(|(_, variables)| variables["input"].clone())
            .collect()
    }

    // Where in the whole conversation each of these documents was asked, so an
    // ordering promise is an assertion about positions rather than about the
    // order this test handed its answers over in.
    fn positions_of(&self, mutation: &str) -> Vec<usize> {
        self.asked
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (document, _))| document.contains(mutation))
            .map(|(at, _)| at)
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

// Three drafts in a chain, with the middle pair saying the same edge from both
// ends: the first `blocks` the second and the second is `blocked_by` the first.
fn ordered_drafts() -> Vec<Draft> {
    let mut drafts = two_drafts();
    drafts.push(a_draft("Write the record", "Beside the brief."));
    drafts[0].blocks = vec![1];
    drafts[1].blocked_by = vec![0];
    drafts[2].blocked_by = vec![1];
    drafts
}

fn relation(number: u32) -> Value {
    json!({ "issueRelationCreate": { "issueRelation": { "id": format!("relation-{number}") } } })
}

fn edge(blocker: &str, waiting: &str) -> Value {
    json!({ "issueId": blocker, "relatedIssueId": waiting, "type": "blocks" })
}

fn commented() -> Value {
    json!({ "commentCreate": { "comment": { "id": "comment-1" } } })
}

fn refused(message: &str) -> Result<Value, LinearError> {
    Err(LinearError::Refused {
        message: message.to_owned(),
    })
}

// The whole operation, less the environment: the repository root is this test's
// temporary directory and the seam is whatever was handed in.
fn cut_into(
    repo: &Path,
    linear: &impl Posts,
    title: &str,
    drafts: &[Draft],
) -> (Result<Cut, Error>, String) {
    cut_after(repo, linear, title, drafts, &[])
}

// The same, for a slice whose `depends_on` names slices this run already filed.
fn cut_after(
    repo: &Path,
    linear: &impl Posts,
    title: &str,
    drafts: &[Draft],
    needs: &[&[LinearIssue]],
) -> (Result<Cut, Error>, String) {
    let mut out = Vec::new();
    let outcome = cut(
        linear,
        repo,
        filing(),
        Slice {
            title,
            drafts,
            needs,
        },
        &mut out,
    );

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
    issues_of(outcome)
        .iter()
        .map(|issue| issue.identifier().to_owned())
        .collect()
}

fn issues_of(outcome: Result<Cut, Error>) -> Vec<LinearIssue> {
    match outcome.expect("a slice that files") {
        Cut::Filed { issues, .. } => issues,
        Cut::Already(issues) => panic!("nothing was sent: {issues:?}"),
    }
}

fn reported(outcome: Result<Cut, Error>) -> Vec<String> {
    match outcome.expect("a slice that files") {
        Cut::Filed { reported, .. } => reported,
        Cut::Already(issues) => panic!("nothing was sent: {issues:?}"),
    }
}

// One earlier slice of this same run, filed against its own stand-in so the
// conversation the slice under test has is only its own. This is the road the
// issues of a `depends_on` arrive by: they are kept from the cut that made
// them, because a cut record holds identifiers and a relation is written by id.
fn an_earlier_slice(repo: &Path) -> Vec<LinearIssue> {
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(100)),
    ]);

    let outcome = cut_into(
        repo,
        &linear,
        "An earlier slice",
        &[a_draft("Walk the tree", "Every directory.")],
    )
    .0;

    issues_of(outcome)
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

#[test]
fn every_issue_exists_before_the_first_relation_is_written() {
    let repo = a_repository();
    let earlier = an_earlier_slice(repo.path());
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        Ok(issue(127)),
        Ok(relation(1)),
        Ok(relation(2)),
        Ok(relation(3)),
        Ok(relation(4)),
        Ok(relation(5)),
    ]);

    let outcome = cut_after(
        repo.path(),
        &linear,
        TITLE,
        &ordered_drafts(),
        &[earlier.as_slice()],
    )
    .0;

    assert_eq!(filed(outcome), ["WAR-125", "WAR-126", "WAR-127"]);
    let creates = linear.positions_of("issueCreate");
    let relations = linear.positions_of("issueRelationCreate");
    assert_eq!(creates.len(), 3, "{creates:?}");
    assert_eq!(relations.len(), 5, "{relations:?}");
    assert!(
        creates.iter().max() < relations.iter().min(),
        "an edge was written before every issue of the slice existed: \
         creates {creates:?}, relations {relations:?}"
    );
}

#[test]
fn a_slice_writes_its_own_edges_and_one_from_every_issue_it_waits_on() {
    let repo = a_repository();
    let earlier = an_earlier_slice(repo.path());
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        Ok(issue(127)),
        Ok(relation(1)),
        Ok(relation(2)),
        Ok(relation(3)),
        Ok(relation(4)),
        Ok(relation(5)),
    ]);

    cut_after(
        repo.path(),
        &linear,
        TITLE,
        &ordered_drafts(),
        &[earlier.as_slice()],
    )
    .0
    .expect("a slice that files");

    assert_eq!(
        linear.relations(),
        [
            // The drafts' own chain, said once though the middle pair names it
            // from both ends.
            edge("issue-125", "issue-126"),
            edge("issue-126", "issue-127"),
            // Then the slice this one waits on, to every issue of this one.
            edge("issue-100", "issue-125"),
            edge("issue-100", "issue-126"),
            edge("issue-100", "issue-127"),
        ],
    );
    assert_eq!(
        linear.documents().len(),
        11,
        "one request per operation and no retry"
    );
}

#[test]
fn a_pair_of_drafts_naming_each_other_is_one_edge() {
    let repo = a_repository();
    let mut drafts = two_drafts();
    drafts[0].blocks = vec![1];
    drafts[1].blocked_by = vec![0];
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        Ok(relation(1)),
    ]);

    cut_into(repo.path(), &linear, TITLE, &drafts)
        .0
        .expect("a slice that files");

    assert_eq!(linear.relations(), [edge("issue-125", "issue-126")]);
}

#[test]
fn a_slice_that_waits_on_nothing_and_orders_nothing_writes_no_relations() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");

    assert!(linear.relations().is_empty(), "{:?}", linear.relations());
}

#[test]
fn a_refused_relation_is_a_reported_line_and_the_slice_still_files() {
    let repo = a_repository();
    let mut drafts = two_drafts();
    drafts[1].blocked_by = vec![0];
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        refused("Entity not found"),
    ]);

    let outcome = cut_into(repo.path(), &linear, TITLE, &drafts).0;

    let Ok(Cut::Filed { issues, reported }) = outcome else {
        panic!("a refused edge failed the slice: {outcome:?}");
    };
    let identifiers: Vec<&str> = issues.iter().map(LinearIssue::identifier).collect();
    assert_eq!(identifiers, ["WAR-125", "WAR-126"]);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("`WAR-125`"), "{}", reported[0]);
    assert!(reported[0].contains("`WAR-126`"), "{}", reported[0]);
    assert!(reported[0].contains("Entity not found"), "{}", reported[0]);
    assert_eq!(
        linear.documents().len(),
        6,
        "the refused edge was tried a second time"
    );

    let cuts = cuts_of(repo.path());
    assert_eq!(cuts.len(), 1, "a missing edge left the slice unrecorded");
    assert_eq!(cuts[0].issues(), ["WAR-125", "WAR-126"]);
}

#[test]
fn every_refused_edge_of_a_slice_is_reported_and_the_rest_are_still_written() {
    let repo = a_repository();
    let earlier = an_earlier_slice(repo.path());
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        Ok(issue(127)),
        refused("Entity not found"),
        Ok(relation(2)),
        refused("Related issue is required"),
        Ok(relation(4)),
        Ok(relation(5)),
    ]);

    let outcome = cut_after(
        repo.path(),
        &linear,
        TITLE,
        &ordered_drafts(),
        &[earlier.as_slice()],
    )
    .0;

    let reported = reported(outcome);
    assert_eq!(reported.len(), 2, "{reported:?}");
    assert!(reported[0].contains("Entity not found"), "{reported:?}");
    assert!(
        reported[1].contains("Related issue is required"),
        "{reported:?}"
    );
    assert_eq!(
        linear.relations().len(),
        5,
        "an edge after a refused one was skipped"
    );
}

#[test]
fn the_project_comment_names_every_issue_and_says_the_status_was_not_moved() {
    let linear = Posting::answering([Ok(commented())]);

    let line = announce(
        &linear,
        PROJECT_ID,
        &["WAR-125".to_owned(), "WAR-126".to_owned()],
    );

    assert!(line.is_none(), "{line:?}");
    let comments = linear.comments();
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0]["projectId"], json!(PROJECT_ID));
    let body = comments[0]["body"].as_str().expect("a comment body");
    assert!(body.contains("`WAR-125`, `WAR-126`"), "{body}");
    assert!(body.contains("status was not moved"), "{body}");
    assert_eq!(
        comments[0]
            .as_object()
            .expect("a comment input")
            .keys()
            .collect::<Vec<_>>(),
        ["projectId", "body"],
        "the comment carried something other than the project and its text",
    );
    assert_eq!(
        linear.documents().len(),
        1,
        "one request per operation and no retry"
    );
}

#[test]
fn a_refused_comment_is_a_reported_line_and_leaves_the_slice_filed() {
    let repo = a_repository();
    let linear = Posting::answering([
        Ok(team_found()),
        Ok(backlog()),
        Ok(label_found()),
        Ok(issue(125)),
        Ok(issue(126)),
        refused("Comment is required"),
    ]);

    let identifiers = filed(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);
    let line = announce(&linear, PROJECT_ID, &identifiers).expect("a refused comment is a line");

    assert_eq!(identifiers, ["WAR-125", "WAR-126"]);
    assert!(line.contains("Comment is required"), "{line}");
    assert!(!line.contains('\n'), "a reported line is one line: {line}");
    assert_eq!(
        linear.documents().len(),
        6,
        "the refused comment was tried a second time"
    );
    assert_eq!(
        cuts_of(repo.path())[0].issues(),
        ["WAR-125", "WAR-126"],
        "a refused comment took the cut record with it"
    );
}
