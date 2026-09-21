use std::fs;
use std::path::{Path, PathBuf};

use super::{
    CutRecord, Error, FILED_FILE, Filed, FiledRecord, SCHEMA_VERSION, filed_path, fold_title,
};
use crate::hash::subtree_hash;

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

fn a_de_error() -> toml::de::Error {
    toml::from_str::<Filed>("version = \"one\"").expect_err("a string is not an integer")
}

fn a_ser_error() -> toml::ser::Error {
    toml::to_string(&42).expect_err("an integer is not a TOML document")
}

fn hand_write(root: &Path, text: &str) {
    let path = filed_path(root);
    fs::create_dir_all(path.parent().expect("the file has a directory")).expect("creates .warlock");
    fs::write(&path, text).expect("writes the filed records");
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
fn an_empty_set_of_records_is_just_the_version() {
    let text = Filed::new().to_toml_string().expect("serialises");

    assert_eq!(text, "version = 2\n");
    assert_eq!(
        Filed::from_toml_str(&text).expect("parses"),
        Filed::default()
    );
}

#[test]
fn a_version_1_file_loads_as_the_records_it_spells() {
    let root = a_root();
    hand_write(
        root.path(),
        concat!(
            "version = 1\n\n",
            "[[filed]]\n",
            "path = \"docs/warlock-brief-21.md\"\n",
            "project_id = \"legacy-id\"\n",
            "url = \"https://linear.app/acme/project/warlock-brief-21\"\n",
            "scope = \" Data Plane \"\n",
            "team = \"dp\"\n",
            "filed_at = \"2026-09-19T09:00:00+01:00\"\n",
        ),
    );

    let loaded = Filed::load(root.path()).expect("a version 1 file is read, not refused");

    assert_eq!(
        loaded.version(),
        SCHEMA_VERSION,
        "read as this schema, so the next save writes this schema"
    );
    let record = loaded
        .record("docs/warlock-brief-21.md")
        .expect("the record is found by the path it spells");
    assert_eq!(record.project_id(), "legacy-id");
    assert_eq!(
        record.url(),
        "https://linear.app/acme/project/warlock-brief-21"
    );
    assert_eq!(record.scope(), " Data Plane ");
    assert_eq!(record.team(), "dp");
    assert_eq!(record.filed_at(), "2026-09-19T09:00:00+01:00");
    assert!(
        record.cuts().is_empty(),
        "a brief filed before there were cut records is a brief with nothing cut"
    );

    assert_eq!(
        Filed::from_toml_str("version = 1\n").expect("and the text path reads it too"),
        Filed::new(),
    );
    assert!(
        toml::from_str::<Filed>("version = 1\n").is_ok(),
        "as does the derived path"
    );
}

#[test]
fn a_version_2_file_with_cut_records_round_trips() {
    let root = a_root();
    let mut brief = filed_brief();
    brief.push_cut(CutRecord::new(
        "  The   Cut Record  ",
        ["WAR-125", "WAR-126"],
        "2026-09-21T09:00:00Z",
    ));
    brief.push_cut(CutRecord::new(
        "`warlock pull`",
        ["WAR-127"],
        "2026-09-21T09:05:00Z",
    ));
    // A second brief with nothing cut, so the file proves both widths of
    // record read back as they went in.
    let filed = Filed::with_records([
        brief,
        FiledRecord::new(
            ".",
            "docs/warlock-brief-21.md",
            "legacy-id",
            "https://linear.app/acme/project/warlock-brief-21",
            "data-plane",
            "DP",
            "2026-09-19T09:00:00+01:00",
        )
        .expect("inside the root"),
    ]);

    filed.save(root.path()).expect("saves");

    let text = fs::read_to_string(filed_path(root.path())).expect("reads");
    assert!(text.starts_with("version = 2\n"), "{text}");
    assert!(
        !text.contains("cut = []"),
        "a record with nothing cut gains no empty array: {text}"
    );

    let loaded = Filed::load(root.path()).expect("loads");
    assert_eq!(loaded, filed);

    let cuts = loaded
        .record("docs/warlock-brief-22.md")
        .expect("the brief is filed")
        .cuts();
    assert_eq!(cuts.len(), 2);
    assert_eq!(cuts[0].title(), "  The   Cut Record  ", "as spelled");
    assert_eq!(cuts[0].key(), "the cut record");
    assert_eq!(cuts[0].issues(), ["WAR-125", "WAR-126"]);
    assert_eq!(cuts[0].cut_at(), "2026-09-21T09:00:00Z");
    assert_eq!(cuts[1].title(), "`warlock pull`");
    assert_eq!(cuts[1].issues(), ["WAR-127"]);
    assert!(
        loaded
            .record("docs/warlock-brief-21.md")
            .expect("the other brief is filed")
            .cuts()
            .is_empty()
    );
}

#[test]
fn titles_that_differ_only_in_case_or_spacing_are_one_key() {
    let same = [
        "The cut record",
        "the cut record",
        "THE CUT RECORD",
        "  The cut record  ",
        "The   cut\trecord",
        "The cut\nrecord",
    ];
    for title in same {
        assert_eq!(
            fold_title(title),
            "the cut record",
            "`{title}` names the slice that was already cut"
        );
        assert_eq!(
            CutRecord::new(title, ["WAR-125"], "2026-09-21T09:00:00Z").key(),
            "the cut record",
            "and a record made from it carries that key"
        );
    }

    // Anything past case and whitespace is a different slice: a retitled one,
    // which brief 23 says is honestly uncut rather than quietly already filed.
    let different = [
        "The cut records",
        "The cut-record",
        "Thecutrecord",
        "record cut The",
        "The cut record.",
        "",
    ];
    for title in different {
        assert_ne!(fold_title(title), "the cut record", "`{title}`");
    }

    assert_eq!(
        CutRecord::new("  The cut record  ", ["WAR-125"], "2026-09-21T09:00:00Z").title(),
        "  The cut record  ",
        "the title itself is stored unfolded and untrimmed"
    );
}

#[test]
fn a_cut_appended_to_a_record_this_warlock_did_not_write_leaves_the_rest_byte_for_byte() {
    let root = a_root();
    // Two hand-written records, neither in the shape this build emits, and
    // the cut goes on the second: what warlock adds has to land under the
    // record it belongs to without moving anything around it.
    let untouched = concat!(
        "[[filed]]\n",
        "path = \"docs/warlock-brief-21.md\"\n",
        "project_id = \"legacy-id\"\n",
        "url = \"https://linear.app/acme/project/warlock-brief-21\"\n",
        "scope = \" Data Plane \"\n",
        "team = \"dp\"\n",
        "filed_at = \"2026-09-19T09:00:00+01:00\"\n",
    );
    let cut_into = concat!(
        "[[filed]]\n",
        "path = \"docs/warlock-brief-23.md\"\n",
        "project_id = \"another-id\"\n",
        "url = \"https://linear.app/acme/project/warlock-brief-23\"\n",
        "scope = \"Cut a planned project\"\n",
        "team = \"dp\"\n",
        "filed_at = \"2026-09-21T08:00:00+01:00\"\n",
    );
    hand_write(
        root.path(),
        &format!("version = 2\n\n{untouched}\n{cut_into}"),
    );

    let mut loaded = Filed::load(root.path()).expect("loads");
    loaded
        .record_mut("docs/warlock-brief-23.md")
        .expect("the brief is filed")
        .push_cut(CutRecord::new(
            "The cut record",
            ["WAR-125"],
            "2026-09-21T09:00:00Z",
        ));
    loaded.save(root.path()).expect("saves");

    let after = fs::read_to_string(filed_path(root.path())).expect("reads");
    assert_eq!(
        after,
        format!(
            "version = 2\n\n{untouched}\n{cut_into}\n\
             [[filed.cut]]\n\
             title = \"The cut record\"\n\
             key = \"the cut record\"\n\
             issues = [\"WAR-125\"]\n\
             cut_at = \"2026-09-21T09:00:00Z\"\n"
        ),
        "both records come back as they were spelled, with the cut appended \
         under the second"
    );
    assert_eq!(
        Filed::load(root.path()).expect("reloads"),
        loaded,
        "and the whole file reloads equal to what was saved"
    );
    assert_eq!(
        warlock_dir_listing(root.path()),
        [FILED_FILE],
        "written through the same temporary and renamed, not left beside it"
    );
}

#[test]
fn a_record_this_warlock_did_not_write_survives_an_append_byte_for_byte() {
    let root = a_root();
    // Hand-written rather than produced by a save, so this is a statement
    // about the file format and not about the serialiser agreeing with
    // itself. The values are deliberately not what this build would emit:
    // an id that is not a UUID, a team key nobody here spells, a timestamp
    // with an offset rather than a `Z`, and an untrimmed scope.
    let body = concat!(
        "[[filed]]\n",
        "path = \"docs/warlock-brief-21.md\"\n",
        "project_id = \"legacy-id\"\n",
        "url = \"https://linear.app/acme/project/warlock-brief-21\"\n",
        "scope = \" Data Plane \"\n",
        "team = \"dp\"\n",
        "filed_at = \"2026-09-19T09:00:00+01:00\"\n",
    );
    hand_write(root.path(), &format!("version = 2\n\n{body}"));

    let mut loaded = Filed::load(root.path()).expect("loads");
    loaded.push(filed_brief());
    loaded.save(root.path()).expect("saves");

    let after = fs::read_to_string(filed_path(root.path())).expect("reads");
    assert!(
        after.starts_with(&format!("version = 2\n\n{body}")),
        "the record that was there is written back unfolded, untrimmed and in \
         place, with the new one after it:\n{after}"
    );
    assert_eq!(
        Filed::load(root.path()).expect("reloads").records(),
        loaded.records(),
        "and both records read back"
    );

    let kept = &loaded.records()[0];
    assert_eq!(kept.path(), "docs/warlock-brief-21.md");
    assert_eq!(kept.project_id(), "legacy-id");
    assert_eq!(
        kept.url(),
        "https://linear.app/acme/project/warlock-brief-21"
    );
    assert_eq!(kept.scope(), " Data Plane ", "not trimmed");
    assert_eq!(kept.team(), "dp", "not folded");
    assert_eq!(
        kept.filed_at(),
        "2026-09-19T09:00:00+01:00",
        "not reformatted"
    );
}

#[test]
fn a_load_then_save_moves_the_version_line_and_nothing_else() {
    let body = concat!(
        "[[filed]]\n",
        "path = \"docs/warlock-brief-21.md\"\n",
        "project_id = \"legacy-id\"\n",
        "url = \"https://linear.app/acme/project/warlock-brief-21\"\n",
        "scope = \"data-plane\"\n",
        "team = \"DP\"\n",
        "filed_at = \"2026-09-19T09:00:00+01:00\"\n",
    );
    let round_trip = |declared: u32| {
        let root = a_root();
        hand_write(root.path(), &format!("version = {declared}\n\n{body}"));
        Filed::load(root.path())
            .expect("loads")
            .save(root.path())
            .expect("saves");
        fs::read_to_string(filed_path(root.path())).expect("reads")
    };

    assert_eq!(
        round_trip(SCHEMA_VERSION),
        format!("version = {SCHEMA_VERSION}\n\n{body}"),
        "so filing something else does not churn the diff of what was filed before"
    );
    assert_eq!(
        round_trip(1),
        format!("version = {SCHEMA_VERSION}\n\n{body}"),
        "and a file from the older schema is upgraded at the version line alone: \
         the record it carried comes back byte for byte"
    );
}

#[test]
fn filing_does_not_change_the_repository_roots_subtree_hash() {
    let root = a_root();
    fs::write(root.path().join("WARLOCK.md"), "# module\n").expect("writes a document");

    let before = subtree_hash(root.path()).expect("hashes");

    Filed::with_records([filed_brief()])
        .save(root.path())
        .expect("saves");
    assert_eq!(
        subtree_hash(root.path()).expect("hashes"),
        before,
        "creating `.warlock/filed.toml` is not an edit to the tree it sits in"
    );

    let mut updated = Filed::load(root.path()).expect("loads");
    updated.push(
        FiledRecord::new(
            root.path(),
            root.path().join("WARLOCK.md"),
            "i",
            "u",
            "s",
            "t",
            "2026-09-20T08:00:00Z",
        )
        .expect("inside the root"),
    );
    updated.save(root.path()).expect("saves again");
    assert_eq!(
        subtree_hash(root.path()).expect("hashes"),
        before,
        "and neither is updating it, so nothing filed can restale a document"
    );
}

#[test]
fn a_newer_version_is_refused_on_both_reading_paths() {
    let text = "version = 3\n";

    assert!(matches!(
        Filed::from_toml_str(text),
        Err(Error::UnsupportedVersion {
            path: None,
            found: 3,
            supported: SCHEMA_VERSION,
        })
    ));
    assert!(
        toml::from_str::<Filed>(text).is_err(),
        "the derived path refuses what the module's reader refuses"
    );
    assert!(
        toml::from_str::<Filed>("version = 999\n").is_err(),
        "and refuses it however far ahead the file is"
    );

    let root = a_root();
    hand_write(root.path(), "version = 999\n\n[[filed]]\npath = \"x\"\n");
    let refusal = Filed::load(root.path()).expect_err("999 is not this schema");
    assert!(
        matches!(
            &refusal,
            Error::UnsupportedVersion { path: Some(path), found: 999, .. }
                if path == &filed_path(root.path())
        ),
        "refused before the records are read, and the file is named: {refusal:?}"
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
    // A directory with no `.warlock` at all is the same answer, not an I/O
    // error about the missing parent.
    assert!(matches!(
        Filed::load(root.path().join("nowhere")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn a_missing_file_is_distinguishable_from_a_corrupt_one() {
    let (missing, corrupt) = (a_root(), a_root());
    hand_write(corrupt.path(), "this is not filed records\n");

    assert!(matches!(
        Filed::load(missing.path()),
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        Filed::load(corrupt.path()),
        Err(Error::Syntax { path: Some(_), .. })
    ));
    assert!(matches!(
        Filed::from_toml_str("not toml"),
        Err(Error::Syntax { path: None, .. })
    ));
}

#[test]
fn a_lookup_answers_by_the_path_as_stored() {
    let mut filed = Filed::new();
    assert_eq!(filed.record("docs/warlock-brief-22.md"), None);

    filed.push(filed_brief());

    assert_eq!(
        filed.record("docs/warlock-brief-22.md"),
        Some(&filed_brief())
    );
    assert_eq!(
        filed
            .record("docs/warlock-brief-22.md")
            .map(FiledRecord::url),
        Some("https://linear.app/acme/project/warlock-brief-22"),
    );
    assert_eq!(filed.record("docs/warlock-brief-23.md"), None);
    assert_eq!(
        filed.record("./docs/warlock-brief-22.md"),
        None,
        "the stored form is the key, and nothing is normalised on the way in"
    );
}

#[test]
fn a_path_already_recorded_is_appended_and_still_findable() {
    // The module keeps both: refusing a second push is the caller's decision,
    // and the lookup it makes that decision with answers with the first.
    let mut filed = Filed::with_records([filed_brief()]);
    let again = FiledRecord::new(
        ".",
        "docs/warlock-brief-22.md",
        "second-id",
        "https://linear.app/acme/project/warlock-brief-22-again",
        "warlock-team",
        "WAR",
        "2026-09-21T07:32:00Z",
    )
    .expect("inside the root");
    filed.push(again);

    assert_eq!(filed.records().len(), 2);
    assert_eq!(
        filed
            .record("docs/warlock-brief-22.md")
            .map(FiledRecord::project_id),
        Some("b229262b-22aa-444a-a8af-0a2a3f4ef100"),
    );
}

#[test]
fn saving_creates_the_directory_and_leaves_no_temporary_file_behind() {
    let root = a_root();
    assert!(!root.path().join(".warlock").exists(), "nothing there yet");

    Filed::with_records([filed_brief()])
        .save(root.path())
        .expect("saves");

    // Exactly one file: the temporary the save wrote through has been renamed
    // away, not left lying next to the records.
    assert_eq!(warlock_dir_listing(root.path()), [FILED_FILE]);
    assert!(filed_path(root.path()).is_file());
}

#[test]
fn saving_again_replaces_the_file_and_still_leaves_one() {
    let root = a_root();
    Filed::with_records([filed_brief()])
        .save(root.path())
        .expect("saves");

    let replacement = Filed::new();
    replacement.save(root.path()).expect("saves over");

    assert_eq!(warlock_dir_listing(root.path()), [FILED_FILE]);
    assert_eq!(Filed::load(root.path()).expect("loads"), replacement);
}

#[test]
fn saving_beside_a_manifest_leaves_the_manifest_alone() {
    let root = a_root();
    crate::manifest::Manifest::new()
        .save(root.path())
        .expect("saves a manifest");

    Filed::with_records([filed_brief()])
        .save(root.path())
        .expect("saves");

    assert_eq!(
        warlock_dir_listing(root.path()),
        ["filed.toml", "pacts.toml"]
    );
    assert_eq!(
        crate::manifest::Manifest::load(root.path()).expect("loads"),
        crate::manifest::Manifest::new(),
    );
}

#[test]
fn paths_are_stored_with_forward_slashes() {
    let document = Path::new("docs").join("briefs").join("brief.md");
    let record = FiledRecord::new(".", &document, "i", "u", "s", "t", "2026-09-20T07:32:00Z")
        .expect("inside the root");

    assert_eq!(record.path(), "docs/briefs/brief.md");

    let text = Filed::with_records([record])
        .to_toml_string()
        .expect("serialises");
    assert!(text.contains("path = \"docs/briefs/brief.md\""), "{text}");
}

#[test]
fn the_same_records_under_two_roots_serialise_identically() {
    let under = |root: &Path| {
        FiledRecord::new(
            root,
            root.join("docs").join("warlock-brief-22.md"),
            "b229262b-22aa-444a-a8af-0a2a3f4ef100",
            "https://linear.app/acme/project/warlock-brief-22",
            "warlock-team",
            "WAR",
            "2026-09-20T07:32:00Z",
        )
        .expect("inside the root")
    };

    assert_eq!(
        under(Path::new("/tmp/one")),
        under(Path::new("/home/someone/else/two")),
    );
}

#[test]
fn the_same_records_saved_under_two_roots_give_byte_identical_files() {
    let saved_under = |root: &Path| {
        let filed = Filed::with_records([
            FiledRecord::new(
                root,
                root.join("docs").join("warlock-brief-22.md"),
                "b229262b-22aa-444a-a8af-0a2a3f4ef100",
                "https://linear.app/acme/project/warlock-brief-22",
                "warlock-team",
                "WAR",
                "2026-09-20T07:32:00Z",
            )
            .expect("inside the root"),
            FiledRecord::new(
                root,
                root,
                "root-id",
                "https://linear.app/acme/project/root",
                "warlock-team",
                "WAR",
                "2026-09-20T07:33:00Z",
            )
            .expect("is the root"),
        ]);
        filed.save(root).expect("saves");
        fs::read(filed_path(root)).expect("reads the file back")
    };

    let (one, two) = (a_root(), a_root());
    assert_ne!(one.path(), two.path(), "two different absolute roots");
    assert_eq!(saved_under(one.path()), saved_under(two.path()));
}

#[test]
fn a_path_outside_the_root_is_rejected() {
    assert!(matches!(
        FiledRecord::new(
            "/repo",
            "/elsewhere/brief.md",
            "i",
            "u",
            "s",
            "t",
            "2026-09-20T07:32:00Z"
        ),
        Err(Error::Path { .. })
    ));
}

#[test]
fn a_malformed_record_is_named_rather_than_read_as_absent() {
    // A missing key, a wrong-typed value and a key nothing declares. Each one
    // is the record being wrong, never the record being missing: a reader that
    // skipped it would say a filed brief had never been filed and push it a
    // second time.
    let bodies = [
        "[[filed]]\npath = \"docs/brief.md\"\nurl = \"u\"\nscope = \"s\"\nteam = \"t\"\nfiled_at = \"2026-09-20T07:32:00Z\"\n",
        "[[filed]]\npath = \"docs/brief.md\"\nproject_id = 7\nurl = \"u\"\nscope = \"s\"\nteam = \"t\"\nfiled_at = \"2026-09-20T07:32:00Z\"\n",
        "[[filed]]\npath = \"docs/brief.md\"\nproject_id = \"i\"\nurl = \"u\"\nscope = \"s\"\nteam = \"t\"\nfiled_at = \"2026-09-20T07:32:00Z\"\nreview_state = \"In Review\"\n",
    ];

    for body in bodies {
        let text = format!("version = 1\n\n{body}");
        match Filed::from_toml_str(&text) {
            Err(Error::Record {
                file: None,
                index: 0,
                brief: Some(brief),
                ..
            }) => assert_eq!(brief, "docs/brief.md", "named by the path it did spell"),
            other => panic!("expected a malformed-record error for `{body}`, got {other:?}"),
        }

        let root = a_root();
        hand_write(root.path(), &text);
        let error = Filed::load(root.path()).expect_err("the record is not readable");
        let message = error.to_string();
        assert!(message.contains("docs/brief.md"), "{message}");
        assert!(
            message.contains(&filed_path(root.path()).display().to_string()),
            "the message points at the file to go and hand-edit: {message}"
        );
    }
}

#[test]
fn a_record_that_does_not_even_spell_a_path_is_named_by_its_index() {
    let error = Filed::from_toml_str("version = 1\n\n[[filed]]\nproject_id = \"i\"\n")
        .expect_err("a record with no path is malformed");

    assert!(
        matches!(
            &error,
            Error::Record {
                index: 0,
                brief: None,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .starts_with("filed record 0 is malformed: "),
        "{error}"
    );
}

#[test]
fn every_error_variant_says_what_happened() {
    let path = PathBuf::from("/repo/.warlock/filed.toml");
    let cases = [
        (
            Error::NotFound { path: path.clone() },
            "no filed records at `/repo/.warlock/filed.toml`",
        ),
        (
            Error::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            },
            "could not read or write `/repo/.warlock/filed.toml`: denied",
        ),
        (
            Error::UnsupportedVersion {
                path: Some(path.clone()),
                found: 999,
                supported: SCHEMA_VERSION,
            },
            "filed records at `/repo/.warlock/filed.toml` declare schema version 999, which is \
             not supported; this build reads version 2 and earlier",
        ),
        (
            // No file behind it: `from_toml_str` was handed text, so the
            // message stops rather than naming a path that would send
            // somebody to the wrong file.
            Error::UnsupportedVersion {
                path: None,
                found: 999,
                supported: SCHEMA_VERSION,
            },
            "filed records declare schema version 999, which is not supported; this build reads \
             version 2 and earlier",
        ),
        (
            Error::Path {
                source: Box::new(
                    crate::manifest::to_manifest_path("/repo", "/elsewhere")
                        .expect_err("outside the root"),
                ),
            },
            "`/elsewhere` is not inside the manifest root `/repo`",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }

    // The three that wrap a parser or serialiser: the message names the record
    // or the file and then quotes what the underlying crate said.
    let named = Error::Record {
        file: Some(path.clone()),
        index: 1,
        brief: Some("docs/brief.md".to_owned()),
        source: Box::new(a_de_error()),
    };
    assert!(
        named.to_string().starts_with(
            "filed record 1 (`docs/brief.md`) in `/repo/.warlock/filed.toml` is malformed: "
        ),
        "{named}"
    );

    let syntax = Error::Syntax {
        path: Some(path),
        source: a_de_error(),
    };
    assert!(
        syntax
            .to_string()
            .starts_with("malformed filed records at `/repo/.warlock/filed.toml`: "),
        "{syntax}"
    );
    let unnamed = Error::Syntax {
        path: None,
        source: a_de_error(),
    };
    assert!(
        unnamed.to_string().starts_with("malformed filed records: "),
        "{unnamed}"
    );

    let serialize = Error::Serialize {
        source: a_ser_error(),
    };
    assert!(
        serialize
            .to_string()
            .starts_with("could not write the filed records as TOML: "),
        "{serialize}"
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
            path: None,
            source: a_de_error(),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Record {
            file: None,
            index: 0,
            brief: None,
            source: Box::new(a_de_error()),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Serialize {
            source: a_ser_error(),
        }
        .source()
        .is_some()
    );
    assert!(
        Error::Path {
            source: Box::new(
                crate::manifest::to_manifest_path("/repo", "/elsewhere")
                    .expect_err("outside the root")
            ),
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
        Error::UnsupportedVersion {
            path: None,
            found: 999,
            supported: SCHEMA_VERSION,
        }
        .source()
        .is_none()
    );
}
