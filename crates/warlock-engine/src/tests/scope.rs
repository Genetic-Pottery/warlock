use super::{
    MAXIMUM_CHARACTERS, RULES, Rule, at_or_above, closed_scopes_at_or_below, is_scope_character,
    scope_covering, scope_opens_to, validate_scope, validate_sigil,
};
use crate::{Manifest, PactEntry, manifest, unpact_subtree};

fn entry(module: &str) -> PactEntry {
    PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
        .expect("a relative module path is inside the root")
}

fn covering<'manifest>(path: &str, manifest: &'manifest Manifest) -> Option<&'manifest str> {
    scope_covering(path, ".", manifest).expect("a relative path is inside the root")
}

fn of_length(characters: usize) -> String {
    let mut scope = String::from("a");
    while scope.chars().count() < characters {
        scope.push('1');
    }
    scope
}

#[test]
fn accepts_a_plain_scope() {
    assert_eq!(validate_scope("data-plane"), Ok(()));
}

#[test]
fn accepts_a_single_letter() {
    assert_eq!(validate_scope("a"), Ok(()));
}

#[test]
fn accepts_digits_and_an_underscore_inside() {
    assert_eq!(validate_scope("d_1"), Ok(()));
}

#[test]
fn accepts_the_longest_allowed_scope() {
    let scope = of_length(MAXIMUM_CHARACTERS);
    assert_eq!(scope.chars().count(), 24);
    assert_eq!(validate_scope(&scope), Ok(()));
}

#[test]
fn rejects_the_empty_string() {
    assert_eq!(validate_scope(""), Err(Rule::Empty));
}

#[test]
fn rejects_a_scope_beginning_with_a_digit() {
    assert_eq!(
        validate_scope("1data"),
        Err(Rule::Beginning { character: '1' })
    );
}

#[test]
fn rejects_a_scope_ending_with_a_separator() {
    assert_eq!(
        validate_scope("data-"),
        Err(Rule::Ending { character: '-' })
    );
    assert_eq!(
        validate_scope("data_"),
        Err(Rule::Ending { character: '_' })
    );
}

#[test]
fn rejects_uppercase() {
    // Folding is the caller's job, not this function's: `Data-Plane` is a
    // rejection here even though the prompt would have lower-cased it
    // before asking.
    assert_eq!(
        validate_scope("Data-Plane"),
        Err(Rule::Character { character: 'D' })
    );
}

#[test]
fn rejects_one_character_too_many() {
    let scope = of_length(MAXIMUM_CHARACTERS + 1);
    assert_eq!(scope.chars().count(), 25);
    assert_eq!(
        validate_scope(&scope),
        Err(Rule::TooLong {
            characters: 25,
            maximum: 24,
        })
    );
}

#[test]
fn rejects_non_ascii() {
    assert_eq!(
        validate_scope("données"),
        Err(Rule::Character { character: 'é' })
    );
    // Short in characters, long in bytes: still refused for what it holds
    // rather than for its length.
    assert_eq!(
        validate_scope("plan\u{2014}e"),
        Err(Rule::Character {
            character: '\u{2014}'
        })
    );
}

#[test]
fn rejects_the_wildcard_as_a_directory_scope() {
    assert_eq!(validate_scope("*"), Err(Rule::Character { character: '*' }));
}

#[test]
fn rejects_separators_and_punctuation_that_would_look_like_a_list() {
    for scope in ["data plane", "data,plane", "data/plane", "data.plane"] {
        assert!(
            validate_scope(scope).is_err(),
            "`{scope}` should not be a scope"
        );
    }
}

#[test]
fn a_scope_is_never_trimmed_before_judging() {
    assert_eq!(
        validate_scope(" data-plane"),
        Err(Rule::Character { character: ' ' })
    );
    assert_eq!(
        validate_scope("data-plane "),
        Err(Rule::Character { character: ' ' })
    );
}

#[test]
fn the_wildcard_is_a_sigil_and_only_a_sigil() {
    assert_eq!(validate_sigil("*"), Ok(()));
    assert!(validate_scope("*").is_err());
}

#[test]
fn a_sigil_is_otherwise_judged_exactly_as_a_scope_is() {
    for candidate in [
        "data-plane",
        "a",
        "d_1",
        "",
        "1data",
        "data-",
        "Data-Plane",
        "**",
        "*.rs",
    ] {
        assert_eq!(
            validate_sigil(candidate),
            validate_scope(candidate),
            "`{candidate}` should read the same on both sides"
        );
    }
}

#[test]
fn every_rule_renders_as_one_line() {
    let rules = [
        Rule::Empty,
        Rule::TooLong {
            characters: 25,
            maximum: 24,
        },
        Rule::Character { character: 'D' },
        Rule::Beginning { character: '1' },
        Rule::Ending { character: '-' },
    ];
    for rule in rules {
        let line = rule.to_string();
        assert!(!line.is_empty(), "{rule:?} renders as nothing");
        assert!(!line.contains('\n'), "{rule:?} renders as more than a line");
    }
}

#[test]
fn an_inner_scope_overrides_the_one_above_it() {
    let manifest = Manifest::with_entries([
        entry("crates").with_scope("platform"),
        entry("crates/engine").with_scope("data-plane"),
    ]);

    // Nearest wins: the outer scope is a default for what has said nothing,
    // not a second boundary the inner one is added to.
    assert_eq!(
        covering("crates/engine/src/lib.rs", &manifest),
        Some("data-plane")
    );
    assert_eq!(covering("crates/engine", &manifest), Some("data-plane"));
    assert_eq!(covering("crates/tui/src", &manifest), Some("platform"));
}

#[test]
fn a_directory_answers_with_its_own_scope() {
    let manifest = Manifest::with_entries([entry("crates/engine").with_scope("data-plane")]);

    assert_eq!(covering("crates/engine", &manifest), Some("data-plane"));
}

#[test]
fn a_path_nothing_covers_has_no_scope() {
    let manifest = Manifest::with_entries([entry("crates/engine").with_scope("data-plane")]);

    assert_eq!(covering("docs/adr/0001.md", &manifest), None);
    assert_eq!(covering("docs", &manifest), None);
    assert_eq!(covering(".", &manifest), None);
    // An empty manifest covers nothing, including the root itself.
    assert_eq!(covering("crates/engine", &Manifest::new()), None);
    assert_eq!(covering(".", &Manifest::new()), None);
}

#[test]
fn an_unscoped_entry_in_the_way_is_walked_straight_past() {
    let manifest = Manifest::with_entries([
        entry("crates").with_scope("platform"),
        entry("crates/engine"),
    ]);

    // A pacted directory with no scope is not a boundary of its own, so the
    // answer comes from above it.
    assert_eq!(covering("crates/engine/src", &manifest), Some("platform"));
}

#[test]
fn an_invalid_scope_is_stepped_over_for_the_next_valid_ancestor() {
    for invalid in [
        "",
        "1data",
        "data-",
        "*",
        "Data-Plane",
        "données",
        &of_length(25),
    ] {
        let manifest = Manifest::with_entries([
            entry("crates").with_scope("platform"),
            entry("crates/engine").with_scope(invalid),
        ]);

        assert_eq!(
            covering("crates/engine/src/lib.rs", &manifest),
            Some("platform"),
            "`{invalid}` should read as no scope and fall through"
        );
        assert_eq!(
            covering("crates/engine", &manifest),
            Some("platform"),
            "`{invalid}` should read as no scope on its own directory too"
        );
    }
}

#[test]
fn an_invalid_scope_with_nothing_valid_above_it_covers_nothing() {
    let manifest = Manifest::with_entries([
        entry("crates").with_scope("Platform"),
        entry("crates/engine").with_scope("data-"),
    ]);

    // Two typos and no valid scope anywhere above: unscoped, and never one
    // of the two strings that are not scopes.
    assert_eq!(covering("crates/engine/src/lib.rs", &manifest), None);
    assert_eq!(covering("crates", &manifest), None);
}

#[test]
fn a_sibling_that_shares_a_prefix_is_not_covered() {
    let manifest = Manifest::with_entries([
        entry("crates/engine").with_scope("data-plane"),
        entry("crates/engine-tools"),
    ]);

    // Textual prefix matching would hand `data-plane` to both of these.
    assert_eq!(covering("crates/engine-tools", &manifest), None);
    assert_eq!(covering("crates/engine-tools/src/main.rs", &manifest), None);
    assert_eq!(covering("crates/engineer", &manifest), None);
}

#[test]
fn a_scope_on_the_root_covers_everything_below_it() {
    let manifest = Manifest::with_entries([
        entry(".").with_scope("whole-repo"),
        entry("crates/engine").with_scope("data-plane"),
    ]);

    assert_eq!(covering(".", &manifest), Some("whole-repo"));
    assert_eq!(covering("docs/adr", &manifest), Some("whole-repo"));
    assert_eq!(covering("crates/engine/src", &manifest), Some("data-plane"));
}

#[test]
fn an_absolute_path_under_the_root_answers_the_same_as_a_relative_one() {
    let manifest = Manifest::with_entries([entry("crates/engine").with_scope("data-plane")]);
    let root = std::path::Path::new("/repo");

    let covered = scope_covering(
        root.join("crates").join("engine").join("src"),
        root,
        &manifest,
    )
    .expect("a path under the root has a manifest-relative form");
    assert_eq!(covered, Some("data-plane"));
}

#[test]
fn a_path_outside_the_root_is_an_error_rather_than_unscoped() {
    let manifest = Manifest::with_entries([entry(".").with_scope("whole-repo")]);

    // Not "open to anyone": this manifest has nothing to say about it.
    assert!(scope_covering("../elsewhere", ".", &manifest).is_err());
}

#[test]
fn ancestors_are_nearest_first_and_end_at_the_root() {
    assert_eq!(
        at_or_above("crates/engine/src/lib.rs").collect::<Vec<_>>(),
        [
            "crates/engine/src/lib.rs",
            "crates/engine/src",
            "crates/engine",
            "crates",
            "."
        ]
    );
    assert_eq!(at_or_above("crates").collect::<Vec<_>>(), ["crates", "."]);
    assert_eq!(at_or_above(".").collect::<Vec<_>>(), ["."]);
}

#[test]
fn the_rules_line_states_the_rules_it_is_about() {
    // The line is shown over a field somebody is typing a scope into, so a
    // ceiling or a character class that drifted from the validator would be
    // a promise the next keystroke breaks.
    assert!(
        RULES.contains(&MAXIMUM_CHARACTERS.to_string()),
        "the line does not name the maximum it is about: {RULES:?}"
    );
    for described in ['-', '_'] {
        assert!(RULES.contains(described), "{described:?} is not named");
        assert!(is_scope_character(described));
    }
    // And what it describes is accepted, character for character: a name
    // built out of exactly what the line names, at exactly the length it
    // promises, is a scope.
    let promised = format!("{}a-9_z", of_length(MAXIMUM_CHARACTERS - 5));
    assert_eq!(promised.chars().count(), MAXIMUM_CHARACTERS);
    assert_eq!(
        validate_scope(&promised),
        Ok(()),
        "the line promises {promised:?} is a scope"
    );
}

fn held(sigils: &[&str]) -> Vec<String> {
    sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
}

#[test]
fn a_directory_nobody_scoped_is_open_to_everyone() {
    assert!(scope_opens_to(None, &held(&["web"])));
    assert!(scope_opens_to(None, &[]));
    assert!(
        scope_opens_to(None, &held(&["data-plane"])),
        "no scope is not a locked scope: there is no boundary to be outside of"
    );
}

#[test]
fn holding_nothing_opens_nothing_that_is_scoped() {
    assert!(
        !scope_opens_to(Some("data-plane"), &[]),
        "a sigil is what opens a scope, so holding none opens none"
    );
    assert!(
        scope_opens_to(None, &[]),
        "but an unscoped directory is open to a machine holding nothing: the \
             permissive default is on the directory and only there"
    );
}

#[test]
fn the_empty_set_is_refused_exactly_as_a_wrong_sigil_is() {
    // "Nobody told me what you hold" and "what you hold does not match" are
    // the same answer to the only question asked here. If they differed, the
    // default state of every machine would be a universal bypass.
    assert_eq!(
        scope_opens_to(Some("data-plane"), &[]),
        scope_opens_to(Some("data-plane"), &held(&["web"]))
    );
}

#[test]
fn any_one_held_sigil_opens_the_scope_it_matches() {
    let sigils = held(&["web", "billing", "data-plane"]);

    for scope in ["web", "billing", "data-plane"] {
        assert!(
            scope_opens_to(Some(scope), &sigils),
            "`{scope}` is held, so it opens: this is membership, not an expression"
        );
    }
}

#[test]
fn a_scope_none_of_the_held_sigils_match_is_closed() {
    assert!(!scope_opens_to(
        Some("data-plane"),
        &held(&["web", "billing"])
    ));
}

#[test]
fn the_wildcard_sigil_opens_every_scope() {
    assert!(scope_opens_to(Some("data-plane"), &held(&["*"])));
    assert!(
        scope_opens_to(Some("billing"), &held(&["web", "*"])),
        "the wildcard opens alongside ordinary sigils, not only alone"
    );
}

#[test]
fn matching_is_byte_for_byte_and_never_partial() {
    assert!(
        !scope_opens_to(Some("data-plane"), &held(&["data"])),
        "a prefix is a different sigil"
    );
    assert!(
        !scope_opens_to(Some("data"), &held(&["data-plane"])),
        "and so is an extension of one"
    );
    assert!(
        !scope_opens_to(Some("web"), &held(&["Web"])),
        "case is not folded here: folding belongs where a person supplies the string"
    );
}

#[test]
fn a_near_miss_wildcard_opens_nothing() {
    for sigil in ["**", "*.rs", "?"] {
        assert!(
            !scope_opens_to(Some("web"), &held(&[sigil])),
            "`{sigil}` is not the wildcard, and `validate_sigil` refuses it anyway"
        );
    }
}

#[test]
fn coverage_and_opening_compose_into_the_whole_question() {
    let manifest = Manifest::with_entries([
        entry("crates").with_scope("platform"),
        entry("crates/engine").with_scope("data-plane"),
        entry("docs"),
    ]);
    let sigils = held(&["platform"]);

    let covering = |path| {
        scope_covering(path, ".", &manifest).expect("a path under the root has a stored form")
    };

    assert!(
        scope_opens_to(covering("crates/tui/src"), &sigils),
        "the nearest scope is `platform`, which is held"
    );
    assert!(
        !scope_opens_to(covering("crates/engine/src"), &sigils),
        "the nearer scope replaces the outer one outright: an outer scope is a default, \
             never a second gate that holding `platform` could satisfy on its behalf"
    );
    assert!(
        scope_opens_to(covering("docs"), &sigils),
        "nothing at or above `docs` carries a scope"
    );
}

fn scoped(modules: &[(&str, Option<&str>)]) -> Manifest {
    Manifest::with_entries(modules.iter().map(|(module, scope)| {
        let entry = PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative path inside the root is storable");
        match scope {
            Some(scope) => entry.with_scope(*scope),
            None => entry,
        }
    }))
}

#[test]
fn a_scoped_descendant_this_machine_does_not_open_blocks() {
    // The case the whole question is about: the target says nothing, so
    // coverage on the target alone answers "open", and the boundary is one
    // directory down.
    let manifest = scoped(&[
        ("crates", None),
        ("crates/engine", Some("data-plane")),
        ("crates/tui", None),
    ]);

    assert_eq!(
        closed_scopes_at_or_below("crates", ".", &manifest, &held(&["web"]))
            .expect("a path inside the root"),
        ["data-plane"],
    );
    // And the entry carrying it is exactly one an un-pact would drop.
    assert!(
        unpact_subtree("crates", ".", &manifest)
            .expect("un-pacts")
            .entry("crates/engine")
            .is_none(),
    );
}

#[test]
fn a_scoped_descendant_this_machine_opens_does_not_block() {
    let manifest = scoped(&[("crates", None), ("crates/engine", Some("data-plane"))]);

    for sigils in [
        held(&["data-plane"]),
        held(&["web", "data-plane"]),
        held(&["*"]),
    ] {
        assert!(
            closed_scopes_at_or_below("crates", ".", &manifest, &sigils)
                .expect("a path inside the root")
                .is_empty(),
            "{sigils:?} opens it",
        );
    }
}

#[test]
fn the_target_s_own_scope_is_asked_about_too_and_does_not_license_what_is_below() {
    let manifest = scoped(&[
        ("crates", Some("platform")),
        ("crates/engine", Some("data-plane")),
    ]);

    // Holding the target's own scope is not permission over the boundary
    // inside it: these are two questions, not one.
    assert_eq!(
        closed_scopes_at_or_below("crates", ".", &manifest, &held(&["platform"]))
            .expect("a path inside the root"),
        ["data-plane"],
    );
    // "At or below" is at, too: the target's own closed scope is blocking.
    assert_eq!(
        closed_scopes_at_or_below("crates", ".", &manifest, &held(&["data-plane"]))
            .expect("a path inside the root"),
        ["platform"],
    );
}

#[test]
fn an_unscoped_root_buys_nothing_over_the_scopes_below_it() {
    // `unpact .` drops every entry there is, so it is asked about every
    // entry there is — the root's silence is the absence of a statement,
    // not permission over the statements under it.
    let manifest = scoped(&[
        (".", None),
        ("crates/engine", Some("data-plane")),
        ("docs", None),
    ]);

    for directory in [".", "/repo"] {
        assert_eq!(
            closed_scopes_at_or_below(directory, "/repo", &manifest, &held(&["web"]))
                .expect("a path inside the root"),
            ["data-plane"],
            "{directory}",
        );
    }
    // Nothing scoped below, or all of it held, and the root un-pact is the
    // ordinary one it has always been.
    assert!(
        closed_scopes_at_or_below(".", "/repo", &manifest, &held(&["data-plane"]))
            .expect("a path inside the root")
            .is_empty(),
    );
    assert!(
        closed_scopes_at_or_below(".", "/repo", &scoped(&[(".", None), ("crates", None)]), &[])
            .expect("a path inside the root")
            .is_empty(),
    );
}

#[test]
fn a_sibling_that_shares_a_prefix_carries_no_blocking_scope() {
    // The same segment-wise match `unpact_subtree` uses: what is not
    // dropped cannot block.
    let manifest = scoped(&[
        ("crates/engine", None),
        ("crates/engine-tools", Some("tooling")),
        ("crates/engineering", Some("estimates")),
    ]);

    assert!(
        closed_scopes_at_or_below("crates/engine", ".", &manifest, &[])
            .expect("a path inside the root")
            .is_empty(),
    );
    assert_eq!(
        closed_scopes_at_or_below("crates/engine-tools", ".", &manifest, &[])
            .expect("a path inside the root"),
        ["tooling"],
    );
}

#[test]
fn a_descendant_scope_that_is_not_a_scope_does_not_block() {
    // Read as no scope, exactly as `scope_covering` reads it: one rule for
    // what a boundary is, not two. A valid one beside it still blocks, and
    // the string that is not a scope is not named in the answer.
    let manifest = scoped(&[
        ("crates", None),
        ("crates/engine", Some("Data Plane!")),
        ("crates/tui", Some("")),
        ("crates/store", Some("data-plane")),
    ]);

    assert_eq!(
        closed_scopes_at_or_below("crates", ".", &manifest, &[]).expect("a path inside the root"),
        ["data-plane"],
    );
    assert!(
        closed_scopes_at_or_below("crates/engine", ".", &manifest, &[])
            .expect("a path inside the root")
            .is_empty(),
    );
}

#[test]
fn a_path_with_no_manifest_relative_form_is_the_same_error_an_un_pact_gives() {
    let manifest = scoped(&[("crates/engine", Some("data-plane"))]);

    assert!(matches!(
        closed_scopes_at_or_below("/elsewhere/crates", "/repo", &manifest, &[]),
        Err(manifest::Error::PathOutsideRoot { .. })
    ));
}
