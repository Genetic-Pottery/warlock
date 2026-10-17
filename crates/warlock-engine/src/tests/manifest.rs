use std::fs;
use std::path::{Path, PathBuf};

use super::{
    BoardScope, Error, FiledTo, MANIFEST_FILE, Manifest, NotLinear, SCHEMA_VERSION, ScopeFault,
    ScopeRecord, ScopedDirectory, TrackerName, from_manifest_path, manifest_path, to_manifest_path,
};

fn engine() -> ScopedDirectory {
    ScopedDirectory::new(".", "crates/warlock-engine", "data-plane")
        .expect("a relative path inside the root is storable")
}

fn tui() -> ScopedDirectory {
    ScopedDirectory::new(".", "crates/warlock-tui", "billing")
        .expect("a relative path inside the root is storable")
}

fn a_de_error() -> toml::de::Error {
    toml::from_str::<Manifest>("version = \"one\"").expect_err("a string is not an integer")
}

fn a_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn warlock_dir_listing(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join(".warlock"))
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

fn prefix_of(record: &ScopeRecord) -> &str {
    BoardScope::of(record).ticket_prefix()
}

fn hand_write(root: &Path, text: &str) {
    let path = manifest_path(root);
    fs::create_dir_all(path.parent().expect("the manifest has a directory"))
        .expect("creates .warlock");
    fs::write(&path, text).expect("writes the manifest");
}

#[test]
fn a_scoped_directory_round_trips_byte_for_byte() {
    let root = a_root();
    // Hand-written, so this says where the keys sit in the file and not
    // merely that the serialiser agrees with itself.
    let original = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded,
        Manifest::with_directories([engine()]),
        "the file and the constructed directory are the same manifest"
    );
    assert_eq!(loaded.directories()[0].path(), "crates/warlock-engine");
    assert_eq!(loaded.directories()[0].scope(), "data-plane");

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "a load-then-save is a no-op on the bytes of a scoped directory"
    );
}

#[test]
fn an_invalid_scope_loads_untouched_and_is_written_back_unchanged() {
    // The reader validates nothing and normalises nothing: these bytes are
    // committed, and correcting them on somebody's next save would put a
    // line in a diff they did not author. Whether a scope is well formed is
    // `validate_scope`'s question, asked elsewhere.
    let invalid = [
        "",
        "1data",
        "data-",
        "*",
        "abcdefghijklmnopqrstuvwxy", // 25 characters
        "données",
        "Data-Plane",
    ];

    for scope in invalid {
        let root = a_root();
        let original = format!(
            concat!(
                "version = 2\n\n",
                "[[directory]]\n",
                "path = \"crates/warlock-engine\"\n",
                "scope = \"{}\"\n",
            ),
            scope
        );
        hand_write(root.path(), &original);

        let loaded = Manifest::load(root.path())
            .unwrap_or_else(|error| panic!("`{scope}` is not fatal, got {error:?}"));
        let directory = &loaded.directories()[0];
        assert_eq!(directory.path(), "crates/warlock-engine", "for `{scope}`");
        assert_eq!(
            directory.scope(),
            scope,
            "read back as written, not repaired and not dropped"
        );

        loaded.save(root.path()).expect("saves");
        assert_eq!(
            fs::read_to_string(manifest_path(root.path())).expect("reads"),
            original,
            "the invalid line survives a load-then-save byte for byte"
        );
    }
}

#[test]
fn a_scope_is_stored_as_written_however_odd() {
    // No folding, no trimming, no rejection: `ScopedDirectory::new` is the
    // store, not the gate. Two spellings staying two spellings is exactly why
    // the folding belongs where a person types.
    for scope in ["Data-Plane", " data-plane ", "*", ""] {
        let directory =
            ScopedDirectory::new(".", "crates/warlock-engine", scope).expect("inside the root");
        assert_eq!(directory.scope(), scope);
    }
}

#[test]
fn a_directory_row_missing_either_key_is_refused() {
    for body in [
        "[[directory]]\npath = \"crates/warlock-engine\"\n",
        "[[directory]]\nscope = \"data-plane\"\n",
    ] {
        let text = format!("version = 2\n\n{body}");
        assert!(
            matches!(
                Manifest::from_toml_str(&text),
                Err(Error::Directory { index: 0, .. })
            ),
            "both keys are required: {body}"
        );
    }
}

#[test]
fn a_directory_row_with_an_unknown_key_is_refused() {
    let text = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n",
        "document = \"crates/warlock-engine/.warlock.md\"\n",
    );
    match Manifest::from_toml_str(text) {
        Err(error @ Error::Directory { index: 0, .. }) => {
            assert!(
                error.to_string().contains("crates/warlock-engine"),
                "{error}"
            );
        }
        other => panic!("expected a directory error, got {other:?}"),
    }
}

#[test]
fn a_scope_record_round_trips_byte_for_byte() {
    let root = a_root();
    // Hand-written, so this says where the table sits in the file and in what
    // order its keys are written, not merely that the serialiser agrees with
    // itself.
    let original = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n\n",
        "[[scope]]\n",
        "name = \"data-plane\"\n",
        "tracker = \"linear\"\n",
        "team = \"War \"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/Data_Plane\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded,
        Manifest::with_directories([engine()]).with_scopes([ScopeRecord::new(
            "data-plane",
            "War ",
            "In Review",
            "area/Data_Plane"
        )]),
        "the file and the constructed manifest are the same manifest"
    );

    let record = &loaded.scopes()[0];
    assert_eq!(record.name(), "data-plane");
    assert_eq!(prefix_of(record), "War ", "neither trimmed nor folded");
    assert_eq!(record.review_state(), "In Review", "not folded");
    assert_eq!(record.label(), "area/Data_Plane", "not folded either");

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "a load-then-save is a no-op on the bytes of a record too"
    );
}

#[test]
fn a_jira_record_round_trips_byte_for_byte() {
    let root = a_root();
    // The second tracker, hand-written for the first one's reason: this says
    // that `tracker` comes before the two keys it decides and that neither the
    // URL's trailing slash nor the spaces around the project key are touched.
    let original = concat!(
        "version = 2\n\n",
        "[[scope]]\n",
        "name = \"payments\"\n",
        "tracker = \"jira\"\n",
        "base_url = \"https://acme.atlassian.net/\"\n",
        "project_key = \" PAY \"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/pay\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded,
        Manifest::new().with_scopes([ScopeRecord::jira(
            "payments",
            "https://acme.atlassian.net/",
            " PAY ",
            "In Review",
            "area/pay",
        )]),
        "the file and the constructed manifest are the same manifest"
    );
    assert!(
        matches!(loaded.scopes()[0].files_to_linear(), Err(NotLinear { .. })),
        "a Jira record files to Jira, which is the whole of what it records"
    );

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "no scheme added, no slash dropped and no key trimmed on the way through"
    );
}

#[test]
fn a_manifest_of_either_tracker_round_trips_through_its_own_text() {
    let manifest = Manifest::with_directories([engine(), tui()]).with_scopes([
        ScopeRecord::new("data-plane", " War ", "In Review", "area/data"),
        ScopeRecord::jira(
            "payments",
            "https://acme.atlassian.net/",
            "PAY",
            "Review ",
            "area/pay",
        ),
    ]);

    let text = manifest.to_toml_string().expect("serialises");
    assert_eq!(
        Manifest::from_toml_str(&text).expect("parses"),
        manifest,
        "both trackers survive a write and a read: {text}"
    );
}

// The constructed record, where the test below reads ones the file reader
// built: a constructor that folded a value on the way in would be a manifest
// written with bytes nobody typed, and no reader could tell afterwards.
#[test]
fn a_constructed_record_read_as_a_scope_carries_its_values_as_spelled() {
    let record = ScopeRecord::new("data-plane", " War ", "In Review", "area/Data_Plane");
    let scope = BoardScope::of(&record);

    assert_eq!(scope.name(), "data-plane");
    assert_eq!(scope.ticket_prefix(), " War ", "neither trimmed nor folded");
    assert_eq!(scope.review_state(), "In Review", "not folded");
    assert_eq!(scope.label(), "area/Data_Plane", "not folded either");
}

// Read off records the file reader built, and with the spacing and the trailing
// slash a hand-edited manifest is allowed to hold: the prefix a view hands over
// is the field as recorded, and a view that trimmed one would be reading a key
// nobody wrote.
#[test]
fn a_board_scope_carries_the_recorded_prefix_of_either_tracker() {
    let text = concat!(
        "version = 2\n\n",
        "[[scope]]\n",
        "name = \"data-plane\"\n",
        "tracker = \"linear\"\n",
        "team = \" War \"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/Data_Plane\"\n\n",
        "[[scope]]\n",
        "name = \"payments\"\n",
        "tracker = \"jira\"\n",
        "base_url = \"https://acme.atlassian.net/\"\n",
        "project_key = \" PAY \"\n",
        "review_state = \"Review \"\n",
        "label = \"area/pay\"\n",
    );
    let loaded = Manifest::from_toml_str(text).expect("parses");
    let [linear, jira] = loaded.scopes() else {
        panic!("the text records two scopes");
    };

    let linear = BoardScope::of(linear);
    assert_eq!(linear.name(), "data-plane");
    assert_eq!(
        linear.ticket_prefix(),
        " War ",
        "the team key of a Linear record, neither trimmed nor folded"
    );
    assert_eq!(linear.review_state(), "In Review");
    assert_eq!(linear.label(), "area/Data_Plane");

    let jira = BoardScope::of(jira);
    assert_eq!(jira.name(), "payments");
    assert_eq!(
        jira.ticket_prefix(),
        " PAY ",
        "the project key of a Jira record, and not the base URL it is a project on"
    );
    assert_eq!(jira.review_state(), "Review ", "not trimmed either");
    assert_eq!(jira.label(), "area/pay");
}

// Read off records the file reader built, because the TUI reads what the
// manifest holds and not what a constructor was handed: the word is the one
// the file spells, and a Jira record's two values arrive with the spacing and
// the trailing slash they were written with.
#[test]
fn a_loaded_record_says_which_board_it_files_to_in_the_word_the_file_spells() {
    let text = concat!(
        "version = 2\n\n",
        "[[scope]]\n",
        "name = \"data-plane\"\n",
        "tracker = \"linear\"\n",
        "team = \"WAR\"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/data\"\n\n",
        "[[scope]]\n",
        "name = \"payments\"\n",
        "tracker = \"jira\"\n",
        "base_url = \"https://acme.atlassian.net/\"\n",
        "project_key = \" PAY \"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/pay\"\n",
    );
    let loaded = Manifest::from_toml_str(text).expect("parses");
    let [linear, jira] = loaded.scopes() else {
        panic!("the text records two scopes");
    };

    assert_eq!(linear.filed_to(), FiledTo::Linear);
    assert_eq!(linear.filed_to().word(), "linear");
    assert_eq!(
        prefix_of(linear),
        "WAR",
        "and the team key is still the view's and not this enum's"
    );

    assert_eq!(jira.filed_to().word(), "jira");
    assert_eq!(
        jira.filed_to(),
        FiledTo::Jira {
            base_url: "https://acme.atlassian.net/",
            project_key: " PAY ",
        },
        "neither trimmed nor parsed on the way out"
    );
}

// The sentence a scope filing somewhere warlock cannot file to arrives with,
// asserted whole: every door to a board says this one and none of them words it
// itself.
#[test]
fn a_scope_that_files_elsewhere_is_refused_by_name_with_nothing_sent() {
    let record = ScopeRecord::jira(
        "payments",
        "https://acme.atlassian.net/",
        " PAY ",
        "In Review",
        "area/pay",
    );

    let refusal = record
        .files_to_linear()
        .expect_err("the record files to Jira");

    assert_eq!(
        refusal.to_string(),
        "`payments` files to Jira, and warlock can only file to Linear, so nothing was sent"
    );
    assert!(
        ScopeRecord::new("data-plane", "WAR", "In Review", "area/data")
            .files_to_linear()
            .is_ok(),
        "a record that files to Linear is not refused"
    );
}

#[test]
fn a_scope_a_directory_names_and_no_record_declares_loads() {
    let root = a_root();
    // A reader that treated a scope with no record as a hole would refuse a
    // manifest that is only one `[[scope]]` table short of routing.
    let original = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-tui\"\n",
        "scope = \"billing\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("a scope with no record is not a failure");
    assert_eq!(
        loaded
            .directories()
            .iter()
            .map(ScopedDirectory::scope)
            .collect::<Vec<_>>(),
        ["data-plane", "billing"],
        "both directories kept the scope they spelled"
    );
    assert!(
        loaded.scopes().is_empty(),
        "a scope that routes to nothing is nothing, not an invented record"
    );

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "and nothing was written down on the scopes' behalf"
    );
}

#[test]
fn a_record_no_directory_names_is_loaded_and_written_back_unchanged() {
    let root = a_root();
    // `third-party` is spelled by no directory here and is still where work
    // under it would be filed: a record is written down before anything is
    // scoped under it and outlives the last directory that named it, so
    // pruning it to what the rows happen to say would lose the only copy.
    let original = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n\n",
        "[[scope]]\n",
        "name = \"third-party\"\n",
        "tracker = \"linear\"\n",
        "team = \"Vendor\"\n",
        "review_state = \"Triage\"\n",
        "label = \"area/vendor\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded.scopes(),
        [ScopeRecord::new(
            "third-party",
            "Vendor",
            "Triage",
            "area/vendor"
        )],
        "the record nothing points at is read like any other"
    );
    assert!(
        loaded
            .directories()
            .iter()
            .all(|directory| directory.scope() != "third-party"),
        "no directory names it, which is the whole point of this fixture"
    );

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "an unused record survives a load and a save byte for byte"
    );
}

#[test]
fn a_manifest_with_no_records_writes_no_scope_table() {
    let with_none = Manifest::with_directories([engine(), tui()]);
    // Emptied rather than never set, which is the case `skip_serializing_if`
    // is there for: a manifest that lost its last record writes no key.
    let emptied = Manifest::with_directories([engine(), tui()])
        .with_scopes([ScopeRecord::new(
            "data-plane",
            "Data Plane",
            "In Review",
            "area",
        )])
        .with_scopes(Vec::new());

    for manifest in [&with_none, &emptied] {
        let text = manifest.to_toml_string().expect("serialises");
        assert_eq!(
            text,
            concat!(
                "version = 2\n\n",
                "[[directory]]\n",
                "path = \"crates/warlock-engine\"\n",
                "scope = \"data-plane\"\n\n",
                "[[directory]]\n",
                "path = \"crates/warlock-tui\"\n",
                "scope = \"billing\"\n",
            ),
        );
        assert!(manifest.scopes().is_empty());
        assert_eq!(Manifest::from_toml_str(&text).expect("parses"), with_none);
    }
}

#[test]
fn a_malformed_record_is_named_rather_than_read_as_absent() {
    // A missing key, a wrong-typed value and a key nothing declares. Each one
    // is the record being wrong, never the record being missing: a reader that
    // skipped it would route work under `data-plane` nowhere and say nothing.
    let linear = "[[scope]]\nname = \"data-plane\"\ntracker = \"linear\"\n";
    let bodies = [
        format!("{linear}team = \"Data Plane\"\nlabel = \"area\"\n"),
        format!("{linear}team = 7\nreview_state = \"In Review\"\nlabel = \"area\"\n"),
        format!(
            "{linear}team = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\nlead = \"someone\"\n"
        ),
    ];

    for body in bodies {
        let text = format!("version = 2\n\n{body}");
        match Manifest::from_toml_str(&text) {
            Err(Error::Scope {
                path: None,
                index: 0,
                name: Some(name),
                source:
                    ScopeFault::MissingKey { .. }
                    | ScopeFault::NotAString { .. }
                    | ScopeFault::UnexpectedKey { .. },
            }) => assert_eq!(name, "data-plane", "named by the name it did spell"),
            other => panic!("expected a malformed-record error for `{body}`, got {other:?}"),
        }

        let root = a_root();
        hand_write(root.path(), &text);
        let error = Manifest::load(root.path()).expect_err("the record is not readable");
        let message = error.to_string();
        assert!(message.contains("data-plane"), "{message}");
        assert!(
            message.contains(&manifest_path(root.path()).display().to_string()),
            "the message points at the file to go and hand-edit: {message}"
        );
    }
}

#[test]
fn a_record_is_refused_by_the_key_at_fault_and_the_tracker_it_declared() {
    // Every way one table can be wrong about its tracker, each answered by the
    // key or the value a person has to go and edit. The version-1 shape is the
    // first of them: a record with no `tracker` key is refused rather than read
    // as the Linear one it used to be.
    let cases: [(&str, &str); 7] = [
        (
            "name = \"data-plane\"\nteam = \"WAR\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `tracker` key is missing",
        ),
        (
            "name = \"data-plane\"\ntracker = \"github\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "`github` is not a tracker warlock knows; a record files to `linear` or `jira`",
        ),
        (
            "name = \"data-plane\"\ntracker = \"linear\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `team` key is missing",
        ),
        (
            "name = \"data-plane\"\ntracker = \"linear\"\nteam = \"WAR\"\nbase_url = \"https://acme.atlassian.net\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `base_url` key is not one a `linear` record holds",
        ),
        (
            "name = \"data-plane\"\ntracker = \"linear\"\nteam = \"WAR\"\nproject_key = \"PAY\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `project_key` key is not one a `linear` record holds",
        ),
        (
            "name = \"payments\"\ntracker = \"jira\"\nbase_url = \"https://acme.atlassian.net\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `project_key` key is missing",
        ),
        (
            "name = \"payments\"\ntracker = \"jira\"\nbase_url = \"https://acme.atlassian.net\"\nproject_key = \"PAY\"\nteam = \"WAR\"\nreview_state = \"In Review\"\nlabel = \"area\"\n",
            "the `team` key is not one a `jira` record holds",
        ),
    ];

    for (body, expected) in cases {
        let text = format!("version = 2\n\n[[scope]]\n{body}");
        match Manifest::from_toml_str(&text) {
            Err(error @ Error::Scope { index: 0, .. }) => {
                let message = error.to_string();
                assert!(
                    message.ends_with(expected),
                    "expected `{expected}`, got `{message}`"
                );
                assert!(
                    message.contains("scope record 0 (`"),
                    "the index and the name it spelled: {message}"
                );
            }
            other => panic!("expected `{body}` to be refused, got {other:?}"),
        }
    }
}

#[test]
fn a_tracker_word_names_a_board_or_is_refused_by_the_word_that_was_typed() {
    // The reader above and every other door read the word through this, so what
    // a word means is settled once: the two words it answers to, the spelling a
    // record serialises back to, and the sentence anything else is refused in.
    for (word, named) in [("linear", TrackerName::Linear), ("jira", TrackerName::Jira)] {
        assert_eq!(
            TrackerName::parse(word).expect("a word a record spells"),
            named
        );
        assert_eq!(named.word(), word);
    }

    for word in ["Linear", "JIRA", " jira", "github", ""] {
        match TrackerName::parse(word) {
            Err(fault @ ScopeFault::UnknownTracker { .. }) => assert_eq!(
                fault.to_string(),
                format!(
                    "`{word}` is not a tracker warlock knows; a record files to `linear` or `jira`"
                ),
            ),
            other => panic!("expected `{word}` to be refused, got {other:?}"),
        }
    }
}

#[test]
fn a_scope_that_is_not_a_table_is_refused_rather_than_skipped() {
    match Manifest::from_toml_str("version = 2\n\nscope = [7]\n") {
        Err(Error::Scope {
            index: 0,
            name: None,
            source: ScopeFault::NotATable,
            ..
        }) => {}
        other => panic!("expected a not-a-table refusal, got {other:?}"),
    }
}

#[test]
fn a_record_name_the_scope_rule_refuses_is_named_rather_than_skipped() {
    // The same rule `validate_scope` applies everywhere else, asked here and
    // nowhere restated. A record is the one place a bad scope is fatal: a
    // directory reads an unreadable scope as unscoped and widens to its
    // parent, but a record nobody can match is a routing table with a hole in
    // it.
    for name in ["", "Data-Plane", "data-", "*", "abcdefghijklmnopqrstuvwxy"] {
        let text = format!(
            "version = 2\n\n[[scope]]\nname = \"{name}\"\ntracker = \"linear\"\nteam = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\n"
        );
        match Manifest::from_toml_str(&text) {
            Err(Error::Scope {
                index: 0,
                name: Some(spelled),
                source: ScopeFault::Refused(_),
                ..
            }) => assert_eq!(spelled, name, "quoted back as written, not repaired"),
            other => panic!("expected `{name}` to be refused, got {other:?}"),
        }
    }
}

#[test]
fn two_records_with_the_same_name_are_a_duplicate_error() {
    let record = "tracker = \"linear\"\nteam = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\n";
    let text = format!(
        "version = 2\n\n[[scope]]\nname = \"data-plane\"\n{record}\n[[scope]]\nname = \"billing\"\n{record}\n[[scope]]\nname = \"data-plane\"\n{record}"
    );

    match Manifest::from_toml_str(&text) {
        Err(Error::DuplicateScope {
            path: None,
            index: 2,
            name,
        }) => assert_eq!(name, "data-plane"),
        other => panic!("expected a duplicate-record error, got {other:?}"),
    }

    let root = a_root();
    hand_write(root.path(), &text);
    let error = Manifest::load(root.path()).expect_err("one name, twice");
    assert!(
        matches!(error, Error::DuplicateScope { index: 2, .. }),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&manifest_path(root.path()).display().to_string()),
        "{error}"
    );
}

#[test]
fn a_manifest_with_two_directories_round_trips() {
    let manifest = Manifest::with_directories([engine(), tui()]);
    let text = manifest.to_toml_string().expect("serialises");
    assert_eq!(Manifest::from_toml_str(&text).expect("parses"), manifest);
    assert_eq!(manifest.directories().len(), 2);
    assert_eq!(manifest.version(), SCHEMA_VERSION);
}

#[test]
fn an_empty_manifest_is_just_the_version() {
    let text = Manifest::new().to_toml_string().expect("serialises");
    assert_eq!(text, "version = 2\n");
    assert_eq!(
        Manifest::from_toml_str(&text).expect("parses"),
        Manifest::default()
    );
}

#[test]
fn push_and_directory_lookup_use_stored_form() {
    let mut manifest = Manifest::new();
    manifest.push(engine());
    assert_eq!(
        manifest.directory("crates/warlock-engine"),
        Some(&engine()),
        "looked up by the path as stored"
    );
    assert!(manifest.directory("crates/warlock-tui").is_none());
}

#[test]
fn paths_are_stored_with_forward_slashes() {
    // Built with the platform separator, stored with slashes.
    let path = Path::new("crates").join("warlock-engine");
    let directory = ScopedDirectory::new(".", &path, "data-plane").expect("inside the root");

    assert_eq!(directory.path(), "crates/warlock-engine");

    let text = Manifest::with_directories([directory])
        .to_toml_string()
        .expect("serialises");
    assert!(text.contains("path = \"crates/warlock-engine\""), "{text}");
}

#[test]
fn the_same_content_under_two_roots_serialises_identically() {
    let under = |root: &str| {
        let root = PathBuf::from(root);
        Manifest::with_directories([
            ScopedDirectory::new(
                &root,
                root.join("crates").join("warlock-engine"),
                "data-plane",
            )
            .expect("inside the root"),
            ScopedDirectory::new(&root, &root, "whole-repo").expect("inside the root"),
        ])
        .to_toml_string()
        .expect("serialises")
    };

    assert_eq!(under("/tmp/one"), under("/home/someone/else/two"));
}

#[test]
fn absolute_paths_become_relative_and_relative_ones_are_left_alone() {
    assert_eq!(
        to_manifest_path("/repo", Path::new("/repo").join("docs")).expect("inside"),
        "docs"
    );
    assert_eq!(
        to_manifest_path("/repo", "docs/adr").expect("inside"),
        "docs/adr"
    );
    assert_eq!(
        to_manifest_path("/repo", "./docs/./adr").expect("inside"),
        "docs/adr"
    );
}

#[test]
fn the_root_itself_is_stored_as_a_dot() {
    assert_eq!(
        to_manifest_path("/repo", "/repo").expect("is the root"),
        "."
    );
    assert_eq!(to_manifest_path("/repo", "").expect("is the root"), ".");
    assert_eq!(from_manifest_path("/repo", "."), Path::new("/repo"));
}

#[test]
fn stored_paths_convert_back_to_paths_under_a_root() {
    let directory = engine();
    assert_eq!(
        directory.path_on_disk("/repo"),
        Path::new("/repo").join("crates").join("warlock-engine")
    );
    // And back again, unchanged.
    assert_eq!(
        to_manifest_path("/repo", directory.path_on_disk("/repo")).expect("inside"),
        directory.path()
    );
}

#[test]
fn a_path_outside_the_root_is_rejected() {
    assert!(matches!(
        to_manifest_path("/repo", "/elsewhere/docs"),
        Err(Error::PathOutsideRoot { .. })
    ));
    assert!(matches!(
        to_manifest_path("/repo", "../escape"),
        Err(Error::PathOutsideRoot { .. })
    ));
    assert!(matches!(
        ScopedDirectory::new("/repo", "/elsewhere", "data-plane"),
        Err(Error::PathOutsideRoot { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_non_utf8_path_is_rejected() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = Path::new(OsStr::from_bytes(b"crates/\xff"));
    assert!(matches!(
        to_manifest_path(".", path),
        Err(Error::NonUtf8Path { .. })
    ));
}

#[test]
fn an_unrecognised_version_is_rejected_before_the_directories_are_read() {
    let text = "version = 999\n\n[[directory]]\npath = \"x\"\nscope = \"data-plane\"\n";
    match Manifest::from_toml_str(text) {
        Err(Error::UnsupportedVersion { found, supported }) => {
            assert_eq!(found, 999);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("expected an unsupported-version error, got {other:?}"),
    }
    // The derived path refuses it too, rather than reading it as this version.
    assert!(toml::from_str::<Manifest>(text).is_err());
}

#[test]
fn a_version_one_manifest_is_refused_by_both_readers_naming_both_numbers() {
    // The shape before the tracker, which nothing reads and nothing rewrites:
    // this repository's own manifest was the one such file and was hand-edited
    // to version 2. The sentence names both numbers, because the fix is to
    // open the file and the person has to know what to write.
    let text = concat!(
        "version = 1\n\n",
        "[[scope]]\n",
        "name = \"data-plane\"\n",
        "team = \"WAR\"\n",
        "review_state = \"In Review\"\n",
        "label = \"area\"\n",
    );
    let sentence = "scope manifest schema version 1 is not supported; this build reads version 2";

    match Manifest::from_toml_str(text) {
        Err(error @ Error::UnsupportedVersion { found: 1, .. }) => {
            assert_eq!(error.to_string(), sentence);
        }
        other => panic!("expected an unsupported-version error, got {other:?}"),
    }

    // The derived path refuses the version too, through `deserialize_version`
    // and in the same words. It is asked here with nothing else in the file to
    // be wrong about, because serde reads the document's keys in its own order
    // and a record would be refused first.
    let derived =
        toml::from_str::<Manifest>("version = 1\n").expect_err("the derived path refuses it too");
    assert!(derived.to_string().contains(sentence), "{derived}");

    let root = a_root();
    hand_write(root.path(), text);
    match Manifest::load(root.path()) {
        Err(error @ Error::UnsupportedVersion { found: 1, .. }) => {
            assert_eq!(error.to_string(), sentence);
        }
        other => panic!("expected an unsupported-version error, got {other:?}"),
    }
}

#[test]
fn a_malformed_directory_names_itself() {
    let text = concat!(
        "version = 2\n\n",
        "[[directory]]\npath = \"crates/warlock-engine\"\nscope = \"data-plane\"\n\n",
        "[[directory]]\npath = \"crates/warlock-tui\"\nscope = 7\n",
    );
    match Manifest::from_toml_str(text) {
        Err(error @ Error::Directory { index: 1, .. }) => {
            assert!(error.to_string().contains("crates/warlock-tui"), "{error}");
        }
        other => panic!("expected a directory error, got {other:?}"),
    }
}

#[test]
fn text_that_is_not_a_manifest_is_a_syntax_error() {
    assert!(matches!(
        Manifest::from_toml_str("not toml"),
        Err(Error::Syntax { .. })
    ));
    assert!(matches!(
        Manifest::from_toml_str(""),
        Err(Error::Syntax { .. })
    ));
}

#[test]
fn every_error_variant_says_what_happened() {
    let cases = [
        (
            Error::NotFound {
                path: PathBuf::from("/repo/.warlock/scopes.toml"),
            },
            "no scope manifest at `/repo/.warlock/scopes.toml`",
        ),
        (
            Error::Io {
                path: PathBuf::from("/repo/.warlock/scopes.toml"),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            },
            "could not read or write `/repo/.warlock/scopes.toml`: denied",
        ),
        (
            Error::UnsupportedVersion {
                found: 999,
                supported: SCHEMA_VERSION,
            },
            "scope manifest schema version 999 is not supported; this build reads version 2",
        ),
        (
            Error::Scope {
                path: Some(PathBuf::from("/repo/.warlock/scopes.toml")),
                index: 1,
                name: Some("Data-Plane".to_owned()),
                source: ScopeFault::Refused(
                    crate::scope::validate_scope("Data-Plane").expect_err("a capital is refused"),
                ),
            },
            "scope record 1 (`Data-Plane`) in `/repo/.warlock/scopes.toml` is refused: a scope \
             holds only lowercase letters, digits, `-` and `_`, and this one holds `D`",
        ),
        (
            Error::DuplicateScope {
                path: Some(PathBuf::from("/repo/.warlock/scopes.toml")),
                index: 2,
                name: "data-plane".to_owned(),
            },
            "scope record 2 in `/repo/.warlock/scopes.toml` repeats the name `data-plane`",
        ),
        (
            // No file behind it: `from_toml_str` was handed text, so the
            // message says which record and stops rather than naming a path
            // that would send somebody to the wrong file.
            Error::DuplicateScope {
                path: None,
                index: 2,
                name: "data-plane".to_owned(),
            },
            "scope record 2 repeats the name `data-plane`",
        ),
        (
            Error::PathOutsideRoot {
                root: PathBuf::from("/repo"),
                path: PathBuf::from("/elsewhere"),
            },
            "`/elsewhere` is not inside the manifest root `/repo`",
        ),
        (
            Error::NonUtf8Path {
                path: PathBuf::from("/repo/odd"),
            },
            "`/repo/odd` is not valid UTF-8 and cannot be stored in the manifest",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }

    // The three that wrap a cause: the message names the directory or the
    // file and then quotes what the underlying crate said.
    let syntax = Error::Syntax {
        source: a_de_error(),
    };
    assert!(
        syntax.to_string().starts_with("malformed scope manifest: "),
        "{syntax}"
    );

    let named = Error::Directory {
        index: 1,
        path: Some("crates/warlock-tui".to_owned()),
        source: a_de_error(),
    };
    assert!(
        named
            .to_string()
            .starts_with("scoped directory 1 (`crates/warlock-tui`) is malformed: "),
        "{named}"
    );

    let unnamed = Error::Directory {
        index: 0,
        path: None,
        source: a_de_error(),
    };
    assert!(
        unnamed
            .to_string()
            .starts_with("scoped directory 0 is malformed: "),
        "{unnamed}"
    );
}

#[test]
fn errors_expose_the_cause_they_wrap() {
    use std::error::Error as _;

    assert!(
        Error::Syntax {
            source: a_de_error()
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Directory {
            index: 0,
            path: None,
            source: a_de_error(),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Io {
            path: PathBuf::from("x"),
            source: std::io::Error::other("boom"),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Scope {
            path: None,
            index: 0,
            name: None,
            source: ScopeFault::Malformed(Box::new(a_de_error())),
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
        Error::DuplicateScope {
            path: None,
            index: 0,
            name: "data-plane".to_owned(),
        }
        .source()
        .is_none()
    );
}

#[test]
fn saving_creates_the_directory_and_leaves_no_temporary_file_behind() {
    let root = a_root();
    assert!(!root.path().join(".warlock").exists(), "nothing there yet");

    Manifest::with_directories([engine(), tui()])
        .save(root.path())
        .expect("saves");

    // Exactly one file: the temporary the save wrote through has been
    // renamed away, not left lying next to the manifest.
    assert_eq!(warlock_dir_listing(root.path()), [MANIFEST_FILE]);
    assert!(manifest_path(root.path()).is_file());
}

#[test]
fn saving_again_replaces_the_manifest_and_still_leaves_one_file() {
    let root = a_root();
    Manifest::with_directories([engine()])
        .save(root.path())
        .expect("saves");

    let replacement = Manifest::with_directories([tui()]);
    replacement.save(root.path()).expect("saves over");

    assert_eq!(warlock_dir_listing(root.path()), [MANIFEST_FILE]);
    assert_eq!(Manifest::load(root.path()).expect("loads"), replacement);
}

#[test]
fn the_same_manifest_saved_under_two_roots_gives_byte_identical_files() {
    let saved_under = |root: &Path| {
        let manifest = Manifest::with_directories([
            ScopedDirectory::new(
                root,
                root.join("crates").join("warlock-engine"),
                "data-plane",
            )
            .expect("inside the root"),
            ScopedDirectory::new(root, root, "whole-repo").expect("inside the root"),
        ]);
        manifest.save(root).expect("saves");
        fs::read(manifest_path(root)).expect("reads the file back")
    };

    let (one, two) = (a_root(), a_root());
    assert_ne!(one.path(), two.path(), "two different absolute roots");
    assert_eq!(saved_under(one.path()), saved_under(two.path()));
}

#[test]
fn loading_what_was_saved_gives_the_manifest_back() {
    let root = a_root();
    let manifest = Manifest::with_directories([engine(), tui()]).with_scopes([ScopeRecord::new(
        "data-plane",
        "WAR",
        "In Review",
        "area/data-plane",
    )]);
    manifest.save(root.path()).expect("saves");

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(loaded, manifest);
    assert_eq!(loaded.directories()[0].scope(), "data-plane");
    assert_eq!(loaded.directories()[1].scope(), "billing");
}

#[test]
fn saving_what_was_loaded_gives_the_file_back_byte_for_byte() {
    let root = a_root();
    // Hand-written rather than produced by a save, so this is a real
    // statement about the file format and not about the serialiser
    // agreeing with itself.
    let original = concat!(
        "version = 2\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-engine\"\n",
        "scope = \"data-plane\"\n\n",
        "[[directory]]\n",
        "path = \"crates/warlock-tui\"\n",
        "scope = \"billing\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    loaded.save(root.path()).expect("saves");

    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "a load-then-save is a no-op on the bytes, so it does not churn the diff"
    );
    assert_eq!(Manifest::load(root.path()).expect("reloads"), loaded);
}

#[test]
fn loading_a_manifest_from_a_future_schema_is_a_version_error() {
    let root = a_root();
    hand_write(
        root.path(),
        "version = 999\n\n[[directory]]\npath = \"x\"\nscope = \"data-plane\"\n",
    );

    match Manifest::load(root.path()) {
        Err(Error::UnsupportedVersion { found, supported }) => {
            assert_eq!(found, 999);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("expected an unsupported-version error, got {other:?}"),
    }
}

#[test]
fn loading_a_manifest_with_a_bad_directory_names_that_directory() {
    let root = a_root();
    hand_write(
        root.path(),
        concat!(
            "version = 2\n\n",
            "[[directory]]\npath = \"crates/warlock-engine\"\nscope = \"data-plane\"\n\n",
            "[[directory]]\npath = \"crates/warlock-tui\"\nscope = 7\n",
        ),
    );

    let error = Manifest::load(root.path()).expect_err("a number is not a scope");
    assert!(
        matches!(error, Error::Directory { index: 1, .. }),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("crates/warlock-tui"),
        "the message points at the directory to go and hand-edit: {error}"
    );
}

#[test]
fn loading_a_manifest_that_is_not_there_is_not_found() {
    let root = a_root();

    // Documented behaviour: absent is `NotFound`, not an empty manifest,
    // so a caller can tell "never scoped" from "scoped nothing".
    match Manifest::load(root.path()) {
        Err(Error::NotFound { path }) => assert_eq!(path, manifest_path(root.path())),
        other => panic!("expected a not-found error, got {other:?}"),
    }

    // And a directory with no `.warlock` at all is the same answer, not an
    // I/O error about the missing parent.
    assert!(matches!(
        Manifest::load(root.path().join("nowhere")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn a_missing_manifest_is_distinguishable_from_a_corrupt_one() {
    let (missing, corrupt) = (a_root(), a_root());
    hand_write(corrupt.path(), "this is not a manifest\n");

    assert!(matches!(
        Manifest::load(missing.path()),
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        Manifest::load(corrupt.path()),
        Err(Error::Syntax { .. })
    ));
}
