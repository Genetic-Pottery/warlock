use std::fs;
use std::path::Path;

use tempfile::TempDir;
use warlock_engine::{
    Manifest, PactEntry, PullRun, RunStatus, ScopeRecord, brief_path, pulls_dir, save_key,
    save_key_binding, save_sigils, state_path,
};

use super::{Ports, Prepared, Progress, Shared, held, prepare, pulled, shared};
use crate::claude::Activity;
use crate::error::Error;
use crate::error::status_for;
use crate::git::{Dirty, Opened};
use crate::linear::{Assignee, Blocker, NamedIssue, Priority, Queue, QueuedIssue, StateType};
use crate::pulling::Pulled;
use crate::queue::{Reason, Refusal};
use crate::stubs::{
    Boarding, Checkout, Forging, GitCall, Refreshing, Sessions, Slicing, VIEWER, said,
};

// Not a key, and named so that nothing reading this file mistakes it for one: it
// is stored only so that a bound name resolves.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "work";

const SCOPE: &str = "warlock-team";

// A scope this repository records that the machine below does not hold.
const CLOSED: &str = "control-plane";

// A Linear team *key*, which is what a `[[scope]]` record's `team` is.
const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const REVIEW: &str = "In Review";

const DEFAULT: &str = "main";

const TICKET: &str = "WAR-140";

const ISSUE: &str = "issue-140";

const TITLE: &str = "Add `warlock pull <SCOPE>`";

const URL: &str = "https://github.com/team/repo/pull/12";

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// Two recorded scopes, one pacted directory under each, so a crossing and a
// held-but-foreign path are both reachable from one manifest.
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
    PactEntry::new(".", directory, format!("{directory}/.warlock.md"))
        .expect("a relative module path is inside the root")
        .with_scope(scope)
}

/// The home of a machine holding the scope's sigil, with a name bound and a key
/// stored under it: everything a pull needs, under a directory of this test's own.
fn a_home(root: &Path) -> TempDir {
    let home = a_dir();
    holding(home.path(), root, &[SCOPE]);
    save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

fn holding(home: &Path, root: &Path, sigils: &[&str]) {
    let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home, root, &sigils).expect("a config that writes");
}

/// The ground every test below stands on: the two temporary directories and the
/// manifest, so nothing here reads the developer's own repository or key store.
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

    fn prepared(&self) -> Prepared<'_> {
        prepare(&self.manifest, self.root.path(), self.home.path(), SCOPE)
            .expect("the scope is recorded, held, and has a key bound")
    }

    fn saved(&self) -> PullRun {
        PullRun::load(self.home.path(), self.root.path(), TICKET).expect("the run wrote its record")
    }

    /// Whether this checkout holds any run record at all, which is how a dry run
    /// says it wrote nothing.
    fn any_record(&self) -> bool {
        pulls_dir(self.home.path(), self.root.path()).exists()
    }
}

fn issue(identifier: &str, title: &str, state: &str, kind: &str) -> QueuedIssue {
    QueuedIssue::new(
        format!("issue-{}", identifier.to_lowercase()),
        identifier,
        title,
        state,
        StateType::new(kind),
        Priority::High,
        Vec::new(),
    )
}

/// The ticket every full run below works: `In Progress` is not it — a fresh
/// ticket sits in an unstarted state and nothing on this machine holds a run.
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

fn blocked(identifier: &str, by: &str, holder: Option<&str>) -> QueuedIssue {
    QueuedIssue::new(
        format!("issue-{}", identifier.to_lowercase()),
        identifier,
        "Something waiting",
        "Todo",
        StateType::new("unstarted"),
        Priority::High,
        vec![Blocker::new(by, holder, StateType::new("started"))],
    )
}

fn queue(issues: impl IntoIterator<Item = QueuedIssue>) -> Queue {
    Queue::new(issues.into_iter().collect())
}

/// A board that answers the queue, whose viewer every ticket below is assigned to.
fn board(queue: Queue) -> Boarding {
    Boarding::filing(URL).queueing(queue)
}

fn named(issue: QueuedIssue) -> NamedIssue {
    NamedIssue::new(
        issue,
        TEAM,
        vec![LABEL.to_owned()],
        Some(Assignee::new(VIEWER, "Ada")),
    )
}

/// A halted run for `ticket` on this machine, which is what makes selection pass
/// its issue over and name `warlock resume`.
fn halting(ground: &Ground, ticket: &str) {
    let mut run = PullRun::new(
        ticket,
        "A ticket somebody stopped",
        SCOPE,
        "war-141/a-ticket-somebody-stopped",
        "2026-09-28T09:00:00+00:00",
    );
    run.set_status(RunStatus::Halted);
    run.save(ground.home.path(), ground.root.path())
        .expect("a record that writes");
}

/// One modified path, which is a tree with something in it.
fn wrote(path: &str) -> Vec<Dirty> {
    vec![Dirty {
        code: " M".to_owned(),
        path: path.to_owned(),
        from: None,
    }]
}

/// The whole command, less the environment and with the seams written down: what
/// `pull` spends once it has resolved a repository, a home and a scope.
fn pull_with(
    ground: &Ground,
    prepared: &Prepared<'_>,
    ports: &Ports<'_, Boarding, Checkout, Forging, Slicing, Sessions>,
    ticket: Option<&str>,
    dry_run: bool,
) -> (Result<(), Error>, String) {
    let progress = shared(Progress::new(
        Vec::new(),
        ground.home.path(),
        ground.root.path(),
    ));
    let outcome = pulled(
        &ground.manifest,
        prepared,
        ticket,
        dry_run,
        ports,
        &progress,
    );

    (outcome, printed(&progress))
}

fn printed(progress: &Shared<Vec<u8>>) -> String {
    String::from_utf8(held(progress).written().clone()).expect("warlock writes its own text")
}

/// A pull that works nothing: the sessions and the split are written down as
/// unanswerable, so a run that reached either panics rather than failing an
/// assertion about what it printed.
fn no_sessions() -> Sessions {
    Sessions::answering([])
}

fn no_split() -> Slicing {
    Slicing::into_chain(TICKET, &["Nothing this test lets a run reach"])
}

// The ordinary branch: nothing pacted was made stale, so the pass has nothing to
// refresh and nothing to commit. What the pass itself does with a stale directory
// is `freshness.rs`'s to test, and what the loop does with the answer is
// `pulling.rs`'s.
fn no_refresh() -> Refreshing {
    Refreshing::quiet()
}

#[test]
fn a_scope_no_record_holds_is_refused_and_the_recorded_scopes_are_named() {
    let ground = Ground::new();

    let error = prepare(
        &ground.manifest,
        ground.root.path(),
        ground.home.path(),
        "billing",
    )
    .expect_err("nothing records `billing`");

    assert!(
        matches!(&error, Error::UnrecordedScope { scope, recorded }
            if scope == "billing" && recorded == &[SCOPE.to_owned(), CLOSED.to_owned()]),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(said.contains("`billing`"), "{said}");
    // Both recorded names, because the scope that was typed is almost certainly a
    // near miss against one of them.
    assert!(said.contains("`warlock-team`"), "{said}");
    assert!(said.contains("`control-plane`"), "{said}");
    // The ordinary refusal and not the boundary's: a manifest one line short is a
    // file in this repository to fix.
    assert_eq!(status_for(&Err(error)), 1);
}

#[test]
fn a_recorded_scope_with_no_record_of_any_kind_says_so_in_words() {
    let root = a_dir();
    let home = a_home(root.path());
    let empty = Manifest::new();

    let error = prepare(&empty, root.path(), home.path(), SCOPE).expect_err("nothing is recorded");

    let said = error.to_string();
    assert!(said.contains("no `[[scope]]` record at all"), "{said}");
}

#[test]
fn a_scope_this_machine_does_not_hold_is_the_boundary_s_status() {
    let ground = Ground::new();

    let error = prepare(
        &ground.manifest,
        ground.root.path(),
        ground.home.path(),
        CLOSED,
    )
    .expect_err("this machine holds no `control-plane` sigil");

    assert!(
        matches!(&error, Error::UnheldScope { scope, held }
            if scope == CLOSED && held == &[SCOPE.to_owned()]),
        "{error:?}"
    );
    let said = error.to_string();
    // The scope wanted and the sigils held, because the usual cause of this is a
    // typo against a sigil this machine already has.
    assert!(said.contains("`control-plane`"), "{said}");
    assert!(said.contains("`warlock-team`"), "{said}");
    assert!(said.contains("warlock config"), "{said}");
    // The register the writes already refuse in, and the reason the record
    // question is asked separately from this one: `resolve_filing` would have
    // answered "no such candidate" and spent a 1 on a boundary.
    assert_eq!(status_for(&Err(error)), 3);
}

#[test]
fn a_machine_holding_the_wildcard_may_pull_under_any_recorded_scope() {
    let ground = Ground::new();
    holding(ground.home.path(), ground.root.path(), &["*"]);

    // `scope_opens_to` and nothing written in `pull.rs`, which is what makes the
    // wildcard work here without being spelled out again.
    let prepared = prepare(
        &ground.manifest,
        ground.root.path(),
        ground.home.path(),
        CLOSED,
    )
    .expect("the wildcard holds every recorded scope");

    assert_eq!(prepared.record().name(), CLOSED);
}

#[test]
fn a_checkout_with_no_key_bound_is_refused_the_way_a_push_and_a_draft_are() {
    let root = a_dir();
    let home = a_dir();
    holding(home.path(), root.path(), &[SCOPE]);
    let manifest = a_manifest();

    let error =
        prepare(&manifest, root.path(), home.path(), SCOPE).expect_err("no key is bound here");

    assert!(matches!(error, Error::Filing { .. }), "{error:?}");
    // The engine's own sentence, which is the one a push and a draft print: the
    // fix is the same command, so the wording is not warlock's to say twice.
    let said = error.to_string();
    assert!(said.contains("warlock key use"), "{said}");
    assert_eq!(status_for(&Err(error)), 1);
}

#[test]
fn a_dirty_tree_is_refused_before_the_board_is_opened() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let repo = Checkout::clean(DEFAULT).trees([wrote("crates/engine/src/lib.rs")]);

    // A board that panics if the key is read at all, which is how this test says
    // the tree is asked about before anything else.
    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &Boarding::unopened(),
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    let error = outcome.expect_err("the tree is not clean");
    assert!(
        matches!(&error, Error::DirtyTree { dirty } if dirty == &wrote("crates/engine/src/lib.rs")),
        "{error:?}"
    );
    // What is dirty, in `git status`'s own code, because the reader is about to
    // run `git status` themselves.
    let said = error.to_string();
    assert!(said.contains("` M crates/engine/src/lib.rs`"), "{said}");
    assert_eq!(status_for(&Err(error)), 1);

    assert_eq!(repo.calls(), vec![GitCall::Dirty]);
    assert_eq!(printed, "");
    assert!(!ground.any_record());
}

#[test]
fn a_dry_run_names_the_ticket_it_would_take_and_everything_it_passed_over() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    halting(&ground, "WAR-141");
    let board = board(queue([
        ready(),
        issue(
            "WAR-141",
            "A ticket somebody stopped",
            "In Progress",
            "started",
        ),
        issue("WAR-142", "Waiting on a human", REVIEW, "started"),
        issue("WAR-143", "Somebody else's run", "In Progress", "started"),
        blocked("WAR-144", "WAR-12", Some("Ada")),
    ]));
    let repo = Checkout::clean(DEFAULT);

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board,
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        None,
        true,
    );

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(
        printed.contains(&format!("warlock: would pull `{TICKET}`")),
        "{printed}"
    );
    assert!(printed.contains("nothing was written"), "{printed}");
    // Every reason the queue has, in the queue's own words rather than reworded
    // here: a halted run names the command that frees it, and a block names the
    // identifier and whose it is.
    assert!(
        printed.contains("passed over `WAR-141` — halted — `warlock resume WAR-141` releases it"),
        "{printed}"
    );
    assert!(
        printed.contains("passed over `WAR-142` — in `In Review`, which is waiting on a human"),
        "{printed}"
    );
    assert!(
        printed.contains(
            "passed over `WAR-143` — in progress elsewhere — this machine holds no run record for \
             it"
        ),
        "{printed}"
    );
    assert!(
        printed.contains("passed over `WAR-144` — blocked by WAR-12 (Ada)"),
        "{printed}"
    );

    // Nothing was written, no `git` ran and no session was raised: the two
    // stand-ins panic when they are reached, and the record and the tree say the
    // rest.
    assert!(repo.calls().is_empty(), "a dry run ran `git`");
    assert!(no_split().asked().is_empty());
    // The halted run's own record is the only thing under the home: the ticket a
    // dry run would have taken has none.
    assert!(
        !state_path(ground.home.path(), ground.root.path(), TICKET).exists(),
        "a dry run wrote a record for the ticket it would have taken"
    );
}

#[test]
fn a_dry_run_on_a_named_ticket_says_whether_that_ticket_would_be_taken() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let repo = Checkout::clean(DEFAULT);

    let (taken, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &Boarding::filing(URL).naming(named(ready())),
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        Some(TICKET),
        true,
    );

    assert!(taken.is_ok(), "{taken:?}");
    assert!(
        printed.contains(&format!("would pull `{TICKET}`")),
        "{printed}"
    );

    // And the same ticket with a halted run behind it, which is the one refusal
    // `--ticket` shares with the chooser.
    halting(&ground, TICKET);
    let halted = named(issue(TICKET, TITLE, "In Progress", "started"));
    let (refused, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &Boarding::filing(URL).naming(halted),
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        Some(TICKET),
        true,
    );

    assert!(refused.is_ok(), "a dry run refuses nothing: {refused:?}");
    assert!(
        printed.contains(&format!(
            "would not pull `{TICKET}`: halted — `warlock resume {TICKET}` releases it"
        )),
        "{printed}"
    );
    assert!(repo.calls().is_empty(), "a dry run ran `git`");
}

#[test]
fn nothing_ready_names_every_ticket_and_why_and_is_an_answer() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let board = board(queue([
        issue("WAR-142", "Waiting on a human", REVIEW, "started"),
        blocked("WAR-144", "WAR-12", None),
    ]));

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board,
            repo: &Checkout::clean(DEFAULT),
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    // A **0**: the queue was read and answered, and an empty answer is an answer.
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(status_for(&outcome), 0);
    assert!(
        printed.contains("passed over `WAR-142` — in `In Review`, which is waiting on a human"),
        "{printed}"
    );
    assert!(
        printed.contains("passed over `WAR-144` — blocked by WAR-12 (unassigned)"),
        "{printed}"
    );
    assert!(
        printed.contains("nothing in the queue for `warlock-team` is ready to work"),
        "{printed}"
    );
    assert!(!ground.any_record());
}

#[test]
fn a_named_ticket_on_a_halted_run_refuses_with_the_command_that_frees_it() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    halting(&ground, TICKET);
    let halted = named(issue(TICKET, TITLE, "In Progress", "started"));

    let (outcome, _) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &Boarding::filing(URL).naming(halted),
            repo: &Checkout::clean(DEFAULT),
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        Some(TICKET),
        false,
    );

    let error = outcome.expect_err("a halted run is not pulled again");
    assert!(
        matches!(&error, Error::NotPulled { ticket, refusal }
            if ticket == TICKET && matches!(refusal, Refusal::NotReady(Reason::Halted { .. }))),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(
        said.contains(&format!("`warlock resume {TICKET}`")),
        "{said}"
    );
    assert_eq!(status_for(&Err(error)), 1);
}

#[test]
fn a_whole_pull_prints_a_header_per_section_with_the_session_s_lines_under_it() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    // Clean for the door's own look at the tree, and holding the session's work
    // for every look after it.
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/engine/src/lib.rs")]);
    let forge = Forging::opening(URL);
    let split = Slicing::into_chain(TICKET, &["Read the queue", "Work the ticket"]);

    let progress = shared(Progress::new(
        Vec::new(),
        ground.home.path(),
        ground.root.path(),
    ));
    let sessions = Sessions::answering([
        said("done", "the queue is read", None),
        said("done", "the ticket is worked", None),
    ])
    .reporting(
        super::watching(&progress),
        [
            Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("crates/engine/src/lib.rs".to_owned()),
            },
            Activity::Thinking,
            // No line at all, following the panel's account card: a cost is a fact
            // about the pass rather than something the pass did.
            Activity::Cost { usd: 0.42 },
        ],
    );

    let outcome = pulled(
        &ground.manifest,
        &prepared,
        None,
        false,
        &Ports {
            open: &board(queue([ready()])),
            repo: &repo,
            forge: &forge,
            split: &split,
            sessions: &sessions,
            freshen: &no_refresh(),
        },
        &progress,
    );
    let printed = printed(&progress);

    assert!(outcome.is_ok(), "{outcome:?}: {printed}");
    // A header per section, the fraction one-based over the split's own answer.
    assert!(
        printed.contains(&format!("warlock: splitting `{TICKET}` — {TITLE}")),
        "{printed}"
    );
    assert!(
        printed.contains("warlock: [1/2] `WAR-140.01` Read the queue"),
        "{printed}"
    );
    assert!(
        printed.contains("warlock: [2/2] `WAR-140.02` Work the ticket"),
        "{printed}"
    );
    assert!(
        printed.contains("is pushed, opening a pull request"),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("`{TICKET}` is in review: {URL}")),
        "{printed}"
    );
    // The session's own lines, under the header they happened under.
    assert!(
        printed.contains("warlock: Read crates/engine/src/lib.rs"),
        "{printed}"
    );
    assert!(printed.contains("warlock: thinking"), "{printed}");
    assert!(!printed.contains("0.42"), "a cost was printed: {printed}");

    // And each sub-task's activity under the execution log heading of its own
    // brief, which the saves either side of a session leave untouched.
    for id in ["WAR-140.01", "WAR-140.02"] {
        let brief = fs::read_to_string(brief_path(
            ground.home.path(),
            ground.root.path(),
            TICKET,
            id,
        ))
        .expect("the run rendered the sub-task's brief");
        assert_eq!(brief.matches("## Execution log").count(), 1, "{brief}");
        let (_, log) = brief
            .split_once("## Execution log")
            .expect("the heading is in the brief");
        assert!(log.contains("- Read crates/engine/src/lib.rs"), "{brief}");
        assert!(log.contains("- thinking"), "{brief}");
    }

    assert_eq!(ground.saved().pr_url(), Some(URL));
}

#[test]
fn a_stretch_of_thinking_or_writing_is_one_line_and_a_repeated_tool_is_not_collapsed() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/engine/src/lib.rs")]);
    let forge = Forging::opening(URL);
    let split = Slicing::into_chain(TICKET, &["Work the ticket"]);
    let read = || Activity::Tool {
        name: "Read".to_owned(),
        detail: Some("crates/engine/src/lib.rs".to_owned()),
    };

    let progress = shared(Progress::new(
        Vec::new(),
        ground.home.path(),
        ground.root.path(),
    ));
    let sessions = Sessions::answering([said("done", "the ticket is worked", None)]).reporting(
        super::watching(&progress),
        [
            Activity::Thinking,
            Activity::Thinking,
            Activity::Writing { bytes: 0 },
            Activity::Writing { bytes: 400 },
            Activity::Writing { bytes: 900 },
            read(),
            read(),
            Activity::Writing { bytes: 0 },
            Activity::Writing { bytes: 120 },
        ],
    );

    let outcome = pulled(
        &ground.manifest,
        &prepared,
        None,
        false,
        &Ports {
            open: &board(queue([ready()])),
            repo: &repo,
            forge: &forge,
            split: &split,
            sessions: &sessions,
            freshen: &no_refresh(),
        },
        &progress,
    );
    let printed = printed(&progress);

    assert!(outcome.is_ok(), "{outcome:?}: {printed}");
    assert_eq!(
        printed.matches("warlock: thinking\n").count(),
        1,
        "{printed}"
    );
    // Once per stretch: the tool lines between the two stretches end the first.
    assert_eq!(
        printed.matches("warlock: writing\n").count(),
        2,
        "{printed}"
    );
    assert_eq!(
        printed
            .matches("warlock: Read crates/engine/src/lib.rs\n")
            .count(),
        2,
        "{printed}"
    );
    assert!(!printed.contains("writing ·"), "{printed}");

    let brief = fs::read_to_string(brief_path(
        ground.home.path(),
        ground.root.path(),
        TICKET,
        "WAR-140.01",
    ))
    .expect("the run rendered the sub-task's brief");
    assert_eq!(brief.matches("- writing\n").count(), 2, "{brief}");
}

#[test]
fn a_finished_pull_on_a_machine_with_no_gh_says_where_the_body_went() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/engine/src/lib.rs")]);

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board(queue([ready()])),
            repo: &repo,
            forge: &Forging::without_gh(),
            split: &Slicing::into_chain(TICKET, &["Read the queue"]),
            sessions: &Sessions::answering([said("done", "the queue is read", None)]),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    // A **0** all the same: the work is done and pushed, and what is missing is a
    // program rather than a step of the run.
    assert!(outcome.is_ok(), "{outcome:?}: {printed}");
    assert_eq!(status_for(&outcome), 0);
    assert!(
        printed.contains("there is no `gh` on this machine"),
        "{printed}"
    );
    assert_eq!(ground.saved().pr_url(), None);
    assert_eq!(Opened::NoGh.url(), None);
}

#[test]
fn a_crossed_sub_task_is_the_boundary_s_status_and_the_loop_and_the_shell_agree() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    // The session wrote under the scope this machine does not hold.
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/control/src/lib.rs")]);
    let forge = Forging::opening(URL);

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board(queue([ready()])),
            repo: &repo,
            forge: &forge,
            split: &Slicing::into_chain(TICKET, &["Read the queue"]),
            sessions: &Sessions::answering([said("done", "the queue is read", None)]),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    let error = outcome.expect_err("a crossing stops the run");
    assert!(
        matches!(&error, Error::Crossed { ticket, subtask }
            if ticket == TICKET && subtask == "WAR-140.01"),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(said.contains("`WAR-140.01`"), "{said}");
    assert!(said.contains("scope this machine does not hold"), "{said}");

    // The two statements of what each ending is worth, held against each other:
    // the loop's own, and the one the shell actually returns.
    assert_eq!(status_for(&Err(error)), 3);
    assert_eq!(
        Pulled::Crossed {
            ticket: TICKET.to_owned(),
            subtask: "WAR-140.01".to_owned(),
        }
        .status(),
        3
    );
    assert_eq!(
        status_for(&Err(Error::Halted {
            ticket: TICKET.to_owned()
        })),
        1
    );
    assert_eq!(
        Pulled::Halted {
            ticket: TICKET.to_owned()
        }
        .status(),
        1
    );
    assert_eq!(status_for(&Ok(())), 0);
    assert_eq!(
        Pulled::Opened {
            ticket: TICKET.to_owned(),
            url: Some(URL.to_owned()),
        }
        .status(),
        0
    );

    // Nothing was committed, the tree was left as the session left it, and no pull
    // request was asked for.
    assert!(repo.commits().is_empty(), "{printed}");
    assert!(forge.asked().is_empty());
}

#[test]
fn a_halted_run_leaves_the_ticket_where_it_is_and_the_shell_spends_a_one() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let repo = Checkout::clean(DEFAULT);

    let (outcome, _) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board(queue([ready()])),
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &Slicing::into_chain(TICKET, &["Read the queue"]),
            sessions: &Sessions::answering([said(
                "blocked",
                "",
                Some("only a person can settle this"),
            )]),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    let error = outcome.expect_err("a run with nothing runnable halts");
    assert!(
        matches!(&error, Error::Halted { ticket } if ticket == TICKET),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(
        said.contains(&format!("`warlock resume {TICKET}`")),
        "{said}"
    );
    assert_eq!(status_for(&Err(error)), 1);
    assert_eq!(ground.saved().status(), RunStatus::Halted);
    assert!(
        state_path(ground.home.path(), ground.root.path(), TICKET).exists(),
        "a halt left no record to resume from"
    );
}

#[test]
fn an_unreadable_run_record_is_a_line_and_not_a_failure() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    // A record that exists and will not parse, which the scan names and walks
    // past: the run behind it may be holding uncommitted work.
    let broken = pulls_dir(ground.home.path(), ground.root.path()).join("WAR-9");
    fs::create_dir_all(&broken).expect("a run directory");
    fs::write(broken.join("state.json"), "{ not json").expect("a broken record");

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board(queue([])),
            repo: &Checkout::clean(DEFAULT),
            forge: &Forging::opening(URL),
            split: &no_split(),
            sessions: &no_sessions(),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(printed.contains("WAR-9"), "{printed}");
    assert!(
        printed.contains("nothing in the queue for `warlock-team` is ready to work"),
        "{printed}"
    );
}

#[test]
fn a_worked_ticket_is_the_one_the_queue_chose_and_the_board_was_asked_the_scope_s_own_words() {
    let ground = Ground::new();
    let prepared = ground.prepared();
    let board = board(queue([ready().with_description("The queue is unread.")]));
    let repo = Checkout::clean(DEFAULT).trees([Vec::new(), wrote("crates/engine/src/lib.rs")]);
    let split = Slicing::into_chain(TICKET, &["Read the queue"]);

    let (outcome, printed) = pull_with(
        &ground,
        &prepared,
        &Ports {
            open: &board,
            repo: &repo,
            forge: &Forging::opening(URL),
            split: &split,
            sessions: &Sessions::answering([said("done", "the queue is read", None)]),
            freshen: &no_refresh(),
        },
        None,
        false,
    );

    assert!(outcome.is_ok(), "{outcome:?}: {printed}");
    // The split was asked about the ticket the queue chose, with the title and
    // the description the board gave: what `forman pull` hands every sub-task
    // as the parent ticket.
    let asked = split.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].ticket, TICKET);
    assert_eq!(asked[0].title, TITLE);
    assert_eq!(asked[0].description, "The queue is unread.");
    // And the branch carries the number out of the identifier.
    assert!(
        ground.saved().branch().starts_with("war-140/"),
        "{:?}",
        ground.saved().branch()
    );
}
