//! One descent, and one save. The panel's `p` and `r` spawn a worker that
//! reports over a channel; `warlock pact` and `warlock refresh` descend on the
//! loop's own thread and report on stdout. Everything about *reporting* is
//! still each door's own; what must not differ is which engine entry point a
//! gesture means and how many times the manifest is written, because a door
//! that saved twice would write a manifest the other never writes and a door
//! that saved on a different rule would record a different repository from the
//! same keystroke.
//!
//! `Descent::Unpact` is here and is not a run: no walk, no pass, no hash, and
//! every `WARLOCK.md` left where it is. It is in this module anyway so that the
//! save below it is the same line rather than a special case. Only the panel's
//! `p` reaches it — the shell's `warlock unpact` is `edits`' road on purpose,
//! since it spends no model pass and needs neither an agent nor an observer.

use std::path::Path;

use warlock_engine::{
    Agent, Manifest, PactedSubtree, Pacting, pact, pact_subtree, refresh_subtree, unpact_subtree,
};
use warlock_tui::Cancel;

use crate::error::Error;
use crate::standing::{FOR_PACT, FOR_REFRESH};

// A carried tag rather than three copies of `descend`, because the two doors
// must not be able to drift into calling a different entry point, or saving a
// different number of times, for the same gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Descent {
    Pact,
    Refresh,
    Unpact,
}

impl Descent {
    // Only the two the shell has subcommands for are ever asked: the un-pact
    // the shell does is `edits`' and carries its own tail, and the un-pact this
    // module does is a keystroke inside a warlock that already found its
    // repository. The pact's tail is the closest thing to true if it is reached.
    pub(crate) const fn wanted(self) -> &'static str {
        match self {
            Self::Pact | Self::Unpact => FOR_PACT,
            Self::Refresh => FOR_REFRESH,
        }
    }
}

// The manifest is saved exactly once, here, after the descent and never during
// it. The engine writes every `WARLOCK.md` and hands back a manifest as a
// *value*, so nothing under `.warlock/` moves until the line at the bottom of
// this function — which is what makes a partly-failed run still worth recording
// and a cancelled run keep what it finished, and a rule that would stop being
// one the moment there were two places saving.
//
// `manifest` is the one in hand rather than one read here: both callers already
// hold it, and reading it again would be a second answer to a settled question,
// one that could disagree with the boundary already judged against it.
//
// The error is typed rather than flattened to a line. The panel wants a footer
// sentence and the shell wants an exit status, and an `Error` is the one value
// that can still become either; flattening here would leave the shell parsing
// prose to find out whether it should exit 1.
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
            repairs: Vec::new(),
        },
    };

    subtree
        .manifest
        .save(repo_root)
        .map_err(|source| Error::Manifest { source })?;
    Ok(subtree)
}

// The two lines every `pact::Observer::starting` in this crate opens with, in
// one place: neither observer has an opinion of its own about when a run should
// end, and this carries somebody else's answer to where the engine asks for it.
// Asked *before* anything is reported, in both, so a cancelled run neither
// announces a directory it will not describe nor describes it.
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

    use warlock_engine::{Agent, Manifest, Unwatched, agent, stub_answer};
    use warlock_tui::Cancel;

    use super::{Descent, carry_on, descend};
    use crate::error::Error;

    struct Answering;

    impl Agent for Answering {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            // Every slot the engine checks for, filled: what a pass has to
            // answer before the engine will write a document down.
            Ok(agent::Response::new(stub_answer(request)))
        }
    }

    struct Never;

    impl Agent for Never {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            panic!("a fresh subtree must cost no pass")
        }
    }

    fn a_checkout() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(repo.path().join(".git")).expect("a repository marker");
        fs::create_dir_all(repo.path().join("src")).expect("a source directory");
        fs::write(repo.path().join("src/lib.rs"), "//! a module\n").expect("a source file");
        repo
    }

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
