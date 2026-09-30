//! A planned project is cut into issues, one scope slice at a time: `warlock
//! draft <PATH>` here, and the panel's `/draft` in [`mod@crate::cutting`], both
//! through [`prepare`], a walk over the [`Planned`] it answers with, [`Filing`] for
//! each slice drafted, and [`Planned::finish`] once the last slice has settled.
//!
//! The split is the promise rather than an arrangement: everything that can
//! refuse — no board, a brief no record names, a project the board has lost or
//! that is not `Planned`, a description that is not a scope, a project with
//! nothing left to cut — is asked by [`prepare`], which sends one read and no
//! mutation, so a refusal costs nothing. Drafting is the door's own business:
//! nothing here opens a session, and a door that drafts hands the drafts back to
//! [`Planned::filing`]. The status is not moved on any road — an issue is created,
//! an edge is written and a comment is said, and nothing else.
//!
//! Nothing a slice drafts becomes an issue on its own. The drafts are printed
//! and a line is read before [`Planned::filing`] is asked for at all, so a skip
//! costs no request, and anything that is neither `accept` nor `skip` is
//! feedback the same session drafts again from. It is the panel's own review
//! window in [`mod@crate::cutting`] with a line typed where that has keys, and
//! the two doors word what they say about a slice out of the same helpers here.
//!
//! What every slice became lives on the [`Planned`] and is written only by
//! [`Planned::settle`], so the edges a later slice asks for are worked out in one
//! place whichever door is driving.
//!
//! No `--json`, matching [`mod@crate::push`] and the other verbs that spend
//! something: the answer worth parsing is the record, which is a file rather
//! than a stream to be caught.
//!
//! No key value is printed here and none can be. [`Planned`], [`Filing`] and
//! [`Announcement`] each carry one with a redacting `Debug`, it is read only on
//! the lines that open a board, and everything that prints takes a
//! [`Destination`], which names the key and never holds it.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use warlock_engine::drafting::Draft;
use warlock_engine::{Destination, Manifest, agent, filed_path, resolve_filing};
use warlock_tui::{
    Board, ChatAgent, Converses, Drafted, Drafting, LinearIssue, LinearOpener, NOTHING_SETTLES_IT,
    Opens, Replied, Slice, propose_answer, scope_block_in,
};

use crate::asking::{self, Asks};
// The module rather than its `cut` and `Slice`, which would both be a second
// name for something this file already has: the slices here are the document's,
// and `cut::Slice` is one slice's drafts on their way to a board.
use crate::cut::{self, Cut, listed};
use crate::error::{Error, one_line};
use crate::push::records;
use crate::standing::{FOR_CUT, Standing};

// The one status a project is read back from, and the only spelling accepted:
// the comparison below trims and folds case, so `planned` and ` Planned ` are
// this and `Backlog` is not.
const PLANNED: &str = "Planned";

// The cursor a read stops on, one mark for both of them: a question, and a
// slice's drafts put up for review. A bare mark rather than a sentence, because
// what a reader needs before they can answer is on the lines above it — for a
// question, warlock's own attempt at it; for drafts, their titles and the two
// words that are read as answers rather than as feedback.
const PROMPT: &str = "> ";

// The two words a review reads as answers. Everything else typed at that prompt
// is feedback about the drafts, so these are the whole of the vocabulary: a
// third word to remember would be a third way to mean "not these".
//
// Case is folded where they are matched, for `is_planned`'s reason: `Accept` is
// somebody answering, not somebody writing feedback in one word.
const ACCEPT: &str = "accept";

const SKIP: &str = "skip";

pub(crate) fn cut(path: &Path, scope: Option<&str>, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_CUT)?;
    // The error rather than `check`'s `.ok()`, for [`mod@crate::push`]'s reason:
    // the sigils under the home pick the board and the key store beside them is
    // what reads it back, so a machine with no home has nothing to cut with
    // rather than an answer of "nothing held".
    let home = Standing::home()?;

    cut_with(
        &standing,
        &home,
        path,
        scope,
        dry_run,
        &LinearOpener,
        // Its own conversation at the register the brief was written in, built
        // once for the run and cheap to build: an agent is a command line and a
        // timeout, so no `claude` exists until a slice asks for a draft. One
        // agent rather than one per slice would be one conversation carrying
        // every earlier slice's talk into the next, which is the opposite of
        // what a per-slice session is for — so the sessions below are opened off
        // this one.
        &ChatAgent::drafting(),
        // And the conversation warlock's own attempt at a question is made in,
        // which is a different one: not the slice's own session, whose next turn
        // is whatever the reader answers. See [`ChatAgent::proposing`]. The
        // panel builds its two the same way and for the same reason.
        &ChatAgent::proposing(),
        // The real read: the prompt on stdout and one cooked line off stdin,
        // which is what a question stops the run at until somebody answers it.
        &mut asking::Stdin,
        &mut io::stdout(),
    )
}

// Split from `cut` the way `pushed` is split from `push`: the environment — the
// working directory, the repository root, the home the sigils and the key store
// sit under — is three parameters rather than three reads, so every refusal can
// be run against a temporary repository and a temporary home, and no test in
// this crate can reach the developer's real key store by standing in the wrong
// directory.
//
// `open` is the socket, `agent` and `proposer` are the models and `ask` is the
// line off stdin, and a test hands in stand-ins that panic when they are
// reached — which is what makes "a dry run drafts nothing" an assertion about
// the order here rather than a reading of it, and what keeps every test in this
// crate off the terminal the developer is sitting at.
//
// Two models and not one, for the panel's reason: a slice's session and the
// conversation warlock's own attempt at its question is made in are two
// conversations, and a caller that handed in one would be opening a session
// warlock does not.
#[expect(
    clippy::too_many_arguments,
    reason = "the environment, the seams and the models are parameters rather \
              than reads, which is the whole of what lets every refusal run \
              against a temporary repository and a temporary home, and every \
              question be answered by a written-down line"
)]
fn cut_with<O: Opens, A: Converses, P: Converses, K: Asks, W: Write>(
    standing: &Standing,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
    dry_run: bool,
    open: &O,
    agent: &A,
    proposer: &P,
    ask: &mut K,
    out: &mut W,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;
    let mut planned = prepare(
        &manifest,
        standing.repo_root(),
        home,
        &standing.target(path),
        scope,
        open,
    )?;

    if dry_run {
        for line in would(planned) {
            say(out, &line);
        }
        return Ok(());
    }

    // Every slice in the cut order, the skips said as well as the work: a
    // reader following along wants to see the whole project go past, and the
    // fraction on each line is its place among all of them.
    while let Some(next) = planned.next() {
        if let Some(issues) = next.already() {
            say(
                out,
                &format!(
                    "{} — already cut as {}, so nothing was sent",
                    next.heading(),
                    listed(issues)
                ),
            );
            continue;
        }

        say(out, &format!("{} — drafting", next.heading()));
        // `?` on the read alone, and not on the drafting: a slice that came to
        // nothing is a line and the next slice, but a stdin that cannot be read
        // is not a fact about this slice — the question after it could not be
        // answered either, and a run that carried on would be asking a sequence
        // of questions into a pipe that is broken.
        let Some(drafts) = drafted(agent, proposer, planned.brief(), next.slice(), ask, out)?
        else {
            continue;
        };

        // `?`, and not a reported line: what reaches here is a team with no
        // `Backlog` state, a create Linear turned down, or a record that would
        // not save — and carrying on to the next slice after any of the three
        // would be warlock filing a second slice into the same wall, or
        // recording nothing about issues that now exist.
        let cut = planned.filing(&next, drafts).file(open, out)?;
        // The edges Linear turned down: an issue that exists with a missing
        // edge is a thing a person can fix on the board, and it is only
        // fixable if they are told.
        if let Settled::Filed { reported, .. } = planned.settle(&next, cut) {
            for line in reported {
                say(out, &line);
            }
        }
    }

    if let Some(announcement) = planned.finish()
        && let Some(line) = announcement.post(open)
    {
        say(out, &line);
    }

    Ok(())
}

/// One slice drafted in one session and reviewed, or `None` with what went
/// wrong — or what was skipped — already said.
///
/// The drafts come back here rather than going on to [`Planned::filing`] on
/// their own: they are printed, a line is read, and only `accept` hands them
/// over. The review is in this call and not beside it because feedback is a turn
/// of *this* session — the conversation that drafted them — and a slice
/// redrafted by a fresh session would be one answering talk nobody in it had
/// heard.
///
/// [`Drafting::for_slice`] rather than [`Drafting::one_shot`]: there is somebody
/// at the shell who started the run and is watching it, so a question is put to
/// them and the line they type is the session's next turn. One session per
/// slice, opened here and dropped at the end of this call, so nothing a slice
/// said reaches the next one.
///
/// The read blocks, which is the whole of what makes the asking worth
/// something: the run waits at the question for as long as whoever started it
/// takes to answer, and nothing here decides on their behalf. A question is put
/// with warlock's own attempt at it over the prompt, and that attempt is a
/// proposal and never an answer — it is sent only when the line read is empty.
fn drafted<A: Converses, P: Converses, K: Asks, W: Write>(
    agent: &A,
    proposer: &P,
    brief: &str,
    slice: &Slice,
    ask: &mut K,
    out: &mut W,
) -> Result<Option<Vec<Draft>>, Error> {
    let mut session = Drafting::for_slice(agent, brief, slice.heading(), slice.prose());
    // What the session had left to spend on the turn that is about to run, read
    // before it rather than after: a question that comes back from a turn there
    // was no round for is the one thing this cannot relay, and after the turn
    // the count has already moved.
    let mut rounds = session.questions_left();
    let mut turned = session.open();

    loop {
        match replied(slice, turned) {
            Reply::Drafts { drafts, lines } => {
                say(out, &drafted_line(slice, &titles(&drafts)));
                for line in lines {
                    say(out, &line);
                }
                match reviewed(slice, drafts, ask, out)? {
                    Reviewed::File(drafts) => return Ok(Some(drafts)),
                    // The line that says so is already printed, here as at a
                    // question: the slice is left uncut, no record names it, and
                    // the next run offers it again.
                    Reviewed::Skip => return Ok(None),
                    Reviewed::Feedback(feedback) => {
                        say(out, &feedback_line(slice, &feedback));
                        // Read before the turn for the reason the opening's is:
                        // a question coming back from a turn there was no round
                        // for is the one thing this cannot relay.
                        rounds = session.questions_left();
                        turned = session.answer(&feedback);
                    }
                }
            }
            // A session counts its own rounds and hands nothing back past the
            // last of them, so this is unreachable — and said rather than
            // panicked on, because a panic in the middle of a run that has
            // filed issues is worth avoiding.
            Reply::Question(_) if rounds == 0 => {
                say(
                    out,
                    &not_drafted(
                        slice,
                        "the session asked a question after its last round was spent",
                    ),
                );
                return Ok(None);
            }
            Reply::Question(question) => {
                say(out, &question_line(slice, &question));
                let Some(answer) = answered(proposer, brief, slice, &question, ask, out)? else {
                    return Ok(None);
                };
                say(out, &answer_line(slice, &answer));
                rounds = session.questions_left();
                turned = session.answer(&answer);
            }
            Reply::Over(line) => {
                say(out, &line);
                return Ok(None);
            }
        }
    }
}

/// What somebody at the shell said to do with one slice's drafts.
///
/// The panel's three answers ([`Reviewed`](warlock_tui::Reviewed)) read off a
/// line rather than off a key, with one difference the road forces: there is no
/// window to light an answer on, so the words are typed and anything that is not
/// one of them is the feedback itself. A reader who has to say what is wrong
/// with the drafts is already typing, and a door that made them say `feedback`
/// first and then the feedback would be asking twice.
///
/// The drafts ride on [`Reviewed::File`] rather than being left with the caller,
/// so the one answer that spends them is the only one that still holds them.
enum Reviewed {
    /// `accept`: these drafts, on their way to the board.
    File(Vec<Draft>),
    /// `skip`, an empty line, or a pipe that ended: the slice left uncut, with
    /// the line that says so already printed.
    Skip,
    /// Anything else, which is what these drafts should be instead.
    Feedback(String),
}

// One slice's drafts put to whoever is at the shell: their titles, the two words
// that are answers, the prompt, and the line that comes back.
//
// Before [`Planned::filing`] is built and long before anything is opened, which
// is the whole of what makes a skip free: nothing here sends, and the road to the
// board starts on `accept` alone.
fn reviewed<K: Asks, W: Write>(
    slice: &Slice,
    drafts: Vec<Draft>,
    ask: &mut K,
    out: &mut W,
) -> Result<Reviewed, Error> {
    say(out, &review_line(slice));

    ask.discard_typed_ahead();
    let Some(line) = ask.ask(PROMPT)? else {
        // EOF, which is Ctrl-D at a terminal and an exhausted pipe everywhere
        // else, read exactly as it is at a question: nobody is there, so nothing
        // is filed and nothing is sent. The newline is because the prompt just
        // asked has none and the cursor is still sitting on it.
        drop(writeln!(out));
        say(out, &unreviewed_line(slice));
        return Ok(Reviewed::Skip);
    };

    // Trimmed for `answered`'s reason: a pipe's trailing newline is never part
    // of what somebody meant to say, and a line of spaces is a line nobody typed
    // anything on.
    let typed = line.trim();
    if typed.eq_ignore_ascii_case(ACCEPT) {
        return Ok(Reviewed::File(drafts));
    }
    // An empty line is the skip, as Enter is at the panel's window: the answer
    // that costs nothing is the one a reader reaches by pressing the key they
    // were already resting on, and drafts are not filed by somebody who typed
    // nothing.
    if typed.is_empty() || typed.eq_ignore_ascii_case(SKIP) {
        say(out, &skipped_line(slice));
        return Ok(Reviewed::Skip);
    }

    Ok(Reviewed::Feedback(typed.to_owned()))
}

// What a draft is said by on the way past: its title, which is what the panel's
// window shows of one and all a line has room for. The bodies are paragraphs,
// and they are what `accept` files.
fn titles(drafts: &[Draft]) -> Vec<String> {
    drafts.iter().map(|draft| draft.title.clone()).collect()
}

// One question put to whoever is at the shell: warlock's attempt at it, the
// prompt, and the line that comes back. `None` is a question nothing answered,
// with the line that says so already printed and the slice left uncut.
//
// Nothing here reads the line for a command or weighs it against the proposal:
// a non-empty line is the answer whatever it says, and an empty one is the
// proposal accepted. That is the whole of the rule, and it is what keeps
// warlock's own attempt an offer rather than a decision.
fn answered<P: Converses, K: Asks, W: Write>(
    proposer: &P,
    brief: &str,
    slice: &Slice,
    question: &str,
    ask: &mut K,
    out: &mut W,
) -> Result<Option<String>, Error> {
    let proposal = proposed(proposer, brief, slice, question, out);

    ask.discard_typed_ahead();
    let Some(line) = ask.ask(PROMPT)? else {
        // EOF, which is Ctrl-D at a terminal and an exhausted pipe everywhere
        // else. Nothing is sent: not the proposal, which is warlock's own
        // attempt and not an answer, and not a blank turn either. The newline is
        // because the prompt just asked has none and the cursor is still sitting
        // on it.
        drop(writeln!(out));
        say(out, &not_drafted(slice, "nobody answered its question"));
        return Ok(None);
    };

    // Trimmed rather than taken as it arrived, for `key::added`'s reason: a
    // pipe's trailing newline is never part of what somebody meant to say, and
    // a line of spaces is a line nobody typed anything on.
    let typed = line.trim();
    if !typed.is_empty() {
        return Ok(Some(typed.to_owned()));
    }
    // An empty line is the proposal accepted, which is the whole of what makes
    // it an offer.
    let Some(proposal) = proposal else {
        // Enter pressed at a question warlock had nothing to offer on: there is
        // no proposal to accept and an empty turn would be the session asked to
        // draft on silence, so the slice is left uncut and said.
        say(
            out,
            &not_drafted(
                slice,
                "nothing was typed and warlock had no answer to propose",
            ),
        );
        return Ok(None);
    };

    Ok(Some(proposal))
}

// Warlock's own attempt at the question, made in its own conversation and said
// as it is made.
//
// `None` is a question there is nothing to offer on — the sentence
// [`propose_answer`] hands back when the brief, the slice and the repository do
// not settle it, or an attempt that never came back at all — and either way the
// line above the prompt says which, the read below happens anyway, and the
// answer is entirely whoever is reading's.
//
// One turn and no retry, exactly as the panel makes the same attempt: the
// failures that reach here are a missing binary, a cancel and a timeout, and
// none of the three is better the second time.
fn proposed<P: Converses, W: Write>(
    proposer: &P,
    brief: &str,
    slice: &Slice,
    question: &str,
    out: &mut W,
) -> Option<String> {
    match propose_answer(proposer, brief, slice.heading(), slice.prose(), question) {
        // Recognised by [`propose_answer`] and not re-read here: the one place
        // that sentence is told from a proposal is the one that asked for it,
        // and a second reader would eventually disagree with it.
        Ok(proposal) if proposal == NOTHING_SETTLES_IT => {
            say(out, &settled_line(slice, &proposal));
            None
        }
        Ok(proposal) => {
            say(out, &proposal_line(slice, &proposal));
            Some(proposal)
        }
        Err(error) => {
            say(out, &unproposed_line(slice, &one_line(&error.to_string())));
            None
        }
    }
}

// A failed write is ignored, exactly as `running.rs`'s `Progress` ignores one
// and for its reason: `warlock draft docs/brief.md | head -1` is a closed stdout,
// and failing a run of drafting sessions over the state of a pipe would spend
// somebody's tokens and then throw away what they bought.
fn say<W: Write>(out: &mut W, fact: &str) {
    drop(writeln!(out, "warlock: {fact}"));
}

// One line for the run and one per slice, rather than push's single sentence:
// what a cut is about to do is an order, and an order is not a thing one line
// can say.
fn would(mut planned: Planned) -> Vec<String> {
    let destination = planned.destination();
    let mut lines = vec![format!(
        "would cut `{}`, which is `{}`, into `{}` under the scope `{}` — {}, {} already cut, and \
         nothing was drafted",
        planned.name(),
        planned.status(),
        destination.team(),
        destination.scope(),
        counted(planned.total()),
        planned.total() - planned.left()
    )];

    while let Some(next) = planned.next() {
        lines.push(match next.already() {
            Some(issues) => format!("{} — already cut as {}", next.heading(), listed(issues)),
            None => next.heading(),
        });
    }

    lines
}

/// A project read back and gated, with every slice's standing: what is left to
/// cut, what earlier runs cut it into, and what this run has created so far.
pub(crate) struct Planned {
    root: PathBuf,
    // The brief as `.warlock/filed.toml` spells it, which is what a cut record
    // is appended under.
    spelled: String,
    project: String,
    name: String,
    // The board's own spelling and not `Planned`, because that is what somebody
    // sent to look will find written on the project: the gate folds case and
    // trims, so the two need not match.
    status: String,
    brief: String,
    destination: Destination,
    value: String,
    // The user the key belongs to, resolved once by `prepare`: every issue this
    // run files is assigned to them, and nothing here can name anybody else.
    assignee: String,
    // `ordered` and not `slices`: the order is the order tickets are filed and
    // blocked in, so a slice is only reached once everything it waits on has
    // been.
    slices: Vec<Slice>,
    // Beside `slices`, one entry each: the identifiers a cut record names, which
    // is all such a record keeps of an issue, and `None` for a slice still to
    // draft.
    already: Vec<Option<Vec<String>>>,
    // One entry per slice of the *document*, indexed by position: what a
    // `depends_on` names is a position. Seeded from the cut records, so a slice
    // an earlier run filed can still be named as a blocker, and filled as this
    // run's slices settle. Empty is "nothing this run can name as a blocker",
    // and an empty entry is left out of a cut's `needs` rather than written as
    // an edge to nothing.
    became: Vec<Vec<LinearIssue>>,
    // What this run created, for the project's one comment. An earlier run's
    // issues are not in it: they were named by an earlier run's comment.
    created: Vec<String>,
    at: usize,
}

// Hand-written for `Target`'s reason: this holds the key value, and the panel
// keeps one across rounds, which a failing assertion anywhere in the suite may
// print.
impl fmt::Debug for Planned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Planned")
            .field("root", &self.root)
            .field("spelled", &self.spelled)
            .field("project", &self.project)
            .field("name", &self.name)
            .field("status", &self.status)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .field("assignee", &self.assignee)
            .field("slices", &self.slices)
            .field("already", &self.already)
            .field("became", &self.became)
            .field("created", &self.created)
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

// `path` is joined onto nothing here: the subcommand hands in its argument
// joined onto the working directory and the panel its manifest spelling joined
// onto the root, and both are spelled back against the root below, which is
// what a record is keyed by.
//
// The board and the brief's own record are resolved before the key is read, so
// a machine that cannot say which board it is standing at, or a brief nothing
// filed, refuses without a request. A project with no slice left to cut is
// refused too, because a run with nothing to draft is not a run that succeeded
// quietly.
pub(crate) fn prepare<O: Opens>(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
    open: &O,
) -> Result<Planned, Error> {
    let target =
        resolve_filing(manifest, root, home, scope).map_err(|source| Error::Filing { source })?;

    let spelled = crate::query::spelled(root, path)?;
    // `records` rather than a second `Filed::load`: the file a push appends to
    // and the file a cut resolves against are one file, and a second loader
    // here would be a second reading of what a missing one means.
    let filed = records(root)?;
    let Some(record) = filed.record(&spelled) else {
        return Err(Error::NoRecord { path: spelled });
    };

    let board = open.open(target.value());
    // Once for the run, here rather than in `cut::cut`, which is called per
    // slice: the id is the same for every issue the run files, and asking per
    // slice — or per draft — would be a request bought again for an answer that
    // cannot have changed. Before the project read for the reason every other
    // refusal is asked here: a board that will not say who the key belongs to
    // has nothing to file for, and that is not a thing to find out once issues
    // exist.
    let assignee = board.viewer()?;

    let project =
        board
            .fetch_project(record.project_id())?
            .ok_or_else(|| Error::UnknownProject {
                id: record.project_id().to_owned(),
                path: filed_path(root),
            })?;

    let Some(status) = project.status().filter(|status| is_planned(status)) else {
        return Err(Error::NotPlanned {
            path: spelled,
            status: project.status().map(ToOwned::to_owned),
        });
    };

    let block = scope_block_in(project.content()).map_err(|source| Error::ScopeBlock { source })?;
    let ordered = block.ordered();
    // Asked of the file rather than worked out here: `cut_state` matches on the
    // key a record spells and not on a fresh fold of the title beside it, which
    // is the difference between a slice somebody renamed and one warlock quietly
    // treats as already filed.
    let state = filed.cut_state(&spelled, ordered.iter().map(|slice| slice.heading()));
    if state.uncut().is_empty() {
        return Err(Error::AllCut { path: spelled });
    }

    let already: Vec<Option<Vec<String>>> = ordered
        .iter()
        .map(|slice| {
            state
                .cut()
                .iter()
                .find(|(title, _)| *title == slice.heading())
                .map(|(_, cut)| cut.issues().to_vec())
        })
        .collect();
    let mut became = vec![Vec::new(); ordered.len()];
    for (slice, issues) in ordered.iter().zip(&already) {
        if let (Some(issues), Some(entry)) = (issues, became.get_mut(at(slice))) {
            *entry = recorded(issues);
        }
    }

    Ok(Planned {
        root: root.to_path_buf(),
        spelled,
        project: record.project_id().to_owned(),
        name: project.name().to_owned(),
        status: status.to_owned(),
        brief: block.brief().to_owned(),
        destination: target.destination(),
        value: target.value().to_owned(),
        assignee,
        slices: ordered.into_iter().cloned().collect(),
        already,
        became,
        created: Vec::new(),
        at: 0,
    })
}

impl Planned {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    pub(crate) fn brief(&self) -> &str {
        &self.brief
    }

    pub(crate) const fn destination(&self) -> &Destination {
        &self.destination
    }

    /// How many slices the project has.
    pub(crate) const fn total(&self) -> usize {
        self.slices.len()
    }

    /// How many of them no cut record names.
    pub(crate) fn left(&self) -> usize {
        self.already
            .iter()
            .filter(|issues| issues.is_none())
            .count()
    }

    /// The next slice in the cut order, cut or not, numbered among all of them.
    pub(crate) fn next(&mut self) -> Option<Next> {
        let slice = self.slices.get(self.at)?.clone();
        let next = Next {
            slice,
            place: self.at,
            total: self.slices.len(),
            already: self.already[self.at].clone(),
        };
        self.at += 1;
        Some(next)
    }

    /// The next slice in the cut order that no cut record names, numbered among
    /// those alone: a walk that only drafts counts what it is drafting.
    pub(crate) fn next_uncut(&mut self) -> Option<Next> {
        while self.already.get(self.at)?.is_some() {
            self.at += 1;
        }
        let slice = self.slices[self.at].clone();
        let place = self.already[..self.at]
            .iter()
            .filter(|issues| issues.is_none())
            .count();
        self.at += 1;
        Some(Next {
            slice,
            place,
            total: self.left(),
            already: None,
        })
    }

    /// One slice's drafts made ready for the board, with the issues the slices
    /// it depends on became.
    ///
    /// Owned, so it can cross to a worker: the panel files on one while the run
    /// on its own side goes on living.
    pub(crate) fn filing(&self, next: &Next, drafts: Vec<Draft>) -> Filing {
        let needs = next
            .slice
            .depends_on()
            .iter()
            .filter_map(|position| self.became.get(position.saturating_sub(1)))
            .filter(|issues| !issues.is_empty())
            .cloned()
            .collect();

        Filing {
            root: self.root.clone(),
            brief: self.spelled.clone(),
            project: self.project.clone(),
            destination: self.destination.clone(),
            value: self.value.clone(),
            assignee: self.assignee.clone(),
            title: next.slice.heading().to_owned(),
            drafts,
            needs,
        }
    }

    /// What a [`Filing`] came to, written down where a later slice's `needs`
    /// will look.
    pub(crate) fn settle(&mut self, next: &Next, cut: Cut) -> Settled {
        let entry = self.became.get_mut(at(&next.slice));
        match cut {
            // Unreachable while the walk skips what the same file already
            // names, and handled rather than asserted: a panic in the middle of
            // a run that has filed issues is worth avoiding.
            Cut::Already(issues) => {
                if let Some(entry) = entry {
                    *entry = recorded(&issues);
                }
                Settled::Already(issues)
            }
            Cut::Filed { issues, reported } => {
                let identifiers: Vec<String> = issues
                    .iter()
                    .map(|issue| issue.identifier().to_owned())
                    .collect();
                self.created.extend(identifiers.iter().cloned());
                if let Some(entry) = entry {
                    *entry = issues;
                }
                Settled::Filed {
                    issues: identifiers,
                    reported,
                }
            }
        }
    }

    /// The project's one comment, owed only when this run created something.
    ///
    /// Asked once, after the last slice settles — including a run somebody
    /// stopped partway: a comment per slice would be a comment per cut rather
    /// than per run, and a run that created nothing has nothing to say on the
    /// project.
    pub(crate) fn finish(&self) -> Option<Announcement> {
        (!self.created.is_empty()).then(|| Announcement {
            project: self.project.clone(),
            issues: self.created.clone(),
            value: self.value.clone(),
        })
    }
}

fn at(slice: &Slice) -> usize {
    slice.position().saturating_sub(1)
}

fn recorded(issues: &[String]) -> Vec<LinearIssue> {
    issues
        .iter()
        .map(|issue| LinearIssue::recorded(issue))
        .collect()
}

/// One slice a walk reached, with its place in that walk.
#[derive(Debug, Clone)]
pub(crate) struct Next {
    slice: Slice,
    place: usize,
    total: usize,
    already: Option<Vec<String>>,
}

impl Next {
    pub(crate) const fn slice(&self) -> &Slice {
        &self.slice
    }

    pub(crate) fn already(&self) -> Option<&[String]> {
        self.already.as_deref()
    }

    /// How many slices of the walk come after this one.
    pub(crate) const fn left(&self) -> usize {
        self.total - self.place - 1
    }

    // The fraction is the place in the walk, one-based as `running.rs`'s is, and
    // the position is where the slice sits in the document — the two differ
    // exactly when a `depends_on` line moved something or a slice was already
    // cut, and the second is what finds the slice in the brief.
    pub(crate) fn heading(&self) -> String {
        format!("[{}/{}] {}", self.place + 1, self.total, named(&self.slice))
    }
}

/// One slice's drafts on their way to the board.
pub(crate) struct Filing {
    root: PathBuf,
    brief: String,
    project: String,
    destination: Destination,
    value: String,
    assignee: String,
    title: String,
    drafts: Vec<Draft>,
    needs: Vec<Vec<LinearIssue>>,
}

impl fmt::Debug for Filing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Filing")
            .field("root", &self.root)
            .field("brief", &self.brief)
            .field("project", &self.project)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .field("assignee", &self.assignee)
            .field("title", &self.title)
            .field("drafts", &self.drafts)
            .field("needs", &self.needs)
            .finish()
    }
}

impl Filing {
    // `out` is where [`cut::cut`]'s own lines go: stdout for the subcommand, and
    // `io::sink` for the panel, which words its own beside the slice they are
    // about.
    pub(crate) fn file<O: Opens, W: Write>(&self, open: &O, out: &mut W) -> Result<Cut, Error> {
        let needs: Vec<&[LinearIssue]> = self.needs.iter().map(Vec::as_slice).collect();

        cut::cut(
            &open.open(&self.value),
            &self.root,
            cut::Filing {
                brief: &self.brief,
                project: &self.project,
                destination: &self.destination,
                assignee: &self.assignee,
            },
            cut::Slice {
                title: &self.title,
                drafts: &self.drafts,
                needs: &needs,
            },
            out,
        )
    }
}

/// What a filed slice came to, by identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settled {
    Already(Vec<String>),
    /// The issues created, with one line per edge Linear turned down.
    Filed {
        issues: Vec<String>,
        reported: Vec<String>,
    },
}

/// The project's one comment, naming what this run created.
pub(crate) struct Announcement {
    project: String,
    issues: Vec<String>,
    value: String,
}

impl fmt::Debug for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Announcement")
            .field("project", &self.project)
            .field("issues", &self.issues)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl Announcement {
    /// Said on the project, answering with the one line to report when Linear
    /// turns it down.
    ///
    /// A refusal is a reported line and not a failure: the issues exist and are
    /// recorded by the time it is said, and losing a run over a note on a
    /// project would be the tail wagging the cut.
    pub(crate) fn post<O: Opens>(&self, open: &O) -> Option<String> {
        cut::announce(&open.open(&self.value), &self.project, &self.issues)
    }
}

/// What one drafting turn came to, worded for whichever door is reporting it.
#[derive(Debug)]
pub(crate) enum Reply {
    /// The drafts, with one line per repair warlock made to get them.
    Drafts {
        drafts: Vec<Draft>,
        lines: Vec<String>,
    },
    /// Prose with rounds left, which only a door with somebody to ask can
    /// relay.
    Question(String),
    /// The slice left uncut, and the line that says why.
    Over(String),
}

// Every way a slice can fail to draft is a line and the next slice, not the end
// of the run: the slices left are other work, they were ordered so that nothing
// is filed before what it waits on, and a run that stopped would leave the
// operator re-running it to reach them anyway.
pub(crate) fn replied(slice: &Slice, replied: Result<Replied, agent::Error>) -> Reply {
    match replied {
        // A repaired draft is a ticket that was drafted, not one that was
        // missed, and a log read tomorrow should be able to tell the two apart.
        Ok(Replied::Answer(Drafted::Drafts { fill, repairs })) => Reply::Drafts {
            drafts: fill.drafts,
            lines: repairs
                .iter()
                .map(|repair| format!("{} — {repair}", named(slice)))
                .collect(),
        },
        // Reported and left uncut rather than filed as the stand-in ticket the
        // document road's floor would supply: a supplied ticket is warlock
        // putting work nobody planned on somebody's board.
        Ok(Replied::Answer(Drafted::Unusable(defect))) => Reply::Over(not_drafted(slice, defect)),
        Ok(Replied::Question(question)) => Reply::Question(question),
        // A missing binary, a timeout or a cancel, none of which is better the
        // second time — see [`Drafting`]'s own note — so the slice is left
        // uncut rather than asked again.
        Err(error) => Reply::Over(not_drafted(slice, error)),
    }
}

pub(crate) fn not_drafted(slice: &Slice, why: impl fmt::Display) -> String {
    format!("{} was not drafted: {why}", named(slice))
}

// The four lines a relayed question is read back by, here rather than beside
// either door: the shell and the panel put the same question to a person and
// send whatever they say, and two copies of these wordings would be two doors
// naming one conversation differently. `named` is in each of them for the same
// reason it is `pub(crate)`.

// The question in the words it was asked, flattened as a line takes it. Whoever
// is at the door answers it, so this line and [`answer_line`] are the pair a
// conversation is read back by: `asked` is the slice talking and `answered` is
// the person, and the two verbs are the whole of how a reader tomorrow tells one
// from the other.
pub(crate) fn question_line(slice: &Slice, question: &str) -> String {
    format!("{} asked: {}", named(slice), one_line(question))
}

// What was sent, in the words it was sent in, and the other half of that pair.
// Warlock's attempt and something typed over it land here identically on
// purpose: what went to the session is what the person's line came to, and a
// line that said which of the two it was would be warlock reporting its own
// draft rather than the answer.
pub(crate) fn answer_line(slice: &Slice, answer: &str) -> String {
    format!("{} was answered: {}", named(slice), one_line(answer))
}

// The drafts as they arrived, by title: the panel says this over the repairs and
// the shell says it over the prompt, and it is the one line a reader decides
// about them from.
pub(crate) fn drafted_line(slice: &Slice, titles: &[String]) -> String {
    format!("{} — drafted {}", named(slice), listed(titles))
}

// What can be said back about them, on the road where there is nothing to light
// an answer on. The two words first and the third answer last, because the third
// is whatever the reader has to say and not a word to remember.
fn review_line(slice: &Slice) -> String {
    format!(
        "{} — `{ACCEPT}` to file these, Enter or `{SKIP}` to leave it for another run, or say \
         what these drafts should be instead",
        named(slice)
    )
}

// A slice left alone. `nothing was recorded` rather than `skipped` alone,
// because what a reader wants to know tomorrow is whether the next draft will
// offer this slice again — and it will.
pub(crate) fn skipped_line(slice: &Slice) -> String {
    format!("{} was skipped; nothing was recorded for it", named(slice))
}

// The same slice left alone by a pipe that ended rather than by somebody saying
// so. It is the skip either way — nothing was filed and nothing was recorded —
// and which of the two it was is worth a reader's while tomorrow.
fn unreviewed_line(slice: &Slice) -> String {
    format!(
        "{} was skipped: nobody said what to do with its drafts, so nothing was recorded for it",
        named(slice)
    )
}

// What the reader told the slice about its drafts, in their own words and the
// other half of the pair [`question_line`] and [`answer_line`] make: the verb
// says this was feedback rather than an answer to anything the slice asked.
pub(crate) fn feedback_line(slice: &Slice, feedback: &str) -> String {
    format!(
        "{} is being redrafted: {}",
        named(slice),
        one_line(feedback)
    )
}

// Warlock's own attempt, offered over the prompt on the road where there is no
// field to put it in. Said as a proposal and never as the answer: it is sent
// only if the line read is empty, and a reader who types anything at all has
// replaced it.
pub(crate) fn proposal_line(slice: &Slice, proposal: &str) -> String {
    format!(
        "{} — warlock's answer: {}",
        named(slice),
        one_line(proposal)
    )
}

// The proposing session having nothing to offer, said in the sentence
// [`propose_answer`] hands back and no other words: the question is still
// somebody's, and the answer is entirely theirs.
pub(crate) fn settled_line(slice: &Slice, settles: &str) -> String {
    format!("{} — {settles}", named(slice))
}

// An attempt that never came back with anything. One line and the question left
// standing: nothing was sent, and whoever is at the door answers in their own
// words.
pub(crate) fn unproposed_line(slice: &Slice, why: &str) -> String {
    format!("{} — no answer was proposed: {why}", named(slice))
}

// The prefix of every line about one slice that is not its place in a walk: the
// position in the document and the heading, which is what a reader takes back
// to the brief. `pub(crate)` for the panel's own lines about a slice, so the
// two doors cannot come to name the same work differently.
pub(crate) fn named(slice: &Slice) -> String {
    format!("slice {} `{}`", slice.position(), slice.heading())
}

// `1 slice`, `9 slices`, so a count is not worded twice or read as `1 slices`.
pub(crate) fn counted(count: usize) -> String {
    let noun = if count == 1 { "slice" } else { "slices" };
    format!("{count} {noun}")
}

// Trimmed and case-folded because the name is typed into Linear by a person and
// read back over a wire, and neither end promises the capitalisation warlock
// filed it under. Nothing wider than that: `Planned` is the gate, so a workspace
// that spells its planned column something else is a refusal rather than a
// guess.
fn is_planned(status: &str) -> bool {
    status.trim().eq_ignore_ascii_case(PLANNED)
}

// Every test drives a temporary repository and a temporary home through the
// [`Board`] seam, so none of them opens a socket or reads a key store that is
// not its own.
#[cfg(test)]
#[path = "tests/planned.rs"]
mod tests;
