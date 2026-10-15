//! The `s` key, from the press to the saved `.warlock/scopes.toml`.
//!
//! Nothing here is a run. A scope is one string written into one row of a file
//! already in this thread's hand, so [`scope_submit`] writes on the event loop's
//! own thread between two frames. Nothing here reloads the tree: the session
//! does that after a save, because it holds the watcher the reload is reported
//! to.
//!
//! A successful write says nothing at all and leaves the message line as it
//! found it, across both a set and a clear. Every rule a scope write follows is
//! [`rescope`]'s, shared with `warlock scope add` and `remove`; what this module
//! adds is where each refusal lands on screen. An empty field clears the scope
//! rather than being refused.
//!
//! The key window at the bottom of this file is the one thing here that does not
//! write the manifest: a binding is a fact about this machine, so it lands in
//! this checkout's config under the home directory and leaves
//! `.warlock/scopes.toml` alone. Everything else about it is [`record_submit`]'s
//! shape — Esc writes nothing, Enter is the one road to disk, and a refusal is
//! the engine's own sentence on the message line.

use std::path::Path;
use std::time::Instant;

use warlock_engine::{Manifest, ScopedDirectory, save_key_binding, to_manifest_path};

use crate::app::{App, Sigils};
use crate::error::Error;
use crate::prompt::{
    Edited, KeyChoice, KeyEdited, KeyPrompt, RecordEdited, RecordField, RecordForm, RecordPrompt,
    ScopeField, ScopePrompt,
};
use crate::rescope::{RecordFields, ScopeRefusal, rescope};
use crate::session::closed_scope;

// The three refusals are ordered deliberately: a pull in flight, then the
// boundary, then whatever the app makes of the row. A refusal during a pull goes
// on the thread, because it has to name the ticket whose session is editing this
// tree.
//
// The field opens on the scope read out of the manifest, never off a
// [`Row`](crate::app::Row): a fourth row field holding this string would be a
// copy of the manifest to keep in step with the manifest.
pub(crate) fn scope_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    pulling: Option<&str>,
    now: Instant,
) -> ScopePrompt {
    // Asked before the boundary, because a pull is a session editing this
    // working tree and a scope written under it would move the boundary that
    // run's own crossing check is about to be judged against.
    if let Some(pulling) = pulling {
        app.panel_mut().note(refused(pulling), now);
        return ScopePrompt::Closed;
    }
    // You must hold a boundary to redraw it. Before `scope_target`, because
    // whether this operator may act here is settled ahead of what the key would
    // have done.
    if closed_scope(app, manifest, repo_root, sigils) {
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
        .directory(&module)
        .map(ScopedDirectory::scope)
        .unwrap_or_default();
    ScopePrompt::open(module, scope)
}

// The key turned down for a pull, in the one sentence every keystroke that races
// one shares plus what this key did not do.
fn refused(pulling: &str) -> String {
    format!("{pulling}; no scope was written")
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
// [`edit_for`](crate::prompt::edit_for) while a prompt is up.
pub(crate) fn scope_edit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    prompt: &ScopePrompt,
    edited: Edited,
) -> Windows {
    match edited {
        Edited::Open(field) => Windows::asking(field),
        Edited::Close => Windows::default(),
        Edited::Submit => match prompt.field() {
            Some(field) => scope_submit(app, manifest, repo_root, field),
            None => Windows::default(),
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
    // path `scope_press` read the row by: one copy of it, so the row that is
    // written cannot be a different one from the row that was read.
    let module = field.directory();
    match rescope(manifest, module, scope, RecordFields::default()) {
        Ok(rescoped) => {
            save(app, manifest, repo_root, rescoped.manifest);
            Windows::default()
        }
        // The engine's sentence about the one rule that was broken, under the
        // field that broke it.
        Err(ScopeRefusal::Rule { rule }) => {
            Windows::asking(field.clone().refused(rule.to_string()))
        }
        Err(ScopeRefusal::NeedsRecord { scope, .. }) => Windows::recording(module, &scope),
        Err(refusal) => {
            app.set_message(refusal.to_string());
            Windows::default()
        }
    }
}

fn save(app: &mut App, manifest: &mut Manifest, repo_root: &Path, next: Manifest) {
    match next.save(repo_root) {
        Ok(()) => *manifest = next,
        Err(source) => app.set_message(Error::Manifest { source }.to_string()),
    }
}

// [`scope_edit`]'s counterpart for the second window, and the same three roads:
// typing and moving the focus move the window alone, Esc puts it down with
// nothing written — the app was never told the first window was answered
// either, so an abandoned record leaves no scope behind — and Enter is the one
// road to disk.
//
// The `None` arm is unreachable for `scope_edit`'s reason: `press_for` only
// consults [`record_edit_for`](crate::prompt::record_edit_for) while this window
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
// and one save, of the directory's scope and the record together.
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
        team_key: Some(form.field(RecordField::TeamKey).text()),
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

    save(app, manifest, repo_root, next);
    RecordPrompt::Closed
}

// The engine's own wording about an empty scope — `a scope cannot be empty` —
// said about the field that is empty here.
fn blank_message(which: RecordField) -> String {
    format!("a {} cannot be blank", which.name())
}

// The key window's three roads, and the same three the two windows above take:
// a move of the highlight moves the window alone, Esc puts it down with nothing
// written — the config is not opened at all on that road, so there is no binding
// to put back — and Enter is the one road to disk.
//
// The home and the root are parameters rather than read here, as everywhere else
// on this path: the loop resolved both once, and a second resolution down here
// could name a different checkout's config from the one the window was opened
// over.
//
// The `None` arm is unreachable for [`scope_edit`]'s reason: `press_for` only
// consults [`key_edit_for`](crate::prompt::key_edit_for) while this window is
// up.
pub(crate) fn key_edit(
    app: &mut App,
    home: &Path,
    root: &Path,
    prompt: &KeyPrompt,
    edited: KeyEdited,
) -> KeyPrompt {
    match edited {
        KeyEdited::Open(choice) => KeyPrompt::Open(choice),
        KeyEdited::Close => KeyPrompt::Closed,
        KeyEdited::Submit => match prompt.choice() {
            Some(choice) => key_submit(app, home, root, choice),
            None => KeyPrompt::Closed,
        },
    }
}

// One binding written for the scope the window was opened over and the row that
// is lit, both read off the one [`KeyChoice`]: a scope or a name taken from
// anywhere else could name a boundary this window was never about, and a
// binding filed under the wrong scope is work pushed into the wrong Linear
// workspace.
//
// No key value is read, held or written here, and there is none to read: the
// choice carries names, and the engine is handed the name. What is under that
// name stays in the store.
//
// A refusal is said on the message line and the window comes down, as
// [`record_submit`] has it: a window closing on a write that never happened is
// the one outcome a reader cannot tell from success. Both roads to one are the
// engine's — a scope or a name whose shape `validate_scope` turns down, and a
// config this machine cannot write — and the sentence is the engine's too,
// flattened by [`Error`]'s `Display` as every other sigils failure in this
// binary is.
//
// The `None` arm cannot be reached either: `key_edit_for` only answers `Submit`
// while a row is lit, which is what keeps a submit off an empty window.
pub(crate) fn key_submit(app: &mut App, home: &Path, root: &Path, choice: &KeyChoice) -> KeyPrompt {
    let Some(name) = choice.highlighted() else {
        return KeyPrompt::Closed;
    };

    if let Err(source) = save_key_binding(home, root, choice.scope(), name) {
        app.set_message(Error::Sigils { source }.to_string());
    }
    KeyPrompt::Closed
}

// The whole path from press to saved file, over a repository of the test's own.
// No terminal, no network, no `claude` and no worker thread: a scope write is a
// function of an app, a manifest and a field.
#[cfg(test)]
#[path = "tests/scoping.rs"]
mod tests;
