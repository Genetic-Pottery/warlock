//! When the disk moves, and what to do about it — as values, with no watcher.
//!
//! Warlock's tree is read from disk, and the disk keeps moving after it has
//! been read: a file is saved in another window, a branch is checked out, a
//! build writes ten thousand object files. Two questions follow, and this
//! module answers both without opening a watcher, a terminal or a file.
//!
//! The first is *which* movements are Warlock's business. The answer is the
//! last walk itself: an event counts only when its immediate parent is a
//! directory the last successful load produced. That is what [`NodeSet`] holds
//! — one path per node of a [`Tree`], and nothing else. There is no gitignore
//! matching here, no skip list and no path rules of this module's own, because
//! the loader already applied all of that on the way to the tree it returned;
//! re-deciding it here would be a second implementation of gitignore semantics
//! that could disagree with the first. `target/debug/` and `.git/` are rejected
//! for the one reason that matters: no walk ever produced them, so nothing
//! inside them has a parent in the set.
//!
//! The second is *when* to act. A single editor save is three or four events —
//! a write, a rename over the original, a chmod — and a `git checkout` is
//! thousands, so reloading per event would reload a tree that nobody asked
//! about dozens of times a second. [`WatchPolicy`] is the timing answer, in
//! three rules that are three named constants: [`QUIET_PERIOD`],
//! [`RELOAD_CEILING`] and [`COALESCED_RELOADS`].
//!
//! # It holds no clock
//!
//! [`Instant::now`] is never called in this file. Every instant the policy
//! reasons about is handed in by whoever is driving the event loop — which
//! already reads the clock once a frame — so a ten-second burst of events can be
//! driven through the policy in a test in microseconds, with no sleeping, no
//! watcher and nothing attached to stdout. What the policy answers is one
//! question, [`WatchPolicy::due`]: reload now, or not yet.
//!
//! # What is not here
//!
//! No watcher: nothing in this module subscribes to anything, and no `notify`
//! type appears in its signatures. Nothing reloads either — the policy says
//! *that* a reload is owed and never performs one, because reading the tree
//! again is the binary's business and has to happen on the thread that draws.
//! And no state outlives a process: a [`NodeSet`] is thrown away and rebuilt by
//! every successful load, which is exactly why it is replaceable.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use warlock_engine::Tree;

/// How long the disk has to be quiet before a reload — the debounce.
///
/// What it protects against: one save arriving as several events. An editor
/// writing a file typically writes a temporary, renames it over the original
/// and fixes its mode, and each of those is an event of its own; a reload per
/// event would re-walk and re-hash the tree three or four times for one
/// keystroke in another window. Waiting for a quarter second of silence after
/// the *last* accepted event turns that burst into one reload.
///
/// A quarter second is chosen to sit above the gap between the events of one
/// save — which are milliseconds apart — and below the point where a person
/// reads the screen as not having noticed. It is longer than the event loop's
/// poll interval, so the deadline is checked several times before it passes.
pub const QUIET_PERIOD: Duration = Duration::from_millis(250);

/// The longest a reload is put off while events keep arriving — the ceiling.
///
/// What it protects against: starvation by a burst that never ends. A `git
/// checkout`, a `cargo build` inside a watched directory or a formatter run
/// over the whole repository emits events faster than [`QUIET_PERIOD`], so the
/// quiet the debounce waits for never comes and a purely debounced tree would
/// sit stale for as long as the burst lasts. Once this much time has passed
/// since the first event of the current burst, the tree is read again whether
/// or not the disk has gone quiet.
///
/// A couple of seconds, so that the two failure modes are both out of reach:
/// the tree never goes minutes without moving, and a long burst still reloads a
/// handful of times rather than four times a second.
pub const RELOAD_CEILING: Duration = Duration::from_secs(2);

/// How many further reloads the events arriving during a reload are worth: one,
/// however many of them there were.
///
/// What it protects against: a queue. Reading the tree again takes real time —
/// a walk and a hash per pacted subtree — and the documents a pact writes land
/// during it, so events arrive while the reload that will already see them is
/// still running. Remembering them as a count, or as a list, would leave the
/// loop owing a reload per event and re-walking the tree until the backlog
/// drained. So they are remembered as a flag instead: one bit, set by the first
/// such event and by every one after it, cleared by the single reload it earns.
///
/// Named as a number rather than left implicit in the code because it is the
/// rule, and the rule is that this number is 1 and can never grow.
pub const COALESCED_RELOADS: usize = 1;

/// The directories the last successful load produced: the filter every
/// filesystem event is held against.
///
/// One path per node of the [`Tree`] a load returned, and nothing more. It is
/// built from the tree rather than from what is on screen on purpose: rows are
/// filtered by the pacted-only and file toggles, so a filter built from them
/// would stop noticing directories the moment somebody pressed a key.
///
/// Paths are compared as stored, with no normalisation and no filesystem
/// access, exactly as [`Tree::find`] compares them. The tree's paths come back
/// from the load rooted where the load was rooted, and a watcher reports events
/// under the same root, so the two agree without anybody canonicalising
/// anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeSet {
    /// Every node path of the tree this set was built from.
    directories: HashSet<PathBuf>,
}

impl NodeSet {
    /// The directories of `tree`, taken from its nodes.
    ///
    /// ```
    /// use warlock_engine::{Node, NodeState, Tree};
    /// use warlock_tui::NodeSet;
    ///
    /// let tree = Tree::new(
    ///     Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale)
    ///         .with_children([Node::new("repo/docs", None, NodeState::Unpacted)]),
    /// );
    /// let watched = NodeSet::from_tree(&tree);
    ///
    /// // A file saved in a directory the walk produced.
    /// assert!(watched.accepts("repo/docs/adr.md"));
    /// // A build artefact: no walk ever produced `repo/target/debug`.
    /// assert!(!watched.accepts("repo/target/debug/warlock"));
    /// ```
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        Self {
            directories: tree.walk().map(|(node, _)| node.path.clone()).collect(),
        }
    }

    /// Whether an event about `path` is Warlock's business: `true` when the
    /// path's *immediate* parent is one of these directories.
    ///
    /// Immediate, not any ancestor, and that is the whole of the rule. It
    /// accepts a file written in a directory the walk produced and a directory
    /// created there — a brand-new directory is a path whose parent is in the
    /// set, so the tree is read again and the new directory becomes a node of
    /// its own. It rejects everything reached through a directory the walk
    /// never produced, which is how an ignored subtree is skipped without any
    /// ignore rules being read: nothing under `target/debug/` or `.git/` has a
    /// parent in the set, however deep it sits.
    ///
    /// A path with no parent at all — the filesystem root — is rejected, as is
    /// the tree's own root path, whose parent is above everything the load
    /// produced.
    #[must_use]
    pub fn accepts(&self, path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .parent()
            .is_some_and(|parent| self.directories.contains(parent))
    }

    /// How many directories are in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.directories.len()
    }

    /// Whether the set holds no directories, which is what a policy built
    /// before any load has one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.directories.is_empty()
    }
}

/// The timing half of watching: it is told what happened and when, and answers
/// whether the tree is owed a reload right now.
///
/// It performs nothing. Reading the tree again is the event loop's job, on the
/// thread that draws; this type only says when that is worth doing, from
/// [`QUIET_PERIOD`], [`RELOAD_CEILING`] and [`COALESCED_RELOADS`]. It reads no
/// clock either: every instant it compares against is one the caller handed it.
///
/// The shape of a use of it, once per turn of the event loop:
///
/// ```
/// use std::time::Instant;
///
/// use warlock_engine::{Node, NodeState, Tree};
/// use warlock_tui::WatchPolicy;
///
/// let tree = Tree::new(Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale));
/// let mut policy = WatchPolicy::new(&tree);
///
/// let saved_at = Instant::now();
/// assert!(policy.saw("repo/WARLOCK.md", saved_at), "a file in a walked directory");
/// assert!(!policy.due(saved_at), "not yet: the disk has only just moved");
///
/// let settled = saved_at + warlock_tui::QUIET_PERIOD;
/// assert!(policy.due(settled));
///
/// // The caller reloads, and says so. The tree it loaded becomes the filter.
/// policy.reload_started();
/// policy.follow(&tree);
/// policy.reload_finished(settled);
/// assert!(!policy.due(settled), "nothing is owed until something else moves");
/// ```
#[derive(Debug, Clone)]
pub struct WatchPolicy {
    /// The directories the last successful load produced. Replaced by
    /// [`follow`](WatchPolicy::follow) after every one, because every load
    /// produces a new one and the old one describes a tree that no longer
    /// exists.
    watched: NodeSet,
    /// When the first accepted event of the burst now pending arrived, or
    /// `None` when nothing is pending. [`RELOAD_CEILING`] is measured from it.
    burst_began: Option<Instant>,
    /// When the most recent accepted event of the pending burst arrived.
    /// [`QUIET_PERIOD`] is measured from it. `Some` exactly when
    /// `burst_began` is.
    last_event: Option<Instant>,
    /// Whether a reload is in progress, between
    /// [`reload_started`](WatchPolicy::reload_started) and
    /// [`reload_finished`](WatchPolicy::reload_finished). Nothing is due while
    /// it is.
    reloading: bool,
    /// Whether anything arrived during the reload now in progress: the
    /// [`COALESCED_RELOADS`] flag, one bit rather than a count, so a thousand
    /// events during one reload are worth exactly one more.
    moved_during_reload: bool,
}

impl WatchPolicy {
    /// A policy filtering against the directories of `tree`, owing nothing.
    #[must_use]
    pub fn new(tree: &Tree) -> Self {
        Self {
            watched: NodeSet::from_tree(tree),
            burst_began: None,
            last_event: None,
            reloading: false,
            moved_during_reload: false,
        }
    }

    /// Filter against the directories of `tree` from now on: what a successful
    /// load hands over, since the walk it just did is the new filter.
    ///
    /// Only the filter is replaced. Whatever the policy owes — a burst waiting
    /// out its quiet period, an event seen during a reload — is untouched,
    /// because a new tree says nothing about events that have already been
    /// accepted.
    pub fn follow(&mut self, tree: &Tree) {
        self.watched = NodeSet::from_tree(tree);
    }

    /// The directories being filtered against.
    #[must_use]
    pub fn watched(&self) -> &NodeSet {
        &self.watched
    }

    /// Take one filesystem event, about `path`, seen at `at`. Answers whether
    /// it was accepted.
    ///
    /// An accepted event either starts a burst or extends the one pending,
    /// unless a reload is in progress, in which case it sets the coalescing
    /// flag instead — see [`COALESCED_RELOADS`]. A rejected event does nothing
    /// at all, which is what makes a `cargo build` free: thousands of paths
    /// under a directory no walk produced, thousands of rejections, no reload
    /// and no hash.
    pub fn saw(&mut self, path: impl AsRef<Path>, at: Instant) -> bool {
        let accepted = self.watched.accepts(path);
        if accepted {
            self.accepted(at);
        }
        accepted
    }

    /// Record an event that something else accepted, seen at `at`.
    ///
    /// The escape hatch for the paths that matter without being under a node of
    /// the tree — the manifest under `.warlock/`, which is hidden and so was
    /// never walked, yet changes what every node's colour should be. Whoever
    /// decides that such a path counts calls this, and the timing rules are then
    /// exactly the same as for anything [`saw`](WatchPolicy::saw) accepted.
    pub fn accepted(&mut self, at: Instant) {
        if self.reloading {
            // One bit, however many events land here.
            self.moved_during_reload = true;
            return;
        }
        self.burst_began.get_or_insert(at);
        self.last_event = Some(at);
    }

    /// Whether the tree is owed a reload as of `now`: the whole question this
    /// module exists to answer.
    ///
    /// True when something has been accepted and either the disk has been quiet
    /// for [`QUIET_PERIOD`] since the last of it, or [`RELOAD_CEILING`] has
    /// passed since the first of it and the events are still coming. False
    /// while a reload is in progress, since that reload will see whatever has
    /// happened.
    #[must_use]
    pub fn due(&self, now: Instant) -> bool {
        if self.reloading {
            return false;
        }
        let (Some(began), Some(last)) = (self.burst_began, self.last_event) else {
            return false;
        };
        now.saturating_duration_since(last) >= QUIET_PERIOD
            || now.saturating_duration_since(began) >= RELOAD_CEILING
    }

    /// Say that the reload the policy asked for has begun.
    ///
    /// The pending burst is discharged here rather than when the reload ends,
    /// because the walk about to happen sees the disk as it is now: an event
    /// that arrived before this call is answered by this reload. Anything
    /// arriving from now until [`reload_finished`](WatchPolicy::reload_finished)
    /// is the coalescing case.
    ///
    /// No instant, because nothing here is measured from the moment a reload
    /// began — the clock that matters starts again when it ends.
    pub fn reload_started(&mut self) {
        self.burst_began = None;
        self.last_event = None;
        self.reloading = true;
        self.moved_during_reload = false;
    }

    /// Say that the reload has finished, at `at`.
    ///
    /// If anything moved while it ran, exactly one further reload is owed —
    /// [`COALESCED_RELOADS`] of them, never a queue — and it is owed on the same
    /// terms as any other burst: a [`QUIET_PERIOD`] from here, so a stream of
    /// events that ran right through the reload settles before the tree is read
    /// again, and the ceiling from here too, so it cannot be put off for ever.
    pub fn reload_finished(&mut self, at: Instant) {
        self.reloading = false;
        if std::mem::take(&mut self.moved_during_reload) {
            self.burst_began = Some(at);
            self.last_event = Some(at);
        }
    }

    /// Whether anything at all is pending: a burst waiting out its quiet
    /// period, or an event seen during the reload now running.
    ///
    /// For a caller that has a reason not to reload just yet — a pact in flight
    /// — and wants to know whether it is sitting on something.
    #[must_use]
    pub fn owes_reload(&self) -> bool {
        self.burst_began.is_some() || self.moved_during_reload
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use warlock_engine::{Node, NodeState, Tree};

    use super::{COALESCED_RELOADS, NodeSet, QUIET_PERIOD, RELOAD_CEILING, WatchPolicy};

    /// How often the driver below asks the policy anything, in milliseconds:
    /// finer than the real event loop's 100ms poll, so a test asserting *when* a
    /// reload happened is not really asserting where the tick boundaries fell.
    const TICK_MS: u64 = 10;

    /// The slack a tick-driven assertion is allowed: a deadline is noticed on
    /// the first tick after it passes, never on the instant itself.
    const SLACK: Duration = Duration::from_millis(TICK_MS * 2);

    /// Long enough after everything a test does that no deadline of any kind is
    /// still ahead: what "and then nothing more is owed" is asserted at.
    const LONG_AFTER: Duration = Duration::from_secs(30);

    /// One simulated filesystem event: how many milliseconds after the start it
    /// arrived, and the path it was about.
    type Event = (u64, &'static str);

    /// A tree shaped like a repository that has been walked once: a root, a
    /// directory of crates and one crate under it. `target/` and `.git/` are
    /// deliberately absent — a walk never produces them, and their absence is
    /// the whole of the filter.
    fn walked() -> Tree {
        Tree::new(
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("repo/crates", None, NodeState::Unpacted).with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/WARLOCK.md",
                    NodeState::PactedFresh,
                )]),
            ]),
        )
    }

    /// The event loop, minus the loop: ticks from a base instant to `until_ms`,
    /// hands the policy every event due by each tick, and reloads whenever the
    /// policy says the tree is owed one. A reload occupies `reload_ms` of
    /// simulated time, during which events keep arriving — which is how the
    /// coalescing rule gets exercised with no thread, no watcher and no
    /// sleeping.
    ///
    /// Returns when it reloaded, in milliseconds from the base instant.
    fn drive(
        policy: &mut WatchPolicy,
        events: &[Event],
        until_ms: u64,
        reload_ms: u64,
    ) -> Vec<u64> {
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

    /// A gap in milliseconds as a duration, so an assertion about timing can be
    /// written against the constants themselves rather than against a number
    /// copied out of them.
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
        /// How far apart the stream's events are.
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
}
