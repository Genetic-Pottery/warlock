//! In-memory stand-ins for the two model seams, for tests that are not about
//! the model.
//!
//! A pact and a turn both end at a `claude` child process, and for a long time
//! the only way to test anything above them was to point the real adapter at
//! `/bin/sh` and have a shell print the wire format back. That works, and
//! `claude.rs`'s own suite still does it, because there the transport *is* the
//! thing under test. Everywhere else it was a language model faked by a shell
//! script: Unix-only, at the mercy of whichever `/bin/sh` the machine has, and
//! slow in the way a process is slow.
//!
//! These two are the other adapter at each seam. [`Passing`] answers a pass with
//! a document it was built with; [`Saying`] answers a turn with a sentence it
//! was built with. Neither spawns anything, so a test over them runs on any
//! platform and in microseconds.
//!
//! ## What they deliberately do not do
//!
//! Neither honours the say-when, and neither reports activities. That is not an
//! oversight — it is what says which tests these are *not* for. A test about
//! cancelling a run, or about the activity port being attached to the agent the
//! worker owns, is a test about the wiring between warlock and a child process,
//! and it wants the real adapter over a stand-in program. `pacting.rs`'s
//! `a_spawned_run_reports_what_its_passes_do_over_the_channel_it_hands_back` is
//! exactly that test and says so itself.
//!
//! Nor does either remember how it was configured. Which model and effort a
//! brief runs at is asserted against the argv a real `ChatAgent` builds, in
//! `chatting.rs`, because that is where the fact is real; a second record of it
//! here would be a copy for the two to disagree over.

use warlock_engine::{Agent, agent};
use warlock_tui::{Activities, Cancel, Converses, Wired};

/// An agent that answers every pass with the same document, out of memory.
///
/// The second adapter at the pass seam. Whatever the request names — the
/// directory, the files under it, the documents of its children — the answer is
/// the string this was built with, because a test above this seam is about what
/// warlock does with a document rather than about what a model wrote.
#[derive(Debug, Clone)]
pub(crate) struct Passing {
    /// What every pass answers with.
    document: String,
}

impl Passing {
    /// An agent whose every pass answers with `document`.
    ///
    /// Long enough to be kept is the caller's business: the engine drops a
    /// document under `MINIMUM_DOCUMENT_BYTES`, so a test that expects one to
    /// land has to hand over one that survives that.
    pub(crate) fn answering(document: impl Into<String>) -> Self {
        Self {
            document: document.into(),
        }
    }
}

impl Agent for Passing {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        Ok(agent::Response::new(self.document.clone()))
    }
}

impl Wired for Passing {
    /// The same agent: there is no child to kill and no port to report on.
    ///
    /// See this module's own note about what that rules out.
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

/// A conversation that answers every turn with the same sentence, out of
/// memory.
///
/// The second adapter at the turn seam, and the reason the conversation, the
/// composer and the `/write` path can be driven end to end without a shell.
#[derive(Debug, Clone)]
pub(crate) struct Saying {
    /// What every turn answers with.
    answer: String,
}

impl Saying {
    /// A conversation whose every turn answers with `answer`.
    pub(crate) fn answering(answer: impl Into<String>) -> Self {
        Self {
            answer: answer.into(),
        }
    }
}

impl Wired for Saying {
    /// The same conversation: there is no child to kill and no port to report
    /// on.
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Saying {
    fn turn(&self, _message: &str) -> Result<String, agent::Error> {
        Ok(self.answer.clone())
    }

    /// The same conversation, and deliberately no record that it was raised.
    ///
    /// Which model and effort a brief runs at is asserted where it is decided
    /// and where it is real — against the argv a [`ChatAgent`](warlock_tui::ChatAgent)
    /// actually builds, in `chatting.rs`. A second answer here would be a copy
    /// of that fact for the two to disagree over.
    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}
