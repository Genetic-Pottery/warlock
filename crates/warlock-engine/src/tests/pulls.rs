use std::path::Path;

use super::{
    Error, PullRun, PullSubtask, ReasonMissing, RunStatus, SubtaskStatus, brief_path, pulls_dir,
    run_dir, run_manifest_path, state_path,
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
