use super::{Error, FILED_FILE, Filed, FiledRecord, SCHEMA_VERSION, filed_path};

fn a_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn filed_brief() -> FiledRecord {
    FiledRecord::new(
        ".",
        "docs/warlock-brief-22.md",
        "b229262b-22aa-444a-a8af-0a2a3f4ef100",
        "https://linear.app/acme/project/warlock-brief-22",
        "warlock-team",
        "WAR",
        "2026-09-20T07:32:00Z",
    )
    .expect("a relative path inside the root is storable")
}

#[test]
fn a_saved_record_loads_back_as_it_went_in() {
    let root = a_root();
    let filed = Filed::with_records([filed_brief()]);

    filed.save(root.path()).expect("saves");

    assert_eq!(Filed::load(root.path()).expect("loads"), filed);
    assert_eq!(filed.version(), SCHEMA_VERSION);
    assert!(filed_path(root.path()).ends_with(FILED_FILE));
}

#[test]
fn another_version_is_refused_on_both_reading_paths() {
    let text = "version = 2\n";

    assert!(matches!(
        Filed::from_toml_str(text),
        Err(Error::UnsupportedVersion {
            found: 2,
            supported: SCHEMA_VERSION,
            ..
        })
    ));
    assert!(
        toml::from_str::<Filed>(text).is_err(),
        "the derived path refuses what the module's reader refuses"
    );
}

#[test]
fn a_missing_file_is_not_an_empty_set_of_records() {
    let root = a_root();

    let refusal = Filed::load(root.path()).expect_err("nothing has been filed here");

    assert!(
        matches!(&refusal, Error::NotFound { path } if path == &filed_path(root.path())),
        "{refusal}"
    );
}
