//! Every test here drives [`Puller`] over a temporary repository and a temporary
//! home, through the seams the event loop hands it: a stand-in board, checkout,
//! forge and three stand-in sessions. Nothing below opens a socket, runs a `git`,
//! raises a `claude` or reads the sigils, the binding or the key store of the
//! machine the suite runs on, and the one key any of it stores is not one.

use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use warlock_engine::{
    Manifest, PactEntry, PullRun, RunStatus, ScopeRecord, save_key, save_key_binding, save_sigils,
};
use warlock_tui::{
    Activity, Answer, App, Assignee, Dirty, Line, NamedIssue, Priority, PullAnswered, Queue,
    QueuedIssue, Section, StateType, Taking,
};

use super::Puller;
use crate::error::{Error, one_line};
use crate::freshness::Freshened;
use crate::stubs::{
    Boarding, Checkout, Forging, Refreshing, Sessions, Slicing, VIEWER, Written, said,
};

// Not a key, and named so that nothing reading this file mistakes it for one: it
// is stored only so that a bound name resolves.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "this-tests-own-name";

const SCOPE: &str = "warlock-team";

// A scope this repository records that the machine below does not hold.
const CLOSED: &str = "control-plane";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const REVIEW: &str = "In Review";

const DEFAULT: &str = "main";

const TICKET: &str = "WAR-140";

const ISSUE: &str = "issue-140";

const TITLE: &str = "Add `warlock pull <SCOPE>`";

const URL: &str = "https://github.com/team/repo/pull/12";

const WROTE: &str = "crates/engine/src/lib.rs";

// Long enough that a worker which never reports fails the test rather than
// hanging the suite, and short enough that it is a failure rather than a wait.
const AT_MOST: Duration = Duration::from_secs(10);

fn now() -> Instant {
    Instant::now()
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// Two recorded scopes, one pacted directory under each, so a crossing and a
// closed scope are both reachable from one manifest.
fn a_manifest() -> Manifest {
    Manifest::with_entries([
        pacted("crates/engine", SCOPE),
        pacted("crates/control", CLOSED),
    ])
    .with_scopes([
        ScopeRecord::new(SCOPE, TEAM, REVIEW, LABEL),
        ScopeRecord::new(CLOSED, "CTL", REVIEW, LABEL),
    ])
}

fn pacted(directory: &str, scope: &str) -> PactEntry {
    PactEntry::new(".", directory, format!("{directory}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
        .with_scope(scope)
}

// A home of this test's own: the sigils that pick the board, the binding, the key
// store and the run records all sit under it.
fn a_home(root: &Path) -> TempDir {
    let home = a_dir();
    save_sigils(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
    save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

/// The ground every test below stands on: the two temporary directories and the
/// manifest, kept alive for as long as the value under test is.
struct Ground {
    root: TempDir,
    home: TempDir,
    manifest: Manifest,
}

impl Ground {
    fn new() -> Self {
        let root = a_dir();
        let home = a_home(root.path());
        Self {
            root,
            home,
            manifest: a_manifest(),
        }
    }

    fn saved(&self) -> PullRun {
        PullRun::load(self.home.path(), self.root.path(), TICKET).expect("the run wrote its record")
    }
}

// A run that works two sub-tasks and reaches a pull request, with a session that
// reads, thinks and spends on the way.
fn working() -> Written {
    Written::of(
        Slicing::into_chain(TICKET, &["Read the queue", "Work the ticket"]),
        Sessions::answering([
            said("done", "the queue is read", None),
            said("done", "the ticket is worked", None),
        ]),
        Refreshing::quiet(),
    )
    .doing([
        Activity::Tool {
            name: "Read".to_owned(),
            detail: Some(WROTE.to_owned()),
        },
        Activity::Thinking,
        Activity::Writing { bytes: 512 },
        Activity::Cost { usd: 0.42 },
    ])
}

// The sessions of a run nothing lets reach one: a split or a sub-task raised at
// all is the failure, which an empty script panics over.
fn unasked() -> Written {
    Written::of(
        Slicing::into_chain(TICKET, &["Nothing this test lets a run reach"]),
        Sessions::answering([]),
        Refreshing::quiet(),
    )
}

fn queue(issues: impl IntoIterator<Item = QueuedIssue>) -> Queue {
    Queue::new(issues.into_iter().collect(), false)
}

fn ready() -> QueuedIssue {
    QueuedIssue::new(
        ISSUE,
        TICKET,
        TITLE,
        "Todo",
        StateType::new("unstarted"),
        Priority::Urgent,
        Vec::new(),
    )
}

fn board(queue: Queue) -> Boarding {
    Boarding::filing(URL).queueing(queue)
}

fn named(issue: QueuedIssue) -> NamedIssue {
    NamedIssue::new(
        issue,
        TEAM,
        vec![LABEL.to_owned()],
        Some(Assignee::new(VIEWER, "Cole")),
    )
}

/// One modified path, which is a tree with something in it.
fn wrote(path: &str) -> Vec<Dirty> {
    vec![Dirty {
        code: " M".to_owned(),
        path: path.to_owned(),
        from: None,
    }]
}

// The checkout a fresh pull runs in: clean for the look before the board is
// opened, and holding the session's work for every look after it.
fn checkout() -> Checkout {
    Checkout::clean(DEFAULT).trees([Vec::new(), wrote(WROTE)])
}

/// The value the loop holds, over the seams a test wrote down.
fn puller(
    ground: &Ground,
    board: Boarding,
    repo: Checkout,
    forge: Forging,
    raises: Written,
) -> Puller<Boarding, Checkout, Forging, Written> {
    Puller::with_seams(
        board,
        repo,
        forge,
        raises,
        Some(ground.home.path().to_path_buf()),
    )
}

type Panel = Puller<Boarding, Checkout, Forging, Written>;

// A `/pull <SCOPE>` pressed at the value the loop holds.
fn press(app: &mut App, puller: &mut Panel, ground: &Ground, ticket: Option<&str>) {
    puller.press(
        app,
        &ground.manifest,
        ground.root.path(),
        Some(Taking {
            scope: SCOPE,
            ticket,
        }),
        now(),
    );
}

// Rounds until selection has reported, drained and never blocked on: the loop
// draws and then drains, so a test that waited on the channel would be a test of
// something the panel does not do.
fn chosen(app: &mut App, puller: &mut Panel) {
    let waited = Instant::now();
    while puller.choosing() && waited.elapsed() < AT_MOST {
        puller.keep_up(app, now());
    }
    assert!(!puller.choosing(), "the pull never chose a ticket");
}

// And the same for the run, answered Yes first.
fn through(app: &mut App, puller: &mut Panel) {
    puller.answered(app, PullAnswered::Pull, now());
    let waited = Instant::now();
    while puller.pulling() && waited.elapsed() < AT_MOST {
        puller.keep_up(app, now());
    }
    assert!(!puller.pulling(), "the run never finished");
}

fn notes(app: &App) -> Vec<String> {
    app.panel()
        .thread()
        .map(|thread| thread.lines(now()))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| match line {
            Line::Note { text } => Some(text),
            _ => None,
        })
        .collect()
}

fn thread(app: &App) -> String {
    notes(app).join("\n")
}

fn sections(app: &App) -> Vec<String> {
    app.panel()
        .account()
        .map(|account| {
            account
                .sections()
                .iter()
                .map(|section| Section::directory(section).display().to_string())
                .collect()
        })
        .unwrap_or_default()
}

// Every line of the account card, headings and all: what a section holds is asked
// of this rather than of the log, because the card is what a reader sees.
fn account(app: &App) -> String {
    app.panel()
        .account()
        .map(|account| {
            account
                .lines(now())
                .into_iter()
                .map(|line| format!("{line:?}"))
                .collect::<Vec<String>>()
                .join("\n")
        })
        .unwrap_or_default()
}

// The engine's own sentence for a refusal, flattened as the thread takes it: the
// tests below assert the line *is* that sentence rather than restating it.
fn refusal(error: &Error) -> String {
    one_line(&error.to_string())
}

/// A released run for this ticket on this machine: `resumed` is the status
/// `warlock resume` leaves behind, and the one selection takes ahead of anything
/// else — a halted one is passed over with the command that frees it.
fn released(ground: &Ground, subtasks: &[(&str, &str)]) {
    let mut run = PullRun::new(
        TICKET,
        TITLE,
        SCOPE,
        "war-140/add-warlock-pull-scope",
        "2026-09-28T09:00:00+00:00",
    );
    for (id, goal) in subtasks {
        run.push_subtask(warlock_engine::PullSubtask::new(
            *id,
            *goal,
            Vec::<String>::new(),
        ));
    }
    run.set_status(RunStatus::Resumed);
    run.save(ground.home.path(), ground.root.path())
        .expect("a record that writes");
}

#[test]
fn a_bare_pull_names_the_scopes_this_machine_holds_and_reads_nothing_else() {
    let ground = Ground::new();
    let linear = Boarding::unreachable();
    let repo = checkout();
    let mut puller = puller(
        &ground,
        linear.clone(),
        repo.clone(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    puller.press(&mut app, &ground.manifest, ground.root.path(), None, now());

    let said = notes(&app).pop().expect("the pull said nothing");
    assert!(
        said.contains(SCOPE),
        "{said:?} does not name the sigil held"
    );
    assert!(
        !said.contains(CLOSED),
        "{said:?} names a scope this machine does not hold"
    );
    assert!(
        repo.calls().is_empty(),
        "a bare `/pull` ran `git`: {:?}",
        repo.calls()
    );
    assert!(!puller.choosing(), "a bare `/pull` started a worker");
    assert!(
        !puller.confirm().is_open(),
        "a bare `/pull` opened the dialog"
    );
    drop(linear);
}

#[test]
fn a_machine_with_no_home_is_refused_in_standing_s_own_words() {
    let ground = Ground::new();
    let mut puller: Panel = Puller::with_seams(
        Boarding::unopened(),
        checkout(),
        Forging::opening(URL),
        unasked(),
        None,
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);

    assert_eq!(notes(&app), vec![refusal(&Error::NoHome)]);
    assert!(!puller.choosing(), "a machine with no home read a queue");
}

#[test]
fn a_scope_no_record_holds_and_one_this_machine_does_not_hold_are_prepare_s_refusals() {
    let ground = Ground::new();
    let repo = checkout();
    let mut puller = puller(
        &ground,
        Boarding::unopened(),
        repo.clone(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    puller.press(
        &mut app,
        &ground.manifest,
        ground.root.path(),
        Some(Taking {
            scope: "billing",
            ticket: None,
        }),
        now(),
    );
    puller.press(
        &mut app,
        &ground.manifest,
        ground.root.path(),
        Some(Taking {
            scope: CLOSED,
            ticket: None,
        }),
        now(),
    );

    let said = notes(&app);
    assert_eq!(
        said,
        vec![
            refusal(&Error::UnrecordedScope {
                scope: "billing".to_owned(),
                recorded: vec![SCOPE.to_owned(), CLOSED.to_owned()],
            }),
            refusal(&Error::UnheldScope {
                scope: CLOSED.to_owned(),
                held: vec![SCOPE.to_owned()],
            }),
        ]
    );
    assert!(repo.calls().is_empty(), "a refused `/pull` ran `git`");
    assert!(!puller.choosing(), "a refused `/pull` read a queue");
}

#[test]
fn a_second_pull_is_refused_with_a_line_naming_the_one_in_flight_and_reads_nothing() {
    let ground = Ground::new();
    let linear = board(queue([ready()]));
    let mut puller = puller(
        &ground,
        linear.clone(),
        checkout(),
        Forging::opening(URL),
        working(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    puller.answered(&mut app, PullAnswered::Pull, now());
    let asked = linear.requests();
    press(&mut app, &mut puller, &ground, None);

    let said = notes(&app).pop().expect("the second pull said nothing");
    assert!(
        said.contains(TICKET),
        "{said:?} does not name the pull in flight"
    );
    assert!(
        said.contains("read nothing"),
        "{said:?} does not say the second pull read nothing"
    );
    assert_eq!(
        linear.requests(),
        asked,
        "a refused `/pull` asked the board something"
    );
    through(&mut app, &mut puller);
}

#[test]
fn a_no_starts_nothing_and_leaves_the_ticket_where_it_was() {
    let ground = Ground::new();
    let repo = checkout();
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        repo.clone(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    assert!(puller.confirm().is_open(), "the dialog never opened");
    puller.answered(&mut app, PullAnswered::Cancel, now());

    assert!(!puller.confirm().is_open(), "a No left the question up");
    assert!(!puller.pulling(), "a No started a run");
    assert_eq!(
        repo.commits(),
        Vec::<String>::new(),
        "a No committed something"
    );
    assert!(
        app.panel().account().is_none(),
        "a No opened the run's output window"
    );
}

#[test]
fn the_dialog_names_the_ticket_the_scope_the_team_and_the_branch_it_will_cut() {
    let ground = Ground::new();
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        checkout(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);

    let undertaking = puller
        .confirm()
        .undertaking()
        .expect("the dialog is up")
        .clone();
    assert_eq!(undertaking.ticket(), TICKET);
    assert_eq!(undertaking.title(), TITLE);
    assert_eq!(undertaking.scope(), SCOPE);
    assert_eq!(undertaking.team(), TEAM);
    assert_eq!(undertaking.branch(), "war-140/add-warlock-pull-scope");
    assert_eq!(undertaking.resuming(), None);
    // No is lit on open, which is the dialog's own promise: the round that puts
    // it up and an Enter straight after it come to nothing at all.
    assert_eq!(undertaking.answer(), Answer::No);
}

#[test]
fn a_run_this_checkout_holds_is_offered_as_a_resume_from_the_sub_task_it_stopped_at() {
    let ground = Ground::new();
    released(&ground, &[("WAR-140.01", "Read the queue")]);
    let mut puller = puller(
        &ground,
        Boarding::filing(URL).naming(named(ready())),
        checkout(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, Some(TICKET));
    chosen(&mut app, &mut puller);

    let undertaking = puller
        .confirm()
        .undertaking()
        .expect("the dialog is up")
        .clone();
    assert_eq!(undertaking.branch(), "war-140/add-warlock-pull-scope");
    assert_eq!(
        undertaking.resuming(),
        Some("WAR-140.01"),
        "the question does not name the sub-task the run carries on from"
    );
}

#[test]
fn a_finished_run_puts_a_section_per_step_on_the_account_and_only_milestones_on_the_thread() {
    let ground = Ground::new();
    let repo = checkout();
    let forge = Forging::opening(URL);
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        repo.clone(),
        forge.clone(),
        working(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    through(&mut app, &mut puller);

    // One section for the split, one per sub-task headed by its id and goal, and
    // one for the pull request.
    let sections = sections(&app);
    assert!(
        sections
            .first()
            .is_some_and(|heading| heading.contains(TICKET) && heading.contains("splitting")),
        "{sections:?} does not open with the split"
    );
    assert!(
        sections.contains(&"WAR-140.01 Read the queue".to_owned())
            && sections.contains(&"WAR-140.02 Work the ticket".to_owned()),
        "{sections:?} is missing a sub-task's section"
    );
    assert!(
        sections
            .last()
            .is_some_and(|heading| heading.contains("pull request")),
        "{sections:?} does not end with the pull request"
    );

    // The session's own lines are on the card and nowhere else.
    let account = account(&app);
    for seen in ["Read", WROTE, "thinking", "writing"] {
        assert!(account.contains(seen), "{account} is missing `{seen}`");
    }
    // The tool's own detail, the two wordless activities and the cost: a
    // milestone naming a sub-task carries its goal, so what is asserted about is
    // what only an activity could have put there.
    let thread = thread(&app);
    for activity in [WROTE, "thinking", "writing", "0.42"] {
        assert!(
            !thread.contains(activity),
            "`{activity}` reached the thread:\n{thread}"
        );
    }

    // And the milestones: the ticket taken, each commit, the pull request's URL.
    assert!(thread.contains(TICKET), "{thread} does not name the ticket");
    for message in repo.commits() {
        assert!(
            thread.contains(&message),
            "a commit is not on the thread:\n{thread}"
        );
    }
    assert_eq!(repo.commits().len(), 2, "{:?}", repo.commits());
    assert!(thread.contains(URL), "{thread} does not carry the URL");
    assert_eq!(ground.saved().pr_url(), Some(URL));
    assert_eq!(forge.asked().len(), 1, "one run opened two pull requests");
}

#[test]
fn a_halt_lands_as_one_line_and_takes_nothing_down() {
    let ground = Ground::new();
    let repo = checkout();
    let written = working()
        .splitting(Slicing::into_chain(TICKET, &["Read the queue"]))
        .sessioning(Sessions::answering([said(
            "blocked",
            "the scope is somebody else's",
            Some("a decision only the human can make"),
        )]));
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        repo.clone(),
        Forging::opening(URL),
        written,
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    through(&mut app, &mut puller);

    let said = notes(&app).pop().expect("the halt said nothing");
    assert_eq!(
        said,
        refusal(&Error::Halted {
            ticket: TICKET.to_owned()
        })
    );
    assert_eq!(ground.saved().status(), RunStatus::Halted);
    assert_eq!(
        repo.commits(),
        Vec::<String>::new(),
        "a halted sub-task was committed"
    );
    // The panel goes on running: the run is over, the next `/pull` is allowed,
    // and the account card still holds what the run did.
    assert!(!puller.pulling());
    assert!(!sections(&app).is_empty(), "the halt took the account down");
}

#[test]
fn a_crossing_names_the_sub_task_that_wrote_past_the_boundary() {
    let ground = Ground::new();
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/control/src/lib.rs")]);
    let written = working()
        .splitting(Slicing::into_chain(TICKET, &["Read the queue"]))
        .sessioning(Sessions::answering([said(
            "done",
            "the queue is read",
            None,
        )]));
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        repo.clone(),
        Forging::opening(URL),
        written,
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    through(&mut app, &mut puller);

    let said = notes(&app).pop().expect("the crossing said nothing");
    assert_eq!(
        said,
        refusal(&Error::Crossed {
            ticket: TICKET.to_owned(),
            subtask: "WAR-140.01".to_owned(),
        })
    );
    assert_eq!(
        repo.commits(),
        Vec::<String>::new(),
        "a crossing was committed"
    );
}

#[test]
fn a_dirty_tree_is_refused_before_the_board_is_opened() {
    let ground = Ground::new();
    let linear = Boarding::unopened();
    let mut puller = puller(
        &ground,
        linear.clone(),
        Checkout::clean(DEFAULT).trees([wrote(WROTE)]),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);

    let said = notes(&app).pop().expect("the refusal said nothing");
    assert_eq!(
        said,
        refusal(&Error::DirtyTree {
            dirty: wrote(WROTE)
        })
    );
    assert!(
        !puller.confirm().is_open(),
        "a dirty tree opened the dialog"
    );
    drop(linear);
}

#[test]
fn a_queue_with_nothing_ready_is_an_answer_and_asks_nothing() {
    let ground = Ground::new();
    let mut puller = puller(
        &ground,
        board(queue([])),
        checkout(),
        Forging::opening(URL),
        unasked(),
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);

    let said = notes(&app).pop().expect("the pull said nothing");
    assert!(said.contains(SCOPE), "{said:?} does not name the scope");
    assert!(
        !puller.confirm().is_open(),
        "an empty queue opened the dialog"
    );
    assert!(!puller.pulling(), "an empty queue started a run");
}

#[test]
fn the_refresh_pass_gets_a_section_of_its_own_per_directory_between_the_work_and_the_request() {
    let ground = Ground::new();
    let asked = Refreshing::answering(Freshened {
        refreshed: vec!["crates/engine".to_owned()],
        left_stale: Vec::new(),
    });
    let written = working()
        .freshening(asked.clone())
        .refreshing(&["crates/engine"]);
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        checkout(),
        Forging::opening(URL),
        written,
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    through(&mut app, &mut puller);

    assert_eq!(asked.asked().len(), 1, "the run did not refresh once");
    let sections = sections(&app);
    let at = sections
        .iter()
        .position(|heading| heading.contains("crates/engine"))
        .unwrap_or_else(|| panic!("{sections:?} has no section for the directory refreshed"));
    // Between the last sub-task and the pull request, which is where the pass
    // runs: after the section of the sub-task whose commit it refreshes from, and
    // before the request opened from what it wrote.
    assert!(
        sections[..at]
            .iter()
            .any(|heading| heading.contains("WAR-140.02")),
        "{sections:?}"
    );
    assert!(
        sections[at + 1..]
            .iter()
            .any(|heading| heading.contains("pull request")),
        "{sections:?}"
    );
}

#[test]
fn a_failure_in_the_run_lands_as_one_line_and_leaves_the_panel_running() {
    let ground = Ground::new();
    let repo = checkout();
    // The one failure the refresh hands back rather than reporting: a checkout
    // that cannot be asked what the branch changed, which is how every other step
    // of the loop fails.
    let written = working().freshening(Refreshing::refusing());
    let forge = Forging::opening(URL);
    let mut puller = puller(
        &ground,
        board(queue([ready()])),
        repo.clone(),
        forge.clone(),
        written,
    );
    let mut app = App::default();

    press(&mut app, &mut puller, &ground, None);
    chosen(&mut app, &mut puller);
    through(&mut app, &mut puller);

    let said = notes(&app).pop().expect("the failure said nothing");
    assert!(
        said.contains("git"),
        "{said:?} is not the failure `git.rs` words"
    );
    assert_eq!(said, one_line(said.trim()), "the failure is not one line");
    // Nothing came down with it: the run is over, the next `/pull` is allowed, the
    // work the run did commit is committed, and the account still holds it.
    assert!(!puller.pulling(), "a failure left the run in flight");
    assert_eq!(repo.commits().len(), 2, "{:?}", repo.commits());
    assert!(
        !sections(&app).is_empty(),
        "a failure took the account down"
    );
    assert!(
        forge.asked().is_empty(),
        "the run opened a pull request past the failure"
    );
}
