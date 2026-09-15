//! Neither agent here honours the say-when, reports activities, or records the
//! model it was raised at, and that is what says which tests they are not for:
//! cancelling a run, attaching the activity port, and the argv a brief runs at
//! are facts about a child process, asserted on the real adapter in
//! `pacting.rs` and `chatting.rs`. A second record of them here would be a copy
//! for the two to disagree over.

use warlock_engine::{Agent, agent, stub_answer};
use warlock_tui::{Activities, Cancel, Converses, Wired};

use crate::clipboard::Clip;

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
