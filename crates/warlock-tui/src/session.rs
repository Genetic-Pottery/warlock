//! Where the tree on screen came from, and how it is kept true to disk.
//!
//! [`Scope`] is the two paths everything else is resolved against — the root
//! the load came back rooted at, and the repository root above it — settled
//! once by [`load_app`] and kept for as long as warlock runs. [`reload_tree`]
//! is the one way the tree is ever read again, on the event loop's thread and
//! no other, and [`Watched`] is what decides when the disk moving under the
//! loop makes that re-read owed. [`note`] is the footer's precedence in one
//! place: housekeeping lines give way to whatever a run had to say.
//!
//! [`sigils_held`] is the last thing settled once and kept: what this machine
//! holds for the repository, read from the config `warlock config` writes and
//! stated on the header beside the scope. It is read here and nowhere else, it
//! cannot fail — a home that will not resolve or a config that will not parse is
//! a state on that line rather than a reason not to draw a tree — and no reload
//! re-reads it, because nothing a running warlock does can change it.
//!
//! [`closed_scope`] is what those sigils are then *for*, and it lives here for
//! that reason: it is the one place the boundary question is asked, by all three
//! keys that can be refused over it, so a pact, a refresh and a scope write are
//! turned down on the same grounds in the same words.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{
    Loaded, Manifest, ManifestError, SigilError, Tree, load_sigils, load_tree, manifest_path,
    repository_root, scope_covering, scope_opens_to,
};
use warlock_tui::{App, Chrome, Sigils, Watch, WatchPolicy, Watching, reseat_on};

use crate::config::home_directory;
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
/// and [`reseat_on`] to carry the reader's viewpoint, footer and panel across to
/// it. The header is not among them and is not re-derived either — it is the
/// [`Chrome`] on the [`Scope`] this function was handed, resolved once at
/// startup from a pair of roots that cannot change while warlock runs, and
/// nothing here touches it. It used to be carried by [`reseat_on`] and then
/// immediately overwritten by the line below, which is two ways of moving a fact
/// that never moves.
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
            *app = reseat_on(app, &tree);
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
    /// What the header line states about the pair above: the tree's identity,
    /// and what this machine holds for the repository it came out of.
    ///
    /// Here rather than on the [`App`] for the reason the two paths are here:
    /// it is resolved once and cannot change while warlock runs, so an app that
    /// is rebuilt on every reload has no business carrying it. The renderer is
    /// handed it directly — see [`warlock_tui::draw`].
    pub(crate) chrome: Chrome,
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

/// Start watching `scope`, and say on the footer if nothing is being watched.
///
/// The two halves of starting a watch, in one call because they are one thing
/// the loop does before it begins: the watcher is asked for over the walk `tree`
/// came from ([`Watched::start`]), and whether the operating system granted one
/// is a line and not an error ([`Watched::off_note`]) — warlock with no live
/// updates is warlock as it was, so a refusal is reported and never returned.
///
/// Said here rather than in the loop, so it is one line said once and not a line
/// re-set ten times a second, and said through [`note`] so it gives way to
/// anything the app already has to say.
pub(crate) fn start_watching(app: &mut App, scope: &Scope, tree: &Tree) -> Watched {
    let watched = Watched::start(scope, tree);
    if let Some(line) = watched.off_note() {
        note(app, line);
    }

    watched
}

/// What the footer says when a key is refused over a boundary this machine does
/// not hold, naming the directory as `label` and the boundary as `scope`.
///
/// The shape every other row-level refusal in warlock uses: the fact about the
/// row, then the thing that would help. What would help here is not a keystroke
/// — every key that could open this boundary is a key this boundary closes — so
/// it names `warlock config`, which is the one place a sigil is recorded and the
/// only road from this line to the work.
///
/// It names the scope out loud, and backticked the way `warlock config` and the
/// header both spell a sigil. A refusal that said only "you may not" would leave
/// the reader with nothing to ask their lead for; the whole social half of this
/// design is somebody being told which sigil to go and get.
///
/// Shared with the headless writes rather than retyped there. `warlock unpact`
/// is refused over the same boundary by the same two engine calls, and prints
/// this sentence through [`Error::ClosedScope`](crate::error::Error) — one rule
/// refused in two registers, in one wording, so the day this sentence changes it
/// changes for the keystroke and the shell prompt together. What differs is only
/// what `label` is: a row's label there, the manifest's spelling of the path
/// here.
pub(crate) fn closed_scope_message(label: &str, scope: &str) -> String {
    format!("{label} is scoped `{scope}` — hold that sigil to work here, with `warlock config`")
}

/// The scope closed to this machine over the row `app` has selected, or `None`
/// when the press may go ahead.
///
/// The one place the boundary question is asked, so `p`, `r` and `s` are refused
/// on the same grounds in the same words. `Some` means refused, and the string
/// is the scope that refused it — already worded onto the app's message line by
/// the time this returns, exactly as [`App::scope_target`] and `App::toggle_pact`
/// word their own row-level refusals before answering `None`.
///
/// # Why the check is here and not on the app
///
/// [`Chrome`] carries the sigils and is deliberately not a field on [`App`] —
/// it is resolved once and cannot change under a running warlock, so an app
/// rebuilt on every reload has no business holding it. That decision is what
/// puts this function outside the app, and it is why the app cannot word this
/// particular refusal itself even though it words every other one. What it lends
/// is [`App::label_for`], so the row is named here the way it is named there.
///
/// # A file row is not this function's business
///
/// Coverage would happily answer for a file — [`scope_covering`] walks up from
/// whatever it is handed — but `p`, `r` and `s` all refuse a file row on better
/// grounds than this, and those refusals name the row for what it is. So a file
/// is passed through as open and the key's own answer stands: the boundary has
/// nothing to say about a press that was never going to happen.
///
/// # A path the manifest cannot spell is open
///
/// It takes a tree rooted outside its own repository to reach, which warlock
/// cannot currently be started in, and the two ways to answer it are to refuse
/// every scoped key on a technicality or to let the key give its own answer. The
/// second is chosen for the reason the permissive defaults are chosen throughout:
/// a boundary nobody could have drawn is not a boundary somebody is crossing, and
/// `s` already has its own wording for exactly this path.
pub(crate) fn closed_scope(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
) -> Option<String> {
    let row = app.selected_row()?;
    if row.is_file() {
        return None;
    }

    let path = row.path.clone();
    // `ok()?` and not a message: a path with no manifest-relative form is the
    // key's own to explain, and explaining it twice would put this line under
    // the one that actually says what went wrong.
    let scope = scope_covering(&path, repo_root, manifest).ok()??.to_owned();
    if scope_opens_to(Some(&scope), sigils.as_slice()) {
        return None;
    }

    let label = app.label_for(&path);
    app.set_message(closed_scope_message(&label, &scope));
    Some(scope)
}

/// What this machine holds for the repository at `repo_root`, for the header to
/// state.
///
/// The one place a running warlock reads the sigil config, and it reads it once
/// — from [`load_app`], before the loop starts. A sigil is written by `warlock
/// config`, on the ordinary screen with warlock not running, so there is nothing
/// for a reload to find that this did not; re-reading it every round would be a
/// file opened ten times a second to answer a question that cannot have changed.
///
/// The home directory is resolved here and handed down as a path — see
/// [`home_directory`], the single point in warlock where the environment becomes
/// one — which is what lets [`sigils_under`] be tested against a temporary
/// directory rather than the developer's own.
///
/// A home that cannot be resolved reads as nothing held rather than as a config
/// that would not read. There is no file in that case and no path to name one
/// by, so [`Sigils::Unknown`] would be claiming that something on disk is broken
/// when nothing on disk was ever looked at.
fn sigils_held(repo_root: &Path) -> Sigils {
    home_directory().map_or(Sigils::Nothing, |home| sigils_under(&home, repo_root))
}

/// What is held for `repo_root` under `home`, as one of the header's three
/// states.
///
/// Never an error, and this is the whole of the reason it is a function of its
/// own: what a machine holds is a line on a header, and warlock is a way of
/// reading a tree. A config that will not parse must not keep the tree off the
/// screen, so it becomes [`Sigils::Unknown`] — said out loud, so that broken is
/// never drawn as absent — and nothing here can return upwards to end the event
/// loop.
///
/// The engine's "not found" is the one error that is not a problem (see
/// [`load_sigils`]) and joins the empty set as [`Sigils::Nothing`]: a machine
/// that has never run `warlock config` and one that cleared its sigils hold the
/// same nothing, and the header says nothing about either.
///
/// Shared with `warlock check`, which asks the same question with nothing on
/// screen: what a machine holds is one resolution of one file, and a second
/// reading of these three cases somewhere else would be a second answer waiting
/// to disagree with the header. It takes `home` rather than looking one up for
/// the reason [`sigils_held`] resolves one and hands it down — see
/// [`home_directory`], the single point where the environment becomes a home
/// path, which is what keeps every test off the developer's own.
pub(crate) fn sigils_under(home: &Path, repo_root: &Path) -> Sigils {
    match load_sigils(home, repo_root) {
        Ok(sigils) => Sigils::held(sigils),
        Err(SigilError::NotFound { .. }) => Sigils::Nothing,
        Err(_) => Sigils::Unknown,
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
/// The sigil config is read here too, once, and for the same reason the two
/// paths are resolved here: it is a fact about the machine and the repository
/// rather than about the frame, and the header states it from the first one on.
/// It is read through [`sigils_held`], which cannot fail — a missing home or an
/// unreadable config is a state on that line, never a way out of a function
/// whose failures end the session before the tree is drawn.
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

    let app = App::from_tree(&tree);
    // Resolved here and nowhere else. Neither half of the header line can
    // change under a running warlock — the roots are fixed for the session and a
    // sigil is written with warlock not running — so it is built once, kept
    // beside the roots it was built from, and handed to the renderer every
    // frame. A reload does not touch it, which is why `reseat_on` no longer
    // carries it and `reload_tree` no longer puts it back afterwards.
    let chrome = Chrome::of(&repo_root, tree.root_path()).with_sigils(sigils_held(&repo_root));
    let scope = Scope {
        root: tree.root_path().to_path_buf(),
        repo_root,
        chrome,
    };
    Ok((app, scope, tree))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{env, fs, process};

    use warlock_engine::{Manifest, PactEntry, manifest_path, save_sigils, sigils_path};
    use warlock_tui::{Chrome, Sigils};

    use super::{Scope, load_manifest, sigils_under};
    use crate::error::Error;

    #[test]
    fn what_is_held_survives_a_reload_because_a_reload_never_touches_it() {
        // The claim this used to make about `App`, made where it now lives. The
        // config is read once, before the loop starts, and a reload is a tree
        // being read again rather than a machine changing what it holds — so a
        // header that stopped stating a holding after a run would read as one
        // dropped.
        //
        // What changed is how it is kept true. It used to be `reseat_on`
        // remembering to carry two more fields; it is now that the fact is not
        // on the app at all, so there is nothing for a reload to carry or drop.
        // `reload_tree` reads `scope.chrome` and writes it nowhere.
        let scope = Scope {
            root: PathBuf::from("/repo/crates"),
            repo_root: PathBuf::from("/repo"),
            chrome: Chrome::of("/repo", "/repo/crates")
                .with_sigils(Sigils::held(["billing", "web"])),
        };

        // A load from a path that is not there fails, which is the arm that
        // keeps the tree already drawn — and the arm that would be the last
        // chance to lose a header if one could still be lost here.
        let mut app = warlock_tui::App::default();
        assert_eq!(super::reload_tree(&mut app, &scope), None);

        assert_eq!(scope.chrome.header(), "crates");
        assert_eq!(
            scope.chrome.sigils(),
            &Sigils::Held(vec!["billing".to_owned(), "web".to_owned()])
        );
    }

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

    #[test]
    fn a_machine_that_never_ran_warlock_config_holds_nothing() {
        // The ordinary state of a machine, and the one the header says nothing
        // at all about: no file, no directory, nothing to say.
        let home = a_dir();
        let repo = a_dir();

        assert_eq!(sigils_under(home.path(), repo.path()), Sigils::Nothing);
    }

    #[test]
    fn the_sigils_on_disk_are_the_sigils_the_header_states() {
        let home = a_dir();
        let repo = a_dir();
        let held = ["billing".to_owned(), "web".to_owned()];
        save_sigils(home.path(), repo.path(), &held).expect("a config that writes");

        assert_eq!(
            sigils_under(home.path(), repo.path()),
            Sigils::Held(held.to_vec())
        );
    }

    #[test]
    fn a_config_holding_the_empty_set_is_the_same_nothing_as_no_config_at_all() {
        // Clearing a holding puts the header back where it was, rather than
        // leaving an empty list drawn on it.
        let home = a_dir();
        let repo = a_dir();
        save_sigils(home.path(), repo.path(), &[]).expect("a config that writes");

        assert_eq!(sigils_under(home.path(), repo.path()), Sigils::Nothing);
    }

    #[test]
    fn a_config_that_will_not_parse_is_said_rather_than_ending_the_session() {
        // The whole point of this returning a state instead of a `Result`: a
        // broken config is a word on a header, never a reason to keep the tree
        // off the screen — and never drawn as absent, since the two mean
        // opposite things about what is on disk.
        let home = a_dir();
        let repo = a_dir();
        let path = sigils_path(home.path(), repo.path());
        fs::create_dir_all(
            path.parent()
                .expect("the config lives in a project directory"),
        )
        .expect("the project directory");
        fs::write(&path, "not a config\n").expect("a file that is not TOML");

        assert_eq!(sigils_under(home.path(), repo.path()), Sigils::Unknown);
    }

    /// A throwaway directory: every test here builds both its home *and* its
    /// repository root out of these, so nothing in this module reads or writes
    /// the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }
}
