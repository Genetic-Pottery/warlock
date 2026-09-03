//! May this operator act here? Asked once, answered once, rendered twice.
//!
//! Warlock has two doors onto every write: the panel's keys and the headless
//! subcommands. Both have to refuse the same things for the same reasons, and
//! until now both worked the answer out for themselves — `session.rs` climbing
//! to the covering scope for the panel, `edits.rs` climbing to it again for the
//! shell, and each of them asking a second question about the scopes *below* a
//! directory in its own place and its own order. What held them together was a
//! shared sentence and one test that had to reach through a `#[cfg(test)]` hole
//! in another module to press both.
//!
//! So the question lives here now, and it is a function over values:
//! [`verdict`] takes a directory, a manifest, a repository root and what this
//! machine holds, and answers with a [`Verdict`]. Nothing in here reads the
//! disk, sets a message, returns an error or knows which door asked.
//!
//! ## The two questions, and why an un-pact asks both
//!
//! A scope covers everything beneath it until a nearer one overrides it, so
//! whether an operator may act *at* a directory is a question about the scopes
//! at and **above** it — the engine's
//! [`scope_covering`](warlock_engine::scope_covering). That is the whole of what
//! `p`, `r` and `s` need, and the whole of what the shell's cheap writes need.
//!
//! An un-pact needs one more, because of what it destroys. Un-pacting drops
//! every pact in the subtree, and a scope is a term of a pact rather than a
//! thing beside it — so it takes their scopes away with them. A directory that
//! is itself open may still sit above pacts that are not, and dropping those
//! would erase boundaries this operator was never entitled to move. That is the
//! engine's
//! [`closed_scopes_at_or_below`](warlock_engine::closed_scopes_at_or_below), and
//! it is asked only for [`Reach::HereAndBelow`].
//!
//! The two are genuinely different questions rather than one asked twice: the
//! first looks up, the second looks down, and neither answer implies the other.
//! What *was* duplicated is the order they are asked in, which is here now and
//! nowhere else: the scope covering the directory is the sharper thing to say
//! and is said first, because an operator who may not act here at all does not
//! need a list of what is underneath.
//!
//! ## The wording lives here too
//!
//! [`closed_scope_message`] and [`blocking_scopes_message`] are what the
//! panel's footer says. The shell says the same things through
//! [`Error`](crate::error::Error)'s `Display`, which calls these — so the two
//! doors cannot drift into wording the same refusal differently, and a test that
//! wants to prove they agree compares two renderings of one value instead of
//! translating one door's sentence into the other's.

use std::path::Path;

use warlock_engine::{Manifest, closed_scopes_at_or_below, scope_covering, scope_opens_to};
use warlock_tui::Sigils;

/// How far a boundary question reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// The directory itself: the scopes at and above it.
    ///
    /// What `p` (pacting), `r`, `s` and the shell's cheap writes ask, because
    /// none of them touches a pact anywhere but the one directory named.
    Here,
    /// The directory and everything under it.
    ///
    /// What an un-pact asks, in both doors, because it drops every pact in the
    /// subtree and their scopes with them.
    HereAndBelow,
}

/// Whether this operator may act on a directory, and what stands in the way when
/// they may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Nothing in the way: no scope covers the directory, or a held sigil opens
    /// the one that does.
    ///
    /// The permissive default sits here rather than on the operator: a pacted
    /// directory with no scope above it is open to anyone.
    Open,
    /// A scope covers the directory that no held sigil opens.
    Closed {
        /// The scope in the way, as the manifest spells it.
        scope: String,
    },
    /// The directory is open, but pacts under it are not.
    ///
    /// Only ever answered for [`Reach::HereAndBelow`], and only ever by an
    /// un-pact.
    ClosedBelow {
        /// The scopes in the way, in the engine's order, deduplicated by it.
        scopes: Vec<String>,
    },
}

impl Verdict {
    /// The one line the panel's footer says about this verdict, for a directory
    /// the reader knows as `label`, or `None` for one that does not refuse.
    ///
    /// The label is the caller's because it is a fact about the view: the panel
    /// spells a directory the way the tree on screen spells it, and the shell
    /// spells it the way the manifest does. What is *said* about it is the same
    /// sentence either way, which is the point of it being here.
    pub(crate) fn message(&self, label: &str) -> Option<String> {
        match self {
            Self::Open => None,
            Self::Closed { scope } => Some(closed_scope_message(label, scope)),
            Self::ClosedBelow { scopes } => {
                let named: Vec<&str> = scopes.iter().map(String::as_str).collect();
                Some(blocking_scopes_message(label, &named))
            }
        }
    }
}

/// May this operator act on `directory`, as far as `reach` looks?
///
/// The one place the question is decided. Both doors call it, neither adds to
/// it, and nothing about which door asked reaches in here.
///
/// A path with no manifest-relative spelling answers [`Verdict::Open`] rather
/// than refusing. It is not a boundary question — it takes a tree rooted outside
/// its own repository to reach — and every caller has a better sentence for it
/// than this one would invent. The engine's own calls refuse it again a moment
/// later, so nothing is let through by it.
pub(crate) fn verdict(
    directory: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    sigils: &Sigils,
    reach: Reach,
) -> Verdict {
    // `Nothing` and `Unknown` are both the empty slice on the way in
    // (`Sigils::as_slice`), which is what closes every scoped directory to a
    // machine that has never been configured and to one whose config will not
    // parse. An operator who has recorded nothing is refused by every scope,
    // exactly as one holding the wrong sigil is.
    let held = sigils.as_slice();

    let covering = scope_covering(directory, repo_root, manifest)
        .ok()
        .flatten();
    if !scope_opens_to(covering, held) {
        // `scope_opens_to` answers `true` for everything nothing covers, so a
        // refusal here is always a refusal by a named scope. Written out rather
        // than unwrapped, because the one thing this arm must never do is invent
        // a scope to refuse in the name of.
        if let Some(scope) = covering {
            return Verdict::Closed {
                scope: scope.to_owned(),
            };
        }
    }

    if reach == Reach::Here {
        return Verdict::Open;
    }

    let Ok(blocking) = closed_scopes_at_or_below(directory, repo_root, manifest, held) else {
        return Verdict::Open;
    };
    if blocking.is_empty() {
        return Verdict::Open;
    }
    Verdict::ClosedBelow {
        scopes: blocking.into_iter().map(str::to_owned).collect(),
    }
}

/// What warlock says when a directory is closed to this machine.
///
/// One sentence, said by the panel's footer and by the shell's error alike, and
/// it names the scope wanted rather than the sigils held: what is missing is the
/// thing to say, and what is held is `warlock config`'s to print.
pub(crate) fn closed_scope_message(label: &str, scope: &str) -> String {
    format!("{label} is scoped `{scope}` — hold that sigil to work here, with `warlock config`")
}

/// What warlock says when an un-pact would drop pacts this machine may not
/// touch.
///
/// A different sentence from [`closed_scope_message`] because it is a different
/// refusal: the directory named is open, and what is in the way is underneath
/// it. It names the way out that does not need a sigil at all — un-pact the
/// parts you hold — because that is usually what was meant.
pub(crate) fn blocking_scopes_message(label: &str, scopes: &[&str]) -> String {
    let named: Vec<String> = scopes.iter().map(|scope| format!("`{scope}`")).collect();
    // Singular for one, because the ordinary refusal is by a single boundary and
    // a line a person reads should not say "hold those sigils" about one of them.
    let sigils = if scopes.len() == 1 {
        "that sigil"
    } else {
        "those sigils"
    };

    format!(
        "un-pacting {label} would drop pacts scoped {} — hold {sigils} with `warlock config`, \
         or un-pact the parts you hold",
        named.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use warlock_engine::{Manifest, PactEntry};

    use super::{Reach, Verdict, verdict};
    use crate::error::Error;

    /// The repository every case below is asked about. Never read: nothing in
    /// this module touches a disk.
    const ROOT: &str = "/repo";

    /// A pacted entry on `module`, scoped `scope` when there is one.
    fn pact(module: &str, scope: Option<&str>) -> PactEntry {
        let entry = PactEntry::new(ROOT, module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root");
        match scope {
            Some(scope) => entry.with_scope(scope),
            None => entry,
        }
    }

    /// A manifest holding `entries`, each a module and the scope it carries.
    fn pacts(entries: &[(&str, Option<&str>)]) -> Manifest {
        Manifest::with_entries(entries.iter().map(|(module, scope)| pact(module, *scope)))
    }

    /// The directory `module` names, as this module's callers hand one over.
    fn at(module: &str) -> PathBuf {
        PathBuf::from(ROOT).join(module)
    }

    /// What a machine holding `sigils` is answered about `module`, at `reach`.
    fn asked(manifest: &Manifest, module: &str, sigils: &[&str], reach: Reach) -> Verdict {
        let held = Sigils::held(sigils.iter().copied());
        verdict(&at(module), &PathBuf::from(ROOT), manifest, &held, reach)
    }

    use warlock_tui::Sigils;

    #[test]
    fn a_directory_nothing_covers_is_open_to_anyone() {
        let manifest = pacts(&[("crates", None)]);

        assert_eq!(
            asked(&manifest, "crates", &[], Reach::Here),
            Verdict::Open,
            "the permissive default sits on the directory, not on the operator"
        );
    }

    #[test]
    fn a_scope_no_held_sigil_opens_closes_the_directory() {
        let manifest = pacts(&[("crates", Some("platform"))]);

        assert_eq!(
            asked(&manifest, "crates", &["web"], Reach::Here),
            Verdict::Closed {
                scope: "platform".to_owned()
            },
            "holding the wrong sigil is refused"
        );
        assert_eq!(
            asked(&manifest, "crates", &[], Reach::Here),
            Verdict::Closed {
                scope: "platform".to_owned()
            },
            "and so is holding none at all"
        );
        assert_eq!(
            asked(&manifest, "crates", &["platform"], Reach::Here),
            Verdict::Open,
            "the sigil that names the scope opens it"
        );
    }

    #[test]
    fn a_scope_covers_everything_beneath_it() {
        let manifest = pacts(&[("crates", Some("platform")), ("crates/engine", None)]);

        assert_eq!(
            asked(&manifest, "crates/engine", &[], Reach::Here),
            Verdict::Closed {
                scope: "platform".to_owned()
            },
            "a directory that says nothing takes the nearest scope above it"
        );
    }

    #[test]
    fn asking_about_here_says_nothing_about_what_is_underneath() {
        // The directory itself is open; the pact under it is not.
        let manifest = pacts(&[("crates", None), ("crates/engine", Some("platform"))]);

        assert_eq!(
            asked(&manifest, "crates", &[], Reach::Here),
            Verdict::Open,
            "`p`, `r` and `s` touch one directory and ask about one directory"
        );
        assert_eq!(
            asked(&manifest, "crates", &[], Reach::HereAndBelow),
            Verdict::ClosedBelow {
                scopes: vec!["platform".to_owned()]
            },
            "an un-pact drops the pacts underneath, so it has to ask about them"
        );
    }

    #[test]
    fn a_boundary_underneath_that_is_held_is_no_obstacle() {
        let manifest = pacts(&[("crates", None), ("crates/engine", Some("platform"))]);

        assert_eq!(
            asked(&manifest, "crates", &["platform"], Reach::HereAndBelow),
            Verdict::Open,
            "the rule refuses over scopes not held, not over having any"
        );
    }

    #[test]
    fn the_scope_above_is_answered_before_the_scopes_below() {
        // Closed both ways at once: a scope on the directory itself, and a
        // different one on a pact beneath it.
        let manifest = pacts(&[
            ("crates", Some("platform")),
            ("crates/engine", Some("data-plane")),
        ]);

        assert_eq!(
            asked(&manifest, "crates", &[], Reach::HereAndBelow),
            Verdict::Closed {
                scope: "platform".to_owned()
            },
            "an operator who may not act here at all does not need a list of \
             what is underneath — and this ordering used to live in two callers"
        );
    }

    #[test]
    fn both_doors_say_the_same_sentence_about_one_verdict() {
        let closed = Verdict::Closed {
            scope: "platform".to_owned(),
        };
        let panel = closed
            .message("crates")
            .expect("a refusing verdict has a line");
        let shell = Error::ClosedScope {
            path: "crates".to_owned(),
            scope: "platform".to_owned(),
        }
        .to_string();

        assert_eq!(
            panel, shell,
            "the footer and the shell word one refusal once"
        );
    }

    #[test]
    fn an_open_verdict_has_nothing_to_say() {
        assert_eq!(
            Verdict::Open.message("crates"),
            None,
            "a verdict that permits puts nothing on the footer"
        );
    }
}
