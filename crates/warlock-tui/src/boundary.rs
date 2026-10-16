//! One function over values, because the panel's `s` key and `warlock scope
//! add` and `remove` have to refuse the same things for the same reasons.
//! Nothing in here reads the disk, sets a message, returns an error or knows
//! which door asked. The wording lives here too, since the footer calls these
//! functions and `Error`'s `Display` calls them as well.

use std::path::Path;

use warlock_engine::{Manifest, scope_covering, scope_opens_to};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Open,
    Closed { scope: String },
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
        }
    }
}

// Whether this operator may redraw the boundary at `directory`: write, change or
// clear the scope on it.
//
// A directory nothing covers is open here, though it is closed to every AI
// session: a scope is the only thing that opens a directory to work, so if
// drawing the first one also needed a scope, no repository could ever be set
// up. Once a scope covers a directory, only a machine that has that scope open
// may move it — a boundary anybody may retype is not a boundary.
//
// `held` is a plain slice rather than the panel's `OpenScopes`, because the
// decision is two-valued — either an open scope matches the covering scope or
// none does — and the panel's third state, a config that would not read, is a
// thing to *say* rather than a third answer to give.
//
// A path with no manifest-relative spelling answers `Verdict::Open` rather than
// refusing: it is not a boundary question, it takes a tree rooted outside its
// own repository to reach, and every caller has a better sentence for it than
// this one would invent. The engine's own calls refuse it again a moment later.
pub(crate) fn permits(
    directory: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    held: &[String],
) -> Verdict {
    let Some(scope) = scope_covering(directory, repo_root, manifest)
        .ok()
        .flatten()
    else {
        return Verdict::Open;
    };
    if scope_opens_to(Some(scope), held) {
        return Verdict::Open;
    }
    Verdict::Closed {
        scope: scope.to_owned(),
    }
}

// Names the scope wanted rather than the scopes open: what is missing is the
// thing to say, and what is open is the "AI can work in" box's to show.
pub(crate) fn closed_scope_message(label: &str, scope: &str) -> String {
    format!(
        "{label} is scoped `{scope}`, which is closed on this machine — `warlock open {scope}` lets AI work here"
    )
}

// No scope to name and so none to open: the way out is the one that draws a
// boundary rather than the one that opens it.
pub(crate) fn unscoped_message(label: &str) -> String {
    format!(
        "nothing scopes {label}, so no AI may work there — draw a scope with `warlock scope add`"
    )
}

#[cfg(test)]
#[path = "tests/boundary.rs"]
mod tests;
