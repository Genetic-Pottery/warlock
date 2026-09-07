//! One function over values, because the panel's keys and the headless
//! subcommands have to refuse the same things for the same reasons and both
//! used to work it out for themselves. Nothing in here reads the disk, sets a
//! message, returns an error or knows which door asked.
//!
//! The two questions are genuinely different rather than one asked twice.
//! Whether an operator may act *at* a directory looks up, at the scopes at and
//! above it; whether an un-pact may proceed looks down, because it drops every
//! pact in the subtree and a directory that is itself open may sit above
//! boundaries this operator was never entitled to move. Neither answer implies
//! the other; what *was* duplicated is the order they are asked in. The wording
//! lives here for the same reason, since the footer calls these functions and
//! `Error`'s `Display` calls them too.

use std::path::Path;

use warlock_engine::{Manifest, closed_scopes_at_or_below, scope_covering, scope_opens_to};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    // What `p` (pacting), `r`, `s` and the shell's cheap writes ask: none of
    // them touches a pact anywhere but the one directory named.
    Here,
    // What an un-pact asks, in both doors.
    HereAndBelow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    // The permissive default sits on the directory rather than on the
    // operator: a pacted directory with no scope above it is open to anyone.
    Open,
    Closed {
        scope: String,
    },
    // Only ever answered for `Reach::HereAndBelow`, and only ever to an
    // un-pact.
    ClosedBelow {
        scopes: Vec<String>,
    },
}

impl Verdict {
    // The label is the caller's because it is a fact about the view: the panel
    // spells a directory the way the tree on screen spells it, and the shell
    // spells it the way the manifest does. What is *said* about it is the same
    // sentence either way, which is the point of it being here.
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

// `held` is a plain slice rather than the header's `Sigils`, because the
// decision is two-valued — either a held sigil matches the covering scope or
// none does — and the header's third state, a config that would not parse, is a
// thing to *say* rather than a third answer to give. Taking the flattened fact
// is what keeps a change to the header from being a change to who may write.
//
// A path with no manifest-relative spelling answers `Verdict::Open` rather than
// refusing: it is not a boundary question, it takes a tree rooted outside its
// own repository to reach, and every caller has a better sentence for it than
// this one would invent. The engine's own calls refuse it again a moment later.
pub(crate) fn verdict(
    directory: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    held: &[String],
    reach: Reach,
) -> Verdict {
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

// Names the scope wanted rather than the sigils held: what is missing is the
// thing to say, and what is held is `warlock config`'s to print.
pub(crate) fn closed_scope_message(label: &str, scope: &str) -> String {
    format!("{label} is scoped `{scope}` — hold that sigil to work here, with `warlock config`")
}

// A different sentence from `closed_scope_message` because it is a different
// refusal: the directory named is open and what is in the way is underneath it.
// It names the way out that needs no sigil at all — un-pact the parts you hold —
// because that is usually what was meant.
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

    // Never read: nothing in this module touches a disk.
    const ROOT: &str = "/repo";

    fn pact(module: &str, scope: Option<&str>) -> PactEntry {
        let entry = PactEntry::new(ROOT, module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root");
        match scope {
            Some(scope) => entry.with_scope(scope),
            None => entry,
        }
    }

    fn pacts(entries: &[(&str, Option<&str>)]) -> Manifest {
        Manifest::with_entries(entries.iter().map(|(module, scope)| pact(module, *scope)))
    }

    fn at(module: &str) -> PathBuf {
        PathBuf::from(ROOT).join(module)
    }

    fn asked(manifest: &Manifest, module: &str, sigils: &[&str], reach: Reach) -> Verdict {
        let held: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        verdict(&at(module), &PathBuf::from(ROOT), manifest, &held, reach)
    }

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
