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
//!
//! A name the manifest holds no `[[scope]]` record for asks a second question —
//! [`record_submit`] — before anything is written, because a scope with no
//! record routes work nowhere and `warlock check` on it cannot print a route.
//! The pact's scope and the new record then go down in one save, so there is no
//! moment on disk where the boundary exists and the route does not; an Esc in
//! between writes nothing at all.

use std::path::Path;

use warlock_engine::{Manifest, PactEntry, ScopeRecord, to_manifest_path, validate_scope};
use warlock_tui::{
    App, Edited, RecordAsk, RecordFields, RecordPrompt, Recorded, ScopeField, ScopePrompt, Sigils,
};

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

// Which window the `s` key leaves up, and never both: the record question is
// asked *instead of* the name, over the name it has already accepted, so two
// windows open at once is not a state the loop can be put in. A closed `s` has
// one spelling, `Asking::Scope(ScopePrompt::Closed)`, because the record
// variant carries its fields rather than a prompt of its own.
#[allow(
    clippy::large_enum_variant,
    reason = "`RecordFields`' own reason: three text fields, returned once per keystroke on the event loop's own thread, and a `Box` would buy an indirection between the answer and the text it is about"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asking {
    Scope(ScopePrompt),
    Record(RecordFields),
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
) -> Asking {
    match edited {
        Edited::Open(field) => Asking::Scope(ScopePrompt::Open(field)),
        Edited::Close => Asking::Scope(ScopePrompt::Closed),
        Edited::Submit => match prompt.field() {
            Some(field) => scope_submit(app, manifest, repo_root, field),
            None => Asking::Scope(ScopePrompt::Closed),
        },
    }
}

// The record window's half of [`scope_edit`], and the same three answers for
// the same reasons. It never reopens the name: the string it is recording was
// accepted by the window before it, and a second chance at typing it would be a
// second chance for the record's name and the pact's scope to disagree.
#[allow(
    dead_code,
    reason = "the event loop has no record window to route keys into yet; the slice that draws and wires it takes this off"
)]
pub(crate) fn record_edit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    prompt: &RecordPrompt,
    recorded: Recorded,
) -> RecordPrompt {
    match recorded {
        Recorded::Open(fields) => RecordPrompt::Open(fields),
        Recorded::Close => RecordPrompt::Closed,
        Recorded::Submit => match prompt.fields() {
            Some(fields) => record_submit(app, manifest, repo_root, fields),
            None => RecordPrompt::Closed,
        },
    }
}

// Judge, then write. A refusal reopens the field over the text and cursor
// exactly as they were and touches no disk, which is what keeps
// `control-plane, data-plane` one refused string rather than two scopes
// somebody meant. An empty field is not judged at all: clearing is an answer.
//
// The save and the belief that follows it are [`written`]'s, shared with
// [`record_submit`] so that the two roads through this file cannot disagree
// about what a failed write leaves behind.
pub(crate) fn scope_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    field: &ScopeField,
) -> Asking {
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
            Err(rule) => {
                return Asking::Scope(ScopePrompt::Open(field.clone().refused(rule.to_string())));
            }
        }
    };

    // The directory the window has been naming all along, which is the manifest
    // path `scope_press` read the entry by: one copy of it, so the entry that is
    // written cannot be a different one from the entry that was read.
    let module = field.directory();
    if manifest.entry(module).is_none() {
        app.set_message(no_pact_message(module));
        return Asking::Scope(ScopePrompt::Closed);
    }

    // Under the two refusals above and over the write, which is the only place
    // it can go: a name nobody may type here is refused before being asked
    // about, and a directory with no pact to carry it is refused before a
    // record is collected for a scope that would then be written onto nothing.
    // A clear asks nothing — `None` never reaches here — because there is no
    // name to record, and a name that already has a record asks nothing either:
    // a record is shared by every directory carrying that scope, so a second
    // question would be an offer to overwrite somebody else's route.
    if let Some(name) = scope.as_deref()
        && !recorded(manifest, name)
    {
        return Asking::Record(RecordFields::new(module, name));
    }

    written(app, manifest, repo_root, |manifest| {
        with_scope_on(manifest, module, scope.as_deref(), None)
    });
    Asking::Scope(ScopePrompt::Closed)
}

// Blank is the whole of the judgement, and it is this module's rather than the
// engine's: a team, a review state and a label are somebody else's vocabulary —
// Linear's — and warlock knows no list of review states to check one against.
// So what is stored is what was typed, to the byte, and the only string turned
// down is the one that would write a record routing work nowhere. Whitespace
// counts as blank because a field holding a space looks empty on screen and a
// reader would read the record as broken rather than as spelled that way.
//
// The three are judged in [`RecordAsk::ORDER`] — the order they are asked in —
// so a submit with all three blank complains about the first rather than about
// whichever arm happened to be written first.
pub(crate) fn record_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    fields: &RecordFields,
) -> RecordPrompt {
    for ask in RecordAsk::ORDER {
        if fields.field(ask).text().trim().is_empty() {
            return RecordPrompt::Open(fields.clone().refused(ask, blank_rule(ask)));
        }
    }

    // The same look the scope window took, taken again: the record window is a
    // second keystroke later, and a pact hand-edited out of the file in between
    // would otherwise be a record saved for a scope written onto nothing.
    let module = fields.module();
    if manifest.entry(module).is_none() {
        app.set_message(no_pact_message(module));
        return RecordPrompt::Closed;
    }

    let record = ScopeRecord::new(
        fields.name(),
        fields.field(RecordAsk::Team).text(),
        fields.field(RecordAsk::ReviewState).text(),
        fields.field(RecordAsk::Label).text(),
    );
    written(app, manifest, repo_root, |manifest| {
        with_scope_on(manifest, module, Some(fields.name()), Some(record))
    });
    RecordPrompt::Closed
}

fn blank_rule(ask: RecordAsk) -> String {
    format!("a {} cannot be blank", ask.heading())
}

// The record is looked up by exactly the string that goes onto the pact, which
// is the folded one: `resolve_route` matches a record to a scope by `==` on the
// name, so a record found here by any looser comparison would be a record
// `warlock check` then failed to find.
fn recorded(manifest: &Manifest, name: &str) -> bool {
    manifest.scopes().iter().any(|record| record.name() == name)
}

// One save, and the manifest the loop holds replaced only *after* it succeeded,
// so what this thread believes is what is on disk. A save that fails is a line
// on the footer rather than a return out of the event loop — a manifest that
// will not write is news, not a reason to tear the screen down.
//
// The rebuild is a closure rather than a `Manifest` the caller built, so there
// is no way to write one manifest and believe another: the value saved and the
// value stored are the one value, and it is built from the manifest that is
// about to be replaced.
fn written(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    rebuild: impl FnOnce(&Manifest) -> Manifest,
) {
    let next = rebuild(manifest);
    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return;
    }
    *manifest = next;
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
//
// `record` is appended to the records the manifest already holds and nothing
// there is looked at on the way past, so no existing record is merged with,
// overwritten or dropped — [`record_submit`] is the caller that decides whether
// a name wants one, and it only asks for a name no record matches. Appended
// rather than sorted in, for `rebuilt_with`'s own reason: the file that is
// saved should differ from the file that was read in the places this edit
// touched and nowhere else.
pub(crate) fn with_scope_on(
    manifest: &Manifest,
    module: &str,
    scope: Option<&str>,
    record: Option<ScopeRecord>,
) -> Manifest {
    let next = manifest.rebuilt_with(manifest.entries().iter().map(|entry| {
        let entry = entry.clone();
        if entry.module() != module {
            return entry;
        }
        match scope {
            Some(scope) => entry.with_scope(scope),
            None => entry.without_scope(),
        }
    }));

    match record {
        Some(record) => next.with_scopes(manifest.scopes().iter().cloned().chain([record])),
        None => next,
    }
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
