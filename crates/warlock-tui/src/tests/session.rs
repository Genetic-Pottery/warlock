use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, fs, process};

use warlock_engine::{Manifest, PactEntry, manifest_path, save_sigils, sigils_path};
use warlock_tui::{App, Chrome, Sigils};

use super::{NOT_CLEANED, Scope, load_app_in, load_manifest, reload, sigils_under};
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
    // `reload` reads `scope.chrome` and writes it nowhere.
    let scope = Scope {
        root: PathBuf::from("/repo/crates"),
        repo_root: PathBuf::from("/repo"),
        chrome: Chrome::of("/repo", "/repo/crates").with_sigils(Sigils::held(["billing", "web"])),
    };

    // A load from a path that is not there fails, which is the arm that
    // keeps the tree already drawn — and the arm that would be the last
    // chance to lose a header if one could still be lost here.
    let mut app = App::default();
    assert_eq!(reload(&mut app, &scope, &mut Manifest::new()), None);

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

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("a file has a directory above it"))
            .expect("the directories above the file");
        fs::write(&path, contents).expect("a file under the scratch directory");
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

    let manifest = load_manifest(&scratch.root).expect("a missing manifest reads as an empty one");

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

// A load walks up looking for `.git/` and refuses without one, so every
// repository here has a `HEAD` in it. `vendor/` is what the rules exclude
// and `crates/` is what survives them.
fn a_repository(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    scratch.write(".git/HEAD", "ref: refs/heads/main\n");
    scratch.write(".warlockignore", "vendor/\n");
    scratch.write("crates/engine/src/lib.rs", "//! Engine.\n");
    scratch.write("vendor/acme/src/lib.rs", "//! Acme.\n");
    scratch
}

// Spelled against `.` rather than against the scratch root, so a temporary
// directory reached through a symlink cannot make the modules disagree with
// what the cleanup walks.
fn pacted(modules: &[&str]) -> Manifest {
    Manifest::with_entries(modules.iter().map(|module| {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a module spelled relative to the root")
    }))
}

fn a_scope(scratch: &Scratch) -> Scope {
    Scope {
        root: scratch.root.clone(),
        repo_root: scratch.root.clone(),
        chrome: Chrome::of(&scratch.root, &scratch.root),
    }
}

fn modules_on_disk(scratch: &Scratch) -> Vec<String> {
    load_manifest(&scratch.root)
        .expect("a manifest that reads")
        .entries()
        .iter()
        .map(|entry| entry.module().to_owned())
        .collect()
}

#[test]
fn a_load_drops_the_entries_the_repository_has_since_excluded() {
    let scratch = a_repository("ignored-dropped");
    pacted(&["crates", "vendor", "vendor/acme"])
        .save(&scratch.root)
        .expect("a manifest that writes");
    let mut app = App::default();

    assert!(
        reload(&mut app, &a_scope(&scratch), &mut Manifest::new()).is_some(),
        "the tree was not read"
    );

    assert_eq!(
        modules_on_disk(&scratch),
        ["crates"],
        "`pacts.toml` still records directories nothing will walk again"
    );
    assert_eq!(
        app.message(),
        None,
        "a cleanup that did its work has nothing to say"
    );
}

#[test]
fn a_load_with_nothing_to_drop_leaves_pacts_toml_exactly_as_it_was() {
    // The watcher compares for `pacts.toml` by name, so a save here would
    // ask for another reload, which would save again: every load writing
    // the file is every load asking for the next one.
    let scratch = a_repository("ignored-nothing");
    pacted(&["crates"])
        .save(&scratch.root)
        .expect("a manifest that writes");
    let path = manifest_path(&scratch.root);
    let before = fs::read(&path).expect("the manifest just saved");
    let written_at = fs::metadata(&path)
        .and_then(|manifest| manifest.modified())
        .expect("a modification time");
    let mut app = App::default();

    assert!(
        reload(&mut app, &a_scope(&scratch), &mut Manifest::new()).is_some(),
        "the tree was not read"
    );

    assert_eq!(
        fs::read(&path).expect("the manifest is still there"),
        before,
        "the manifest was rewritten with nothing to remove"
    );
    assert_eq!(
        fs::metadata(&path)
            .and_then(|manifest| manifest.modified())
            .expect("a modification time"),
        written_at,
        "the file was written again, byte for byte"
    );
    assert_eq!(app.message(), None, "nothing happened, so nothing is said");
}

#[test]
fn a_cleanup_that_cannot_be_finished_is_one_line_and_not_an_ended_session() {
    let scratch = a_repository("ignored-unreadable");
    scratch.write(".warlock/pacts.toml", "not a manifest\n");
    let mut app = App::default();

    reload(&mut app, &a_scope(&scratch), &mut Manifest::new());

    let message = app
        .message()
        .expect("the reader is told nothing was dropped");
    assert!(
        message.starts_with(NOT_CLEANED),
        "the cleanup's own line is the one on the footer: {message}"
    );
    assert!(
        !message.contains('\n'),
        "a footer line that wraps is a footer line that hides a row: {message}"
    );
}

#[cfg(unix)]
#[test]
fn a_cleanup_that_cannot_save_says_so_and_still_draws_the_tree() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = a_repository("ignored-readonly");
    let manifest = pacted(&["crates", "vendor"]);
    manifest.save(&scratch.root).expect("the first save works");
    let directory = scratch.root.join(".warlock");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555))
        .expect("chmods the manifest directory read-only");
    let mut app = App::default();

    let tree = reload(&mut app, &a_scope(&scratch), &mut Manifest::new());

    // Back to writable before anything can fail, so the scratch repository
    // can still be removed.
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).expect("chmods it back");

    assert!(
        tree.is_some(),
        "a cleanup that could not write kept the tree off the screen"
    );
    let message = app
        .message()
        .expect("the reader is told nothing was dropped");
    assert!(
        message.starts_with(NOT_CLEANED),
        "the cleanup's own line is the one on the footer: {message}"
    );
    assert!(
        !message.contains('\n'),
        "a footer line that wraps is a footer line that hides a row: {message}"
    );
    assert_eq!(
        load_manifest(&scratch.root).expect("a manifest that reads"),
        manifest,
        "a save that failed took entries with it"
    );
}

#[test]
fn the_first_load_of_the_session_drops_the_entries_the_repository_has_since_excluded() {
    // The rule can be added with warlock not running, so the startup load
    // is the first chance anything has to notice it — waiting for a second
    // load would draw one tree from a manifest the reader can see is wrong.
    let scratch = a_repository("ignored-at-startup");
    pacted(&["crates", "vendor", "vendor/acme"])
        .save(&scratch.root)
        .expect("a manifest that writes");

    let (app, scope, _tree, _manifest) =
        load_app_in(&scratch.root, None).expect("a repository that loads");

    assert_eq!(
        modules_on_disk(&scratch),
        ["crates"],
        "`pacts.toml` still records directories nothing will walk again"
    );
    assert_eq!(
        scope.repo_root, scratch.root,
        "the repository root resolved before the load is the one kept"
    );
    assert_eq!(
        app.message(),
        None,
        "a cleanup that did its work has nothing to say"
    );
}

#[cfg(unix)]
#[test]
fn a_cleanup_that_cannot_be_finished_at_startup_is_a_line_rather_than_a_refusal_to_start() {
    // The startup load refuses a tree whose nodes it could not colour from
    // what is on disk, and this is deliberately not that: the manifest the
    // cleanup could not rewrite is exactly the one the rows were coloured
    // from, so there is nothing dishonest to draw.
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = a_repository("ignored-at-startup-readonly");
    let manifest = pacted(&["crates", "vendor"]);
    manifest.save(&scratch.root).expect("the first save works");
    let directory = scratch.root.join(".warlock");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555))
        .expect("chmods the manifest directory read-only");

    let started = load_app_in(&scratch.root, None);

    // Back to writable before anything can fail, so the scratch repository
    // can still be removed.
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).expect("chmods it back");

    let (app, _scope, _tree, _manifest) =
        started.expect("a cleanup that could not save ended the session");
    let message = app
        .message()
        .expect("the reader is told nothing was dropped");
    assert!(
        message.starts_with(NOT_CLEANED),
        "the cleanup's own line is the one on the footer: {message}"
    );
    assert!(
        !message.contains('\n'),
        "a footer line that wraps is a footer line that hides a row: {message}"
    );
    assert_eq!(
        load_manifest(&scratch.root).expect("a manifest that reads"),
        manifest,
        "a save that failed took entries with it"
    );
}

#[test]
fn a_cleanup_that_failed_is_attempted_again_on_the_next_load() {
    // Nothing remembers the failure, which is the point: a manifest fixed
    // in another window between two reloads is cleaned by the second, with
    // no key pressed and no restart.
    let scratch = a_repository("ignored-again");
    scratch.write(".warlock/pacts.toml", "not a manifest\n");
    let scope = a_scope(&scratch);
    let mut app = App::default();

    reload(&mut app, &scope, &mut Manifest::new());
    assert!(
        app.message()
            .is_some_and(|line| line.starts_with(NOT_CLEANED)),
        "the first load's cleanup was expected to fail"
    );
    pacted(&["crates", "vendor"])
        .save(&scratch.root)
        .expect("a manifest that writes");

    reload(&mut app, &scope, &mut Manifest::new());

    assert_eq!(
        modules_on_disk(&scratch),
        ["crates"],
        "the second load did not try the cleanup again"
    );
}

// Every test here builds both its home *and* its repository root out of
// these, so nothing in this module goes near the developer's real home.
fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}
