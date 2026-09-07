//! Two questions about a moving disk, answered as values — which movements are
//! Warlock's business ([`NodeSet`]) and when to act on them ([`WatchPolicy`]) —
//! plus the impure third that hears about them ([`Watch`]).
//!
//! The filter is the last walk itself. The loader already applied gitignore,
//! skip lists and hidden-file rules on the way to the tree it returned, so
//! re-deciding any of that here would be a second implementation free to
//! disagree with the first; `target/` and `.git/` are rejected for one reason
//! only, that no walk ever produced them.
//!
//! [`Instant::now`] is never called in this file. Every instant the policy
//! compares against is handed in by the event loop, which already reads the
//! clock once a frame, so a ten-second burst is driven through it in
//! microseconds with no sleeping and no real disk.
//!
//! Nothing here reloads. The policy says a reload is *owed*; reading the tree
//! again is the binary's business, on the thread that draws. No `notify` type
//! appears in a signature — paths leave as [`PathBuf`], a failed start as a
//! [`String`].

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use warlock_engine::{Tree, manifest_path};

/// The debounce, measured from the *last* accepted event. One editor save
/// arrives as several events — write, rename over the original, chmod — and a
/// reload apiece would re-walk and re-hash the tree for one keystroke in
/// another window. A quarter second sits above the millisecond gaps within one
/// save and below being read as not having noticed, and is longer than the
/// event loop's poll interval, so the deadline is checked before it passes.
pub const QUIET_PERIOD: Duration = Duration::from_millis(250);

/// The starvation guard, measured from the *first* event of the burst. A `git
/// checkout` or a formatter run over the repository emits faster than
/// [`QUIET_PERIOD`], so the quiet a pure debounce waits for never comes and the
/// tree would sit stale for as long as the burst lasted.
pub const RELOAD_CEILING: Duration = Duration::from_secs(2);

/// How many further reloads the events arriving *during* a reload are worth
/// between them, however many of them there were. Held as one bit rather than a
/// count or a list, because a pact writes documents while the reload that will
/// already see them runs: remembering each would leave the loop owing a reload
/// per event and re-walking until the backlog drained. Named rather than left
/// implicit in the code because the number is the rule, and it cannot grow.
pub const COALESCED_RELOADS: usize = 1;

/// One path per node of the [`Tree`] a load returned: the filter every
/// filesystem event is held against.
///
/// Built from the tree and never from what is on screen — rows are filtered by
/// the pacted-only and file toggles, so a filter built from them would stop
/// noticing directories the moment somebody pressed a key.
///
/// Paths are compared as stored, with no normalisation and no filesystem
/// access. Tree paths and watcher events are both rooted where the load was
/// rooted, so the two agree without anybody canonicalising.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeSet {
    directories: HashSet<PathBuf>,
}

impl NodeSet {
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

    /// The path's *immediate* parent, not any ancestor, and that is the whole
    /// of the rule. Matching any ancestor would accept everything under
    /// `target/` and `.git/` the moment the root was walked; matching the
    /// immediate parent rejects them however deep they sit, with no ignore
    /// rules read here, while still accepting a brand-new directory — whose
    /// parent *was* walked — so it becomes a node of its own on the next load.
    ///
    /// The filesystem root has no parent, and the tree's own root has one above
    /// everything the load produced; both are rejected.
    #[must_use]
    pub fn accepts(&self, path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .parent()
            .is_some_and(|parent| self.directories.contains(parent))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.directories.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.directories.is_empty()
    }
}

/// The timing half: told what happened and when, it answers whether the tree is
/// owed a reload. It performs nothing and reads no clock — every instant it
/// compares against is one the caller handed it.
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
    watched: NodeSet,
    /// [`RELOAD_CEILING`] is measured from this, [`QUIET_PERIOD`] from
    /// `last_event`; the two are `Some` together or not at all, and `None`
    /// means nothing is pending.
    burst_began: Option<Instant>,
    last_event: Option<Instant>,
    reloading: bool,
    /// The [`COALESCED_RELOADS`] bit, not a count.
    moved_during_reload: bool,
}

impl WatchPolicy {
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

    /// Only the filter is replaced. What the policy owes — a burst waiting out
    /// its quiet period, an event seen during a reload — survives, because a
    /// new tree says nothing about events already accepted.
    pub fn follow(&mut self, tree: &Tree) {
        self.watched = NodeSet::from_tree(tree);
    }

    #[must_use]
    pub fn watched(&self) -> &NodeSet {
        &self.watched
    }

    /// Returns whether the event was accepted. A rejected event does nothing at
    /// all, which is what makes a `cargo build` free: thousands of paths under
    /// a directory no walk produced, no reload and no hash.
    pub fn saw(&mut self, path: impl AsRef<Path>, at: Instant) -> bool {
        let accepted = self.watched.accepts(path);
        if accepted {
            self.accepted(at);
        }
        accepted
    }

    /// The way in for a path that matters without being under a node of the
    /// tree: the manifest is hidden, so no walk produced it and
    /// [`saw`](WatchPolicy::saw) would reject it, yet it changes what every
    /// node's colour should be. Timing rules are the same from here on.
    pub fn accepted(&mut self, at: Instant) {
        if self.reloading {
            // One bit, however many events land here.
            self.moved_during_reload = true;
            return;
        }
        self.burst_began.get_or_insert(at);
        self.last_event = Some(at);
    }

    /// False while a reload is in progress: that reload will see whatever has
    /// happened, so owing another for it would be owing it twice.
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

    /// The pending burst is discharged here, at the start, not when the reload
    /// ends: the walk about to happen sees the disk as it is now, so every
    /// event that arrived before this call is answered by this reload and
    /// everything after it is the coalescing case.
    ///
    /// No instant, because nothing is measured from the moment a reload began —
    /// the clock that matters starts again when it ends.
    pub fn reload_started(&mut self) {
        self.burst_began = None;
        self.last_event = None;
        self.reloading = true;
        self.moved_during_reload = false;
    }

    /// Anything that moved while the reload ran is owed one further reload, on
    /// the same terms as any other burst and dated from `at` rather than from
    /// when the events arrived — so a stream running right through a reload
    /// still has to settle, and still cannot be put off past the ceiling.
    pub fn reload_finished(&mut self, at: Instant) {
        self.reloading = false;
        if std::mem::take(&mut self.moved_during_reload) {
            self.burst_began = Some(at);
            self.last_event = Some(at);
        }
    }

    /// Unlike [`due`](WatchPolicy::due) this ignores the clock and the
    /// in-progress flag: it is for a caller with its own reason to hold a
    /// reload back — a pact in flight — asking whether it is sitting on
    /// anything.
    #[must_use]
    pub fn owes_reload(&self) -> bool {
        self.burst_began.is_some() || self.moved_during_reload
    }
}

/// A value rather than a `Result`, because there is nothing here for a caller
/// to fail over: warlock without a watcher still draws, pacts, navigates and
/// quits, and merely reloads only when asked. The failure carries the operating
/// system's own words as a plain [`String`], so no `notify` type leaves this
/// module and the caller is left free to frame it for the footer.
#[derive(Debug)]
pub enum Watching {
    Live(Watch),
    Off(String),
}

/// The only part of this module that talks to the operating system, and it
/// decides nothing: every path is passed on as it arrived, because a filter
/// here would be a second one living in the half that cannot be tested without
/// a real disk.
pub struct Watch {
    /// Held only to keep the watch alive: `notify` stops watching when the
    /// handle is dropped. Nothing is read through this field — what the watcher
    /// has to say arrives on `events`.
    _handle: RecommendedWatcher,
    events: Receiver<PathBuf>,
    live: bool,
}

impl Watch {
    /// `root` is the tree's own root, the path the load came back rooted at, and
    /// never `repo_root` standing in for it: warlock run from a subdirectory
    /// shows that subdirectory, and watching the repository above would hear
    /// about a build in a sibling crate that is not on screen.
    ///
    /// The manifest is watched separately because a pact granted in another
    /// window changes what every node's colour should be while nothing inside
    /// the tree has moved.
    #[must_use]
    pub fn start(root: impl AsRef<Path>, repo_root: impl AsRef<Path>) -> Watching {
        let (sender, events) = mpsc::channel();
        // The closure runs on `notify`'s own thread. It unwraps each event into
        // the paths it was about and sends them on; an error from the watcher
        // and a send to a receiver nobody holds any more are both dropped,
        // since there is no one on that thread to tell and nothing it could do.
        let handler = move |event: notify::Result<Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    let _ = sender.send(path);
                }
            }
        };

        let mut handle = match notify::recommended_watcher(handler) {
            Ok(handle) => handle,
            Err(error) => return Watching::Off(error.to_string()),
        };
        if let Err(error) = handle.watch(root.as_ref(), RecursiveMode::Recursive) {
            return Watching::Off(error.to_string());
        }
        watch_manifest(&mut handle, repo_root.as_ref());

        Watching::Live(Self {
            _handle: handle,
            events,
            live: true,
        })
    }

    /// [`try_recv`](Receiver::try_recv) and never a blocking receive: the thread
    /// that drains is the thread that draws, and a frame is not worth waiting
    /// on a quiet disk for.
    ///
    /// A watcher that died takes the channel with it, which surfaces here as an
    /// empty drain and [`live`](Watch::live) going false — not an error, and
    /// never a reason to stop.
    pub fn drain(&mut self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(path) => paths.push(path),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.live = false;
                    break;
                }
            }
        }
        paths
    }

    /// Only a drain can notice the watcher going, since nothing else here
    /// touches the channel.
    #[must_use]
    pub fn live(&self) -> bool {
        self.live
    }
}

impl fmt::Debug for Watch {
    // Hand-written because the handle is a platform type with nothing readable
    // in it and the channel cannot be inspected without draining it. Liveness
    // is all that can be printed honestly.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Watch")
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

/// Falls back from the file to `.warlock/` because the manifest is often absent
/// — no pact made yet — and an absent path cannot be watched; the directory
/// watch also catches the create, and the rename a write lands as.
///
/// Failure is not reported because there is nobody to report it to: the cost is
/// one reload that happens at the end of a pact instead of during it.
fn watch_manifest(handle: &mut RecommendedWatcher, repo_root: &Path) {
    let manifest = manifest_path(repo_root);
    if handle.watch(&manifest, RecursiveMode::NonRecursive).is_ok() {
        return;
    }
    if let Some(directory) = manifest.parent() {
        let _ = handle.watch(directory, RecursiveMode::NonRecursive);
    }
}

#[cfg(test)]
mod tests {
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
            Node::new("repo", "repo/WARLOCK.md", NodeState::PactedStale).with_children([
                Node::new("repo/crates", None, NodeState::Unpacted).with_children([Node::new(
                    "repo/crates/engine",
                    "repo/crates/engine/WARLOCK.md",
                    NodeState::PactedFresh,
                )]),
            ]),
        )
    }

    // The event loop minus the loop, returning the millisecond offsets it
    // reloaded at. A reload occupies `reload_ms` of simulated time and events
    // keep arriving through it, which is how the coalescing rule is exercised
    // with no thread, no watcher and no sleeping.
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
}
