//! Where the tree on screen came from, and how it is kept true to disk.
//!
//! [`Scope`] is the two paths everything else is resolved against — the root
//! the load came back rooted at, and the repository root above it — settled
//! once by [`load_app`] and kept for as long as warlock runs. [`reload_tree`]
//! is the one way the tree is ever read again, on the event loop's thread and
//! no other, and [`Watched`] is what decides when the disk moving under the
//! loop makes that re-read owed. [`note`] is the footer's precedence in one
//! place: housekeeping lines give way to whatever a run had to say.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{
    Loaded, Manifest, ManifestError, Tree, load_tree, manifest_path, repository_root,
};
use warlock_tui::{App, Watch, WatchPolicy, Watching, reseat_on};

use crate::error::{Error, one_line};

/// What the footer says when the reload after a run could not read the tree,
/// ahead of the load's own reason for it.
///
/// It says what the reader lost, which is the refresh and nothing else: the run
/// is over, its documents are on disk and its manifest is saved, and the rows
/// under this line are the ones that were there before — true, only older than
/// disk. Worded as a fact about the view rather than as a failure of the run,
/// because the run did not fail.
pub(crate) const NOT_REFRESHED: &str = "the view could not be refreshed and is the tree as it was";

/// What the footer says when no watcher could be started, ahead of the
/// operating system's own reason for it.
///
/// It says what the reader lost, which is the noticing and nothing else: every
/// key still works, a pact still re-reads the tree when its run ends, and the
/// rows under this line are the ones the load produced — true, and true for as
/// long as nothing else writes to the repository. What will not happen is a row
/// appearing because somebody saved a file in another window.
///
/// Worded as a fact about this session rather than as a failure, because
/// nothing the reader asked for failed, and said once when the watcher is asked
/// for rather than on every frame: a line that is re-set ten times a second is a
/// line that talks over everything else the footer has to say.
pub(crate) const NOT_WATCHING: &str = "live updates are off; the tree is the one loaded at startup";

/// Re-read the tree at `scope` from disk and put the view back on top of it.
///
/// Two calls and no judgement of its own: [`load_tree`] for what is on disk now,
/// and [`reseat_on`] to carry the reader's selection, collapsed set, filters,
/// window and footer across to it. The header is re-derived the way
/// [`load_app`] derives it, from the repository root and the root the new tree
/// came back rooted at, so the line at the top names the tree the engine just
/// walked rather than the one it walked at startup.
///
/// Called on the event loop's thread and on no other. A worker thread must never
/// reach in here: it would be reading a tree while the thread that draws it is
/// drawing one, for a result only the drawing thread can use.
///
/// A load that fails is not an error out of the event loop, and this is the
/// deliberate difference from [`load_app`], where the same failure is fatal.
/// Warlock is up, the documents the run wrote are whole on disk and the manifest
/// that records them is saved; quitting here would throw away a run that cost
/// minutes and money, over nothing worse than a stale screen. So the tree
/// already drawn is kept and the reader carries on with it — the one thing they
/// lose is the refresh. Problems that did not stop the load are a different
/// matter: the engine has already coloured each affected node conservatively, so
/// a tree that has the new documents in it beats the stale one it would replace,
/// and it is taken.
///
/// Either way there is a line to write — [`NOT_REFRESHED`] and the load's reason
/// for one that failed, the problems' own wording and their count for one that
/// did not — and it goes on the footer only when the run left the footer empty.
/// The pact's message wins because it is the news: what a run made of the
/// subtree the reader asked for is worth more than how the redraw after it went,
/// and the footer is one line. Precedence, not merging: two sentences joined by
/// a semicolon would be a line nobody reads to the end of.
///
/// The tree that was read comes back with it, and `None` when none was. It is
/// what the watcher's filter is rebuilt from — the directories of the walk that
/// produced what is now on screen — and it is handed back rather than looked up
/// again because this function is the only thing that has it. Nothing else about
/// the view is in it: the re-seating above is the whole of the restoration, here
/// as it is after a pact.
pub(crate) fn reload_tree(app: &mut App, scope: &Scope) -> Option<Tree> {
    match load_tree(&scope.root) {
        Ok(Loaded { tree, problems }) => {
            *app = reseat_on(app, &tree).with_scope(&scope.repo_root, tree.root_path());
            // The same count, in the same words, as the startup load that
            // refuses to draw a tree with problems in it: one problem quoted
            // and the rest counted. A node the engine could not hash is
            // already stale on screen, so this line is the only place the
            // number of them appears.
            if let Some(counted) = Error::from_problems(&problems) {
                note(app, counted.to_string());
            }
            Some(tree)
        }
        // Flattened for the same reason `Error` flattens it: a manifest that
        // will not parse arrives as the TOML parser's several lines, and the
        // footer is one.
        Err(source) => {
            note(
                app,
                format!("{NOT_REFRESHED}: {}", one_line(&source.to_string())),
            );
            None
        }
    }
}

/// Say `line` on the footer, unless something is already saying something
/// there.
///
/// The footer's precedence in one place, because there are now two lines that
/// have it: how the redraw went ([`reload_tree`]) and whether the disk is being
/// watched at all ([`NOT_WATCHING`]). Both give way to whatever else is on the
/// line, which in practice means a pact's own message: what a run made of the
/// subtree the reader asked for is the news, and how warlock is keeping itself
/// up to date is housekeeping. Precedence, not merging — two sentences joined by
/// a semicolon would be a line nobody reads to the end of.
pub(crate) fn note(app: &mut App, line: impl Into<String>) {
    if app.message().is_none() {
        app.set_message(line);
    }
}

/// Where the tree on screen came from: the directory it is rooted at, and the
/// repository root above that directory.
///
/// Both are resolved once, by [`load_app`], and kept for as long as warlock
/// runs. The root is where a re-read starts, and the repository root is what the
/// manifest is written under and what the header spells the root relative to —
/// which is why they travel together rather than being guessed at again from a
/// working directory that has since had a pact run over it.
pub(crate) struct Scope {
    /// The directory the tree is rooted at, as the load that built it came back
    /// rooted — not the working directory as typed.
    pub(crate) root: PathBuf,
    /// The repository root above `root`: the nearest ancestor with a `.git/`.
    pub(crate) repo_root: PathBuf,
}

/// Everything the loop keeps about the disk moving under it.
///
/// Three things, and no thread of its own: a [`Watching`] — which is a watcher
/// or the reason there is none — the [`WatchPolicy`] that decides what the paths
/// it reports are worth, and the manifest's path, which is the one path that
/// counts without any walk ever having produced it.
///
/// Nothing here decides *which* paths matter or *when* to act on them; both of
/// those are the library's questions — [`WatchPolicy`] answers them as values,
/// with no clock and no disk. What this type adds is the one thing that has to
/// happen on
/// this thread: reading the tree again, through the same [`reload_tree`] a pact
/// ends with, and handing the tree it read back to the policy so the next
/// round's filter is the walk that produced what is on screen.
pub(crate) struct Watched {
    /// The watcher, or the reason the operating system would not start one.
    pub(crate) watching: Watching,
    /// The filter and the timing rules, over the last successful load's walk.
    pub(crate) policy: WatchPolicy,
    /// `.warlock/pacts.toml`, resolved once. It is hidden, so no walk produces
    /// it and [`NodeSet`](warlock_tui::NodeSet) rejects it — and yet a pact
    /// granted or dropped in another window changes the colour of every row on
    /// screen while nothing inside the tree has moved. So it is compared for by
    /// name here, which is the only rule of this file's own about a path, and it
    /// is a path rather than a pattern.
    pub(crate) manifest: PathBuf,
}

impl Watched {
    /// Start watching `scope`, filtering against the walk `tree` came from.
    ///
    /// The watcher is over [`Scope::root`] — the path the load came back rooted
    /// at, so warlock run in a subdirectory hears about that subdirectory and
    /// not about a build in a sibling crate — and over the manifest under
    /// [`Scope::repo_root`]. Whether it started is not asked here: the answer is
    /// kept as a value and said once, on the footer, by whoever is drawing.
    pub(crate) fn start(scope: &Scope, tree: &Tree) -> Self {
        Self {
            watching: Watch::start(&scope.root, &scope.repo_root),
            policy: WatchPolicy::new(tree),
            manifest: manifest_path(&scope.repo_root),
        }
    }

    /// The one line the footer owes when nothing is being watched, or `None`
    /// when something is.
    ///
    /// Asked once, before the loop starts. It is a question about how warlock
    /// was started rather than about anything happening now, so asking it every
    /// frame would be re-answering a fact that cannot change and re-writing a
    /// line the reader has already read past.
    pub(crate) fn off_note(&self) -> Option<String> {
        match &self.watching {
            Watching::Live(_) => None,
            // Flattened for the same reason the reload's line is: what the
            // operating system says can run to several lines, and the footer is
            // one.
            Watching::Off(reason) => Some(format!("{NOT_WATCHING}: {}", one_line(reason))),
        }
    }

    /// One round of the event loop's watching: hear what the disk did, and read
    /// the tree again if that is now owed. Answers whether it read it.
    ///
    /// Every path drained is offered to the policy, which rejects the ones no
    /// walk produced — the whole of the cost of a `cargo build` under a watched
    /// root, thousands of paths compared against a set and dropped. The manifest
    /// is the exception it cannot see for itself, and it is the only one.
    ///
    /// `in_flight` is a pact running somewhere else, and it does not stop the
    /// draining, only the reloading: the events a run's own documents set off
    /// are remembered by the policy and answered by the reload at the end of the
    /// run ([`caught_up`](Watched::caught_up)), so a run whose documents moved
    /// the disk reads the tree once, at the end, rather than twice. Reloading
    /// under a run would also be reloading a tree the run is still writing to.
    pub(crate) fn round(
        &mut self,
        app: &mut App,
        scope: &Scope,
        in_flight: bool,
        now: Instant,
    ) -> bool {
        if let Watching::Live(watch) = &mut self.watching {
            for path in watch.drain() {
                if !self.policy.saw(&path, now) && path == self.manifest {
                    self.policy.accepted(now);
                }
            }
        }

        if in_flight || !self.policy.due(now) {
            return false;
        }
        let tree = reload_tree(app, scope);
        // The clock is read again, and this is the one place in the loop that
        // does: everything else in a round is over in microseconds, while the
        // load between these two lines is a walk and a hash per pacted subtree.
        // What the policy measures from here is the quiet period owed to
        // anything that moved *during* that load, and measuring it from before
        // the load would be starting the wait before the events it is waiting
        // for could arrive.
        self.caught_up(tree.as_ref(), Instant::now());
        true
    }

    /// Somebody else read the tree, at `at`: the reload at the end of a run.
    ///
    /// Two things, and the second is why this exists at all. The policy is told
    /// that a reload happened, which discharges whatever it was owed — an event
    /// from a pact's own documents is answered by the reload that came after it
    /// — and it is re-seated on the tree that reload produced, so the next
    /// round's filter is the walk behind what is on screen rather than the walk
    /// behind what used to be. A load that failed hands over `None`: the burst
    /// is still discharged, because the tree was read and this is as fresh as
    /// the view is going to get, and the filter stays on the last walk that
    /// worked, which is the one the rows still come from.
    pub(crate) fn caught_up(&mut self, tree: Option<&Tree>, at: Instant) {
        self.policy.reload_started();
        if let Some(tree) = tree {
            self.policy.follow(tree);
        }
        self.policy.reload_finished(at);
    }
}

/// The repository's manifest, or an empty one if it has never pacted anything.
///
/// The same reading of a missing file the loader takes: nothing on disk and
/// nothing pacted are the same thing to draw, and the difference only matters
/// to code that would refuse to create the file, which this is not — pressing
/// `p` in a repository with no `.warlock/` is how the first manifest gets
/// written.
pub(crate) fn load_manifest(repo_root: &Path) -> Result<Manifest, Error> {
    match Manifest::load(repo_root) {
        Err(ManifestError::NotFound { .. }) => Ok(Manifest::new()),
        other => other.map_err(|source| Error::Manifest { source }),
    }
}

/// The app state for the directory warlock was invoked from, the [`Scope`] it
/// was loaded at, and the tree it was built from.
///
/// The tree comes back as well as the app because the app is not a tree: its
/// rows are filtered by what the reader has toggled, and the directories one
/// walk produced are what the watcher's filter is ([`Watched`]). Handing it over
/// here is what keeps that filter and the rows on screen born of the same walk.
///
/// The two paths are handed back rather than dropped once the header is built.
/// The repository root is where the manifest lives: every pact written during
/// the run is written relative to it, and finding it is a walk up the
/// filesystem that should happen once. The tree's own root is where a re-read
/// starts, and it is kept for the same reason — it is the path the engine came
/// back rooted at, which is the one thing a later load must be given rather than
/// guess.
///
/// This is the whole of the front end's knowledge about scope, and it is
/// section 12's modular invocation rule spelled out: the tree is rooted at the
/// working directory, and the manifest that colours it comes from the single
/// repository root above that directory — which is why the header needs both.
/// The root is taken from the loaded tree rather than from the working
/// directory as typed, so the header names the same path the engine walked.
///
/// A load that reported problems is refused rather than drawn. The problems
/// are files Warlock could not read, so the nodes above them are coloured
/// stale on no evidence; showing that as an ordinary tree would put a colour
/// on screen that nothing on disk backs up. That is a startup rule and stays
/// one: mid-session, with a tree already on screen and a run's documents
/// already on disk, the same problems are taken rather than fatal — see
/// [`reload_tree`].
pub(crate) fn load_app() -> Result<(App, Scope, Tree), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    let Loaded { tree, problems } =
        load_tree(&working_dir).map_err(|source| Error::Load { source })?;
    if let Some(error) = Error::from_problems(&problems) {
        return Err(error);
    }

    // The load succeeded, so a repository root was found; asking again is a
    // walk up a path, not a second load. The fallback is unreachable, and
    // labelling the root as itself is the closest thing to true if it ever is
    // reached.
    let repo_root = repository_root(tree.root_path()).unwrap_or(working_dir);

    let app = App::from_tree(&tree).with_scope(&repo_root, tree.root_path());
    let scope = Scope {
        root: tree.root_path().to_path_buf(),
        repo_root,
    };
    Ok((app, scope, tree))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{env, fs, process};

    use warlock_engine::{Manifest, PactEntry, manifest_path};

    use super::load_manifest;
    use crate::error::Error;

    /// A scratch repository root of this module's own, deleted when the test
    /// drops it. `load_manifest` is entirely about what is and is not on
    /// disk, so each test gets a directory nobody else writes to.
    struct Scratch {
        /// The root the manifest is looked for under.
        root: PathBuf,
    }

    impl Scratch {
        /// An empty directory, named after the test using it so a leftover
        /// says where it came from.
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let root =
                env::temp_dir().join(format!("warlock-session-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&root).expect("a scratch directory under the temp directory");
            Self { root }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_repository_that_never_pacted_anything_starts_with_an_empty_manifest() {
        // The missing file is the ordinary state of a repository before its
        // first pact, so it is the empty manifest rather than an error —
        // pressing `p` is how the first `.warlock/pacts.toml` gets written.
        let scratch = Scratch::new("never-pacted");

        let manifest =
            load_manifest(&scratch.root).expect("a missing manifest reads as an empty one");

        assert_eq!(manifest, Manifest::new());
    }

    #[test]
    fn the_manifest_on_disk_is_the_one_the_session_starts_from() {
        let scratch = Scratch::new("round-trip");
        let mut saved = Manifest::new();
        saved.push(
            PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")
                .expect("a module spelled relative to the root"),
        );
        saved.save(&scratch.root).expect("a manifest that writes");

        let loaded = load_manifest(&scratch.root).expect("the manifest just saved loads");

        assert_eq!(loaded, saved);
    }

    #[test]
    fn a_manifest_that_will_not_parse_is_an_error_rather_than_an_empty_start() {
        // Only the *missing* file maps to the empty manifest. A file that is
        // there and will not parse is somebody's record of what is pacted,
        // and quietly starting from nothing would offer the next keystroke a
        // save that erases it.
        let scratch = Scratch::new("broken");
        let path = manifest_path(&scratch.root);
        fs::create_dir_all(path.parent().expect("the manifest lives under `.warlock/`"))
            .expect("the manifest directory");
        fs::write(&path, "not a manifest\n").expect("a file that is not TOML");

        let error = load_manifest(&scratch.root).expect_err("a broken manifest must not load");

        assert!(
            matches!(error, Error::Manifest { .. }),
            "the engine's own case was rewrapped: {error:?}"
        );
        let message = error.to_string();
        assert!(
            !message.contains('\n'),
            "`main` prints one line, and this wraps: {message}"
        );
    }
}
