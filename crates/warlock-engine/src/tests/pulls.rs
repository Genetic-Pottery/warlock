use super::{PullRun, PullSubtask, ReasonMissing, RunStatus, SubtaskStatus};

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
        ],
    );
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
