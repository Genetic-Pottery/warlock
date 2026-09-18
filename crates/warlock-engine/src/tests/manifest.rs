use std::fs;
use std::path::{Path, PathBuf};

use super::{
    Error, MANIFEST_FILE, Manifest, PactEntry, SCHEMA_VERSION, ScopeFault, ScopeRecord,
    from_manifest_path, manifest_path, to_manifest_path,
};

fn unjudged() -> PactEntry {
    PactEntry::new(
        ".",
        "crates/warlock-engine",
        "crates/warlock-engine/WARLOCK.md",
    )
    .expect("a relative path inside the root is storable")
}

fn judged() -> PactEntry {
    unjudged().with_grant("d0f5a1", "2026-08-19T07:32:00Z")
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

fn hand_write(root: &Path, text: &str) {
    let path = manifest_path(root);
    fs::create_dir_all(path.parent().expect("the manifest has a directory"))
        .expect("creates .warlock");
    fs::write(&path, text).expect("writes the manifest");
}

#[test]
fn an_unjudged_entry_omits_both_grant_keys() {
    let manifest = Manifest::with_entries([unjudged()]);
    let text = manifest.to_toml_string().expect("serialises");

    assert!(!text.contains("granted_hash"), "{text}");
    assert!(!text.contains("granted_at"), "{text}");

    let parsed = Manifest::from_toml_str(&text).expect("parses");
    assert_eq!(parsed, manifest);
    assert_eq!(parsed.entries()[0].granted_hash(), None);
    assert_eq!(parsed.entries()[0].granted_at(), None);
    assert!(!parsed.entries()[0].is_judged());
    // The round trip re-emits the file without the keys: no sentinel value
    // has crept in on the way through.
    assert_eq!(parsed.to_toml_string().expect("re-serialises"), text);
}

#[test]
fn a_judged_entry_carries_both_grant_keys() {
    let manifest = Manifest::with_entries([judged()]);
    let text = manifest.to_toml_string().expect("serialises");

    assert!(text.contains("granted_hash = \"d0f5a1\""), "{text}");
    assert!(
        text.contains("granted_at = \"2026-08-19T07:32:00Z\""),
        "{text}"
    );

    let parsed = Manifest::from_toml_str(&text).expect("parses");
    assert_eq!(parsed, manifest);
    assert_eq!(parsed.entries()[0].granted_hash(), Some("d0f5a1"));
    assert_eq!(
        parsed.entries()[0].granted_at(),
        Some("2026-08-19T07:32:00Z")
    );
    assert!(parsed.entries()[0].is_judged());
    assert_eq!(parsed.to_toml_string().expect("re-serialises"), text);
}

#[test]
fn an_unscoped_entry_omits_the_scope_key() {
    let manifest = Manifest::with_entries([unjudged(), judged()]);
    let text = manifest.to_toml_string().expect("serialises");

    assert!(!text.contains("scope"), "{text}");
    assert_eq!(
        text,
        concat!(
            "version = 1\n\n",
            "[[pact]]\n",
            "module = \"crates/warlock-engine\"\n",
            "document = \"crates/warlock-engine/WARLOCK.md\"\n\n",
            "[[pact]]\n",
            "module = \"crates/warlock-engine\"\n",
            "document = \"crates/warlock-engine/WARLOCK.md\"\n",
            "granted_hash = \"d0f5a1\"\n",
            "granted_at = \"2026-08-19T07:32:00Z\"\n",
        ),
        "an entry with no scope serialises to exactly the bytes it did \
             before the field existed"
    );

    let parsed = Manifest::from_toml_str(&text).expect("parses");
    assert_eq!(parsed, manifest);
    assert_eq!(parsed.entries()[0].scope(), None);
    assert_eq!(parsed.entries()[1].scope(), None);
}

#[test]
fn a_manifest_written_before_scopes_existed_loads_unscoped() {
    // Byte for byte the shape a previous build wrote: no `scope` key
    // anywhere, and no version bump to say the schema moved.
    let root = a_root();
    let original = concat!(
        "version = 1\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-engine\"\n",
        "document = \"crates/warlock-engine/WARLOCK.md\"\n",
        "granted_hash = \"d0f5a1\"\n",
        "granted_at = \"2026-08-19T07:32:00Z\"\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-tui\"\n",
        "document = \"crates/warlock-tui/WARLOCK.md\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads without a scope key in sight");
    assert_eq!(loaded.version(), SCHEMA_VERSION);
    assert!(
        loaded.entries().iter().all(|entry| entry.scope().is_none()),
        "every entry reads as unscoped"
    );

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "and saving it back does not add the new key"
    );
}

#[test]
fn a_scoped_entry_round_trips_byte_for_byte() {
    let root = a_root();
    // Hand-written, so this says where the key sits in the file and not
    // merely that the serialiser agrees with itself.
    let original = concat!(
        "version = 1\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-engine\"\n",
        "document = \"crates/warlock-engine/WARLOCK.md\"\n",
        "scope = \"data-plane\"\n",
        "granted_hash = \"d0f5a1\"\n",
        "granted_at = \"2026-08-19T07:32:00Z\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded,
        Manifest::with_entries([judged().with_scope("data-plane")]),
        "the file and the constructed entry are the same manifest"
    );
    assert_eq!(loaded.entries()[0].scope(), Some("data-plane"));

    loaded.save(root.path()).expect("saves");
    assert_eq!(
        fs::read_to_string(manifest_path(root.path())).expect("reads"),
        original,
        "a load-then-save is a no-op on the bytes of a scoped entry too"
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
                "version = 1\n\n",
                "[[pact]]\n",
                "module = \"crates/warlock-engine\"\n",
                "document = \"crates/warlock-engine/WARLOCK.md\"\n",
                "scope = \"{}\"\n",
                "granted_hash = \"d0f5a1\"\n",
                "granted_at = \"2026-08-19T07:32:00Z\"\n",
            ),
            scope
        );
        hand_write(root.path(), &original);

        let loaded = Manifest::load(root.path())
            .unwrap_or_else(|error| panic!("`{scope}` is not fatal, got {error:?}"));
        let entry = &loaded.entries()[0];
        assert_eq!(entry.module(), "crates/warlock-engine", "for `{scope}`");
        assert_eq!(
            entry.document(),
            "crates/warlock-engine/WARLOCK.md",
            "for `{scope}`"
        );
        assert_eq!(entry.granted_hash(), Some("d0f5a1"), "for `{scope}`");
        assert_eq!(
            entry.granted_at(),
            Some("2026-08-19T07:32:00Z"),
            "for `{scope}`"
        );
        assert_eq!(
            entry.scope(),
            Some(scope),
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
fn setting_or_clearing_a_scope_leaves_the_rest_of_the_entry_alone() {
    let before = judged();
    let scoped = before.clone().with_scope("data-plane");
    let rescoped = scoped.clone().with_scope("billing");
    let unscoped = rescoped.clone().without_scope();

    for entry in [&scoped, &rescoped, &unscoped] {
        assert_eq!(entry.module(), before.module());
        assert_eq!(entry.document(), before.document());
        assert_eq!(entry.granted_hash(), before.granted_hash());
        assert_eq!(entry.granted_at(), before.granted_at());
        assert!(entry.is_judged());
    }

    assert_eq!(scoped.scope(), Some("data-plane"));
    assert_eq!(rescoped.scope(), Some("billing"), "one scope, replaced");
    assert_eq!(unscoped.scope(), None);
    // Clearing a scope that was never set is the entry it started as.
    assert_eq!(unscoped, before);
    assert_eq!(before.clone().without_scope(), before);
}

#[test]
fn a_scope_is_stored_as_written_however_odd() {
    // No folding, no trimming, no rejection: `with_scope` is the store, not
    // the gate. Two spellings staying two spellings is exactly why the
    // folding belongs where a person types.
    for scope in ["Data-Plane", " data-plane ", "*", ""] {
        assert_eq!(judged().with_scope(scope).scope(), Some(scope));
    }
}

#[test]
fn a_scope_record_round_trips_byte_for_byte() {
    let root = a_root();
    // Hand-written, so this says where the table sits in the file and in what
    // order its keys are written, not merely that the serialiser agrees with
    // itself.
    let original = concat!(
        "version = 1\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-engine\"\n",
        "document = \"crates/warlock-engine/WARLOCK.md\"\n",
        "scope = \"data-plane\"\n\n",
        "[[scope]]\n",
        "name = \"data-plane\"\n",
        "team = \"Data Plane \"\n",
        "review_state = \"In Review\"\n",
        "label = \"area/Data_Plane\"\n",
    );
    hand_write(root.path(), original);

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(
        loaded,
        Manifest::with_entries([unjudged().with_scope("data-plane")]).with_scopes([
            ScopeRecord::new("data-plane", "Data Plane ", "In Review", "area/Data_Plane")
        ]),
        "the file and the constructed manifest are the same manifest"
    );

    let record = &loaded.scopes()[0];
    assert_eq!(record.name(), "data-plane");
    assert_eq!(record.team(), "Data Plane ", "not trimmed");
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
fn a_manifest_with_no_records_writes_the_bytes_it_always_did() {
    let with_none = Manifest::with_entries([unjudged(), judged()]);
    // Emptied rather than never set, which is the case `skip_serializing_if`
    // is there for: a manifest that lost its last record writes no key.
    let emptied = Manifest::with_entries([unjudged(), judged()])
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
                "version = 1\n\n",
                "[[pact]]\n",
                "module = \"crates/warlock-engine\"\n",
                "document = \"crates/warlock-engine/WARLOCK.md\"\n\n",
                "[[pact]]\n",
                "module = \"crates/warlock-engine\"\n",
                "document = \"crates/warlock-engine/WARLOCK.md\"\n",
                "granted_hash = \"d0f5a1\"\n",
                "granted_at = \"2026-08-19T07:32:00Z\"\n",
            ),
            "a manifest with no records is byte-identical to one written before \
             records existed"
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
    let bodies = [
        "[[scope]]\nname = \"data-plane\"\nteam = \"Data Plane\"\nlabel = \"area\"\n",
        "[[scope]]\nname = \"data-plane\"\nteam = 7\nreview_state = \"In Review\"\nlabel = \"area\"\n",
        "[[scope]]\nname = \"data-plane\"\nteam = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\nlead = \"someone\"\n",
    ];

    for body in bodies {
        let text = format!("version = 1\n\n{body}");
        match Manifest::from_toml_str(&text) {
            Err(Error::Scope {
                path: None,
                index: 0,
                name: Some(name),
                source: ScopeFault::Malformed(_),
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
fn a_record_name_the_scope_rule_refuses_is_named_rather_than_skipped() {
    // The same rule `validate_scope` applies everywhere else, asked here and
    // nowhere restated. A record is the one place a bad scope is fatal: a pact
    // reads an unreadable scope as unscoped and widens to its parent, but a
    // record nobody can match is a routing table with a hole in it.
    for name in ["", "Data-Plane", "data-", "*", "abcdefghijklmnopqrstuvwxy"] {
        let text = format!(
            "version = 1\n\n[[scope]]\nname = \"{name}\"\nteam = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\n"
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
    let record = "team = \"Data Plane\"\nreview_state = \"In Review\"\nlabel = \"area\"\n";
    let text = format!(
        "version = 1\n\n[[scope]]\nname = \"data-plane\"\n{record}\n[[scope]]\nname = \"billing\"\n{record}\n[[scope]]\nname = \"data-plane\"\n{record}"
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
fn dropping_a_grant_drops_both_keys_again() {
    let text = Manifest::with_entries([judged().without_grant()])
        .to_toml_string()
        .expect("serialises");
    assert!(!text.contains("granted"), "{text}");
}

#[test]
fn a_manifest_with_one_judged_and_one_unjudged_entry_round_trips() {
    let manifest = Manifest::with_entries([judged(), unjudged()]);
    let text = manifest.to_toml_string().expect("serialises");
    assert_eq!(Manifest::from_toml_str(&text).expect("parses"), manifest);
    assert_eq!(manifest.entries().len(), 2);
    assert_eq!(manifest.version(), SCHEMA_VERSION);
}

#[test]
fn an_empty_manifest_is_just_the_version() {
    let text = Manifest::new().to_toml_string().expect("serialises");
    assert_eq!(text, "version = 1\n");
    assert_eq!(
        Manifest::from_toml_str(&text).expect("parses"),
        Manifest::default()
    );
}

#[test]
fn push_and_entry_lookup_use_stored_form() {
    let mut manifest = Manifest::new();
    manifest.push(unjudged());
    assert_eq!(
        manifest.entry("crates/warlock-engine"),
        Some(&unjudged()),
        "looked up by the path as stored"
    );
    assert!(manifest.entry("crates/warlock-tui").is_none());
}

#[test]
fn paths_are_stored_with_forward_slashes() {
    // Built with the platform separator, stored with slashes.
    let module = Path::new("crates").join("warlock-engine");
    let document = module.join("WARLOCK.md");
    let entry = PactEntry::new(".", &module, &document).expect("inside the root");

    assert_eq!(entry.module(), "crates/warlock-engine");
    assert_eq!(entry.document(), "crates/warlock-engine/WARLOCK.md");

    let text = Manifest::with_entries([entry])
        .to_toml_string()
        .expect("serialises");
    assert!(
        text.contains("module = \"crates/warlock-engine\""),
        "{text}"
    );
}

#[test]
fn the_same_content_under_two_roots_serialises_identically() {
    let under = |root: &str| {
        let root = PathBuf::from(root);
        let module = root.join("crates").join("warlock-engine");
        let document = module.join("WARLOCK.md");
        Manifest::with_entries([
            PactEntry::new(&root, &module, &document)
                .expect("inside the root")
                .with_grant("d0f5a1", "2026-08-19T07:32:00Z"),
            PactEntry::new(&root, &root, root.join("WARLOCK.md")).expect("inside the root"),
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
    let entry = unjudged();
    assert_eq!(
        entry.module_path("/repo"),
        Path::new("/repo").join("crates").join("warlock-engine")
    );
    assert_eq!(
        entry.document_path("/repo"),
        Path::new("/repo")
            .join("crates")
            .join("warlock-engine")
            .join("WARLOCK.md")
    );
    // And back again, unchanged.
    assert_eq!(
        to_manifest_path("/repo", entry.module_path("/repo")).expect("inside"),
        entry.module()
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
        PactEntry::new("/repo", "/elsewhere", "/elsewhere/WARLOCK.md"),
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
fn an_unrecognised_version_is_rejected_before_the_entries_are_read() {
    let text = "version = 999\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\n";
    match Manifest::from_toml_str(text) {
        Err(Error::UnsupportedVersion { found, supported }) => {
            assert_eq!(found, 999);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("expected an unsupported-version error, got {other:?}"),
    }
    // The derived path refuses it too, rather than reading it as version 1.
    assert!(toml::from_str::<Manifest>(text).is_err());
}

#[test]
fn a_malformed_entry_names_itself() {
    let text = concat!(
        "version = 1\n\n",
        "[[pact]]\nmodule = \"crates/warlock-engine\"\ndocument = \"crates/warlock-engine/WARLOCK.md\"\n\n",
        "[[pact]]\nmodule = \"crates/warlock-tui\"\ndocument = 7\n",
    );
    match Manifest::from_toml_str(text) {
        Err(error @ Error::Entry { index: 1, .. }) => {
            assert!(error.to_string().contains("crates/warlock-tui"), "{error}");
        }
        other => panic!("expected an entry error, got {other:?}"),
    }
}

#[test]
fn an_entry_with_an_unknown_key_is_an_error_too() {
    let text =
        "version = 1\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\nfresh = true\n";
    assert!(matches!(
        Manifest::from_toml_str(text),
        Err(Error::Entry { .. })
    ));
}

#[test]
fn the_old_readme_key_is_an_error_rather_than_an_alias() {
    // The document key was called `readme` before Warlock's artifact was
    // named. There are no manifests in the wild carrying it, so it is an
    // unknown key like any other — no alias, no migration, no quiet
    // acceptance that would let two spellings of one field coexist.
    let text = "version = 1\n\n[[pact]]\nmodule = \"x\"\nreadme = \"x/WARLOCK.md\"\n";
    assert!(matches!(
        Manifest::from_toml_str(text),
        Err(Error::Entry { .. })
    ));

    // And the key that is written is the new one.
    let written = Manifest::with_entries([unjudged()])
        .to_toml_string()
        .expect("serialises");
    assert!(
        written.contains("document = \"crates/warlock-engine/WARLOCK.md\""),
        "{written}"
    );
    assert!(!written.contains("readme"), "{written}");
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
                path: PathBuf::from("/repo/.warlock/pacts.toml"),
            },
            "no pact manifest at `/repo/.warlock/pacts.toml`",
        ),
        (
            Error::Io {
                path: PathBuf::from("/repo/.warlock/pacts.toml"),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            },
            "could not read or write `/repo/.warlock/pacts.toml`: denied",
        ),
        (
            Error::UnsupportedVersion {
                found: 999,
                supported: SCHEMA_VERSION,
            },
            "pact manifest schema version 999 is not supported; this build reads version 1",
        ),
        (
            Error::Scope {
                path: Some(PathBuf::from("/repo/.warlock/pacts.toml")),
                index: 1,
                name: Some("Data-Plane".to_owned()),
                source: ScopeFault::Refused(
                    crate::scope::validate_scope("Data-Plane").expect_err("a capital is refused"),
                ),
            },
            "scope record 1 (`Data-Plane`) in `/repo/.warlock/pacts.toml` is refused: a scope \
             holds only lowercase letters, digits, `-` and `_`, and this one holds `D`",
        ),
        (
            Error::DuplicateScope {
                path: Some(PathBuf::from("/repo/.warlock/pacts.toml")),
                index: 2,
                name: "data-plane".to_owned(),
            },
            "scope record 2 in `/repo/.warlock/pacts.toml` repeats the name `data-plane`",
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

    // The three that wrap a cause: the message names the entry or the file
    // and then quotes what the underlying crate said.
    let syntax = Error::Syntax {
        source: a_de_error(),
    };
    assert!(
        syntax.to_string().starts_with("malformed pact manifest: "),
        "{syntax}"
    );

    let named = Error::Entry {
        index: 1,
        module: Some("crates/warlock-tui".to_owned()),
        source: a_de_error(),
    };
    assert!(
        named
            .to_string()
            .starts_with("pact entry 1 (`crates/warlock-tui`) is malformed: "),
        "{named}"
    );

    let unnamed = Error::Entry {
        index: 0,
        module: None,
        source: a_de_error(),
    };
    assert!(
        unnamed
            .to_string()
            .starts_with("pact entry 0 is malformed: "),
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
        Error::Entry {
            index: 0,
            module: None,
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

    Manifest::with_entries([judged(), unjudged()])
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
    Manifest::with_entries([judged()])
        .save(root.path())
        .expect("saves");

    let replacement = Manifest::with_entries([unjudged()]);
    replacement.save(root.path()).expect("saves over");

    assert_eq!(warlock_dir_listing(root.path()), [MANIFEST_FILE]);
    assert_eq!(Manifest::load(root.path()).expect("loads"), replacement);
}

#[test]
fn the_same_manifest_saved_under_two_roots_gives_byte_identical_files() {
    let saved_under = |root: &Path| {
        let module = root.join("crates").join("warlock-engine");
        let manifest = Manifest::with_entries([
            PactEntry::new(root, &module, module.join("WARLOCK.md"))
                .expect("inside the root")
                .with_grant("d0f5a1", "2026-08-19T07:32:00Z"),
            PactEntry::new(root, root, root.join("WARLOCK.md")).expect("inside the root"),
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
    let manifest = Manifest::with_entries([judged(), unjudged()]);
    manifest.save(root.path()).expect("saves");

    let loaded = Manifest::load(root.path()).expect("loads");
    assert_eq!(loaded, manifest);
    assert_eq!(loaded.entries()[0].granted_hash(), Some("d0f5a1"));
    assert_eq!(loaded.entries()[1].granted_hash(), None);
}

#[test]
fn saving_what_was_loaded_gives_the_file_back_byte_for_byte() {
    let root = a_root();
    // Hand-written rather than produced by a save, so this is a real
    // statement about the file format and not about the serialiser
    // agreeing with itself.
    let original = concat!(
        "version = 1\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-engine\"\n",
        "document = \"crates/warlock-engine/WARLOCK.md\"\n",
        "granted_hash = \"d0f5a1\"\n",
        "granted_at = \"2026-08-19T07:32:00Z\"\n\n",
        "[[pact]]\n",
        "module = \"crates/warlock-tui\"\n",
        "document = \"crates/warlock-tui/WARLOCK.md\"\n",
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
        "version = 999\n\n[[pact]]\nmodule = \"x\"\ndocument = \"x/WARLOCK.md\"\n",
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
fn loading_a_manifest_with_a_bad_entry_names_that_entry() {
    let root = a_root();
    hand_write(
        root.path(),
        concat!(
            "version = 1\n\n",
            "[[pact]]\nmodule = \"crates/warlock-engine\"\ndocument = \"crates/warlock-engine/WARLOCK.md\"\n\n",
            "[[pact]]\nmodule = \"crates/warlock-tui\"\ndocument = 7\n",
        ),
    );

    let error = Manifest::load(root.path()).expect_err("a number is not a path");
    assert!(matches!(error, Error::Entry { index: 1, .. }), "{error:?}");
    assert!(
        error.to_string().contains("crates/warlock-tui"),
        "the message points at the entry to go and hand-edit: {error}"
    );
}

#[test]
fn loading_a_manifest_that_is_not_there_is_not_found() {
    let root = a_root();

    // Documented behaviour: absent is `NotFound`, not an empty manifest,
    // so a caller can tell "never pacted" from "pacted nothing".
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
