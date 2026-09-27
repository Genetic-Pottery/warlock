use warlock_engine::{PullRun, PullSubtask, SubtaskStatus};

use super::{Pulled, halt_comment, next_runnable};

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
