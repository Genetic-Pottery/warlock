use std::path::Path;

use tempfile::TempDir;
use warlock_engine::splitting::{Caught, Cycle, Numbered};
use warlock_engine::{
    Manifest, PactEntry, PullRun, PullSubtask, RunStatus, ScopeRecord, SubtaskStatus, state_path,
};

use super::{
    Error, Heading, PullEvent, Pulled, Pulling, Reached, Ticket, halt_comment, next_runnable,
};
use crate::claude::{Split, Stopped, Unsplit, Worked};
use crate::freshness::Freshened;
use crate::git::{
    Dirty, Error as GitError, Finished, Freshness, HUMAN_GATE, LeftStale, Repository, Touched,
    commit_message, pull_request_body, pull_request_title,
};
use crate::linear::IssueProject;
use crate::pulling::StaleDirectory;
use crate::stubs::{
    Boarding, Call, Checkout, Forging, GitCall, Op, Refreshing, Sessions, Slicing, said,
};

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
    crate::git::branch_name(TEAM, NUMBER, TITLE)
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
    let freshen = Refreshing::quiet();
    let mut events = Vec::new();
    let reached = {
        let mut sink = |event: PullEvent| events.push(event);
        Pulling {
            board,
            repo,
            forge: &forge,
            split,
            sessions,
            freshen: &freshen,
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
    // The refresh belongs to the finish, after the last sub-task's commit: a
    // sub-task loop that asked for one would be describing directories the run is
    // still editing.
    assert!(
        freshen.asked().is_empty(),
        "the sub-task loop asked for a refresh"
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
    pull_freshening(
        ground,
        board,
        repo,
        forge,
        split,
        sessions,
        &Refreshing::quiet(),
    )
}

/// The same pull with the freshness pass written down, for the tests that are
/// about what the refresh came to rather than about the work.
fn pull_freshening(
    ground: &Ground,
    board: &Boarding,
    repo: &Checkout,
    forge: &Forging,
    split: &Slicing,
    sessions: &Sessions,
    freshen: &Refreshing,
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
            freshen,
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
            Call::Team(TEAM.to_owned()),
            Call::WorkflowState {
                team: "team-1".to_owned(),
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
        [
            Call::Team(TEAM.to_owned()),
            Call::WorkflowState {
                team: "team-1".to_owned(),
                name: "In Progress".to_owned(),
            }
        ]
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
/// builds it — which is how the freshness section being absent, or saying exactly
/// this, is asserted rather than described.
fn expected_body(freshness: &Freshness<'_>) -> String {
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
        freshness,
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
    assert_eq!(asked[0].body, expected_body(&Freshness::default()));
    assert!(asked[0].body.contains(HUMAN_GATE));
    assert!(asked[0].body.contains(OTHER));

    // The last sub-task's commit, then the push, and nothing between them but the
    // reading of the branch to open against and the check that it changed
    // something.
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
            GitCall::ChangedAgainst(DEFAULT.to_owned()),
            GitCall::Publish(branch()),
        ]
    );

    // The board, in order: the ticket moved to `In Progress` before the split, the
    // URL commented, and then the move to the scope record's review state.
    assert_eq!(
        board.calls(),
        [
            Call::Team(TEAM.to_owned()),
            Call::WorkflowState {
                team: "team-1".to_owned(),
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
            Call::Team(TEAM.to_owned()),
            Call::WorkflowState {
                team: "team-1".to_owned(),
                name: REVIEW.to_owned(),
            },
            Call::MoveIssue {
                issue: ISSUE.to_owned(),
                state: "state-backlog".to_owned(),
            },
            Call::IssueProject(ISSUE.to_owned()),
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

// The refresh, over the seam: what the loop asks it, where in the finish it is
// asked, and what the body says about the answer. What the pass itself decides —
// which directories are stale, what the boundary refuses, what a failed pass
// reports — is `freshness.rs`'s to test, and is written down here rather than
// produced.
#[test]
fn a_branch_that_left_nothing_stale_refreshes_nothing_and_gets_no_freshness_headings() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();
    let freshen = Refreshing::quiet();

    let (pulled, _) = pull_freshening(&ground, &board, &repo, &forge, &split, &sessions, &freshen);

    assert!(pulled.is_ok(), "{pulled:?}");
    // The pass is asked once — not once per sub-task and not once per directory —
    // and what it is asked with is the run's own: the ticket the commit message is
    // built from, the repository root, the sigils the boundary is judged against,
    // and the manifest the loop is holding rather than one loaded again.
    let asked = freshen.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].ticket, TICKET);
    assert_eq!(asked[0].root, ground.root.path());
    assert_eq!(asked[0].held, ground.held);
    assert_eq!(asked[0].pacted, ["crates/engine", "crates/control", "docs"]);

    // One split, one session per sub-task, and no third of either.
    assert_eq!(split.asked().len(), 1);
    assert_eq!(sessions.openings().len(), 2);
    // One commit per sub-task: no `<TICKET>: refresh WARLOCK.md` beside them, because
    // the pass found nothing to write.
    assert_eq!(
        repo.commits(),
        [
            commit_message(TICKET, "WAR-140.01", "Add the reader"),
            commit_message(TICKET, "WAR-140.02", "Use the reader"),
        ]
    );
    // And neither heading is in the body: an empty answer says nothing at all,
    // rather than two headings saying there was nothing.
    let body = &forge.asked()[0].body;
    assert_eq!(*body, expected_body(&Freshness::default()));
    assert!(!body.contains("Documents refreshed"), "{body}");
    assert!(!body.contains("Directories left stale"), "{body}");
}

#[test]
fn a_refresh_names_its_directories_in_the_body_and_commits_between_the_work_and_the_push() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();
    let message = format!("{TICKET}: refresh WARLOCK.md");
    // Children before parents, as the pass hands them back, and the one commit it
    // makes through the checkout it was given.
    let freshen = Refreshing::answering(Freshened {
        refreshed: vec!["crates/engine".to_owned(), "crates".to_owned()],
        left_stale: Vec::new(),
    })
    .committing(
        &message,
        &["crates/engine/WARLOCK.md", ".warlock/pacts.toml"],
    );

    let (pulled, events) =
        pull_freshening(&ground, &board, &repo, &forge, &split, &sessions, &freshen);

    assert!(pulled.is_ok(), "{pulled:?}");
    let body = &forge.asked()[0].body;
    assert_eq!(
        *body,
        expected_body(&Freshness {
            refreshed: &["crates/engine", "crates"],
            left_stale: &[],
        })
    );
    // Named in the order the passes ran, and nothing is claimed to be left stale.
    assert!(body.contains("## Documents refreshed"), "{body}");
    assert!(
        body.find("- `crates/engine`") < body.find("- `crates`"),
        "{body}"
    );
    assert!(!body.contains("Directories left stale"), "{body}");

    // Where the pass ran, in the checkout's own log: after the last sub-task's
    // commit and the check that the branch changed something, and before the
    // push.
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
            GitCall::ChangedAgainst(DEFAULT.to_owned()),
            GitCall::CommitPaths {
                message: message.clone(),
                paths: vec![
                    "crates/engine/WARLOCK.md".to_owned(),
                    ".warlock/pacts.toml".to_owned(),
                ],
            },
            GitCall::Publish(branch()),
        ]
    );
    // The refresh commit is its own, beside the sub-tasks' rather than instead of
    // one of them.
    assert_eq!(
        repo.commits(),
        [
            commit_message(TICKET, "WAR-140.01", "Add the reader"),
            commit_message(TICKET, "WAR-140.02", "Use the reader"),
            message,
        ]
    );
    // And the refresh opens no section of its own: the sections are still the
    // sub-tasks' and the pull request's.
    let mut expected = headings(&[
        ("WAR-140.01", "Add the reader"),
        ("WAR-140.02", "Use the reader"),
    ]);
    expected.push(PullEvent::Heading(Heading::PullRequest {
        branch: branch(),
    }));
    assert_eq!(events, expected);
}

#[test]
fn a_closed_scope_and_a_failed_pass_are_named_stale_and_the_pull_request_is_still_opened() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();
    // One of each of the two reasons a directory is left stale, and one directory
    // refreshed beside them: the three are one finding, and the body carries all
    // three.
    let closed = "closed to this machine, which holds no `control-plane` sigil";
    let failed = "the refresh pass failed: `docs` — the model answered nothing";
    let freshen = Refreshing::answering(Freshened {
        refreshed: vec!["crates/engine".to_owned()],
        left_stale: vec![
            StaleDirectory {
                directory: "crates/control".to_owned(),
                reason: closed.to_owned(),
            },
            StaleDirectory {
                directory: "docs".to_owned(),
                reason: failed.to_owned(),
            },
        ],
    });

    let (pulled, _) = pull_freshening(&ground, &board, &repo, &forge, &split, &sessions, &freshen);

    // A pass that failed is not the run failing: the sub-task commits are on the
    // branch, and the request is opened with the failure named in it.
    assert_eq!(
        pulled.expect("a failed pass still reaches the pull request"),
        Pulled::Opened {
            ticket: TICKET.to_owned(),
            url: Some(URL.to_owned()),
        }
    );
    let body = &forge.asked()[0].body;
    assert_eq!(
        *body,
        expected_body(&Freshness {
            refreshed: &["crates/engine"],
            left_stale: &[
                LeftStale {
                    directory: "crates/control",
                    reason: closed,
                },
                LeftStale {
                    directory: "docs",
                    reason: failed,
                },
            ],
        })
    );
    // Both reasons, in the pass's own words, under the one heading — and the
    // refreshed directory above it.
    assert!(
        body.find("## Documents refreshed") < body.find("## Directories left stale"),
        "{body}"
    );
    assert!(
        body.contains(&format!("- `crates/control` — {closed}")),
        "{body}"
    );
    assert!(body.contains(&format!("- `docs` — {failed}")), "{body}");
    // Nothing the pass reported is a commit the loop made: the pass commits its
    // own documents or nothing at all.
    assert_eq!(
        repo.commits(),
        [
            commit_message(TICKET, "WAR-140.01", "Add the reader"),
            commit_message(TICKET, "WAR-140.02", "Use the reader"),
        ]
    );
}

#[test]
fn a_checkout_the_pass_could_not_ask_fails_the_run_before_the_branch_is_pushed() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, _) = pull_freshening(
        &ground,
        &board,
        &repo,
        &forge,
        &split,
        &sessions,
        &Refreshing::refusing(),
    );

    // The one failure the pass hands back rather than reporting, and it fails the
    // run the way every other `git` failure in this loop does: a checkout that
    // cannot be asked what the branch changed is not a fact to put in a body.
    let error = pulled.expect_err("the checkout could not be asked");
    assert!(matches!(&error, Error::Git { .. }), "{error:?}");
    // Nothing was pushed and no request was opened, so the finish stopped where it
    // failed.
    assert!(!repo.calls().contains(&GitCall::Publish(branch())));
    assert!(forge.asked().is_empty());
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
            team: "team-1".to_owned(),
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
    assert!(
        posted[0].ends_with(&expected_body(&Freshness::default())),
        "{}",
        posted[0]
    );

    // In review, with no URL to record, and the ticket moved anyway.
    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::InReview);
    assert_eq!(saved.pr_url(), None);
    assert_eq!(board.positions_of(Op::MoveIssue).len(), 2);
}

#[test]
fn a_run_whose_branch_changed_nothing_opens_no_pull_request_and_says_so_on_the_ticket() {
    let ground = Ground::new();
    let board = Boarding::filing("");
    let forge = Forging::opening(URL);
    let repo = Checkout::clean(DEFAULT).changed([Vec::new()]);
    let split = Slicing::into_chain(TICKET, &["Add the reader"]);
    let sessions = Sessions::answering([said("done", "The reader was already there.", None)]);

    let (pulled, _) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    let Ok(ended) = pulled else {
        panic!("a sub-task that found its work done is still a finished run: {pulled:?}");
    };
    assert_eq!(
        ended,
        Pulled::Unchanged {
            ticket: TICKET.to_owned(),
        }
    );
    assert_eq!(ended.status(), 0);

    // Nothing pushed and nothing asked of the forge: `gh` refuses a pull request
    // with no commits on it.
    assert!(!repo.calls().contains(&GitCall::Publish(branch())));
    assert!(forge.asked().is_empty());

    // What the session found is on the ticket, and the ticket went to review.
    let posted = comments(&board);
    assert_eq!(posted.len(), 1);
    assert!(
        posted[0].starts_with("No change was needed"),
        "{}",
        posted[0]
    );
    assert!(posted[0].contains(DEFAULT), "{}", posted[0]);
    assert!(
        posted[0].contains("`WAR-140.01` The reader was already there."),
        "{}",
        posted[0]
    );
    assert_eq!(board.positions_of(Op::MoveIssue).len(), 2);

    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::InReview);
    assert_eq!(saved.pr_url(), None);
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

// Not a test of the loop but of the checkout the loop is driven through: the two
// calls a freshness pass will make are answered out of memory and written down,
// so the slice that makes them can be tested with no repository at all.
#[test]
fn a_checkout_answers_a_scripted_diff_and_writes_down_a_documents_only_commit() {
    let repo = Checkout::clean(DEFAULT).changed([
        vec!["crates/engine/src/read.rs", "crates/tui/src/panel.rs"],
        vec![],
    ]);

    assert_eq!(
        repo.changed_against(DEFAULT).expect("the scripted diff"),
        ["crates/engine/src/read.rs", "crates/tui/src/panel.rs"]
    );
    // The second answer, and then the last one repeating.
    for _ in 0..2 {
        assert!(
            repo.changed_against(DEFAULT)
                .expect("the scripted diff")
                .is_empty()
        );
    }

    let paths = vec![
        "crates/engine/WARLOCK.md".to_owned(),
        ".warlock/pacts.toml".to_owned(),
    ];
    let message = format!("{TICKET}: refresh WARLOCK.md");
    repo.commit_paths(&message, &paths)
        .expect("the commit is made");

    assert_eq!(
        repo.calls(),
        vec![
            GitCall::ChangedAgainst(DEFAULT.to_owned()),
            GitCall::ChangedAgainst(DEFAULT.to_owned()),
            GitCall::ChangedAgainst(DEFAULT.to_owned()),
            GitCall::CommitPaths {
                message: message.clone(),
                paths,
            },
        ]
    );
    // Written down as a commit like any other, so a halt asserted to have
    // committed nothing still means what it says.
    assert_eq!(repo.commits(), std::slice::from_ref(&message));

    // And a commit of nothing is refused here as `Git` refuses it, rather than
    // recorded as a commit that swept whatever was staged.
    let error = repo
        .commit_paths(&message, &[])
        .expect_err("a commit of no paths is not a commit");
    assert!(matches!(error, GitError::Empty { .. }), "{error:?}");
}

const REVIEWED: &str =
    "every issue in `A brief` is in review or closed, so the project moved to `In Review`";

fn reviewing(project: IssueProject) -> (Boarding, Vec<PullEvent>) {
    let ground = Ground::new();
    let board = Boarding::filing("").in_project(project);
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, events) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    assert!(pulled.is_ok(), "{pulled:?}");
    (board, events)
}

#[test]
fn the_last_ticket_of_a_drafted_project_into_review_moves_the_project() {
    let (board, events) = reviewing(IssueProject::new(
        "In Progress",
        &[
            ("In Review", "started"),
            ("Done", "completed"),
            ("Won't do", "canceled"),
        ],
    ));

    assert_eq!(
        board.moves(),
        [("project-1".to_owned(), "status-in-progress".to_owned())]
    );
    assert!(
        board
            .calls()
            .contains(&Call::ProjectStatus(REVIEW.to_owned())),
        "the project status is the scope record's review state"
    );
    assert_eq!(
        events.last(),
        Some(&PullEvent::Project {
            line: REVIEWED.to_owned()
        })
    );
}

#[test]
fn a_project_with_an_issue_still_open_stays_where_it_is() {
    let (board, events) = reviewing(IssueProject::new(
        "In Progress",
        &[("In Review", "started"), ("In Progress", "started")],
    ));

    assert!(board.moves().is_empty());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PullEvent::Project { .. }))
    );
}

#[test]
fn a_project_still_being_drafted_stays_planned_whatever_its_issues_are() {
    let (board, events) = reviewing(IssueProject::new("Planned", &[("In Review", "started")]));

    assert!(board.moves().is_empty());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PullEvent::Project { .. }))
    );
}

#[test]
fn a_workspace_with_no_review_project_status_is_a_line_and_the_pull_still_finishes() {
    let ground = Ground::new();
    let board = Boarding::filing("")
        .in_project(IssueProject::new(
            "In Progress",
            &[("In Review", "started")],
        ))
        .without_project_status();
    let forge = Forging::opening(URL);
    let (repo, split, sessions) = two_sub_tasks();

    let (pulled, events) = pull(&ground, &board, &repo, &forge, &split, &sessions);

    assert!(pulled.is_ok(), "{pulled:?}");
    assert!(board.moves().is_empty());
    assert!(
        matches!(events.last(), Some(PullEvent::Project { line }) if line.contains("no project status called `In Review`")),
        "{events:?}"
    );
}
