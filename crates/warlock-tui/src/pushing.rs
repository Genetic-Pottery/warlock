//! `/push`, from the command word to the question the reader answers.
//!
//! Everything here happens on the event loop's own thread between two frames
//! and opens no socket at all: which board a brief files to is a manifest, a
//! sigil file and a key store, all of them local, and the request the answer
//! may start is the loop after this one.
//!
//! [`resolve_filing`] is asked once per press and the boundary rule is nowhere
//! in this file — a sigil compared against a `[[scope]]` record here would be a
//! second copy of `scope_opens_to`, and it is the copy that would forget the
//! wildcard. Every refusal but the ambiguous one is the engine's own sentence,
//! put on the thread verbatim, so the key refusals stay worded in `route.rs`
//! where `warlock check` meets them first.
//!
//! No `Target` outlives a call below. It borrows the manifest and carries the
//! Linear key's value, so what the window keeps is the scope name that found
//! it, and a confirmed question resolves it again.

use std::path::{Path, PathBuf};
use std::time::Instant;

use warlock_engine::{Manifest, Target, filing, from_manifest_path, resolve_filing};
use warlock_tui::{App, Edited, PushConfirm, ScopeField, ScopePrompt, brief_at};

use crate::error::one_line;
use crate::standing::Standing;

pub(crate) const FILING_HEADING: &str = "Scope to file the brief to";

const NO_SCOPE: &str = "type the name of a scope to file to, or press Esc to file nothing";

// Both windows in one value, for `scoping::Windows`'s reason: a submit of the
// field is the very act that takes it down and puts the dialog up, and two
// returns would let a caller apply half of that. They are never both open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pushing {
    pub(crate) confirm: PushConfirm,
    pub(crate) field: ScopePrompt,
}

impl Pushing {
    pub(crate) const fn closed() -> Self {
        Self {
            confirm: PushConfirm::Closed,
            field: ScopePrompt::Closed,
        }
    }

    fn asking(field: ScopeField) -> Self {
        Self {
            confirm: PushConfirm::Closed,
            field: ScopePrompt::Open(field),
        }
    }

    fn confirming(confirm: PushConfirm) -> Self {
        Self {
            confirm,
            field: ScopePrompt::Closed,
        }
    }
}

pub(crate) fn push_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    written: &str,
    now: Instant,
) -> Pushing {
    let Some(home) = home_for(app, now) else {
        return Pushing::closed();
    };

    filing_to(app, manifest, repo_root, &home, written, None, now)
}

// Typing and abandoning move nothing but the field: nothing has been resolved
// and nothing has been sent, so an Esc has nothing to put back.
//
// `written` is asked of the session again rather than carried on the field,
// because there is one record of what `/write` wrote and this is not a second
// one. It is `None` only in a state the keys cannot reach — the field is up
// because a brief was written, and nothing can un-write one — and that reads
// as nothing to file rather than as a refusal to word.
pub(crate) fn push_edit(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    written: Option<&str>,
    prompt: &ScopePrompt,
    edited: Edited,
    now: Instant,
) -> Pushing {
    match edited {
        Edited::Open(field) => Pushing::asking(field),
        Edited::Close => Pushing::closed(),
        // An empty field is refused with a line of its own and before the home
        // is so much as looked for: the engine would answer about a scope named
        // nothing, and what is true is that the reader has not typed yet.
        Edited::Submit => match (prompt.field(), written) {
            (Some(field), _) if field.text().trim().is_empty() => {
                Pushing::asking(field.clone().refused(NO_SCOPE))
            }
            (Some(field), Some(written)) => match home_for(app, now) {
                Some(home) => filing_to(app, manifest, repo_root, &home, written, Some(field), now),
                None => Pushing::closed(),
            },
            _ => Pushing::closed(),
        },
    }
}

// `asked` is the field this answer came out of, and `None` is the `/push` that
// asked nobody anything. It is both the name the engine is handed and the field
// a refusal reopens over, so the text a reader is looking at and the text that
// was judged cannot come apart.
fn filing_to(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    home: &Path,
    written: &str,
    asked: Option<&ScopeField>,
    now: Instant,
) -> Pushing {
    let name = asked.map(|field| field.text().trim());
    match resolve_filing(manifest, repo_root, home, name) {
        Ok(target) => confirming(app, repo_root, written, &target, now),
        // The one sentence of the engine's this does not repeat: its own names
        // `--scope`, which is a flag on the subcommand and nothing a panel has,
        // and here the field that is about to open is the instruction.
        Err(filing::Error::Several { candidates }) => {
            Pushing::asking(ScopeField::new(FILING_HEADING, "").refused(pick_one(&candidates)))
        }
        // Back to the field with the candidates under it and the typing where
        // it was, one character from being right. The other arm cannot happen —
        // there is no unknown name without a name — and answers it the way
        // every other refusal is answered rather than by inventing a window.
        Err(error @ filing::Error::Unknown { .. }) => match asked {
            Some(field) => Pushing::asking(field.clone().refused(one_line(&error.to_string()))),
            None => refused(app, &error, now),
        },
        Err(error) => refused(app, &error, now),
    }
}

// The name is read off the brief here rather than from the path, because it is
// the name the project would be filed under: the file the reader is looking at
// is the document, and a title guessed from its filename would be a second
// opinion about what it is called.
fn confirming(
    app: &mut App,
    repo_root: &Path,
    written: &str,
    target: &Target<'_>,
    now: Instant,
) -> Pushing {
    let path = from_manifest_path(repo_root, written);
    match brief_at(repo_root, &path) {
        Ok(brief) => Pushing::confirming(PushConfirm::open(
            brief.name(),
            target.scope(),
            target.record().team(),
            // The key by name. `Target::value` is not read on this path at all.
            target.key(),
        )),
        Err(error) => saying(app, one_line(&error.to_string()), now),
    }
}

fn refused(app: &mut App, error: &filing::Error, now: Instant) -> Pushing {
    saying(app, one_line(&error.to_string()), now)
}

// On the thread and not the footer, where `/push`'s other refusal already
// goes: a command typed into the conversation is answered in the conversation.
fn saying(app: &mut App, line: impl Into<String>, now: Instant) -> Pushing {
    app.panel_mut().note(line, now);
    Pushing::closed()
}

// A home that will not resolve is a line and no window, rather than an error
// out of the event loop: warlock is running, the brief is still on disk, and
// what is missing is the directory the sigils and the key store sit under.
fn home_for(app: &mut App, now: Instant) -> Option<PathBuf> {
    match Standing::home() {
        Ok(home) => Some(home),
        Err(error) => {
            app.panel_mut().note(one_line(&error.to_string()), now);
            None
        }
    }
}

fn pick_one(candidates: &[String]) -> String {
    let named: Vec<String> = candidates
        .iter()
        .map(|candidate| format!("`{candidate}`"))
        .collect();
    format!("this machine can file to {}: type one", named.join(", "))
}

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pushing.rs"]
mod tests;
