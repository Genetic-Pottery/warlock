use std::path::Path;

use tempfile::TempDir;
use warlock_engine::Destination;
use warlock_engine::drafting::Draft;

use super::{Cut, Filing, Slice, cut, fold_title, noted, skip};
use crate::error::Error;
use crate::error::status_for;
use crate::linear::{Board, Issue as LinearIssue};
use crate::stubs::{Boarding, Call, IssueAsked, Op, VIEWER};

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const TITLE: &str = "Read the file";

fn a_repository() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn a_draft(title: &str, body: &str) -> Draft {
    Draft {
        title: title.to_owned(),
        body: body.to_owned(),
        blocked_by: Vec::new(),
        blocks: Vec::new(),
        waits_on: Vec::new(),
    }
}

fn destination() -> Destination {
    Destination::new(SCOPE, TEAM, LABEL, "work")
}

fn filing(destination: &Destination) -> Filing<'_> {
    Filing {
        project: PROJECT_ID,
        destination,
        // What `prepare` resolved for the run: `cut` is handed the id rather
        // than asking for it, so nothing here makes a viewer request.
        assignee: VIEWER,
    }
}

// A workspace that has the team, its `Backlog` state and the label, and numbers
// the issues it creates from 125.
fn a_whole_cut() -> Boarding {
    Boarding::filing("").numbering_from(125)
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

fn edge(blocker: &str, waiting: &str) -> (String, String) {
    (blocker.to_owned(), waiting.to_owned())
}

// The whole operation, less the environment: the repository root is this test's
// temporary directory and the seam is whatever was handed in.
fn cut_into(
    repo: &Path,
    linear: &impl Board,
    title: &str,
    drafts: &[Draft],
) -> (Result<Cut, Error>, String) {
    cut_after(repo, linear, title, drafts, &[])
}

// The same, for a slice whose `depends_on` names slices this run already filed.
fn cut_after(
    repo: &Path,
    linear: &impl Board,
    title: &str,
    drafts: &[Draft],
    needs: &[&[LinearIssue]],
) -> (Result<Cut, Error>, String) {
    let mut out = Vec::new();
    let outcome = cut(
        linear,
        repo,
        filing(&destination()),
        Slice {
            title,
            drafts,
            needs,
            open: &[],
        },
        &mut out,
    );

    (
        outcome,
        String::from_utf8(out).expect("warlock writes its own text"),
    )
}

// The same cut for a run whose viewer answered somebody else, which is the one
// way to tell an assignee that came through `Filing` from one this module could
// have gone and asked for itself.
fn cut_assigning(repo: &Path, linear: &impl Board, assignee: &str) -> Result<Cut, Error> {
    let destination = destination();
    let mut out = Vec::new();

    cut(
        linear,
        repo,
        Filing {
            assignee,
            ..filing(&destination)
        },
        Slice {
            title: TITLE,
            drafts: &two_drafts(),
            needs: &[],
            open: &[],
        },
        &mut out,
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

fn filed(outcome: Result<Cut, Error>) -> Vec<String> {
    issues_of(outcome)
        .iter()
        .map(|issue| issue.identifier().to_owned())
        .collect()
}

fn issues_of(outcome: Result<Cut, Error>) -> Vec<LinearIssue> {
    outcome.expect("a slice that files").issues
}

fn reported(outcome: Result<Cut, Error>) -> Vec<String> {
    outcome.expect("a slice that files").reported
}

fn notes_of(linear: &Boarding) -> Vec<String> {
    linear
        .comments()
        .into_iter()
        .map(|(project, body)| {
            assert_eq!(project, PROJECT_ID, "a note went to another project");
            body
        })
        .collect()
}

// One earlier slice of this same run, filed against its own stand-in so the
// conversation the slice under test has is only its own. This is the road the
// issues of a `depends_on` arrive by: they are kept from the cut that made
// them, because a note holds identifiers and a relation is written by id.
fn an_earlier_slice(repo: &Path) -> Vec<LinearIssue> {
    let linear = Boarding::filing("").numbering_from(100);

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
fn a_slice_becomes_one_issue_per_draft_and_one_note() {
    let repo = a_repository();
    let linear = a_whole_cut();

    let (outcome, printed) = cut_into(repo.path(), &linear, TITLE, &two_drafts());

    assert_eq!(filed(outcome), ["WAR-125", "WAR-126"]);
    assert_eq!(
        linear.ops(),
        [
            Op::Team,
            Op::BacklogState,
            Op::IssueLabel,
            Op::CreateIssue,
            Op::CreateIssue,
            Op::Comment
        ],
        "one call per operation, one create per draft, one note, and no retry"
    );
    assert!(printed.contains(TITLE), "{printed}");
    assert!(printed.contains("`WAR-125`, `WAR-126`"), "{printed}");
    assert_eq!(
        notes_of(&linear),
        ["Warlock cut slice `Read the file` into `WAR-125`, `WAR-126`."]
    );
}

#[test]
fn each_issue_carries_the_scope_records_team_the_project_and_the_backlog_state() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");

    let issue = |title: &str, body: &str| IssueAsked {
        title: title.to_owned(),
        body: body.to_owned(),
        team: "team-1".to_owned(),
        project: PROJECT_ID.to_owned(),
        label: "label-held".to_owned(),
        state: "state-backlog".to_owned(),
        assignee: VIEWER.to_owned(),
    };
    assert_eq!(
        linear.issues_created(),
        [
            issue("Read the file off disk", "The bytes, whole."),
            issue("Fold its title", "Lowercased, collapsed."),
        ],
    );
}

#[test]
fn the_assignee_the_filing_carries_reaches_every_create_and_is_not_asked_for_here() {
    // The id `prepare` resolved for the whole run arrives built, so an issue is
    // assigned to whoever `Filing` names and nothing else: a cut that went and
    // asked the board itself would be a request per slice for an answer that
    // cannot have changed, and would still be right by accident while the
    // stand-in answers the same user.
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_assigning(repo.path(), &linear, "user-somebody-else").expect("a slice that files");

    let issues = linear.issues_created();
    assert_eq!(issues.len(), 2, "{issues:?}");
    for issue in &issues {
        assert_eq!(issue.assignee, "user-somebody-else", "{issue:?}");
    }
    assert!(
        !linear.ops().contains(&Op::Viewer),
        "a cut asked the board who its issues are for: {:?}",
        linear.ops()
    );
}

#[test]
fn the_label_is_resolved_as_an_issue_label_and_not_as_a_project_label() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");

    // Resolved on the team, by the name the scope record gives. That the issue
    // label queries never reach the project label ones is `linear.rs`'s to
    // hold; what a cut can do wrong is resolve the label as a project's, which
    // only `create_project` does.
    let calls = linear.calls();
    assert!(
        calls.contains(&Call::IssueLabel {
            name: LABEL.to_owned(),
            team: "team-1".to_owned(),
        }),
        "{calls:?}"
    );
    assert!(
        !linear.ops().contains(&Op::CreateProject),
        "a project label's id is not usable by an issue: {calls:?}"
    );
}

#[test]
fn the_label_the_board_answers_is_the_one_every_issue_carries() {
    // Whether the board found the label or had to make it is `linear.rs`'s own
    // question; this is the id that came back reaching the create.
    let repo = a_repository();
    let linear = a_whole_cut().labelled("label-made");

    let outcome = cut_into(repo.path(), &linear, TITLE, &two_drafts()[..1]).0;

    assert_eq!(filed(outcome), ["WAR-125"]);
    assert_eq!(linear.issues_created()[0].label, "label-made");
}

#[test]
fn a_team_with_no_backlog_state_is_refused_naming_the_team() {
    let repo = a_repository();
    let linear = a_whole_cut().without_backlog_state();

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(
        matches!(&error, Error::NoBacklog { team } if team == TEAM),
        "{error:?}"
    );
    assert!(
        linear.issues_created().is_empty(),
        "an issue was created: {:?}",
        linear.issues_created()
    );
    let message = said(&error);
    assert!(message.contains(TEAM), "{message}");
    assert!(message.contains("Backlog"), "{message}");
    assert!(linear.comments().is_empty(), "a refusal noted a cut");
}

#[test]
fn the_backlog_state_is_asked_for_before_the_label_and_before_any_create() {
    let repo = a_repository();
    let linear = a_whole_cut().without_backlog_state();

    refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert_eq!(linear.ops(), [Op::Team, Op::BacklogState]);
}

#[test]
fn an_unknown_team_is_refused_before_anything_is_created() {
    let repo = a_repository();
    let linear = a_whole_cut().without_team();

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(
        matches!(&error, Error::UnknownTeam { team, .. } if team == TEAM),
        "{error:?}"
    );
    assert_eq!(
        linear.ops(),
        [Op::Team],
        "anything after the team was asked"
    );
}

#[test]
fn a_create_that_fails_partway_notes_nothing() {
    let repo = a_repository();
    let linear = a_whole_cut().refuse_from(Op::CreateIssue, 1, "Entity not found");

    let error = refusal(cut_into(repo.path(), &linear, TITLE, &two_drafts()).0);

    assert!(matches!(&error, Error::Linear { .. }), "{error:?}");
    assert!(
        said(&error).contains("Entity not found"),
        "{}",
        said(&error)
    );
    assert!(
        linear.comments().is_empty(),
        "a half filed slice was noted as cut"
    );
}

#[test]
fn the_identifiers_come_back_when_the_note_is_refused() {
    let repo = a_repository();
    let linear = a_whole_cut().refuse(Op::Comment, "Comment is required");

    let (outcome, printed) = cut_into(repo.path(), &linear, TITLE, &two_drafts());

    let error = refusal(outcome);
    assert!(
        matches!(&error, Error::Uncut { issues, .. } if issues == &["WAR-125", "WAR-126"]),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("`WAR-125`, `WAR-126`"), "{message}");
    assert!(message.contains("Comment is required"), "{message}");
    assert!(
        printed.contains("`WAR-125`, `WAR-126`"),
        "the issues exist and were not named: {printed}"
    );
    assert_eq!(
        linear.positions_of(Op::Comment).len(),
        1,
        "the refused note was tried a second time"
    );
}

#[test]
fn every_issue_exists_before_the_first_relation_is_written() {
    let repo = a_repository();
    let earlier = an_earlier_slice(repo.path());
    let linear = a_whole_cut();

    let outcome = cut_after(
        repo.path(),
        &linear,
        TITLE,
        &ordered_drafts(),
        &[earlier.as_slice()],
    )
    .0;

    assert_eq!(filed(outcome), ["WAR-125", "WAR-126", "WAR-127"]);
    let creates = linear.positions_of(Op::CreateIssue);
    let relations = linear.positions_of(Op::Relation);
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
    let linear = a_whole_cut();

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
    assert_eq!(linear.requests(), 12, "one call per operation and no retry");
}

#[test]
fn a_pair_of_drafts_naming_each_other_is_one_edge() {
    let repo = a_repository();
    let mut drafts = two_drafts();
    drafts[0].blocks = vec![1];
    drafts[1].blocked_by = vec![0];
    let linear = a_whole_cut();

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
    let linear = a_whole_cut().refuse(Op::Relation, "Entity not found");

    let outcome = cut_into(repo.path(), &linear, TITLE, &drafts).0;

    let Ok(Cut { issues, reported }) = outcome else {
        panic!("a refused edge failed the slice: {outcome:?}");
    };
    let identifiers: Vec<&str> = issues.iter().map(LinearIssue::identifier).collect();
    assert_eq!(identifiers, ["WAR-125", "WAR-126"]);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("`WAR-125`"), "{}", reported[0]);
    assert!(reported[0].contains("`WAR-126`"), "{}", reported[0]);
    assert!(reported[0].contains("Entity not found"), "{}", reported[0]);
    assert_eq!(
        linear.requests(),
        7,
        "the refused edge was tried a second time"
    );
    assert_eq!(
        notes_of(&linear),
        ["Warlock cut slice `Read the file` into `WAR-125`, `WAR-126`."],
        "a missing edge left the slice unnoted"
    );
}

#[test]
fn every_refused_edge_of_a_slice_is_reported_and_the_rest_are_still_written() {
    let repo = a_repository();
    let earlier = an_earlier_slice(repo.path());
    let linear = a_whole_cut()
        .refuse_at(Op::Relation, 0, "Entity not found")
        .refuse_at(Op::Relation, 2, "Related issue is required");

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
fn the_note_is_said_after_every_issue_and_every_edge() {
    let repo = a_repository();
    let linear = a_whole_cut();

    cut_into(repo.path(), &linear, TITLE, &ordered_drafts())
        .0
        .expect("a slice that files");

    let notes = linear.positions_of(Op::Comment);
    assert_eq!(notes.len(), 1, "{:?}", linear.ops());
    assert_eq!(notes[0], linear.requests() - 1, "{:?}", linear.ops());
}

#[test]
fn a_skip_is_one_note_naming_the_slice() {
    let linear = a_whole_cut();

    skip(&linear, PROJECT_ID, TITLE).expect("a skip that is noted");

    assert_eq!(
        notes_of(&linear),
        ["Warlock skipped slice `Read the file`."]
    );
    assert_eq!(linear.ops(), [Op::Comment]);
}

#[test]
fn a_refused_skip_is_unskipped_naming_the_slice() {
    let linear = a_whole_cut().refuse(Op::Comment, "Comment is required");

    let error = skip(&linear, PROJECT_ID, TITLE).expect_err("a refused note");

    assert!(
        matches!(&error, Error::Unskipped { title, .. } if title == TITLE),
        "{error:?}"
    );
    assert!(said(&error).contains("Comment is required"), "{error}");
}

#[test]
fn both_note_shapes_read_back_as_written() {
    let linear = a_whole_cut();
    let repo = a_repository();
    cut_into(repo.path(), &linear, TITLE, &two_drafts())
        .0
        .expect("a slice that files");
    skip(&linear, PROJECT_ID, "Fold the Title").expect("a skip that is noted");

    let read = noted(&notes_of(&linear));

    assert_eq!(
        read,
        [
            (
                "read the file".to_owned(),
                vec!["WAR-125".to_owned(), "WAR-126".to_owned()]
            ),
            ("fold the title".to_owned(), Vec::new()),
        ]
    );
}

#[test]
fn a_comment_that_is_not_a_note_is_left_out() {
    let read = noted(&[
        "Warlock cut slice without a title".to_owned(),
        "Warlock skipped slice nothing in backticks.".to_owned(),
        "Warlock cut slice `Named` into nothing at all.".to_owned(),
        "Looks good to me".to_owned(),
        "Warlock skipped slice `Kept`.\n".to_owned(),
    ]);

    assert_eq!(read, [("kept".to_owned(), Vec::new())]);
}

#[test]
fn a_title_is_folded_for_case_and_whitespace_only() {
    assert_eq!(fold_title("  READ   the\tFile "), "read the file");
    assert_ne!(fold_title("Read the file"), fold_title("Read the files"));
}

// Forman's `blocked_by: ["TEAM-42"]`: a draft waiting on a ticket that already
// exists, which is one of the open tickets its session was shown.
#[test]
fn a_draft_waiting_on_an_open_ticket_it_was_shown_is_blocked_by_it_and_no_other() {
    let repo = a_repository();
    let mut drafts = two_drafts();
    drafts[0].waits_on = vec!["war-142".to_owned()];
    drafts[1].waits_on = vec!["WAR-999".to_owned()];
    let open = [LinearIssue::new("issue-142", "WAR-142", "")];
    let linear = a_whole_cut();
    let mut out = Vec::new();

    let outcome = cut(
        &linear,
        repo.path(),
        filing(&destination()),
        Slice {
            title: TITLE,
            drafts: &drafts,
            needs: &[],
            open: &open,
        },
        &mut out,
    );

    // Matched without regard to case, as Linear's identifiers are.
    assert_eq!(linear.relations(), [edge("issue-142", "issue-125")]);
    // And a ticket nobody showed the session is a line, not a lookup.
    let reported = reported(outcome);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("`WAR-999`"), "{reported:?}");
}
