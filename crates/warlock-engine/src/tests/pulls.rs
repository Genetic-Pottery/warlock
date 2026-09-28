use std::path::Path;

use crate::splitting;

use super::{
    Error, PullRun, PullSubtask, ReasonMissing, Reset, ResetMode, RunStatus, SubtaskStatus,
    brief_path, halted_and_resumed_runs, pulls_dir, run_dir, run_manifest_path, state_path,
};

const WAR_124: &str = r#"{
  "ticket": "WAR-124",
  "title": "Draft a slice's tickets in one per-slice claude session",
  "scope": "warlock-team",
  "status": "in_review",
  "branch": "war-124/draft-a-slice-s-tickets-in-one-per-slice-claude",
  "pulled_at": "2026-09-21T13:57:42+00:00",
  "pr_url": "https://github.com/Genetic-Pottery/warlock/pull/132",
  "subtasks": [
    {
      "id": "WAR-124.01",
      "goal": "Add the drafting session's spawn surface",
      "status": "done",
      "depends_on": [],
      "blocked_reason": null,
      "log": "Added the per-slice drafting spawn surface.",
      "started_at": "2026-09-21T14:00:21+00:00",
      "finished_at": "2026-09-21T14:06:17+00:00",
      "session_id": "5b80b301-8d38-451e-9f55-0034e0877152",
      "cost_usd": 2.384245
    },
    {
      "id": "WAR-124.02",
      "goal": "Drive the interactive session",
      "status": "blocked",
      "depends_on": [
        "WAR-124.01"
      ],
      "blocked_reason": "`crates/control` is scoped control-plane",
      "log": null,
      "started_at": "2026-09-21T14:06:17+00:00",
      "finished_at": null,
      "session_id": "6b2fbcbf-0b42-4a15-8df6-d0bf88018009",
      "cost_usd": 0.5309135
    }
  ]
}"#;

fn a_run() -> PullRun {
    PullRun::new(
        "WAR-140",
        "The Linear queue query",
        "warlock-team",
        "war-140/the-linear-queue-query",
        "2026-09-27T06:21:55+00:00",
    )
}

fn a_subtask(id: &str) -> PullSubtask {
    PullSubtask::new(id, "A goal", [] as [&str; 0])
}

// A split of `WAR-140` as a pass would have answered it — two sub-tasks, the
// second waiting on the first, every brief slot filled — ordered, numbered and
// turned into the record's own sub-tasks. Built through `splitting::number`
// rather than by hand, so a test of the record is a test of what the split
// actually hands it.
fn a_split() -> Vec<PullSubtask> {
    subtasks(&splitting::Fill {
        subtasks: vec![
            splitting::Subtask {
                goal: "Add the issues query".to_owned(),
                definition_of_done: vec![
                    "`warlock pull` lists the queue".to_owned(),
                    "The query is paged".to_owned(),
                ],
                likely_files: vec!["crates/warlock-linear/src/queue.rs".to_owned()],
                test_plan: "cargo test -p warlock-linear".to_owned(),
                notes: "Read `issues.rs` first.".to_owned(),
                ..splitting::Subtask::default()
            },
            splitting::Subtask {
                goal: "Drive the query from the pull".to_owned(),
                depends_on: vec![1],
                definition_of_done: vec!["A pull names the issue it took".to_owned()],
                likely_files: vec!["crates/warlock-tui/src/pull.rs".to_owned()],
                test_plan: "cargo test -p warlock-tui".to_owned(),
                notes: "The queue order is settled; do not re-sort it.".to_owned(),
            },
        ],
    })
}

fn subtasks(fill: &splitting::Fill) -> Vec<PullSubtask> {
    splitting::number(fill, "WAR-140")
        .expect("nothing in the fill waits on itself")
        .into_iter()
        .map(PullSubtask::from)
        .collect()
}

fn subtask_json(status: &str, reason: &str) -> String {
    format!(
        r#"{{
          "id": "WAR-140.01",
          "goal": "A goal",
          "status": "{status}",
          "depends_on": [],
          "blocked_reason": {reason},
          "log": null,
          "started_at": null,
          "finished_at": null,
          "session_id": null,
          "cost_usd": null
        }}"#
    )
}

#[test]
fn a_new_run_is_pulled_with_nothing_in_it() {
    let run = a_run();

    assert_eq!(run.ticket(), "WAR-140");
    assert_eq!(run.title(), "The Linear queue query");
    assert_eq!(run.scope(), "warlock-team");
    assert_eq!(run.branch(), "war-140/the-linear-queue-query");
    assert_eq!(run.pulled_at(), "2026-09-27T06:21:55+00:00");
    assert_eq!(run.status(), RunStatus::Pulled);
    assert_eq!(run.pr_url(), None);
    assert!(run.subtasks().is_empty());
}

#[test]
fn a_record_round_trips_through_serde_json_unchanged() {
    let mut blocked = a_subtask("WAR-140.02");
    blocked.set_status(SubtaskStatus::Blocked("control-plane is closed".to_owned()));
    blocked.set_session_id("6b2fbcbf-0b42-4a15-8df6-d0bf88018009");
    blocked.set_cost_usd(0.530_913_5);
    blocked.set_started_at("2026-09-27T06:30:00+00:00");

    let mut done = PullSubtask::new("WAR-140.01", "A first goal", ["WAR-140.02"]);
    done.set_status(SubtaskStatus::Done);
    done.set_log("What the session did.");
    done.set_finished_at("2026-09-27T06:29:00+00:00");

    let mut run = a_run().with_subtasks([done, blocked]);
    run.set_status(RunStatus::Halted);
    run.set_pr_url("https://github.com/Genetic-Pottery/warlock/pull/140");

    let text = serde_json::to_string_pretty(&run).expect("a record serialises");
    let read: PullRun = serde_json::from_str(&text).expect("a record it wrote reads back");

    assert_eq!(read, run);
    assert_eq!(
        serde_json::to_string_pretty(&read).expect("a record serialises"),
        text,
    );
}

#[test]
fn the_wire_keys_are_the_ones_a_forman_state_file_spells() {
    let run: PullRun = serde_json::from_str(WAR_124).expect("the recorded shape reads");

    assert_eq!(run.status(), RunStatus::InReview);
    assert_eq!(
        run.pr_url(),
        Some("https://github.com/Genetic-Pottery/warlock/pull/132"),
    );

    let done = run.subtask("WAR-124.01").expect("the first sub-task");
    assert_eq!(done.status(), &SubtaskStatus::Done);
    assert_eq!(done.depends_on(), [] as [String; 0]);
    assert_eq!(
        done.log(),
        Some("Added the per-slice drafting spawn surface.")
    );
    assert_eq!(done.started_at(), Some("2026-09-21T14:00:21+00:00"));
    assert_eq!(done.finished_at(), Some("2026-09-21T14:06:17+00:00"));
    assert_eq!(
        done.session_id(),
        Some("5b80b301-8d38-451e-9f55-0034e0877152")
    );
    assert_eq!(done.cost_usd(), Some(2.384_245));

    let blocked = run.subtask("WAR-124.02").expect("the second sub-task");
    assert_eq!(blocked.depends_on(), ["WAR-124.01"]);
    assert_eq!(
        blocked.status(),
        &SubtaskStatus::Blocked("`crates/control` is scoped control-plane".to_owned()),
    );

    // Written back with the same keys, spelled the same way, so a record and a
    // `.forman` state file stay readable side by side.
    let text = serde_json::to_value(&run).expect("a record serialises");
    let keys: Vec<&str> = text
        .as_object()
        .expect("a record is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "ticket",
            "title",
            "scope",
            "status",
            "branch",
            "pulled_at",
            "pr_url",
            "subtasks",
        ],
    );

    let subtask_keys: Vec<&str> = text["subtasks"][0]
        .as_object()
        .expect("a sub-task is an object")
        .keys()
        .map(String::as_str)
        .collect();
    // The brief's four keys come last, after every key a record written before
    // they existed holds — `WAR_124` above is one, and it read — so the diff
    // between the two is four added lines and nothing moved.
    assert_eq!(
        subtask_keys,
        [
            "id",
            "goal",
            "status",
            "depends_on",
            "blocked_reason",
            "log",
            "started_at",
            "finished_at",
            "session_id",
            "cost_usd",
            "definition_of_done",
            "likely_files",
            "test_plan",
            "notes",
        ],
    );
}

// The record `WAR_124` holds was written before a sub-task carried a brief, and
// it is read back by this build every day of a run in progress. It has to read as
// a split that answered nothing rather than fail.
#[test]
fn a_record_written_before_the_brief_keys_existed_reads_as_a_split_that_answered_none_of_them() {
    let run: PullRun = serde_json::from_str(WAR_124).expect("the older shape still reads");

    let subtask = run.subtask("WAR-124.01").expect("the first sub-task");
    assert!(subtask.definition_of_done().is_empty());
    assert!(subtask.likely_files().is_empty());
    assert_eq!(subtask.test_plan(), None);
    assert_eq!(subtask.notes(), None);

    // And the brief rendered from it is the brief that build rendered: a goal,
    // and no heading over anything nobody answered.
    let brief = subtask.to_brief_string("WAR-124");
    assert!(
        brief.contains("\n## Goal\nAdd the drafting session's spawn surface\n"),
        "{brief}",
    );
    for heading in [
        "## Definition of done",
        "## Likely files",
        "## Test plan",
        "## Notes",
    ] {
        assert!(!brief.contains(heading), "{brief} names {heading:?}");
    }
}

#[test]
fn every_status_is_spelled_in_snake_case() {
    for (status, spelling) in [
        (RunStatus::Pulled, "pulled"),
        (RunStatus::InProgress, "in_progress"),
        (RunStatus::Halted, "halted"),
        (RunStatus::Resumed, "resumed"),
        (RunStatus::InReview, "in_review"),
    ] {
        assert_eq!(status.as_str(), spelling);
        assert_eq!(status.to_string(), spelling);
        assert_eq!(
            serde_json::to_value(status).expect("a status serialises"),
            serde_json::Value::String(spelling.to_owned()),
        );
    }

    for (status, spelling) in [
        (SubtaskStatus::Pending, "pending"),
        (SubtaskStatus::InProgress, "in_progress"),
        (SubtaskStatus::Done, "done"),
        (SubtaskStatus::Blocked("why".to_owned()), "blocked"),
        (SubtaskStatus::Failed("why".to_owned()), "failed"),
        (SubtaskStatus::Crossed("why".to_owned()), "crossed"),
    ] {
        assert_eq!(status.as_str(), spelling);

        let mut subtask = a_subtask("WAR-140.01");
        subtask.set_status(status);
        let value = serde_json::to_value(&subtask).expect("a sub-task serialises");
        assert_eq!(
            value["status"],
            serde_json::Value::String(spelling.to_owned())
        );
    }
}

#[test]
fn a_reason_carrying_status_cannot_be_read_without_its_reason() {
    for status in ["blocked", "failed", "crossed"] {
        let error = serde_json::from_str::<PullSubtask>(&subtask_json(status, "null"))
            .expect_err("a reason-carrying status with no reason is refused");
        let message = error.to_string();
        assert!(
            message.contains("WAR-140.01") && message.contains(status),
            "{message} names neither the sub-task nor `{status}`",
        );

        let read: PullSubtask = serde_json::from_str(&subtask_json(status, "\"why\""))
            .expect("the same status with a reason reads");
        assert_eq!(read.status().reason(), Some("why"));
    }
}

#[test]
fn a_reason_on_a_status_that_carries_none_is_dropped_rather_than_refused() {
    for status in ["pending", "in_progress", "done"] {
        let read: PullSubtask = serde_json::from_str(&subtask_json(status, "\"stale\""))
            .expect("a stray reason does not strand the record");
        assert_eq!(read.status().as_str(), status);
        assert_eq!(read.status().reason(), None);
        assert_eq!(
            serde_json::to_value(&read).expect("a sub-task serialises")["blocked_reason"],
            serde_json::Value::Null,
        );
    }
}

#[test]
fn a_status_displays_with_its_reason() {
    assert_eq!(
        SubtaskStatus::Crossed("wrote crates/control/src/lib.rs".to_owned()).to_string(),
        "crossed: wrote crates/control/src/lib.rs",
    );
    assert_eq!(SubtaskStatus::Done.to_string(), "done");
}

#[test]
fn the_missing_reason_error_names_the_sub_task_and_the_status() {
    let error = ReasonMissing {
        id: "WAR-140.03".to_owned(),
        status: "failed",
    };

    assert_eq!(error.id(), "WAR-140.03");
    assert_eq!(error.status(), "failed");
    assert_eq!(
        error.to_string(),
        "sub-task `WAR-140.03` is `failed` with no `blocked_reason`",
    );
}

#[test]
fn a_session_id_and_a_cost_are_recorded_for_every_outcome() {
    for status in [
        SubtaskStatus::Pending,
        SubtaskStatus::InProgress,
        SubtaskStatus::Done,
        SubtaskStatus::Blocked("why".to_owned()),
        SubtaskStatus::Failed("why".to_owned()),
        SubtaskStatus::Crossed("why".to_owned()),
    ] {
        let mut run = a_run().with_subtasks([a_subtask("WAR-140.01")]);
        let subtask = run
            .subtask_mut("WAR-140.01")
            .expect("the sub-task is there");
        subtask.set_status(status.clone());
        subtask.set_session_id("5b80b301");
        subtask.set_cost_usd(1.5);

        let read: PullRun =
            serde_json::from_str(&serde_json::to_string(&run).expect("a record serialises"))
                .expect("a record reads back");
        let subtask = read.subtask("WAR-140.01").expect("the sub-task survived");

        assert_eq!(subtask.status(), &status);
        assert_eq!(subtask.session_id(), Some("5b80b301"));
        assert_eq!(subtask.cost_usd(), Some(1.5));
    }
}

#[test]
fn sub_tasks_are_reached_by_identifier_and_ignore_a_name_no_sub_task_has() {
    let mut run = a_run().with_subtasks([a_subtask("WAR-140.01")]);
    run.push_subtask(a_subtask("WAR-140.02"));

    assert_eq!(
        run.subtasks()
            .iter()
            .map(PullSubtask::id)
            .collect::<Vec<_>>(),
        ["WAR-140.01", "WAR-140.02"],
    );
    assert!(run.subtask("WAR-140.03").is_none());
    assert!(run.subtask_mut("WAR-140.03").is_none());

    for subtask in run.subtasks_mut() {
        subtask.set_status(SubtaskStatus::Done);
    }
    assert!(
        run.subtasks()
            .iter()
            .all(|subtask| subtask.status() == &SubtaskStatus::Done)
    );
}

#[test]
fn an_unknown_key_is_refused_rather_than_ignored() {
    let text = WAR_124.replace("\"scope\"", "\"scopes\"");

    assert!(serde_json::from_str::<PullRun>(&text).is_err());
}

// A halted run carrying one sub-task in each of the six statuses, so a test of
// the reset says what it does to all of them rather than to the one the run
// happened to stop on.
fn a_halted_run() -> PullRun {
    let mut run = a_run();
    for (id, status) in [
        ("WAR-140.01", SubtaskStatus::Pending),
        ("WAR-140.02", SubtaskStatus::InProgress),
        ("WAR-140.03", SubtaskStatus::Done),
        (
            "WAR-140.04",
            SubtaskStatus::Blocked("control-plane is closed".to_owned()),
        ),
        (
            "WAR-140.05",
            SubtaskStatus::Failed("`cargo test` came back red".to_owned()),
        ),
        (
            "WAR-140.06",
            SubtaskStatus::Crossed("wrote crates/control/src/lib.rs".to_owned()),
        ),
    ] {
        let mut subtask = a_subtask(id);
        subtask.set_status(status);
        run.push_subtask(subtask);
    }
    run.set_status(RunStatus::Halted);
    run
}

fn statuses(run: &PullRun) -> Vec<String> {
    run.subtasks()
        .iter()
        .map(|subtask| format!("{} {}", subtask.id(), subtask.status()))
        .collect()
}

fn changes(changed: &[Reset]) -> Vec<String> {
    changed
        .iter()
        .map(|reset| format!("{} {}", reset.id(), reset.was()))
        .collect()
}

#[test]
fn a_resume_releases_every_stopped_sub_task_and_leaves_the_rest_alone() {
    let mut run = a_halted_run();

    let changed = run.resume(ResetMode::Everything);

    assert_eq!(
        changes(&changed),
        [
            "WAR-140.04 blocked: control-plane is closed",
            "WAR-140.05 failed: `cargo test` came back red",
            "WAR-140.06 crossed: wrote crates/control/src/lib.rs",
        ],
    );
    assert_eq!(run.status(), RunStatus::Resumed);
    assert_eq!(
        statuses(&run),
        [
            "WAR-140.01 pending",
            "WAR-140.02 in_progress",
            "WAR-140.03 done",
            "WAR-140.04 pending",
            "WAR-140.05 pending",
            "WAR-140.06 pending",
        ],
    );
}

#[test]
fn a_failed_only_resume_leaves_a_blocker_and_a_crossing_their_status_and_reason() {
    let mut run = a_halted_run();

    let changed = run.resume(ResetMode::FailedOnly);

    assert_eq!(
        changes(&changed),
        ["WAR-140.05 failed: `cargo test` came back red"]
    );
    assert_eq!(run.status(), RunStatus::Resumed);
    assert_eq!(
        statuses(&run),
        [
            "WAR-140.01 pending",
            "WAR-140.02 in_progress",
            "WAR-140.03 done",
            "WAR-140.04 blocked: control-plane is closed",
            "WAR-140.05 pending",
            "WAR-140.06 crossed: wrote crates/control/src/lib.rs",
        ],
    );
}

// The run's own status is what tells a pull whether to carry the run on, so a
// reset that released nothing must not move it: `warlock resume` refuses in that
// case, and it can only refuse if the record it holds is still the record it
// read.
#[test]
fn a_resume_with_nothing_to_release_leaves_the_run_exactly_as_it_was() {
    for status in [
        RunStatus::Pulled,
        RunStatus::InProgress,
        RunStatus::Halted,
        RunStatus::Resumed,
        RunStatus::InReview,
    ] {
        for mode in [ResetMode::Everything, ResetMode::FailedOnly] {
            let mut run = a_run().with_subtasks([a_subtask("WAR-140.01")]);
            run.subtask_mut("WAR-140.01")
                .expect("the sub-task is there")
                .set_status(SubtaskStatus::Done);
            run.set_status(status);
            let before = run.clone();

            assert!(run.resume(mode).is_empty());
            assert_eq!(run, before);
        }
    }
}

// A blocker is exactly the case `--failed-only` is for, so a run whose only halt
// is one has nothing to release in that mode and stays `halted`.
#[test]
fn a_failed_only_resume_of_a_run_that_only_blocked_releases_nothing() {
    let mut run = a_halted_run();
    run.subtask_mut("WAR-140.05")
        .expect("the failed sub-task is there")
        .set_status(SubtaskStatus::Done);

    assert!(run.resume(ResetMode::FailedOnly).is_empty());
    assert_eq!(run.status(), RunStatus::Halted);
}

// The reason is dropped from the file as well as from the type: a `pending`
// sub-task still carrying the `blocked_reason` it stopped on is a halt the
// manifest would go on reporting after the operator released it.
#[test]
fn a_released_sub_task_leaves_no_reason_behind_in_the_record() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let mut run = a_halted_run();
    run.resume(ResetMode::Everything);

    run.save(home.path(), root.path()).expect("a run saves");

    let text = std::fs::read_to_string(state_path(home.path(), root.path(), "WAR-140"))
        .expect("the record reads");
    assert_eq!(
        text.matches("\"blocked_reason\": null").count(),
        6,
        "{text}"
    );
    for reason in [
        "control-plane is closed",
        "`cargo test` came back red",
        "wrote crates/control/src/lib.rs",
    ] {
        assert!(!text.contains(reason), "{text}");
        assert!(!manifest_of(home.path(), root.path()).contains(reason));
        assert!(!brief_of(home.path(), root.path(), "WAR-140.04").contains(reason));
    }
    assert_eq!(
        PullRun::load(home.path(), root.path(), "WAR-140").expect("the record reads back"),
        run,
    );
}

// A record with something in every field, so a round trip proves the whole shape
// survives the file and not just the four keys a new run happens to fill.
fn a_worked_run() -> PullRun {
    let mut done = PullSubtask::new("WAR-140.01", "A first goal", [] as [&str; 0]);
    done.set_status(SubtaskStatus::Done);
    done.set_log("What the session did.");
    done.set_started_at("2026-09-27T06:22:00+00:00");
    done.set_finished_at("2026-09-27T06:29:00+00:00");
    done.set_session_id("5b80b301-8d38-451e-9f55-0034e0877152");
    done.set_cost_usd(2.384_245);

    let mut blocked = PullSubtask::new("WAR-140.02", "A second goal", ["WAR-140.01"]);
    blocked.set_status(SubtaskStatus::Blocked("control-plane is closed".to_owned()));
    blocked.set_started_at("2026-09-27T06:29:00+00:00");
    blocked.set_session_id("6b2fbcbf-0b42-4a15-8df6-d0bf88018009");
    blocked.set_cost_usd(0.530_913_5);

    let mut run = a_run().with_subtasks([done, blocked]);
    run.set_status(RunStatus::Halted);
    run.set_pr_url("https://github.com/Genetic-Pottery/warlock/pull/140");
    run
}

// Every path written anywhere under a directory, so a test can say what a save
// touched rather than only checking the file it expected is there.
fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(entries(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

#[test]
fn a_run_round_trips_through_a_save_and_a_load() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let run = a_worked_run();

    run.save(home.path(), root.path()).expect("a run saves");
    let read = PullRun::load(home.path(), root.path(), "WAR-140").expect("the run it wrote loads");

    assert_eq!(read, run);
}

#[test]
fn a_save_writes_the_record_the_manifest_and_a_brief_each_and_nothing_else() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );

    a_worked_run()
        .save(home.path(), root.path())
        .expect("a run saves");

    // Four files, each at its derived path, and no temporary left beside them.
    assert_eq!(
        entries(home.path()),
        [
            brief_path(home.path(), root.path(), "WAR-140", "WAR-140.01"),
            brief_path(home.path(), root.path(), "WAR-140", "WAR-140.02"),
            run_manifest_path(home.path(), root.path(), "WAR-140"),
            state_path(home.path(), root.path(), "WAR-140"),
        ],
    );
    assert_eq!(
        state_path(home.path(), root.path(), "WAR-140").parent(),
        Some(run_dir(home.path(), root.path(), "WAR-140").as_path()),
    );
    assert!(
        run_dir(home.path(), root.path(), "WAR-140")
            .starts_with(pulls_dir(home.path(), root.path()))
    );
}

// The run record is machine-local, so a save must leave the checkout it is about
// untouched: a `state.json` inside the repository would turn up in the diff of
// the commit the run is making.
#[test]
fn a_save_writes_nothing_inside_the_repository_root() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    std::fs::write(root.path().join("Cargo.toml"), "[package]\n").expect("a file in the checkout");

    a_worked_run()
        .save(home.path(), root.path())
        .expect("a run saves");

    assert_eq!(entries(root.path()), [root.path().join("Cargo.toml")]);
    assert!(
        state_path(home.path(), root.path(), "WAR-140").starts_with(home.path()),
        "the state file must sit under the home it was handed",
    );
}

#[test]
fn a_ticket_this_machine_holds_no_run_for_is_not_found_rather_than_an_empty_run() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );

    let error = PullRun::load(home.path(), root.path(), "WAR-140")
        .expect_err("a run nobody pulled is not there");

    assert!(matches!(error, Error::NotFound { .. }));
    let message = error.to_string();
    assert!(
        message.contains("WAR-140") && message.contains("state.json"),
        "{message} does not name the run it looked for",
    );
    assert!(std::error::Error::source(&error).is_none());
}

// The run belongs to the checkout, not to the machine: two clones of one
// repository can both be working the same ticket, and neither may read the
// other's record.
#[test]
fn two_checkouts_hold_their_own_run_for_the_same_ticket() {
    let (home, here, there) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("one checkout"),
        tempfile::tempdir().expect("another checkout"),
    );

    a_worked_run()
        .save(home.path(), here.path())
        .expect("a run saves");

    assert!(matches!(
        PullRun::load(home.path(), there.path(), "WAR-140"),
        Err(Error::NotFound { .. }),
    ));
    assert_ne!(
        state_path(home.path(), here.path(), "WAR-140"),
        state_path(home.path(), there.path(), "WAR-140"),
    );
}

#[test]
fn a_record_broken_by_hand_is_a_parse_error_naming_the_file() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let path = state_path(home.path(), root.path(), "WAR-140");
    std::fs::create_dir_all(path.parent().expect("the run directory"))
        .expect("the run directory is made");
    std::fs::write(&path, "{ not json").expect("a broken record");

    let error =
        PullRun::load(home.path(), root.path(), "WAR-140").expect_err("broken bytes are refused");

    assert!(matches!(error, Error::Parse { .. }));
    assert!(
        error.to_string().contains("state.json"),
        "{error} does not name the file at fault",
    );
    assert!(std::error::Error::source(&error).is_some());
}

// The reason a `blocked` sub-task cannot be read without its reason reaches the
// caller through the same variant, because `serde_json` reports it as a
// deserialisation failure.
#[test]
fn a_reason_missing_from_a_saved_record_is_reported_as_a_parse_error() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    a_worked_run()
        .save(home.path(), root.path())
        .expect("a run saves");

    let path = state_path(home.path(), root.path(), "WAR-140");
    let text = std::fs::read_to_string(&path)
        .expect("the record reads")
        .replace("\"control-plane is closed\"", "null");
    std::fs::write(&path, text).expect("the edited record is written");

    let error = PullRun::load(home.path(), root.path(), "WAR-140")
        .expect_err("a blocked sub-task with no reason is refused");

    assert!(matches!(error, Error::Parse { .. }));
    assert!(
        error.to_string().contains("WAR-140.02"),
        "{error} does not name the sub-task",
    );
}

#[test]
fn a_saved_record_is_pretty_printed_with_a_trailing_newline() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let run = a_worked_run();

    run.save(home.path(), root.path()).expect("a run saves");
    let text = std::fs::read_to_string(state_path(home.path(), root.path(), "WAR-140"))
        .expect("the record reads");

    assert_eq!(text, run.to_json_string().expect("a record serialises"));
    assert!(text.starts_with("{\n  \"ticket\": \"WAR-140\","), "{text}");
    assert!(text.ends_with("}\n"), "{text}");
}

#[test]
fn a_second_save_replaces_the_record_rather_than_appending_to_it() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let mut run = a_run();

    run.save(home.path(), root.path()).expect("a run saves");
    run.set_status(RunStatus::InProgress);
    run.push_subtask(a_subtask("WAR-140.01"));
    run.save(home.path(), root.path())
        .expect("a run saves again");

    let read = PullRun::load(home.path(), root.path(), "WAR-140").expect("the run loads");
    assert_eq!(read, run);
    assert_eq!(read.status(), RunStatus::InProgress);
    assert_eq!(
        entries(home.path()),
        [
            brief_path(home.path(), root.path(), "WAR-140", "WAR-140.01"),
            run_manifest_path(home.path(), root.path(), "WAR-140"),
            state_path(home.path(), root.path(), "WAR-140"),
        ],
    );
}

fn manifest_of(home: &Path, root: &Path) -> String {
    std::fs::read_to_string(run_manifest_path(home, root, "WAR-140")).expect("the manifest reads")
}

fn brief_of(home: &Path, root: &Path, subtask: &str) -> String {
    std::fs::read_to_string(brief_path(home, root, "WAR-140", subtask)).expect("the brief reads")
}

#[test]
fn a_save_renders_the_manifest_from_the_record() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let run = a_worked_run();

    run.save(home.path(), root.path()).expect("a run saves");
    let manifest = manifest_of(home.path(), root.path());

    assert_eq!(manifest, run.to_manifest_string());
    assert!(
        manifest.starts_with("# WAR-140: The Linear queue query\n\n"),
        "{manifest}",
    );
    for line in [
        "- status: `halted`\n",
        "- branch: `war-140/the-linear-queue-query`\n",
        "- pulled at: 2026-09-27T06:21:55+00:00\n",
        "- pull request: https://github.com/Genetic-Pottery/warlock/pull/140\n",
        "<!-- Rendered from state.json on every write. Do not edit by hand. -->\n",
        "## Sub-tasks\n",
        "- [x] `WAR-140.01` A first goal  $2.3842\n",
        "- [ ] `WAR-140.02` A second goal — blocked: control-plane is closed  $0.5309\n",
        "**Total: $2.9152**\n",
    ] {
        assert!(manifest.contains(line), "{manifest} is missing {line:?}");
    }
}

// The run's own state, not the sub-tasks': a pulled run with nothing worked yet
// has no pull request and nothing spent, and the manifest still has to read.
#[test]
fn a_manifest_of_an_unworked_run_names_the_missing_pull_request_and_totals_nothing() {
    let manifest = a_run()
        .with_subtasks([a_subtask("WAR-140.01")])
        .to_manifest_string();

    assert!(manifest.contains("- status: `pulled`\n"), "{manifest}");
    assert!(manifest.contains("- pull request: none\n"), "{manifest}");
    assert!(
        manifest.contains("- [ ] `WAR-140.01` A goal\n"),
        "{manifest}"
    );
    assert!(!manifest.contains("Total"), "{manifest}");
    assert!(!manifest.contains('$'), "{manifest}");
}

#[test]
fn a_save_writes_one_brief_per_sub_task_with_an_execution_log_to_append_under() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );

    a_worked_run()
        .save(home.path(), root.path())
        .expect("a run saves");

    let first = brief_of(home.path(), root.path(), "WAR-140.01");
    assert!(
        first.starts_with(
            "---\nsubtask_id: WAR-140.01\nparent: WAR-140\nstatus: done\ndepends_on: []\n---\n"
        ),
        "{first}",
    );
    assert!(first.contains("\n## Goal\nA first goal\n"), "{first}");
    assert!(
        first.ends_with(
            "\n---\n## Execution log\n<!-- spawn appends below this line; never edits above it -->\n"
        ),
        "{first}",
    );

    // A reason cannot be omitted from the record, so it cannot be omitted from
    // the brief either — and it is quoted, because a reason is prose.
    let second = brief_of(home.path(), root.path(), "WAR-140.02");
    assert!(second.contains("\nstatus: blocked\n"), "{second}");
    assert!(
        second.contains("\nblocked_reason: \"control-plane is closed\"\n"),
        "{second}",
    );
    assert!(second.contains("\ndepends_on: [WAR-140.01]\n"), "{second}");
}

#[test]
fn a_reason_carrying_a_colon_or_a_quote_stays_a_readable_front_matter_line() {
    let mut subtask = a_subtask("WAR-140.01");
    subtask.set_status(SubtaskStatus::Crossed(
        "wrote \"crates/control\": scope control-plane".to_owned(),
    ));

    let brief = subtask.to_brief_string("WAR-140");

    assert!(
        brief.contains("\nblocked_reason: \"wrote \\\"crates/control\\\": scope control-plane\"\n"),
        "{brief}",
    );
}

#[test]
fn a_second_save_rewrites_the_manifest_from_the_changed_record() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let mut run = a_run().with_subtasks([a_subtask("WAR-140.01")]);

    run.save(home.path(), root.path()).expect("a run saves");
    let before = manifest_of(home.path(), root.path());
    assert!(before.contains("- status: `pulled`\n"), "{before}");
    assert!(before.contains("- [ ] `WAR-140.01` A goal\n"), "{before}");

    run.set_status(RunStatus::InReview);
    run.set_pr_url("https://github.com/Genetic-Pottery/warlock/pull/140");
    let subtask = run
        .subtask_mut("WAR-140.01")
        .expect("the sub-task is there");
    subtask.set_status(SubtaskStatus::Done);
    subtask.set_cost_usd(1.25);
    run.save(home.path(), root.path())
        .expect("a run saves again");

    let after = manifest_of(home.path(), root.path());
    assert!(after.contains("- status: `in_review`\n"), "{after}");
    assert!(
        after.contains("- pull request: https://github.com/Genetic-Pottery/warlock/pull/140\n"),
        "{after}",
    );
    assert!(
        after.contains("- [x] `WAR-140.01` A goal  $1.2500\n"),
        "{after}",
    );
    assert!(after.contains("**Total: $1.2500**\n"), "{after}");
    assert!(!after.contains("`pulled`"), "{after}");

    // The brief tracks the record too, so its front matter cannot go on saying
    // `pending` about a sub-task that finished.
    let brief = brief_of(home.path(), root.path(), "WAR-140.01");
    assert!(brief.contains("\nstatus: done\n"), "{brief}");
    assert!(!brief.contains("pending"), "{brief}");
}

// The trap this module is built around: a sub-task's session appends its account
// to its own brief, and a save happens before and after every sub-task, so a
// brief re-rendered whole would delete the account of the work that just
// finished.
#[test]
fn a_log_appended_to_a_brief_survives_every_later_save() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let mut run = a_run().with_subtasks([a_subtask("WAR-140.01")]);
    run.save(home.path(), root.path()).expect("a run saves");

    let path = brief_path(home.path(), root.path(), "WAR-140", "WAR-140.01");
    let appended = format!(
        "{}\n### 2026-09-27 — done\n\n- What the session did.\n",
        std::fs::read_to_string(&path).expect("the brief reads"),
    );
    std::fs::write(&path, &appended).expect("a log is appended");

    let subtask = run
        .subtask_mut("WAR-140.01")
        .expect("the sub-task is there");
    subtask.set_status(SubtaskStatus::Done);
    run.save(home.path(), root.path())
        .expect("a run saves again");

    let brief = brief_of(home.path(), root.path(), "WAR-140.01");
    assert!(brief.contains("### 2026-09-27 — done\n"), "{brief}");
    assert!(brief.contains("- What the session did.\n"), "{brief}");
    // Carried across once, with the head re-rendered from the record above it.
    assert_eq!(brief.matches("## Execution log").count(), 1, "{brief}");
    assert_eq!(brief.matches("What the session did.").count(), 1, "{brief}");
    assert!(brief.contains("\nstatus: done\n"), "{brief}");
}

// The whole of this sub-task's job, end to end: what a split answered becomes a
// run record, and the briefs written beside it are what a fresh session is handed.
#[test]
fn a_numbered_split_is_saved_as_a_record_and_a_brief_per_sub_task() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let run = a_run().with_subtasks(a_split());

    run.save(home.path(), root.path()).expect("a run saves");

    assert!(state_path(home.path(), root.path(), "WAR-140").is_file());
    assert!(run_manifest_path(home.path(), root.path(), "WAR-140").is_file());
    // Numbered from one, one brief each, and nothing else in the directory but
    // the record and the manifest.
    for subtask in ["WAR-140.01", "WAR-140.02"] {
        assert!(
            brief_path(home.path(), root.path(), "WAR-140", subtask).is_file(),
            "no brief for {subtask}",
        );
    }
    let held = std::fs::read_dir(run_dir(home.path(), root.path(), "WAR-140"))
        .expect("the run directory reads")
        .count();
    assert_eq!(held, 4);

    // Read back, the record still holds every slot the split filled, so the next
    // save renders the same briefs.
    let read = PullRun::load(home.path(), root.path(), "WAR-140").expect("the run loads");
    assert_eq!(read, run);
    let first = read.subtask("WAR-140.01").expect("the first sub-task");
    assert_eq!(first.goal(), "Add the issues query");
    assert_eq!(first.depends_on(), [] as [String; 0]);
    assert_eq!(
        first.definition_of_done(),
        ["`warlock pull` lists the queue", "The query is paged"],
    );
    assert_eq!(first.likely_files(), ["crates/warlock-linear/src/queue.rs"]);
    assert_eq!(first.test_plan(), Some("cargo test -p warlock-linear"));
    assert_eq!(first.notes(), Some("Read `issues.rs` first."));
    assert_eq!(
        read.subtask("WAR-140.02")
            .expect("the second sub-task")
            .depends_on(),
        ["WAR-140.01"],
    );

    let brief = brief_of(home.path(), root.path(), "WAR-140.01");
    assert!(
        brief.starts_with(
            "---\nsubtask_id: WAR-140.01\nparent: WAR-140\nstatus: pending\ndepends_on: []\n---\n"
        ),
        "{brief}",
    );
    for section in [
        "\n## Goal\nAdd the issues query\n",
        "\n## Definition of done\n- `warlock pull` lists the queue\n- The query is paged\n",
        "\n## Likely files / touchpoints\n- crates/warlock-linear/src/queue.rs\n",
        "\n## Test plan\ncargo test -p warlock-linear\n",
        "\n## Notes for executor\nRead `issues.rs` first.\n",
    ] {
        assert!(brief.contains(section), "{brief} is missing {section:?}");
    }
    // The log heading is last, and the brief ends there with nothing under it:
    // the account of the work is the session's to append.
    assert!(
        brief.ends_with(
            "\n---\n## Execution log\n<!-- spawn appends below this line; never edits above it -->\n"
        ),
        "{brief}",
    );

    // And the sub-task that waits says so where a session would look for it.
    let second = brief_of(home.path(), root.path(), "WAR-140.02");
    assert!(second.contains("\ndepends_on: [WAR-140.01]\n"), "{second}");
}

// Only the goal is required of a split, so most of a brief can be absent — and
// absent has to read as nothing said rather than as nothing to do.
#[test]
fn a_slot_the_split_left_empty_is_no_heading_rather_than_an_empty_one() {
    let fill = splitting::Fill {
        subtasks: vec![splitting::Subtask {
            goal: "Add the issues query".to_owned(),
            // A blank entry, a whitespace-only test plan and an unanswered notes
            // block: three ways of saying nothing, and the brief says none of
            // them.
            likely_files: vec![String::new()],
            test_plan: "   \n ".to_owned(),
            ..splitting::Subtask::default()
        }],
    };
    let subtask = subtasks(&fill).pop().expect("one sub-task");

    // The record keeps the list it was handed — `splitting::mend` is what drops a
    // blank entry, and an unmended split is allowed here — and the brief is where
    // a blank is passed over.
    assert_eq!(subtask.likely_files(), [""]);
    assert_eq!(subtask.test_plan(), None);
    assert_eq!(subtask.notes(), None);

    let brief = subtask.to_brief_string("WAR-140");

    assert!(
        brief.contains("\n## Goal\nAdd the issues query\n"),
        "{brief}"
    );
    for heading in [
        "## Definition of done",
        "## Likely files",
        "## Test plan",
        "## Notes",
    ] {
        assert!(!brief.contains(heading), "{brief} names {heading:?}");
    }
    // Which leaves the goal running straight into the log, with no run of blank
    // lines where the headings would have been.
    assert!(
        brief.ends_with(
            "\n## Goal\nAdd the issues query\n\n---\n## Execution log\n<!-- spawn appends below \
             this line; never edits above it -->\n"
        ),
        "{brief}",
    );
}

// The brief holds two writers' work now — warlock's rendering above the line and
// the session's account below it — so the trap `a_log_appended_to_a_brief_...`
// guards has to hold with the sections there as well.
#[test]
fn a_re_save_keeps_the_appended_log_and_re_renders_the_sections_above_it() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let mut run = a_run().with_subtasks(a_split());
    run.save(home.path(), root.path()).expect("a run saves");

    let path = brief_path(home.path(), root.path(), "WAR-140", "WAR-140.01");
    let appended = format!(
        "{}\n### 2026-09-27 — done\n\n- Added the query, and the test plan ran.\n",
        std::fs::read_to_string(&path).expect("the brief reads"),
    );
    std::fs::write(&path, &appended).expect("a log is appended");

    let subtask = run
        .subtask_mut("WAR-140.01")
        .expect("the sub-task is there");
    subtask.set_status(SubtaskStatus::Done);
    run.save(home.path(), root.path())
        .expect("a run saves again");

    let brief = brief_of(home.path(), root.path(), "WAR-140.01");
    // The log crossed once, heading and marker verbatim.
    assert_eq!(brief.matches("## Execution log").count(), 1, "{brief}");
    assert_eq!(
        brief
            .matches("<!-- spawn appends below this line; never edits above it -->")
            .count(),
        1,
        "{brief}",
    );
    assert_eq!(
        brief
            .matches("Added the query, and the test plan ran.")
            .count(),
        1,
        "{brief}",
    );
    // The sections above it are rendered again from the record, once each, and
    // the status the save changed came with them.
    assert!(brief.contains("\nstatus: done\n"), "{brief}");
    for heading in [
        "## Goal",
        "## Definition of done",
        "## Likely files / touchpoints",
        "## Test plan",
        "## Notes for executor",
    ] {
        assert_eq!(brief.matches(heading).count(), 1, "{brief} — {heading:?}");
    }
}

// The four keys are read as leniently as they are written: a record hand-edited
// to a blank test plan is a record whose split said nothing about testing, and
// one this build wrote comes back byte for byte.
#[test]
fn the_brief_keys_round_trip_and_a_blank_one_reads_as_unanswered() {
    let run = a_run().with_subtasks(a_split());

    let text = serde_json::to_string_pretty(&run).expect("a record serialises");
    let read: PullRun = serde_json::from_str(&text).expect("a record it wrote reads back");

    assert_eq!(read, run);
    assert_eq!(
        serde_json::to_string_pretty(&read).expect("a record serialises"),
        text,
    );

    let edited = text.replace(
        "\"test_plan\": \"cargo test -p warlock-linear\"",
        "\"test_plan\": \"  \"",
    );
    let read: PullRun = serde_json::from_str(&edited).expect("a hand edit does not strand the run");
    assert_eq!(
        read.subtask("WAR-140.01")
            .expect("the first sub-task")
            .test_plan(),
        None,
    );
}

// `manifest.md` is a rendering and never an input: nothing reads it back, so
// whatever it says about the run cannot reach the record.
#[test]
fn a_loaded_record_is_unaffected_by_whatever_the_manifest_says() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    let run = a_worked_run();
    run.save(home.path(), root.path()).expect("a run saves");

    std::fs::write(
        run_manifest_path(home.path(), root.path(), "WAR-140"),
        "# WAR-999: Something else entirely\n\n- status: `in_review`\n",
    )
    .expect("the manifest is overwritten");

    let read = PullRun::load(home.path(), root.path(), "WAR-140").expect("the run loads");
    assert_eq!(read, run);
    assert_eq!(read.status(), RunStatus::Halted);

    // And the next save renders it back over the edit rather than keeping any of
    // it.
    read.save(home.path(), root.path())
        .expect("a run saves again");
    assert_eq!(
        manifest_of(home.path(), root.path()),
        run.to_manifest_string()
    );
}

// One saved run per line of a fixture, so a test can say what a home holds in a
// sentence instead of five statements per record.
fn saved(home: &Path, root: &Path, ticket: &str, scope: &str, status: RunStatus) {
    let mut run = PullRun::new(
        ticket,
        "A ticket",
        scope,
        format!("{}/a-ticket", ticket.to_lowercase()),
        "2026-09-27T06:21:55+00:00",
    );
    run.set_status(status);
    run.save(home, root).expect("a run saves");
}

fn tickets(runs: &[PullRun]) -> Vec<&str> {
    runs.iter().map(PullRun::ticket).collect()
}

#[test]
fn a_checkout_that_has_never_pulled_holds_no_run_and_no_open_ones() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );

    // There is no `pulls` directory to read, and neither lookup treats that as a
    // failure.
    assert!(!pulls_dir(home.path(), root.path()).exists());
    assert_eq!(
        PullRun::find(home.path(), root.path(), "WAR-140").expect("a missing run is not an error"),
        None
    );

    let found = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("a missing pulls directory is not an error");
    assert!(found.is_empty());
    assert!(found.runs().is_empty());
    assert!(found.unreadable().is_empty());
}

#[test]
fn a_ticket_this_machine_is_not_working_has_no_run_beside_the_ones_it_is() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    saved(
        home.path(),
        root.path(),
        "WAR-140",
        "warlock-team",
        RunStatus::InProgress,
    );

    assert_eq!(
        PullRun::find(home.path(), root.path(), "WAR-141").expect("a missing run is not an error"),
        None
    );

    let held = PullRun::find(home.path(), root.path(), "WAR-140")
        .expect("the run reads")
        .expect("this machine holds it");
    assert_eq!(held.ticket(), "WAR-140");
    assert_eq!(held.status(), RunStatus::InProgress);
}

// The fixture the two lookups are worth having for: two scopes, every run status,
// and one record for a ticket in neither of the statuses being asked about.
fn two_scopes(home: &Path, root: &Path) {
    saved(home, root, "WAR-140", "warlock-team", RunStatus::Pulled);
    saved(home, root, "WAR-141", "warlock-team", RunStatus::InProgress);
    saved(home, root, "WAR-142", "warlock-team", RunStatus::Halted);
    saved(home, root, "WAR-143", "warlock-team", RunStatus::Resumed);
    saved(home, root, "WAR-144", "warlock-team", RunStatus::InReview);
    saved(home, root, "WAR-9", "warlock-docs", RunStatus::Halted);
    saved(home, root, "WAR-10", "warlock-docs", RunStatus::Resumed);
    saved(home, root, "WAR-11", "warlock-docs", RunStatus::InProgress);
}

#[test]
fn the_scope_lookup_returns_the_halted_and_resumed_runs_of_that_scope_only() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    two_scopes(home.path(), root.path());

    let team = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("the scan reads the home");

    // `pulled`, `in_progress` and `in_review` are somebody else's problem: the
    // first two are a pull's to carry, and the third is waiting on a reviewer.
    assert_eq!(tickets(team.runs()), ["WAR-142", "WAR-143"]);
    assert_eq!(team.runs()[0].status(), RunStatus::Halted);
    assert_eq!(team.runs()[1].status(), RunStatus::Resumed);
    assert!(team.unreadable().is_empty());

    // The other scope's runs are in the same directory and stay out of the
    // answer, including its own halted and resumed ones.
    let docs = halted_and_resumed_runs(home.path(), root.path(), "warlock-docs")
        .expect("the scan reads the home");
    assert_eq!(tickets(docs.runs()), ["WAR-10", "WAR-9"]);

    // A scope nothing was ever pulled for reads empty rather than everything.
    assert!(
        halted_and_resumed_runs(home.path(), root.path(), "warlock-control")
            .expect("the scan reads the home")
            .is_empty()
    );
}

// Two calls over one home have to agree, whatever order the filesystem hands the
// directory over in, because a caller prints this list.
#[test]
fn the_scope_lookup_is_ordered_by_ticket_identifier() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    for ticket in ["WAR-9", "WAR-140", "WAR-10", "WAR-2"] {
        saved(
            home.path(),
            root.path(),
            ticket,
            "warlock-team",
            RunStatus::Halted,
        );
    }

    let first = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("the scan reads the home");
    let again = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("the scan reads the home");

    // Sorted by identifier as text, so `WAR-10` precedes `WAR-9`. Ordering by the
    // number in the identifier is selection's job, not this lookup's.
    assert_eq!(
        tickets(first.runs()),
        ["WAR-10", "WAR-140", "WAR-2", "WAR-9"]
    );
    assert_eq!(tickets(first.runs()), tickets(again.runs()));
}

#[test]
fn every_run_this_machine_holds_is_found_by_its_ticket() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    two_scopes(home.path(), root.path());

    for ticket in [
        "WAR-140", "WAR-141", "WAR-142", "WAR-143", "WAR-144", "WAR-9", "WAR-10", "WAR-11",
    ] {
        let held = PullRun::find(home.path(), root.path(), ticket)
            .expect("the run reads")
            .expect("this machine holds it");
        assert_eq!(held.ticket(), ticket);
    }
}

// A record broken by a hand edit must never read as a ticket nobody pulled: the
// run behind it may be halted with uncommitted work on its branch.
#[test]
fn a_malformed_record_is_an_error_from_the_ticket_lookup() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    saved(
        home.path(),
        root.path(),
        "WAR-140",
        "warlock-team",
        RunStatus::Halted,
    );
    std::fs::write(
        state_path(home.path(), root.path(), "WAR-140"),
        "{ not json at all",
    )
    .expect("the record is mangled");

    assert!(matches!(
        PullRun::find(home.path(), root.path(), "WAR-140"),
        Err(Error::Parse { .. }),
    ));
}

#[test]
fn a_malformed_record_is_named_by_the_scope_lookup_rather_than_skipped() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    two_scopes(home.path(), root.path());
    let mangled = state_path(home.path(), root.path(), "WAR-142");
    std::fs::write(&mangled, "{ not json at all").expect("the record is mangled");

    let found = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("one broken record does not fail the scan");

    // The rest of the scope still comes back, so one bad record cannot stop a
    // resumed run being taken.
    assert_eq!(tickets(found.runs()), ["WAR-143"]);
    assert!(!found.is_empty());

    // And the broken one is named, with the path to fix.
    assert_eq!(found.unreadable().len(), 1);
    let Error::Parse { path, .. } = &found.unreadable()[0] else {
        panic!(
            "a malformed record is a parse error: {:?}",
            found.unreadable()
        );
    };
    assert_eq!(path, &mangled);

    // Its scope is the field that will not parse, so it is named for whichever
    // scope asked.
    let docs = halted_and_resumed_runs(home.path(), root.path(), "warlock-docs")
        .expect("one broken record does not fail the scan");
    assert_eq!(docs.unreadable().len(), 1);
}

// A `blocked` sub-task with no reason is the same class of broken as invalid
// JSON, and `serde_json` reports it the same way.
#[test]
fn a_sub_task_missing_its_reason_is_named_by_the_scope_lookup() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    saved(
        home.path(),
        root.path(),
        "WAR-124",
        "warlock-team",
        RunStatus::Halted,
    );
    std::fs::write(
        state_path(home.path(), root.path(), "WAR-124"),
        WAR_124
            .replace("\"in_review\"", "\"halted\"")
            .replace("\"`crates/control` is scoped control-plane\"", "null"),
    )
    .expect("the record is written");

    let found = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("one broken record does not fail the scan");

    assert!(found.runs().is_empty());
    assert_eq!(found.unreadable().len(), 1);
    assert!(
        found.unreadable()[0]
            .to_string()
            .contains("no `blocked_reason`"),
        "{}",
        found.unreadable()[0],
    );
}

// Nothing under `pulls/` is trusted to be a run: a stray file, a directory
// holding no record, and a name that is not a ticket all have to leave the scan
// standing.
#[test]
fn a_name_under_pulls_that_is_not_a_run_is_passed_over() {
    let (home, root) = (
        tempfile::tempdir().expect("a temporary home"),
        tempfile::tempdir().expect("a temporary root"),
    );
    saved(
        home.path(),
        root.path(),
        "WAR-142",
        "warlock-team",
        RunStatus::Halted,
    );
    let pulls = pulls_dir(home.path(), root.path());
    std::fs::write(pulls.join("README"), "a note somebody left").expect("a stray file is written");
    std::fs::write(pulls.join(".DS_Store"), "junk").expect("a stray dotfile is written");
    std::fs::create_dir(pulls.join("WAR-999")).expect("an empty directory is created");
    std::fs::write(pulls.join("WAR-999").join("manifest.md"), "# WAR-999\n")
        .expect("a directory with no record is left");

    let found = halted_and_resumed_runs(home.path(), root.path(), "warlock-team")
        .expect("the scan reads the home");

    // Passed over in silence: none of them is a record that broke, so none is
    // worth telling the operator about.
    assert_eq!(tickets(found.runs()), ["WAR-142"]);
    assert!(found.unreadable().is_empty());

    // And a directory with no `state.json` is not a run this machine holds.
    assert_eq!(
        PullRun::find(home.path(), root.path(), "WAR-999")
            .expect("a missing record is not an error"),
        None
    );
}

// The per-checkout separation `sigils.rs` gives every path here applies to the
// lookups too: two checkouts of one repository do not see each other's runs.
#[test]
fn the_lookups_only_see_this_checkouts_runs() {
    let home = tempfile::tempdir().expect("a temporary home");
    let (here, elsewhere) = (
        tempfile::tempdir().expect("a temporary root"),
        tempfile::tempdir().expect("another temporary root"),
    );
    saved(
        home.path(),
        here.path(),
        "WAR-142",
        "warlock-team",
        RunStatus::Halted,
    );
    saved(
        home.path(),
        elsewhere.path(),
        "WAR-143",
        "warlock-team",
        RunStatus::Resumed,
    );

    let found = halted_and_resumed_runs(home.path(), here.path(), "warlock-team")
        .expect("the scan reads the home");

    assert_eq!(tickets(found.runs()), ["WAR-142"]);
    assert_eq!(
        PullRun::find(home.path(), here.path(), "WAR-143").expect("a missing run is not an error"),
        None
    );
}
