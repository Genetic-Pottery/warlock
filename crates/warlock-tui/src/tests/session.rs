use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, fs, process};

use warlock_engine::{Manifest, Node, ScopedDirectory, Tree, manifest_path};

use super::{Scope, closed_scope, load_app_in, load_manifest, reload};
use crate::app::{App, Chrome, Held, Sigils};
use crate::boundary::closed_scope_message;
use crate::error::Error;
use crate::settings::Settings;

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
        chrome: Chrome::of("/repo", "/repo/crates").with_held(Held {
            sigils: Sigils::held(["billing", "web"]),
            base: None,
        }),
        settings: Settings::at(None, "/repo"),
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
fn a_repository_that_never_scoped_anything_starts_with_an_empty_manifest() {
    // The missing file is the ordinary state of a repository before its
    // first scope, so it is the empty manifest rather than an error —
    // pressing `s` is how the first `.warlock/scopes.toml` gets written.
    let scratch = Scratch::new("never-scoped");

    let manifest = load_manifest(&scratch.root).expect("a missing manifest reads as an empty one");

    assert_eq!(manifest, Manifest::new());
}

#[test]
fn the_manifest_on_disk_is_the_one_the_session_starts_from() {
    let scratch = Scratch::new("round-trip");
    let mut saved = Manifest::new();
    saved.push(
        ScopedDirectory::new(".", "crates/engine", "data-plane")
            .expect("a directory spelled relative to the root"),
    );
    saved.save(&scratch.root).expect("a manifest that writes");

    let loaded = load_manifest(&scratch.root).expect("the manifest just saved loads");

    assert_eq!(loaded, saved);
}

#[test]
fn a_manifest_that_will_not_parse_is_an_error_rather_than_an_empty_start() {
    // Only the *missing* file maps to the empty manifest. A file that is
    // there and will not parse is somebody's record of what is scoped,
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

// A load walks up looking for `.git/` and refuses without one, so every
// repository here has a `HEAD` in it.
fn a_repository(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    scratch.write(".git/HEAD", "ref: refs/heads/main\n");
    scratch.write("crates/engine/src/lib.rs", "//! Engine.\n");
    scratch
}

// Spelled against `.` rather than against the scratch root, so a temporary
// directory reached through a symlink cannot make the paths disagree with what
// the load walks.
fn scoped(directories: &[(&str, &str)]) -> Manifest {
    Manifest::with_directories(directories.iter().map(|(path, scope)| {
        ScopedDirectory::new(".", path, *scope).expect("a directory spelled relative to the root")
    }))
}

fn a_scope(scratch: &Scratch) -> Scope {
    Scope {
        root: scratch.root.clone(),
        repo_root: scratch.root.clone(),
        chrome: Chrome::of(&scratch.root, &scratch.root),
        settings: Settings::at(None, &scratch.root),
    }
}

#[test]
fn a_reload_replaces_the_sessions_manifest_with_the_one_on_disk() {
    // A scope written in another terminal: the rows show it, so every key
    // has to judge and save against it too.
    let scratch = a_repository("reload-manifest");
    let on_disk = scoped(&[("crates", "platform")]);
    on_disk.save(&scratch.root).expect("a manifest that writes");
    let mut app = App::default();
    let mut manifest = Manifest::new();

    assert!(
        reload(&mut app, &a_scope(&scratch), &mut manifest).is_some(),
        "the tree was not read"
    );

    assert_eq!(manifest, on_disk);
    let crates = app
        .rows()
        .iter()
        .find(|row| row.path == scratch.root.join("crates"))
        .expect("a row for the scoped directory");
    assert!(crates.covered, "the reloaded rows do not show the scope");
}

#[test]
fn a_reload_that_finds_a_scope_it_cannot_read_draws_the_tree_and_says_so() {
    let scratch = a_repository("reload-problem");
    scoped(&[("crates", "Not A Scope")])
        .save(&scratch.root)
        .expect("a manifest that writes");
    let mut app = App::default();

    assert!(
        reload(&mut app, &a_scope(&scratch), &mut Manifest::new()).is_some(),
        "a problem kept the tree off the screen mid-session"
    );

    let message = app.message().expect("the problem is said");
    assert!(message.contains("Not A Scope"), "{message}");
    assert!(!message.contains('\n'), "{message}");
}

#[test]
fn the_first_load_refuses_a_scope_it_cannot_read_rather_than_drawing_it_unscoped() {
    let scratch = a_repository("startup-problem");
    scoped(&[("crates", "Not A Scope")])
        .save(&scratch.root)
        .expect("a manifest that writes");

    let Err(error) = load_app_in(&scratch.root, None) else {
        panic!("a tree with a misspelt scope was drawn");
    };

    assert!(matches!(error, Error::Problems { .. }), "{error:?}");
}

#[test]
fn the_first_load_keeps_the_repository_root_it_resolved() {
    let scratch = a_repository("startup-root");

    let (app, scope, _tree, manifest) =
        load_app_in(&scratch.root, None).expect("a repository that loads");

    assert_eq!(scope.repo_root, scratch.root);
    assert_eq!(manifest, Manifest::new());
    assert_eq!(app.message(), None, "a clean load has nothing to say");
}

const ROOT: &str = "/repo";

//     /repo                 unscoped
//     ├── crates            scoped `platform`
//     │   └── engine        covered by `platform`
//     │       lib.rs
//     └── assets            unscoped
fn boundary_app() -> (App, Manifest) {
    let root = PathBuf::from(ROOT);
    let engine = root.join("crates/engine");
    let tree = Tree::new(
        Node::new(&root).with_children([
            Node::new(root.join("crates"))
                .with_scope(Some("platform".to_owned()))
                .with_children([Node::new(&engine)
                    .with_covering(Some("platform".to_owned()))
                    .with_files([engine.join("lib.rs")])]),
            Node::new(root.join("assets")),
        ]),
    );
    let mut app = App::from_tree(&tree);
    app.toggle_files();
    (app, scoped(&[("crates", "platform")]))
}

fn select(app: &mut App, path: &str) {
    let path = PathBuf::from(ROOT).join(path);
    app.select_first();
    while app.selected_row().expect("the fixture has rows").path != path {
        let before = app.selected();
        app.select_next();
        assert_ne!(app.selected(), before, "no row for {}", path.display());
    }
}

#[test]
fn an_unscoped_directory_is_open_to_s_so_the_first_scope_can_be_drawn() {
    let (mut app, manifest) = boundary_app();
    select(&mut app, "assets");

    assert!(!closed_scope(
        &mut app,
        &manifest,
        PathBuf::from(ROOT).as_path(),
        &Sigils::held(Vec::<String>::new()),
    ));
    assert_eq!(app.message(), None, "an open boundary said something");
}

#[test]
fn a_directory_under_a_scope_this_machine_does_not_hold_is_refused_in_the_boundarys_words() {
    let (mut app, manifest) = boundary_app();
    select(&mut app, "crates/engine");

    assert!(closed_scope(
        &mut app,
        &manifest,
        PathBuf::from(ROOT).as_path(),
        &Sigils::held(["data-plane"]),
    ));
    assert_eq!(
        app.message(),
        Some(closed_scope_message("crates/engine", "platform").as_str())
    );
}

#[test]
fn a_directory_under_a_held_scope_is_open() {
    let (mut app, manifest) = boundary_app();
    select(&mut app, "crates/engine");

    assert!(!closed_scope(
        &mut app,
        &manifest,
        PathBuf::from(ROOT).as_path(),
        &Sigils::held(["platform"]),
    ));
    assert_eq!(app.message(), None);
}

#[test]
fn a_file_row_is_left_for_s_to_refuse_in_its_own_words() {
    let (mut app, manifest) = boundary_app();
    select(&mut app, "crates/engine/lib.rs");

    assert!(!closed_scope(
        &mut app,
        &manifest,
        PathBuf::from(ROOT).as_path(),
        &Sigils::held(Vec::<String>::new()),
    ));
    assert_eq!(app.message(), None);
}
