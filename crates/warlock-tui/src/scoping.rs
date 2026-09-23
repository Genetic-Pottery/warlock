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
//! found it, across both a set and a clear. Every rule a scope write follows is
//! [`rescope`]'s, shared with `warlock scope add` and `remove`; what this module
//! adds is where each refusal lands on screen. An empty field clears the scope
//! rather than being refused.

use std::path::Path;

use warlock_engine::{Manifest, PactEntry, to_manifest_path};
use warlock_tui::{
    App, Edited, RecordEdited, RecordField, RecordForm, RecordPrompt, ScopeField, ScopePrompt,
    Sigils,
};

use crate::boundary::Operation;
use crate::error::Error;
use crate::rescope::{RecordFields, ScopeRefusal, rescope};
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
    if closed_scope(app, Operation::Scope, manifest, repo_root, sigils) {
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

// Both windows in one value rather than a return each, because a submit of the
// first one can take it down and put the second one up in the same breath: two
// returns would let a caller apply half of that and leave the reader with both
// windows or neither. They are never both open — the record window opens
// exactly as the scope window closes — and nothing below writes a value where
// they are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Windows {
    pub(crate) scope: ScopePrompt,
    pub(crate) record: RecordPrompt,
}

impl Windows {
    pub(crate) const fn closed() -> Self {
        Self {
            scope: ScopePrompt::Closed,
            record: RecordPrompt::Closed,
        }
    }

    fn asking(field: ScopeField) -> Self {
        Self {
            scope: ScopePrompt::Open(field),
            record: RecordPrompt::Closed,
        }
    }

    fn recording(module: &str, scope: &str) -> Self {
        Self {
            scope: ScopePrompt::Closed,
            record: RecordPrompt::open(module, scope),
        }
    }
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
) -> Windows {
    match edited {
        Edited::Open(field) => Windows::asking(field),
        Edited::Close => Windows::closed(),
        Edited::Submit => match prompt.field() {
            Some(field) => scope_submit(app, manifest, repo_root, field),
            None => Windows::closed(),
        },
    }
}

// A refusal of what was typed reopens the field over the text and cursor
// exactly as they were and touches no disk. An empty field is a clear rather
// than a scope to judge.
//
// A name no `[[scope]]` record claims is not written here at all: it goes to the
// record window, and `record_submit` writes the scope and the record together.
// So this is the only place that decides which of the two roads a submit takes,
// and by the time the record window is up the name is known to be unrecorded.
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
) -> Windows {
    let typed = field.text();
    let scope = (!typed.is_empty()).then_some(typed);

    // The directory the window has been naming all along, which is the manifest
    // path `scope_press` read the entry by: one copy of it, so the entry that is
    // written cannot be a different one from the entry that was read.
    let module = field.directory();
    match rescope(manifest, module, scope, RecordFields::default()) {
        Ok(rescoped) => saved(app, manifest, repo_root, rescoped.manifest),
        // The engine's sentence about the one rule that was broken, under the
        // field that broke it.
        Err(ScopeRefusal::Rule { rule }) => {
            Windows::asking(field.clone().refused(rule.to_string()))
        }
        Err(ScopeRefusal::NeedsRecord { scope, .. }) => Windows::recording(module, &scope),
        Err(refusal) => {
            app.set_message(refusal.to_string());
            Windows::closed()
        }
    }
}

fn saved(app: &mut App, manifest: &mut Manifest, repo_root: &Path, next: Manifest) -> Windows {
    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return Windows::closed();
    }
    *manifest = next;
    Windows::closed()
}

// [`scope_edit`]'s counterpart for the second window, and the same three roads:
// typing and moving the focus move the window alone, Esc puts it down with
// nothing written — the app was never told the first window was answered
// either, so an abandoned record leaves no scope behind — and Enter is the one
// road to disk.
//
// The `None` arm is unreachable for `scope_edit`'s reason: `press_for` only
// consults [`record_edit_for`](warlock_tui::record_edit_for) while this window
// is up.
pub(crate) fn record_edit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    record: &RecordPrompt,
    edited: RecordEdited,
) -> RecordPrompt {
    match edited {
        RecordEdited::Open(form) => RecordPrompt::Open(form),
        RecordEdited::Close => RecordPrompt::Closed,
        RecordEdited::Submit => match record.form() {
            Some(form) => record_submit(app, manifest, repo_root, form),
            None => RecordPrompt::Closed,
        },
    }
}

// The other half of a submit that named a scope nothing records: three values
// and one save, of the pact's scope and the record together.
//
// A form with two fields blank is refused under the upper one, which is where
// the reader is already looking.
//
// A refusal said on the message line rather than smoothed over, because a
// window that came down on a write that never happened is the one outcome a
// reader cannot tell from success. Only a manifest edited in another window
// since warlock read it reaches one.
pub(crate) fn record_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    form: &RecordForm,
) -> RecordPrompt {
    let record = RecordFields {
        team: Some(form.field(RecordField::Team).text()),
        review_state: Some(form.field(RecordField::ReviewState).text()),
        label: Some(form.field(RecordField::Label).text()),
    };

    let next = match rescope(manifest, form.path(), Some(form.scope()), record) {
        Ok(rescoped) => rescoped.manifest,
        Err(ScopeRefusal::BlankRecord { fields }) => {
            let Some(&which) = fields.first() else {
                return RecordPrompt::Closed;
            };
            return RecordPrompt::Open(form.clone().refused(which, blank_message(which)));
        }
        Err(refusal) => {
            app.set_message(refusal.to_string());
            return RecordPrompt::Closed;
        }
    };

    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return RecordPrompt::Closed;
    }
    *manifest = next;
    RecordPrompt::Closed
}

// The engine's own wording about an empty scope — `a scope cannot be empty` —
// said about the field that is empty here.
fn blank_message(which: RecordField) -> String {
    format!("a {} cannot be blank", which.name())
}

// The whole path from press to saved file, over a repository of the test's own.
// No terminal, no network, no `claude` and no worker thread: a scope write is a
// function of an app, a manifest and a field.
#[cfg(test)]
#[path = "tests/scoping.rs"]
mod tests;
