use std::path::Path;

use tempfile::TempDir;
use warlock_engine::splitting::{Caught, Cycle, Numbered};
use warlock_engine::{
    Manifest, PactEntry, PullRun, PullSubtask, RunStatus, ScopeRecord, SubtaskStatus, state_path,
};
use warlock_tui::{
    Dirty, Finished, HUMAN_GATE, Split, Stopped, Touched, Unsplit, Worked, commit_message,
    pull_request_body, pull_request_title,
};

use super::{
    Error, Heading, PullEvent, Pulled, Pulling, Reached, Ticket, halt_comment, next_runnable,
};
use crate::stubs::{Boarding, Call, Checkout, Forging, GitCall, Op, Sessions, Slicing, said};

const TICKET: &str = "WAR-140";

const SCOPE: &str = "warlock-team";

// One run, built the way the loop builds one: `new` and then `push_subtask`, so
// nothing here depends on a file, a clock or a board.
fn run(subtasks: &[(&str, &[&str], SubtaskStatus)]) -> PullRun {
    let mut run = PullRun::new(
        TICKET,
        "Add `warlock pull <SCOPE>`",
        SCOPE,
        "war-140/add-warlock-pull-scope",
        "2026-09-28T09:00:00+00:00",
    );
    for (id, depends_on, status) in subtasks {
        let mut subtask =
            PullSubtask::new(*id, format!("Goal of {id}"), depends_on.iter().copied());
        subtask.set_status(status.clone());
        run.push_subtask(subtask);
    }
    run
}

fn next(run: &PullRun) -> Option<&str> {
    next_runnable(run).map(PullSubtask::id)
}

#[test]
fn a_linear_chain_gives_up_one_sub_task_at_a_time() {
    let mut chain = run(&[
        ("WAR-140.01", &[], SubtaskStatus::Pending),
        ("WAR-140.02", &["WAR-140.01"], SubtaskStatus::Pending),
        ("WAR-140.03", &["WAR-140.02"], SubtaskStatus::Pending),
    ]);

    assert_eq!(next(&chain), Some("WAR-140.01"));

    chain
        .subtask_mut("WAR-140.01")
        .expect("the sub-task is in the run")
        .set_status(SubtaskStatus::Done);
    assert_eq!(next(&chain), Some("WAR-140.02"));

    chain
        .subtask_mut("WAR-140.02")
        .expect("the sub-task is in the run")
        .set_status(SubtaskStatus::Done);
    assert_eq!(next(&chain), Some("WAR-140.03"));
}

#[test]
fn a_sub_task_in_progress_is_not_offered_again() {
    let working = run(&[
        ("WAR-140.01", &[], SubtaskStatus::InProgress),
        ("WAR-140.02", &[], SubtaskStatus::Pending),
    ]);

    assert_eq!(next(&working), Some("WAR-140.02"));
}

#[test]
fn a_blocked_branch_of_a_diamond_holds_only_what_waits_on_it() {
    let diamond = run(&[
        ("WAR-140.01", &[], SubtaskStatus::Done),
        (
            "WAR-140.02",
            &["WAR-140.01"],
            SubtaskStatus::Blocked("`crates/control` is scoped control-plane".to_owned()),
        ),
        ("WAR-140.03", &["WAR-140.01"], SubtaskStatus::Pending),
        (
            "WAR-140.04",
            &["WAR-140.02", "WAR-140.03"],
            SubtaskStatus::Pending,
        ),
    ]);

    // The sibling that does not wait on the blocked one still runs.
    assert_eq!(next(&diamond), Some("WAR-140.03"));

    let mut diamond = diamond;
    diamond
        .subtask_mut("WAR-140.03")
        .expect("the sub-task is in the run")
        .set_status(SubtaskStatus::Done);

    // The join waits on the blocked branch, so nothing is runnable.
    assert_eq!(next(&diamond), None);
}

#[test]
fn everything_left_waiting_on_a_failed_sub_task_is_nothing_runnable() {
    let failed = run(&[
        (
            "WAR-140.01",
            &[],
            SubtaskStatus::Failed("the tests would not build".to_owned()),
        ),
        ("WAR-140.02", &["WAR-140.01"], SubtaskStatus::Pending),
        ("WAR-140.03", &["WAR-140.01"], SubtaskStatus::Pending),
    ]);

    assert_eq!(next(&failed), None);
}

#[test]
fn a_crossed_sub_task_holds_its_dependants_the_same_way() {
    let crossed = run(&[
        (
            "WAR-140.01",
            &[],
            SubtaskStatus::Crossed("wrote crates/control/src/lib.rs".to_owned()),
        ),
        ("WAR-140.02", &["WAR-140.01"], SubtaskStatus::Pending),
    ]);

    assert_eq!(next(&crossed), None);
}

#[test]
fn a_run_with_nothing_but_done_has_nothing_to_run() {
    let finished = run(&[
        ("WAR-140.01", &[], SubtaskStatus::Done),
        ("WAR-140.02", &["WAR-140.01"], SubtaskStatus::Done),
    ]);

    assert_eq!(next(&finished), None);
    assert_eq!(next(&run(&[])), None);
}

// A record is hand-editable, so an id nothing in the run answers to is reachable.
// Unmet rather than ignored: the other reading works a sub-task whose dependency
// may never have happened.
#[test]
fn a_dependency_the_run_does_not_hold_is_not_met() {
    let mistyped = run(&[
        ("WAR-140.01", &[], SubtaskStatus::Done),
        ("WAR-140.02", &["WAR-140.1"], SubtaskStatus::Pending),
    ]);

    assert_eq!(next(&mistyped), None);
}

#[test]
fn the_halt_comment_lists_the_three_groups_and_ends_with_both_commands() {
    let halted = run(&[
        ("WAR-140.01", &[], SubtaskStatus::Done),
        (
            "WAR-140.02",
            &["WAR-140.01"],
            SubtaskStatus::Failed("the tests would not build".to_owned()),
        ),
        (
            "WAR-140.03",
            &["WAR-140.01"],
            SubtaskStatus::Blocked("only a person can settle the wire format".to_owned()),
        ),
        ("WAR-140.04", &["WAR-140.02"], SubtaskStatus::Pending),
    ]);

    let comment = halt_comment(&halted);

    assert_eq!(
        comment,
        "This pull halted, so the ticket has not moved. The branch \
         `war-140/add-warlock-pull-scope` holds one commit per finished sub-task and nothing else \
         was committed.\n\n\
         ## Finished\n\n\
         - `WAR-140.01` Goal of WAR-140.01\n\n\
         ## Stopped\n\n\
         - `WAR-140.02` Goal of WAR-140.02 — `failed`: the tests would not build\n\n\
         - `WAR-140.03` Goal of WAR-140.03 — `blocked`: only a person can settle the wire \
         format\n\n\
         ## Not started\n\n\
         - `WAR-140.04` Goal of WAR-140.04\n\n\
         `warlock resume WAR-140` puts the stopped sub-tasks back, and then `warlock pull \
         warlock-team --ticket WAR-140` works the ticket again."
    );

    let resume = comment
        .find("warlock resume WAR-140")
        .expect("the comment names the resume");
    let pull = comment
        .find("warlock pull warlock-team --ticket WAR-140")
        .expect("the comment names the pull");
    assert!(resume < pull);
}

#[test]
fn a_halt_with_nothing_finished_writes_no_empty_heading() {
    let halted = run(&[
        (
            "WAR-140.01",
            &[],
            SubtaskStatus::Failed("the tests would not build".to_owned()),
        ),
        ("WAR-140.02", &["WAR-140.01"], SubtaskStatus::Pending),
    ]);

    let comment = halt_comment(&halted);

    assert!(!comment.contains("## Finished"));
    assert!(comment.contains("## Stopped"));
    assert!(comment.contains("## Not started"));
}

// A run killed mid-session leaves a sub-task `in_progress` with nothing to say
// about how it ended. It is listed as stopped anyway: warlock started it, and a
// reader told it never started would go looking in the wrong place.
#[test]
fn a_sub_task_left_in_progress_is_listed_as_stopped() {
    let halted = run(&[("WAR-140.01", &[], SubtaskStatus::InProgress)]);

    let comment = halt_comment(&halted);

    assert!(comment.contains("## Stopped"));
    assert!(comment.contains("- `WAR-140.01` Goal of WAR-140.01 — `in_progress`"));
    assert!(!comment.contains("## Not started"));
}

#[test]
fn a_split_that_produced_nothing_still_names_both_commands() {
    let comment = halt_comment(&run(&[]));

    assert!(!comment.contains("##"));
    assert!(comment.ends_with(
        "`warlock resume WAR-140` puts the stopped sub-tasks back, and then `warlock pull \
         warlock-team --ticket WAR-140` works the ticket again."
    ));
}

#[test]
fn the_three_endings_spend_what_the_shell_spells_them_with() {
    let opened = Pulled::Opened {
        ticket: TICKET.to_owned(),
        url: Some("https://github.com/team/repo/pull/12".to_owned()),
    };
    let no_gh = Pulled::Opened {
        ticket: TICKET.to_owned(),
        url: None,
    };

    assert_eq!(opened.status(), 0);
    // No `gh` on the machine is still a run that did the work.
    assert_eq!(no_gh.status(), 0);
    assert_eq!(
        Pulled::Halted {
            ticket: TICKET.to_owned()
        }
        .status(),
        1
    );
    assert_eq!(
        Pulled::Crossed {
            ticket: TICKET.to_owned(),
            subtask: "WAR-140.02".to_owned(),
        }
        .status(),
        3
    );
    assert_eq!(opened.ticket(), TICKET);
}

// Everything below drives the whole of `Pulling::work` against fakes: no socket,
// no repository, no `claude`, and no clock read that anything asserts. What is
// real is a temporary home, because the run record is a file and the promise about
// it is that the next invocation reads back what this one wrote.

const TEAM: &str = "WAR";

const NUMBER: u32 = 140;

const ISSUE: &str = "issue-140";

const TITLE: &str = "Add `warlock pull <SCOPE>`";

const DESCRIPTION: &str =
    "The pieces do not add up to a command until something orchestrates them.";

// Detected, never guessed: `main` here is what this checkout's `origin/HEAD`
// answered, and every assertion about the branch reads it back rather than
// assuming it.
const DEFAULT: &str = "main";

// A scope in the manifest that this machine's sigils do not open.
const CLOSED: &str = "control-plane";

// A scope this machine holds that is not the one the ticket was pulled under: a
// path under it is not a crossing, and the pull request names it.
const OTHER: &str = "docs-team";

/// The ground a pull stands on that is not a fake: the two directories, and the
/// scope, manifest and sigils the door resolved before any of this ran.
struct Ground {
    home: TempDir,
    root: TempDir,
    scope: ScopeRecord,
    manifest: Manifest,
    held: Vec<String>,
}

impl Ground {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temporary repository");
        let manifest = Manifest::with_entries([
            pacted(root.path(), "crates/engine", SCOPE),
            pacted(root.path(), "crates/control", CLOSED),
            pacted(root.path(), "docs", OTHER),
        ]);
        Self {
            home: tempfile::tempdir().expect("a temporary home"),
            root,
            scope: ScopeRecord::new(SCOPE, TEAM, "In Review", "warlock"),
            manifest,
            held: vec![SCOPE.to_owned(), OTHER.to_owned()],
        }
    }

    /// The record as it is on disk, which is the only copy the next invocation of
    /// `warlock pull` will ever see.
    fn saved(&self) -> PullRun {
        PullRun::load(self.home.path(), self.root.path(), TICKET).expect("the run wrote its record")
    }
}

fn pacted(root: &Path, directory: &str, scope: &str) -> PactEntry {
    let at = root.join(directory);
    PactEntry::new(root, &at, at.join("WARLOCK.md"))
        .expect("the directory is under the root")
        .with_scope(scope)
}

fn ticket() -> Ticket<'static> {
    Ticket {
        id: ISSUE,
        identifier: TICKET,
        number: NUMBER,
        title: TITLE,
        description: DESCRIPTION,
    }
}

fn branch() -> String {
    warlock_tui::branch_name(TEAM, NUMBER, TITLE)
}

/// One modified path, which is a tree with something in it to commit.
fn wrote(path: &str) -> Vec<Dirty> {
    vec![Dirty {
        code: " M".to_owned(),
        path: path.to_owned(),
        from: None,
    }]
}

fn numbered(id: &str, goal: &str, depends_on: &[&str]) -> Numbered {
    Numbered {
        id: id.to_owned(),
        goal: goal.to_owned(),
        depends_on: depends_on.iter().map(|id| (*id).to_owned()).collect(),
        definition_of_done: vec![format!("{goal}, and the tests say so")],
        likely_files: vec!["crates/warlock-tui/src/pulling.rs".to_owned()],
        test_plan: String::new(),
        notes: String::new(),
    }
}

fn sliced(subtasks: Vec<Numbered>, repairs: Vec<String>) -> Slicing {
    Slicing::answering(Split::Subtasks { subtasks, repairs })
}

/// One whole pull, over the seams, with the progress it printed.
///
/// The forge is built here and asserted untouched on every road: opening the pull
/// request is the finish's, and a run that stopped short of it must not have asked
/// for one.
fn work(
    ground: &Ground,
    board: &Boarding,
    repo: &Checkout,
    split: &Slicing,
    sessions: &Sessions,
) -> (Result<Reached, Error>, Vec<PullEvent>) {
    let forge = Forging::opening("https://github.com/team/repo/pull/12");
    let mut events = Vec::new();
    let reached = {
        let mut sink = |event: PullEvent| events.push(event);
        Pulling {
            board,
            repo,
            forge: &forge,
            split,
            sessions,
            scope: &ground.scope,
            manifest: &ground.manifest,
            held: &ground.held,
            root: ground.root.path(),
            home: ground.home.path(),
            progress: &mut sink,
        }
        .work(&ticket())
    };

    assert!(
        forge.asked().is_empty(),
        "the sub-task loop asked for a pull request"
    );
    (reached, events)
}

/// One whole pull, the finish included, over the same seams: what a door spends.
fn pull(
    ground: &Ground,
    board: &Boarding,
    repo: &Checkout,
    forge: &Forging,
    split: &Slicing,
    sessions: &Sessions,
) -> (Result<Pulled, Error>, Vec<PullEvent>) {
    let mut events = Vec::new();
    let pulled = {
        let mut sink = |event: PullEvent| events.push(event);
        Pulling {
            board,
            repo,
            forge,
            split,
            sessions,
            scope: &ground.scope,
            manifest: &ground.manifest,
            held: &ground.held,
            root: ground.root.path(),
            home: ground.home.path(),
            progress: &mut sink,
        }
        .pull(&ticket())
    };
    (pulled, events)
}

/// Every comment this run left on its ticket, which a halt promises is exactly
/// one.
fn comments(board: &Boarding) -> Vec<String> {
    board
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::IssueComment { issue, body } => {
                assert_eq!(issue, ISSUE, "a comment went to another issue");
                Some(body)
            }
            _ => None,
        })
        .collect()
}

fn status_of(run: &PullRun, id: &str) -> SubtaskStatus {
    run.subtask(id)
        .expect("the sub-task is in the run")
        .status()
        .clone()
}

/// The headings a run prints when every sub-task is worked in order: the split,
/// and then one per sub-task with its one-based place in the record.
fn headings(worked: &[(&str, &str)]) -> Vec<PullEvent> {
    let mut events = vec![PullEvent::Heading(Heading::Split {
        ticket: TICKET.to_owned(),
        title: TITLE.to_owned(),
    })];
    for (at, (id, goal)) in worked.iter().enumerate() {
        events.push(PullEvent::Heading(Heading::Subtask {
            id: (*id).to_owned(),
            goal: (*goal).to_owned(),
            position: at + 1,
            total: worked.len(),
        }));
    }
    events
}

#[test]
fn a_run_cuts_the_branch_moves_the_ticket_splits_it_and_commits_every_sub_task() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([
        wrote("crates/engine/src/read.rs"),
        vec![
            Dirty {
                code: " M".to_owned(),
                path: "crates/engine/src/route.rs".to_owned(),
                from: None,
            },
            Dirty {
                code: "??".to_owned(),
                path: "docs/route.md".to_owned(),
                from: None,
            },
        ],
    ]);
    let split = Slicing::into_chain(TICKET, &["Add the reader", "Use the reader"]);
    let sessions = Sessions::answering([
        said("done", "Added the reader.", None),
        said("done", "Used the reader.", None),
    ]);

    let (reached, events) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Worked { run, touched }) = reached else {
        panic!("every sub-task finished: {reached:?}");
    };
    assert_eq!(run.status(), RunStatus::InProgress);
    assert_eq!(run.branch(), branch());
    assert_eq!(status_of(&run, "WAR-140.01"), SubtaskStatus::Done);
    assert_eq!(status_of(&run, "WAR-140.02"), SubtaskStatus::Done);
    assert_eq!(
        run.subtask("WAR-140.01").and_then(PullSubtask::log),
        Some("Added the reader.")
    );
    // The held scope the ticket was not pulled under, kept across the sessions:
    // the commit that ended the second one emptied the tree it was read from.
    assert_eq!(touched.len(), 1);
    assert_eq!(touched[0].scope, OTHER);
    assert_eq!(touched[0].paths, ["docs/route.md"]);

    assert_eq!(
        repo.commits(),
        [
            commit_message(TICKET, "WAR-140.01", "Add the reader"),
            commit_message(TICKET, "WAR-140.02", "Use the reader"),
        ]
    );
    // The branch is cut from the detected default branch, after a
    // fast-forward-only pull of it, and the switch comes first because that pull
    // merges into whatever is checked out.
    assert_eq!(
        repo.calls()[..4],
        [
            GitCall::DefaultBranch,
            GitCall::SwitchTo(DEFAULT.to_owned()),
            GitCall::CatchUp(DEFAULT.to_owned()),
            GitCall::CutBranch {
                branch: branch(),
                from: DEFAULT.to_owned(),
            },
        ]
    );
    assert!(!repo.calls().contains(&GitCall::Publish(branch())));

    // The ticket moved, and it moved before the split was asked for.
    assert_eq!(
        board.calls(),
        [
            Call::WorkflowState {
                team: TEAM.to_owned(),
                name: "In Progress".to_owned(),
            },
            Call::MoveIssue {
                issue: ISSUE.to_owned(),
                state: "state-backlog".to_owned(),
            },
        ]
    );
    assert_eq!(split.asked().len(), 1);
    assert_eq!(split.asked()[0].description, DESCRIPTION);
    // A finished run leaves no comment: the pull request URL is the finish's to
    // say.
    assert!(comments(&board).is_empty());

    // What the next invocation would read back.
    assert_eq!(ground.saved(), run);

    assert_eq!(
        events,
        headings(&[
            ("WAR-140.01", "Add the reader"),
            ("WAR-140.02", "Use the reader"),
        ])
    );

    // Each session got its own brief and the ticket as context, and the second
    // was told what the first reported.
    let openings = sessions.openings();
    assert_eq!(openings.len(), 2);
    assert!(openings[0].contains("Add the reader"));
    assert!(openings[0].contains(DESCRIPTION));
    assert!(!openings[0].contains("already finished"));
    assert!(openings[1].contains("Added the reader."));
}

#[test]
fn a_crossing_halts_at_once_commits_nothing_and_leaves_the_tree_alone() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([
        wrote("crates/engine/src/read.rs"),
        wrote("crates/control/src/lib.rs"),
    ]);
    let split = Slicing::into_chain(TICKET, &["Add the reader", "Use the reader"]);
    let sessions = Sessions::answering([
        said("done", "Added the reader.", None),
        said("done", "Used the reader, and a little more.", None),
    ]);

    let (reached, _) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Stopped(Pulled::Crossed { ticket, subtask })) = reached else {
        panic!("the second session crossed a boundary: {reached:?}");
    };
    assert_eq!(ticket, TICKET);
    assert_eq!(subtask, "WAR-140.02");
    assert_eq!(
        Pulled::Crossed {
            ticket,
            subtask: subtask.clone()
        }
        .status(),
        3
    );

    // The first sub-task's commit and nothing after it.
    assert_eq!(
        repo.commits(),
        [commit_message(TICKET, "WAR-140.01", "Add the reader")]
    );
    // The tree is left as the session left it: the last thing asked of the
    // checkout is the status that found the crossing.
    assert_eq!(repo.calls().last(), Some(&GitCall::Dirty));

    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::Halted);
    assert_eq!(status_of(&saved, "WAR-140.01"), SubtaskStatus::Done);
    let crossed = status_of(&saved, &subtask);
    assert_eq!(crossed.as_str(), "crossed");
    let reason = crossed.reason().expect("a crossing says what it wrote");
    assert!(reason.contains("crates/control/src/lib.rs"), "{reason}");
    assert!(reason.contains(CLOSED), "{reason}");

    // One comment, and it is the halt's account of the run.
    let posted = comments(&board);
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0], halt_comment(&saved));
    assert!(posted[0].contains("## Finished"));
    assert!(posted[0].contains("`crossed`"));
    // The ticket stays where it is: the only move is the one to `In Progress`.
    assert_eq!(board.positions_of(Op::MoveIssue).len(), 1);
}

#[test]
fn a_moved_head_halts_the_run_as_failed_and_names_the_commit() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT)
        .heads(["4e724822589a", "9f1c3a7b0d21"])
        .trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    let sessions = Sessions::answering([said("done", "Added the reader, and committed it.", None)]);

    let (reached, _) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Stopped(Pulled::Halted { ticket })) = reached else {
        panic!("the session committed, so the run halted: {reached:?}");
    };
    assert_eq!(ticket, TICKET);

    assert!(repo.commits().is_empty());
    // The tree is not even read: what the session wrote is in a commit, so the
    // status would answer about a tree that no longer holds it.
    assert!(!repo.calls().contains(&GitCall::Dirty));

    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::Halted);
    let failed = status_of(&saved, "WAR-140.01");
    assert_eq!(failed.as_str(), "failed");
    let reason = failed.reason().expect("a failure says why");
    assert!(reason.contains("4e724822"), "{reason}");
    assert!(reason.contains("9f1c3a7b"), "{reason}");
    assert_eq!(comments(&board).len(), 1);
}

#[test]
fn a_blocked_sub_task_lets_a_sibling_that_does_not_wait_on_it_run() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = sliced(
        vec![
            numbered("WAR-140.01", "Settle the wire format", &[]),
            numbered("WAR-140.02", "Write the format out", &["WAR-140.01"]),
            numbered("WAR-140.03", "Add the reader", &[]),
        ],
        vec!["dropped `depends_on` reference 4, which nothing answers to".to_owned()],
    );
    let sessions = Sessions::answering([
        said(
            "blocked",
            "The format is a decision.",
            Some("only a person can settle the wire format"),
        ),
        said("done", "Added the reader.", None),
    ]);

    let (reached, events) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Stopped(Pulled::Halted { .. })) = reached else {
        panic!("the run ran out of runnable sub-tasks: {reached:?}");
    };
    // Two sessions: the blocked one, then the sibling that does not wait on it.
    assert_eq!(sessions.openings().len(), 2);
    assert_eq!(
        repo.commits(),
        [commit_message(TICKET, "WAR-140.03", "Add the reader")]
    );

    let saved = ground.saved();
    assert_eq!(status_of(&saved, "WAR-140.01").as_str(), "blocked");
    // The one waiting on the blocked sub-task was never started.
    assert_eq!(status_of(&saved, "WAR-140.02"), SubtaskStatus::Pending);
    assert_eq!(status_of(&saved, "WAR-140.03"), SubtaskStatus::Done);

    let posted = comments(&board);
    assert_eq!(posted.len(), 1);
    assert!(posted[0].contains("## Finished"));
    assert!(posted[0].contains("only a person can settle the wire format"));
    assert!(posted[0].contains("## Not started"));

    // The repair the split needed is said rather than swallowed.
    assert!(events.contains(&PullEvent::Repair {
        note: "dropped `depends_on` reference 4, which nothing answers to".to_owned(),
    }));
    // The header's fraction is the sub-task's place in the record, not a count of
    // sessions.
    assert!(events.contains(&PullEvent::Heading(Heading::Subtask {
        id: "WAR-140.03".to_owned(),
        goal: "Add the reader".to_owned(),
        position: 3,
        total: 3,
    })));
}

#[test]
fn a_blocked_sub_task_with_nothing_else_runnable_halts_the_run() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Settle the wire format", "Write the format out"]);
    let sessions = Sessions::answering([said(
        "blocked",
        "The format is a decision.",
        Some("only a person can settle the wire format"),
    )]);

    let (reached, _) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Stopped(Pulled::Halted { .. })) = reached else {
        panic!("nothing was runnable: {reached:?}");
    };
    // One session, and nothing committed: the tree still holds what it wrote.
    assert_eq!(sessions.openings().len(), 1);
    assert!(repo.commits().is_empty());

    let posted = comments(&board);
    assert_eq!(posted.len(), 1);
    assert!(!posted[0].contains("## Finished"));
    assert!(posted[0].contains("## Stopped"));
}

#[test]
fn a_session_that_never_answered_is_failed_with_what_stopped_it() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    let sessions = Sessions::answering([Worked::Halted(Stopped::TimedOut)]);

    let (reached, _) = work(&ground, &board, &repo, &split, &sessions);

    assert!(matches!(
        reached,
        Ok(Reached::Stopped(Pulled::Halted { .. }))
    ));
    assert!(repo.commits().is_empty());

    let saved = ground.saved();
    let failed = status_of(&saved, "WAR-140.01");
    assert_eq!(failed.as_str(), "failed");
    assert_eq!(
        failed.reason(),
        Some(Stopped::TimedOut.to_string().as_str())
    );
    // No message to read, so nothing is logged in the session's name.
    assert_eq!(saved.subtask("WAR-140.01").and_then(PullSubtask::log), None);
}

#[test]
fn a_dirty_tree_on_a_resumed_run_refuses_and_commits_nothing() {
    let ground = Ground::new();
    let mut resumed =
        PullRun::new(TICKET, TITLE, SCOPE, branch(), "2026-09-28T09:00:00Z").with_subtasks([
            PullSubtask::new("WAR-140.01", "Add the reader", [] as [&str; 0]),
        ]);
    resumed.set_status(RunStatus::Resumed);
    resumed
        .save(ground.home.path(), ground.root.path())
        .expect("the record is written");

    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/half-done.rs")]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    // A board no call may reach and a session list nothing may draw from: the
    // refusal is asserted by the absence of everything it would have done.
    let sessions = Sessions::answering([]);
    let (reached, events) = work(&ground, &Boarding::unreachable(), &repo, &split, &sessions);

    let Err(Error::Dirty { branch: on, dirty }) = reached else {
        panic!("a dirty tree on a resumed run is a refusal: {reached:?}");
    };
    assert_eq!(on, branch());
    assert_eq!(dirty.len(), 1);
    let said = Error::Dirty { branch: on, dirty }.to_string();
    assert!(said.contains(&branch()), "{said}");
    assert!(said.contains("crates/engine/src/half-done.rs"), "{said}");

    // The run's own branch was checked out, the tree was read, and nothing else
    // happened.
    assert_eq!(repo.calls(), [GitCall::SwitchTo(branch()), GitCall::Dirty]);
    assert!(split.asked().is_empty());
    assert!(sessions.openings().is_empty());
    assert!(events.is_empty());
    // The record is untouched, so the run is still there to be picked up.
    assert_eq!(ground.saved(), resumed);
}

#[test]
fn a_clean_resumed_run_is_not_split_again_and_works_only_what_is_left() {
    let ground = Ground::new();
    let mut done = PullSubtask::new("WAR-140.01", "Add the reader", [] as [&str; 0]);
    done.set_status(SubtaskStatus::Done);
    done.set_log("Added the reader.");
    let mut resumed = PullRun::new(TICKET, TITLE, SCOPE, branch(), "2026-09-28T09:00:00Z")
        .with_subtasks([
            done,
            PullSubtask::new("WAR-140.02", "Use the reader", ["WAR-140.01"]),
        ]);
    resumed.set_status(RunStatus::Resumed);
    resumed
        .save(ground.home.path(), ground.root.path())
        .expect("the record is written");

    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/engine/src/route.rs")]);
    let split = Slicing::into_chain(TICKET, &["Something else entirely"]);
    let sessions = Sessions::answering([said("done", "Used the reader.", None)]);

    let (reached, events) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Worked { run, .. }) = reached else {
        panic!("the one sub-task left finished: {reached:?}");
    };
    assert!(split.asked().is_empty(), "a resumed run was split again");
    assert_eq!(run.subtasks().len(), 2);
    assert_eq!(status_of(&run, "WAR-140.02"), SubtaskStatus::Done);
    assert_eq!(
        repo.commits(),
        [commit_message(TICKET, "WAR-140.02", "Use the reader")]
    );
    assert_eq!(
        events,
        [PullEvent::Heading(Heading::Subtask {
            id: "WAR-140.02".to_owned(),
            goal: "Use the reader".to_owned(),
            position: 2,
            total: 2,
        })]
    );
    let openings = sessions.openings();
    assert_eq!(openings.len(), 1);
    assert!(openings[0].contains("Added the reader."));
}

#[test]
fn a_split_that_halts_halts_the_run_with_its_own_comment() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT);
    let unsplit = Unsplit::Circle(Cycle {
        ticket: TICKET.to_owned(),
        caught: vec![
            Caught {
                position: 1,
                goal: "Add the reader".to_owned(),
            },
            Caught {
                position: 2,
                goal: "Use the reader".to_owned(),
            },
        ],
    });
    let split = Slicing::answering(Split::Halted(unsplit.clone()));
    let sessions = Sessions::answering([]);

    let (reached, events) = work(&ground, &board, &repo, &split, &sessions);

    let Ok(Reached::Stopped(Pulled::Halted { ticket })) = reached else {
        panic!("an unsplit ticket halts the run: {reached:?}");
    };
    assert_eq!(ticket, TICKET);

    // No session was raised and nothing was committed, so no branch work is lost.
    assert!(sessions.openings().is_empty());
    assert!(repo.commits().is_empty());
    // The branch was cut all the same, which is why the record is written: it is
    // the only thing that says this machine holds the run.
    assert!(repo.calls().contains(&GitCall::CutBranch {
        branch: branch(),
        from: DEFAULT.to_owned(),
    }));

    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::Halted);
    assert!(saved.subtasks().is_empty());

    // The engine wrote the sentence; the run says it and nothing over the top.
    assert_eq!(comments(&board), [unsplit.to_string()]);
    assert_eq!(
        events,
        [PullEvent::Heading(Heading::Split {
            ticket: TICKET.to_owned(),
            title: TITLE.to_owned(),
        })]
    );
}

#[test]
fn a_team_with_no_in_progress_state_gets_a_line_and_the_run_carries_on() {
    let ground = Ground::new();
    let board = Boarding::filing("").without_backlog_state();
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    let sessions = Sessions::answering([said("done", "Added the reader.", None)]);

    let (reached, events) = work(&ground, &board, &repo, &split, &sessions);

    assert!(matches!(reached, Ok(Reached::Worked { .. })));
    assert_eq!(
        events[0],
        PullEvent::NoStartState {
            team: TEAM.to_owned(),
        }
    );
    // The state was asked for and no move was attempted.
    assert_eq!(
        board.calls(),
        [Call::WorkflowState {
            team: TEAM.to_owned(),
            name: "In Progress".to_owned(),
        }]
    );
    assert_eq!(
        repo.commits(),
        [commit_message(TICKET, "WAR-140.01", "Add the reader")]
    );
}

// The record is saved on both sides of every session, and only the save before one
// is invisible from outside: what the file holds afterwards would look the same
// either way. A run killed mid-session resumes from this.
#[test]
fn the_record_says_a_sub_task_is_in_progress_while_its_session_runs() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    let sessions = Sessions::answering([said("done", "Added the reader.", None)])
        .watching(state_path(ground.home.path(), ground.root.path(), TICKET));

    let (reached, _) = work(&ground, &board, &repo, &split, &sessions);

    assert!(matches!(reached, Ok(Reached::Worked { .. })));
    let seen = sessions.seen();
    assert_eq!(seen.len(), 1);
    let mid: PullRun =
        serde_json::from_str(&seen[0]).expect("the record was on disk before the session");
    assert_eq!(mid.status(), RunStatus::InProgress);
    assert_eq!(status_of(&mid, "WAR-140.01"), SubtaskStatus::InProgress);
    assert!(
        mid.subtask("WAR-140.01")
            .and_then(PullSubtask::started_at)
            .is_some()
    );
    // And the save after it is what the next invocation reads.
    assert_eq!(
        status_of(&ground.saved(), "WAR-140.01"),
        SubtaskStatus::Done
    );
}

// The finish: everything after the last sub-task's commit. Driven through
// `pull`, which is what a door calls, so the order asserted below is the order a
// real run does these in.

const URL: &str = "https://github.com/team/repo/pull/12";

const REVIEW: &str = "In Review";

/// The run every finish test below works: two sub-tasks, the second of which
/// also writes under a held scope the ticket was not pulled under.
fn two_sub_tasks() -> (Checkout, Slicing, Sessions) {
    let repo = Checkout::clean(DEFAULT).trees([
        wrote("crates/engine/src/read.rs"),
        vec![
            Dirty {
                code: " M".to_owned(),
                path: "crates/engine/src/route.rs".to_owned(),
                from: None,
            },
            Dirty {
                code: "??".to_owned(),
                path: "docs/route.md".to_owned(),
                from: None,
            },
        ],
    ]);
    let split = Slicing::into_chain(TICKET, &["Add the reader", "Use the reader"]);
    let sessions = Sessions::answering([
        said("done", "Added the reader.", None),
        said("done", "Used the reader.", None),
    ]);
    (repo, split, sessions)
}

/// The body that run's pull request carries, built the same way the finish
/// builds it — which is how the stale list being empty is asserted rather than
/// described.
fn expected_body() -> String {
    pull_request_body(
        DESCRIPTION,
        &[
            Finished {
                id: "WAR-140.01",
                goal: "Add the reader",
                summary: "Added the reader.",
            },
            Finished {
                id: "WAR-140.02",
                goal: "Use the reader",
                summary: "Used the reader.",
            },
        ],
        &[Touched {
            scope: OTHER,
            paths: vec!["docs/route.md"],
        }],
        &[],
    )
}

#[test]
fn a_finished_run_pushes_opens_the_pull_request_comments_and_moves_the_ticket() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, events) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    let Ok(opened) = pulled else {
        panic!("every sub-task finished, so the run opened a pull request: {pulled:?}");
    };
    assert_eq!(
        opened,
        Pulled::Opened {
            ticket: TICKET.to_owned(),
            url: Some(URL.to_owned()),
        }
    );
    assert_eq!(opened.status(), 0);

    // The request: opened against the detected default branch, from the run's own
    // branch, with the title and the body the pull request modules render.
    let asked = forge.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].base, DEFAULT);
    assert_eq!(asked[0].head, branch());
    assert_eq!(asked[0].title, pull_request_title(TICKET, TITLE));
    assert_eq!(asked[0].body, expected_body());
    assert!(asked[0].body.contains(HUMAN_GATE));
    assert!(asked[0].body.contains(OTHER));

    // The last sub-task's commit, then the push, and nothing between them but the
    // reading of the branch to open against.
    let calls = repo.calls();
    let last = calls
        .iter()
        .rposition(|call| matches!(call, GitCall::CommitAll(_)))
        .expect("the run committed its sub-tasks");
    assert_eq!(
        calls[last..],
        [
            GitCall::CommitAll(commit_message(TICKET, "WAR-140.02", "Use the reader")),
            GitCall::DefaultBranch,
            GitCall::Publish(branch()),
        ]
    );

    // The board, in order: the ticket moved to `In Progress` before the split, the
    // URL commented, and then the move to the scope record's review state.
    assert_eq!(
        board.calls(),
        [
            Call::WorkflowState {
                team: TEAM.to_owned(),
                name: "In Progress".to_owned(),
            },
            Call::MoveIssue {
                issue: ISSUE.to_owned(),
                state: "state-backlog".to_owned(),
            },
            Call::IssueComment {
                issue: ISSUE.to_owned(),
                body: format!(
                    "`{}` is pushed and its pull request is open: {URL}",
                    branch()
                ),
            },
            Call::WorkflowState {
                team: TEAM.to_owned(),
                name: REVIEW.to_owned(),
            },
            Call::MoveIssue {
                issue: ISSUE.to_owned(),
                state: "state-backlog".to_owned(),
            },
        ]
    );

    // What the next invocation reads back: the URL, and a run in review.
    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::InReview);
    assert_eq!(saved.pr_url(), Some(URL));

    // The pull request is a section of its own, and the last one.
    let mut expected = headings(&[
        ("WAR-140.01", "Add the reader"),
        ("WAR-140.02", "Use the reader"),
    ]);
    expected.push(PullEvent::Heading(Heading::PullRequest {
        branch: branch(),
    }));
    assert_eq!(events, expected);
}

// The refresh is a seam and not a behaviour yet: slice 9 of the brief fills it.
// What is asserted is that it costs nothing — no `git`, no session, no second
// split — so the day it does something, this test is what says so.
#[test]
fn the_refresh_call_site_runs_no_git_command_and_no_pass() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, _) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    assert!(pulled.is_ok(), "{pulled:?}");
    // One split, one session per sub-task, and no third of either.
    assert_eq!(split.asked().len(), 1);
    assert_eq!(sessions.openings().len(), 2);
    // One commit per sub-task: no `<TICKET>: refresh WARLOCK.md` beside them.
    assert_eq!(
        repo.commits(),
        [
            commit_message(TICKET, "WAR-140.01", "Add the reader"),
            commit_message(TICKET, "WAR-140.02", "Use the reader"),
        ]
    );
    // And nothing was left stale, so the body has no heading for it.
    assert!(!forge.asked()[0].body.contains("Directories left stale"));
}

#[test]
fn a_team_with_no_review_state_gets_a_line_and_the_run_still_finishes() {
    let ground = Ground::new();
    let board = Boarding::filing("").without_backlog_state();
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, events) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    let Ok(opened) = pulled else {
        panic!("a board with no such state is a line, not a failure: {pulled:?}");
    };
    assert_eq!(opened.status(), 0);
    assert!(events.contains(&PullEvent::NoReviewState {
        team: TEAM.to_owned(),
        state: REVIEW.to_owned(),
    }));

    // The state was asked for by the scope record's name and no move followed.
    assert!(board.positions_of(Op::MoveIssue).is_empty());
    assert_eq!(
        board.calls().last(),
        Some(&Call::WorkflowState {
            team: TEAM.to_owned(),
            name: REVIEW.to_owned(),
        })
    );
    // The pull request was opened and the URL is on the ticket all the same.
    assert_eq!(forge.asked().len(), 1);
    assert_eq!(comments(&board).len(), 1);
    assert_eq!(ground.saved().pr_url(), Some(URL));
}

#[test]
fn no_gh_comments_the_body_on_the_ticket_and_the_run_still_counts_as_finished() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::without_gh();
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, _) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    let Ok(opened) = pulled else {
        panic!("no `gh` is still a run that did the work: {pulled:?}");
    };
    assert_eq!(
        opened,
        Pulled::Opened {
            ticket: TICKET.to_owned(),
            url: None,
        }
    );
    assert_eq!(opened.status(), 0);

    // The branch was pushed and the request was asked for: what came back is that
    // there is no `gh` to ask.
    assert!(repo.calls().contains(&GitCall::Publish(branch())));
    assert_eq!(forge.asked().len(), 1);

    // The body is on the ticket instead, under a sentence naming the branch.
    let posted = comments(&board);
    assert_eq!(posted.len(), 1);
    assert!(posted[0].contains("no `gh`"), "{}", posted[0]);
    assert!(posted[0].contains(&branch()), "{}", posted[0]);
    assert!(posted[0].ends_with(&expected_body()), "{}", posted[0]);

    // In review, with no URL to record, and the ticket moved anyway.
    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::InReview);
    assert_eq!(saved.pr_url(), None);
    assert_eq!(board.positions_of(Op::MoveIssue).len(), 2);
}

#[test]
fn a_run_that_halted_never_reaches_the_forge() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/read.rs")]);
    let split = Slicing::into_chain(TICKET, &["Settle the wire format", "Write the format out"]);
    let sessions = Sessions::answering([said(
        "blocked",
        "The format is a decision.",
        Some("only a person can settle the wire format"),
    )]);

    let (pulled, events) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    let Ok(Pulled::Halted { ticket }) = pulled else {
        panic!("nothing was runnable: {pulled:?}");
    };
    assert_eq!(ticket, TICKET);
    assert!(forge.asked().is_empty());
    assert!(!repo.calls().contains(&GitCall::Publish(branch())));
    assert_eq!(ground.saved().status(), RunStatus::Halted);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PullEvent::Heading(Heading::PullRequest { .. })))
    );
}
