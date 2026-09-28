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

// `/resume <TICKET>` typed into the composer, driven over the same temporary home
// the subcommand's tests use: a conversation with a stand-in agent, a whole `App`,
// and one call of the very function the loop calls. Nothing here opens a socket,
// raises a `claude` or reads the machine's own home, and the stand-in is written
// so that a turn started by any of it is the test failing.
mod panel {
    use std::time::Instant;

    use warlock_tui::{App, Line, Mode};

    use super::{
        BLOCKED, CROSSED, FAILED, Ground, MARKER, PullRun, RunStatus, SCOPE, SubtaskStatus, TICKET,
        a_run, halted, saving, state_path, stopped,
    };
    use crate::chatting::Chat;
    use crate::error::Error;
    use crate::resume::resume_press;
    use crate::stubs::Saying;

    // The pull the refusals below name, in [`Puller::in_flight`]'s own words: the
    // line comes from the value holding the run, and a resume only adds what it
    // did not do to it.
    const IN_FLIGHT: &str = "`WAR-140` is being pulled";

    // A conversation whose agent answers instantly if anything asks it anything,
    // and whose answer is a sentence no assertion below looks for: a turn started
    // by a resume shows up as `Chat::answering`, and the text is there so that a
    // thread carrying it fails loudly.
    fn conversation() -> Chat<Saying> {
        Chat::with_agent("/warlock/no/such/repository", Saying::answering(ANSWERED))
    }

    const ANSWERED: &str = "no resume asks a model anything";

    fn now() -> Instant {
        Instant::now()
    }

    /// The command typed at the panel, with the home this test wrote and no pull in
    /// flight unless one is named.
    fn press(
        app: &mut App,
        chat: &mut Chat<Saying>,
        ground: &Ground,
        in_flight: Option<&str>,
    ) -> Instant {
        let at = now();
        resume_press(
            app,
            chat,
            Some(ground.home.path()),
            ground.root.path(),
            TICKET,
            in_flight,
            at,
        );
        at
    }

    fn notes(app: &App, now: Instant) -> Vec<String> {
        app.panel()
            .thread()
            .map(|thread| thread.lines(now))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|line| match line {
                Line::Note { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    fn thread(app: &App, now: Instant) -> String {
        notes(app, now).join("\n")
    }

    // Everything on the thread that is not a note: a question, an answer, a tool
    // call or an ending. A resume puts none of them up, and a turn started by one
    // would show as the question it was asked with.
    fn other_lines(app: &App, now: Instant) -> Vec<Line> {
        app.panel()
            .thread()
            .map(|thread| thread.lines(now))
            .unwrap_or_default()
            .into_iter()
            .filter(|line| !matches!(line, Line::Note { .. }))
            .collect()
    }

    fn status_of(run: &PullRun, id: &str) -> SubtaskStatus {
        run.subtask(id)
            .expect("a sub-task this test wrote")
            .status()
            .clone()
    }

    #[test]
    fn a_halt_released_from_the_panel_puts_every_change_on_the_thread() {
        let ground = Ground::new();
        halted(&ground);
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, None);

        let said = thread(&app, at);
        // The subcommand's own lines, reason and all, which the reset dropped from
        // the record: one per sub-task put back and none about the two it left.
        assert!(
            said.contains(&format!(
                "`WAR-142.02` was `failed` and is `pending` again — {FAILED}"
            )),
            "{said}"
        );
        assert!(said.contains("`WAR-142.03` was `blocked`"), "{said}");
        assert!(said.contains(BLOCKED), "{said}");
        assert!(said.contains("`WAR-142.04` was `crossed`"), "{said}");
        assert!(said.contains(CROSSED), "{said}");
        assert!(!said.contains("WAR-142.01"), "{said}");
        assert_eq!(notes(&app, at).len(), 3, "{said}");
        // The shell's hand-off is not repeated here: what works the ticket again
        // is in the field, which is the next assertion's.
        assert!(!said.contains("warlock pull"), "{said}");

        let saved = ground.saved();
        assert_eq!(saved.status(), RunStatus::Resumed);
        for id in ["WAR-142.02", "WAR-142.03", "WAR-142.04"] {
            assert_eq!(status_of(&saved, id), SubtaskStatus::Pending, "{id}");
        }
        assert_eq!(status_of(&saved, "WAR-142.01"), SubtaskStatus::Done);
    }

    #[test]
    fn a_release_leaves_the_next_pull_in_the_field_with_the_cursor_at_its_end() {
        let ground = Ground::new();
        halted(&ground);
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, None);

        // The scope off the run record and not off a board: `/pull` takes the two
        // words in that order, and this is the command that works the ticket.
        let draft = format!("/pull {SCOPE} {TICKET}");
        assert_eq!(chat.composer().draft(), draft);
        // An ordinary draft: the cursor is at the end of it, so the next key typed
        // appends, and Enter would send whatever the field then holds.
        assert_eq!(chat.composer().cursor(), draft.len());
        assert!(chat.composer().is_submittable());
        assert!(!chat.composer().is_muted());

        // And nothing was sent. No turn is in flight, the mode is untouched, and
        // the thread holds notes alone.
        assert!(!chat.answering());
        assert_eq!(app.panel().mode(), Mode::Chat);
        assert_eq!(other_lines(&app, at), Vec::new());
    }

    #[test]
    fn a_resume_while_a_pull_is_in_flight_is_refused_and_changes_no_run_record() {
        let ground = Ground::new();
        halted(&ground);
        let before = ground.state();
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, Some(IN_FLIGHT));

        // One line, naming the pull that is holding the tree and what this
        // keystroke did not do.
        assert_eq!(
            notes(&app, at),
            vec![format!("{IN_FLIGHT}; this `/resume` changed no run record")]
        );
        // Byte-identical: the halt is still a halt, and the run in flight is still
        // the only thing deciding what those sub-tasks come to.
        assert_eq!(ground.state(), before);
        assert_eq!(ground.saved().status(), RunStatus::Halted);
        // And nothing was offered into the field, so a refusal cannot be answered
        // by pressing Enter.
        assert_eq!(chat.composer().draft(), "");
    }

    #[test]
    fn a_ticket_with_no_run_record_is_refused_in_the_subcommands_words() {
        let ground = Ground::new();
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, None);

        let wanted = Error::NoRun {
            ticket: TICKET.to_owned(),
            directory: super::run_dir(ground.home.path(), ground.root.path(), TICKET),
        };
        assert_eq!(notes(&app, at), vec![wanted.to_string()]);
        assert_eq!(chat.composer().draft(), "");
        // Nothing was written anywhere: no run to release is no run to record.
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
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, None);

        // The whole halt is what the panel asks for, so the sentence is the plain
        // resume's and never `--failed-only`'s: there is no spelling for the flag
        // in the composer.
        let wanted = Error::NothingToResume {
            ticket: TICKET.to_owned(),
            status: RunStatus::InReview,
            failed_only: false,
        };
        assert_eq!(notes(&app, at), vec![wanted.to_string()]);
        let said = thread(&app, at);
        assert!(!said.contains("--failed-only"), "{said}");
        assert_eq!(ground.state(), before);
        assert_eq!(chat.composer().draft(), "");
    }

    #[test]
    fn a_record_that_will_not_read_says_so_rather_than_that_there_is_no_run() {
        let ground = Ground::new();
        halted(&ground);
        std::fs::write(
            state_path(ground.home.path(), ground.root.path(), TICKET),
            "{ this is not the record warlock wrote",
        )
        .expect("a record this test broke on purpose");
        let mut app = App::default();
        let mut chat = conversation();

        let at = press(&mut app, &mut chat, &ground, None);

        let said = thread(&app, at);
        // Not the `NoRun` sentence: a record broken by a hand edit describes a
        // branch that may be holding uncommitted work.
        assert!(!said.contains("never pulled"), "{said}");
        assert_eq!(notes(&app, at).len(), 1, "{said}");
        assert_eq!(chat.composer().draft(), "");
    }

    #[test]
    fn a_machine_with_no_home_holds_no_run_to_release() {
        let ground = Ground::new();
        halted(&ground);
        let before = ground.state();
        let mut app = App::default();
        let mut chat = conversation();

        let at = now();
        resume_press(
            &mut app,
            &mut chat,
            None,
            ground.root.path(),
            TICKET,
            None,
            at,
        );

        assert_eq!(notes(&app, at), vec![Error::NoHome.to_string()]);
        assert_eq!(ground.state(), before);
        assert_eq!(chat.composer().draft(), "");
    }
}
