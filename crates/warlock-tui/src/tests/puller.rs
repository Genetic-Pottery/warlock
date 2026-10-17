//! Every test here drives [`Puller`] over a temporary repository and a temporary
//! home, through the seams the event loop hands it: a stand-in board, checkout,
//! forge and three stand-in sessions. Nothing below opens a socket, runs a `git`,
//! raises a `claude` or reads the open scopes, the binding or the key store of the
//! machine the suite runs on, and the one key any of it stores is not one.

use std::path::Path;
use std::time::Instant;

use tempfile::TempDir;
use warlock_engine::{
    Manifest, PullRun, RunStatus, ScopeRecord, ScopedDirectory, save_key, save_key_binding,
    save_open_scopes,
};

use super::Puller;
use crate::account::Line;
use crate::app::App;
use crate::board::{Assignee, NamedTicket, Priority, Queue, QueuedTicket, StateType};
use crate::claude::Activity;
use crate::confirm::{Answer, PullAnswered};
use crate::error::{Error, NO_QUEUE, one_line};
use crate::git::Dirty;
use crate::prompt::KeyPrompt;
use crate::stubs::{Boarding, Checkout, Forging, Op, Sessions, Slicing, VIEWER, Written, said};
use crate::submission::Taking;

// Not a key, and named so that nothing reading this file mistakes it for one: it
// is stored only so that a bound name resolves.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "this-tests-own-name";

const SCOPE: &str = "warlock-team";

// A scope this repository records that is closed on the machine below.
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

fn now() -> Instant {
    Instant::now()
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// Two recorded scopes, one scoped directory under each, so a crossing and a
// closed scope are both reachable from one manifest.
fn a_manifest() -> Manifest {
    Manifest::with_directories([
        scoped("crates/engine", SCOPE),
        scoped("crates/control", CLOSED),
    ])
    .with_scopes([
        ScopeRecord::new(SCOPE, TEAM, REVIEW, LABEL),
        ScopeRecord::new(CLOSED, "CTL", REVIEW, LABEL),
    ])
}

// The same two records with the open scope's one recording the board warlock
// cannot pull from: the tracker is the only thing that differs, so the refusal
// is about that and not about a scope spelled differently or a key nobody bound.
fn a_jira_manifest() -> Manifest {
    Manifest::with_directories([
        scoped("crates/engine", SCOPE),
        scoped("crates/control", CLOSED),
    ])
    .with_scopes([
        ScopeRecord::jira(SCOPE, "https://acme.atlassian.net", "PAY", REVIEW, LABEL),
        ScopeRecord::new(CLOSED, "CTL", REVIEW, LABEL),
    ])
}

// The sentence `warlock-engine` words for a record that files somewhere else,
// spelled out because nothing outside that crate can build the refusal to ask
// it: this is the line a reader sees on the thread.
fn not_linear(scope: &str) -> String {
    format!("`{scope}` files to Jira, and warlock can only file to Linear, so nothing was sent")
}

fn scoped(directory: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", directory, scope).expect("a relative module path is inside the root")
}

// A home of this test's own: the open scopes, the binding, the key store and the
// run records all sit under it.
fn a_home(root: &Path) -> TempDir {
    let home = a_dir();
    save_open_scopes(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
    save_key_binding(home.path(), root, SCOPE, KEY_NAME).expect("a binding that writes");
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
    )
}

fn queue(issues: impl IntoIterator<Item = QueuedTicket>) -> Queue {
    Queue::new(issues.into_iter().collect())
}

fn ready() -> QueuedTicket {
    QueuedTicket::new(
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

fn named(issue: QueuedTicket) -> NamedTicket {
    NamedTicket::new(
        issue,
        TEAM,
        vec![LABEL.to_owned()],
        Some(Assignee::new(VIEWER, "Ada")),
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
    .inline()
}

type Panel = Puller<Boarding, Checkout, Forging, Written>;

// A `/pull <SCOPE>` pressed at the value the loop holds, over a scope with a key
// bound: every test that goes through here is one of those, and the key window
// that comes back is the assertion that none of them was asked for a binding
// instead.
fn press(app: &mut App, puller: &mut Panel, ground: &Ground, ticket: Option<&str>) {
    let prompt = puller.press(
        app,
        &ground.manifest,
        ground.root.path(),
        &Taking {
            scope: SCOPE.to_owned(),
            ticket: ticket.map(str::to_owned),
        },
        now(),
    );
    assert_eq!(
        prompt,
        KeyPrompt::Closed,
        "a bound scope was asked for a binding"
    );
}

// The round after the press: the worker ran inline, so one drain reports it.
fn chosen(app: &mut App, puller: &mut Panel) {
    puller.keep_up(app, now());
    assert!(!puller.choosing(), "the pull never chose a ticket");
}

// And the same for the run, answered Yes first.
fn through(app: &mut App, puller: &mut Panel) {
    puller.answered(app, PullAnswered::Pull, now());
    puller.keep_up(app, now());
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

// The run's phases as the thread draws them: a note with a live work turn
// directly under it, which a milestone never has.
fn sections(app: &App) -> Vec<String> {
    let lines = app
        .panel()
        .thread()
        .map(|thread| thread.lines(now()))
        .unwrap_or_default();
    lines
        .windows(2)
        .filter_map(|pair| match pair {
            [Line::Note { text }, Line::Clocked { .. }] => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// The work rows the phases ticked on the thread, which is where a session's
// activity goes.
fn account(app: &App) -> String {
    app.panel()
        .thread()
        .map(|thread| thread.lines(now()))
        .unwrap_or_default()
        .into_iter()
        .filter(|line| matches!(line, Line::Clocked { .. }))
        .map(|line| format!("{line:?}"))
        .collect::<Vec<String>>()
        .join("\n")
}

// The engine's own sentence for a refusal, flattened as the thread takes it: the
// tests below assert the line *is* that sentence rather than restating it.
fn refusal(error: &Error) -> String {
    one_line(error)
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
fn a_scope_no_record_holds_and_one_closed_on_this_machine_are_prepare_s_refusals() {
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
    // Both scopes are bound, and that is what these two refusals are about: an
    // unbound one is the question asked ahead of `prepare` rather than one of
    // its refusals, so the binding is written for each so that the `/pull`
    // reaches `prepare` to be told what is wrong with the scope itself.
    for scope in ["billing", CLOSED] {
        save_key_binding(ground.home.path(), ground.root.path(), scope, KEY_NAME)
            .expect("a binding that writes");
    }

    for scope in ["billing", CLOSED] {
        let prompt = puller.press(
            &mut app,
            &ground.manifest,
            ground.root.path(),
            &Taking {
                scope: scope.to_owned(),
                ticket: None,
            },
            now(),
        );
        assert_eq!(prompt, KeyPrompt::Closed, "a refusal put a window up");
    }

    let said = notes(&app);
    assert_eq!(
        said,
        vec![
            refusal(&Error::UnrecordedScope {
                scope: "billing".to_owned(),
                recorded: vec![SCOPE.to_owned(), CLOSED.to_owned()],
                cost: NO_QUEUE,
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
fn a_finished_run_puts_a_phase_per_step_on_the_thread_with_its_work_under_it() {
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

    // One phase for the split, one per sub-task headed by its id and goal, and
    // one for the pull request.
    let sections = sections(&app);
    assert!(
        sections
            .first()
            .is_some_and(|heading| heading.contains(TICKET) && heading.contains("splitting")),
        "{sections:?} does not open with the split"
    );
    assert!(
        sections.contains(&"[1/2] `WAR-140.01` Read the queue".to_owned())
            && sections.contains(&"[2/2] `WAR-140.02` Work the ticket".to_owned()),
        "{sections:?} is missing a sub-task's phase"
    );
    assert!(
        sections
            .last()
            .is_some_and(|heading| heading.contains("pull request")),
        "{sections:?} does not end with the pull request"
    );

    // The session's own lines tick under the phase they were spent in, and are
    // never a line of their own.
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
    // and the thread still holds what the run did.
    assert!(!puller.pulling());
    assert!(
        !sections(&app).is_empty(),
        "the halt took the run's phases down"
    );
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

// The question asked ahead of `prepare`, over a temporary repository and a
// temporary home of each test's own: the binding this checkout holds for the
// scope typed is read out of the config under that home and nowhere else. Ported
// from `tests/pushing.rs` and `tests/cutting.rs`, which ask the same question in
// the same place — the difference being what the answer leads to, which here is
// the queue.
mod binding {
    use std::path::Path;

    use tempfile::TempDir;
    use warlock_engine::{load_key_binding, save_key, save_key_binding, save_open_scopes};

    use super::{
        Answer, App, Boarding, Checkout, Forging, KEY_NAME, KeyPrompt, NOT_A_KEY, Panel,
        PullAnswered, Puller, SCOPE, TICKET, TITLE, Taking, URL, a_dir, a_jira_manifest,
        a_manifest, board, checkout, not_linear, notes, now, queue, ready, unasked,
    };
    use crate::error::{Error, one_line};

    // A home with the scope open and a key in the store, and no binding for any
    // scope: the checkout somebody has just cloned, which is the road this
    // question exists for.
    fn an_unbound_home(root: &Path) -> TempDir {
        let home = a_dir();
        save_open_scopes(home.path(), root, &[SCOPE.to_owned()]).expect("a config that writes");
        save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
        home
    }

    // The value the loop holds over the board and the checkout handed in, with
    // the workers inline and a model no run may reach: every test here is
    // decided before a session is raised.
    fn puller(linear: Boarding, repo: Checkout, home: &Path) -> Panel {
        Puller::with_seams(
            linear,
            repo,
            Forging::opening(URL),
            unasked(),
            Some(home.to_path_buf()),
        )
        .inline()
    }

    // `/pull <SCOPE>` pressed, and the key window it came to.
    fn asking(app: &mut App, puller: &mut Panel, root: &Path) -> KeyPrompt {
        puller.press(
            app,
            &a_manifest(),
            root,
            &Taking {
                scope: SCOPE.to_owned(),
                ticket: None,
            },
            now(),
        )
    }

    #[test]
    fn a_pull_of_a_scope_bound_to_nothing_asks_for_the_key_and_reads_nothing() {
        let root = a_dir();
        let home = an_unbound_home(root.path());
        let repo = checkout();
        let mut puller = puller(Boarding::unopened(), repo.clone(), home.path());
        let mut app = App::default();

        let prompt = asking(&mut app, &mut puller, root.path());

        let choice = prompt.choice().expect("the key window is up");
        assert_eq!(
            choice.scope(),
            SCOPE,
            "the window names a scope the command never did"
        );
        assert_eq!(
            choice.names(),
            [KEY_NAME],
            "the rows are not this home's key store"
        );
        assert!(!puller.choosing(), "the question read a queue");
        assert!(!puller.pulling(), "the question started a run");
        assert!(
            !puller.confirm().is_open(),
            "the question put the pull dialog up as well"
        );
        assert!(repo.calls().is_empty(), "the question ran `git`");
        // No reading line either: nothing is being read, so saying so would be a
        // line about a request that was never made. The board panics if it is
        // opened, so nothing being asked of Linear is the stand-in's own
        // assertion.
        assert!(notes(&app).is_empty(), "{:?}", notes(&app));
    }

    #[test]
    fn the_binding_written_reads_the_queue_and_puts_the_pull_dialog_up() {
        // What the key window's Enter does, in the engine's own call, and then
        // the `/pull` picked up behind it: the reading line, the choice on a
        // worker, and the dialog over the ticket the queue gave up.
        let root = a_dir();
        let home = an_unbound_home(root.path());
        let linear = board(queue([ready()]));
        let mut puller = puller(linear, checkout(), home.path());
        let mut app = App::default();
        let prompt = asking(&mut app, &mut puller, root.path());
        assert!(prompt.is_open(), "{:?}", notes(&app));
        save_key_binding(home.path(), root.path(), SCOPE, KEY_NAME).expect("a binding that writes");

        puller.bound(&mut app, &a_manifest(), root.path(), now());

        let said = notes(&app);
        assert!(
            said.first().is_some_and(|line| line.contains(SCOPE)),
            "the pick-up did not say what it was reading: {said:?}"
        );
        assert!(puller.choosing(), "the binding read no queue");
        puller.keep_up(&mut app, now());
        let undertaking = puller
            .confirm()
            .undertaking()
            .expect("the dialog the binding led to")
            .clone();
        assert_eq!(undertaking.ticket(), TICKET);
        assert_eq!(undertaking.title(), TITLE);
        assert_eq!(undertaking.scope(), SCOPE);
        assert_eq!(undertaking.answer(), Answer::No, "the dialog opened on Yes");
        for line in notes(&app) {
            assert!(!line.contains(NOT_A_KEY), "{line} carries the key");
        }

        // And the `/pull` is picked up once: the question is not waiting to be
        // asked again, so a second Enter at a window somebody else's road put up
        // reads nothing.
        puller.answered(&mut app, PullAnswered::Cancel, now());
        puller.bound(&mut app, &a_manifest(), root.path(), now());

        assert!(!puller.choosing(), "the queue was read twice");
        assert!(!puller.confirm().is_open(), "a second dialog came up");
    }

    #[test]
    fn an_abandoned_question_puts_the_whole_pull_down_and_writes_no_binding() {
        // Esc on the key window: the window is the one that writes, and it never
        // opens the config, so what is asserted here is the other half — nothing
        // is read, no dialog opens and the scope is still bound to nothing.
        let root = a_dir();
        let home = an_unbound_home(root.path());
        let repo = checkout();
        let mut puller = puller(Boarding::unopened(), repo.clone(), home.path());
        let mut app = App::default();
        let prompt = asking(&mut app, &mut puller, root.path());
        assert!(prompt.is_open(), "{:?}", notes(&app));

        puller.put_down();
        // The press that answers the window is answered by the flow either way,
        // so an Esc followed by an Enter at a later window cannot pick this pull
        // up.
        puller.bound(&mut app, &a_manifest(), root.path(), now());

        assert!(!puller.choosing(), "a queue was read");
        assert!(!puller.confirm().is_open(), "a dialog came up");
        assert!(!puller.pulling(), "a run started");
        assert!(repo.calls().is_empty(), "an abandoned pull ran `git`");
        assert_eq!(
            load_key_binding(home.path(), root.path(), SCOPE).expect("the config still reads"),
            None,
            "the scope was bound by an abandoned question"
        );
        assert!(notes(&app).is_empty(), "{:?}", notes(&app));
    }

    #[test]
    fn a_pull_of_a_jira_scope_is_one_line_with_no_question_asked_and_nothing_parked() {
        // The home binds nothing, which is the road that would otherwise put the
        // key window up and park the `/pull` behind it: the tracker is read
        // before that, so what comes back is the refusal and no question at all.
        // The board panics if it is opened, so reaching one is the failure rather
        // than something asserted afterwards.
        let root = a_dir();
        let home = an_unbound_home(root.path());
        let repo = checkout();
        let linear = Boarding::unopened();
        let mut puller = puller(linear.clone(), repo.clone(), home.path());
        let mut app = App::default();

        let prompt = puller.press(
            &mut app,
            &a_jira_manifest(),
            root.path(),
            &Taking {
                scope: SCOPE.to_owned(),
                ticket: None,
            },
            now(),
        );

        assert_eq!(notes(&app), vec![not_linear(SCOPE)]);
        assert_eq!(prompt, KeyPrompt::Closed, "a refused pull asked for a key");
        assert!(puller.parked.is_none(), "a refused pull was parked");
        assert!(!puller.choosing(), "a refused pull read a queue");
        assert!(!puller.pulling(), "a refused pull started a run");
        assert!(repo.calls().is_empty(), "a refused `/pull` ran `git`");
        assert!(
            linear.issues_created().is_empty(),
            "a refused pull created an issue"
        );
    }

    #[test]
    fn a_checkout_with_no_config_at_all_is_one_line_on_the_thread_and_no_question() {
        // The other answer `load_key_binding` has, and the reason the question
        // is only asked on `Ok(None)`: there is no config to write a binding
        // into, so the engine's own sentence about that goes on the thread and
        // nothing comes up.
        let root = a_dir();
        let home = a_dir();
        let mut puller = puller(Boarding::unopened(), checkout(), home.path());
        let mut app = App::default();

        let prompt = asking(&mut app, &mut puller, root.path());

        let source = load_key_binding(home.path(), root.path(), SCOPE)
            .expect_err("a checkout nobody configured");
        assert_eq!(notes(&app), vec![one_line(Error::Config { source })]);
        assert_eq!(prompt, KeyPrompt::Closed, "a question was asked anyway");
        assert!(!puller.choosing(), "a refusal read a queue");
    }
}

#[test]
fn a_failure_in_the_run_lands_as_one_line_and_leaves_the_panel_running() {
    let ground = Ground::new();
    let repo = checkout();
    // The board turning down the move to review, which is the run's last step.
    let written = working();
    let forge = Forging::opening(URL);
    let mut puller = puller(
        &ground,
        board(queue([ready()])).refuse_from(Op::MoveIssue, 1, "Entity not found"),
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
        said.contains("Entity not found"),
        "{said:?} is not the failure the board gave"
    );
    assert_eq!(said, one_line(said.trim()), "the failure is not one line");
    // Nothing came down with it: the run is over, the next `/pull` is allowed, the
    // work the run did commit is committed, and the thread still holds it.
    assert!(!puller.pulling(), "a failure left the run in flight");
    assert_eq!(repo.commits().len(), 2, "{:?}", repo.commits());
    assert!(
        !sections(&app).is_empty(),
        "a failure took the run's phases down"
    );
    assert_eq!(
        forge.asked().len(),
        1,
        "the pull request came before the board"
    );
}
