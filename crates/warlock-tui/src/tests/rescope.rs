use std::fs;
use std::path::Path;

use warlock_engine::{Manifest, PactEntry, ScopeRecord, route_facts, scope, validate_scope};
use warlock_tui::RecordField;

use super::{RecordFields, Rescoped, ScopeRefusal, records_scope, rescope};

const HASH: &str = "d0f5a1";

const AT: &str = "2026-08-19T07:32:00Z";

fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
        .with_grant(HASH, AT)
}

/// Three `[[pact]]` rows and three `[[scope]]` records, more than any one
/// write below is allowed to reach.
fn recorded() -> Manifest {
    Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui"),
        entry("docs").with_scope("third-party"),
    ])
    .with_scopes([
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
        ScopeRecord::new("platform", "Platform", "Backlog", "area/platform"),
    ])
}

fn a_record() -> RecordFields<'static> {
    RecordFields {
        team: Some("Billing"),
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
    assert_eq!(
        rescoped.manifest.entry("docs").and_then(PactEntry::scope),
        Some("data-plane")
    );
}

#[test]
fn a_directory_with_no_entry_is_refused_for_a_set_and_a_clear() {
    for scope in [Some("billing"), None] {
        assert_eq!(
            refused("docs/adr", scope, a_record()),
            ScopeRefusal::NoPact {
                module: "docs/adr".to_owned()
            },
            "{scope:?}"
        );
    }
    assert_eq!(
        ScopeRefusal::NoPact {
            module: "crates/engine".to_owned(),
        }
        .to_string(),
        "`crates/engine` is not in the manifest, so there is no pact to carry a \
         scope; pact it in warlock first, with `p`"
    );
}

#[test]
fn the_scope_is_judged_before_the_entry_is_looked_for() {
    assert!(matches!(
        refused("docs/adr", Some("Not A Scope!"), a_record()),
        ScopeRefusal::Rule { .. }
    ));
}

#[test]
fn a_name_nothing_records_names_every_field_that_was_not_given() {
    for (record, missing) in [
        (
            RecordFields::default(),
            vec![
                RecordField::Team,
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
                team: None,
                ..a_record()
            },
            vec![RecordField::Team],
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
            RecordField::Team,
            RecordField::ReviewState,
            RecordField::Label,
        ],
    }
    .to_string();
    assert!(
        said.contains("`--team`, `--review-state` and `--label`"),
        "{said}"
    );
    assert!(said.contains("nothing was written"), "{said}");
}

#[test]
fn a_blank_value_is_refused_rather_than_trimmed_into_acceptability() {
    for (record, blank) in [
        (
            RecordFields {
                team: Some(""),
                ..a_record()
            },
            vec![RecordField::Team],
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
                team: Some(" "),
                review_state: Some(""),
                label: Some("\t "),
            },
            vec![
                RecordField::Team,
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
            team: Some(""),
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
        "`data-plane` already has a record in `.warlock/pacts.toml`, and warlock does not \
         rewrite one"
    );
    assert_eq!(
        crate::error::Error::Scope { refusal }.to_string(),
        "`data-plane` already has a record in `.warlock/pacts.toml`, and warlock does not \
         rewrite one: run without `--team`, `--review-state` and `--label` to write the \
         scope, or edit the file to change the record"
    );
}

#[test]
fn a_name_something_records_is_written_with_no_second_record() {
    let rescoped = written("crates/tui", Some("data-plane"), RecordFields::default());

    assert_eq!(rescoped.manifest.scopes(), recorded().scopes());
    assert_eq!(
        rescoped
            .manifest
            .entry("crates/tui")
            .and_then(PactEntry::scope),
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
            team: Some(" Billing Squad "),
            review_state: Some("In Review"),
            label: Some("Area/Billing"),
        },
    );
    let manifest = rescoped.manifest;

    assert_eq!(manifest.scopes()[..3], recorded().scopes()[..]);
    let record = manifest.scopes().last().expect("the record was added");
    assert_eq!(record.name(), "billing");
    assert_eq!(record.team(), " Billing Squad ");
    assert_eq!(record.review_state(), "In Review");
    assert_eq!(record.label(), "Area/Billing");

    let tui = manifest
        .entry("crates/tui")
        .expect("the entry is still there");
    assert_eq!(tui.scope(), Some(record.name()));
    assert_eq!(tui.document(), "crates/tui/WARLOCK.md");
    assert_eq!(tui.granted_hash(), Some(HASH));
    assert_eq!(tui.granted_at(), Some(AT));
    assert_eq!(
        manifest.entry("crates/engine"),
        recorded().entry("crates/engine")
    );
}

/// Every `\n\n`-separated block of the saved file that mentions none of
/// `written`. Over the file rather than the `Manifest`, because records
/// dropped by a rebuild are invisible in a value compared against the value
/// that dropped them.
fn blocks_apart_from(root: &Path, written: &[&str]) -> Vec<String> {
    fs::read_to_string(root.join(".warlock").join("pacts.toml"))
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
    // The version header, two pact rows and three records.
    assert_eq!(after.len(), 6, "{after:#?}");
}

#[test]
fn a_clear_reports_the_scope_it_took_away() {
    let rescoped = written("docs", None, RecordFields::default());

    assert_eq!(rescoped.scope, None);
    assert_eq!(rescoped.was.as_deref(), Some("third-party"));
    assert_eq!(
        rescoped.manifest.entry("docs").and_then(PactEntry::scope),
        None
    );
    assert_eq!(rescoped.manifest.scopes(), recorded().scopes());
}

#[test]
fn the_lookup_and_the_router_agree_about_which_names_are_recorded() {
    // `third-party` is recorded only under a capitalised spelling — a
    // different name to the router, which compares byte for byte.
    let manifest = Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/tui").with_scope("billing"),
        entry("docs").with_scope("third-party"),
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
