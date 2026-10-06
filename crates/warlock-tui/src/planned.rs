//! A planned project is cut into issues, one scope slice at a time: `warlock
//! draft <SCOPE> <SLUG>` here, and the panel's `/draft` in [`mod@crate::cutting`],
//! both through [`prepare`], a walk over the [`Planned`] it answers with, and
//! [`Filing`] for each slice drafted. Without a slug, both doors print
//! [`listing`]'s lines instead and stop: picking a project is whoever is at the
//! door's job, and warlock takes only the exact slug.
//!
//! The split is the promise rather than an arrangement: everything that can
//! refuse — no board, a slug the board does not know, a project that is not
//! `Planned`, a description that is not a scope, a project with nothing left to
//! cut — is asked by [`prepare`], which sends reads and no mutation, so a
//! refusal costs nothing. Drafting is the door's own business: nothing here
//! opens a session, and a door that drafts hands the drafts back to
//! [`Planned::filing`]. A run creates issues, writes edges and says notes, and
//! moves the project to `In Progress` once every slice is settled and at least
//! one became issues: before that a project left in `Planned` is one the
//! listing still offers, which is what a run stopped halfway needs.
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
//! No key value is printed here and none can be. [`Planned`], [`Filing`] and
//! [`Skipping`] each carry one with a redacting `Debug`, it is read only on the
//! lines that open a board, and everything that prints takes a [`Destination`],
//! which names the key and never holds it.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use warlock_engine::drafting::{self, Draft};
use warlock_engine::{Destination, Manifest, agent, resolve_filing};

use crate::asking::{self, Asks};
use crate::brief::{Slice, scope_block_in};
use crate::claude::{
    Activities, Activity, ChatAgent, Converses, Drafted, Drafting, NOTHING_SETTLES_IT, Replied,
    propose_answer,
};
use crate::linear::{
    Board, Issue as LinearIssue, Listing, Opener as LinearOpener, Opens, QueuedIssue,
};
// The module rather than its `cut` and `Slice`, which would both be a second
// name for something this file already has: the slices here are the document's,
// and `cut::Slice` is one slice's drafts on their way to a board.
use crate::cut::{self, Cut, fold_title, listed};
use crate::editing::{self, NO_EDITOR, run_editor};
use crate::error::{Error, one_line};
use crate::pull::activity_line;
use crate::standing::{FOR_CUT, Standing};

// The one status a project is read back from, and the only spelling accepted:
// the comparison below trims and folds case, so `planned` and ` Planned ` are
// this and `Backlog` is not.
const PLANNED: &str = "Planned";

// The cursor a question stops on. A bare mark rather than a sentence, because
// what a reader needs before they can answer is on the lines above it:
// warlock's own attempt at it.
const PROMPT: &str = "> ";

// Forman's review words, read as Forman's `parse_decision` reads them: case
// folded and trimmed, an empty line is assent, and anything that is none of them
// is feedback, because the alternative is scolding somebody for describing what
// they want in their own words.
const CREATE: [&str; 4] = ["c", "create", "y", "yes"];

const QUIT: [&str; 4] = ["q", "quit", "n", "no"];

const EDIT: [&str; 2] = ["e", "edit"];

// Red's question after a skip, and its default: a run carries on by somebody
// saying so.
const CARRY_PROMPT: &str = "carry on with the remaining slices? [y/N] ";

// How many open tickets a session is shown: Forman's number, enough to cover a
// project's worth of in-flight work without turning the brief into a backlog.
const BACKLOG_LIMIT: usize = 40;

// `WAR-9` before `WAR-10`.
fn natural(identifier: &str) -> (String, u64) {
    match identifier.rsplit_once('-') {
        Some((team, number)) => (team.to_owned(), number.parse().unwrap_or(u64::MAX)),
        None => (identifier.to_owned(), u64::MAX),
    }
}

pub fn cut(scope: &str, project: Option<&str>, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_CUT)?;
    // The error rather than `check`'s `.ok()`, for [`mod@crate::push`]'s reason:
    // the sigils under the home pick the board and the key store beside them is
    // what reads it back, so a machine with no home has nothing to cut with
    // rather than an answer of "nothing held".
    let home = Standing::home()?;

    let Some(project) = project else {
        let manifest = standing.manifest()?;
        let (destination, listing) =
            listing(&manifest, standing.repo_root(), &home, scope, &LinearOpener)?;
        print_listing(&mut io::stdout(), &destination, &listing);
        return Ok(());
    };

    cut_with(
        &standing,
        &home,
        scope,
        project,
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
        printing,
    )
}

// What a slice's session is seen doing, on stdout as it happens rather than
// after the turn: a slice is minutes of a session working, and a shell that
// said `drafting` and then nothing reads as hung. Straight to stdout because
// [`Activities`] wants a `'static` sink and `out` is borrowed; on this road the
// two are the same stream, and nothing is written to `out` while a turn runs.
fn printing() -> Activities {
    let watched = Mutex::new(Watched::default());
    Activities::new(move |activity| {
        let line = watched
            .lock()
            .expect("nothing panics holding the watch")
            .line(&activity);
        if let Some(line) = line {
            say(&mut io::stdout(), &line);
        }
    })
}

/// The activity lines of one session, with a stretch of thinking or writing
/// said once: a session reports `Writing` again with every delta, and printed on
/// a screen that only scrolls that is a column of identical lines. A tool line
/// repeats only when the session did the same thing twice, so it is never
/// collapsed. `/pull`'s shell door collapses the same way.
#[derive(Debug, Default)]
struct Watched {
    said: Option<String>,
}

impl Watched {
    fn line(&mut self, activity: &Activity) -> Option<String> {
        let line = activity_line(activity)?;
        let running = matches!(activity, Activity::Thinking | Activity::Writing { .. });
        if running && self.said.as_deref() == Some(line.as_str()) {
            return None;
        }
        self.said = Some(line.clone());
        Some(line)
    }
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
    scope: &str,
    project: &str,
    dry_run: bool,
    open: &O,
    agent: &A,
    proposer: &P,
    ask: &mut K,
    out: &mut W,
    watching: fn() -> Activities,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;
    let mut planned = prepare(&manifest, standing.repo_root(), home, scope, project, open)?;

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
                    "{} — {}, so nothing was sent",
                    next.heading(),
                    cut::settled_as(issues)
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
        let drafts = match drafted(
            agent,
            proposer,
            &planned.drafting_brief(),
            next.slice(),
            ask,
            out,
            watching(),
        )? {
            Ended::File(drafts) => drafts,
            Ended::Left => continue,
            Ended::Skipped => {
                match planned.skipping(&next).post(open) {
                    Ok(moved) => {
                        if let Some(line) = moved {
                            say(out, &line);
                        }
                    }
                    Err(error) => say(out, &error.to_string()),
                }
                if next.left() == 0 {
                    continue;
                }
                ask.discard_typed_ahead();
                let carry = ask.ask(CARRY_PROMPT)?.unwrap_or_default();
                if matches!(carry.trim().to_lowercase().as_str(), "y" | "yes") {
                    continue;
                }
                say(out, &stopped_line(next.left()));
                break;
            }
        };

        // `?`, and not a reported line: what reaches here is a team with no
        // `Backlog` state, a create Linear turned down, or a note it would not
        // take — and carrying on to the next slice after any of the three would
        // be warlock filing a second slice into the same wall, or noting
        // nothing about issues that now exist.
        let cut = planned.filing(&next, drafts).file(open, out)?;
        // The edges Linear turned down: an issue that exists with a missing
        // edge is a thing a person can fix on the board, and it is only
        // fixable if they are told.
        for line in planned.settle(&next, cut).reported {
            say(out, &line);
        }
    }

    Ok(())
}

/// The projects a bare `warlock draft <SCOPE>` names, read off the board the
/// scope files to.
pub(crate) fn listing<O: Opens>(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    scope: &str,
    open: &O,
) -> Result<(Destination, Listing), Error> {
    let target = resolve_filing(manifest, root, home, Some(scope))
        .map_err(|source| Error::Filing { source })?;
    let destination = target.destination();
    let listing = open
        .open(target.value())
        .planned_projects(destination.team_key(), destination.label())?;

    Ok((destination, listing))
}

// The project lines bare rather than behind `warlock: `, so the slug is the
// first word of its line and nothing has to be cut away before it is typed back.
fn print_listing<W: Write>(out: &mut W, destination: &Destination, listing: &Listing) {
    for line in listing_lines(destination, listing) {
        drop(writeln!(out, "{line}"));
    }
}

/// One line per project, `slug  name`, or one line saying so when there is
/// nothing to list. The panel notes the same lines, so the two doors list
/// projects identically.
pub(crate) fn listing_lines(destination: &Destination, listing: &Listing) -> Vec<String> {
    if listing.projects().is_empty() {
        return vec![format!(
            "warlock: no project in `{}` is `Planned` and labelled `{}`",
            destination.team_key(),
            destination.label()
        )];
    }

    listing
        .projects()
        .iter()
        .map(|(slug, name)| format!("{slug}  {name}"))
        .collect()
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
    watching: Activities,
) -> Result<Ended, Error> {
    let mut session =
        Drafting::for_slice(agent, brief, slice.heading(), slice.prose()).reporting(watching);
    // What the session had left to spend on the turn that is about to run, read
    // before it rather than after: a question that comes back from a turn there
    // was no round for is the one thing this cannot relay, and after the turn
    // the count has already moved.
    let mut rounds = session.questions_left();
    let mut turned = session.open();

    loop {
        match replied(slice, turned) {
            Reply::Drafts { mut drafts, lines } => {
                say(out, &drafted_line(slice, &titles(&drafts)));
                for line in lines {
                    say(out, &line);
                }
                // Round again after an edit: the drafts as saved are printed
                // and asked about, exactly as Forman's loop goes back to its
                // prompt with them.
                let feedback = loop {
                    for line in drafts_document(slice, &drafts) {
                        say(out, &line);
                    }
                    match reviewed(slice, drafts, ask, out)? {
                        Reviewed::File(drafts) => return Ok(Ended::File(drafts)),
                        // The line that says so is already printed in both.
                        Reviewed::Skip => return Ok(Ended::Skipped),
                        Reviewed::Unanswered => return Ok(Ended::Left),
                        Reviewed::Edit(before) => {
                            drafts = match editing::editor() {
                                None => {
                                    say(out, &unedited_line(slice, NO_EDITOR));
                                    before
                                }
                                Some(editor) => {
                                    match edited_drafts(slice, &before, |path| {
                                        run_editor(&editor, path)
                                    }) {
                                        Ok(edited) => edited,
                                        Err(why) => {
                                            say(out, &unedited_line(slice, &why));
                                            before
                                        }
                                    }
                                }
                            };
                        }
                        Reviewed::Feedback(feedback) => break feedback,
                    }
                };
                say(out, &feedback_line(slice, &feedback));
                // Read before the turn for the reason the opening's is: a
                // question coming back from a turn there was no round for is the
                // one thing this cannot relay.
                rounds = session.questions_left();
                turned = session.answer(&feedback);
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
                return Ok(Ended::Left);
            }
            Reply::Question(question) => {
                for line in question_lines(slice, &question) {
                    say(out, &line);
                }
                let Some(answer) = answered(proposer, brief, slice, &question, ask, out)? else {
                    return Ok(Ended::Left);
                };
                say(out, &answer_line(slice, &answer));
                rounds = session.questions_left();
                turned = session.answer(&answer);
            }
            Reply::Over(line) => {
                say(out, &line);
                return Ok(Ended::Left);
            }
        }
    }
}

/// How one slice's drafting came out at the shell.
enum Ended {
    /// Accepted, on their way to the board.
    File(Vec<Draft>),
    /// Somebody said no at the review: noted, and never offered again.
    Skipped,
    /// Nothing to file and nothing decided — a session that failed, a question
    /// nobody answered, a pipe that ended — so the next run offers it again.
    Left,
}

/// What somebody at the shell said to do with one slice's drafts, read the way
/// Forman's terminal reviewer reads it. The drafts ride on [`Reviewed::File`]
/// rather than being left with the caller, so the one answer that spends them
/// is the only one that still holds them.
enum Reviewed {
    File(Vec<Draft>),
    /// `[e]dit`, holding the drafts to open.
    Edit(Vec<Draft>),
    Skip,
    /// A pipe that ended or Ctrl-D: nobody is there, so it is no answer at all.
    Unanswered,
    Feedback(String),
}

// One slice's drafts put to whoever is at the shell, after every draft has been
// printed whole: Forman's prompt, the line that comes back, and what it means.
//
// Before [`Planned::filing`] is built and long before anything is opened, which
// is the whole of what makes a skip free: nothing here sends, and the road to the
// board starts on a create alone.
fn reviewed<K: Asks, W: Write>(
    slice: &Slice,
    drafts: Vec<Draft>,
    ask: &mut K,
    out: &mut W,
) -> Result<Reviewed, Error> {
    ask.discard_typed_ahead();
    let Some(line) = ask.ask(&review_prompt(drafts.len()))? else {
        // EOF, which is Ctrl-D at a terminal and an exhausted pipe everywhere
        // else, read exactly as it is at a question: nobody is there, so nothing
        // is filed, nothing is sent and nothing is recorded. The newline is
        // because the prompt just asked has none and the cursor is still on it.
        drop(writeln!(out));
        say(out, &unreviewed_line(slice));
        return Ok(Reviewed::Unanswered);
    };

    let typed = line.trim();
    let folded = typed.to_lowercase();
    if typed.is_empty() || CREATE.contains(&folded.as_str()) {
        return Ok(Reviewed::File(drafts));
    }
    if QUIT.contains(&folded.as_str()) {
        say(out, &skipped_line(slice));
        return Ok(Reviewed::Skip);
    }
    if EDIT.contains(&folded.as_str()) {
        return Ok(Reviewed::Edit(drafts));
    }

    Ok(Reviewed::Feedback(typed.to_owned()))
}

// Forman's own prompt, count and all.
fn review_prompt(count: usize) -> String {
    format!("[c]reate {count} ticket(s), [e]dit, [q]uit, or type feedback to redraft: ")
}

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
        destination.team_key(),
        destination.scope(),
        counted(planned.total()),
        planned.total() - planned.left()
    )];

    while let Some(next) = planned.next() {
        lines.push(match next.already() {
            Some(issues) => format!("{} — {}", next.heading(), cut::settled_as(issues)),
            None => next.heading(),
        });
    }

    lines
}

/// A project read back and gated, with every slice's standing: what is left to
/// cut, what earlier runs cut it into, and what this run has created so far.
pub(crate) struct Planned {
    root: PathBuf,
    // Linear's id for the project, which issues are created in and notes are
    // said on.
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
    // Beside `slices`, one entry each: the identifiers a cut note names, which
    // is all such a note keeps of an issue, and `None` for a slice still to
    // draft. Fixed for the run, because the walk's counts are read off it.
    already: Vec<Option<Vec<String>>>,
    // Beside `slices` too, and the one that moves: whether each slice is
    // settled — `Some(true)` for issues, `Some(false)` for a skip — by an
    // earlier run or this one. What decides that a slice's filing or skip is
    // the one that finishes the project.
    settled: Vec<Option<bool>>,
    // The open tickets the scope's queue held when the run started: Forman's
    // backlog digest, and what a draft's `waits_on` resolves against.
    open: Vec<QueuedIssue>,
    // One entry per slice of the *document*, indexed by position: what a
    // `depends_on` names is a position. Seeded from the cut notes, so a slice
    // an earlier run filed can still be named as a blocker, and filled as this
    // run's slices settle. Empty is "nothing this run can name as a blocker",
    // and an empty entry is left out of a cut's `needs` rather than written as
    // an edge to nothing.
    became: Vec<Vec<LinearIssue>>,
    at: usize,
}

// Hand-written for `Target`'s reason: this holds the key value, and the panel
// keeps one across rounds, which a failing assertion anywhere in the suite may
// print.
impl fmt::Debug for Planned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Planned")
            .field("root", &self.root)
            .field("project", &self.project)
            .field("name", &self.name)
            .field("status", &self.status)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .field("assignee", &self.assignee)
            .field("slices", &self.slices)
            .field("already", &self.already)
            .field("settled", &self.settled)
            .field("became", &self.became)
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

// The board is resolved before the key is read, so a machine that cannot say
// which board it is standing at refuses without a request. A project with no
// slice left to cut is refused too, because a run with nothing to draft is not a
// run that succeeded quietly.
pub(crate) fn prepare<O: Opens>(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    scope: &str,
    slug: &str,
    open: &O,
) -> Result<Planned, Error> {
    let target = resolve_filing(manifest, root, home, Some(scope))
        .map_err(|source| Error::Filing { source })?;

    let board = open.open(target.value());
    // Once for the run, here rather than in `cut::cut`, which is called per
    // slice: the id is the same for every issue the run files, and asking per
    // slice — or per draft — would be a request bought again for an answer that
    // cannot have changed. Before the project read for the reason every other
    // refusal is asked here: a board that will not say who the key belongs to
    // has nothing to file for, and that is not a thing to find out once issues
    // exist.
    let assignee = board.viewer()?;

    let project = board
        .fetch_project(slug)?
        .ok_or_else(|| Error::UnknownProject {
            slug: slug.to_owned(),
            scope: scope.to_owned(),
        })?;

    let Some(status) = project.status().filter(|status| is_planned(status)) else {
        return Err(Error::NotPlanned {
            name: project.name().to_owned(),
            status: project.status().map(ToOwned::to_owned),
        });
    };

    let block = scope_block_in(project.content()).map_err(|source| Error::ScopeBlock { source })?;
    let ordered = block.ordered();
    // A cut note beats a skip note for the same title: the issues exist whatever
    // somebody said about the slice afterwards.
    let notes = cut::noted(project.notes());
    let already: Vec<Option<Vec<String>>> = ordered
        .iter()
        .map(|slice| {
            let key = fold_title(slice.heading());
            notes
                .iter()
                .filter(|(noted, _)| *noted == key)
                .max_by_key(|(_, issues)| !issues.is_empty())
                .map(|(_, issues)| issues.clone())
        })
        .collect();
    if already.iter().all(Option::is_some) {
        return Err(Error::AllCut {
            name: project.name().to_owned(),
        });
    }

    // Forman's backlog digest: the open tickets this scope's queue holds for
    // the assignee, so a draft can wait on one by identifier. A queue that
    // cannot be read is not worth failing a draft over.
    let open = board
        .scope_queue(
            target.destination().team_key(),
            target.destination().label(),
            &assignee,
        )
        .map(|queue| queue.issues().to_vec())
        .unwrap_or_default();

    let mut became = vec![Vec::new(); ordered.len()];
    for (slice, issues) in ordered.iter().zip(&already) {
        if let (Some(issues), Some(entry)) = (issues, became.get_mut(at(slice))) {
            *entry = recorded(issues);
        }
    }

    Ok(Planned {
        root: root.to_path_buf(),
        project: project.id().to_owned(),
        name: project.name().to_owned(),
        status: status.to_owned(),
        brief: block.brief().to_owned(),
        destination: target.destination(),
        value: target.value().to_owned(),
        assignee,
        slices: ordered.into_iter().cloned().collect(),
        settled: already
            .iter()
            .map(|issues| issues.as_ref().map(|issues| !issues.is_empty()))
            .collect(),
        already,
        became,
        at: 0,
        open,
    })
}

impl Planned {
    /// The brief a slice's session and a proposal are given: the brief, then
    /// Forman's digest of the open tickets, in Forman's words, which is what a
    /// draft's `blocked_by` can name by identifier.
    pub(crate) fn drafting_brief(&self) -> String {
        let mut open: Vec<&QueuedIssue> = self.open.iter().collect();
        if open.is_empty() {
            return self.brief.clone();
        }
        open.sort_by_key(|issue| natural(issue.identifier()));
        let rows: Vec<String> = open
            .iter()
            .take(BACKLOG_LIMIT)
            .map(|issue| {
                format!(
                    "{}  [{}]  {}",
                    issue.identifier(),
                    issue.state(),
                    issue.title()
                )
            })
            .collect();
        format!(
            "{}\n\nThese tickets already exist and are still open. If what you are drafting \
             cannot start until one of them has landed, put that identifier in blocked_by:\n\n{}",
            self.brief.trim_end(),
            rows.join("\n")
        )
    }

    /// The note that records `next` as skipped, so no later run offers it,
    /// ready to be said on whichever thread the door says it on. See
    /// [`cut::skip`].
    ///
    /// The slice counts as settled from here, before the note lands: a note
    /// Linear turns down is said on its own line, and a later slice that then
    /// finishes the project hides this one. That takes two failures in a row,
    /// and holding the walk on a worker's answer was not worth it.
    pub(crate) fn skipping(&mut self, next: &Next) -> Skipping {
        let finishes = self.finishes(next) && self.settled.contains(&Some(true));
        if let Some(entry) = self.settled.get_mut(next.index) {
            *entry = Some(false);
        }
        Skipping {
            project: self.project.clone(),
            title: next.slice.heading().to_owned(),
            value: self.value.clone(),
            finishes,
        }
    }

    // Every slice but this one settled, so settling this one leaves nothing to
    // draft.
    fn finishes(&self, next: &Next) -> bool {
        self.settled
            .iter()
            .enumerate()
            .all(|(index, settled)| index == next.index || settled.is_some())
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    pub(crate) const fn destination(&self) -> &Destination {
        &self.destination
    }

    /// How many slices the project has.
    pub(crate) const fn total(&self) -> usize {
        self.slices.len()
    }

    /// How many of them no note names.
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
            index: self.at,
            place: self.at,
            total: self.slices.len(),
            already: self.already[self.at].clone(),
        };
        self.at += 1;
        Some(next)
    }

    /// The next slice in the cut order that no note names, numbered among
    /// those alone: a walk that only drafts counts what it is drafting.
    pub(crate) fn next_uncut(&mut self) -> Option<Next> {
        while self.already.get(self.at)?.is_some() {
            self.at += 1;
        }
        let slice = self.slices[self.at].clone();
        let index = self.at;
        let place = self.already[..self.at]
            .iter()
            .filter(|issues| issues.is_none())
            .count();
        self.at += 1;
        Some(Next {
            slice,
            index,
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
            project: self.project.clone(),
            destination: self.destination.clone(),
            value: self.value.clone(),
            assignee: self.assignee.clone(),
            title: next.slice.heading().to_owned(),
            drafts,
            needs,
            open: self.open.iter().map(LinearIssue::listed).collect(),
            finishes: self.finishes(next),
        }
    }

    /// What a [`Filing`] came to, written down where a later slice's `needs`
    /// will look.
    pub(crate) fn settle(&mut self, next: &Next, cut: Cut) -> Settled {
        let issues = cut
            .issues
            .iter()
            .map(|issue| issue.identifier().to_owned())
            .collect();
        if let Some(entry) = self.became.get_mut(at(&next.slice)) {
            *entry = cut.issues;
        }
        if let Some(entry) = self.settled.get_mut(next.index) {
            *entry = Some(true);
        }
        Settled {
            issues,
            reported: cut.reported,
        }
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
    // Where the slice sits in the walk, which `place` is not once the walk
    // passes over what is already cut.
    index: usize,
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
    project: String,
    destination: Destination,
    value: String,
    assignee: String,
    title: String,
    drafts: Vec<Draft>,
    needs: Vec<Vec<LinearIssue>>,
    open: Vec<LinearIssue>,
    finishes: bool,
}

impl fmt::Debug for Filing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Filing")
            .field("root", &self.root)
            .field("project", &self.project)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .field("assignee", &self.assignee)
            .field("title", &self.title)
            .field("drafts", &self.drafts)
            .field("needs", &self.needs)
            .field("open", &self.open)
            .field("finishes", &self.finishes)
            .finish()
    }
}

impl Filing {
    // `out` is where [`cut::cut`]'s own lines go: stdout for the subcommand, and
    // `io::sink` for the panel, which words its own beside the slice they are
    // about.
    pub(crate) fn file<O: Opens, W: Write>(&self, open: &O, out: &mut W) -> Result<Cut, Error> {
        let needs: Vec<&[LinearIssue]> = self.needs.iter().map(Vec::as_slice).collect();
        let board = open.open(&self.value);

        let mut cut = cut::cut(
            &board,
            &self.root,
            cut::Filing {
                project: &self.project,
                destination: &self.destination,
                assignee: &self.assignee,
            },
            cut::Slice {
                title: &self.title,
                drafts: &self.drafts,
                needs: &needs,
                open: &self.open,
            },
            out,
        )?;
        if self.finishes {
            cut.reported.push(cut::finish(&board, &self.project));
        }
        Ok(cut)
    }
}

/// What a filed slice came to, by identifier, with one line per edge Linear
/// turned down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settled {
    pub(crate) issues: Vec<String>,
    pub(crate) reported: Vec<String>,
}

/// A slice's skip note on its way to the project.
pub(crate) struct Skipping {
    project: String,
    title: String,
    value: String,
    finishes: bool,
}

impl fmt::Debug for Skipping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Skipping")
            .field("project", &self.project)
            .field("title", &self.title)
            .field("value", &"<redacted>")
            .field("finishes", &self.finishes)
            .finish()
    }
}

impl Skipping {
    /// The note said, and the line about the project's move when this skip is
    /// the one that finishes it.
    pub(crate) fn post<O: Opens>(&self, open: &O) -> Result<Option<String>, Error> {
        let board = open.open(&self.value);
        cut::skip(&board, &self.project, &self.title)?;
        Ok(self.finishes.then(|| cut::finish(&board, &self.project)))
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

/// Drafts read back out of [`drafts_document`]'s layout: a `## N. title` line
/// opens each, everything under it is its body, and trailing `Blocked by #N` and
/// `Blocks #N` lines are its edges. Anything above the first draft — the slice's
/// heading — is not part of any draft. The body's own `## Problem` sections do
/// not open a draft, because a draft's heading starts with its number.
pub(crate) fn drafts_from_document(text: &str) -> Result<Vec<Draft>, String> {
    let mut opened: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(title) = draft_heading(line) {
            opened.push((title.to_owned(), Vec::new()));
        } else if let Some((_, body)) = opened.last_mut() {
            body.push(line);
        }
    }
    if opened.is_empty() {
        return Err("no draft is left in it: each starts with a `## 1. title` line".to_owned());
    }
    let count = opened.len();
    let mut drafts = Vec::with_capacity(count);
    for (title, mut body) in opened {
        let mut draft = Draft {
            title,
            ..Draft::default()
        };
        loop {
            while body.last().is_some_and(|line| line.trim().is_empty()) {
                body.pop();
            }
            let Some(last) = body.last() else { break };
            if let Some(edges) = last.trim().strip_prefix("Blocked by ") {
                (draft.blocked_by, draft.waits_on) = edge_numbers(edges, count)?;
            } else if let Some(edges) = last.trim().strip_prefix("Blocks ") {
                (draft.blocks, _) = edge_numbers(edges, count)?;
            } else {
                break;
            }
            body.pop();
        }
        body.join("\n").trim().clone_into(&mut draft.body);
        drafts.push(draft);
    }
    Ok(drafts)
}

fn draft_heading(line: &str) -> Option<&str> {
    let (number, title) = line.strip_prefix("## ")?.split_once(". ")?;
    (!number.is_empty() && number.chars().all(|c| c.is_ascii_digit())).then(|| title.trim())
}

// `#2, WAR-142` back to the positions they name, counting from 0 as the drafts
// do, and the existing tickets by identifier.
fn edge_numbers(edges: &str, count: usize) -> Result<(Vec<usize>, Vec<String>), String> {
    let mut positions = Vec::new();
    let mut named = Vec::new();
    for edge in edges.split(',').map(str::trim) {
        if drafting::is_identifier(edge) {
            named.push(edge.to_owned());
            continue;
        }
        match edge.trim_start_matches('#').parse::<usize>() {
            Ok(at) if (1..=count).contains(&at) => positions.push(at - 1),
            _ => {
                return Err(format!(
                    "`{edge}` is not one of the drafts, which are #1 to #{count}, or a ticket \
                     like `WAR-42`"
                ));
            }
        }
    }
    Ok((positions, named))
}

/// Forman's `[e]dit`: the drafts written out, `run` handed the file (it runs
/// `$EDITOR` and says what went wrong, if anything did), and what was saved
/// read back and put through the same repairs a model's answer gets, so the
/// caps and the edges hold whatever was typed. `Err` is one line saying why
/// nothing changed.
pub(crate) fn edited_drafts(
    slice: &Slice,
    drafts: &[Draft],
    run: impl FnOnce(&Path) -> Option<String>,
) -> Result<Vec<Draft>, String> {
    // The counter is what makes the name unique within a process. The clock
    // alone is not: macOS reports it in microseconds, and two calls in the same
    // microsecond — parallel tests did it — shared a file and read each other's
    // edits.
    static EDITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "warlock-drafts-{}-{}-{}.md",
        std::process::id(),
        EDITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    ));
    let mut text = drafts_document(slice, drafts).join("\n");
    text.push('\n');
    std::fs::write(&path, text).map_err(|error| one_line(&error.to_string()))?;
    let said = run(&path);
    let read = std::fs::read_to_string(&path);
    drop(std::fs::remove_file(&path));
    if let Some(said) = said {
        return Err(said);
    }
    let read = read.map_err(|error| one_line(&error.to_string()))?;
    let drafts = drafts_from_document(&read)?;
    let (mended, _) = drafting::mend(&drafting::Fill { drafts }, slice.heading(), slice.prose());
    Ok(mended.drafts)
}

// What a failed edit says: the reason, and that the drafts are as they were.
pub(crate) fn unedited_line(slice: &Slice, why: &str) -> String {
    format!("{} was not edited: {why}, so nothing changed", named(slice))
}

// The question in the words it was asked, a printed line per line of it: the
// contract asks for lettered options on lines of their own, and flattening them
// into one would bury the choice being offered. Whoever is at the door answers
// it, so these lines and [`answer_line`] are the pair a conversation is read
// back by: `asked` is the slice talking and `answered` is the person, and the
// two verbs are the whole of how a reader tomorrow tells one from the other.
pub(crate) fn question_lines(slice: &Slice, question: &str) -> Vec<String> {
    let mut lines = question
        .lines()
        .map(one_line)
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or_default();
    let mut said = vec![format!("{} asked: {first}", named(slice))];
    said.extend(lines);
    said
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
// the shell says it over the prompt.
pub(crate) fn drafted_line(slice: &Slice, titles: &[String]) -> String {
    format!("{} — drafted {}", named(slice), listed(titles))
}

/// Every draft whole — title, body and the edges between them — as the issues
/// `accept` would file. The panel puts it on the document card and the shell
/// prints it, both before the review: an answer about drafts nobody has read is
/// an answer about their titles, and feedback on a title is feedback on a guess.
///
/// Numbered from one, and the edges by those numbers: a draft's `blocked_by`
/// is an index into this slice's own drafts, which is the order printed here.
///
/// [`drafts_from_document`] reads this layout back, which is what Forman's
/// `[e]dit` is here: the drafts in `$EDITOR`, and whatever is saved is what the
/// review asks about next.
pub(crate) fn drafts_document(slice: &Slice, drafts: &[Draft]) -> Vec<String> {
    let mut lines = vec![format!("# {}", named(slice)), String::new()];
    for (index, draft) in drafts.iter().enumerate() {
        lines.push(format!("## {}. {}", index + 1, draft.title.trim()));
        lines.push(String::new());
        lines.extend(draft.body.trim().lines().map(str::to_owned));
        for (word, edges, named) in [
            ("Blocked by", &draft.blocked_by, draft.waits_on.as_slice()),
            ("Blocks", &draft.blocks, &[][..]),
        ] {
            let mut names: Vec<String> = edges.iter().map(|at| format!("#{}", at + 1)).collect();
            names.extend(named.iter().cloned());
            if !names.is_empty() {
                lines.push(String::new());
                lines.push(format!("{word} {}", names.join(", ")));
            }
        }
        lines.push(String::new());
    }
    lines.pop_if(|line| line.is_empty());
    lines
}

// A slice somebody said no to, in Red's words, and what that means for the
// next run: it is noted, so it is not offered again.
pub(crate) fn skipped_line(slice: &Slice) -> String {
    format!(
        "{} was skipped: nothing was created for it, and the next draft passes it over",
        named(slice)
    )
}

// The run ended by a No to the carry-on question, counting what was never
// offered: those slices are untouched rather than refused, so the next draft
// finds them exactly as this one did.
pub(crate) fn stopped_line(left: usize) -> String {
    format!("the run stopped; {} left for another draft", counted(left))
}

// The same slice left alone by a pipe that ended rather than by somebody saying
// so. Nothing was filed and nothing was noted, so unlike a skip the next run
// offers it again, and which of the two it was is worth a reader's while
// tomorrow.
fn unreviewed_line(slice: &Slice) -> String {
    format!(
        "{} was left: nobody said what to do with its drafts, so nothing was recorded and the \
         next draft offers it again",
        named(slice)
    )
}

// What the reader told the slice about its drafts, in their own words and the
// other half of the pair [`question_lines`] and [`answer_line`] make: the verb
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
