//! The result contract of a sub-task session: the one thing warlock reads back
//! out of a session that was given writing tools.
//!
//! A sub-task session ends with one JSON object as its last message —
//! `{"status": ..., "summary": ..., "blocked_reason": ...}` — and [`accept`]
//! turns whatever actually came back into a [`Reported`] warlock can act on.
//! Nothing here runs a session or decides what to do with the answer: the
//! engine opens no sockets and spawns no subprocesses, and the orchestration
//! that spends an outcome lives above it.
//!
//! [`RESULT_PROMPT`] is the shape written down in prose, so the object the
//! session is asked for and the object this module reads are one statement and
//! not two. The rest of a sub-task prompt — the brief, the ticket's context,
//! the scope and the sigils, the refusals — belongs to the caller that raises
//! the session.
//!
//! # Why this is not `pulls::SubtaskStatus`
//!
//! [`crate::SubtaskStatus`] names six statuses a sub-task record on disk can
//! hold; a session may report exactly three of them. Reusing it here would
//! make `pending`, `in_progress` and above all `crossed` constructible out of a
//! session's own answer, and `crossed` is warlock's verdict from the
//! post-session check of the tree — a session that wrote outside its scopes
//! must not be able to claim it did not, and one that behaved must not be able
//! to claim the verdict either. So the wire contract is its own enum, and the
//! conversion runs one way, from this module into the record
//! (`impl From<Reported> for SubtaskStatus`). The `blocked_reason` key is
//! spelt the same in both on purpose: the answer and the record it lands in
//! read against each other.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::pulls::SubtaskStatus;

/// What a sub-task session said it did. Every variant carries its reason in the
/// variant rather than in a field beside it, for the reason
/// [`crate::SubtaskStatus`] does: a `blocked` with nothing to say cannot be
/// constructed, so the reason cannot go missing between the session that knew
/// it and the comment that reports it. A session that reports `blocked`
/// without one is a failure ([`Unreadable::BlockedWithNoReason`]), not a
/// reasonless block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reported {
    Done,
    Blocked(String),
    Failed(String),
}

impl Reported {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Done => DONE,
            Self::Blocked(_) => BLOCKED,
            Self::Failed(_) => FAILED,
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Done => None,
            Self::Blocked(reason) | Self::Failed(reason) => Some(reason),
        }
    }
}

impl fmt::Display for Reported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())?;
        match self.reason() {
            Some(reason) => write!(f, ": {reason}"),
            None => Ok(()),
        }
    }
}

impl From<Reported> for SubtaskStatus {
    fn from(reported: Reported) -> Self {
        match reported {
            Reported::Done => Self::Done,
            Reported::Blocked(reason) => Self::Blocked(reason),
            Reported::Failed(reason) => Self::Failed(reason),
        }
    }
}

/// The session's last message, read. `reply` is that message verbatim whatever
/// happened, because it is the only account of the attempt there is: an answer
/// warlock could not read is still the sentence a person needs in order to see
/// why, and a summary of a failure is worth nothing beside the failure itself.
///
/// [`Self::unreadable`] is `Some` exactly when warlock wrote the outcome rather
/// than reading it, and in that case [`Self::reported`] is always
/// [`Reported::Failed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    reported: Reported,
    summary: String,
    reply: String,
    unreadable: Option<Unreadable>,
}

impl Accepted {
    #[must_use]
    pub const fn reported(&self) -> &Reported {
        &self.reported
    }

    /// What the session said it did, in its own words — or, for a reply warlock
    /// could not read, warlock's own line saying so.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// The session's last message, exactly as it arrived.
    #[must_use]
    pub fn reply(&self) -> &str {
        &self.reply
    }

    #[must_use]
    pub const fn unreadable(&self) -> Option<&Unreadable> {
        self.unreadable.as_ref()
    }
}

/// Why a last message was not the object it was asked for. Each of these is a
/// [`Reported::Failed`], because a session whose answer cannot be read has said
/// nothing about whether it finished — but they are told apart, since what to
/// put in the halt comment differs, and a turn limit cut off mid-object reads
/// very differently from a session that answered in prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// No `{...}` in the message at all.
    NoObject,
    /// Something object-shaped, which serde would not read. `detail` is serde's
    /// own message, which says where.
    NotJson { detail: String },
    /// A `status` this contract does not name — including `crossed`, which is
    /// never a session's to claim.
    UnknownStatus { status: String },
    /// `blocked` with `blocked_reason` absent, null or blank.
    BlockedWithNoReason,
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoObject => f.write_str("the last message held no JSON object"),
            Self::NotJson { detail } => {
                write!(
                    f,
                    "the last message's JSON object could not be read: {detail}"
                )
            }
            Self::UnknownStatus { status } => {
                write!(
                    f,
                    "`{status}` is not one of `{DONE}`, `{BLOCKED}` or `{FAILED}`"
                )
            }
            Self::BlockedWithNoReason => {
                write!(f, "`{BLOCKED}` was reported with no `blocked_reason`")
            }
        }
    }
}

// The three status spellings, named once: they are a wire format shared with
// `RESULT_PROMPT` below, with `pulls.rs`'s record on disk and with the prose of
// the sub-task instructions, so changing one of these strings is changing an
// agreement and not a name.
const DONE: &str = "done";
const BLOCKED: &str = "blocked";
const FAILED: &str = "failed";

/// The line warlock writes in place of a summary when the reply was not the
/// object asked for. It opens by saying warlock wrote it, for the reason
/// [`crate::drafting`]'s fallbacks do: a filled-in summary that reads like a
/// session's own account is worse than an obviously absent one, because it gets
/// believed.
const UNREAD_SUMMARY: &str = "Warlock could not read this sub-task session's last message as the \
                              result object it asked for, so nothing the session did is recorded \
                              here. Its last message is kept verbatim.";

/// The line warlock writes in place of a reason when a session reports `failed`
/// and says nothing at all — the record on disk has nowhere to put a failure
/// without one.
const UNSAID_FAILURE: &str = "the session reported `failed` and said nothing further";

/// Read a sub-task session's last message.
///
/// The object is taken as the last one in the message, so the ordinary shape —
/// the object, then whatever the harness or the model appended after it — is
/// read without the caller trimming anything. Everything that is not a
/// readable object of this shape comes back as a [`Reported::Failed`] carrying
/// an [`Unreadable`], with the message kept verbatim: there is no refusal here
/// and no `Result`, because by the time this runs the session has already
/// edited the tree, and an unreadable answer is a fact to record rather than an
/// error to return to a caller who has no better answer either.
#[must_use]
pub fn accept(reply: &str) -> Accepted {
    match read(reply) {
        Ok(stated) => stated.accepted(reply),
        Err(unreadable) => accept_unreadable(reply, unreadable),
    }
}

// `status` is read as a plain string rather than an enum so an unknown one is
// reported as itself — `UnknownStatus { status: "complete" }` names what the
// session actually said, where serde's own error would only say the field was
// wrong. Unknown keys are allowed through: a session that adds a key has still
// answered, and refusing over it would throw away work already in the tree.
#[derive(Deserialize, Serialize)]
struct Stated {
    status: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    blocked_reason: Option<String>,
}

impl Stated {
    fn accepted(self, reply: &str) -> Accepted {
        let summary = self.summary.trim().to_owned();
        let reason = self
            .blocked_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty());
        let status = self.status.trim();

        // Matched case-insensitively: the statuses are written down in
        // `RESULT_PROMPT` in lower case, and a `Done` is that instruction
        // followed in every way that matters.
        let reported = if status.eq_ignore_ascii_case(DONE) {
            Reported::Done
        } else if status.eq_ignore_ascii_case(BLOCKED) {
            match reason {
                Some(reason) => Reported::Blocked(reason.to_owned()),
                // A block with no reason is the one shape of block warlock
                // will not record: the reason is the whole content of a
                // block — it is what the operator reads, and often a scope
                // refusal warlock itself issued.
                None => return accept_unreadable(reply, Unreadable::BlockedWithNoReason),
            }
        } else if status.eq_ignore_ascii_case(FAILED) {
            // A failure's reason, in order of preference: what the session put
            // in `blocked_reason`, then its own summary, then warlock's line.
            // The record on disk holds no reasonless failure, and inventing
            // the reason here beats leaving the caller to.
            let reason = reason
                .map(str::to_owned)
                .or_else(|| (!summary.is_empty()).then(|| summary.clone()))
                .unwrap_or_else(|| UNSAID_FAILURE.to_owned());
            Reported::Failed(reason)
        } else {
            return accept_unreadable(
                reply,
                Unreadable::UnknownStatus {
                    status: status.to_owned(),
                },
            );
        };

        // An empty summary is kept rather than refused or filled in. A session
        // that finished the work and wrote a thin last line has still finished
        // the work, and the tree it left is the evidence; the log entry reads
        // poorly, which is the honest outcome.
        Accepted {
            reported,
            summary,
            reply: reply.to_owned(),
            unreadable: None,
        }
    }
}

fn accept_unreadable(reply: &str, unreadable: Unreadable) -> Accepted {
    Accepted {
        reported: Reported::Failed(unreadable.to_string()),
        summary: UNREAD_SUMMARY.to_owned(),
        reply: reply.to_owned(),
        unreadable: Some(unreadable),
    }
}

// The last object that reads, and the rightmost one's error if none do. Trying
// earlier candidates matters because a brace left unbalanced in prose — a
// `fn main() {` quoted in a summary line outside the object — would otherwise
// swallow the real answer into a span that cannot parse.
fn read(reply: &str) -> Result<Stated, Unreadable> {
    let objects = objects(reply);
    let Some((last, earlier)) = objects.split_last() else {
        return Err(Unreadable::NoObject);
    };
    serde_json::from_str(last).or_else(|error| {
        earlier
            .iter()
            .rev()
            .find_map(|object| serde_json::from_str(object).ok())
            .ok_or_else(|| Unreadable::NotJson {
                detail: error.to_string(),
            })
    })
}

// Every brace-balanced span of the message, outermost only and in the order
// they start. String literals are tracked so a `{` inside a summary's own text
// counts for nothing, and an unterminated final span is handed over whole
// rather than dropped: a session cut off by its turn limit mid-object should be
// reported as an object that would not read, which says what happened, and not
// as a message with no object in it.
fn objects(reply: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in reply.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            // A `}` with nothing open is prose, and prose is not an error here:
            // the object is looked for, not validated around.
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some(from) = start.take()
                {
                    found.push(&reply[from..=index]);
                }
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.push(&reply[from..]);
    }

    found
}

// The caps and the key names are written into the prose rather than asked
// about, and the tests below read the same constants this does, so moving a
// status spelling and leaving the prose behind fails the build rather than
// quietly instructing a session to answer in a vocabulary warlock cannot read.
/// The fragment of a sub-task session's prompt that states the result
/// contract. It says the shape and nothing about the work: what the sub-task
/// is, what the session may touch and what it must not do are the caller's to
/// add.
pub const RESULT_PROMPT: &str = "\
When you are finished, output one JSON object as the whole of your last \
message, with no code fence and nothing after it:
{\"status\": \"done\" | \"blocked\" | \"failed\", \"summary\": \"...\", \
\"blocked_reason\": null | \"...\"}

\"status\": \"done\" if the sub-task is finished, \"blocked\" if something \
outside your reach stopped it, \"failed\" if you could not finish it for any \
other reason. These three and no others.

\"summary\": what you did, in a few sentences. Facts, not intentions: what \
changed, and what you left alone.

\"blocked_reason\": the reason, when the status is \"blocked\" — one sentence \
naming what stopped you. A \"blocked\" with no reason is read as a failure. \
Otherwise null.

If your last message is not this object, warlock records the sub-task as \
failed and keeps your message as it stands.";

/// The last message a test double hands back for a sub-task session, as
/// [`crate::drafting::stub_answer`] does for a drafting pass. It exists so the
/// key names live in this module only: a fake that hand-writes the object gets
/// to drift from the contract, and then tests pass over a shape nothing real
/// would send.
#[must_use]
pub fn stub_answer(summary: &str) -> String {
    stub_reply(DONE, summary, None)
}

/// [`stub_answer`] for the other two statuses. `status` and `reason` are passed
/// as written so a double can hand back a shape this module refuses — an
/// unknown status, a `blocked` with no reason — without spelling the object out
/// itself.
#[must_use]
pub fn stub_reply(status: &str, summary: &str, reason: Option<&str>) -> String {
    serde_json::to_string_pretty(&Stated {
        status: status.to_owned(),
        summary: summary.trim().to_owned(),
        blocked_reason: reason.map(str::to_owned),
    })
    .expect("a stated result is three plain strings")
}

#[cfg(test)]
#[path = "tests/working.rs"]
mod tests;
