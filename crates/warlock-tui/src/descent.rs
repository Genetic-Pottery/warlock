//! The run itself, for both doors: which engine entry point a press or a
//! subcommand means, and the one save at the end of it.
//!
//! Warlock runs a subtree from two places. The panel's `p` and `r` keys spawn a
//! worker ([`pacting`](crate::pacting)) that reports over a channel into the
//! account card; `warlock pact` and `warlock refresh` descend on the loop's own
//! thread ([`running`](crate::running)) and report as lines on stdout. What
//! happens in between was written twice — match the tag, call one of
//! [`pact_subtree`], [`refresh_subtree`] or [`unpact_subtree`], save the
//! manifest exactly once — and the two copies agreed only because each argued
//! the save rule at length in its own doc comment.
//!
//! [`descend`] is that middle, once.
//!
//! # What each door still owns
//!
//! Everything about *reporting*. The panel turns a [`PactedSubtree`] into a
//! [`Toggled`](crate::pacting::Toggled) with a footer line and a refusal per
//! directory; the shell turns the same value into stderr lines and an exit
//! status. Those are genuinely different — one is a card on a screen and the
//! other is a pipe — and neither is this module's business.
//!
//! What is this module's business is the part that must not differ: **one
//! descent, and one save.** A door that saved twice would write a manifest the
//! other never writes, and a door that saved on a different rule would record a
//! different repository from the same keystroke.
//!
//! # The un-pact is here and is not a run
//!
//! [`Descent::Unpact`] takes the third arm, and it is manifest editing and
//! nothing else: no walk, no pass, no hash, and every `WARLOCK.md` left exactly
//! where it is. It comes back shaped like the other two — a [`PactedSubtree`]
//! with an empty failure list — so that the save below it is the same line
//! rather than a special case, which is the whole reason it is in here rather
//! than beside the key that presses it.
//!
//! Note that only the panel's `p` reaches it. The shell's `warlock unpact` is a
//! different road on purpose ([`edits`](crate::edits)): it spends no model pass,
//! so it is gated and applied without an agent or an observer ever being built.

use std::path::Path;

use warlock_engine::{
    Agent, Manifest, PactedSubtree, Pacting, pact, pact_subtree, refresh_subtree, unpact_subtree,
};
use warlock_tui::Cancel;

use crate::error::Error;
use crate::standing::{FOR_PACT, FOR_REFRESH};

/// Which of the three things a run over a subtree can be.
///
/// A carried tag rather than three copies of the function below, because the
/// two doors must not be able to drift into calling a different entry point or
/// saving a different number of times for the same gesture. [`Copy`], because it
/// is a tag and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Descent {
    /// Describe every directory of the subtree, whatever state it was in:
    /// `warlock pact`, and the panel's `p` on an unpacted row.
    Pact,
    /// Describe the ones the engine finds stale and leave the fresh ones exactly
    /// as they are — grant, document and all: `warlock refresh`, and `r`.
    Refresh,
    /// Drop the subtree's entries and the scopes that were terms of them,
    /// touching no file on disk: the panel's `p` on a pacted row.
    Unpact,
}

impl Descent {
    /// What this descent wanted a repository root for, as the tail of the
    /// sentence a missing `.git` is refused with.
    ///
    /// Only the two the shell has subcommands for are ever asked: the un-pact
    /// the shell does is [`edits`](crate::edits)' and carries its own tail, and
    /// the un-pact this module does is a keystroke inside a warlock that already
    /// found its repository. Answering with the pact's tail is the closest thing
    /// to true if it is ever reached.
    pub(crate) const fn wanted(self) -> &'static str {
        match self {
            Self::Pact | Self::Unpact => FOR_PACT,
            Self::Refresh => FOR_REFRESH,
        }
    }
}

/// Run `descent` over the subtree at `target`, and save what it earned.
///
/// The whole of what the panel's keys and the shell's subcommands share, and
/// the reason it is one function: **the manifest is saved exactly once, here,
/// after the descent and never during it.** The engine writes every
/// `WARLOCK.md` and hands back a manifest as a *value*; nothing under
/// `.warlock/` moves until the line at the bottom of this function. That is
/// what makes a partly-failed run still worth recording and a cancelled run
/// keep what it finished, and it is a rule that would stop being one the moment
/// there were two places saving.
///
/// `manifest` is the one in hand rather than one read here, because both callers
/// already hold it: the loop keeps the manifest a keystroke edits, and the shell
/// read it through [`Opened`](crate::edits::Opened) before the boundary was
/// asked. Reading it again would be a second answer to a settled question, and
/// one that could disagree with the boundary that was already judged against it.
///
/// # Errors
///
/// [`Error::Pact`] when the subtree cannot be walked — which happens before any
/// pass is spent — and [`Error::Manifest`] when the manifest will not save,
/// which happens after every document is already on disk. Nothing else: a
/// directory that fails on its own comes back in
/// [`PactedSubtree::failures`] with the rest of the run intact, because one
/// directory failing is not the run failing.
///
/// Typed rather than flattened to a line. The panel wants a footer sentence and
/// the shell wants an exit status, and an [`Error`] is the one value that can
/// still become either — flattening here would leave the shell parsing prose to
/// find out whether it should exit 1.
pub(crate) fn descend(
    descent: Descent,
    target: &Path,
    repo_root: &Path,
    manifest: &Manifest,
    agent: &dyn Agent,
    observer: &mut dyn pact::Observer,
) -> Result<PactedSubtree, Error> {
    let subtree = match descent {
        // Every directory in the subtree, whatever state it was in.
        Descent::Pact => pact_subtree(target, repo_root, manifest, agent, observer)
            .map_err(|source| Error::Pact { source })?,
        // Only the stale ones, and which those are is the engine's judgement
        // from the same manifest handed in here: it keeps the grant of
        // everything it skipped, so a fresh directory costs no pass and loses
        // nothing.
        Descent::Refresh => refresh_subtree(target, repo_root, manifest, agent, observer)
            .map_err(|source| Error::Pact { source })?,
        // Pure manifest editing: the only thing it can refuse is a path the
        // manifest has no spelling for, and it reaches neither the agent nor the
        // observer. Shaped like the other two so the save below is one line.
        Descent::Unpact => PactedSubtree {
            manifest: unpact_subtree(target, repo_root, manifest)
                .map_err(|source| Error::Manifest { source })?,
            failures: Vec::new(),
            problems: Vec::new(),
        },
    };

    subtree
        .manifest
        .save(repo_root)
        .map_err(|source| Error::Manifest { source })?;
    Ok(subtree)
}

/// Whether a run should carry on into the directory it was just offered.
///
/// The two lines every [`pact::Observer::starting`] in this crate opens with, in
/// one place. Both observers read the same latch and answer the same way, and
/// both used to say so in prose instead: the panel's stops so the worker thread
/// ends between directories, and the shell's stops so the descent does, and
/// neither has an opinion of its own about when a run should end. This port
/// carries somebody else's answer to the one place the engine asks for it.
///
/// Asked *before* anything is reported, in both, so a cancelled run neither
/// announces a directory it will not describe nor describes it.
pub(crate) fn carry_on(cancel: &Cancel) -> Pacting {
    if cancel.is_cancelled() {
        Pacting::Stop
    } else {
        Pacting::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use warlock_engine::{Agent, Manifest, Unwatched, agent};
    use warlock_tui::Cancel;

    use super::{Descent, carry_on, descend};
    use crate::error::Error;

    /// A model that answers every pass with the same document.
    struct Answering;

    impl Agent for Answering {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            // Comfortably over `MINIMUM_DOCUMENT_BYTES`, which is the whole of
            // what the engine asks of an answer before it will write it down.
            Ok(agent::Response::new(
                "# a directory\n\nLong enough to be a document that warlock will \
                 actually write down. The engine measures the trimmed answer and \
                 refuses anything under a couple of hundred bytes, on the grounds \
                 that a pass which came back with a sentence did not read the \
                 directory, so this fixture says rather more than it needs to.\n",
            ))
        }
    }

    /// A model that must never be asked anything: a run that spends a pass
    /// through this one fails loudly rather than passing quietly.
    struct Never;

    impl Agent for Never {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            panic!("a fresh subtree must cost no pass")
        }
    }

    /// A checkout with one source directory in it.
    fn a_checkout() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(repo.path().join(".git")).expect("a repository marker");
        fs::create_dir_all(repo.path().join("src")).expect("a source directory");
        fs::write(repo.path().join("src/lib.rs"), "//! a module\n").expect("a source file");
        repo
    }

    /// The modules the manifest on disk holds, in its own order.
    fn stored(repo: &Path) -> Vec<String> {
        Manifest::load(repo)
            .expect("a manifest that reads")
            .entries()
            .iter()
            .map(|entry| entry.module().to_owned())
            .collect()
    }

    #[test]
    fn a_pact_saves_once_and_the_manifest_on_disk_is_what_came_back() {
        let repo = a_checkout();

        let subtree = descend(
            Descent::Pact,
            repo.path(),
            repo.path(),
            &Manifest::new(),
            &Answering,
            &mut Unwatched,
        )
        .expect("a pact of a readable subtree");

        assert!(subtree.failures.is_empty(), "{:?}", subtree.failures);
        assert_eq!(
            stored(repo.path()),
            subtree
                .manifest
                .entries()
                .iter()
                .map(|entry| entry.module().to_owned())
                .collect::<Vec<_>>(),
            "the manifest on disk is not the one the descent handed back"
        );
    }

    #[test]
    fn an_un_pact_drops_the_entries_and_leaves_every_document_on_disk() {
        let repo = a_checkout();
        descend(
            Descent::Pact,
            repo.path(),
            repo.path(),
            &Manifest::new(),
            &Answering,
            &mut Unwatched,
        )
        .expect("a pact of a readable subtree");
        let documents: Vec<_> = ["WARLOCK.md", "src/WARLOCK.md"]
            .into_iter()
            .map(|name| fs::read(repo.path().join(name)).expect("a document the pact wrote"))
            .collect();

        let manifest = Manifest::load(repo.path()).expect("a manifest that reads");
        descend(
            Descent::Unpact,
            repo.path(),
            repo.path(),
            &manifest,
            &Answering,
            &mut Unwatched,
        )
        .expect("an un-pact of a pacted subtree");

        assert!(
            stored(repo.path()).is_empty(),
            "an entry survived the un-pact"
        );
        for (name, before) in ["WARLOCK.md", "src/WARLOCK.md"].into_iter().zip(documents) {
            assert_eq!(
                fs::read(repo.path().join(name)).expect("the document is still there"),
                before,
                "un-pacting touched `{name}` on disk"
            );
        }
    }

    #[test]
    fn a_refresh_of_a_fresh_subtree_spends_no_pass_and_still_saves() {
        let repo = a_checkout();
        descend(
            Descent::Pact,
            repo.path(),
            repo.path(),
            &Manifest::new(),
            &Answering,
            &mut Unwatched,
        )
        .expect("a pact of a readable subtree");
        let before = stored(repo.path());

        let manifest = Manifest::load(repo.path()).expect("a manifest that reads");
        let subtree = descend(
            Descent::Refresh,
            repo.path(),
            repo.path(),
            &manifest,
            &Never,
            &mut Unwatched,
        )
        .expect("a refresh with nothing stale");

        assert!(subtree.failures.is_empty());
        assert_eq!(
            stored(repo.path()),
            before,
            "a no-op refresh moved an entry"
        );
    }

    #[test]
    fn a_subtree_that_cannot_be_walked_is_a_pact_error_and_saves_nothing() {
        let repo = a_checkout();

        let error = descend(
            Descent::Pact,
            &repo.path().join("nowhere"),
            repo.path(),
            &Manifest::new(),
            &Answering,
            &mut Unwatched,
        )
        .expect_err("a directory that is not there cannot be pacted");

        assert!(matches!(error, Error::Pact { .. }), "{error:?}");
        assert!(
            !repo.path().join(".warlock/pacts.toml").exists(),
            "a run that never started wrote a manifest"
        );
    }

    #[test]
    fn an_un_pact_of_a_path_the_manifest_cannot_spell_is_a_manifest_error() {
        let repo = a_checkout();

        let error = descend(
            Descent::Unpact,
            Path::new("/elsewhere"),
            repo.path(),
            &Manifest::new(),
            &Answering,
            &mut Unwatched,
        )
        .expect_err("nothing outside the repository has a manifest form");

        assert!(matches!(error, Error::Manifest { .. }), "{error:?}");
    }

    #[test]
    fn a_pulled_say_when_stops_a_run_and_an_unpulled_one_does_not() {
        let cancel = Cancel::new();

        assert_eq!(
            carry_on(&cancel),
            warlock_engine::Pacting::Continue,
            "nobody has said stop"
        );

        cancel.cancel();

        assert_eq!(
            carry_on(&cancel),
            warlock_engine::Pacting::Stop,
            "the latch both doors read was pulled and one of them carried on"
        );
    }

    #[test]
    fn every_descent_wants_a_root_for_something_it_can_name() {
        for descent in [Descent::Pact, Descent::Refresh, Descent::Unpact] {
            assert!(
                !descent.wanted().is_empty(),
                "{descent:?} cannot say what it wanted a repository for"
            );
        }
    }
}
