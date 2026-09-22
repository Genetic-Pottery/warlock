//! The two long keystrokes, `p` and `r`, as a worker thread, a channel of
//! [`PactEvent`], and one reload at the end.
//!
//! A pact and a refresh are one machine, not two: [`Work`] is the value that
//! says which a run is, and the difference comes down to one call in
//! [`apply_toggle`] and one verb on the footer. One run at a time falls out of
//! [`Pact::press`] reading the `run` field before it decides anything, so
//! neither key has to be told about the other. Esc and quitting are kept apart
//! on purpose — [`Pact::stop`] leaves the worker still saving what it finished,
//! while dropping the [`Pact`] drops the [`CancelGuard`] and leaves the
//! manifest as it was.
//!
//! [`descend`] saves the manifest once, after the whole descent, so until that
//! save lands disk still holds the pre-run record. Re-reading the tree mid-run
//! would re-derive every row from it and wipe the green the run has been
//! painting; the single `reload` at the foot of [`drain`] is not an
//! optimisation but the only point at which disk is the honest account.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;
use std::{fs, io, thread};

use warlock_engine::{
    Agent, DOCUMENT_FILE, Manifest, NodeState, PactedSubtree, Tree, fitting, pact, to_manifest_path,
};
use warlock_tui::{
    Activities, Activity, App, Cancel, ClaudeAgent, Outcome, PactIntent, PactToggle, Run, Section,
    Sigils, Wired,
};

use crate::boundary::{Reach, Verdict, verdict};
use crate::descent::{Descent, RunEvent, descend};
use crate::error::one_line;
use crate::session::{Scope, closed_scope, reload};

// The worker reports on every path it takes itself, so the only way the channel
// closes without a `Finished` is a panic. The hook has already printed it; what
// is left to say on the footer is that the record did not move.
const PACT_LOST: &str = "the pact stopped without saying how it went; nothing new was recorded";

const PACT_CANCELLED: &str = "the pact was cancelled; what it finished first is recorded";

pub(crate) struct Pact<P> {
    agent: P,
    run: Option<Running>,
}

// Returned only on the round a run ends, because the watcher must be told once
// that the walk it was following has been replaced — see
// `Watched::caught_up`. The caller working the same fact out by comparing
// `running()` either side of the drain is an edge detector kept by hand over
// something this module already knows. `None` inside is a reload that failed,
// which leaves the tree already drawn and is not an error.
pub(crate) struct Reloaded(pub(crate) Option<Tree>);

impl Pact<ClaudeAgent> {
    // Building the agent costs nothing — it is a command line and a timeout,
    // not a connection — so no `claude` exists until a key asks for a pass.
    pub(crate) fn new() -> Self {
        Self::with_agent(ClaudeAgent::new())
    }

    #[cfg(test)]
    pub(crate) fn with_run(run: Running) -> Self {
        Self {
            agent: ClaudeAgent::new(),
            run: Some(run),
        }
    }
}

impl<P: Wired + Agent> Pact<P> {
    // The seam a test drives the real value over a stand-in agent through,
    // rather than assembling the pieces underneath and proving something about
    // an arrangement the event loop never has.
    pub(crate) fn with_agent(agent: P) -> Self {
        Self { agent, run: None }
    }

    pub(crate) const fn running(&self) -> bool {
        self.run.is_some()
    }

    pub(crate) fn stop(&self) {
        if let Some(run) = self.run.as_ref() {
            run.cancel.cancel();
        }
    }

    pub(crate) fn press(
        &mut self,
        kind: Run,
        app: &mut App,
        manifest: &Manifest,
        scope: &Scope,
        now: Instant,
    ) {
        let before = app.clone();
        // Read here, off the one run this type keeps, rather than handed in by a
        // caller who would have had to look at the same field to know it.
        let running = self.running();
        let (repo_root, sigils) = (&scope.repo_root, scope.chrome.sigils());
        let work = match kind {
            Run::Refresh => {
                refresh_press(app, manifest, repo_root, sigils, running, now).map(Work::Refresh)
            }
            Run::Pact => pact_press(app, manifest, repo_root, sigils, running, now).map(Work::Pact),
        };
        if let Some(work) = work {
            // The worker, the channel and the say-when, in the one value this
            // type keeps about a run it is not doing — see [`start_run`], which
            // is where both keys start theirs.
            self.run = Some(start_run(
                work,
                before,
                manifest,
                &scope.repo_root,
                &self.agent,
            ));
        }
    }

    pub(crate) fn keep_up(
        &mut self,
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        now: Instant,
    ) -> Option<Reloaded> {
        drain(&mut self.run, app, manifest, scope, now)
    }
}

pub(crate) struct Running {
    pub(crate) events: Receiver<PactEvent>,
    pub(crate) cancel: CancelGuard,
    pub(crate) work: Work,
    pub(crate) before: App,
    pub(crate) unchanged: Vec<PathBuf>,
    // Directory and the failure below it that cost it its pass. Remembered for
    // the same reason `unchanged` is: the filesystem cannot tell a document
    // this run declined to rewrite from one it wrote, and the section's closing
    // line turns on exactly that.
    pub(crate) skipped: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug, Clone)]
pub(crate) enum Work {
    Pact(PactToggle),
    Refresh(PathBuf),
}

impl Work {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Pact(toggle) => &toggle.path,
            Self::Refresh(directory) => directory,
        }
    }

    pub(crate) const fn descent(&self) -> Descent {
        match self {
            Self::Pact(toggle) if toggle.pacted => Descent::Pact,
            Self::Pact(_) => Descent::Unpact,
            Self::Refresh(_) => Descent::Refresh,
        }
    }

    pub(crate) const fn kind(&self) -> Run {
        match self {
            Self::Pact(_) => Run::Pact,
            Self::Refresh(_) => Run::Refresh,
        }
    }

    // Whether a cancel is worth rewording the outcome for. An un-pact spends no
    // model time — it rewrites the manifest and returns — so a cancel that
    // arrives during one is a cancel of something already over, and saying so
    // would report a run that did everything asked of it as stopped.
    const fn is_cancellable(&self) -> bool {
        match self {
            Self::Pact(toggle) => toggle.pacted,
            Self::Refresh(_) => true,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CancelGuard {
    cancel: Cancel,
}

impl CancelGuard {
    pub(crate) fn new() -> Self {
        Self {
            cancel: Cancel::new(),
        }
    }

    pub(crate) fn handle(&self) -> Cancel {
        self.cancel.clone()
    }

    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

// Dropping the guard is what makes quitting kill the `claude` in flight: no
// exit path has to remember to stop the run, because losing the `Pact` loses
// the `Running` and losing the `Running` cancels. Esc is the other spelling and
// goes through `cancel()` directly, leaving the run alive to save what it has.
impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug)]
pub(crate) enum PactEvent {
    Run(RunEvent),
    Doing(Activity),
    Finished(Result<Toggled, String>),
}

fn activity_port(events: &Sender<PactEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(PactEvent::Doing(activity));
    })
}

pub(crate) fn spawn_pact<P: Wired + Agent>(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &P,
    cancel: Cancel,
) -> Receiver<PactEvent> {
    let (events, received) = mpsc::channel();
    let (manifest, repo_root, work) = (manifest.clone(), repo_root.to_path_buf(), work.clone());
    // This run's copy of the agent, and the only one that answers to this run's
    // handle: the agent the event loop keeps has a handle of its own that nobody
    // else holds, so cancelling one run can never reach into the next.
    //
    // The activity port is attached the same way and for the same reason, in the
    // same breath — see `activity_port`. Both are one-per-run, and both die with
    // the copy of the agent this thread owns.
    let agent = agent.wired(cancel.clone(), activity_port(&events));
    thread::spawn(move || run_pact(&manifest, &repo_root, &work, &agent, &cancel, &events));
    received
}

fn start_run<P: Wired + Agent>(
    work: Work,
    before: App,
    manifest: &Manifest,
    repo_root: &Path,
    agent: &P,
) -> Running {
    let cancel = CancelGuard::new();
    Running {
        events: spawn_pact(manifest, repo_root, &work, agent, cancel.handle()),
        cancel,
        work,
        before,
        unchanged: Vec::new(),
        skipped: Vec::new(),
    }
}

fn run_pact(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &dyn Agent,
    cancel: &Cancel,
    events: &Sender<PactEvent>,
) {
    let outcome = apply_toggle(manifest, repo_root, work, agent, cancel, &mut |event| {
        // Ignored: a receiver that has gone away is an application that is
        // quitting.
        let _ = events.send(PactEvent::Run(event));
    });
    // Every run that spends minutes on model passes is stoppable, and both of
    // them are: a refresh is reworded here exactly as a pact is. The one run
    // that is not is an un-pact — see [`Work::is_cancellable`].
    let outcome = match outcome {
        Ok(toggled) if work.is_cancellable() && cancel.is_cancelled() => Ok(cancelled(toggled)),
        outcome => outcome,
    };
    let _ = events.send(PactEvent::Finished(outcome));
}

fn cancelled(toggled: Toggled) -> Toggled {
    Toggled {
        granted: false,
        message: Some(PACT_CANCELLED.to_owned()),
        ..toggled
    }
}

// The two refusals both keys share, in the order they have to be asked in. A
// run already in flight is answered by rewording a line that is already on
// screen, which is the whole of the refusal and says the same thing however
// often it is pressed; the boundary is asked second, and both are past before
// anything that paints rather than asks.
fn turned_down(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    in_flight: bool,
) -> bool {
    if in_flight {
        app.set_pact_refused();
        return true;
    }
    closed_scope(app, manifest, repo_root, sigils).is_some()
}

fn pact_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    in_flight: bool,
    at: Instant,
) -> Option<PactToggle> {
    if turned_down(app, manifest, repo_root, sigils, in_flight) {
        return None;
    }
    // The downward question, asked before the toggle for the reason the two
    // above it are. Last of the three, because "may this operator act here at
    // all" is settled before "what would this press reach".
    if blocked_unpact(app, manifest, repo_root, sigils) {
        return None;
    }
    // Both boundary questions are past, so what is left is the app's own
    // rules about the row. Asked and then carried out, rather than in one
    // painting call, so the press acted on is provably the press the two
    // questions above were asked about.
    let toggle = match app.pact_intent() {
        PactIntent::Toggles(toggle) => toggle,
        PactIntent::Refused(message) => {
            app.set_message(message);
            return None;
        }
        PactIntent::NoRow => return None,
    };
    app.apply_toggle(&toggle);
    if toggle.pacted {
        app.start_account(at);
    }
    Some(toggle)
}

fn blocked_unpact(app: &mut App, manifest: &Manifest, repo_root: &Path, sigils: &Sigils) -> bool {
    // Which directory the press reaches and which way it goes, both off one
    // answer: a press that would pact rather than un-pact loses no boundary.
    let Some(reach) = app.pact_reach() else {
        return false;
    };
    if reach.pacted {
        return false;
    }

    // The decision is [`verdict`]'s, asked at the reach an un-pact needs; this
    // is the panel's half of what to do about it. `closed_scope` has already
    // asked the narrower question by the time this runs — see [`pact_press`] —
    // so what is left to say here is only ever about what is underneath.
    let path = reach.path;
    let answer = verdict(
        &path,
        repo_root,
        manifest,
        sigils.as_slice(),
        Reach::HereAndBelow,
    );
    if !matches!(answer, Verdict::ClosedBelow { .. }) {
        return false;
    }

    let label = app.label_for(&path);
    if let Some(line) = answer.message(&label) {
        app.set_message(line);
    }
    true
}

// `edits.rs` drives the real `p` rather than a copy of its rules; the two
// booleans it does not care about are pinned here so a test cannot pin them
// differently.
#[cfg(test)]
pub(crate) fn pressed_p(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
) -> Option<PactToggle> {
    pact_press(app, manifest, repo_root, sigils, false, Instant::now())
}

fn refresh_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    in_flight: bool,
    at: Instant,
) -> Option<PathBuf> {
    if turned_down(app, manifest, repo_root, sigils, in_flight) {
        return None;
    }
    // No downward question here, unlike `pact_press`: a refresh never drops a
    // pact, so there is no boundary below the row for it to lose.
    let directory = app.refresh()?;
    app.start_account(at);
    Some(directory)
}

fn drain(
    run: &mut Option<Running>,
    app: &mut App,
    manifest: &mut Manifest,
    scope: &Scope,
    now: Instant,
) -> Option<Reloaded> {
    // No run, nothing drained, nothing reloaded — which is what almost every
    // frame of warlock's life does here. Taken mutably because a carried
    // directory is recorded on the run as its event arrives; the borrow is over
    // before the `take` below.
    let running = run.as_mut()?;

    let outcome = loop {
        match running.events.try_recv() {
            Ok(PactEvent::Run(RunEvent::Starting {
                directory,
                position,
                total,
            })) => {
                // Two places, one fact, and they are two because they are read
                // at two different speeds: the footer says which directory of
                // how many is being worked *now* and replaces itself every time,
                // while the panel keeps a section per directory for as long as
                // the run lasts. The section is opened first so that the
                // activities drained after it — which may be in this same batch
                // — land under the directory they belong to.
                //
                // The panel goes through `App::write_run`, here and for every
                // other event below: one way in, so a line of a run cannot be
                // put on the panel by two different routes. The label is
                // computed once, outside the closure — one spelling of a
                // directory, handed to the account holding this run.
                let heading = section_label(&scope.root, &directory);
                app.panel_mut()
                    .write_run(|account| account.open_section(&heading, now));
                // The fraction is the observer's own, whichever run is
                // reporting: a refresh of a subtree of forty directories with
                // seven stale ones counts to seven, because seven is what the
                // engine planned to visit and said so. Nothing here counts
                // anything. The kind rides along so the line reads as
                // refreshing rather than pacting — see `Work::kind`.
                app.set_run_in_flight(running.work.kind(), directory, position, total);
            }
            // Filed under whichever directory is open, which is the one the
            // `Starting` before it named: an activity carries no directory
            // because it needs none, and the account's live section is the
            // answer. An account is
            // always there during a run — the press that started it made one —
            // so a run with neither is one nobody started this way, which is a
            // test driving the events directly; dropping the line is the honest
            // thing to do with it either way. See `App::write_run`.
            //
            // What each activity comes to is the account's business and not this
            // file's: a tool is its name and its one detail, thinking is the
            // word `thinking`, and a cost is added to the section's spend rather
            // than drawn as a line of its own. See `Account::record`.
            Ok(PactEvent::Doing(activity)) => {
                app.panel_mut()
                    .write_run(|account| account.record(&activity, now));
            }
            // The panel only, and one line: the request this directory's pass
            // was handed, filed under whichever section the `Starting` before
            // it opened, exactly as an activity is.
            // The footer is left alone — it is already saying which directory
            // of how many is being worked, which is the question it answers,
            // and a byte total is not that.
            //
            // Filed at the handover rather than drawn on the placeholder
            // above, because the clock is the point: this line counts the
            // silence of *this* directory's own pass, from the moment its
            // request went over, while the placeholder counts from the section
            // opening and would label a multi-pass directory's whole wait with
            // it. See `Account::record_waiting`.
            Ok(PactEvent::Run(RunEvent::Requesting { files, bytes })) => {
                app.panel_mut()
                    .write_run(|account| account.record_waiting(files, bytes, now));
            }
            // Both places again, and for the same reason `Starting` is both: the
            // panel keeps the stretch of file passes as one reworded line, and
            // the footer's bar fills by it. The bar is the only reason this
            // reaches the footer at all — the line there still names the
            // directory of how many, because that is the question it answers.
            Ok(PactEvent::Run(RunEvent::Describing {
                position,
                total,
                bytes,
            })) => {
                app.panel_mut()
                    .write_run(|account| account.record_describing(position, total, bytes, now));
                app.set_files_in_flight(position, total);
            }
            // The panel only, and one line, filed like the request line above
            // it: why this directory is about to cost a second pass, or why it
            // is about to fail, in the engine's own words.
            Ok(PactEvent::Run(RunEvent::Rejected {
                defects,
                attempt,
                attempts,
            })) => {
                app.panel_mut()
                    .write_run(|account| account.record_rejected(&defects, attempt, attempts, now));
            }
            // The panel only, and one line per mend, filed under the same
            // section as the rejections above it: the asking ran out and the
            // document was written anyway, so this is the one place a reader
            // learns that a slot of it is warlock's own words rather than the
            // model's.
            //
            // The engine names the directory and the event carries it, because
            // a repair is a fact about one directory and nothing reading these
            // events should have to infer which. Nothing is done with it here:
            // the line lands where every line of a pass lands, in the section
            // the `Starting` before it opened, which is that same directory's.
            Ok(PactEvent::Run(RunEvent::Repaired { mend, .. })) => {
                app.panel_mut()
                    .write_run(|account| account.record_repaired(&mend, now));
            }
            // The one recolouring a run does before it is over. The engine
            // only says this of a directory whose whole subtree delivered —
            // the very condition its grant is decided on — so painting the
            // subtree green here repeats the engine's judgement rather than
            // second-guessing it, and a directory that finished third of five
            // is green while the fourth is still being paid for. The reload at
            // the end repaints from the manifest either way, which is what
            // catches the one thing this preview cannot know: a hash that
            // fails in phase two.
            Ok(PactEvent::Run(RunEvent::Documented { directory })) => {
                app.set_subtree_state(&directory, NodeState::PactedFresh);
                // The document the pass just wrote, put on screen where it was
                // written: beside the directory, in the colour the paint above
                // has this moment given it. The paint comes first so the row is
                // born green rather than repainted into it, though the order is
                // not load-bearing — `set_subtree_state` paints a directory's
                // files along with the directory, so an insertion above it
                // would end up the same colour by the other road.
                //
                // And no reload, here or anywhere else mid-run. The manifest on
                // disk is still the pre-pact one until the single save at the
                // end of the run, so re-reading the tree now would re-derive
                // every row's state from that stale record and wipe the green
                // the run has been painting a directory at a time. That is why
                // the one reload stays at the bottom of this function, after the
                // outcome has landed and the manifest is written: by then disk
                // is the honest account, and this preview is the thing it
                // corrects rather than the thing it contradicts.
                app.insert_file_row(directory.join(DOCUMENT_FILE));
            }
            // The same two paints as `Documented` — the tree cannot tell the
            // two apart and should not try — and one thing besides: the
            // directory is remembered, because the filesystem cannot say
            // afterwards whether the document on disk was written by this run
            // or kept from the last, and the section's closing line turns on
            // exactly that.
            Ok(PactEvent::Run(RunEvent::Unchanged { directory })) => {
                app.set_subtree_state(&directory, NodeState::PactedFresh);
                app.insert_file_row(directory.join(DOCUMENT_FILE));
                running.unchanged.push(directory);
            }
            // No paint at all, and that is the point: a skipped directory is
            // one the refresh found stale and left stale, so the row is already
            // the colour it should be. Remembered so its section can close
            // saying what happened rather than reading the document on disk and
            // calling it a write.
            Ok(PactEvent::Run(RunEvent::Skipped { directory, below })) => {
                running.skipped.push((directory, below));
            }
            Ok(PactEvent::Finished(outcome)) => break Some(outcome),
            // Still running, and nothing new to say.
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    let running = run
        .take()
        .expect("the pact drained just above is still here");
    app.clear_pact_in_flight();
    // Read before the outcome is taken apart, because one arm below moves the
    // rest of the run out from under it: whether the reader stopped this run is
    // what decides how the panel's last section ends.
    let cancelled = running.cancel.is_cancelled();
    let refusals = match outcome {
        Some(Ok(Toggled {
            manifest: next,
            granted,
            message,
            refusals,
        })) => {
            *manifest = next;
            // The app painted the subtree stale when the key was pressed, and
            // the run's `Documented` announcements have been turning finished
            // branches green one at a time since. A pact that came back with
            // nothing wrong wrote, hashed and granted every directory in it,
            // so the one paint left is the whole subtree at once — the same
            // fact the announcements said piecemeal, said once at the end.
            //
            // A pact with a failure in it paints nothing here: the finished
            // branches are already green by the engine's own word, and the
            // failure's ancestors stay yellow — "pacted, not proven fresh" —
            // which is what the manifest is about to say of them. Colouring
            // anything else from this arm would be this file second-guessing
            // per node a manifest it did not compute.
            //
            // A refresh reads the same way, and it is the engine that makes
            // that true rather than anything here: a refresh keeps the grant of
            // every directory it skipped (WAR-39), so a run with no failure in
            // it leaves the whole subtree granted even though it described a
            // handful of directories. The paint is of the subtree the run
            // covered either way.
            if granted {
                app.set_subtree_state(running.work.path(), NodeState::PactedFresh);
            }
            if let Some(message) = message {
                app.set_message(message);
            }
            // Kept for the panel, which is the only place they can be said one
            // directory at a time.
            refusals
        }
        // Nothing was recorded: either the subtree could not be listed, or the
        // manifest would not save. Documents may well be on disk in the second
        // case, but the manifest is the record of what is pacted and it still
        // says what it said before, so the rows go back to matching it and the
        // reason goes on the app's line — the same one a refused toggle uses —
        // rather than out of the loop, which would take the screen with it.
        Some(Err(message)) => {
            restore(app, running.before, message);
            Vec::new()
        }
        // The worker died with the manifest in this thread's hand untouched, so
        // the rows go back to matching it exactly as they do for a run that
        // recorded nothing, and the footer says the run is over.
        None => {
            restore(app, running.before, PACT_LOST);
            Vec::new()
        }
    };

    // Every section gets its ending and the run gets its summary, whichever arm
    // above ran and whatever the run had to say for itself. After the arms
    // rather than inside them, because two of them put a different app on
    // screen and the account belongs to whichever one survives — and because a
    // run that ends without its clocks stopped is a finished run whose newest
    // line goes on counting up for as long as warlock is open.
    close_account(
        app,
        scope,
        &refusals,
        &running.unchanged,
        &running.skipped,
        cancelled,
        now,
    );

    // The run is over and everything it recorded is on disk, so the rows on
    // screen are one load out of date whichever arm above ran. Saying so is the
    // whole of what is left owed outside this module.
    Some(Reloaded(reload(app, scope, manifest)))
}

fn close_account(
    app: &mut App,
    scope: &Scope,
    refusals: &[Refusal],
    unchanged: &[PathBuf],
    skipped: &[(PathBuf, PathBuf)],
    cancelled: bool,
    now: Instant,
) {
    app.panel_mut().write_run(|account| {
        if cancelled {
            account.close_section(&Outcome::Cancelled, now);
        }
        account.close_open_sections(now, |section| {
            section_outcome(section, refusals, unchanged, skipped, &scope.root)
        });
        account.finish(now);
    });
}

fn section_outcome(
    section: &Section,
    refusals: &[Refusal],
    unchanged: &[PathBuf],
    skipped: &[(PathBuf, PathBuf)],
    root: &Path,
) -> Outcome {
    let is_this_section =
        |directory: &Path| Path::new(&section_label(root, directory)) == section.directory();

    if let Some(refusal) = refusals
        .iter()
        .find(|refusal| is_this_section(&refusal.directory))
    {
        return Outcome::Refused {
            reason: refusal.reason.clone(),
        };
    }

    // Asked before the filesystem for the reason the carry below is: this
    // directory has a `WARLOCK.md` and did not write it, so metadata would read
    // as a write that never happened.
    if let Some((_, below)) = skipped
        .iter()
        .find(|(directory, _)| is_this_section(directory))
    {
        return Outcome::Skipped {
            below: PathBuf::from(section_label(root, below)),
        };
    }

    // The heading's own spelling with the document's name under it, and the
    // absolute path to go and look at it: the label is relative to the root, so
    // joining it back on is what the run walked.
    let document = section.directory().join(DOCUMENT_FILE);

    // Asked before the filesystem is, because the filesystem cannot answer it:
    // a document this run carried forward and one it wrote a moment ago are
    // indistinguishable on disk, and calling the first one written would put a
    // write in the panel that never happened.
    if unchanged.iter().any(|carried| is_this_section(carried)) {
        return Outcome::Unchanged { document };
    }
    match fs::metadata(root.join(&document)) {
        Ok(written) => Outcome::Wrote {
            document,
            bytes: written.len(),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Outcome::Refused {
            reason: "no document was written".to_owned(),
        },
        Err(error) => Outcome::Refused {
            reason: format!("its document could not be read: {error}"),
        },
    }
}

fn restore(app: &mut App, before: App, message: impl Into<String>) {
    app.restore_from(before);
    app.set_message(message);
}

fn section_label(root: &Path, directory: &Path) -> String {
    match to_manifest_path(root, directory) {
        Ok(relative) if relative != "." => relative,
        _ => directory.display().to_string(),
    }
}

#[derive(Debug)]
pub(crate) struct Toggled {
    manifest: Manifest,
    granted: bool,
    message: Option<String>,
    refusals: Vec<Refusal>,
}

#[derive(Debug)]
struct Refusal {
    directory: PathBuf,
    reason: String,
}

impl From<&pact::Failure> for Refusal {
    fn from(failure: &pact::Failure) -> Self {
        Self {
            directory: failure.directory().to_path_buf(),
            // Flattened for the same reason every other line here is: a failure
            // can carry a manifest error, which carries the TOML parser's
            // several lines, and a panel row is one row.
            reason: one_line(&failure.to_string()),
        }
    }
}

fn apply_toggle(
    manifest: &Manifest,
    repo_root: &Path,
    work: &Work,
    agent: &dyn Agent,
    cancel: &Cancel,
    sink: &mut dyn FnMut(RunEvent),
) -> Result<Toggled, String> {
    // The descent and the one save are [`descend`]'s, shared with the shell's
    // `warlock pact` and `warlock refresh` — see [`mod@crate::descent`]. What is
    // left here is the panel's half: an un-pact grants nothing and has nothing
    // to report, and the other two are read for a footer line and a refusal per
    // directory.
    let subtree = descend(
        work.descent(),
        work.path(),
        repo_root,
        manifest,
        agent,
        cancel,
        sink,
    )
    .map_err(|error| one_line(&error.to_string()))?;

    Ok(match work {
        Work::Pact(toggle) if !toggle.pacted => Toggled {
            manifest: subtree.manifest,
            granted: false,
            message: None,
            refusals: Vec::new(),
        },
        Work::Pact(_) | Work::Refresh(_) => described(subtree),
    })
}

fn described(subtree: PactedSubtree) -> Toggled {
    let PactedSubtree {
        manifest,
        failures,
        problems,
        repairs,
    } = subtree;
    // Failures alone decide freshness, and neither the byte caps' problems nor
    // the repairs do: a request that left a lockfile out still produced a
    // document, a hash and a grant, and so did one warlock had to mend a slot
    // of. They are still worth a line, which is why the three travel separately
    // from here on.
    let granted = failures.is_empty();
    // And the same failures a second time, per directory: the footer takes
    // one of them and the panel takes all of them, because the panel has a
    // section for each and can say which is which.
    let refusals = failures.iter().map(Refusal::from).collect();
    Toggled {
        manifest,
        granted,
        message: pact_message(&failures, &problems, &repairs),
        refusals,
    }
}

// Three tiers, worst first, and only one of them is ever on the footer: a
// directory that was refused is the fact the operator needs, a request that
// went out short is the next one, and a slot warlock mended itself is worth
// saying only when nothing louder happened. Failures and problems share the
// first tier because a refusal and a short request are both things about the
// same attempt; repairs stand alone because they happened to a document that
// was written and granted.
fn pact_message(
    failures: &[pact::Failure],
    problems: &[fitting::Problem],
    repairs: &[pact::Repaired],
) -> Option<String> {
    let (first, rest) = if let Some((first, others)) = failures.split_first() {
        (first.to_string(), others.len() + problems.len())
    } else if let Some((first, others)) = problems.split_first() {
        (first.to_string(), others.len())
    } else if let Some((first, others)) = repairs.split_first() {
        // The repair's own sentence, not a parallel wording of it: `Repaired`
        // already says which directory and what was done to which slot.
        (first.to_string(), others.len())
    } else {
        return None;
    };

    let first = one_line(&first);
    Some(match rest {
        0 => first,
        rest => format!("{first} (and {rest} more)"),
    })
}

#[cfg(test)]
#[path = "tests/pacting.rs"]
mod tests;
