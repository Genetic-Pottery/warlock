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

use std::path::Path;

use warlock_engine::{Manifest, PactEntry, ScopeRecord, to_manifest_path, validate_scope};
use warlock_tui::{
    App, Edited, RecordField, RecordForm, RecordPrompt, ScopeField, ScopePrompt, Sigils,
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

// Judge, then write. A refusal reopens the field over the text and cursor
// exactly as they were and touches no disk, which is what keeps
// `control-plane, data-plane` one refused string rather than two scopes
// somebody meant. An empty field is not judged at all: clearing is an answer.
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
            Err(rule) => return Windows::asking(field.clone().refused(rule.to_string())),
        }
    };

    // The directory the window has been naming all along, which is the manifest
    // path `scope_press` read the entry by: one copy of it, so the entry that is
    // written cannot be a different one from the entry that was read.
    let module = field.directory();
    if manifest.entry(module).is_none() {
        app.set_message(no_pact_message(module));
        return Windows::closed();
    }

    // Ahead of the write and behind the two refusals above, so nobody fills in
    // three fields for a name that was never going to be stored.
    if let Some(scope) = scope.as_deref()
        && !records_scope(manifest, scope)
    {
        return Windows::recording(module, scope);
    }

    let next = with_scope_on(manifest, module, scope.as_deref());
    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return Windows::closed();
    }
    *manifest = next;
    Windows::closed()
}

// The other half of a submit that named a scope nothing records: three values
// and one save, of the pact's scope and the record together.
//
// Blank is the only thing judged, and it is judged on a trimmed copy while the
// untrimmed one is what gets stored — a team, a review state and a label belong
// to somebody's tracker, and warlock is in no position to correct their
// spelling. The fields are taken in `RecordField::ALL`'s order so that a form
// with two of them empty complains about the upper one, which is where the
// reader is already looking.
//
// `with_scope_recorded`'s `None` is a name already recorded, which `scope_submit`
// sends to the write road rather than here. It is said out loud rather than
// smoothed over for `no_pact_message`'s reason: a window that came down on a
// write that never happened is the one outcome a reader cannot tell from
// success.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_submit(
    app: &mut App,
    manifest: &mut Manifest,
    repo_root: &Path,
    form: &RecordForm,
) -> RecordPrompt {
    for which in RecordField::ALL {
        if form.field(which).text().trim().is_empty() {
            return RecordPrompt::Open(form.clone().refused(which, blank_message(which)));
        }
    }

    let Some(next) = with_scope_recorded(
        manifest,
        form.path(),
        form.scope(),
        form.field(RecordField::Team).text(),
        form.field(RecordField::ReviewState).text(),
        form.field(RecordField::Label).text(),
    ) else {
        app.set_message(already_recorded_message(form.scope()));
        return RecordPrompt::Closed;
    };

    if let Err(source) = next.save(repo_root) {
        app.set_message(Error::Manifest { source }.to_string());
        return RecordPrompt::Closed;
    }
    *manifest = next;
    RecordPrompt::Closed
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

// The same comparison [`route_facts`](warlock_engine::route_facts) routes by,
// and it has to stay that way: a lookup that folded, trimmed or matched loosely
// here would answer "no record" for a name `warlock check` then routes through,
// and [`scope_submit`] would put the record window up over a name that already
// routes, to write a second record the router never reads.
pub(crate) fn records_scope(manifest: &Manifest, name: &str) -> bool {
    manifest.scopes().iter().any(|record| record.name() == name)
}

// `None` is the refusal, and it is the only one: a name already recorded is
// handed back untouched rather than overwritten or merged, because editing and
// deleting records from warlock is not a thing this binary does — a record is
// hand-written prose about somebody's tracker, and the one destructive edit
// available here would be the one nobody asked for.
//
// `scope` is written in both places from the one string, so the pact cannot come
// to name a record spelled differently from the one this call created. Folding
// and validating happened in the caller (`scope_submit`,
// [`Opened::scoped`](crate::edits)); the three record values are passed to
// `ScopeRecord::new` exactly as given, which is what its own comment requires.
//
// Both halves of the write are one returned `Manifest` so the caller saves once:
// a scope on disk whose record failed to write is the half-state this exists to
// make impossible.
pub(crate) fn with_scope_recorded(
    manifest: &Manifest,
    module: &str,
    scope: &str,
    team: &str,
    review_state: &str,
    label: &str,
) -> Option<Manifest> {
    if records_scope(manifest, scope) {
        return None;
    }

    let recorded = manifest.scopes().iter().cloned().chain([ScopeRecord::new(
        scope,
        team,
        review_state,
        label,
    )]);

    Some(with_scope_on(manifest, module, Some(scope)).with_scopes(recorded))
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

// The engine's own wording about an empty scope — `a scope cannot be empty` —
// said about the field that is empty here.
fn blank_message(which: RecordField) -> String {
    format!("a {} cannot be blank", which.name())
}

// Reachable only from a manifest that gained the record between the two windows,
// which is not a thing the loop does to itself.
fn already_recorded_message(scope: &str) -> String {
    format!(
        "`{scope}` already has a record in `.warlock/pacts.toml`, and warlock does not rewrite one; edit the file to change it"
    )
}

// The whole path from press to saved file, over a repository of the test's own.
// No terminal, no network, no `claude` and no worker thread: a scope write is a
// function of an app, a manifest and a field.
#[cfg(test)]
#[path = "tests/scoping.rs"]
mod tests;
