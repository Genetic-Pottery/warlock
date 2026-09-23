//! One function over values, because the panel's keys and the headless
//! subcommands have to refuse the same things for the same reasons and both
//! used to work it out for themselves. Nothing in here reads the disk, sets a
//! message, returns an error or knows which door asked. A caller names the
//! operation it is about to perform and never how far the question reaches,
//! because a caller choosing its own reach is how an un-pact comes to be asked
//! only the narrow question.
//!
//! The two questions are genuinely different rather than one asked twice.
//! Whether an operator may act *at* a directory looks up, at the scopes at and
//! above it; whether an un-pact may proceed looks down as well, because it drops
//! every pact in the subtree and a directory that is itself open may sit above
//! boundaries this operator was never entitled to move. Neither answer implies
//! the other. The wording lives here too, since the footer calls these
//! functions and `Error`'s `Display` calls them as well.

use std::path::Path;

use warlock_engine::{Manifest, closed_scopes_at_or_below, scope_covering, scope_opens_to};

use crate::descent::Descent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Pact,
    Refresh,
    Scope,
    Unpact,
}

impl From<Descent> for Operation {
    fn from(descent: Descent) -> Self {
        match descent {
            Descent::Pact => Self::Pact,
            Descent::Refresh => Self::Refresh,
            Descent::Unpact => Self::Unpact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    // The permissive default sits on the directory rather than on the
    // operator: a pacted directory with no scope above it is open to anyone.
    Open,
    Closed { scope: String },
    // Only ever answered to `Operation::Unpact`.
    ClosedBelow { scopes: Vec<String> },
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
//
// The upward question is answered first for every operation, so an operator
// who may not act here at all is told that rather than handed a list of what is
// underneath. Only an un-pact goes on to look down: a pact, a refresh and a
// scope write leave every pact below the directory where they found it, and
// gating a root refresh on holding every sigil in a monorepo would refuse the
// ordinary gesture. See
// `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
pub(crate) fn permits(
    operation: Operation,
    directory: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    held: &[String],
) -> Verdict {
    let covering = scope_covering(directory, repo_root, manifest)
        .ok()
        .flatten();
    // `scope_opens_to` answers `true` for everything nothing covers, so a
    // refusal here is always a refusal by a named scope. Matched rather than
    // unwrapped, because the one thing this arm must never do is invent a scope
    // to refuse in the name of.
    if !scope_opens_to(covering, held)
        && let Some(scope) = covering
    {
        return Verdict::Closed {
            scope: scope.to_owned(),
        };
    }

    match operation {
        Operation::Pact | Operation::Refresh | Operation::Scope => Verdict::Open,
        Operation::Unpact => {
            let Ok(blocking) = closed_scopes_at_or_below(directory, repo_root, manifest, held)
            else {
                return Verdict::Open;
            };
            if blocking.is_empty() {
                return Verdict::Open;
            }
            Verdict::ClosedBelow {
                scopes: blocking.into_iter().map(str::to_owned).collect(),
            }
        }
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
#[path = "tests/boundary.rs"]
mod tests;
