use std::fs;
use std::path::{Path, PathBuf};

use super::{
    Error, Forgotten, KEY_FILE, Unparseable, forget_key, keys_path, load_key, load_key_names,
    save_key,
};

// The string every test that must not see a key looks for. It is a plausible
// Linear key rather than the word "secret", so a test that passes proves the
// redaction and not an unlucky choice of needle.
const KEY: &str = "lin_api_9f3c0d11aa47";

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn hand_write(home: &Path, text: &str) {
    let path = keys_path(home);
    fs::create_dir_all(path.parent().expect("the store has a directory"))
        .expect("creates the warlock directory");
    fs::write(&path, text).expect("writes the store");
}

// `Config` has no `Debug` on purpose, so `expect_err` is not available here.
fn parse_error(text: &str) -> toml::de::Error {
    match toml::from_str::<super::Config>(text) {
        Err(error) => error,
        Ok(_) => panic!("`{text}` was meant to be unparseable"),
    }
}

fn key_dir_listing(home: &Path) -> Vec<String> {
    let path = keys_path(home);
    let dir = path.parent().expect("the store has a directory");
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

fn rendered(error: &Error) -> String {
    use std::error::Error as _;
    use std::fmt::Write as _;

    let mut text = format!("{error} | {error:?}");
    let mut cause = error.source();
    while let Some(source) = cause {
        write!(text, " | {source} | {source:?}").expect("a string never fails to grow");
        cause = source.source();
    }
    text
}

#[test]
fn the_store_sits_under_the_home_it_was_given() {
    let home = a_dir();

    assert_eq!(
        keys_path(home.path()),
        home.path().join(".warlock").join(KEY_FILE)
    );
    assert!(
        keys_path(home.path()).starts_with(home.path()),
        "the home is a parameter, and it is the only place read from"
    );
}

#[test]
fn a_missing_store_is_not_found_rather_than_no_keys() {
    let home = a_dir();

    match load_key_names(home.path()) {
        Err(Error::NotFound { path }) => assert_eq!(path, keys_path(home.path())),
        other => panic!("expected a not-found error, got {other:?}"),
    }
    assert!(
        matches!(load_key(home.path(), "work"), Err(Error::NotFound { .. })),
        "a lookup against a store nobody wrote says so too"
    );

    // And it stays distinguishable from the store that really holds nothing.
    hand_write(home.path(), "\n# no keys yet\n");
    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        Vec::<String>::new()
    );
    assert_eq!(load_key(home.path(), "work").expect("looks up"), None);
}

#[test]
fn a_name_nobody_stored_is_absent_and_a_stored_name_gives_its_key() {
    let home = a_dir();
    hand_write(
        home.path(),
        &format!("work = \"{KEY}\"\npersonal = \"lin_api_other\"\n"),
    );

    assert_eq!(
        load_key(home.path(), "work").expect("looks up").as_deref(),
        Some(KEY)
    );
    assert_eq!(
        load_key(home.path(), "nobody").expect("looks up"),
        None,
        "an absent name is an answer, not an error: the store itself is fine"
    );
    assert_eq!(
        load_key(home.path(), "Work").expect("looks up"),
        None,
        "a name is matched as written, and nothing here folds case"
    );
}

#[test]
fn the_names_come_back_sorted() {
    let home = a_dir();
    hand_write(
        home.path(),
        "web = \"lin_api_c\"\nbilling = \"lin_api_a\"\nops = \"lin_api_b\"\n",
    );

    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        ["billing", "ops", "web"],
        "one order on every machine, whatever order they were typed in"
    );
}

#[test]
fn a_malformed_store_is_an_error_and_never_a_panic() {
    let home = a_dir();

    for text in [
        "this is not toml\n",
        "work = 7\n",
        "work = [\"a\"]\n",
        "[work]\nvalue = \"a\"\n",
    ] {
        hand_write(home.path(), text);
        match load_key_names(home.path()) {
            Err(error @ Error::Syntax { .. }) => assert!(
                error.to_string().contains(KEY_FILE),
                "the message names the file to go and fix: {error}"
            ),
            other => panic!("expected a syntax error for `{text}`, got {other:?}"),
        }
        assert!(
            matches!(load_key(home.path(), "work"), Err(Error::Syntax { .. })),
            "a lookup against a broken store is broken, not absent"
        );
    }
}

#[test]
fn no_error_about_a_broken_store_carries_the_key_it_held() {
    let home = a_dir();

    for text in [
        // The parse failure is on the line the key is on, which is the case a
        // `toml::de::Error` carried as the source would print verbatim.
        format!("work = \"{KEY}\" oops\n"),
        format!("work = \"{KEY}\"\n[work]\n"),
        format!("work = {{ value = \"{KEY}\" }}\n"),
        format!("# {KEY}\nwork = \n"),
    ] {
        hand_write(home.path(), &text);
        let error = load_key_names(home.path()).expect_err("a broken store");
        assert!(matches!(error, Error::Syntax { .. }), "{error:?}");
        assert!(
            !rendered(&error).contains(KEY),
            "a key reached an error for `{text}`: {}",
            rendered(&error)
        );
    }
}

#[cfg(unix)]
#[test]
fn a_store_that_cannot_be_read_is_an_io_error_and_never_no_keys() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = a_dir();
    hand_write(home.path(), &format!("work = \"{KEY}\"\n"));
    let path = keys_path(home.path());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmods");
    if fs::read(&path).is_ok() {
        // Running as root: no file is unreadable, so there is nothing here to
        // assert against.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmods back");
        return;
    }

    let error = load_key_names(home.path()).expect_err("cannot be read");
    match &error {
        Error::Io { path: named, .. } => assert_eq!(named, &path),
        other => panic!("expected an I/O error, got {other:?}"),
    }
    assert!(!rendered(&error).contains(KEY), "{}", rendered(&error));
    assert!(
        matches!(load_key(home.path(), "work"), Err(Error::Io { .. })),
        "a store nobody can read never reads as holding no keys"
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmods back");
    assert_eq!(load_key_names(home.path()).expect("loads"), ["work"]);
}

#[test]
fn saving_creates_the_store_and_a_second_save_replaces_a_name() {
    let home = a_dir();
    assert!(!home.path().join(".warlock").exists(), "nothing there yet");

    save_key(home.path(), "work", KEY).expect("saves");
    save_key(home.path(), "personal", "lin_api_other").expect("saves a second name");
    assert_eq!(
        load_key(home.path(), "work").expect("looks up").as_deref(),
        Some(KEY)
    );
    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        ["personal", "work"]
    );

    save_key(home.path(), "work", "lin_api_rotated").expect("replaces");
    assert_eq!(
        load_key(home.path(), "work").expect("looks up").as_deref(),
        Some("lin_api_rotated"),
        "a name is the identity, so the second save replaces rather than doubles"
    );
    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        ["personal", "work"]
    );
    assert_eq!(
        key_dir_listing(home.path()),
        [KEY_FILE],
        "the temporary is renamed over the target, not left beside it"
    );
}

#[test]
fn forgetting_a_stored_name_is_told_apart_from_forgetting_one_that_is_not() {
    let home = a_dir();

    assert!(
        matches!(forget_key(home.path(), "work"), Err(Error::NotFound { .. })),
        "a store nobody wrote is not a store that has no `work` in it"
    );

    save_key(home.path(), "work", KEY).expect("saves");
    save_key(home.path(), "personal", "lin_api_other").expect("saves");

    assert_eq!(
        forget_key(home.path(), "work").expect("forgets"),
        Forgotten::Key
    );
    assert_eq!(
        forget_key(home.path(), "work").expect("forgets"),
        Forgotten::Nothing,
        "gone is an answer, and never the same answer as removed"
    );
    assert_eq!(
        load_key(home.path(), "work").expect("looks up"),
        None,
        "and the key really is out of the file"
    );
    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        ["personal"],
        "forgetting one name leaves the rest of the store alone"
    );
    assert_eq!(key_dir_listing(home.path()), [KEY_FILE]);

    // A name `validate_scope` would refuse can still be sitting in a
    // hand-edited file, and it has to be removable.
    hand_write(home.path(), "Work = \"lin_api_capital\"\n");
    assert_eq!(
        forget_key(home.path(), "Work").expect("forgets"),
        Forgotten::Key
    );
    assert_eq!(
        load_key_names(home.path()).expect("loads"),
        Vec::<String>::new()
    );
}

#[test]
fn a_key_name_that_is_not_a_name_is_refused_and_nothing_is_written() {
    let home = a_dir();

    for name in ["", "Work", "-work", "work-", "work key", &"w".repeat(25)] {
        match save_key(home.path(), name, KEY) {
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
    assert!(
        !keys_path(home.path()).exists(),
        "a refused name never creates the store it was going to be written to"
    );

    save_key(home.path(), "work", KEY).expect("saves");
    let before = fs::read_to_string(keys_path(home.path())).expect("reads");
    assert!(save_key(home.path(), "Work", "lin_api_other").is_err());
    assert_eq!(
        fs::read_to_string(keys_path(home.path())).expect("reads"),
        before,
        "and never reaches a store that already exists"
    );
}

#[test]
fn a_store_that_will_not_parse_is_never_replaced_by_a_save() {
    let home = a_dir();
    let broken = format!("work = \"{KEY}\" oops\n");
    hand_write(home.path(), &broken);

    let error = save_key(home.path(), "personal", "lin_api_other").expect_err("refuses");
    assert!(matches!(error, Error::Syntax { .. }), "{error:?}");
    assert!(!rendered(&error).contains(KEY), "{}", rendered(&error));
    assert!(
        matches!(forget_key(home.path(), "work"), Err(Error::Syntax { .. })),
        "forgetting from a store nobody can parse is broken, not absent"
    );
    assert_eq!(
        fs::read_to_string(keys_path(home.path())).expect("reads"),
        broken,
        "a key store holds credentials that exist nowhere else on the \
         machine, so a file warlock cannot read is one it does not overwrite"
    );
}

#[cfg(unix)]
#[test]
fn a_saved_store_is_readable_only_by_its_owner() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = a_dir();
    save_key(home.path(), "work", KEY).expect("saves");

    let mode = fs::metadata(keys_path(home.path()))
        .expect("the store a save just wrote")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "a Linear API key is not a file for the rest of the machine to read"
    );

    // And the mode survives a save onto a store somebody chmodded open.
    fs::set_permissions(keys_path(home.path()), fs::Permissions::from_mode(0o644))
        .expect("chmods open");
    save_key(home.path(), "personal", "lin_api_other").expect("saves again");
    assert_eq!(
        fs::metadata(keys_path(home.path()))
            .expect("the store")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn the_other_savers_write_the_files_they_always_wrote() {
    use std::os::unix::fs::PermissionsExt as _;

    let (home, elsewhere) = (a_dir(), a_dir());
    let root = elsewhere.path().join("warlock");
    fs::create_dir_all(&root).expect("creates the checkout");

    // A file written the ordinary way, as the reference: comparing against it
    // rather than against a literal keeps this about "unchanged by the key
    // store's 0o600" and not about whatever umask this machine runs under.
    let control = elsewhere.path().join("control");
    fs::write(&control, "x").expect("writes");
    let ordinary = fs::metadata(&control).expect("reads").permissions().mode() & 0o777;

    crate::sigils::save_sigils(home.path(), &root, &["web".to_owned()]).expect("saves");
    assert_eq!(
        fs::metadata(crate::sigils::sigils_path(home.path(), &root))
            .expect("the config")
            .permissions()
            .mode()
            & 0o777,
        ordinary,
        "the sigil config goes through the same `write_and_sync`, and this \
         module's mode is not applied to it"
    );

    crate::Manifest::new().save(&root).expect("saves");
    assert_eq!(
        fs::metadata(crate::manifest_path(&root))
            .expect("the manifest")
            .permissions()
            .mode()
            & 0o777,
        ordinary,
        "the manifest is committed, and a committed file nobody else can read \
         is a broken checkout"
    );
}

#[cfg(unix)]
#[test]
fn a_save_that_fails_leaves_no_temporary_and_the_store_that_was_held() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = a_dir();
    save_key(home.path(), "work", KEY).expect("saves");

    let dir = home.path().join(".warlock");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmods");
    if fs::File::create(dir.join("probe")).is_ok() {
        // Running as root: there is no such thing as an unwritable directory
        // here, so there is nothing to assert against.
        fs::remove_file(dir.join("probe")).expect("removes the probe");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
        return;
    }

    let error = save_key(home.path(), "personal", "lin_api_other").expect_err("cannot be written");
    assert!(matches!(error, Error::Io { .. }), "{error:?}");
    assert!(
        !rendered(&error).contains(KEY) && !rendered(&error).contains("lin_api_other"),
        "not even the key being written reaches the error: {}",
        rendered(&error)
    );
    let error = forget_key(home.path(), "work").expect_err("cannot be written");
    assert!(matches!(error, Error::Io { .. }), "{error:?}");
    assert!(!rendered(&error).contains(KEY), "{}", rendered(&error));

    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmods back");
    assert_eq!(
        key_dir_listing(home.path()),
        [KEY_FILE],
        "the temporary is cleaned up rather than left holding a key beside the store"
    );
    assert_eq!(
        load_key(home.path(), "work").expect("looks up").as_deref(),
        Some(KEY),
        "and the store that was held is the store that is still held"
    );
}

#[test]
fn every_error_variant_says_what_happened_and_where() {
    let not_found = Error::NotFound {
        path: PathBuf::from("/home/someone/.warlock/keys.toml"),
    };
    assert_eq!(
        not_found.to_string(),
        "no key store at `/home/someone/.warlock/keys.toml`"
    );

    let io = Error::Io {
        path: PathBuf::from("/home/someone/.warlock/keys.toml"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
    };
    assert_eq!(
        io.to_string(),
        "could not read or write `/home/someone/.warlock/keys.toml`: denied"
    );

    let text = "work = \"a\"\nbroken\n";
    let syntax = Error::Syntax {
        path: PathBuf::from("/home/someone/.warlock/keys.toml"),
        source: Unparseable::of(&parse_error(text), text),
    };
    assert_eq!(
        syntax.to_string(),
        "malformed key store at `/home/someone/.warlock/keys.toml`: \
         line 2 is not `name = \"key\"`",
        "the line is the whole of the parse error that is safe to keep"
    );

    assert_eq!(
        Error::Serialize.to_string(),
        "could not write the key store as TOML"
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

    let text = "broken\n";
    assert!(
        Error::Syntax {
            path: PathBuf::from("x"),
            source: Unparseable::of(&parse_error(text), text),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Name {
            name: "Work".to_owned(),
            rule: crate::scope::validate_scope("Work").expect_err("a capital"),
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
    assert!(
        Error::Serialize.source().is_none(),
        "a `toml::ser::Error` can carry the value it choked on, and here the \
         values are the keys"
    );
}
