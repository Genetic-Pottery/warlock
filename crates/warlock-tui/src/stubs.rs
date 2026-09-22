//! No agent here stops a turn on the say-when, reports activities, or records
//! the model it was raised at, and that is what says which tests they are not
//! for: cancelling a run, attaching the activity port, and the argv a brief
//! runs at are facts about a child process, asserted on the real adapter in
//! `pacting.rs` and `chatting.rs`. A second record of them here would be a copy
//! for the two to disagree over.
//!
//! [`Scripted`] keeping the handles it was wired to is not that record: what it
//! answers never depends on them, and what they buy is the one question the
//! real adapter cannot be asked from the panel's side — whether the session in
//! flight was told to stop.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};
use warlock_engine::{Agent, agent, drafting, stub_answer};
use warlock_tui::{Activities, Cancel, Converses, LinearError, Posts, Wired};

use crate::clipboard::Clip;
use crate::pushing::Opens;

// A clipboard nothing on the machine has to provide. The refusal is kept as the
// description rather than as an `arboard::Error`, because that type is not
// `Clone` and a stand-in that could only fail once would be a stand-in that
// silently succeeded on the second copy.
#[derive(Debug, Default)]
pub(crate) struct Copying {
    refusing: Option<String>,
    copied: Vec<String>,
}

impl Copying {
    pub(crate) fn taking() -> Self {
        Self::default()
    }

    pub(crate) fn refusing(description: impl Into<String>) -> Self {
        Self {
            refusing: Some(description.into()),
            copied: Vec::new(),
        }
    }

    pub(crate) fn copied(&self) -> &[String] {
        &self.copied
    }
}

impl Clip for Copying {
    fn put(&mut self, text: &str) -> Result<(), arboard::Error> {
        if let Some(description) = &self.refusing {
            return Err(arboard::Error::Unknown {
                description: description.clone(),
            });
        }
        self.copied.push(text.to_owned());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Passing;

impl Passing {
    pub(crate) fn filling() -> Self {
        Self
    }
}

impl Agent for Passing {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        Ok(agent::Response::new(stub_answer(request)))
    }
}

impl Wired for Passing {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        *self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Saying {
    answer: String,
}

impl Saying {
    pub(crate) fn answering(answer: impl Into<String>) -> Self {
        Self {
            answer: answer.into(),
        }
    }
}

impl Wired for Saying {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Saying {
    fn turn(&self, _message: &str) -> Result<String, agent::Error> {
        Ok(self.answer.clone())
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

/// One turn's answer, written down before the run starts.
///
/// Prose and the drafts object are one variant because the session decides
/// which is which — prose is a question while a round is left and a failed
/// attempt once the asking is over — and a stand-in that named the two apart
/// would be a second opinion about what it had just said.
#[derive(Debug, Clone)]
pub(crate) enum Answering {
    Says(String),
    /// No `claude` on the machine, which is one of the three ways a session's
    /// turn ends in a failure rather than an answer.
    Missing,
}

impl Answering {
    pub(crate) fn says(text: impl Into<String>) -> Self {
        Self::Says(text.into())
    }

    /// The drafting road's own stub object, named for the slice it stands in
    /// for: two drafts inside every cap, so it is accepted rather than
    /// repaired.
    pub(crate) fn drafts(slice: &str) -> Self {
        Self::Says(drafting::stub_answer(slice))
    }

    pub(crate) const fn missing() -> Self {
        Self::Missing
    }
}

/// A model that answers a written-down sequence, one entry per turn, and keeps
/// what it was asked and every cancel handle it was wired to.
///
/// The cancels are the point of the record: a drafting session mints its own
/// and wires its agent to it, so a test that wants to know whether quitting
/// reached the turn in flight has nowhere else to look.
///
/// `Arc<Mutex<_>>` rather than the `Rc<RefCell<_>>` a subcommand's stand-in
/// uses, for the difference this path has: a slice's turn runs on a worker
/// thread, so a model that could not cross one would not stand in for the thing
/// being tested.
#[derive(Debug, Clone)]
pub(crate) struct Scripted {
    answers: Arc<Mutex<VecDeque<Answering>>>,
    said: Arc<Mutex<Vec<String>>>,
    cancels: Arc<Mutex<Vec<Cancel>>>,
    held: Option<Arc<Gate>>,
}

impl Scripted {
    pub(crate) fn saying(answers: impl IntoIterator<Item = Answering>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            said: Arc::new(Mutex::new(Vec::new())),
            cancels: Arc::new(Mutex::new(Vec::new())),
            held: None,
        }
    }

    /// The same model, answering nothing until the gate is opened: what a turn
    /// that takes minutes looks like from the loop's side, without a clock.
    pub(crate) fn held_at(mut self, gate: &Arc<Gate>) -> Self {
        self.held = Some(Arc::clone(gate));
        self
    }

    pub(crate) fn turns(&self) -> usize {
        self.said.lock().expect("a stand-in nothing poisoned").len()
    }

    /// Whether anything that was given a handle on a turn of this model has been
    /// told to stop.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancels
            .lock()
            .expect("a stand-in nothing poisoned")
            .iter()
            .any(Cancel::is_cancelled)
    }
}

impl Wired for Scripted {
    fn wired(&self, cancel: Cancel, _activities: Activities) -> Self {
        self.cancels
            .lock()
            .expect("a stand-in nothing poisoned")
            .push(cancel);
        self.clone()
    }
}

impl Converses for Scripted {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        self.said
            .lock()
            .expect("a stand-in nothing poisoned")
            .push(message.to_owned());
        if let Some(gate) = &self.held {
            gate.wait();
        }

        let answer = self
            .answers
            .lock()
            .expect("a stand-in nothing poisoned")
            .pop_front();
        match answer {
            Some(Answering::Says(text)) => Ok(text),
            Some(Answering::Missing) => Err(agent::Error::NotFound {
                program: "claude".into(),
            }),
            // A turn the script has no answer for is a session opened where the
            // test meant none to be, which is worth failing over rather than
            // answering.
            None => panic!("a turn this model was not scripted for: {message}"),
        }
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

/// A Linear that answers the four requests a push makes out of memory, and the
/// seam it arrives through: a `Boarding` is its own [`Opens::Client`], so a test
/// keeps a handle on the very client the session opened and can read afterwards
/// what was asked of it.
///
/// `Arc<Mutex<_>>` and not the `Rc<RefCell<_>>` of `tests/push.rs`'s stand-in,
/// for the difference this path has: the panel's push runs on a worker thread,
/// so a client that could not cross one would not be a stand-in for the thing
/// being tested.
///
/// The key it is opened with is taken and dropped, never stored. A stand-in
/// holding it would put it back into a `Debug` rendering, which is the one thing
/// every test on this path asserts is nowhere.
#[derive(Debug, Clone)]
pub(crate) struct Boarding {
    asked: Arc<Mutex<Vec<String>>>,
    url: String,
    refusing: Option<String>,
    held: Option<Arc<Gate>>,
}

impl Boarding {
    /// A workspace that has the team, the backlog status and the label already,
    /// and creates the project at `url`.
    pub(crate) fn filing(url: impl Into<String>) -> Self {
        Self {
            asked: Arc::new(Mutex::new(Vec::new())),
            url: url.into(),
            refusing: None,
            held: None,
        }
    }

    /// The same workspace, answering nothing until the gate is opened: what a
    /// slow request looks like from the loop's side, without a clock.
    pub(crate) fn held_at(mut self, gate: &Arc<Gate>) -> Self {
        self.held = Some(Arc::clone(gate));
        self
    }

    /// Linear's own words for a request it understood and would not do, on
    /// whichever request comes first: the failure the panel has to survive.
    pub(crate) fn refusing(message: impl Into<String>) -> Self {
        Self {
            refusing: Some(message.into()),
            ..Self::filing("")
        }
    }

    /// How many requests reached the workspace, which is how a test says that a
    /// second `/push` sent nothing: one push is four.
    pub(crate) fn requests(&self) -> usize {
        self.asked
            .lock()
            .expect("no test panics holding this")
            .len()
    }
}

impl Opens for Boarding {
    type Client = Self;

    fn open(&self, _key: &str) -> Self {
        self.clone()
    }
}

impl Posts for Boarding {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        self.asked
            .lock()
            .expect("no test panics holding this")
            .push(document.to_owned());
        if let Some(gate) = &self.held {
            gate.wait();
        }

        if let Some(message) = &self.refusing {
            return Err(LinearError::Refused {
                message: message.clone(),
            });
        }
        Ok(answered(document, &self.url))
    }
}

// The `data` object of each answer, by the operation that asked for it: the
// client unwraps `data` before its callers see it, so this is the shape they
// read. A document this does not know is a fifth request nobody meant to send.
fn answered(document: &str, url: &str) -> Value {
    if document.contains("teams(") {
        json!({ "teams": { "nodes": [{ "id": "team-held" }] } })
    } else if document.contains("projectStatuses") {
        json!({ "projectStatuses": { "nodes": [{ "id": "status-backlog", "name": "Backlog" }] } })
    } else if document.contains("projectLabels(") {
        json!({ "projectLabels": { "nodes": [{ "id": "label-held" }] } })
    } else if document.contains("projectCreate(") {
        json!({ "projectCreate": { "project": { "id": "project-filed", "url": url } } })
    } else {
        panic!("a push asked for something no workspace was given: {document}");
    }
}

/// The other direction over the same seam: a Linear that answers the one
/// request a `/pull` makes — `project(id:)` — out of memory, and is its own
/// [`Opens::Client`] for [`Boarding`]'s reason.
///
/// A sibling rather than a fifth arm on `answered` above, because the two are
/// stand-ins for two different conversations: a push's workspace has a team, a
/// status and a label and creates something, a pull's has one project and is
/// only read. Folding them into one value would make every test set fields the
/// path it drives never looks at.
#[derive(Debug, Clone)]
pub(crate) struct Reading {
    asked: Arc<Mutex<Vec<String>>>,
    name: String,
    // `None` is a project sitting in no status at all, which is a workspace
    // whose board has none rather than a broken answer — so the gate has to be
    // able to say it, and a test has to be able to make it.
    status: Option<String>,
    content: String,
    held: Option<Arc<Gate>>,
}

impl Reading {
    /// A workspace holding one project, under that name, in that status, with
    /// that description — which is where the scope block a pull parses lives.
    pub(crate) fn holding(
        name: impl Into<String>,
        status: Option<&str>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            asked: Arc::new(Mutex::new(Vec::new())),
            name: name.into(),
            status: status.map(ToOwned::to_owned),
            content: content.into(),
            held: None,
        }
    }

    /// The same workspace, answering nothing until the gate is opened: a slow
    /// request, without a clock.
    pub(crate) fn held_at(mut self, gate: &Arc<Gate>) -> Self {
        self.held = Some(Arc::clone(gate));
        self
    }

    /// How many requests reached the workspace, which is how a test says that a
    /// refusal read nothing: one pull is one request.
    pub(crate) fn requests(&self) -> usize {
        self.asked
            .lock()
            .expect("no test panics holding this")
            .len()
    }
}

impl Opens for Reading {
    type Client = Self;

    fn open(&self, _key: &str) -> Self {
        self.clone()
    }
}

impl Posts for Reading {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        self.asked
            .lock()
            .expect("no test panics holding this")
            .push(document.to_owned());
        if let Some(gate) = &self.held {
            gate.wait();
        }

        assert!(
            document.contains("project(id:"),
            "a pull asked for something no workspace was given: {document}"
        );
        Ok(json!({
            "project": {
                "name": self.name,
                "content": self.content,
                "url": "https://linear.app/acme/project/pulled-1a2b3c",
                "status": self.status.as_ref().map(|status| json!({ "name": status })),
            }
        }))
    }
}

/// A request held open, for the one thing about a worker a finished push cannot
/// show: that the loop goes round — drawing, answering keys — while it is in
/// flight. A test holds the gate shut, counts rounds, and opens it.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub(crate) fn shut() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Notified rather than dropped, so the worker goes on to answer and the
    /// test can then assert what the push said: a gate nobody opens is a thread
    /// parked until the test binary exits.
    pub(crate) fn open(&self) {
        *self.open.lock().expect("no test panics holding this") = true;
        self.changed.notify_all();
    }

    // Looped for the spurious wakeup `Condvar` is allowed, which is the whole of
    // why this is not a bare `park`.
    fn wait(&self) {
        let mut open = self.open.lock().expect("no test panics holding this");
        while !*open {
            open = self
                .changed
                .wait(open)
                .expect("no test panics holding this");
        }
    }
}
