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
//! # What this module decides, and what it still does not
//!
//! A scope is a label. This module says whether a label is well formed, which
//! label a given path sits under ([`scope_covering`]), and whether that label is
//! open to somebody holding a given set of sigils ([`scope_opens_to`]). The
//! three together are the whole of the boundary as a *question*.
//!
//! What is still not here is the answer's consequence. Nothing in this crate
//! refuses anything, reports a crossing, or knows which verbs a closed boundary
//! stops: a caller asks the question and decides what to do about it, which is
//! how the same two functions serve a keystroke the TUI turns down and a diff a
//! non-interactive check will one day walk.

use std::fmt;
use std::path::Path;

use crate::manifest::{Error as ManifestError, Manifest, PactEntry, ROOT_MODULE, to_manifest_path};

/// The most characters a scope may hold.
///
/// A scope is drawn in a tree row after the directory name, so its ceiling is
/// how much of a terminal line it is worth spending on a label rather than on
/// the path. Twenty-four is comfortably longer than a team name and comfortably
/// shorter than a sentence.
const MAXIMUM_CHARACTERS: usize = 24;

/// The rules a scope keeps, as one line, for a prompt to show before anything
/// has been typed.
///
/// Here rather than wherever a scope is asked for, because it is a *statement of
/// [`validate_scope`]* and belongs beside it: a window that spelled the ceiling
/// or the character class out for itself would be a second copy of the rules,
/// free to drift from the one that actually judges, and the caller that showed
/// it would be judging a scope in the act of describing one. Every caller that
/// asks for a scope shows this same line, so the prompt, `warlock config` and
/// anything else that grows one all say the same thing.
///
/// The length and the character class, and not the two rules about the ends.
/// The line sits over a field somebody is about to type into, and what it is for
/// is that they type something acceptable first time; "begins with a letter" and
/// "does not end with `-` or `_`" are rules a name that follows this one breaks
/// about once in a hundred, and [`Rule`] has a line ready for the moment either
/// is broken. A hint long enough to need reading twice is not read once.
///
/// The maximum is spelled out rather than interpolated — a `const` cannot format
/// one — and a test below holds the two together, so a changed
/// [`MAXIMUM_CHARACTERS`] fails here rather than quietly leaving a window
/// promising a number the validator stopped keeping.
pub const RULES: &str = "1 to 24 characters: lowercase letters, digits, `-` and `_`";

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

/// The scope covering `path`: the one written on the nearest directory at or
/// above it that carries a valid one, or `None` if no such directory exists.
///
/// `path` may be a file or a directory, absolute or relative to `root`, and
/// `root` is the repository root the manifest's paths are relative to. Nothing
/// here touches the filesystem: `path` is put into the manifest's own
/// forward-slash form and walked upwards against the stored module paths, so a
/// path that does not exist on disk still answers, and a scope on
/// `crates/engine` does not cover `crates/engine-tools` — the walk is
/// segment-wise, like [`unpact_subtree`](crate::unpact_subtree)'s, never a
/// textual prefix.
///
/// # Nearest wins, and one answer only
///
/// The signature holds at most one scope, and that is the design rather than a
/// simplification to be relaxed later. Walking up stops at the first valid
/// scope: an inner scope replaces an outer one outright, and the outer one is a
/// *default* for everything below it that has said nothing — not a second gate
/// to also satisfy. There is no accumulating, no "needs both", and no list.
///
/// # An invalid scope is stepped over
///
/// A scope that [`validate_scope`] refuses reads as no scope, exactly as it
/// does everywhere else: the walk carries on past it to the next ancestor that
/// has a valid one, or off the top to `None`. So this never hands back a string
/// that is not a scope, and one typo in a hand-edited manifest widens the
/// boundary to its parent's rather than inventing a boundary nobody wrote.
///
/// # One home to ask coverage from
///
/// It exists so that callers have one home to ask from rather than three each
/// walking up the tree their own way. [`scope_opens_to`] is the half that reads
/// its answer, and the TUI's run keys are what pair them.
///
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
/// # Ok::<(), warlock_engine::ManifestError>(())
/// ```
///
/// # Errors
///
/// [`ManifestError::PathOutsideRoot`] or [`ManifestError::NonUtf8Path`] if
/// `path` has no manifest-relative form, i.e. it does not sit under `root` or
/// cannot be spelled as text. Such a path is not unscoped — it is a path this
/// manifest has nothing to say about at all, and saying so beats answering
/// "open to anyone" for a file that was never in the repository.
pub fn scope_covering(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
    manifest: &Manifest,
) -> Result<Option<&str>, ManifestError> {
    let stored = to_manifest_path(root, path)?;
    Ok(at_or_above(&stored).find_map(|module| valid_scope_on(manifest, module)))
}

/// The scope written on the module stored at `module`, if it has an entry, if
/// that entry carries a scope, and if that scope is one.
///
/// The three "if"s collapse to the same answer — `None`, meaning this directory
/// says nothing and the walk goes on — which is what makes an invalid scope
/// indistinguishable from an absent one to every caller.
fn valid_scope_on<'manifest>(
    manifest: &'manifest Manifest,
    module: &str,
) -> Option<&'manifest str> {
    valid_scope(manifest.entry(module)?)
}

/// The scope written on `entry`, if it carries one and that one is a scope.
///
/// The half of [`valid_scope_on`] that has an entry in hand already, split out
/// so that a caller walking the entries itself —
/// [`closed_scopes_at_or_below`](crate::closed_scopes_at_or_below), which
/// visits each entry once rather than looking each one up by module —
/// reads an invalid scope as no scope by calling *this* rule rather than by
/// writing a second copy of it.
pub(crate) fn valid_scope(entry: &PactEntry) -> Option<&str> {
    entry.scope().filter(|scope| validate_scope(scope).is_ok())
}

/// The stored path `stored` and every stored path above it, nearest first,
/// ending at [`ROOT_MODULE`].
///
/// Segments are cut at `/`, which is what keeps this segment-wise rather than
/// textual: the ancestors of `crates/engine-tools` are `crates` and `.`, and
/// `crates/engine` is never among them however much of a prefix it looks like.
fn at_or_above(stored: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(stored);
    std::iter::from_fn(move || {
        let current = next?;
        next = match current.rsplit_once('/') {
            // A path with a parent segment: `crates/engine` above
            // `crates/engine/src`.
            Some((parent, _)) => Some(parent),
            // A single segment sits directly under the root, and the root sits
            // under nothing.
            None if current == ROOT_MODULE => None,
            None => Some(ROOT_MODULE),
        };
        Some(current)
    })
}

/// Whether the scope `covering` a directory is open to somebody holding
/// `held`.
///
/// The matcher [`scope_covering`] was written to have one home for: coverage
/// says *which* boundary a path sits inside, and this says whether the person at
/// the keyboard is on the inside of it. Both halves are pure functions of their
/// arguments — no manifest walked twice, no config read, no keystroke — so the
/// one question warlock refuses work over is answered in a place a test can hold
/// up on its own.
///
/// # It is a membership test, never an expression
///
/// A directory carries at most one scope and a person may hold several sigils,
/// so any one held sigil that matches opens the boundary. There is nothing here
/// to evaluate: no AND across levels, no accumulation up the tree — the nearest
/// scope is the whole of what applies, which is [`scope_covering`]'s promise
/// rather than this function's — and no precedence to get wrong.
///
/// # The permissive default is on the directory, and only there
///
/// `None` is a directory that has said nothing, and it is open to anyone: an
/// unscoped directory is not a locked one, it is one nobody has drawn a boundary
/// on. That is the whole of the permissiveness, and it is why a repository that
/// has never scoped anything is unaffected by this function existing — every
/// path in it answers `None` and every key goes through.
///
/// An empty `held` is emphatically **not** the same principle from the other
/// end, and the temptation to make it one has to be resisted. A sigil is what
/// opens a scope; holding none opens none. A machine that has never run
/// `warlock config` is refused by every scoped directory it meets, exactly as a
/// machine holding the wrong sigil is, because "nobody told me what you hold" and
/// "what you hold does not match" are the same answer to the only question asked
/// here.
///
/// Reading the empty set as unrestricted would make the default state of every
/// machine a universal bypass, which is not a permissive default but the absence
/// of the feature: the boundary would hold for exactly those people who had
/// already opted into being bound by it. Onboarding is where this is paid for
/// instead — you are handed your scopes, you record them with `warlock config`,
/// and until you do a scoped repository does not open for you.
///
/// # The wildcard is a sigil and never a scope
///
/// `*` held means "may work anywhere" and opens every scope. There is
/// deliberately no matching case for a directory scoped `*`, because
/// [`validate_scope`] refuses the character: on the directory side "open to
/// anyone" is already spelled by having no scope at all, and two spellings of
/// one meaning is how a vocabulary rots. See [`validate_sigil`], where the
/// asymmetry is argued.
///
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

    // No special case for an empty `held`, deliberately: it falls through to the
    // membership test below and matches nothing, which is the answer. See above.
    held.iter().any(|sigil| sigil == WILDCARD || sigil == scope)
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
    use super::{
        MAXIMUM_CHARACTERS, RULES, Rule, at_or_above, is_scope_character, scope_covering,
        scope_opens_to, validate_scope, validate_sigil,
    };
    use crate::{Manifest, PactEntry};

    /// An entry for `module`, documented the way a pact would document it.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
    }

    /// The scope covering `path` in `manifest`, for paths the manifest can
    /// always spell.
    fn covering<'manifest>(path: &str, manifest: &'manifest Manifest) -> Option<&'manifest str> {
        scope_covering(path, ".", manifest).expect("a relative path is inside the root")
    }

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

    /// Sigils as `scope_opens_to` takes them.
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
