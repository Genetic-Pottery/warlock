//! Where the tree on screen came from, and how it is kept true to disk.
//!
//! [`Scope`] is the two paths everything else is resolved against, settled once
//! by [`load_app`] and kept for as long as warlock runs, and it carries the
//! [`Chrome`] for the same reason: neither the roots nor what this machine holds
//! can change under a running warlock, so an app rebuilt on every reload has no
//! business carrying them. That is also why the sigil config is read once, on
//! the way in, and why that read cannot fail — a home that will not resolve or a
//! config that will not parse is a state on the header rather than a reason not
//! to draw a tree. The three cases it can come back as are turned into a
//! [`Sigils`] by [`sigils_under`], which `warlock check` and the headless writes
//! borrow rather than reading the file a second way.
//!
//! [`closed_scope`] is the one place the boundary question is asked, by all
//! three keys that can be refused over it, so a pact, a refresh and a scope
//! write are turned down on the same grounds in the same words.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{
    Loaded, Manifest, Tree, load_sigils, load_tree, manifest_path, repository_root, sigils,
};
use warlock_tui::{App, Chrome, Sigils, Watch, WatchPolicy, Watching, reseat_on};

use crate::boundary::{Reach, Verdict, closed_scope_message, verdict};
use crate::error::{Error, one_line};
use crate::standing::Standing;

// Says what the reader lost, which is the refresh and nothing else: the run is
// over, its documents are on disk, its manifest is saved, and the rows under
// this line are the ones that were there before — true, only older than disk. A
// fact about the view rather than a failure of the run, because the run did not
// fail.
pub(crate) const NOT_REFRESHED: &str = "the view could not be refreshed and is the tree as it was";

// Says what the reader lost, which is the noticing: every key still works and a
// pact still re-reads the tree when its run ends. What will not happen is a row
// appearing because somebody saved a file in another window. Said once, when the
// watcher is asked for, rather than on every frame — a line re-set ten times a
// second talks over everything else the footer has to say.
pub(crate) const NOT_WATCHING: &str = "live updates are off; the tree is the one loaded at startup";

// Called on the event loop's thread and on no other. A worker thread must never
// reach in here: it would be reading a tree while the thread that draws it is
// drawing one, for a result only the drawing thread can use.
//
// A load that fails is not an error out of the loop, which is the deliberate
// difference from `load_app`, where the same failure is fatal. Warlock is up,
// the documents a run wrote are whole on disk and the manifest recording them is
// saved; quitting here would throw away a run that cost minutes and money over
// nothing worse than a stale screen. Problems that did not stop the load are a
// different matter and the tree is taken: the engine has already coloured each
// affected node conservatively, and a tree with the new documents in it beats
// the stale one it would replace.
//
// The header is neither carried nor re-derived — it is the `Chrome` on the
// `Scope` handed in. It used to be carried by `reseat_on` and then immediately
// overwritten here, which is two ways of moving a fact that never moves.
//
// The tree that was read comes back because this function is the only thing that
// has it, and the watcher's filter has to be rebuilt from the walk that produced
// what is now on screen.
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

// The footer's precedence in one place, because two lines have it: how the
// redraw went and whether the disk is being watched at all. Both give way to
// whatever else is on the line, which in practice is a pact's own message —
// what a run made of the subtree the reader asked for is the news, and how
// warlock keeps itself up to date is housekeeping. Precedence, not merging: two
// sentences joined by a semicolon would be a line nobody reads to the end of.
pub(crate) fn note(app: &mut App, line: impl Into<String>) {
    if app.message().is_none() {
        app.set_message(line);
    }
}

// The root is where a re-read starts and the repository root is what the
// manifest is written under, which is why they travel together rather than being
// guessed at again from a working directory that has since had a pact run over
// it.
pub(crate) struct Scope {
    // As the load came back rooted, not the working directory as typed.
    pub(crate) root: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) chrome: Chrome,
}

// Nothing here decides *which* paths matter or *when* to act on them; both are
// `WatchPolicy`'s, answered as values with no clock and no disk. What this type
// adds is the one thing that has to happen on the loop's thread: reading the tree
// again, and handing the tree it read back to the policy so the next round's
// filter is the walk that produced what is on screen.
pub(crate) struct Watched {
    pub(crate) watching: Watching,
    pub(crate) policy: WatchPolicy,
    // `.warlock/pacts.toml` is hidden, so no walk produces it and the policy's
    // filter rejects it — and yet a pact granted or dropped in another window
    // changes the colour of every row on screen while nothing inside the tree has
    // moved. So it is compared for by name, which is the one rule of this file's
    // own about a path.
    pub(crate) manifest: PathBuf,
}

impl Watched {
    // The watcher is over `Scope::root` — the path the load came back rooted at,
    // so warlock run in a subdirectory hears about that subdirectory and not
    // about a build in a sibling crate. Whether it started is not asked here: the
    // answer is kept as a value and said once, by whoever is drawing.
    pub(crate) fn start(scope: &Scope, tree: &Tree) -> Self {
        Self {
            watching: Watch::start(&scope.root, &scope.repo_root),
            policy: WatchPolicy::new(tree),
            manifest: manifest_path(&scope.repo_root),
        }
    }

    // Asked once, before the loop starts: it is a question about how warlock was
    // started rather than about anything happening now, so asking every frame
    // would re-answer a fact that cannot change.
    pub(crate) fn off_note(&self) -> Option<String> {
        match &self.watching {
            Watching::Live(_) => None,
            // Flattened for the same reason the reload's line is: what the
            // operating system says can run to several lines, and the footer is
            // one.
            Watching::Off(reason) => Some(format!("{NOT_WATCHING}: {}", one_line(reason))),
        }
    }

    // `in_flight` is a pact running somewhere else, and it stops the reloading
    // and not the draining: the events a run's own documents set off are
    // remembered by the policy and answered by the reload at the end of the run,
    // so a run whose documents moved the disk reads the tree once rather than
    // twice — and never while the run is still writing to it.
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

    // Two things, and the second is why this exists: the policy is told a reload
    // happened, which discharges what it was owed, and it is re-seated on the tree
    // that reload produced, so the next round's filter is the walk behind what is
    // on screen. A load that failed hands over `None` — the burst is still
    // discharged, because the tree was read and this is as fresh as the view is
    // going to get, and the filter stays on the last walk that worked.
    pub(crate) fn caught_up(&mut self, tree: Option<&Tree>, at: Instant) {
        self.policy.reload_started();
        if let Some(tree) = tree {
            self.policy.follow(tree);
        }
        self.policy.reload_finished(at);
    }
}

// Whether the operating system granted a watcher is a line and never an error:
// warlock with no live updates is warlock as it was. Said here rather than in the
// loop so it is said once, and through `note` so it gives way to anything the app
// already has to say.
pub(crate) fn start_watching(app: &mut App, scope: &Scope, tree: &Tree) -> Watched {
    let watched = Watched::start(scope, tree);
    if let Some(line) = watched.off_note() {
        note(app, line);
    }

    watched
}

// The one place the boundary question is asked, so `p`, `r` and `s` are refused
// on the same grounds in the same words. `Some` means refused, and the sentence
// is already on the app's message line by the time it returns.
//
// `scope_covering` walks *up*, so what this answers is whether the operator may
// act at the selected row — never what the act would reach below it. Un-pacting
// reaches the whole subtree and takes the scopes on it, so it is refused by a
// second, downward question, asked in `pacting.rs` and by `warlock unpact` in
// `edits.rs`. This function is deliberately not widened to cover that: it is
// `r`'s and `s`'s too, a pact and a refresh provably leave every scope where they
// found it, and gating a root refresh on holding every sigil in a monorepo would
// refuse the ordinary gesture. See
// `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
//
// A file row passes through as open even though coverage would happily answer for
// one: `p`, `r` and `s` all refuse a file on better grounds, and those refusals
// name the row for what it is. So does a path the manifest cannot spell — a
// boundary nobody could have drawn is not a boundary somebody is crossing.
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

    // The whole of the decision is [`verdict`]'s, and this is the panel's half
    // of what to do about it: put the sentence on the footer and hand the scope
    // back to the key that asked. The shell renders the same verdict as an
    // `Error` and neither of them works the answer out for itself.
    let path = row.path.clone();
    let Verdict::Closed { scope } =
        verdict(&path, repo_root, manifest, sigils.as_slice(), Reach::Here)
    else {
        return None;
    };

    let label = app.label_for(&path);
    app.set_message(closed_scope_message(&label, &scope));
    Some(scope)
}

// Read once, from `load_app`, before the loop starts. A sigil is written by
// `warlock config` with warlock not running, so there is nothing for a reload to
// find that this did not, and re-reading it every round would be a file opened
// ten times a second to answer a question that cannot have changed.
//
// A home that cannot be resolved reads as nothing held rather than as a config
// that would not read: there is no file in that case and no path to name one by,
// so `Sigils::Unknown` would be claiming something on disk is broken when nothing
// on disk was ever looked at.
fn sigils_held(repo_root: &Path) -> Sigils {
    Standing::home().map_or(Sigils::Nothing, |home| sigils_under(&home, repo_root))
}

// Never an error, and that is the whole reason it is a function of its own: what
// a machine holds is a line on a header, and warlock is a way of reading a tree.
// A config that will not parse becomes `Sigils::Unknown` — said out loud, so that
// broken is never drawn as absent — and nothing here can return upwards to end
// the event loop.
//
// Shared with `warlock check`, which asks the same question with nothing on
// screen: a second reading of these three cases elsewhere would be a second
// answer waiting to disagree with the header. It takes `home` rather than looking
// one up, which is what keeps every test off the developer's own.
pub(crate) fn sigils_under(home: &Path, repo_root: &Path) -> Sigils {
    match load_sigils(home, repo_root) {
        Ok(sigils) => Sigils::held(sigils),
        Err(sigils::Error::NotFound { .. }) => Sigils::Nothing,
        Err(_) => Sigils::Unknown,
    }
}

// The loop holds two paths rather than a `Standing`, so this is the one line
// that stands the front end where the subcommands already stand.
pub(crate) fn load_manifest(repo_root: &Path) -> Result<Manifest, Error> {
    Standing::at(repo_root.to_path_buf(), repo_root.to_path_buf()).manifest()
}

// The tree comes back as well as the app because the app is not a tree: its rows
// are filtered by what the reader has toggled, while the directories one walk
// produced are what the watcher's filter is. Handing it over here is what keeps
// that filter and the rows on screen born of the same walk. The two paths come
// back for the same reason they are resolved here — finding the repository root
// is a walk up the filesystem that should happen once, and the tree's root is the
// path the engine came back rooted at, which a later load must be given rather
// than guess.
//
// A load that reported problems is refused rather than drawn: the problems are
// files warlock could not read, so the nodes above them are coloured stale on no
// evidence, and drawing that would put a colour on screen nothing on disk backs
// up. That is a startup rule and stays one — mid-session, with a tree already
// showing and a run's documents already written, `reload_tree` takes the same
// problems.
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

    // `load_manifest` is entirely about what is and is not on disk, so each test
    // gets a directory nobody else writes to.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        // Named after the test using it, so a leftover says where it came from.
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

    // Every test here builds both its home *and* its repository root out of
    // these, so nothing in this module goes near the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }
}
