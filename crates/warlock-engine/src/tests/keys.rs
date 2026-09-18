use std::fs;
use std::path::{Path, PathBuf};

use super::{Error, KEY_FILE, Unparseable, keys_path, load_key, load_key_names};

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
        Error::NotFound {
            path: PathBuf::from("x")
        }
        .source()
        .is_none()
    );
}
