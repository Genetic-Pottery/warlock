use std::fs;
use std::path::{Path, PathBuf};

use super::{
    DIGEST_CHARACTERS, Error, MAXIMUM_NAME_CHARACTERS, SIGIL_FILE, held_sigils, load_key_binding,
    load_sigils, project_directory, save_key_binding, save_sigils, sigils_path,
};

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn named(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir_all(&path).expect("creates the directory");
    path
}

fn owned(sigils: &[&str]) -> Vec<String> {
    sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
}

fn hand_write(home: &Path, root: &Path, text: &str) {
    let path = sigils_path(home, root);
    fs::create_dir_all(path.parent().expect("the config has a directory"))
        .expect("creates the project directory");
    fs::write(&path, text).expect("writes the config");
}

fn project_dir_listing(home: &Path, root: &Path) -> Vec<String> {
    let dir = sigils_path(home, root);
    let dir = dir.parent().expect("the config has a directory");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the directory a save just created")
        .map(|entry| {
            entry
                .expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn the_same_root_derives_the_same_directory_every_time() {
    let home = a_dir();
    let root = named(home.path(), "warlock");

    let derived = project_directory(&root);
    assert_eq!(derived, project_directory(&root), "byte for byte");
    assert_eq!(derived, project_directory(root.join(".")), "and again");

    let (name, digest) = derived
        .rsplit_once('-')
        .expect("a name, a dash and a digest");
    assert_eq!(name, "warlock", "the directory's own name, to read it by");
    assert_eq!(digest.len(), DIGEST_CHARACTERS);
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "{digest}"
    );
}

#[test]
fn two_checkouts_with_the_same_name_derive_different_directories() {
    let (one, two) = (a_dir(), a_dir());
    let (here, there) = (named(one.path(), "warlock"), named(two.path(), "warlock"));
    assert_ne!(here, there, "two genuinely different absolute paths");

    assert!(project_directory(&here).starts_with("warlock-"));
    assert!(project_directory(&there).starts_with("warlock-"));
    assert_ne!(
        project_directory(&here),
        project_directory(&there),
        "the name alone collides across checkouts, which is what the \
             digest of the absolute path is for"
    );

    // And the consequence that matters: neither can read the other's file.
    let home = a_dir();
    save_sigils(home.path(), &here, &owned(&["data-plane"])).expect("saves");
    assert!(matches!(
        load_sigils(home.path(), &there),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn a_root_that_cannot_be_canonicalised_still_derives_deterministically() {
    let home = a_dir();
    let gone = home.path().join("deleted-mid-run");

    assert_eq!(
        project_directory(&gone),
        project_directory(&gone),
        "the path as handed over is digested instead, rather than a panic"
    );
    assert_ne!(
        project_directory(&gone),
        project_directory(home.path().join("a-different-one")),
        "and it is still a derivation, not a constant"
    );
}

#[test]
fn the_derived_directory_is_one_readable_path_segment() {
    let home = a_dir();
    let awkward = named(home.path(), "my repo (2)!");
    let derived = project_directory(&awkward);

    assert!(derived.starts_with("my-repo--2--"), "{derived}");
    assert_eq!(
        Path::new(&derived).components().count(),
        1,
        "whatever a repository is called, this is one directory name: {derived}"
    );

    let long = named(home.path(), &"a".repeat(MAXIMUM_NAME_CHARACTERS + 40));
    let derived = project_directory(&long);
    assert_eq!(
        derived.len(),
        MAXIMUM_NAME_CHARACTERS + 1 + DIGEST_CHARACTERS,
        "the name is capped so the segment fits a filesystem: {derived}"
    );
}

#[test]
fn the_config_sits_under_the_home_it_was_given() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    assert_eq!(
        sigils_path(home.path(), &root),
        home.path()
            .join(".warlock")
            .join(project_directory(&root))
            .join(SIGIL_FILE),
    );
    assert!(
        sigils_path(home.path(), &root).starts_with(home.path()),
        "the home is a parameter, and it is the only place written to"
    );
}

#[test]
fn saving_then_loading_gives_the_set_back() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    for set in [vec![], owned(&["data-plane"]), owned(&["billing", "web"])] {
        save_sigils(home.path(), &root, &set).expect("saves");
        assert_eq!(
            load_sigils(home.path(), &root).expect("loads"),
            set,
            "a set replaces whatever was held, empty included"
        );
    }
}

#[test]
fn the_file_holds_a_sigils_key_and_nothing_else() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    save_sigils(home.path(), &root, &owned(&["billing", "web"])).expect("saves");
    assert_eq!(
        fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
        "sigils = [\"billing\", \"web\"]\n",
    );

    save_sigils(home.path(), &root, &[]).expect("saves an empty set");
    assert_eq!(
        fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
        "sigils = []\n",
        "an empty set is written as one, not as an absent key or a deleted file"
    );
}

#[test]
fn saving_creates_the_directory_and_leaves_no_temporary_behind() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    assert!(!home.path().join(".warlock").exists(), "nothing there yet");

    save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");
    assert_eq!(project_dir_listing(home.path(), &root), [SIGIL_FILE]);

    // And a second save renames over the first rather than accumulating.
    save_sigils(home.path(), &root, &owned(&["billing"])).expect("saves again");
    assert_eq!(project_dir_listing(home.path(), &root), [SIGIL_FILE]);
    assert_eq!(load_sigils(home.path(), &root).expect("loads"), ["billing"]);
}

#[test]
fn nothing_is_written_inside_the_repository() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");
    load_sigils(home.path(), &root).expect("loads");

    assert_eq!(
        fs::read_dir(&root).expect("reads the repository").count(),
        0,
        "a sigil is a fact about a machine, and never a file in a checkout"
    );
}

#[test]
fn a_missing_config_is_not_found_rather_than_an_empty_set() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    match load_sigils(home.path(), &root) {
        Err(Error::NotFound { path }) => assert_eq!(path, sigils_path(home.path(), &root)),
        other => panic!("expected a not-found error, got {other:?}"),
    }

    // A home with no `.warlock` at all is the same answer, not an I/O error
    // about the missing parent.
    assert!(matches!(
        load_sigils(home.path().join("nowhere"), &root),
        Err(Error::NotFound { .. })
    ));

    // And it stays distinguishable from the set that really is empty.
    save_sigils(home.path(), &root, &[]).expect("saves");
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        Vec::<String>::new()
    );
}

#[test]
fn a_malformed_config_is_an_error_and_never_a_panic() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    for text in [
        "this is not toml\n",
        "sigils = 7\n",
        "sigils = [1, 2]\n",
        // The misspelling `deny_unknown_fields` exists to catch: read
        // leniently, this would be a config holding nothing.
        "sigil = [\"data-plane\"]\n",
    ] {
        hand_write(home.path(), &root, text);
        match load_sigils(home.path(), &root) {
            Err(error @ Error::Syntax { .. }) => assert!(
                error.to_string().contains(SIGIL_FILE),
                "the message names the file to go and fix: {error}"
            ),
            other => panic!("expected a syntax error for `{text}`, got {other:?}"),
        }
    }

    // Broken is never absent, and never an empty set either.
    hand_write(home.path(), &root, "sigils = [\"data-plane\"]\n");
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        ["data-plane"]
    );
}

#[test]
fn a_config_that_says_nothing_about_sigils_holds_none() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    hand_write(home.path(), &root, "\n# nothing here yet\n");

    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        Vec::<String>::new()
    );
}

#[test]
fn sigils_are_stored_and_read_back_exactly_as_written() {
    // No folding, no trimming, no rejection, no de-duplication: this is the
    // store, not the gate. `validate_sigil` is asked where a person types.
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    let odd = owned(&["Data-Plane", " billing ", "*", "", "web", "web"]);

    save_sigils(home.path(), &root, &odd).expect("saves");
    assert_eq!(load_sigils(home.path(), &root).expect("loads"), odd);
}

#[test]
fn a_config_written_before_bindings_existed_reads_as_unbound() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    hand_write(home.path(), &root, "sigils = [\"billing\", \"web\"]\n");
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        ["billing", "web"],
        "the field is optional, so yesterday's file is still today's file"
    );
    assert_eq!(
        load_key_binding(home.path(), &root).expect("loads"),
        None,
        "no binding, rather than a default one"
    );

    // And unbound stays distinguishable from never configured.
    let never = named(elsewhere.path(), "other");
    assert!(matches!(
        load_key_binding(home.path(), &never),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn a_config_that_names_a_key_still_gives_its_sigils_back() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    hand_write(
        home.path(),
        &root,
        "sigils = [\"billing\", \"web\"]\nkey = \"work\"\n",
    );

    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        ["billing", "web"]
    );
    assert_eq!(
        load_key_binding(home.path(), &root).expect("loads"),
        Some("work".to_owned())
    );
}

#[test]
fn a_binding_and_a_set_of_sigils_do_not_overwrite_each_other() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    save_sigils(home.path(), &root, &owned(&["billing", "web"])).expect("saves");
    save_key_binding(home.path(), &root, "work").expect("binds");
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        ["billing", "web"],
        "binding a key is not a way to lose the sigils already held"
    );
    assert_eq!(
        fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
        "sigils = [\"billing\", \"web\"]\nkey = \"work\"\n",
    );

    save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves again");
    assert_eq!(
        load_key_binding(home.path(), &root).expect("loads"),
        Some("work".to_owned()),
        "and changing the sigils is not a way to lose the binding"
    );

    // A rebinding replaces the name rather than accumulating names or files.
    save_key_binding(home.path(), &root, "personal").expect("rebinds");
    assert_eq!(
        load_key_binding(home.path(), &root).expect("loads"),
        Some("personal".to_owned())
    );
    assert_eq!(project_dir_listing(home.path(), &root), [SIGIL_FILE]);
}

#[test]
fn binding_the_first_key_for_a_checkout_needs_no_config_to_already_exist() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    save_key_binding(home.path(), &root, "work").expect("binds");
    assert_eq!(
        load_key_binding(home.path(), &root).expect("loads"),
        Some("work".to_owned())
    );
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        Vec::<String>::new()
    );
}

#[test]
fn a_key_name_that_is_not_a_name_is_refused_and_nothing_is_written() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    save_sigils(home.path(), &root, &owned(&["web"])).expect("saves");
    let before = fs::read_to_string(sigils_path(home.path(), &root)).expect("reads");

    for name in ["", "Work", "-work", "work-", "work key", &"w".repeat(25)] {
        match save_key_binding(home.path(), &root, name) {
            Err(Error::Name {
                name: refused,
                rule,
            }) => {
                assert_eq!(refused, name);
                assert!(
                    !rule.to_string().is_empty(),
                    "the refusal names the rule broken"
                );
            }
            other => panic!("expected `{name}` to be refused, got {other:?}"),
        }
    }

    assert_eq!(
        fs::read_to_string(sigils_path(home.path(), &root)).expect("reads"),
        before,
        "a refused name never reaches the file"
    );
}

#[test]
fn every_error_variant_says_what_happened_and_where() {
    let not_found = Error::NotFound {
        path: PathBuf::from("/home/someone/.warlock/warlock-abc/config.toml"),
    };
    assert_eq!(
        not_found.to_string(),
        "no sigil config at `/home/someone/.warlock/warlock-abc/config.toml`"
    );

    let io = Error::Io {
        path: PathBuf::from("/home/someone/.warlock"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
    };
    assert_eq!(
        io.to_string(),
        "could not read or write `/home/someone/.warlock`: denied"
    );

    let syntax = Error::Syntax {
        path: PathBuf::from("/home/someone/.warlock/warlock-abc/config.toml"),
        source: toml::from_str::<super::Config>("sigils = 7").expect_err("a number is not a list"),
    };
    assert!(
        syntax.to_string().starts_with(
            "malformed sigil config at `/home/someone/.warlock/warlock-abc/config.toml`: "
        ),
        "{syntax}"
    );

    let name = Error::Name {
        name: "Work".to_owned(),
        rule: crate::scope::validate_scope("Work").expect_err("a capital is not a scope character"),
    };
    assert_eq!(
        name.to_string(),
        "`Work` is not a key name: a scope holds only lowercase letters, \
         digits, `-` and `_`, and this one holds `W`"
    );
}

#[test]
fn errors_expose_the_cause_they_wrap() {
    use std::error::Error as _;

    assert!(
        Error::Io {
            path: PathBuf::from("x"),
            source: std::io::Error::other("boom"),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Syntax {
            path: PathBuf::from("x"),
            source: toml::from_str::<super::Config>("sigils = 7")
                .expect_err("a number is not a list"),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::NotFound {
            path: PathBuf::from("x")
        }
        .source()
        .is_none()
    );
}

#[cfg(unix)]
#[test]
fn two_paths_to_one_checkout_derive_one_directory() {
    let home = a_dir();
    let root = named(home.path(), "warlock");
    let link = home.path().join("shortcut");
    std::os::unix::fs::symlink(&root, &link).expect("links to the checkout");

    assert_eq!(
        project_directory(&link),
        project_directory(&root),
        "canonicalising is what makes one checkout one project directory"
    );
}

#[cfg(unix)]
#[test]
fn a_save_that_fails_leaves_no_temporary_and_no_half_written_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");
    save_sigils(home.path(), &root, &owned(&["data-plane"])).expect("saves");

    let config = sigils_path(home.path(), &root);
    let dir = config
        .parent()
        .expect("the config has a directory")
        .to_path_buf();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmods");
    if fs::File::create(dir.join("probe")).is_ok() {
        // Running as root: there is no such thing as an unwritable
        // directory here, so there is nothing to assert against.
        fs::remove_file(dir.join("probe")).expect("removes the probe");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
        return;
    }

    let error =
        save_sigils(home.path(), &root, &owned(&["billing"])).expect_err("cannot be written");
    assert!(matches!(error, Error::Io { .. }), "{error:?}");

    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
    assert_eq!(
        project_dir_listing(home.path(), &root),
        [SIGIL_FILE],
        "the temporary is cleaned up rather than left beside the config"
    );
    assert_eq!(
        load_sigils(home.path(), &root).expect("loads"),
        ["data-plane"],
        "and the set that was held is the set that is still held"
    );
}

#[test]
fn held_sigils_reads_a_missing_config_as_nothing_held_and_a_broken_one_as_broken() {
    let (home, elsewhere) = (a_dir(), a_dir());
    let root = named(elsewhere.path(), "warlock");

    assert_eq!(
        held_sigils(home.path(), &root).expect("absent is an answer"),
        Vec::<String>::new()
    );

    save_sigils(home.path(), &root, &["data-plane".to_owned()]).expect("saves");
    assert_eq!(
        held_sigils(home.path(), &root).expect("loads"),
        ["data-plane"]
    );

    hand_write(home.path(), &root, "this is not toml\n");
    assert!(matches!(
        held_sigils(home.path(), &root),
        Err(Error::Syntax { .. })
    ));
}
