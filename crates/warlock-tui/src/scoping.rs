//! The `s` key, from the press to the saved `.warlock/pacts.toml`.
//!
//! Unlike a pact — the sibling to read this against, in [`mod@crate::pacting`] —
//! nothing here is a run. A scope is one string written into one entry of a file
//! already in this thread's hand, so [`scope_submit`] writes on the event loop's
//! own thread between two frames. It reloads nothing either: a scope changes no
//! directory's state and no row's colour, so re-reading the tree would walk the
//! whole repository to arrive at the tree already on screen.
//!
//! A successful write says nothing at all and leaves the message line as it
//! found it, across both a set and a clear. Whether a string is a scope is
//! [`validate_scope`]'s answer and nobody else's — no length constant and no
//! character predicate lives in this crate — and case is folded before asking,
//! so the string judged is the string stored. An empty field clears the scope
//! rather than being refused.

use std::iter;
use std::path::Path;

use warlock_engine::{Manifest, PactEntry, ScopeRecord, to_manifest_path, validate_scope};
use warlock_tui::{App, Edited, ScopeField, ScopePrompt, Sigils};

use crate::error::Error;
use crate::session::closed_scope;

// The three refusals are ordered deliberately, matching
// [`pact_press`](crate::pacting::pact_press): a run in flight, then the
// boundary, then whatever the app makes of the row. A refusal during a run
// goes to the progress line rather than the message line, because a run has
// taken the message line and a sentence left there is the one sentence the
// reader cannot see.
//
// The field opens on the scope read out of the manifest, never off a
// [`Row`](warlock_tui::Row): a fourth row field holding this string would be a
// copy of the manifest to keep in step with the manifest.
pub(crate) fn scope_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    in_flight: bool,
) -> ScopePrompt {
    if in_flight {
        // The whole of the refusal: a bit of wording on a line that is already
        // on screen. Setting it again says the same thing, so a reader leaning
        // on the key changes nothing after the first press.
        app.set_pact_refused();
        return ScopePrompt::Closed;
    }
    // The third refusal, and the sharpest of the three this key can meet: you
    // must hold a boundary to redraw it. Without this, the one key whose whole
    // purpose is to move a scope would be the one key a scope did not cover, and
    // a boundary anybody may retype is not a boundary. Before `scope_target` for
    // the reason `pact_press` puts it before the toggle — whether this operator
    // may act here is settled ahead of what the key would have done.
    if closed_scope(app, manifest, repo_root, sigils).is_some() {
        return ScopePrompt::Closed;
    }
    // Every row-level refusal leaves through here as `None`, having already put
    // its sentence on the app: there is one place that decides what this press
    // means over a row, and it is not this file.
    let Some(directory) = app.scope_target() else {
        return ScopePrompt::Closed;
    };

    let module = match to_manifest_path(repo_root, &directory) {
        Ok(module) => module,
        // The engine's own wording, flattened by `Error`'s `Display` as every
        // other manifest failure in this binary is.
        Err(source) => {
            app.set_message(Error::Manifest { source }.to_string());
            return ScopePrompt::Closed;
        }
    };

    let scope = manifest
        .entry(&module)
        .and_then(PactEntry::scope)
        .unwrap_or_default();
    ScopePrompt::open(module, scope)
}

// Typing and abandoning move nothing but the prompt: the app was never told
// the question was asked, so an Esc has nothing to put back.
//
// The `None` arm is unreachable rather than silent —
// [`press_for`](crate::input::press_for) only consults
// [`edit_for`](warlock_tui::edit_for) while a prompt is up.
pub(crate) fn scope_edit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    prompt: &ScopePrompt,
    edited: Edited,
) -> ScopePrompt {
    match edited {
        Edited::Open(field) => ScopePrompt::Open(field),
        Edited::Close => ScopePrompt::Closed,
        Edited::Submit => match prompt.field() {
            Some(field) => scope_submit(app, manifest, repo_root, field),
            None => ScopePrompt::Closed,
        },
    }
}

// Judge, then write. A refusal reopens the field over the text and cursor
// exactly as they were and touches no disk, which is what keeps
// `control-plane, data-plane` one refused string rather than two scopes
// somebody meant. An empty field is not judged at all: clearing is an answer.
//
// The manifest the loop holds is replaced only *after* the save succeeded, so
// what this thread believes is what is on disk. A save that fails is a line on
// the footer rather than a return out of the event loop — a manifest that will
// not write is news, not a reason to tear the screen down.
pub(crate) fn scope_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    field: &ScopeField,
) -> ScopePrompt {
    // `to_ascii_lowercase` rather than `to_lowercase`, for the reason
    // `config::sigils_in` gives: a scope is drawn from ASCII, so folding a
    // non-ASCII capital would produce a character the next line refuses anyway,
    // and this way what is refused is closer to what was typed.
    let typed = field.text().to_ascii_lowercase();
    let scope = if typed.is_empty() {
        None
    } else {
        match validate_scope(&typed) {
            Ok(()) => Some(typed),
            // The engine's sentence about the one rule that was broken, under
            // the field that broke it.
            Err(rule) => return ScopePrompt::Open(field.clone().refused(rule.to_string())),
        }
    };

    // The directory the window has been naming all along, which is the manifest
    // path `scope_press` read the entry by: one copy of it, so the entry that is
    // written cannot be a different one from the entry that was read.
    let module = field.directory();
    if manifest.entry(module).is_none() {
        app.set_message(no_pact_message(module));
        return ScopePrompt::Closed;
    }

    let next = with_scope_on(manifest, module, scope.as_deref());
    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return ScopePrompt::Closed;
    }
    *manifest = next;
    ScopePrompt::Closed
}

// A rebuild rather than a mutation, because [`Manifest`] has no mutating scope
// setter and should not grow one for this. [`Manifest::rebuilt_with`] and not
// `Manifest::with_entries`: that one starts from an empty manifest and would
// drop the `[[scope]]` records out of the file. Every other entry is cloned as
// it stands and the map preserves order, so the saved file differs from the one
// on disk in one place; the edited entry keeps its document, granted hash and
// granted timestamp, none of which are this edit's to move.
//
// A `module` no entry matches hands back a copy. No caller reaches that:
// [`scope_submit`] and [`Opened::scoped`](crate::edits) both refuse first.
//
// Shared with the headless `warlock scope add`/`remove` rather than copied,
// since a second rebuild would be a second chance to forget the above.
pub(crate) fn with_scope_on(manifest: &Manifest, module: &str, scope: Option<&str>) -> Manifest {
    manifest.rebuilt_with(manifest.entries().iter().map(|entry| {
        let entry = entry.clone();
        if entry.module() != module {
            return entry;
        }
        match scope {
            Some(scope) => entry.with_scope(scope),
            None => entry.without_scope(),
        }
    }))
}

// One value for the caller to save once: a scope half-written — set on the pact
// but routing to no record, or recorded with no pact naming it — is what two
// saves leave behind when the second one fails.
//
// `None` rather than a manifest when the name is already recorded, because the
// alternative is this function choosing between the record on disk and the one
// handed in, and every choice it could make edits a `[[scope]]` block somebody
// hand-wrote. Editing a record is not this crate's to do at all, so the answer
// is to hand nothing back and let the caller say so.
//
// The four record values are stored as passed; folding, trimming and judging
// all happened before the call, so what was judged is what is written.
pub(crate) fn with_scope_and_record_on(
    manifest: &Manifest,
    module: &str,
    name: &str,
    team: &str,
    review_state: &str,
    label: &str,
) -> Option<Manifest> {
    if records_scope(manifest, name) {
        return None;
    }
    let record = ScopeRecord::new(name, team, review_state, label);
    let scopes = manifest.scopes().iter().cloned().chain(iter::once(record));
    Some(with_scope_on(manifest, module, Some(name)).with_scopes(scopes))
}

// The same comparison [`route_facts`](warlock_engine::route_facts) routes by, so
// a name this answers `false` for is a name that would route to nothing.
pub(crate) fn records_scope(manifest: &Manifest, name: &str) -> bool {
    manifest.scopes().iter().any(|record| record.name() == name)
}

// Reachable only when the manifest was edited in another window since warlock
// read it — rows and manifest come from one load and otherwise agree. Said out
// loud rather than smoothed over, because a prompt that closed on a write that
// never happened is the one outcome a reader cannot tell from success.
fn no_pact_message(module: &str) -> String {
    format!(
        "`{module}` is not in the manifest, so there is no pact to write a scope on; press `p` to pact it"
    )
}

// The whole path from press to saved file, over a repository of the test's own.
// No terminal, no network, no `claude` and no worker thread: a scope write is a
// function of an app, a manifest and a field.
#[cfg(test)]
#[path = "tests/scoping.rs"]
mod tests;
