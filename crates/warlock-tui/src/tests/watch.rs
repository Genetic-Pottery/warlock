use std::time::{Duration, Instant};

use warlock_engine::{Node, NodeState, Tree};

use super::{COALESCED_RELOADS, NodeSet, QUIET_PERIOD, RELOAD_CEILING, WatchPolicy};

// Finer than the real event loop's 100ms poll, so a test asserting *when* a
// reload happened is not really asserting where the tick boundaries fell.
const TICK_MS: u64 = 10;

// A deadline is noticed on the first tick after it passes, never on the
// instant itself, so tick-driven assertions need this much slack.
const SLACK: Duration = Duration::from_millis(TICK_MS * 2);

const LONG_AFTER: Duration = Duration::from_secs(30);

// Milliseconds after the start, and the path the event was about.
type Event = (u64, &'static str);

// `target/` and `.git/` are deliberately absent: a walk never produces them,
// and their absence is the whole of the filter.
fn walked() -> Tree {
    Tree::new(
        Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([Node::new(
            "repo/crates",
            None,
            NodeState::Unpacted,
        )
        .with_children([Node::new(
            "repo/crates/engine",
            "repo/crates/engine/WARLOCK.md",
            NodeState::PactedFresh,
        )])]),
    )
}

// The event loop minus the loop, returning the millisecond offsets it
// reloaded at. A reload occupies `reload_ms` of simulated time and events
// keep arriving through it, which is how the coalescing rule is exercised
// with no thread, no watcher and no sleeping.
fn drive(policy: &mut WatchPolicy, events: &[Event], until_ms: u64, reload_ms: u64) -> Vec<u64> {
    let base = Instant::now();
    let at = |ms: u64| base + Duration::from_millis(ms);
    let mut reloads = Vec::new();
    let mut next = 0;
    let mut ms = 0;

    while ms <= until_ms {
        while let Some(&(when, path)) = events.get(next)
            && when <= ms
        {
            policy.saw(path, at(when));
            next += 1;
        }

        if policy.due(at(ms)) {
            reloads.push(ms);
            policy.reload_started();
            // The reload runs, and the events landing inside it are the
            // ones the coalescing flag exists for.
            let ends = ms + reload_ms;
            while let Some(&(when, path)) = events.get(next)
                && when <= ends
            {
                policy.saw(path, at(when));
                next += 1;
            }
            policy.reload_finished(at(ends));
            ms = ends;
        }

        ms += TICK_MS;
    }

    reloads
}

// So a timing assertion can be written against the constants themselves
// rather than against a number copied out of them.
fn gap(from_ms: u64, to_ms: u64) -> Duration {
    Duration::from_millis(to_ms - from_ms)
}

#[test]
fn a_path_under_a_walked_directory_is_accepted() {
    let watched = NodeSet::from_tree(&walked());

    assert!(watched.accepts("repo/README.md"));
    assert!(watched.accepts("repo/crates/engine/WARLOCK.md"));
    assert!(
        watched.accepts("repo/crates/engine/src"),
        "a path is a path: the filter never asks whether it is a file",
    );
    assert_eq!(watched.len(), 3);
    assert!(!watched.is_empty());
}

#[test]
fn a_path_under_a_directory_no_walk_produced_is_rejected() {
    let watched = NodeSet::from_tree(&walked());

    // The two directories this whole rule exists for. Neither is in the
    // set, because the loader's own walk never yielded them, so nothing
    // beneath them has a parent in the set — at any depth.
    assert!(!watched.accepts("repo/target/debug/warlock"));
    assert!(!watched.accepts("repo/target/debug/deps/libwarlock.rlib"));
    assert!(!watched.accepts("repo/.git/index"));
    assert!(!watched.accepts("repo/.git/objects/ab/cdef"));

    // The near miss: the *grandparent* is walked, and that is not enough.
    assert!(!watched.accepts("repo/crates/engine/src/lib.rs"));

    // Nothing outside the tree, and no path without a parent.
    assert!(!watched.accepts("/elsewhere/notes.md"));
    assert!(!watched.accepts("/"));
    assert!(
        !watched.accepts("repo"),
        "the root's own parent is above everything the load produced",
    );
}

#[test]
fn a_brand_new_directory_whose_parent_is_walked_is_accepted() {
    let watched = NodeSet::from_tree(&walked());

    // Nothing in the set is called this — that is the point. A directory
    // created inside a walked one is accepted, so the tree is read again
    // and the new directory becomes a node with a row of its own.
    assert!(!watched.accepts("repo/crates/tui/WARLOCK.md"));
    assert!(watched.accepts("repo/crates/tui"));
}

#[test]
fn the_filter_is_replaced_by_the_tree_a_reload_produced() {
    let mut policy = WatchPolicy::new(&walked());
    assert!(!policy.watched().accepts("repo/crates/tui/WARLOCK.md"));

    // The reload the new directory triggered found it, and from now on its
    // contents count too.
    let mut root = walked().root;
    root.children[0]
        .children
        .push(Node::new("repo/crates/tui", None, NodeState::Unpacted));
    policy.follow(&Tree::new(root));

    assert!(policy.watched().accepts("repo/crates/tui/WARLOCK.md"));
    assert_eq!(policy.watched().len(), 4);
}

#[test]
fn one_editor_save_is_one_reload() {
    let mut policy = WatchPolicy::new(&walked());

    // What saving a file in an editor actually looks like: a write to a
    // temporary, a rename over the original, a chmod behind it, all inside
    // a few milliseconds of each other.
    let save = [
        (0, "repo/crates/engine/.WARLOCK.md.swp"),
        (4, "repo/crates/engine/WARLOCK.md"),
        (11, "repo/crates/engine/WARLOCK.md"),
    ];
    let reloads = drive(&mut policy, &save, 3_000, 0);

    assert_eq!(reloads.len(), 1, "one save, one reload: {reloads:?}");
    let waited = gap(11, reloads[0]);
    assert!(
        waited >= QUIET_PERIOD && waited < QUIET_PERIOD + SLACK,
        "reloaded {waited:?} after the last event of the burst",
    );
    assert!(!policy.owes_reload(), "the burst is discharged");
}

#[test]
fn events_more_than_a_quiet_period_apart_are_two_reloads() {
    let mut policy = WatchPolicy::new(&walked());

    // Two saves, a second apart: the debounce coalesces a burst, not a
    // session.
    let saves = [(0, "repo/README.md"), (1_000, "repo/README.md")];
    let reloads = drive(&mut policy, &saves, 3_000, 0);

    assert_eq!(reloads.len(), 2, "{reloads:?}");
}

#[test]
fn a_continuous_stream_reloads_at_the_ceiling_rather_than_every_quarter_second() {
    // How far apart the stream's events are.
    const SPACING: Duration = Duration::from_millis(50);

    let mut policy = WatchPolicy::new(&walked());

    // Ten seconds of events every 50ms — a `git checkout` of a long branch,
    // or a formatter run over the repository. The disk is never quiet for a
    // quarter second, so the debounce alone would never fire at all.
    let stream: Vec<Event> = (0..200)
        .map(|n| (n * 50, "repo/crates/engine/WARLOCK.md"))
        .collect();
    let reloads = drive(&mut policy, &stream, 10_000, 0);

    assert!(
        !reloads.is_empty(),
        "a stream that never goes quiet still has to reload",
    );
    // Roughly one per ceiling over ten seconds, and nothing like one per
    // quiet period, which would be forty.
    assert!(
        (4..=6).contains(&reloads.len()),
        "reloaded {} times in ten seconds: {reloads:?}",
        reloads.len(),
    );

    let mut previous = 0;
    for &reload in &reloads {
        let between = gap(previous, reload);
        // The ceiling is measured from the first event of the burst, and
        // the first event after a reload is up to one spacing behind it, so
        // the interval is the ceiling plus that and the tick it was noticed
        // on — never less than the ceiling, and never a multiple of it.
        assert!(
            between >= RELOAD_CEILING && between < RELOAD_CEILING + SPACING + SLACK,
            "reloaded {between:?} after the last one: {reloads:?}",
        );
        assert!(
            between > QUIET_PERIOD * 2,
            "a stream reloading every {between:?} is the debounce misapplied: {reloads:?}",
        );
        previous = reload;
    }
}

#[test]
fn events_during_a_reload_cause_exactly_one_further_reload() {
    let mut policy = WatchPolicy::new(&walked());

    // One save settles and a reload starts; the reload takes half a second,
    // and a dozen more events land while it runs — the documents a pact
    // wrote, say. However many they are, they are worth one more reload.
    let mut events = vec![(0, "repo/README.md")];
    events.extend((0..12).map(|n| (300 + n * 20, "repo/crates/engine/WARLOCK.md")));
    let reloads = drive(&mut policy, &events, 5_000, 500);

    assert_eq!(
        reloads.len(),
        1 + COALESCED_RELOADS,
        "the reload plus exactly one more: {reloads:?}",
    );
    assert!(!policy.owes_reload(), "no queue is left behind");

    // And the second reload is the flag's, not a per-event backlog: it
    // comes one quiet period after the first one ended, rather than twelve
    // reloads deep.
    let waited = gap(reloads[0] + 500, reloads[1]);
    assert!(
        waited >= QUIET_PERIOD && waited < QUIET_PERIOD + SLACK,
        "the coalesced reload came {waited:?} after the first one ended: {reloads:?}",
    );
}

#[test]
fn a_thousand_events_during_one_reload_are_still_worth_one() {
    let mut policy = WatchPolicy::new(&walked());
    let base = Instant::now();

    policy.saw("repo/README.md", base);
    assert!(policy.due(base + QUIET_PERIOD));
    policy.reload_started();

    for n in 0..1_000u64 {
        policy.saw(
            "repo/crates/engine/WARLOCK.md",
            base + Duration::from_millis(n),
        );
    }
    assert!(
        policy.owes_reload(),
        "the flag is set while the reload is in flight",
    );
    policy.reload_finished(base + Duration::from_secs(1));

    // One further reload, and then nothing: a flag, not a count.
    assert!(policy.due(base + Duration::from_secs(1) + QUIET_PERIOD));
    policy.reload_started();
    policy.reload_finished(base + Duration::from_secs(2));
    assert!(!policy.owes_reload());
    assert!(!policy.due(base + LONG_AFTER));
}

#[test]
fn a_reload_with_nothing_moving_under_it_is_the_last_one() {
    let mut policy = WatchPolicy::new(&walked());
    let base = Instant::now();

    policy.saw("repo/README.md", base);
    assert!(policy.due(base + QUIET_PERIOD));
    policy.reload_started();
    assert!(
        !policy.due(base + QUIET_PERIOD + RELOAD_CEILING),
        "nothing is due while a reload is in progress, ceiling or no ceiling",
    );
    policy.reload_finished(base + QUIET_PERIOD + Duration::from_millis(400));

    assert!(!policy.owes_reload());
    assert!(!policy.due(base + LONG_AFTER));
}

#[test]
fn a_rejected_event_is_worth_nothing_at_all() {
    let mut policy = WatchPolicy::new(&walked());
    let base = Instant::now();

    // A build writing thousands of files into a directory no walk produced.
    for n in 0..5_000u64 {
        let accepted = policy.saw(
            "repo/target/debug/deps/warlock.o",
            base + Duration::from_millis(n),
        );
        assert!(!accepted, "event {n} was accepted");
    }

    assert!(!policy.owes_reload());
    assert!(!policy.due(base + LONG_AFTER));
}

#[test]
fn a_path_accepted_elsewhere_follows_the_same_timing() {
    let mut policy = WatchPolicy::new(&walked());
    let base = Instant::now();

    // The manifest: hidden, so no walk produced its directory, and yet it
    // decides every colour on screen. Whoever knows that calls `accepted`.
    assert!(!policy.saw("repo/.warlock/pacts.toml", base));
    assert!(!policy.owes_reload());

    policy.accepted(base);
    assert!(!policy.due(base));
    assert!(policy.due(base + QUIET_PERIOD));
}

// The only tests in this file that touch a disk. Neither waits for an
// event: how fast the operating system reports a save is not this module's
// business, so all that is asserted is what holds the instant a watcher is
// asked for — that it started, or that not starting came back as a value.
mod live {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{env, fs, process};

    use super::super::{Watch, Watching};

    // Hand-rolled in the style `claude.rs` already uses here, rather than
    // adding this crate's first dev-dependency for two tests.
    fn scratch(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("warlock-watch-{}-{name}-{unique}", process::id()));
        fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
        directory
    }

    #[test]
    fn a_watcher_that_cannot_be_started_is_a_value_and_not_an_error() {
        // A root that is not there is the cheapest way to be refused by the
        // operating system, and it stands in for every other way — no
        // inotify instances left, a filesystem that reports nothing, a
        // platform with no backend at all. What matters is the shape of the
        // answer, which is a variant rather than a `Result`: nothing above
        // has to handle it, and warlock keeps running.
        let directory = scratch("missing");
        let missing = directory.join("gone");

        match Watch::start(&missing, &missing) {
            Watching::Off(reason) => assert!(!reason.is_empty(), "says why, in one line"),
            Watching::Live(_) => panic!("a watcher over a path that is not there"),
        }

        // Best effort: a leftover under `/tmp` is untidy, not a failure.
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_watcher_over_a_real_directory_starts_and_has_nothing_to_say_yet() {
        // No `.warlock/` in it either, which is the ordinary case before the
        // first pact: the manifest cannot be watched, and that is not
        // allowed to cost the tree its watch.
        let root = scratch("root");

        let Watching::Live(mut watch) = Watch::start(&root, &root) else {
            panic!("a watcher over a directory that exists");
        };

        assert!(watch.live());
        // Nothing has written anything, so this is a fact about a quiet
        // disk and not a race with one: the drain never blocks, and there
        // is nothing for it to have picked up.
        assert!(watch.drain().is_empty());
        assert!(watch.live(), "an empty drain is not a dead watcher");

        // Best effort: a leftover under `/tmp` is untidy, not a failure.
        let _ = fs::remove_dir_all(&root);
    }
}
