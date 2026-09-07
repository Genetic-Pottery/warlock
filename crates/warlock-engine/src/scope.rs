use std::fmt;
use std::path::Path;

use crate::manifest::{self, Manifest, PactEntry, ROOT_MODULE, to_manifest_path};

const MAXIMUM_CHARACTERS: usize = 24;

// The one line every caller that asks a person for a scope shows, so a prompt
// cannot spell out a second copy of the rules and drift from the one that
// judges. The number cannot be interpolated into a `const`, so a test below
// holds this string and `MAXIMUM_CHARACTERS` together: change the ceiling and
// that test fails rather than a window quietly promising the old one. It leaves
// out the two rules about the ends on purpose — a hint long enough to need
// reading twice is not read once, and `Rule` has a line ready for the moment
// either is broken.
pub const RULES: &str = "1 to 24 characters: lowercase letters, digits, `-` and `_`";

const WILDCARD: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Empty,
    TooLong { characters: usize, maximum: usize },
    Character { character: char },
    Beginning { character: char },
    Ending { character: char },
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "a scope cannot be empty"),
            Self::TooLong {
                characters,
                maximum,
            } => write!(
                f,
                "a scope is at most {maximum} characters, and this one is {characters}"
            ),
            Self::Character { character } => write!(
                f,
                "a scope holds only lowercase letters, digits, `-` and `_`, and this one holds `{character}`"
            ),
            Self::Beginning { character } => write!(
                f,
                "a scope begins with a lowercase letter, and this one begins with `{character}`"
            ),
            Self::Ending { character } => write!(
                f,
                "a scope does not end with `-` or `_`, and this one ends with `{character}`"
            ),
        }
    }
}

impl std::error::Error for Rule {}

// Judges, and deliberately never normalises: no `String` comes back, so there is
// no repaired scope anywhere in this crate to be confused with the one somebody
// wrote. Folding case here — the obvious kindness — would put the manifest
// reader in the business of correcting committed bytes. It belongs instead at
// the two places a person supplies the string, the scope prompt and
// `warlock config`, which lower-case and *then* ask.
pub fn validate_scope(scope: &str) -> Result<(), Rule> {
    // Taking the two end characters here is also the emptiness check: a string
    // with no first character is the empty one.
    let (Some(first), Some(last)) = (scope.chars().next(), scope.chars().next_back()) else {
        return Err(Rule::Empty);
    };

    // Characters rather than bytes: the two agree for everything this accepts,
    // and counting bytes would call a short non-ASCII name long as well as
    // ill-charactered, which is the less useful of the two things to say.
    let characters = scope.chars().count();
    if characters > MAXIMUM_CHARACTERS {
        return Err(Rule::TooLong {
            characters,
            maximum: MAXIMUM_CHARACTERS,
        });
    }

    if let Some(character) = scope
        .chars()
        .find(|character| !is_scope_character(*character))
    {
        return Err(Rule::Character { character });
    }

    if !first.is_ascii_lowercase() {
        return Err(Rule::Beginning { character: first });
    }

    if is_separator(last) {
        return Err(Rule::Ending { character: last });
    }

    Ok(())
}

// The wildcard is spellable on the sigil side and refused on the scope side, and
// the asymmetry is the design. A person holding nothing and a person holding
// everything are different claims. On a directory `*` would mean "open to
// anyone", which having no scope already means, and two spellings of one meaning
// is how a vocabulary rots. Everything else defers to `validate_scope`, so there
// is still exactly one character class in the workspace.
pub fn validate_sigil(sigil: &str) -> Result<(), Rule> {
    if sigil == WILDCARD {
        return Ok(());
    }

    validate_scope(sigil)
}

/// ```
/// use warlock_engine::{Manifest, PactEntry, scope_covering};
///
/// let manifest = Manifest::with_entries([
///     PactEntry::new(".", "crates", "crates/WARLOCK.md")?.with_scope("platform"),
///     PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?.with_scope("data-plane"),
///     PactEntry::new(".", "crates/engine-tools", "crates/engine-tools/WARLOCK.md")?,
/// ]);
///
/// // The nearest scoped ancestor wins, and a directory answers with its own.
/// assert_eq!(scope_covering("crates/engine/src/lib.rs", ".", &manifest)?, Some("data-plane"));
/// assert_eq!(scope_covering("crates/engine", ".", &manifest)?, Some("data-plane"));
/// // A sibling that merely shares a prefix is not below it: `crates` covers this.
/// assert_eq!(scope_covering("crates/engine-tools/src", ".", &manifest)?, Some("platform"));
/// // Nothing at or above it carries a scope.
/// assert_eq!(scope_covering("docs/adr", ".", &manifest)?, None);
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
// At most one scope, nearest wins, and that is the design rather than a
// simplification waiting to be relaxed. An inner scope replaces an outer one
// outright; the outer one is a default for everything below that has said
// nothing, never a second gate to also satisfy. Nothing accumulates and there is
// no list.
pub fn scope_covering(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
) -> Result<Option<&str>, manifest::Error> {
    let stored = to_manifest_path(root, path)?;
    Ok(at_or_above(&stored).find_map(|module| valid_scope_on(manifest, module)))
}

fn valid_scope_on<'manifest>(
    manifest: &'manifest Manifest,
    module: &str,
) -> Option<&'manifest str> {
    valid_scope(manifest.entry(module)?)
}

// A scope the validator refuses reads as no scope, indistinguishable from an
// absent one to every caller: one typo in a hand-edited manifest widens the
// boundary to its parent's rather than inventing a boundary nobody wrote, and
// this never hands back a string that is not a scope. Crate-visible so that a
// caller walking the entries itself calls this rule rather than writing a second
// copy of it.
pub(crate) fn valid_scope(entry: &PactEntry) -> Option<&str> {
    entry.scope().filter(|scope| validate_scope(scope).is_ok())
}

// Segments cut at `/`, which is what keeps the walk segment-wise rather than
// textual: the ancestors of `crates/engine-tools` are `crates` and `.`, and
// `crates/engine` is never among them however much of a prefix it looks like.
fn at_or_above(stored: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(stored);
    std::iter::from_fn(move || {
        let current = next?;
        next = match current.rsplit_once('/') {
            Some((parent, _)) => Some(parent),
            None if current == ROOT_MODULE => None,
            None => Some(ROOT_MODULE),
        };
        Some(current)
    })
}

/// ```
/// use warlock_engine::scope_opens_to;
///
/// let held = ["web".to_owned(), "billing".to_owned()];
///
/// // A directory nobody has scoped is open to anyone.
/// assert!(scope_opens_to(None, &held));
/// // Any one held sigil opens the scope it matches.
/// assert!(scope_opens_to(Some("billing"), &held));
/// // A scope none of them matches is closed.
/// assert!(!scope_opens_to(Some("data-plane"), &held));
/// // Holding nothing opens nothing that is scoped.
/// assert!(!scope_opens_to(Some("data-plane"), &[]));
/// // But an unscoped directory is still open to a machine holding nothing.
/// assert!(scope_opens_to(None, &[]));
/// // The wildcard sigil opens everything.
/// assert!(scope_opens_to(Some("data-plane"), &["*".to_owned()]));
/// ```
#[must_use]
pub fn scope_opens_to(covering: Option<&str>, held: &[String]) -> bool {
    let Some(scope) = covering else {
        // Nobody drew a boundary here, so there is none to be outside of.
        return true;
    };

    // No special case for an empty `held`, deliberately. It falls through to the
    // membership test and matches nothing, which is the answer: a sigil is what
    // opens a scope, so holding none opens none. Reading the empty set as
    // unrestricted — the symmetrical-looking counterpart to the `None` above —
    // would make the default state of every machine a universal bypass, and the
    // boundary would then hold only for people who had already opted into being
    // bound by it. Onboarding is where this is paid for instead.
    held.iter().any(|sigil| sigil == WILDCARD || sigil == scope)
}

fn is_scope_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || is_separator(character)
}

fn is_separator(character: char) -> bool {
    character == '-' || character == '_'
}

#[cfg(test)]
mod tests {
    use super::{
        MAXIMUM_CHARACTERS, RULES, Rule, at_or_above, is_scope_character, scope_covering,
        scope_opens_to, validate_scope, validate_sigil,
    };
    use crate::{Manifest, PactEntry};

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
}
