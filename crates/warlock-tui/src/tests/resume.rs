use std::fs;
use std::path::Path;

use tempfile::TempDir;
use warlock_engine::{
    PullRun, PullSubtask, RunStatus, SubtaskStatus, pulls_dir, run_dir, run_manifest_path,
    state_path,
};

use super::resumed;
use crate::error::Error;
use crate::status_for;

const TICKET: &str = "WAR-142";

const SCOPE: &str = "warlock-team";

const TITLE: &str = "Add `warlock resume <TICKET>`";

const BRANCH: &str = "war-142/add-warlock-resume";

const FAILED: &str = "`cargo test` came back red";

const BLOCKED: &str = "the Linear key for `control-plane` is not on this machine";

const CROSSED: &str = "wrote `crates/control/src/lib.rs`";

// The one file the repository holds, so "nothing inside the repository is
// written" is a listing compared against something rather than against emptiness.
const MARKER: &str = "README.md";

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// A temporary home and a temporary checkout, and nothing else: a resume opens no
/// socket, runs no subprocess and reads no manifest, so there is no sigil, no key
/// and no `[[scope]]` record to write here. Every test that succeeds below is a
/// test that a scope was never asked about.
struct Ground {
    home: TempDir,
    root: TempDir,
}

impl Ground {
    fn new() -> Self {
        let root = a_dir();
        fs::write(root.path().join(MARKER), "# A checkout\n").expect("a file in the checkout");
        Self {
            home: a_dir(),
            root,
        }
    }

    /// The whole subcommand, less the environment: what `resume` does once it has
    /// resolved a repository and a home.
    fn resume(&self, failed_only: bool) -> (Result<(), Error>, String) {
        let mut out = Vec::new();
        let outcome = resumed(
            self.home.path(),
            self.root.path(),
            TICKET,
            failed_only,
            &mut out,
        );
        (
            outcome,
            String::from_utf8(out).expect("warlock writes its own text"),
        )
    }

    fn saved(&self) -> PullRun {
        PullRun::load(self.home.path(), self.root.path(), TICKET)
            .expect("the record this test wrote")
    }

    fn state(&self) -> String {
        fs::read_to_string(state_path(self.home.path(), self.root.path(), TICKET))
            .expect("the record this test wrote")
    }

    fn manifest(&self) -> String {
        fs::read_to_string(run_manifest_path(
            self.home.path(),
            self.root.path(),
            TICKET,
        ))
        .expect("a manifest is rendered on every save")
    }

    /// What the checkout holds, which a resume never adds to: the run record lives
    /// under the home.
    fn in_the_repository(&self) -> Vec<String> {
        listing(self.root.path())
    }
}

fn listing(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("a directory that lists")
        .map(|entry| {
            entry
                .expect("an entry that reads")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// A halted run with one of each sub-task status, saved under this test's home:
/// the `done` and `pending` ones are there so that every assertion about what a
/// resume leaves alone has something to be about.
fn halted(ground: &Ground) -> PullRun {
    let mut run = a_run();
    run.set_status(RunStatus::Halted);
    stopped(
        &mut run,
        "WAR-142.02",
        SubtaskStatus::Failed(FAILED.to_owned()),
    );
    stopped(
        &mut run,
        "WAR-142.03",
        SubtaskStatus::Blocked(BLOCKED.to_owned()),
    );
    stopped(
        &mut run,
        "WAR-142.04",
        SubtaskStatus::Crossed(CROSSED.to_owned()),
    );
    stopped(&mut run, "WAR-142.01", SubtaskStatus::Done);
    saving(ground, &run);
    run
}

fn a_run() -> PullRun {
    PullRun::new(TICKET, TITLE, SCOPE, BRANCH, "2026-09-28T09:00:00+00:00").with_subtasks([
        PullSubtask::new("WAR-142.01", "The engine's reset", [] as [&str; 0]),
        PullSubtask::new("WAR-142.02", "The subcommand", ["WAR-142.01"]),
        PullSubtask::new("WAR-142.03", "The panel's `/resume`", ["WAR-142.02"]),
        PullSubtask::new("WAR-142.04", "The documentation", ["WAR-142.02"]),
    ])
}

fn stopped(run: &mut PullRun, id: &str, status: SubtaskStatus) {
    run.subtask_mut(id)
        .expect("a sub-task this test wrote")
        .set_status(status);
}

fn saving(ground: &Ground, run: &PullRun) {
    run.save(ground.home.path(), ground.root.path())
        .expect("a record that writes");
}

fn status_of(run: &PullRun, id: &str) -> SubtaskStatus {
    run.subtask(id)
        .expect("a sub-task this test wrote")
        .status()
        .clone()
}

#[test]
fn a_halt_is_released_whole_and_every_change_is_printed_with_the_status_it_had() {
    let ground = Ground::new();
    halted(&ground);

    let (outcome, printed) = ground.resume(false);

    outcome.expect("there are three sub-tasks to put back");
    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::Resumed);
    for id in ["WAR-142.02", "WAR-142.03", "WAR-142.04"] {
        assert_eq!(status_of(&saved, id), SubtaskStatus::Pending, "{id}");
    }
    // The two a resume never touches, whatever the mode.
    assert_eq!(status_of(&saved, "WAR-142.01"), SubtaskStatus::Done);

    // One line per change, each naming the sub-task, the status it had and the
    // reason that status carried — which the reset dropped from the record, so
    // this line is the only surviving copy.
    assert!(
        printed.contains("warlock: `WAR-142.02` was `failed` and is `pending` again — `cargo test` came back red\n"),
        "{printed}"
    );
    assert!(printed.contains("`WAR-142.03` was `blocked`"), "{printed}");
    assert!(printed.contains(BLOCKED), "{printed}");
    assert!(printed.contains("`WAR-142.04` was `crossed`"), "{printed}");
    assert!(printed.contains(CROSSED), "{printed}");
    // Nothing about the sub-tasks it left as they were: a resume reports what it
    // changed.
    assert!(!printed.contains("WAR-142.01"), "{printed}");

    // The hand-off, with the scope read out of the record rather than typed.
    assert!(
        printed.ends_with(&format!(
            "warlock: `warlock pull {SCOPE} --ticket {TICKET}` works the ticket again\n"
        )),
        "{printed}"
    );
    assert_eq!(status_for(&Ok(())), 0);
}

#[test]
fn failed_only_puts_the_failure_back_and_leaves_the_blocker_and_its_reason() {
    let ground = Ground::new();
    halted(&ground);

    let (outcome, printed) = ground.resume(true);

    outcome.expect("there is a failed sub-task to put back");
    let saved = ground.saved();
    assert_eq!(saved.status(), RunStatus::Resumed);
    assert_eq!(status_of(&saved, "WAR-142.02"), SubtaskStatus::Pending);
    // Both blockers keep their status *and* their reason: what `--failed-only` is
    // for is the run where one of the two is still in the way.
    assert_eq!(
        status_of(&saved, "WAR-142.03"),
        SubtaskStatus::Blocked(BLOCKED.to_owned())
    );
    assert_eq!(
        status_of(&saved, "WAR-142.04"),
        SubtaskStatus::Crossed(CROSSED.to_owned())
    );

    assert!(printed.contains("`WAR-142.02` was `failed`"), "{printed}");
    assert!(!printed.contains("WAR-142.03"), "{printed}");
    assert!(!printed.contains("WAR-142.04"), "{printed}");
    // The reason a blocker kept is on no line here, because nothing about it
    // changed.
    assert!(!printed.contains(BLOCKED), "{printed}");
}

#[test]
fn a_release_writes_the_record_and_nothing_inside_the_repository() {
    let ground = Ground::new();
    halted(&ground);
    let before = ground.in_the_repository();

    ground.resume(false).0.expect("a halt with work to release");

    assert_eq!(ground.in_the_repository(), before);
    assert_eq!(before, vec![MARKER.to_owned()]);
    // `manifest.md` is re-rendered from the record it was saved beside, which is
    // the whole of what a reader looking at the run by hand sees change.
    let manifest = ground.manifest();
    assert!(manifest.contains("- status: `resumed`\n"), "{manifest}");
    assert!(
        manifest.contains("- [ ] `WAR-142.02` The subcommand\n"),
        "{manifest}"
    );
    // The record's own copy of every reason is gone, in the file as well as in
    // memory: a `pending` sub-task carrying why it stopped last time reads as one
    // that is still stopped. The surviving copy is the line that was printed above.
    let state = ground.state();
    assert!(state.contains("\"status\": \"resumed\""), "{state}");
    for reason in [FAILED, BLOCKED, CROSSED] {
        assert!(!state.contains(reason), "{state}");
    }
}

#[test]
fn a_ticket_with_no_run_record_names_the_directory_that_was_looked_in() {
    let ground = Ground::new();

    let (outcome, printed) = ground.resume(false);

    let error = outcome.expect_err("this machine has never pulled the ticket");
    let looked = run_dir(ground.home.path(), ground.root.path(), TICKET);
    assert!(
        matches!(&error, Error::NoRun { ticket, directory }
            if ticket == TICKET && directory == &looked),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(said.contains(&looked.display().to_string()), "{said}");
    assert!(said.contains("warlock pull"), "{said}");
    // The ordinary refusal and not the boundary's: a resume has no scope to fail.
    assert_eq!(status_for(&Err(error)), 1);

    // Nothing written, and nothing printed: the refusal is `main`'s one line on
    // stderr.
    assert!(printed.is_empty(), "{printed}");
    assert!(!pulls_dir(ground.home.path(), ground.root.path()).exists());
    assert_eq!(ground.in_the_repository(), vec![MARKER.to_owned()]);
}

#[test]
fn a_run_with_nothing_to_release_is_refused_with_the_record_untouched() {
    let ground = Ground::new();
    let mut run = a_run();
    run.set_status(RunStatus::InReview);
    stopped(&mut run, "WAR-142.01", SubtaskStatus::Done);
    saving(&ground, &run);
    let before = ground.state();

    let (outcome, printed) = ground.resume(false);

    let error = outcome.expect_err("a finished run has nothing to put back");
    assert!(
        matches!(&error, Error::NothingToResume { ticket, status, failed_only }
            if ticket == TICKET && *status == RunStatus::InReview && !failed_only),
        "{error:?}"
    );
    // The status the record was holding when it was read, which is the answer: the
    // run finished rather than stopping.
    let said = error.to_string();
    assert!(said.contains("`in_review`"), "{said}");
    assert_eq!(status_for(&Err(error)), 1);

    assert!(printed.is_empty(), "{printed}");
    // Byte-identical, which is the promise: the reset lives in memory until it is
    // saved, and a refusal never saves.
    assert_eq!(ground.state(), before);
}

#[test]
fn failed_only_over_a_run_stopped_by_a_blocker_alone_says_which_mode_was_asked() {
    let ground = Ground::new();
    let mut run = a_run();
    run.set_status(RunStatus::Halted);
    stopped(
        &mut run,
        "WAR-142.03",
        SubtaskStatus::Blocked(BLOCKED.to_owned()),
    );
    saving(&ground, &run);
    let before = ground.state();

    let error = ground
        .resume(true)
        .0
        .expect_err("`--failed-only` releases no blocker");

    let said = error.to_string();
    assert!(said.contains("`halted`"), "{said}");
    // The flag is half of why there was nothing to put back, so the line says so
    // rather than leaving the reader to read the record for it.
    assert!(said.contains("--failed-only"), "{said}");
    assert_eq!(ground.state(), before);
}

#[test]
fn a_record_that_will_not_read_is_not_a_ticket_nobody_pulled() {
    let ground = Ground::new();
    halted(&ground);
    fs::write(
        state_path(ground.home.path(), ground.root.path(), TICKET),
        "{ this is not the record warlock wrote",
    )
    .expect("a record this test broke on purpose");

    let error = ground
        .resume(false)
        .0
        .expect_err("a record that will not parse");

    // Not `NoRun`: a record broken by a hand edit describes a branch that may be
    // holding uncommitted work, and telling the operator there is no run would
    // send them to start the ticket again.
    assert!(matches!(&error, Error::Runs { .. }), "{error:?}");
    assert_eq!(status_for(&Err(error)), 1);
}
