use crate::StateType;

use super::{
    Blocker, IN_PROGRESS, Priority, PullRun, Queue, QueuedIssue, Reason, RunStatus,
    blocking_counts, choose, number_in, order_of,
};

// Enough of an issue to choose between: the id is never matched on here, and the
// title is only ever printed, so both are derived from the identifier and the
// facts the rules read are the parameters.
fn issue(identifier: &str, state: &str, priority: Priority, blockers: Vec<Blocker>) -> QueuedIssue {
    QueuedIssue::new(
        format!("id-{identifier}"),
        identifier,
        format!("The work of {identifier}"),
        state,
        // The queue's own state type is never read by choosing: the query
        // already dropped everything settled, and `In Progress` and the review
        // state are matched by name.
        StateType::new("started"),
        priority,
        blockers,
    )
}

fn todo(identifier: &str, priority: Priority) -> QueuedIssue {
    issue(identifier, "Todo", priority, Vec::new())
}

fn open(identifier: &str, assignee: Option<&str>) -> Blocker {
    Blocker::new(identifier, assignee, StateType::new("unstarted"))
}

fn settled(identifier: &str, state_type: &str) -> Blocker {
    Blocker::new(identifier, Some("Someone Else"), StateType::new(state_type))
}

fn run(ticket: &str, status: RunStatus) -> PullRun {
    let mut run = PullRun::new(
        ticket,
        "The work of that ticket",
        "warlock-team",
        format!("war-{}/the-work", ticket.to_lowercase()),
        "2026-09-27T09:00:00Z",
    );
    run.set_status(status);
    run
}

fn queue_of(issues: Vec<QueuedIssue>) -> Queue {
    Queue::new(issues, false)
}

fn taken(queue: &Queue, review_state: &str, runs: &[PullRun]) -> Option<String> {
    choose(queue, review_state, runs)
        .taken()
        .map(|issue| issue.identifier().to_owned())
}

fn reasons(queue: &Queue, review_state: &str, runs: &[PullRun]) -> Vec<(String, String)> {
    choose(queue, review_state, runs)
        .skipped()
        .iter()
        .map(|skipped| {
            (
                skipped.issue().identifier().to_owned(),
                skipped.reason().to_string(),
            )
        })
        .collect()
}

// The order a queue is sorted in, which `choose` only ever shows the first of.
// Taken from the same two functions it uses, so the assertion below is about the
// ordering rather than about a copy of it.
fn order(queue: &Queue) -> Vec<String> {
    let blocks = blocking_counts(queue);
    let mut issues: Vec<&QueuedIssue> = queue.issues().iter().collect();
    issues.sort_by_key(|issue| order_of(issue, &blocks));
    issues
        .iter()
        .map(|issue| issue.identifier().to_owned())
        .collect()
}

#[test]
fn an_empty_queue_chooses_nothing_and_skips_nothing() {
    let chosen = choose(&queue_of(Vec::new()), "In Review", &[]);

    assert_eq!(chosen.taken(), None);
    assert!(chosen.skipped().is_empty());
}

#[test]
fn the_first_issue_in_the_order_is_taken_and_the_rest_are_not_reported_as_skipped() {
    let queue = queue_of(vec![
        todo("WAR-10", Priority::Low),
        todo("WAR-11", Priority::Urgent),
    ]);

    let chosen = choose(&queue, "In Review", &[]);

    assert_eq!(chosen.taken().map(QueuedIssue::identifier), Some("WAR-11"));
    // A ready issue that merely lost the ordering is not a skip: the reasons are
    // the four a person can act on.
    assert!(chosen.skipped().is_empty());
}

#[test]
fn an_issue_is_ready_when_every_blocker_is_completed_or_canceled() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        "Todo",
        Priority::Medium,
        vec![settled("WAR-1", "completed"), settled("WAR-2", "canceled")],
    )]);

    assert_eq!(taken(&queue, "In Review", &[]).as_deref(), Some("WAR-10"));
    assert!(reasons(&queue, "In Review", &[]).is_empty());
}

#[test]
fn a_blocker_holds_an_issue_up_whoever_owns_it_and_the_reason_names_each_open_one() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        "Todo",
        Priority::Medium,
        vec![
            settled("WAR-1", "completed"),
            open("WAR-2", Some("Someone Else")),
            open("WAR-3", None),
        ],
    )]);

    assert_eq!(taken(&queue, "In Review", &[]), None);
    assert_eq!(
        reasons(&queue, "In Review", &[]),
        [(
            "WAR-10".to_owned(),
            "blocked by WAR-2 (Someone Else), WAR-3 (unassigned)".to_owned()
        )]
    );
}

#[test]
fn a_blocker_in_a_state_type_nobody_has_heard_of_is_still_in_the_way() {
    // The six types Linear has today are not a promise, and a seventh is not a
    // reason to call an issue ready.
    let queue = queue_of(vec![issue(
        "WAR-10",
        "Todo",
        Priority::Medium,
        vec![settled("WAR-1", "parked")],
    )]);

    assert_eq!(taken(&queue, "In Review", &[]), None);
}

#[test]
fn an_issue_in_the_records_review_state_is_never_taken() {
    let queue = queue_of(vec![
        issue("WAR-10", "In Review", Priority::Urgent, Vec::new()),
        todo("WAR-11", Priority::None),
    ]);

    assert_eq!(taken(&queue, "In Review", &[]).as_deref(), Some("WAR-11"));
    assert_eq!(
        reasons(&queue, "In Review", &[]),
        [(
            "WAR-10".to_owned(),
            "in `In Review`, which is waiting on a human".to_owned()
        )]
    );
}

#[test]
fn the_review_state_and_in_progress_are_matched_trimmed_and_case_insensitively() {
    let queue = queue_of(vec![
        issue("WAR-10", " in review ", Priority::Urgent, Vec::new()),
        issue("WAR-11", "IN PROGRESS", Priority::Urgent, Vec::new()),
    ]);

    assert_eq!(taken(&queue, "  In Review", &[]), None);
    assert_eq!(
        reasons(&queue, "  In Review", &[])
            .iter()
            .map(|(identifier, _)| identifier.as_str())
            .collect::<Vec<_>>(),
        ["WAR-10", "WAR-11"]
    );
    // And the state prints as the board spells it, not as the record does.
    assert_eq!(
        reasons(&queue, "  In Review", &[])[0].1,
        "in ` in review `, which is waiting on a human"
    );
}

#[test]
fn an_issue_in_progress_with_no_run_record_here_is_in_progress_elsewhere() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        IN_PROGRESS,
        Priority::Urgent,
        Vec::new(),
    )]);

    assert_eq!(taken(&queue, "In Review", &[]), None);
    assert_eq!(
        reasons(&queue, "In Review", &[]),
        [(
            "WAR-10".to_owned(),
            "in progress elsewhere — this machine holds no run record for it".to_owned()
        )]
    );
}

#[test]
fn an_issue_in_progress_is_taken_when_this_machine_holds_a_run_record_for_it() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        IN_PROGRESS,
        Priority::Low,
        Vec::new(),
    )]);

    for status in [
        RunStatus::Pulled,
        RunStatus::InProgress,
        RunStatus::Resumed,
        RunStatus::InReview,
    ] {
        let runs = [run("WAR-10", status)];

        assert_eq!(taken(&queue, "In Review", &runs).as_deref(), Some("WAR-10"));
        assert!(reasons(&queue, "In Review", &runs).is_empty());
    }
}

#[test]
fn a_record_for_a_different_ticket_does_not_release_an_issue_in_progress() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        IN_PROGRESS,
        Priority::Low,
        Vec::new(),
    )]);
    let runs = [run("WAR-11", RunStatus::InProgress)];

    assert_eq!(taken(&queue, "In Review", &runs), None);
}

#[test]
fn a_halted_run_is_skipped_with_the_command_that_releases_it() {
    let queue = queue_of(vec![
        issue("WAR-10", IN_PROGRESS, Priority::Urgent, Vec::new()),
        todo("WAR-11", Priority::None),
    ]);
    let runs = [run("WAR-10", RunStatus::Halted)];

    assert_eq!(taken(&queue, "In Review", &runs).as_deref(), Some("WAR-11"));
    assert_eq!(
        reasons(&queue, "In Review", &runs),
        [(
            "WAR-10".to_owned(),
            "halted — `warlock resume WAR-10` releases it".to_owned()
        )]
    );
}

#[test]
fn halted_is_named_as_halted_rather_than_as_blocked_or_in_progress_elsewhere() {
    // Its ticket is in `In Progress` and something is in its way, and neither is
    // the thing to do about it.
    let queue = queue_of(vec![issue(
        "WAR-10",
        IN_PROGRESS,
        Priority::Urgent,
        vec![open("WAR-2", Some("Someone Else"))],
    )]);
    let runs = [run("WAR-10", RunStatus::Halted)];

    assert_eq!(
        reasons(&queue, "In Review", &runs)[0].1,
        "halted — `warlock resume WAR-10` releases it"
    );
}

#[test]
fn an_issue_both_blocked_and_in_review_reads_as_in_review() {
    let queue = queue_of(vec![issue(
        "WAR-10",
        "In Review",
        Priority::Urgent,
        vec![open("WAR-2", None)],
    )]);

    assert_eq!(
        reasons(&queue, "In Review", &[])[0].1,
        "in `In Review`, which is waiting on a human"
    );
}

#[test]
fn a_resumed_run_is_taken_ahead_of_any_issue_with_no_record() {
    let queue = queue_of(vec![
        // Urgent, first in the queue, and nothing has been started on it.
        todo("WAR-1", Priority::Urgent),
        issue("WAR-10", IN_PROGRESS, Priority::None, Vec::new()),
    ]);
    let runs = [run("WAR-10", RunStatus::Resumed)];

    assert_eq!(taken(&queue, "In Review", &runs).as_deref(), Some("WAR-10"));
}

#[test]
fn several_resumed_runs_come_out_in_the_queues_order() {
    let queue = queue_of(vec![
        issue("WAR-30", IN_PROGRESS, Priority::None, Vec::new()),
        issue("WAR-10", IN_PROGRESS, Priority::Urgent, Vec::new()),
        todo("WAR-1", Priority::Urgent),
    ]);
    let runs = [
        run("WAR-10", RunStatus::Resumed),
        run("WAR-30", RunStatus::Resumed),
    ];

    // The queue's order, not the ordering's: nothing here knows which of the two
    // was resumed first, and `WAR-30` is the one the board handed over first.
    assert_eq!(taken(&queue, "In Review", &runs).as_deref(), Some("WAR-30"));
}

#[test]
fn a_resumed_run_that_is_blocked_or_in_review_is_still_skipped() {
    let blocked = queue_of(vec![issue(
        "WAR-10",
        IN_PROGRESS,
        Priority::Urgent,
        vec![open("WAR-2", None)],
    )]);
    let reviewing = queue_of(vec![issue(
        "WAR-10",
        "In Review",
        Priority::Urgent,
        Vec::new(),
    )]);
    let runs = [run("WAR-10", RunStatus::Resumed)];

    assert_eq!(taken(&blocked, "In Review", &runs), None);
    assert_eq!(taken(&reviewing, "In Review", &runs), None);
}

#[test]
fn a_run_record_for_a_ticket_that_left_the_queue_changes_nothing() {
    // A halted run whose ticket somebody closed on the board: the queue is what
    // is chosen from, so there is nothing to skip and nothing to say.
    let queue = queue_of(vec![todo("WAR-11", Priority::Low)]);
    let runs = [run("WAR-99", RunStatus::Halted)];

    assert_eq!(taken(&queue, "In Review", &runs).as_deref(), Some("WAR-11"));
    assert!(reasons(&queue, "In Review", &runs).is_empty());
}

#[test]
fn ready_issues_are_ordered_by_priority_then_by_how_much_they_block_then_by_number() {
    let queue = queue_of(vec![
        todo("WAR-10", Priority::High),
        todo("WAR-9", Priority::High),
        // Two issues in the queue are held up by WAR-40, which is what puts it
        // ahead of the other two highs.
        todo("WAR-40", Priority::High),
        issue(
            "WAR-41",
            "Todo",
            Priority::Low,
            vec![open("WAR-40", Some("Cole"))],
        ),
        issue(
            "WAR-42",
            "Todo",
            Priority::Low,
            vec![open("WAR-40", Some("Cole"))],
        ),
        todo("WAR-2", Priority::Urgent),
        todo("WAR-3", Priority::Medium),
        todo("WAR-4", Priority::None),
    ]);

    assert_eq!(
        order(&queue),
        [
            "WAR-2", "WAR-40", "WAR-9", "WAR-10", "WAR-3", "WAR-41", "WAR-42", "WAR-4"
        ]
    );
    // And the head of that order is what `choose` takes.
    assert_eq!(taken(&queue, "In Review", &[]).as_deref(), Some("WAR-2"));
}

#[test]
fn an_identifier_whose_suffix_is_not_a_number_is_ordered_last_rather_than_refused() {
    let queue = queue_of(vec![
        todo("SUPPORT-CHORE", Priority::Medium),
        todo("WAR-10", Priority::Medium),
        todo("NO_DASH", Priority::Medium),
        todo("WAR-9", Priority::Medium),
    ]);

    assert_eq!(number_in("WAR-9"), Some(9));
    assert_eq!(number_in("SUPPORT-CHORE"), None);
    assert_eq!(number_in("NO_DASH"), None);
    // The two without a number are ordered after the two with one, and between
    // themselves by their text, so the order is total and the same every run.
    assert_eq!(
        order(&queue),
        ["WAR-9", "WAR-10", "NO_DASH", "SUPPORT-CHORE"]
    );
}

#[test]
fn every_skip_in_a_queue_with_nothing_ready_is_reported_in_the_queues_order() {
    let queue = queue_of(vec![
        issue(
            "WAR-1",
            "Todo",
            Priority::Urgent,
            vec![open("WAR-100", Some("Someone Else"))],
        ),
        issue("WAR-2", "In Review", Priority::Urgent, Vec::new()),
        issue("WAR-3", IN_PROGRESS, Priority::Urgent, Vec::new()),
        issue("WAR-4", IN_PROGRESS, Priority::Urgent, Vec::new()),
    ]);
    let runs = [run("WAR-4", RunStatus::Halted)];

    let chosen = choose(&queue, "In Review", &runs);

    assert_eq!(chosen.taken(), None);
    assert_eq!(
        reasons(&queue, "In Review", &runs),
        [
            (
                "WAR-1".to_owned(),
                "blocked by WAR-100 (Someone Else)".to_owned()
            ),
            (
                "WAR-2".to_owned(),
                "in `In Review`, which is waiting on a human".to_owned()
            ),
            (
                "WAR-3".to_owned(),
                "in progress elsewhere — this machine holds no run record for it".to_owned()
            ),
            (
                "WAR-4".to_owned(),
                "halted — `warlock resume WAR-4` releases it".to_owned()
            ),
        ]
    );
    assert!(matches!(
        chosen.skipped()[0].reason(),
        Reason::Blocked { blockers } if blockers.len() == 1
    ));
}
