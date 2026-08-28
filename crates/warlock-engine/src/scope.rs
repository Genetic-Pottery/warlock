//! What a scope string may be.
//!
//! A scope is the string a pacted directory carries to say *whose* work it is
//! — `data-plane`, `billing`, `web` — committed in `.warlock/pacts.toml` beside
//! the module's document and its grant. This module is the one place in the
//! workspace that decides which strings qualify, so the scope prompt, the
//! manifest reader and `warlock config` all reach the same verdict and a
//! boundary cannot mean one thing when it is typed and another when it is read
//! back.
//!
//! The rules, in full: 1 to 24 characters; ASCII lowercase letters, digits,
//! `-` and `_`; begins with a letter; does not end with `-` or `_`. Short
//! enough to sit in a tree row beside a directory name, narrow enough that the
//! same scope cannot be written two ways, and dull enough to be a filename, a
//! shell word and a URL fragment all at once.
//!
//! # This judges, it does not normalise
//!
//! [`validate_scope`] takes a `&str` and hands back a verdict. It never
//! lower-cases, never trims, never substitutes, and never returns a `String`:
//! there is no repaired scope anywhere in this crate to be confused with the
//! one somebody wrote.
//!
//! That split is deliberate, and it puts folding where a person supplies the
//! string rather than here. The scope prompt and `warlock config` lower-case
//! what was typed and *then* ask, so `Data-Plane` and `data-plane` can never
//! become two stored boundaries. The manifest reader asks without folding,
//! because those bytes are committed: correcting them would put a line in
//! somebody's diff that nobody authored, on a save they only asked for because
//! they pacted something else. An invalid scope in a hand-edited manifest is
//! therefore reported and read as no scope, and left on disk exactly as it was
//! written.
//!
//! # Nothing here enforces anything
//!
//! A scope is a label. This module says whether a label is well formed; it has
//! no opinion on who may work where, and nothing in this crate matches a scope
//! against anything.

use std::fmt;

/// The most characters a scope may hold.
///
/// A scope is drawn in a tree row after the directory name, so its ceiling is
/// how much of a terminal line it is worth spending on a label rather than on
/// the path. Twenty-four is comfortably longer than a team name and comfortably
/// shorter than a sentence.
const MAXIMUM_CHARACTERS: usize = 24;

/// The wildcard sigil: "may work anywhere".
///
/// Spelled once, here, so the one place that accepts it and any future place
/// that means it cannot drift apart.
const WILDCARD: &str = "*";

/// Why a string is not a scope: the single rule it broke, in a form a caller
/// can render as one line.
///
/// One rule and not a list. Every caller that has a scope to reject — the
/// prompt refusing a submit, the loader recording a problem — has one line to
/// say it in, and the first rule broken is the one worth saying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// The string is empty. A directory with no scope is written by leaving the
    /// scope off, not by writing an empty one.
    Empty,
    /// The string is longer than [`MAXIMUM_CHARACTERS`].
    TooLong {
        /// How many characters it holds.
        characters: usize,
        /// The most it may hold, i.e. [`MAXIMUM_CHARACTERS`].
        maximum: usize,
    },
    /// The string holds a character outside ASCII lowercase letters, digits,
    /// `-` and `_`. That covers uppercase, spaces, commas, `*`, and every
    /// non-ASCII character.
    Character {
        /// The first character that is not allowed.
        character: char,
    },
    /// The string does not begin with an ASCII lowercase letter.
    Beginning {
        /// The character it begins with.
        character: char,
    },
    /// The string ends with `-` or `_`.
    Ending {
        /// The character it ends with.
        character: char,
    },
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

/// Whether `scope` is a scope a directory may carry: `Ok(())` if it is, and the
/// [`Rule`] it broke if it is not.
///
/// Judges only. The string comes back untouched or not at all — see the module
/// docs for why folding case belongs to the caller that took the input from a
/// person.
///
/// The rules are checked in the order a reader would read them, and the first
/// one broken is the one returned: empty, then too long, then the character
/// class, then the first character, then the last.
///
/// # Errors
///
/// The one rule broken, ready to render as a line.
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

/// Whether `sigil` is a sigil a person may hold: everything [`validate_scope`]
/// accepts, plus the wildcard `*`.
///
/// # Why the asymmetry exists
///
/// `*` is a sigil meaning "may work anywhere", and it has to be spellable
/// because a person holding nothing and a person holding everything are
/// different claims: the first is a machine nobody has configured yet, the
/// second is a deliberate "this one goes everywhere". On a directory the same
/// character would mean "open to anyone" — which blank already means, since a
/// directory with no scope is open. Two spellings of one meaning is how a
/// vocabulary rots, so the directory side takes the blank and refuses the star,
/// and the star lives here, on the side where it says something blank cannot.
///
/// This is a sibling entry point rather than a second set of rules: everything
/// but the wildcard is [`validate_scope`]'s answer, so there is still exactly
/// one character class in the workspace.
///
/// # Errors
///
/// The one rule broken, ready to render as a line. A near-miss wildcard —
/// `*.rs`, `**` — is refused as a character, because that is what it is.
pub fn validate_sigil(sigil: &str) -> Result<(), Rule> {
    if sigil == WILDCARD {
        return Ok(());
    }

    validate_scope(sigil)
}

/// Whether `character` may appear anywhere in a scope.
///
/// ASCII lowercase, digits and the two separators, and deliberately nothing
/// else: no uppercase (one scope, one spelling), no dot or slash (a scope is
/// not a path), no space or comma (a directory has one scope, not a list), and
/// no non-ASCII (this string is typed at a prompt, read out of a committed
/// file, and matched byte for byte).
fn is_scope_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || is_separator(character)
}

/// Whether `character` is one of the two separators a scope may hold inside it
/// but not end with.
fn is_separator(character: char) -> bool {
    character == '-' || character == '_'
}

#[cfg(test)]
mod tests {
    use super::{MAXIMUM_CHARACTERS, Rule, validate_scope, validate_sigil};

    /// A scope of exactly `characters` characters, all of them legal: a letter
    /// and then digits, so the beginning and ending rules are satisfied and
    /// length is the only thing under test.
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
}
