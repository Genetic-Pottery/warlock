//! The scope key, from the keystroke to the saved manifest.
//!
//! The other half of [`mod@crate::prompt`](warlock_tui::ScopePrompt): that
//! module says what a key does to a field, and this one says what a *press of
//! `s`* comes to and what an Enter in that field writes. [`scope_press`] decides
//! whether the window opens and what it opens holding, [`scope_edit`] is what
//! the loop does about a key typed into it, and [`scope_submit`] judges what was
//! typed and writes `.warlock/pacts.toml`.
//!
//! ## This is not a run, and that is the whole shape of it
//!
//! [`mod@crate::pacting`] is the sibling to read this against, because the
//! difference is the point. A pact is minutes of model passes, so it is a worker
//! thread, a channel, a say-when, an account, a progress line and a reload. A
//! scope is one string written into one entry of a file that is already in this
//! thread's hand: there is nothing to report on, nothing to cancel, and nothing
//! for a second thread to do except make the manifest reachable from two of
//! them. So [`scope_submit`] runs on the event loop's own thread, between two
//! frames, and spawns nothing.
//!
//! It reloads nothing either. A scope changes no directory's state and no row's
//! colour — it is a label on an entry — so re-reading the tree afterwards would
//! walk the whole repository to arrive at the tree already on screen.
//!
//! ## A successful write says nothing
//!
//! No message, no colour, no marker. Warlock states facts rather than narrating
//! its own actions, and the fact a scope write produces is the `(scope)` label
//! beside the directory — which belongs to a sibling slice and is not here yet.
//! Until it lands, Enter is deliberately invisible: the alternative is a sentence
//! invented to stand in for a label, which would have to be taken out again the
//! week the label arrives. What that costs is recorded rather than worked around.
//!
//! The corollary is that the message line is left exactly as it was found across
//! a set and a clear, which is the same promise [`App::scope_target`] makes about
//! the press: the reader who opened a window and wrote a label did not answer the
//! keystroke that put the last line up.
//!
//! ## One judge, and folding before it
//!
//! Whether a string is a scope is [`validate_scope`]'s answer and nobody else's:
//! there is no length constant and no character predicate anywhere in this crate.
//! What this module does before asking is fold case, because that is where
//! folding belongs — see the module docs of `warlock_engine::scope`. `Data-Plane`
//! and `data-plane` are one boundary, so the string that is judged and the string
//! that is stored are both the lower-cased one.
//!
//! Two answers are not refusals. An empty field *clears* the scope, which is how
//! a directory goes back to belonging to no particular boundary; and a refusal
//! keeps the prompt up over the text that earned it, one character away from
//! being fixed.

use std::path::Path;

use warlock_engine::{Manifest, PactEntry, to_manifest_path, validate_scope};
use warlock_tui::{App, Edited, ScopeField, ScopePrompt, Sigils};

use crate::error::Error;
use crate::session::closed_scope;

/// What one press of the scope key comes to, given whether a run is going
/// already: the prompt the event loop holds from here on.
///
/// [`pact_press`](crate::pacting::pact_press)'s shape for the third key, and
/// deliberately the same two refusals in the same two places. A press while
/// *any* run is in flight opens nothing and says so on the progress line the
/// reader is already watching, through
/// [`App::set_pact_refused`](warlock_tui::App::set_pact_refused) — the very
/// channel a second `p` and an `r` mid-run use, so a scope refused during a pact
/// and a pact refused during a refresh read alike. Deliberately not a message:
/// the message line is the one a run in flight has taken, so a sentence left
/// there would be the one sentence the reader could not see. A press the app
/// itself turns down — a file row, a directory that is not pacted — has no run
/// over it and has its say the ordinary way, in
/// [`App::message`](warlock_tui::App::message), which
/// [`App::scope_target`](warlock_tui::App::scope_target) has already written by
/// the time this returns.
///
/// What an accepted press opens on is the scope that directory carries *now*,
/// read out of the manifest this loop is holding: [`to_manifest_path`] spells
/// the row's path the way the manifest stores it, and [`Manifest::entry`]
/// answers. Never off a [`Row`](warlock_tui::Row) — a row knows a path, a state
/// and a document, and inventing a fourth field on it for a string only this
/// window reads would be a copy of the manifest to keep in step with the
/// manifest. A directory with no scope opens on an empty field, which is exactly
/// what a submit of an empty field then writes back: nothing.
///
/// A path with no manifest-relative spelling has nowhere to be written and so
/// opens nothing, and says why on the message line. It takes a tree rooted
/// outside its own repository to reach, which is a situation warlock cannot
/// currently be started in — but the alternative to answering it is a window
/// whose Enter has nowhere to go.
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

/// What one keystroke *inside* the open window comes to: the prompt the event
/// loop holds from here on.
///
/// [`edit_for`](warlock_tui::edit_for) has already said which of the three
/// things a key in a field can be; this is what the loop does about each, kept
/// here beside the write rather than spread across the match that dispatches
/// it. Typing and abandoning move nothing but the prompt — the app was never
/// told the question was asked, so an Esc has nothing to put back — and Enter is
/// [`scope_submit`] and only [`scope_submit`].
///
/// A closed prompt cannot submit: [`press_for`](crate::input::press_for) only
/// consults `edit_for` while one is up, so the `None` road below is unreachable
/// rather than silent. It answers with a closed prompt because a submit that
/// found no field to write is not a window anybody can still be typing into.
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

/// What Enter in the scope prompt comes to: the manifest written, or the prompt
/// still up over the reason it was not.
///
/// The one keystroke in this file that touches disk, and it touches it here, on
/// the event loop's own thread — see the module docs for why a scope is not a
/// run. What comes back is the prompt the loop holds next: [`ScopePrompt::Closed`]
/// for a submit that was answered one way or another, and an open prompt over the
/// same text for one the engine refused.
///
/// The order is judge, then write. [`validate_scope`] is asked about the
/// lower-cased text and its verdict decides the road: a rule broken reopens the
/// field through [`ScopeField::refused`] with the text and the cursor exactly
/// where they were, and nothing is written to disk — which is what keeps
/// `control-plane, data-plane` a single refused string rather than two scopes
/// somebody meant. An empty field is not judged at all, because clearing is an
/// answer rather than an error.
///
/// The entry is edited by rebuilding the manifest through
/// [`Manifest::with_entries`] over [`PactEntry::with_scope`] /
/// [`PactEntry::without_scope`], which is what keeps this to the one field a
/// person owns: every other entry is cloned untouched, and the edited one keeps
/// its document, its granted hash and its granted timestamp. Entry order is kept
/// too, so the saved file differs from the one on disk by the scope line and
/// nothing else.
///
/// The manifest in the loop's hand is replaced only *after* the save succeeded,
/// so what this thread believes is what is on disk. A save that fails puts its
/// reason on [`App::message`](warlock_tui::App::message) and never returns out of
/// the event loop, exactly as an un-pact that will not write does (see the
/// `Some(Err(message))` arm of
/// [`apply_progress`](crate::pacting::apply_progress)): a manifest that would not
/// write is news for the footer, not a reason to tear the screen down. The prompt
/// comes down with it, because the reason is a line on the very footer the window
/// is drawn over.
///
/// Nothing else moves. No thread, no channel, no account, no progress line, no
/// success message and no reload of the tree.
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

/// `manifest` with the entry for `module` carrying `scope`, or carrying none
/// when `scope` is `None`.
///
/// A rebuild rather than a mutation, because [`Manifest`] has no mutating scope
/// setter and should not grow one for this: the entries go through
/// [`PactEntry::with_scope`] and [`PactEntry::without_scope`], both of which are
/// the person's field and leave the grant alone, and every other entry is cloned
/// as it stands. Order is preserved by the map, so the file this saves to
/// differs from the one on disk in one place.
///
/// A `module` no entry matches hands back a copy of the manifest, and no caller
/// gets that far: [`scope_submit`] and
/// [`Opened::scoped`](crate::edits) each refuse such a write before reaching
/// here.
///
/// Shared with the headless writes rather than copied into them: `warlock scope
/// add` and `warlock scope remove` write the same field of the same file as the
/// keystroke above, and a second rebuild would be a second chance to forget that
/// the grant, the document and the entry order are not this edit's to move. The
/// sharing is a visibility and nothing else — what this does, and what
/// [`scope_submit`] does with it, are unchanged.
pub(crate) fn with_scope_on(manifest: &Manifest, module: &str, scope: Option<&str>) -> Manifest {
    Manifest::with_entries(manifest.entries().iter().map(|entry| {
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

/// What the footer says when the directory the prompt was opened over is not in
/// the manifest by the time Enter is pressed.
///
/// A scope is written on a pact's entry, so a directory with no entry has
/// nowhere to keep one. The app's rows and the manifest are derived from the
/// same load and agree in every ordinary session, so this is the answer to a
/// manifest hand-edited in another window since warlock read it — said out loud
/// rather than smoothed over, because a prompt that closed on a write that never
/// happened is the one outcome a reader cannot tell from success.
fn no_pact_message(module: &str) -> String {
    format!(
        "`{module}` is not in the manifest, so there is no pact to write a scope on; press `p` to pact it"
    )
}

/// What one press of the scope key and one Enter in the window it opens
/// actually do: which prompt the loop holds next, what ends up in
/// `.warlock/pacts.toml`, and what — deliberately — does not move at all.
///
/// The whole path is driven here, from the press to the saved file, over a
/// repository of the test's own under the temporary directory. No terminal, no
/// network, no `claude` and no worker thread: a scope write is a function of an
/// app, a manifest and a field, which is the point of the module above and is
/// what makes every rule below one assertion.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::{fs, io};

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tempfile::TempDir;
    use warlock_engine::{Manifest, Node, NodeState, PactEntry, Tree, manifest, validate_scope};
    use warlock_tui::{App, Edited, ScopeField, ScopePrompt, Sigils, edit_for};

    use super::{scope_edit, scope_submit};

    /// [`super::scope_press`] with no boundary in the way.
    ///
    /// The wildcard sigil, which opens every scope, so
    /// [`closed_scope`](crate::session::closed_scope) answers `None` and the
    /// press behaves exactly as it did before boundaries existed — which is what
    /// every test using this shadow is about. It is `*` rather than
    /// [`Sigils::Nothing`] because the fixture manifest *does* scope
    /// `crates/engine`, and holding nothing opens nothing that is scoped: an
    /// empty set here would turn a suite about the scope prompt into a suite
    /// about being refused. The tests that *are* about the boundary call
    /// `super::scope_press` directly.
    fn scope_press(
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        in_flight: bool,
    ) -> ScopePrompt {
        super::scope_press(app, manifest, repo_root, &Sigils::held(["*"]), in_flight)
    }

    /// The grant every entry below carries, so that "the write left the grant
    /// alone" is an assertion about two values that are really there.
    const HASH: &str = "d0f5a1";

    /// When that grant happened, in the form the manifest stores.
    const AT: &str = "2026-08-19T07:32:00Z";

    /// The line the last keystroke left on the footer, which no press of `s`
    /// and no Enter in its window is allowed to spend.
    const LAST_KEY: &str = "something the last key said";

    /// A repository of this test's own, removed when the test that made it
    /// ends. The tree below is written out by hand rather than loaded, so
    /// nothing is inside it until a scope is saved.
    fn a_repo() -> TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The tree the app is built over, rooted at `root`.
    ///
    /// Two pacted directories and one unpacted one, which is everything the
    /// press has to tell apart: `crates/engine` is fresh and carries a scope in
    /// the manifest below, `crates/tui` is stale and carries none, and `crates`
    /// itself is pacted by nothing.
    ///
    /// ```text
    /// <root>                      pacted, stale
    /// └── crates                  unpacted
    ///     ├── crates/engine       pacted, fresh
    ///     └── crates/tui          pacted, stale
    /// ```
    fn tree(root: &Path) -> Tree {
        Tree::new(
            Node::new(root, root.join("WARLOCK.md"), NodeState::PactedStale).with_children([
                Node::new(root.join("crates"), None::<PathBuf>, NodeState::Unpacted).with_children(
                    [
                        Node::new(
                            root.join("crates/engine"),
                            root.join("crates/engine/WARLOCK.md"),
                            NodeState::PactedFresh,
                        ),
                        Node::new(
                            root.join("crates/tui"),
                            root.join("crates/tui/WARLOCK.md"),
                            NodeState::PactedStale,
                        ),
                    ],
                ),
            ]),
        )
    }

    /// Which row of [`tree`] each directory is, in walk order.
    const ROOT_ROW: usize = 0;
    const ENGINE_ROW: usize = 2;
    const TUI_ROW: usize = 3;

    /// The app the event loop would hold for `root`, with `row` selected and a
    /// line on the footer from the keystroke before this one.
    fn app_on(root: &Path, row: usize) -> App {
        let mut app = App::from_tree(&tree(root));
        app.select_row(row);
        // After the selection, because moving the selection is what takes a
        // message down: every assertion below about the message line being left
        // alone needs there to be one.
        app.set_message(LAST_KEY);
        app
    }

    /// A granted entry for `module`, documented the way a pact documents it.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
            .with_grant(HASH, AT)
    }

    /// The manifest the loop holds: `crates/engine` scoped, `crates/tui` not.
    ///
    /// Named for what it holds rather than for its type, so that a test can
    /// have a `manifest` of its own to edit and still ask what the fixture said.
    fn pacts() -> Manifest {
        Manifest::with_entries([
            entry("crates/engine").with_scope("data-plane"),
            entry("crates/tui"),
        ])
    }

    /// The scope stored on `module`, or `None` for an entry with none.
    fn scope_on<'manifest>(manifest: &'manifest Manifest, module: &str) -> Option<&'manifest str> {
        manifest
            .entry(module)
            .expect("the fixture manifest holds this module")
            .scope()
    }

    /// The manifest as it sits on disk under `root`, or `None` when none was
    /// ever written there.
    fn saved(root: &Path) -> Option<Manifest> {
        match Manifest::load(root) {
            Ok(manifest) => Some(manifest),
            Err(manifest::Error::NotFound { .. }) => None,
            Err(error) => panic!("the saved manifest could not be read: {error}"),
        }
    }

    /// A field over `module` holding `text`, as the window would be by the time
    /// Enter is pressed.
    fn field(module: &str, text: &str) -> ScopeField {
        ScopeField::new(module, text)
    }

    /// The engine's own sentence about why `text` is not a scope: the one that
    /// belongs under the field, asked of the one judge rather than spelled out
    /// here.
    fn refusal(text: &str) -> String {
        validate_scope(text)
            .expect_err("this text is not a scope")
            .to_string()
    }

    /// A plain press of `code`, as crossterm reports one.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// What the footer says when the boundary turns `s` down over
    /// `crates/engine`, which the fixture scopes `data-plane`.
    const CLOSED: &str = "crates/engine is scoped `data-plane` — hold that sigil to work here, with `warlock config`";

    #[test]
    fn s_is_refused_on_a_directory_whose_scope_this_machine_does_not_hold() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let before = app.clone();

        let prompt = super::scope_press(
            &mut app,
            &pacts(),
            repo.path(),
            &Sigils::held(["web"]),
            false,
        );

        // You must hold a boundary to redraw it. Without this the one key whose
        // whole purpose is to move a scope would be the one key a scope did not
        // cover.
        assert_eq!(prompt, ScopePrompt::Closed, "the window must not open");
        assert_eq!(
            app.message(),
            Some(CLOSED),
            "the refusal names the scope wanted and where a sigil is recorded"
        );
        // Saying so is the whole of it: the same app with the new line on it and
        // nothing else moved — no colour, no selection, no account started.
        let said = {
            let mut said = before.clone();
            said.set_message(CLOSED);
            said
        };
        assert_eq!(app, said, "the refusal did more to the app than say so");
    }

    #[test]
    fn holding_a_matching_sigil_opens_the_prompt() {
        let repo = a_repo();

        for sigils in [
            Sigils::held(["data-plane"]),
            Sigils::held(["web", "data-plane"]),
            Sigils::held(["*"]),
        ] {
            let mut app = app_on(repo.path(), ENGINE_ROW);

            let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &sigils, false);

            assert_eq!(
                prompt,
                ScopePrompt::open("crates/engine", "data-plane"),
                "{sigils:?} opens `data-plane`"
            );
            assert_eq!(app.message(), Some(LAST_KEY), "{sigils:?} said something");
        }
    }

    #[test]
    fn a_machine_that_holds_no_sigil_is_refused_by_a_scoped_directory() {
        let repo = a_repo();

        for sigils in [
            // Nobody has run `warlock config` here. A sigil is what opens a
            // scope, so holding none opens none — the empty set is not a
            // universal bypass, or the boundary would hold for exactly those
            // people who had already opted into being bound by it.
            Sigils::Nothing,
            // And a config that will not parse cannot establish that anything is
            // held, which is the same answer. The header says `holding unknown`
            // for as long as it lasts, so the two are told apart on screen.
            Sigils::Unknown,
        ] {
            let mut app = app_on(repo.path(), ENGINE_ROW);

            let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &sigils, false);

            assert_eq!(
                prompt,
                ScopePrompt::Closed,
                "{sigils:?} opened `data-plane`"
            );
            assert_eq!(
                app.message(),
                Some(CLOSED),
                "{sigils:?} said the wrong thing"
            );
        }
    }

    #[test]
    fn an_unscoped_directory_stays_open_to_a_machine_holding_nothing() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);

        // The permissive default lives on the directory and only there, which is
        // what keeps a repository that has never scoped anything unaffected by
        // boundaries existing at all.
        let prompt = super::scope_press(&mut app, &pacts(), repo.path(), &Sigils::Nothing, false);

        assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
    }

    #[test]
    fn a_directory_no_scope_covers_is_open_to_a_machine_holding_something_else() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);

        // `crates/tui` carries no scope and nothing above it does either, so
        // there is no boundary here to be outside of.
        let prompt = super::scope_press(
            &mut app,
            &pacts(),
            repo.path(),
            &Sigils::held(["data-plane"]),
            false,
        );

        assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
    }

    #[test]
    fn an_inner_scope_replaces_the_outer_one_rather_than_adding_to_it() {
        let repo = a_repo();
        let manifest = Manifest::with_entries([
            entry("crates/engine").with_scope("data-plane"),
            entry("crates/tui"),
            entry(".").with_scope("platform"),
        ]);

        // Holding the outer scope alone does not reach the inner one: an outer
        // scope is a default for what said nothing below it, never a second gate
        // that `platform` could satisfy on `data-plane`'s behalf.
        let mut app = app_on(repo.path(), ENGINE_ROW);
        assert_eq!(
            super::scope_press(
                &mut app,
                &manifest,
                repo.path(),
                &Sigils::held(["platform"]),
                false
            ),
            ScopePrompt::Closed
        );

        // And the outer scope does cover the directory that said nothing.
        let mut app = app_on(repo.path(), TUI_ROW);
        assert_eq!(
            super::scope_press(
                &mut app,
                &manifest,
                repo.path(),
                &Sigils::held(["platform"]),
                false
            ),
            ScopePrompt::open("crates/tui", "")
        );
    }

    #[test]
    fn a_run_in_flight_is_answered_before_the_boundary_is() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        app.set_pact_in_flight("/repo/crates/engine", 1, 2);

        let prompt = super::scope_press(
            &mut app,
            &pacts(),
            repo.path(),
            &Sigils::held(["web"]),
            true,
        );

        // Both refusals apply; the in-flight one is the one on screen. It goes
        // on the progress line the reader is already watching, and the message
        // line the run has taken is left alone — a boundary sentence written
        // there would be the one sentence they could not see.
        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(
            app.message(),
            Some(LAST_KEY),
            "the boundary spoke over a run's own line"
        );
    }

    #[test]
    fn s_on_a_pacted_row_opens_the_prompt_on_the_scope_that_directory_carries() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let before = app.clone();

        let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

        // Over the directory as the manifest spells it, holding what is true
        // right now — so an Enter pressed straight afterwards writes back what
        // was already there rather than clearing it.
        assert_eq!(prompt, ScopePrompt::open("crates/engine", "data-plane"));
        assert_eq!(
            app, before,
            "opening the window changed something about the view"
        );
    }

    #[test]
    fn esc_closes_the_prompt_and_leaves_the_app_exactly_as_it_was() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let before = app.clone();

        let prompt = scope_press(&mut app, &pacts(), repo.path(), false);
        let field = prompt.field().expect("the prompt is up").clone();

        // The loop's own Esc arm, through the pure function that decides it:
        // the prompt comes down and nothing else is done, because there is
        // nothing to put back.
        assert_eq!(edit_for(press(KeyCode::Esc), &field), Edited::Close);
        assert_eq!(app, before, "Esc left something behind");
        assert_eq!(app.message(), Some(LAST_KEY), "and it kept the last line");
    }

    #[test]
    fn a_directory_carrying_no_scope_opens_on_an_empty_field() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);

        let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

        assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));
    }

    #[test]
    fn a_pacted_directory_the_manifest_has_no_entry_for_opens_on_an_empty_field() {
        // The root of this fixture is pacted on screen and absent from the
        // manifest, which is what a hand-edited file in another window leaves.
        // The window still opens — the reader asked about the row in front of
        // them — and holds nothing, because nothing is what is stored.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ROOT_ROW);

        let prompt = scope_press(&mut app, &pacts(), repo.path(), false);

        assert_eq!(prompt, ScopePrompt::open(".", ""));
    }

    #[test]
    fn the_field_holds_what_the_manifest_says_and_never_anything_off_the_row() {
        // The same app and the same row against two manifests: what is in the
        // field moves with the manifest, which it could not do if it were being
        // read off the tree.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let elsewhere = Manifest::with_entries([entry("crates/engine").with_scope("web")]);

        assert_eq!(
            scope_press(&mut app, &pacts(), repo.path(), false),
            ScopePrompt::open("crates/engine", "data-plane")
        );
        assert_eq!(
            scope_press(&mut app, &elsewhere, repo.path(), false),
            ScopePrompt::open("crates/engine", "web")
        );
    }

    #[test]
    fn a_press_while_a_run_is_in_flight_opens_nothing_and_says_so_on_the_progress_line() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        // The run the press is going to bounce off, as the event loop would
        // have told the app about it.
        app.set_pact_in_flight(repo.path().join("crates/tui"), 3, 12);
        let before = app.clone();

        let prompt = scope_press(&mut app, &pacts(), repo.path(), true);

        assert_eq!(prompt, ScopePrompt::Closed, "no window over a running pact");
        // Saying so is the whole of it: the same app with the flag set and
        // nothing else moved, exactly as a second `p` leaves it.
        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
        assert!(
            app.pact_line()
                .is_some_and(|line| line.ends_with("— already running")),
            "the refusal is worded onto the line the reader is watching: {:?}",
            app.pact_line()
        );
        assert_eq!(
            app.message(),
            Some(LAST_KEY),
            "the refusal did not go through the message"
        );
    }

    #[test]
    fn leaning_on_the_scope_key_during_a_run_changes_nothing_further() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        app.set_pact_in_flight(repo.path().join("crates/tui"), 3, 12);

        assert_eq!(
            scope_press(&mut app, &pacts(), repo.path(), true),
            ScopePrompt::Closed
        );
        let after_one = app.clone();

        assert_eq!(
            scope_press(&mut app, &pacts(), repo.path(), true),
            ScopePrompt::Closed
        );
        assert_eq!(app, after_one, "pressing again changed something");
    }

    #[test]
    fn a_directory_with_no_manifest_spelling_opens_nothing_and_says_why() {
        // A tree rooted outside its own repository: nothing on disk, because
        // whether a path can be spelled relative to a root is arithmetic.
        let mut app = App::from_tree(&Tree::new(Node::new(
            "/outside/thing",
            "/outside/thing/WARLOCK.md",
            NodeState::PactedFresh,
        )));

        let prompt = scope_press(&mut app, &Manifest::new(), Path::new("/repo"), false);

        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(
            app.message().is_some_and(|line| !line.is_empty()),
            "a window with nowhere to write said nothing"
        );
    }

    #[test]
    fn enter_sets_the_scope_and_leaves_the_document_and_the_grant_alone() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();

        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", "billing"),
        );

        assert_eq!(prompt, ScopePrompt::Closed, "the window is answered");
        let written = saved(repo.path()).expect("the submit wrote the manifest");
        assert_eq!(
            written, manifest,
            "what is on disk is what this thread believes"
        );
        assert_eq!(scope_on(&written, "crates/engine"), Some("billing"));

        // The one field a person owns, and nothing else on the entry: the run's
        // fields are exactly as they were, and so is the entry beside it.
        let engine = written
            .entry("crates/engine")
            .expect("the entry is still there");
        assert_eq!(engine.document(), "crates/engine/WARLOCK.md");
        assert_eq!(engine.granted_hash(), Some(HASH));
        assert_eq!(engine.granted_at(), Some(AT));
        assert_eq!(written.entry("crates/tui"), pacts().entry("crates/tui"));
        // And the order the file is written in, so the diff is the scope line.
        assert_eq!(
            written
                .entries()
                .iter()
                .map(PactEntry::module)
                .collect::<Vec<_>>(),
            ["crates/engine", "crates/tui"]
        );
    }

    #[test]
    fn enter_on_an_empty_field_clears_the_scope_and_keeps_the_grant() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();

        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", ""),
        );

        assert_eq!(prompt, ScopePrompt::Closed);
        let written = saved(repo.path()).expect("the submit wrote the manifest");
        assert_eq!(written, manifest);
        assert_eq!(
            scope_on(&written, "crates/engine"),
            None,
            "an empty field clears the scope rather than storing one"
        );
        let engine = written
            .entry("crates/engine")
            .expect("the entry is still there");
        assert_eq!(engine.granted_hash(), Some(HASH));
        assert_eq!(engine.granted_at(), Some(AT));
    }

    #[test]
    fn a_set_and_a_clear_leave_the_run_state_and_the_message_line_untouched() {
        // The whole of what a scope write is *not*: no worker thread to be in
        // flight, no progress line, no account, no success message, and not
        // even the last keystroke's sentence spent. The app is compared whole,
        // which covers every one of those at once.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();
        let before = app.clone();

        for text in ["billing", ""] {
            let prompt = scope_submit(
                &mut app,
                &mut manifest,
                repo.path(),
                &field("crates/engine", text),
            );

            assert_eq!(prompt, ScopePrompt::Closed);
            assert!(!app.is_pacting(), "writing {text:?} started a run");
            assert_eq!(app.pact_line(), None, "and put a progress line up");
            assert!(app.account().is_none(), "and opened an account");
            assert_eq!(app.message(), Some(LAST_KEY), "and took the message line");
            assert_eq!(app, before, "writing {text:?} moved the view");
        }
    }

    #[test]
    fn input_the_engine_refuses_keeps_the_prompt_up_and_writes_nothing() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();
        let before = app.clone();
        let typed = field("crates/engine", "Control Plane");

        let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

        // The text and the cursor exactly where they were, one character away
        // from being fixed, with the broken rule under the field.
        assert_eq!(
            prompt,
            ScopePrompt::Open(typed.refused(refusal("control plane"))),
        );
        assert_eq!(saved(repo.path()), None, "a refusal wrote to disk");
        assert_eq!(manifest, pacts(), "a refusal edited the manifest");
        assert_eq!(app, before, "a refusal moved the view");
    }

    #[test]
    fn a_list_typed_into_the_field_is_refused_and_never_stored_as_two_scopes() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();
        let typed = field("crates/engine", "control-plane, data-plane");

        let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &typed);

        // The character rule, and it is the engine's own sentence: a directory
        // has one scope, so the comma is what is refused rather than a
        // separator warlock would have split on.
        assert_eq!(
            prompt,
            ScopePrompt::Open(typed.refused(refusal("control-plane, data-plane"))),
        );
        assert_eq!(saved(repo.path()), None);
        assert_eq!(
            manifest.entries().len(),
            2,
            "the list became a second entry"
        );
        assert_eq!(scope_on(&manifest, "crates/engine"), Some("data-plane"));
    }

    #[test]
    fn what_was_typed_is_folded_before_it_is_judged_and_stored() {
        // `Data-Plane` and `data-plane` are one boundary, not two, and folding
        // belongs here rather than in the validator — which refuses a capital
        // outright, as the assertion below its sibling shows.
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = pacts();

        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/tui", "Data-Plane"),
        );

        assert_eq!(
            prompt,
            ScopePrompt::Closed,
            "the fold happened before the judge"
        );
        assert!(validate_scope("Data-Plane").is_err());
        assert_eq!(scope_on(&manifest, "crates/tui"), Some("data-plane"));
        assert_eq!(
            scope_on(&saved(repo.path()).expect("the submit wrote"), "crates/tui"),
            Some("data-plane"),
        );
    }

    #[test]
    fn a_manifest_that_will_not_save_puts_its_reason_on_the_message_line() {
        // A file where `.warlock/` has to be: the save cannot make the
        // directory, which is the cheapest real version of a disk that will not
        // take the write.
        let repo = a_repo();
        fs::write(repo.path().join(".warlock"), "not a directory\n")
            .expect("writes a file in the way of the manifest directory");
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();

        let prompt = scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", "billing"),
        );

        // The reason goes on the footer and the window comes down off it; the
        // manifest this thread holds still says what is on disk, which is that
        // the scope was not changed.
        assert_eq!(prompt, ScopePrompt::Closed);
        assert!(
            app.message().is_some_and(|line| !line.is_empty()),
            "a save that failed said nothing"
        );
        assert_ne!(app.message(), Some(LAST_KEY), "and said it here");
        assert_eq!(manifest, pacts(), "the failed write was believed");
        assert_eq!(
            fs::read_to_string(repo.path().join(".warlock"))
                .expect("the file in the way is still readable"),
            "not a directory\n"
        );
    }

    #[test]
    fn a_directory_the_manifest_has_no_entry_for_writes_nothing_and_says_so() {
        // A pact hand-edited out of the file in another window between the
        // press and the Enter: there is no entry to write a scope on, and a
        // window that closed silently would be indistinguishable from success.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ROOT_ROW);
        let mut manifest = pacts();

        let prompt = scope_submit(&mut app, &mut manifest, repo.path(), &field(".", "billing"));

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(saved(repo.path()), None, "something was written anyway");
        assert_eq!(manifest, pacts());
        let message = app.message().expect("the refusal said nothing");
        assert!(message.contains('.'), "{message}");
        assert!(message.contains("`p`"), "{message}");
    }

    #[test]
    fn the_write_goes_to_the_manifest_under_the_repository_root() {
        // Where the file lands, said once: `.warlock/pacts.toml` under the root
        // handed in, and nowhere else. The engine owns the path, and this is
        // the assertion that the root travelling through here is the one the
        // loop resolved.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();

        scope_submit(
            &mut app,
            &mut manifest,
            repo.path(),
            &field("crates/engine", "billing"),
        );

        let written = repo.path().join(".warlock").join("pacts.toml");
        assert!(written.is_file(), "{} is not a file", written.display());
        assert!(
            fs::read_to_string(&written)
                .expect("the manifest reads back")
                .contains("billing")
        );
        // And nothing was left beside it: the save renames its temporary file
        // over the target rather than leaving one behind.
        let beside: Vec<String> = fs::read_dir(repo.path().join(".warlock"))
            .and_then(|entries| {
                entries
                    .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
                    .collect::<io::Result<Vec<String>>>()
            })
            .expect("the manifest directory reads back");
        assert_eq!(beside, ["pacts.toml"]);
    }

    #[test]
    fn typing_and_abandoning_move_the_prompt_and_nothing_else() {
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();
        let before = app.clone();
        let typed = field("crates/engine", "web");

        assert_eq!(
            scope_edit(
                &mut app,
                &mut manifest,
                repo.path(),
                &ScopePrompt::open("crates/engine", "we"),
                Edited::Open(typed.clone()),
            ),
            ScopePrompt::Open(typed),
            "a character typed did something other than land in the field",
        );
        assert_eq!(
            scope_edit(
                &mut app,
                &mut manifest,
                repo.path(),
                &ScopePrompt::open("crates/engine", "web"),
                Edited::Close,
            ),
            ScopePrompt::Closed,
        );
        assert_eq!(saved(repo.path()), None, "typing wrote to disk");
        assert_eq!(manifest, pacts(), "typing edited the manifest");
        assert_eq!(app, before, "typing moved the view");
    }

    #[test]
    fn a_submit_with_no_window_up_writes_nothing() {
        // The road `press_for` cannot take — it only consults `edit_for` while a
        // window is up — answered rather than left to be discovered.
        let repo = a_repo();
        let mut app = app_on(repo.path(), ENGINE_ROW);
        let mut manifest = pacts();

        let prompt = scope_edit(
            &mut app,
            &mut manifest,
            repo.path(),
            &ScopePrompt::Closed,
            Edited::Submit,
        );

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(saved(repo.path()), None);
        assert_eq!(manifest, pacts());
    }

    #[test]
    fn the_whole_path_from_the_press_to_the_saved_manifest_is_one_key_at_a_time() {
        // The event loop's own arms, in the order a reader presses them: `s`,
        // four characters, a Backspace over the last of them, Enter. Nothing
        // here is a terminal, a network or a `claude` — the whole path is these
        // three functions over a directory of this test's own.
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = pacts();

        let mut prompt = scope_press(&mut app, &manifest, repo.path(), false);
        assert_eq!(prompt, ScopePrompt::open("crates/tui", ""));

        for code in [
            KeyCode::Char('w'),
            KeyCode::Char('e'),
            KeyCode::Char('b'),
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Enter,
        ] {
            let edited = {
                let field = prompt.field().expect("the window is still up");
                edit_for(press(code), field)
            };
            prompt = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
        }

        assert_eq!(prompt, ScopePrompt::Closed, "Enter left the window up");
        assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
        assert_eq!(
            scope_on(&saved(repo.path()).expect("the keys wrote"), "crates/tui"),
            Some("web"),
        );
        // And the run state and the message line came through the whole of it
        // untouched: no key of those six was a run.
        assert!(!app.is_pacting());
        assert_eq!(app.pact_line(), None);
        assert_eq!(app.message(), Some(LAST_KEY));
    }

    #[test]
    fn a_refusal_typed_out_can_be_fixed_and_written_without_reopening_the_window() {
        // Enter on something the engine turns down, then a Backspace and a
        // character over the very text that was refused: the window stayed up
        // holding it, which is the whole point of a refusal not closing.
        let repo = a_repo();
        let mut app = app_on(repo.path(), TUI_ROW);
        let mut manifest = pacts();
        let mut prompt = ScopePrompt::open("crates/tui", "web!");

        for code in [KeyCode::Enter, KeyCode::Backspace, KeyCode::Enter] {
            let edited = {
                let field = prompt.field().expect("the window is still up");
                edit_for(press(code), field)
            };
            prompt = scope_edit(&mut app, &mut manifest, repo.path(), &prompt, edited);
            if code == KeyCode::Backspace {
                assert_eq!(saved(repo.path()), None, "the refused text was written");
            }
        }

        assert_eq!(prompt, ScopePrompt::Closed);
        assert_eq!(scope_on(&manifest, "crates/tui"), Some("web"));
    }
}
