use std::fs;
use std::path::Path;

use warlock_engine::{
    LinearScope, Manifest, ScopeRecord, ScopedDirectory, route_facts, scope, validate_scope,
};

use super::{RecordFields, Rescoped, ScopeRefusal, TrackerFields, records_scope, rescope};
use crate::prompt::RecordField;

// The team key is read through the view, which is the only way there is to
// read one off a record.
fn team_of(record: &ScopeRecord) -> &str {
    LinearScope::of(record)
        .expect("the record was written as a Linear scope")
        .team_key()
}

fn row(module: &str, scope: &str) -> ScopedDirectory {
    ScopedDirectory::new(".", module, scope).expect("a relative module path is inside the root")
}

fn scope_at<'a>(manifest: &'a Manifest, module: &str) -> Option<&'a str> {
    manifest.directory(module).map(ScopedDirectory::scope)
}

/// Two `[[directory]]` rows and three `[[scope]]` records, more than any one
/// write below is allowed to reach. `crates/tui` has no row.
fn recorded() -> Manifest {
    Manifest::with_directories([
        row("crates/engine", "data-plane"),
        row("docs", "third-party"),
    ])
    .with_scopes([
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
        ScopeRecord::new("platform", "Platform", "Backlog", "area/platform"),
    ])
}

fn a_record() -> RecordFields<'static> {
    RecordFields {
        tracker: TrackerFields::Linear {
            team_key: Some("Billing"),
        },
        review_state: Some("In Review"),
        label: Some("area/billing"),
    }
}

fn refused(module: &str, scope: Option<&str>, record: RecordFields<'_>) -> ScopeRefusal {
    rescope(&recorded(), module, scope, record).expect_err("this write is refused")
}

fn written(module: &str, scope: Option<&str>, record: RecordFields<'_>) -> Rescoped {
    rescope(&recorded(), module, scope, record).expect("this write is allowed")
}

#[test]
fn what_was_typed_is_folded_and_then_judged_by_the_engine() {
    // The list, the capital-with-a-space, the empty string and a trailing
    // separator. Each is judged after the fold, so the sentence expected is the
    // engine's about the lower-cased text.
    for (given, folded) in [
        ("control-plane, data-plane", "control-plane, data-plane"),
        ("Control Plane", "control plane"),
        ("", ""),
        ("data-plane-", "data-plane-"),
    ] {
        let refusal = refused("docs", Some(given), a_record());
        let rule = validate_scope(folded).expect_err("this text is not a scope");

        assert_eq!(
            refusal,
            ScopeRefusal::Rule { rule: rule.clone() },
            "{given:?}"
        );
        assert_eq!(refusal.to_string(), rule.to_string(), "{given:?}");
    }
    assert_eq!(
        ScopeRefusal::Rule {
            rule: scope::Rule::Empty
        }
        .to_string(),
        "a scope cannot be empty"
    );

    let rescoped = written("docs", Some("Data-Plane"), RecordFields::default());
    assert_eq!(rescoped.scope.as_deref(), Some("data-plane"));
    assert_eq!(scope_at(&rescoped.manifest, "docs"), Some("data-plane"));
}

#[test]
fn a_directory_with_no_row_gains_one_for_a_set_and_stays_without_for_a_clear() {
    let set = written("docs/adr", Some("third-party"), RecordFields::default());
    assert_eq!(scope_at(&set.manifest, "docs/adr"), Some("third-party"));
    assert_eq!(set.was, None);

    let cleared = written("docs/adr", None, RecordFields::default());
    assert_eq!(cleared.manifest, recorded());
    assert_eq!(cleared.was, None);
}

#[test]
fn a_name_nothing_records_names_every_field_that_was_not_given() {
    for (record, missing) in [
        (
            RecordFields::default(),
            vec![
                RecordField::TeamKey,
                RecordField::ReviewState,
                RecordField::Label,
            ],
        ),
        (
            RecordFields {
                review_state: None,
                label: None,
                ..a_record()
            },
            vec![RecordField::ReviewState, RecordField::Label],
        ),
        (
            RecordFields {
                tracker: TrackerFields::Linear { team_key: None },
                ..a_record()
            },
            vec![RecordField::TeamKey],
        ),
        // The tracker it was handed decides which fields a refusal can name
        // and the order it names them in: Jira's two, in the order
        // `ScopeRecord::jira` takes them, and no team key.
        (
            RecordFields {
                tracker: TrackerFields::Jira {
                    base_url: None,
                    project_key: Some("PAY"),
                },
                ..a_record()
            },
            vec![RecordField::BaseUrl],
        ),
        (
            RecordFields {
                tracker: TrackerFields::Jira {
                    base_url: None,
                    project_key: None,
                },
                label: None,
                ..a_record()
            },
            vec![
                RecordField::BaseUrl,
                RecordField::ProjectKey,
                RecordField::Label,
            ],
        ),
    ] {
        assert_eq!(
            refused("crates/tui", Some("billing"), record),
            ScopeRefusal::NeedsRecord {
                scope: "billing".to_owned(),
                missing: missing.clone(),
            },
            "{record:?}"
        );
    }

    let said = ScopeRefusal::NeedsRecord {
        scope: "billing".to_owned(),
        missing: vec![
            RecordField::TeamKey,
            RecordField::ReviewState,
            RecordField::Label,
        ],
    }
    .to_string();
    assert!(
        said.contains("`--team-key`, `--review-state` and `--label`"),
        "{said}"
    );
    assert!(said.contains("nothing was written"), "{said}");
}

#[test]
fn a_blank_value_is_refused_rather_than_trimmed_into_acceptability() {
    for (record, blank) in [
        (
            RecordFields {
                tracker: TrackerFields::Linear { team_key: Some("") },
                ..a_record()
            },
            vec![RecordField::TeamKey],
        ),
        (
            RecordFields {
                review_state: Some("   "),
                ..a_record()
            },
            vec![RecordField::ReviewState],
        ),
        (
            RecordFields {
                tracker: TrackerFields::Linear {
                    team_key: Some(" "),
                },
                review_state: Some(""),
                label: Some("\t "),
            },
            vec![
                RecordField::TeamKey,
                RecordField::ReviewState,
                RecordField::Label,
            ],
        ),
    ] {
        assert_eq!(
            refused("crates/tui", Some("billing"), record),
            ScopeRefusal::BlankRecord {
                fields: blank.clone()
            },
            "{record:?}"
        );
    }

    let said = ScopeRefusal::BlankRecord {
        fields: vec![RecordField::Label],
    }
    .to_string();
    assert_eq!(said, "`--label` cannot be blank, so nothing was written");
}

#[test]
fn a_value_at_a_name_something_already_records_is_refused_before_it_is_judged_blank() {
    for record in [
        a_record(),
        RecordFields {
            label: Some("area/other"),
            ..RecordFields::default()
        },
        RecordFields {
            tracker: TrackerFields::Linear { team_key: Some("") },
            ..RecordFields::default()
        },
    ] {
        assert_eq!(
            refused("crates/tui", Some("data-plane"), record),
            ScopeRefusal::Recorded {
                scope: "data-plane".to_owned()
            },
            "{record:?}"
        );
    }
}

#[test]
fn the_shell_adds_its_flag_advice_to_a_recorded_refusal_and_the_panel_does_not() {
    let refusal = ScopeRefusal::Recorded {
        scope: "data-plane".to_owned(),
    };

    assert_eq!(
        refusal.to_string(),
        "`data-plane` already has a record in `.warlock/scopes.toml`, and warlock does not \
         rewrite one"
    );
    assert_eq!(
        crate::error::Error::Scope { refusal }.to_string(),
        "`data-plane` already has a record in `.warlock/scopes.toml`, and warlock does not \
         rewrite one: run with the path and the scope alone — no tracker, no `--team-key`, \
         `--review-state` or `--label` — to write the scope, or edit the file to change the \
         record"
    );
}

#[test]
fn a_name_something_records_is_written_with_no_second_record() {
    let rescoped = written("crates/tui", Some("data-plane"), RecordFields::default());

    assert_eq!(rescoped.manifest.scopes(), recorded().scopes());
    assert_eq!(
        scope_at(&rescoped.manifest, "crates/tui"),
        Some("data-plane")
    );
    assert_eq!(rescoped.was, None);
}

#[test]
fn a_new_record_is_stored_as_given_after_the_ones_already_there() {
    // Spaced and capitalised on purpose: a team, a review state and a label
    // belong to somebody's tracker, and anything trimmed or folded on the way
    // through here is a name that no longer matches the one over there.
    let rescoped = written(
        "crates/tui",
        Some("billing"),
        RecordFields {
            tracker: TrackerFields::Linear {
                team_key: Some(" Billing Squad "),
            },
            review_state: Some("In Review"),
            label: Some("Area/Billing"),
        },
    );
    let manifest = rescoped.manifest;

    assert_eq!(manifest.scopes()[..3], recorded().scopes()[..]);
    let record = manifest.scopes().last().expect("the record was added");
    assert_eq!(record.name(), "billing");
    assert_eq!(team_of(record), " Billing Squad ");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "Area/Billing");

    assert_eq!(scope_at(&manifest, "crates/tui"), Some(record.name()));
    assert_eq!(
        manifest.directory("crates/engine"),
        recorded().directory("crates/engine")
    );
}

#[test]
fn jira_fields_are_written_as_a_jira_record_with_both_values_as_given() {
    // The trailing slash and the spaces around the project key are somebody's
    // site and somebody's project as they typed them, so they are handed on
    // untouched for the reason the Linear write above is.
    let rescoped = written(
        "crates/tui",
        Some("payments"),
        RecordFields {
            tracker: TrackerFields::Jira {
                base_url: Some("https://acme.atlassian.net/"),
                project_key: Some(" PAY "),
            },
            review_state: Some("In Review"),
            label: Some("area/pay"),
        },
    );
    let manifest = rescoped.manifest;

    assert_eq!(manifest.scopes()[..3], recorded().scopes()[..]);
    let record = manifest.scopes().last().expect("the record was added");
    assert_eq!(
        record,
        &ScopeRecord::jira(
            "payments",
            "https://acme.atlassian.net/",
            " PAY ",
            "In Review",
            "area/pay",
        ),
        "the record written is the one `ScopeRecord::jira` builds from the same values"
    );
    assert!(
        LinearScope::of(record).is_err(),
        "a Jira record is not a Linear scope, which is the whole of what the tracker decides"
    );
    assert_eq!(scope_at(&manifest, "crates/tui"), Some("payments"));

    // Through the file as well as the value: which board a record names is a
    // key somebody reads and hand-edits.
    let text = manifest.to_toml_string().expect("the manifest serialises");
    for line in [
        "tracker = \"jira\"",
        "base_url = \"https://acme.atlassian.net/\"",
        "project_key = \" PAY \"",
    ] {
        assert!(text.contains(line), "no `{line}` in {text}");
    }
}

/// Every `\n\n`-separated block of the saved file that mentions none of
/// `written`. Over the file rather than the `Manifest`, because records
/// dropped by a rebuild are invisible in a value compared against the value
/// that dropped them.
fn blocks_apart_from(root: &Path, written: &[&str]) -> Vec<String> {
    fs::read_to_string(root.join(".warlock").join("scopes.toml"))
        .expect("the manifest was saved")
        .trim_end_matches('\n')
        .split("\n\n")
        .filter(|block| !written.iter().any(|needle| block.contains(needle)))
        .map(str::to_owned)
        .collect()
}

#[test]
fn recording_a_scope_leaves_every_row_and_record_it_did_not_touch_byte_identical() {
    let repo = tempfile::tempdir().expect("a temporary directory");
    let touched = ["crates/tui", "billing"];
    recorded()
        .save(repo.path())
        .expect("the fixture was written");
    let before = blocks_apart_from(repo.path(), &touched);

    written("crates/tui", Some("billing"), a_record())
        .manifest
        .save(repo.path())
        .expect("the write was saved");

    let after = blocks_apart_from(repo.path(), &touched);
    assert_eq!(after, before);
    // The version header, two directory rows and three records.
    assert_eq!(after.len(), 6, "{after:#?}");
}

#[test]
fn a_clear_reports_the_scope_it_took_away() {
    let rescoped = written("docs", None, RecordFields::default());

    assert_eq!(rescoped.scope, None);
    assert_eq!(rescoped.was.as_deref(), Some("third-party"));
    assert_eq!(scope_at(&rescoped.manifest, "docs"), None);
    assert_eq!(rescoped.manifest.scopes(), recorded().scopes());
}

#[test]
fn the_lookup_and_the_router_agree_about_which_names_are_recorded() {
    // `third-party` is recorded only under a capitalised spelling — a
    // different name to the router, which compares byte for byte.
    let manifest = Manifest::with_directories([
        row("crates/engine", "data-plane"),
        row("crates/tui", "billing"),
        row("docs", "third-party"),
    ])
    .with_scopes([
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("Third-Party", "Vendor", "Triage", "area/vendor"),
    ]);

    for (module, scope, has_a_record) in [
        ("crates/engine", "data-plane", true),
        ("crates/tui", "billing", false),
        ("docs", "third-party", false),
    ] {
        assert_eq!(records_scope(&manifest, scope), has_a_record, "{scope}");
        assert_eq!(
            route_facts(module, ".", &manifest, None)
                .expect("the module path is inside the root")
                .record()
                .is_some(),
            has_a_record,
            "{scope} covers {module}",
        );
    }
}
