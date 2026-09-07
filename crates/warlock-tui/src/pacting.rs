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
//! painting; the single `reload_tree` at the foot of [`drain`] is not an
//! optimisation but the only point at which disk is the honest account.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;
use std::{fs, io, thread};

use warlock_engine::{
    Agent, Manifest, NodeState, PactedSubtree, Pacting, Tree, document::Defect, fitting, pact,
    to_manifest_path,
};
use warlock_tui::{
    Activities, Activity, App, Cancel, ClaudeAgent, Outcome, PactIntent, PactToggle, Run, Section,
    Sigils, Wired,
};

use crate::boundary::{Reach, Verdict, verdict};
use crate::descent::{Descent, carry_on, descend};
use crate::error::one_line;
use crate::session::{Scope, closed_scope, reload_tree};

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
        let work = match kind {
            Run::Refresh => refresh_press(
                app,
                manifest,
                &scope.repo_root,
                scope.chrome.sigils(),
                running,
                now,
            )
            .map(Work::Refresh),
            Run::Pact => pact_press(
                app,
                manifest,
                &scope.repo_root,
                scope.chrome.sigils(),
                running,
                now,
            )
            .map(Work::Pact),
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
    Starting {
        directory: PathBuf,
        position: usize,
        total: usize,
    },
    Doing(Activity),
    Requesting {
        files: usize,
        bytes: u64,
    },
    Rejected {
        defects: Vec<String>,
        attempt: usize,
        attempts: usize,
    },
    Documented {
        directory: PathBuf,
    },
    Unchanged {
        directory: PathBuf,
    },
    Finished(Result<Toggled, String>),
}

fn activity_port(events: &Sender<PactEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(PactEvent::Doing(activity));
    })
}

struct Reporting<'a> {
    events: &'a Sender<PactEvent>,
    cancel: &'a Cancel,
}

impl pact::Observer for Reporting<'_> {
    // The one place a run is asked to stop between passes, and the reason the
    // cancel is read here rather than in the loop below: the engine offers this
    // hook before each directory, and a pass that has already started is the
    // agent's own to give up on.
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        if carry_on(self.cancel) == Pacting::Stop {
            return Pacting::Stop;
        }
        let _ = self.events.send(PactEvent::Starting {
            directory: directory.to_path_buf(),
            position,
            total,
        });
        Pacting::Continue
    }

    fn requesting(&mut self, files: usize, bytes: u64) {
        let _ = self.events.send(PactEvent::Requesting { files, bytes });
    }

    fn rejected(&mut self, _directory: &Path, defects: &[Defect], attempt: usize, attempts: usize) {
        let _ = self.events.send(PactEvent::Rejected {
            defects: defects.iter().map(ToString::to_string).collect(),
            attempt,
            attempts,
        });
    }

    fn documented(&mut self, directory: &Path) {
        let _ = self.events.send(PactEvent::Documented {
            directory: directory.to_path_buf(),
        });
    }

    fn unchanged(&mut self, directory: &Path) {
        let _ = self.events.send(PactEvent::Unchanged {
            directory: directory.to_path_buf(),
        });
    }
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
    let outcome = apply_toggle(
        manifest,
        repo_root,
        work,
        agent,
        &mut Reporting { events, cancel },
    );
    // Every run that spends minutes on model passes is stoppable, and both of
    // them are: a refresh is reworded here exactly as a pact is. The one run
    // that is not is an un-pact — see [`Work::is_cancellable`].
    let outcome = match outcome {
        Ok(toggled) if work.is_cancellable() && cancel.is_cancelled() => Ok(cancelled(toggled)),
        outcome => outcome,
    };
    // Ignored for the same reason `Reporting`'s sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(PactEvent::Finished(outcome));
}

fn cancelled(toggled: Toggled) -> Toggled {
    Toggled {
        granted: false,
        message: Some(PACT_CANCELLED.to_owned()),
        ..toggled
    }
}

fn pact_press(
    app: &mut App,
    manifest: &Manifest,
    repo_root: &Path,
    sigils: &Sigils,
    in_flight: bool,
    at: Instant,
) -> Option<PactToggle> {
    if in_flight {
        // The whole of the refusal: a bit of wording on a line that is already
        // on screen. Setting it again says the same thing, so a reader leaning
        // on the key changes nothing after the first press.
        app.set_pact_refused();
        return None;
    }
    // Before the toggle, which paints rather than asks. See above.
    if closed_scope(app, manifest, repo_root, sigils).is_some() {
        return None;
    }
    // And the downward question, asked in the same place and for the same
    // reason. Second, because "may this operator act here at all" is settled
    // before "what would this press reach".
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
    if in_flight {
        // The whole of the refusal, and the very one a second pact press gets:
        // a bit of wording on a line that is already on screen.
        app.set_pact_refused();
        return None;
    }
    if closed_scope(app, manifest, repo_root, sigils).is_some() {
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
            Ok(PactEvent::Starting {
                directory,
                position,
                total,
            }) => {
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
            Ok(PactEvent::Requesting { files, bytes }) => {
                app.panel_mut()
                    .write_run(|account| account.record_waiting(files, bytes, now));
            }
            // The panel only, and one line, filed like the request line above
            // it: why this directory is about to cost a second pass, or why it
            // is about to fail, in the engine's own words.
            Ok(PactEvent::Rejected {
                defects,
                attempt,
                attempts,
            }) => {
                app.panel_mut()
                    .write_run(|account| account.record_rejected(&defects, attempt, attempts, now));
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
            Ok(PactEvent::Documented { directory }) => {
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
            Ok(PactEvent::Unchanged { directory }) => {
                app.set_subtree_state(&directory, NodeState::PactedFresh);
                app.insert_file_row(directory.join(DOCUMENT_FILE));
                running.unchanged.push(directory);
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
    close_account(app, scope, &refusals, &running.unchanged, cancelled, now);

    // The run is over and everything it recorded is on disk, so the rows on
    // screen are one load out of date whichever arm above ran. Saying so is the
    // whole of what is left owed outside this module.
    Some(Reloaded(reload_tree(app, scope)))
}

const DOCUMENT_FILE: &str = "WARLOCK.md";

fn close_account(
    app: &mut App,
    scope: &Scope,
    refusals: &[Refusal],
    unchanged: &[PathBuf],
    cancelled: bool,
    now: Instant,
) {
    app.panel_mut().write_run(|account| {
        if cancelled {
            account.close_section(&Outcome::Cancelled, now);
        }
        account.close_open_sections(now, |section| {
            section_outcome(section, refusals, unchanged, &scope.root)
        });
        account.finish(now);
    });
}

fn section_outcome(
    section: &Section,
    refusals: &[Refusal],
    unchanged: &[PathBuf],
    root: &Path,
) -> Outcome {
    let refused = refusals
        .iter()
        .find(|refusal| Path::new(&section_label(root, &refusal.directory)) == section.directory());
    if let Some(refusal) = refused {
        return Outcome::Refused {
            reason: refusal.reason.clone(),
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
    if unchanged
        .iter()
        .any(|carried| Path::new(&section_label(root, carried)) == section.directory())
    {
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
    observer: &mut dyn pact::Observer,
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
        observer,
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
    } = subtree;
    // Failures alone decide freshness, and the byte caps' problems do not:
    // a request that left a lockfile out still produced a document, a hash
    // and a grant. They are still worth a line, which is why the two travel
    // separately from here on.
    let granted = failures.is_empty();
    // And the same failures a second time, per directory: the footer takes
    // one of them and the panel takes all of them, because the panel has a
    // section for each and can say which is which.
    let refusals = failures.iter().map(Refusal::from).collect();
    Toggled {
        manifest,
        granted,
        message: pact_message(&failures, &problems),
        refusals,
    }
}

fn pact_message(failures: &[pact::Failure], problems: &[fitting::Problem]) -> Option<String> {
    let (first, rest) = match (failures.split_first(), problems.split_first()) {
        (Some((first, others)), _) => (first.to_string(), others.len() + problems.len()),
        (None, Some((first, others))) => (first.to_string(), others.len()),
        (None, None) => return None,
    };

    let first = one_line(&first);
    Some(match rest {
        0 => first,
        rest => format!("{first} (and {rest} more)"),
    })
}

#[cfg(test)]
mod tests {
    const ROOT: &str = "/repo";

    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::time::{Duration, Instant};
    use std::{env, fs, process};

    use warlock_engine::{
        Agent, Loaded, Manifest, Node, NodeState, PactEntry, Tree, Unwatched, agent, decide_state,
        load_tree, repository_root, stub_answer, subtree_hash,
    };
    use warlock_tui::{
        Account, Activities, Activity, App, Chrome, ClaudeAgent, Line, Mode, PactToggle, Run,
        Section, Sigils,
    };

    use warlock_tui::Cancel;

    use super::{
        CancelGuard, PACT_CANCELLED, PACT_LOST, Pact, PactEvent, Reloaded, Running, Toggled, Work,
        activity_port, apply_toggle, run_pact, spawn_pact,
    };
    use crate::chatting::Chat;
    use crate::session::{NOT_REFRESHED, Scope};

    fn pact_press(app: &mut App, in_flight: bool, at: Instant) -> Option<PactToggle> {
        super::pact_press(
            app,
            &Manifest::new(),
            Path::new(ROOT),
            &Sigils::Nothing,
            in_flight,
            at,
        )
    }

    fn refresh_press(app: &mut App, in_flight: bool, at: Instant) -> Option<PathBuf> {
        super::refresh_press(
            app,
            &Manifest::new(),
            Path::new(ROOT),
            &Sigils::Nothing,
            in_flight,
            at,
        )
    }

    const DOCUMENT_FILE: &str = "WARLOCK.md";

    struct Canned {
        root: PathBuf,
        refused: Vec<&'static str>,
        cancel_at: Option<(&'static str, Cancel)>,
        activities: Activities,
        seen: RefCell<Vec<(agent::Request, bool)>>,
    }

    impl Canned {
        fn new(scratch: &Scratch, refused: impl IntoIterator<Item = &'static str>) -> Self {
            Self {
                root: scratch.root.clone(),
                refused: refused.into_iter().collect(),
                cancel_at: None,
                activities: Activities::none(),
                seen: RefCell::new(Vec::new()),
            }
        }

        fn reporting(mut self, activities: Activities) -> Self {
            self.activities = activities;
            self
        }

        fn cancelling_at(mut self, directory: &'static str, cancel: Cancel) -> Self {
            self.cancel_at = Some((directory, cancel));
            self
        }

        fn directories(&self) -> Vec<PathBuf> {
            self.seen
                .borrow()
                .iter()
                .map(|(request, _)| self.relative(request.directory()))
                .collect()
        }

        fn requests(&self) -> Vec<agent::Request> {
            self.seen
                .borrow()
                .iter()
                .map(|(request, _)| request.clone())
                .collect()
        }

        fn relative(&self, directory: &Path) -> PathBuf {
            directory
                .strip_prefix(&self.root)
                .unwrap_or(directory)
                .to_path_buf()
        }

        fn saw_a_manifest(&self) -> bool {
            self.seen.borrow().iter().any(|(_, saved)| *saved)
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            let relative = self.relative(request.directory());
            self.seen
                .borrow_mut()
                .push((request.clone(), saved(&self.root).is_some()));

            // Reported before anything is answered, because that is when a
            // real pass reports: while it is still running.
            self.activities.report(Activity::Tool {
                name: "Read".to_owned(),
                detail: Some(relative.display().to_string()),
            });
            self.activities.report(Activity::Thinking);
            self.activities.report(Activity::Cost { usd: 0.25 });

            if let Some((at, cancel)) = &self.cancel_at
                && Path::new(at) == relative
            {
                cancel.cancel();
            }
            if self.refused.iter().any(|name| Path::new(name) == relative) {
                // Not the object the engine asked for, so it is turned down —
                // on every attempt, since the answer never changes: the
                // cheapest way to fail one directory of a pact for real,
                // rather than by reaching into the engine's error types, which
                // are `#[non_exhaustive]` and cannot be built from here.
                return Ok(agent::Response::new("no."));
            }
            Ok(agent::Response::new(stub_answer(request)))
        }
    }

    fn saved(root: &Path) -> Option<Manifest> {
        Manifest::load(root).ok()
    }

    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let root =
                env::temp_dir().join(format!("warlock-pact-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&root).expect("a scratch repository under the temp directory");
            Self { root }
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("a file has a parent"))
                .expect("creates the directories above a file");
            fs::write(&path, contents).expect("writes a file");
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn toggle(scratch: &Scratch, relative: &str, pacted: bool) -> Work {
        Work::Pact(PactToggle {
            path: scratch.path(relative),
            pacted,
        })
    }

    fn refreshing(scratch: &Scratch, relative: &str) -> Work {
        Work::Refresh(scratch.path(relative))
    }

    fn pact_of(path: impl Into<PathBuf>) -> Work {
        Work::Pact(PactToggle {
            path: path.into(),
            pacted: true,
        })
    }

    fn one_crate(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        scratch.write("crates/engine/src/lib.rs", "//! Core engine.\n");
        scratch
    }

    // A load walks up looking for `.git/` and refuses without one, so the file
    // below is what makes `load` work at all. It is never read or walked —
    // hidden directories are skipped — and exists only because `Scratch` makes
    // directories by writing files into them.
    fn one_crate_to_load(name: &str) -> Scratch {
        let scratch = one_crate(name);
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        scratch
    }

    fn load(scratch: &Scratch) -> (App, Scope) {
        let Loaded { tree, .. } =
            load_tree(&scratch.root).expect("a scratch repository with a `.git/` loads");
        let repo_root =
            repository_root(tree.root_path()).expect("the load found a repository root");
        let app = App::from_tree(&tree);
        let scope = Scope {
            chrome: Chrome::of(&repo_root, tree.root_path()),
            root: tree.root_path().to_path_buf(),
            repo_root,
        };
        (app, scope)
    }

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn panel_text(app: &App, now: Instant) -> Vec<String> {
        as_text(
            &app.panel()
                .account()
                .map(|account| account.lines(now))
                .unwrap_or_default(),
        )
    }

    fn as_text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                // A summary is the account's last line; a text line is a
                // document's own, and the only thing on that card.
                Line::Summary { text }
                | Line::Text { text }
                | Line::Said { text }
                | Line::Note { text }
                | Line::Wrapped { text, .. } => text.clone(),
            })
            .collect()
    }

    fn shown(app: &App, now: Instant) -> Vec<String> {
        as_text(&app.panel().window(now))
    }

    // Paths that are on no disk, so the reload at the foot of a run fails —
    // which is the case where the tree already on screen is kept.
    fn nowhere() -> Scope {
        Scope {
            root: PathBuf::from("/repo/crates"),
            repo_root: PathBuf::from("/repo"),
            chrome: Chrome::of("/repo", "/repo/crates"),
        }
    }

    fn state_of(app: &App, path: &Path) -> Option<NodeState> {
        app.rows()
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.state)
    }

    fn documents(app: &App, scratch: &Scratch) -> Vec<PathBuf> {
        app.rows()
            .iter()
            .filter(|row| {
                row.path
                    .file_name()
                    .is_some_and(|name| name == DOCUMENT_FILE)
            })
            .map(|row| {
                row.path
                    .strip_prefix(&scratch.root)
                    .unwrap_or(&row.path)
                    .to_path_buf()
            })
            .collect()
    }

    fn run_and_apply(
        scratch: &Scratch,
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        work: &Work,
        agent: &dyn Agent,
    ) {
        let before = app.clone();
        let (events, received) = mpsc::channel();
        run_pact(
            manifest,
            &scratch.root,
            work,
            agent,
            &Cancel::new(),
            &events,
        );

        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: work.clone(),
            before,
            unchanged: Vec::new(),
        });
        pact.keep_up(app, manifest, scope, Instant::now());
        assert!(!pact.running(), "the run reported its outcome and is over");
    }

    #[test]
    fn a_pact_documents_every_directory_in_the_subtree_and_grants_it() {
        let scratch = one_crate("grants");
        let agent = Canned::new(&scratch, []);

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        assert!(granted, "nothing went wrong, so the subtree is fresh");
        assert_eq!(message, None, "and there is nothing to report");

        let mut modules: Vec<&str> = manifest.entries().iter().map(PactEntry::module).collect();
        modules.sort_unstable();
        assert_eq!(modules, ["crates/engine", "crates/engine/src"]);

        for entry in manifest.entries() {
            let directory = entry.module_path(&scratch.root);
            let module = entry.module();
            assert_eq!(entry.document(), format!("{module}/{DOCUMENT_FILE}"));
            assert!(
                directory.join(DOCUMENT_FILE).is_file(),
                "{module} has no document"
            );
            // The point of the two phases: every document was written
            // before any hash was taken, so a parent is as fresh as its
            // children.
            let hash = subtree_hash(&directory).expect("a directory just written hashes");
            assert_eq!(
                decide_state(Some(entry), &hash),
                NodeState::PactedFresh,
                "{module} is not fresh"
            );
        }
    }

    #[test]
    fn the_manifest_is_written_once_and_only_when_the_pact_is_over() {
        let scratch = one_crate("once");
        let agent = Canned::new(&scratch, []);

        let Toggled { manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        // Two passes ran, and neither of them found a manifest: a save per
        // directory would have left one on disk for the second to see.
        assert_eq!(
            agent.directories(),
            [
                PathBuf::from("crates/engine/src"),
                PathBuf::from("crates/engine")
            ],
            "children before parents",
        );
        assert!(
            !agent.saw_a_manifest(),
            "the manifest was written while the pact was still running"
        );
        // And it is there afterwards, saying exactly what came back.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn entries_outside_the_pacted_subtree_are_kept_exactly_as_they_were() {
        let scratch = one_crate("outside");
        scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
        let outside = PactEntry::new(
            &scratch.root,
            scratch.path("crates/tui"),
            scratch.path("crates/tui").join(DOCUMENT_FILE),
        )
        .expect("a path under the root can be stored")
        .with_grant("earned-earlier", "2026-01-01T00:00:00Z");
        let agent = Canned::new(&scratch, []);

        let Toggled { manifest, .. } = apply_toggle(
            &Manifest::with_entries([outside.clone()]),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        // Same entry, same grant: a pact of one subtree is no judgement of
        // any other.
        assert_eq!(manifest.entry("crates/tui"), Some(&outside));
        assert!(
            !scratch.path("crates/tui").join(DOCUMENT_FILE).exists(),
            "a directory outside the pact was written to"
        );
        assert_eq!(agent.directories().len(), 2, "and no pass ran for it");
    }

    #[test]
    fn a_directory_the_pass_refuses_is_summarised_on_the_footers_one_line() {
        let scratch = one_crate("refused");
        let agent = Canned::new(&scratch, ["crates/engine/src"]);

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("half a pact is still a manifest worth writing");

        assert!(!granted, "a subtree with a failure in it is not fresh");
        let message = message.expect("the failure is reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains("crates/engine/src"),
            "the failing directory is named: {message}"
        );
        assert!(
            !message.contains("(and"),
            "one failure has nothing to count: {message}"
        );

        // No document, no entry; the ancestor inside the pact is recorded
        // with nothing granted, which is what draws it yellow.
        assert_eq!(manifest.entry("crates/engine/src"), None);
        let entry = manifest
            .entry("crates/engine")
            .expect("the ancestor is still pacted");
        assert_eq!(entry.granted_hash(), None);
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn several_things_going_wrong_are_one_line_with_the_rest_counted() {
        let scratch = one_crate("counted");
        let agent = Canned::new(&scratch, ["crates/engine/src", "crates/engine"]);

        let Toggled {
            manifest, message, ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a pact that documented nothing still saves");

        let message = message.expect("the failures are reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        // The first failure in full — children first, so it is the deeper
        // directory — and the other counted rather than quoted.
        assert!(
            message.contains("crates/engine/src"),
            "the first failure is quoted: {message}"
        );
        assert!(message.ends_with("(and 1 more)"), "{message}");
        assert!(
            manifest.entries().is_empty(),
            "nothing was documented, so nothing is recorded: {manifest:?}"
        );
    }

    #[test]
    fn un_pacting_saves_a_manifest_without_the_subtree_and_keeps_the_documents() {
        let scratch = one_crate("unpact");
        let agent = Canned::new(&scratch, []);
        let pacted = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes")
        .manifest;

        let Toggled {
            manifest,
            granted,
            message,
            ..
        } = apply_toggle(
            &pacted,
            &scratch.root,
            &toggle(&scratch, "crates/engine", false),
            &agent,
            &mut Unwatched,
        )
        .expect("dropping entries needs nothing but the manifest");

        assert!(!granted, "un-pacting grants nothing");
        assert_eq!(
            message, None,
            "the app has already said what un-pacting leaves behind"
        );
        assert!(manifest.entries().is_empty());
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        // The writing survives the claim being taken back.
        for module in ["crates/engine", "crates/engine/src"] {
            assert!(
                scratch.path(module).join(DOCUMENT_FILE).is_file(),
                "{module}'s document was deleted"
            );
        }
        assert_eq!(
            agent.directories().len(),
            2,
            "un-pacting runs no model passes"
        );
    }

    #[test]
    fn a_directory_outside_the_repository_root_is_refused_rather_than_stored() {
        // No filesystem: un-pacting is path arithmetic, and this path has
        // no manifest spelling to do it with.
        let outside = Work::Pact(PactToggle {
            path: PathBuf::from("/elsewhere/crates/engine"),
            pacted: false,
        });
        let scratch = Scratch::new("elsewhere");

        let message = apply_toggle(
            &Manifest::new(),
            Path::new(ROOT),
            &outside,
            &Canned::new(&scratch, []),
            &mut Unwatched,
        )
        .expect_err("a path outside the root has no manifest spelling");

        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains("/elsewhere/crates/engine"),
            "the refused path is named: {message}"
        );
        assert!(
            saved(Path::new(ROOT)).is_none(),
            "nothing was written anywhere"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_manifest_that_cannot_be_saved_leaves_the_previous_one_alone() {
        use std::os::unix::fs::PermissionsExt as _;

        let scratch = one_crate("readonly");
        let agent = Canned::new(&scratch, []);
        // Something for the failed save to spare: a manifest from an
        // earlier run, and the bytes it is expected to still hold.
        let previous = Manifest::with_entries([PactEntry::new(
            &scratch.root,
            scratch.path("crates/tui"),
            scratch.path("crates/tui").join(DOCUMENT_FILE),
        )
        .expect("a path under the root can be stored")]);
        previous.save(&scratch.root).expect("the first save works");
        let manifest_dir = scratch.path(".warlock");
        let before = fs::read(manifest_dir.join("pacts.toml")).expect("reads what was saved");
        fs::set_permissions(&manifest_dir, fs::Permissions::from_mode(0o555))
            .expect("chmods the manifest directory read-only");

        let message = apply_toggle(
            &previous,
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect_err("a manifest directory nobody can write to");

        // Back to writable before anything can fail, so the scratch
        // repository can still be removed.
        fs::set_permissions(&manifest_dir, fs::Permissions::from_mode(0o755))
            .expect("chmods it back");

        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.starts_with("could not read or write "),
            "the engine's own wording: {message}"
        );
        assert_eq!(
            fs::read(manifest_dir.join("pacts.toml")).expect("reads it again"),
            before,
            "the previous manifest was not touched"
        );
    }

    fn events_of(
        scratch: &Scratch,
        work: &Work,
        agent: &dyn Agent,
        cancel: &Cancel,
    ) -> Vec<PactEvent> {
        events_from(scratch, &Manifest::new(), work, agent, cancel)
    }

    fn events_from(
        scratch: &Scratch,
        manifest: &Manifest,
        work: &Work,
        agent: &dyn Agent,
        cancel: &Cancel,
    ) -> Vec<PactEvent> {
        let (events, received) = mpsc::channel();
        run_pact(manifest, &scratch.root, work, agent, cancel, &events);
        // The worker's body runs on this thread, so the sender has to be
        // dropped before anything is read or the iterator below never ends.
        // The handle stands in for the one the event loop keeps: a fresh one
        // nobody touches is a run nobody stops, and a fake that latches it part
        // way through is somebody pressing Esc.
        drop(events);
        received.into_iter().collect()
    }

    fn fractions(events: &[PactEvent]) -> Vec<(usize, usize)> {
        events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Starting {
                    position, total, ..
                } => Some((*position, *total)),
                PactEvent::Doing(_)
                | PactEvent::Requesting { .. }
                | PactEvent::Rejected { .. }
                | PactEvent::Documented { .. }
                | PactEvent::Unchanged { .. }
                | PactEvent::Finished(_) => None,
            })
            .collect()
    }

    fn announced(events: &[PactEvent], scratch: &Scratch) -> Vec<PathBuf> {
        events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Starting { directory, .. } => Some(
                    directory
                        .strip_prefix(&scratch.root)
                        .unwrap_or(directory)
                        .to_path_buf(),
                ),
                PactEvent::Doing(_)
                | PactEvent::Requesting { .. }
                | PactEvent::Rejected { .. }
                | PactEvent::Documented { .. }
                | PactEvent::Unchanged { .. }
                | PactEvent::Finished(_) => None,
            })
            .collect()
    }

    fn outcome_of(events: &[PactEvent]) -> &Result<Toggled, String> {
        match events.last() {
            Some(PactEvent::Finished(outcome)) => outcome,
            _ => panic!("the worker said: {events:?}"),
        }
    }

    fn two_crates(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        scratch.write("crates/alpha/src/lib.rs", "//! Alpha.\n");
        scratch.write("crates/beta/src/lib.rs", "//! Beta.\n");
        scratch
    }

    fn pacted(scratch: &Scratch, relative: &str) -> Manifest {
        let Toggled {
            manifest, granted, ..
        } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(scratch, relative, true),
            &Canned::new(scratch, []),
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");
        assert!(granted, "the subtree a refresh test starts from is fresh");
        manifest
    }

    #[test]
    fn the_worker_names_each_directory_in_turn_and_then_says_how_the_run_went() {
        let scratch = one_crate("progress");
        let agent = Canned::new(&scratch, []);

        let events = events_of(
            &scratch,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
        );

        // One announcement per directory, in the order the passes run —
        // children before parents — counted from one against a total that
        // does not move, each followed by what its request weighed as the
        // pass is handed it and answered by the word that its pass delivered,
        // and then exactly one outcome and nothing after it.
        let [
            PactEvent::Starting {
                directory: first,
                position: 1,
                total: 2,
            },
            PactEvent::Requesting { .. },
            PactEvent::Documented {
                directory: first_done,
            },
            PactEvent::Starting {
                directory: second,
                position: 2,
                total: 2,
            },
            PactEvent::Requesting { .. },
            PactEvent::Documented {
                directory: second_done,
            },
            PactEvent::Finished(Ok(Toggled {
                manifest,
                granted: true,
                message: None,
                ..
            })),
        ] = events.as_slice()
        else {
            panic!("the worker said: {events:?}");
        };

        assert_eq!(first, &scratch.path("crates/engine/src"));
        assert_eq!(second, &scratch.path("crates/engine"));
        assert_eq!(
            first_done, first,
            "documented names the pass that delivered"
        );
        assert_eq!(second_done, second);
        assert_eq!(agent.directories().len(), 2, "and it ran both passes");
        // The outcome that reaches the loop is the one that reached disk:
        // saved once, at the end, by the worker itself.
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
    }

    #[test]
    fn what_each_pass_is_doing_arrives_between_its_directory_and_the_next() {
        let scratch = one_crate("activities");
        // The worker's own channel, made here so the fake can be given the
        // very port `spawn_pact` gives this run's agent: one function, one
        // route, and no second channel anywhere in the picture.
        let (events, received) = mpsc::channel();
        let agent = Canned::new(&scratch, []).reporting(activity_port(&events));

        run_pact(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
            &events,
        );
        // Both ends the worker would have held: its own, and the one inside
        // the port attached to its agent. The channel closes when the last
        // of them goes, which on the real path is the worker's thread ending
        // and here is these two lines.
        drop(events);
        drop(agent);
        let events: Vec<PactEvent> = received.into_iter().collect();

        // Both streams in one sequence, in the order the run produced them:
        // a directory is announced, then what its request weighed, then what
        // its pass did, then the next directory. The three kinds of activity
        // arrive whole and unaltered — this channel carries them, it does not
        // interpret them.
        let [
            PactEvent::Starting {
                directory: first,
                position: 1,
                total: 2,
            },
            PactEvent::Requesting { .. },
            PactEvent::Doing(Activity::Tool {
                name: first_tool,
                detail: Some(first_detail),
            }),
            PactEvent::Doing(Activity::Thinking),
            PactEvent::Doing(Activity::Cost { usd: first_cost }),
            PactEvent::Documented { .. },
            PactEvent::Starting {
                directory: second,
                position: 2,
                total: 2,
            },
            PactEvent::Requesting { .. },
            PactEvent::Doing(Activity::Tool {
                detail: Some(second_detail),
                ..
            }),
            PactEvent::Doing(Activity::Thinking),
            PactEvent::Doing(Activity::Cost { .. }),
            PactEvent::Documented { .. },
            PactEvent::Finished(Ok(Toggled { granted: true, .. })),
        ] = events.as_slice()
        else {
            panic!("the worker said: {events:?}");
        };

        assert_eq!(first, &scratch.path("crates/engine/src"));
        assert_eq!(second, &scratch.path("crates/engine"));
        assert_eq!(first_tool, "Read");
        // Carried, not computed with: what is asserted is that the number
        // the fake reported is the number that came out the other end.
        assert!(
            (*first_cost - 0.25).abs() < f64::EPSILON,
            "the cost arrives as the pass said it: {first_cost}"
        );
        assert_eq!(first_detail, "crates/engine/src");
        assert_eq!(second_detail, "crates/engine");

        // The rule activities do not get to break: one outcome, and it is
        // the last thing on the channel. Everything the worker ever sent is
        // in this vector — its end was dropped before a single event was
        // read — so a stray activity after the outcome would be here.
        let outcomes = events
            .iter()
            .filter(|event| matches!(event, PactEvent::Finished(_)))
            .count();
        assert_eq!(outcomes, 1, "exactly one outcome: {events:?}");
        assert!(
            matches!(events.last(), Some(PactEvent::Finished(_))),
            "and nothing follows it: {events:?}"
        );
        // And the announcements are what they were before activities shared
        // the channel with them.
        assert_eq!(
            announced(&events, &scratch),
            [
                PathBuf::from("crates/engine/src"),
                PathBuf::from("crates/engine")
            ]
        );
    }

    #[cfg(unix)]
    // Single quotes around JSON that contains none, as in `claude.rs`'s
    // stand-ins, and `printf '%s\n' a b` is one process and no loop, so every
    // line arrives whole.
    fn stand_in(document: &str) -> String {
        let tool = concat!(
            r#"{"type":"assistant","message":{"role":"assistant","content":"#,
            r#"[{"type":"tool_use","id":"toolu_1","name":"Read","#,
            r#""input":{"file_path":"src/lib.rs"}}]}}"#,
        );
        let result = format!(
            r#"{{"type":"result","subtype":"success","result":"{document}","total_cost_usd":0.5}}"#
        );
        format!("printf '%s\\n' '{tool}' '{result}'")
    }

    #[cfg(unix)]
    #[test]
    fn a_spawned_run_reports_what_its_passes_do_over_the_channel_it_hands_back() {
        // The one thing `spawn_pact` does that driving `run_pact` cannot
        // show: attaching the port. It goes on a `ClaudeAgent` and on
        // nothing else, so this run has a real one, over a shell stand-in
        // printing a stream a pass would print. Handing the fake a port the
        // test made itself would prove only that the test can call
        // `activity_port`.
        let scratch = one_crate("spawned-activities");
        // The object the engine accepts for the one directory this run covers
        // — the leaf, so a single canned answer fits every pass — escaped as
        // the contents of a JSON string, which is how a result line carries
        // it. The quotes inside become `\"`, and the shell's single quotes
        // around the whole line leave them alone.
        let fill = serde_json::json!({
            "purpose": "The source of the engine crate.",
            "files": {"lib.rs": "the crate root of the engine library"},
        })
        .to_string();
        let escaped = serde_json::to_string(&fill).expect("a string serialises");
        let document = &escaped[1..escaped.len() - 1];
        let script = stand_in(document);
        let agent = ClaudeAgent::new()
            .with_program("/bin/sh")
            .with_args(["-c", script.as_str()]);

        let received = spawn_pact(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine/src", true),
            &agent,
            Cancel::new(),
        );
        // Blocks until every sender is gone, which is the worker's own end
        // and the one inside the port on the agent it owns. That it returns
        // at all is half the assertion: a port that outlived its run would
        // hang this line rather than fail it.
        let events: Vec<PactEvent> = received.into_iter().collect();

        let activities: Vec<&Activity> = events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Doing(activity) => Some(activity),
                _ => None,
            })
            .collect();
        // One directory, and its pass says two things: the tool it used, with
        // its one whitelisted detail, and what it cost.
        assert_eq!(
            activities.len(),
            2,
            "the pass reported through the port `spawn_pact` attached: {events:?}"
        );
        assert!(
            activities.iter().all(|activity| matches!(
                activity,
                Activity::Tool { name, detail: Some(detail) }
                    if name == "Read" && detail == "src/lib.rs"
            ) || matches!(activity, Activity::Cost { .. })),
            "and said what the stream said: {activities:?}"
        );

        // The rule a real agent does not get to break either.
        assert!(
            matches!(events.last(), Some(PactEvent::Finished(Ok(_)))),
            "one outcome, and it is the last thing on the channel: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, PactEvent::Finished(_)))
                .count(),
            1,
            "exactly one outcome: {events:?}"
        );
    }

    #[test]
    fn a_run_that_reaches_no_model_comes_back_naming_the_program_it_looked_for() {
        let scratch = one_crate("no-agent");
        // A name nothing on any `PATH` answers to, standing in for a machine
        // with no `claude` installed. The spawn fails for every directory,
        // and the run still comes back as an outcome with a line for the
        // footer rather than as a crash or as silence.
        let missing = "warlock-claude-that-is-not-installed";
        let agent = ClaudeAgent::new().with_program(missing);

        let events = events_of(
            &scratch,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &Cancel::new(),
        );

        let Some(PactEvent::Finished(Ok(Toggled {
            granted, message, ..
        }))) = events.last()
        else {
            panic!("the worker said: {events:?}");
        };

        assert!(!*granted, "nothing was documented, so nothing is fresh");
        let message = message.as_deref().expect("the failure is reported");
        assert!(!message.contains('\n'), "the footer is one line: {message}");
        assert!(
            message.contains(missing),
            "the binary that could not be run is named: {message}"
        );
    }

    #[test]
    fn the_pact_key_starts_nothing_while_a_pact_is_in_flight() {
        // No filesystem: which subtree a press asks for is app state, and
        // the tree it is asked of is written out here.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        // The run the press is going to bounce off, as the event loop would
        // have told the app about it, and the sentence the last keystroke
        // left, which is not this press's to spend.
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);
        app.set_message("something the last key said");
        let before = app.clone();

        assert_eq!(
            pact_press(&mut app, true, Instant::now()),
            None,
            "no second pact"
        );
        // Saying so is the whole of it: the same app with the flag set and
        // nothing else moved — no colour, no selection, no account started
        // and the rows exactly as they were.
        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running"),
            "the refusal is worded onto the line the reader is watching"
        );
        assert_eq!(
            app.message(),
            Some("something the last key said"),
            "the refusal did not go through the message"
        );

        // The same press, with nothing running, is the press that starts a
        // pact: same key, same app, different answer. The run being over is
        // the loop's news, so the app hears it the way it always does.
        app.clear_pact_in_flight();
        let toggle =
            pact_press(&mut app, false, Instant::now()).expect("a directory can be pacted");
        assert_eq!(toggle.path, PathBuf::from("/repo/crates"));
        assert!(toggle.pacted);
        assert_ne!(app, before, "the subtree it covers is painted");
    }

    #[test]
    fn leaning_on_the_pact_key_while_a_pact_is_in_flight_changes_nothing_further() {
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);

        assert_eq!(pact_press(&mut app, true, Instant::now()), None);
        let after_one = app.clone();

        // A reader who presses again is asking the same question, and the
        // answer already on screen is the same one. There is nothing to
        // count and nothing to say twice.
        assert_eq!(pact_press(&mut app, true, Instant::now()), None);
        assert_eq!(pact_press(&mut app, true, Instant::now()), None);

        assert_eq!(app, after_one, "pressing again changed something");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running")
        );
    }

    #[test]
    fn a_press_on_a_file_row_with_nothing_running_still_refuses_through_the_message() {
        // A directory with a file under it, and the files shown, so the
        // selection can sit on the one row a pact is never about.
        let tree = Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, NodeState::Unpacted)
                .with_files([PathBuf::from("/repo/crates/Cargo.toml")]),
        );
        let mut app = App::from_tree(&tree);
        app.toggle_files();
        app.select_next();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("/repo/crates/Cargo.toml")),
            "the selection is on the file"
        );

        assert_eq!(
            pact_press(&mut app, false, Instant::now()),
            None,
            "a file is not a thing to pact"
        );

        // Its own refusal, in its own place: no run has the message line, so
        // the sentence the app wrote is the one that is drawn. And no flag,
        // because no run turned this press down.
        assert!(
            app.message().is_some_and(|message| !message.is_empty()),
            "the file row said nothing"
        );
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");
    }

    #[test]
    fn a_press_on_a_row_the_ignore_file_keeps_out_starts_nothing() {
        // A directory the repository's `.warlockignore` excludes, said the way
        // the load says it: a flag on the node, so no filesystem is needed to
        // answer the key.
        let tree =
            Tree::new(
                Node::new("/repo", None::<PathBuf>, NodeState::Unpacted).with_children([
                    Node::new("/repo/notes", None::<PathBuf>, NodeState::Unpacted)
                        .with_ignored(true),
                ]),
            );
        let mut app = App::from_tree(&tree);
        app.select_next();
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(PathBuf::from("/repo/notes")),
            "the selection is on the excluded directory"
        );
        let before = app.clone();

        assert_eq!(
            pact_press(&mut app, false, Instant::now()),
            None,
            "there is nothing in there to pact"
        );

        // The refusal is the app's, so it goes through the message, and this
        // function adds nothing to it: no account is opened for a run that never
        // started, and no flag is set, because no run turned the press down.
        assert!(
            app.message()
                .is_some_and(|message| message.contains(".warlockignore")),
            "the excluded row said nothing about the rules that keep it out"
        );
        assert!(
            !app.panel().has_account(),
            "an account was opened for no run"
        );
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");
        let refused = {
            let mut refused = before.clone();
            refused.set_message(app.message().expect("the app said why"));
            refused
        };
        assert_eq!(app, refused, "the press did more than say so");
    }

    fn scoped() -> Manifest {
        Manifest::with_entries([PactEntry::new(
            ROOT,
            format!("{ROOT}/crates"),
            format!("{ROOT}/crates/WARLOCK.md"),
        )
        .expect("a module under the root")
        .with_scope("data-plane")])
    }

    const CLOSED: &str =
        "/repo/crates is scoped `data-plane` — hold that sigil to work here, with `warlock config`";

    fn app_over(state: NodeState) -> App {
        App::from_tree(&Tree::new(Node::new(
            format!("{ROOT}/crates"),
            None::<PathBuf>,
            state,
        )))
    }

    #[test]
    fn p_is_refused_on_a_directory_whose_scope_this_machine_does_not_hold() {
        let mut app = app_over(NodeState::Unpacted);
        let before = app.clone();

        let toggle = super::pact_press(
            &mut app,
            &scoped(),
            Path::new(ROOT),
            &Sigils::held(["web"]),
            false,
            Instant::now(),
        );

        assert_eq!(toggle, None, "the press must not paint or start a run");
        assert_eq!(app.message(), Some(CLOSED));
        assert!(
            !app.panel().has_account(),
            "a refused press opened an account for a run that never happened"
        );
        let said = {
            let mut said = before.clone();
            said.set_message(CLOSED);
            said
        };
        assert_eq!(app, said, "the refusal did more to the app than say so");
    }

    #[test]
    fn the_un_pact_direction_is_refused_too_and_is_what_this_is_for() {
        // The hazard the sigil guards against, spelled out: `p` on a pacted row
        // is an un-pact, `unpact_subtree` drops the entries and the scope with
        // them, and a second `p` costs a full model pass and does not bring the
        // boundary back. So this direction is not "its own undo" and must not be
        // one fumbled keystroke away in somebody else's subtree.
        for state in [NodeState::PactedFresh, NodeState::PactedStale] {
            let mut app = app_over(state);
            let before = app.clone();

            let toggle = super::pact_press(
                &mut app,
                &scoped(),
                Path::new(ROOT),
                &Sigils::held(["web"]),
                false,
                Instant::now(),
            );

            assert_eq!(toggle, None, "{state:?} was un-pacted across a boundary");
            assert_eq!(app.message(), Some(CLOSED));
            let said = {
                let mut said = before.clone();
                said.set_message(CLOSED);
                said
            };
            assert_eq!(app, said, "{state:?} moved under a refused press");
        }
    }

    fn scoped_below() -> Manifest {
        let entry = |module: &str| {
            PactEntry::new(
                ROOT,
                format!("{ROOT}/{module}"),
                format!("{module}/WARLOCK.md"),
            )
            .expect("a module under the root")
        };
        Manifest::with_entries([
            entry("crates"),
            entry("crates/engine").with_scope("data-plane"),
        ])
    }

    const CLOSED_BELOW: &str = "un-pacting /repo/crates would drop pacts scoped `data-plane` — \
                                hold that sigil with `warlock config`, or un-pact the parts you \
                                hold";

    #[test]
    fn p_un_pacting_ward_is_refused_by_a_boundary_below_the_row() {
        // The row is unscoped and opens to anybody, and the entry under it is
        // not: an un-pact drops that entry, an entry is the only home a scope
        // has, and a boundary that can be erased by aiming at its parent is not
        // one. Argued in
        // `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
        for state in [NodeState::PactedFresh, NodeState::PactedStale] {
            let mut app = app_over(state);
            let before = app.clone();

            let toggle = super::pact_press(
                &mut app,
                &scoped_below(),
                Path::new(ROOT),
                &Sigils::held(["web"]),
                false,
                Instant::now(),
            );

            assert_eq!(toggle, None, "{state:?} un-pacted a boundary below it");
            assert_eq!(app.message(), Some(CLOSED_BELOW));
            let said = {
                let mut said = before.clone();
                said.set_message(CLOSED_BELOW);
                said
            };
            assert_eq!(app, said, "{state:?} moved under a refused press");
        }
    }

    #[test]
    fn the_boundary_below_binds_the_un_pact_direction_and_no_other_key() {
        // The rule follows destruction, not traversal. A pact and a refresh
        // provably leave every scope where they found it, and gating `r` at the
        // root of a scoped monorepo would refuse the ordinary gesture for
        // everybody holding fewer than all of its sigils.
        let mut app = app_over(NodeState::Unpacted);
        let toggle = super::pact_press(
            &mut app,
            &scoped_below(),
            Path::new(ROOT),
            &Sigils::held(["web"]),
            false,
            Instant::now(),
        )
        .expect("a pact leaves every scope below exactly as it found it");
        assert!(toggle.pacted);
        assert_eq!(app.message(), None, "the pact direction was told something");

        let mut app = app_over(NodeState::PactedStale);
        assert_eq!(
            super::refresh_press(
                &mut app,
                &scoped_below(),
                Path::new(ROOT),
                &Sigils::held(["web"]),
                false,
                Instant::now()
            ),
            Some(PathBuf::from(format!("{ROOT}/crates"))),
            "a refresh was gated on a boundary it cannot destroy"
        );
        assert_eq!(app.message(), None, "the refresh was told something");
    }

    #[test]
    fn holding_the_boundary_below_lets_the_un_pact_through() {
        for sigils in [Sigils::held(["data-plane"]), Sigils::held(["*"])] {
            let mut app = app_over(NodeState::PactedFresh);

            let toggle = super::pact_press(
                &mut app,
                &scoped_below(),
                Path::new(ROOT),
                &sigils,
                false,
                Instant::now(),
            )
            .unwrap_or_else(|| panic!("{sigils:?} opens the boundary below"));

            assert!(!toggle.pacted, "the press was an un-pact");
            // The toggle words its own success line, so `None` is not the
            // assertion here: what matters is that the boundary did not speak.
            assert_ne!(app.message(), Some(CLOSED_BELOW), "{sigils:?} was refused");
        }
    }

    #[test]
    fn r_is_refused_on_a_directory_whose_scope_this_machine_does_not_hold() {
        let mut app = app_over(NodeState::PactedStale);
        let before = app.clone();

        let directory = super::refresh_press(
            &mut app,
            &scoped(),
            Path::new(ROOT),
            &Sigils::held(["web"]),
            false,
            Instant::now(),
        );

        // What a refresh spends is model time inside somebody else's boundary,
        // on documents that are theirs to have an opinion about.
        assert_eq!(directory, None);
        assert_eq!(app.message(), Some(CLOSED));
        assert!(
            !app.panel().has_account(),
            "a refused refresh opened an account"
        );
        let said = {
            let mut said = before.clone();
            said.set_message(CLOSED);
            said
        };
        assert_eq!(app, said, "the refusal did more to the app than say so");
    }

    #[test]
    fn holding_the_scope_lets_both_keys_through() {
        for sigils in [
            Sigils::held(["data-plane"]),
            Sigils::held(["web", "data-plane"]),
            Sigils::held(["*"]),
        ] {
            let mut app = app_over(NodeState::Unpacted);
            let toggle = super::pact_press(
                &mut app,
                &scoped(),
                Path::new(ROOT),
                &sigils,
                false,
                Instant::now(),
            )
            .unwrap_or_else(|| panic!("{sigils:?} opens `data-plane` for a pact"));
            assert!(toggle.pacted);
            assert_eq!(app.message(), None, "{sigils:?} said something");

            let mut app = app_over(NodeState::PactedStale);
            assert_eq!(
                super::refresh_press(
                    &mut app,
                    &scoped(),
                    Path::new(ROOT),
                    &sigils,
                    false,
                    Instant::now()
                ),
                Some(PathBuf::from(format!("{ROOT}/crates"))),
                "{sigils:?} opens `data-plane` for a refresh"
            );
        }
    }

    #[test]
    fn a_machine_that_holds_no_sigil_is_refused_by_both_keys() {
        // The state every machine is in before anybody runs `warlock config`,
        // and the one this was originally built the wrong way round: an empty
        // set opens nothing that is scoped, because a sigil is what opens a
        // scope. Reading it as unrestricted would make the default state of
        // every machine a universal bypass.
        for sigils in [Sigils::Nothing, Sigils::Unknown] {
            let mut app = app_over(NodeState::Unpacted);
            assert_eq!(
                super::pact_press(
                    &mut app,
                    &scoped(),
                    Path::new(ROOT),
                    &sigils,
                    false,
                    Instant::now()
                ),
                None,
                "{sigils:?} pacted across a boundary"
            );
            assert_eq!(app.message(), Some(CLOSED));

            let mut app = app_over(NodeState::PactedStale);
            assert_eq!(
                super::refresh_press(
                    &mut app,
                    &scoped(),
                    Path::new(ROOT),
                    &sigils,
                    false,
                    Instant::now()
                ),
                None,
                "{sigils:?} refreshed across a boundary"
            );
            assert_eq!(app.message(), Some(CLOSED));
        }
    }

    #[test]
    fn an_unscoped_directory_stays_open_to_a_machine_holding_nothing() {
        // The permissive default is on the directory and only there: a
        // repository nobody has scoped is unaffected by boundaries existing.
        let mut app = app_over(NodeState::Unpacted);

        let toggle = super::pact_press(
            &mut app,
            &Manifest::new(),
            Path::new(ROOT),
            &Sigils::Nothing,
            false,
            Instant::now(),
        )
        .expect("nothing scopes this directory");

        assert!(toggle.pacted);
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_run_in_flight_is_answered_before_the_boundary_is() {
        let mut app = app_over(NodeState::PactedStale);
        app.set_pact_in_flight(format!("{ROOT}/crates"), 1, 2);

        // Both refusals apply, and the in-flight one is the one on screen: it
        // goes on the progress line the reader is already watching, leaving the
        // message line a run has taken alone.
        assert_eq!(
            super::pact_press(
                &mut app,
                &scoped(),
                Path::new(ROOT),
                &Sigils::held(["web"]),
                true,
                Instant::now()
            ),
            None
        );
        assert_eq!(
            app.message(),
            None,
            "the boundary spoke over a run's own line"
        );
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting /repo/crates (1/2) — already running")
        );
    }

    #[test]
    fn the_press_that_starts_a_run_starts_an_account_and_an_un_pact_leaves_it_alone() {
        // One directory and three presses of the same key: a pact, the
        // un-pact that undoes it, and a second pact. No filesystem, because
        // what is under test is what a press does to the app.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();

        assert!(!app.panel().has_account(), "no pact has run this session");

        // The pact, and a line of the run it started.
        let toggle = pact_press(&mut app, false, base).expect("a directory can be pacted");
        assert!(toggle.pacted);
        let account = app
            .panel_mut()
            .account_mut()
            .expect("the press started an account");
        account.open_section("crates", base);
        account.record(&Activity::Thinking, at(base, 1));
        assert_eq!(app.panel().account().map(Account::line_count), Some(2));

        // The un-pact. It runs no pass and reports nothing, so wiping the
        // record of the run that wrote the documents it is removing would
        // cost the reader the only account they have for no news at all.
        let toggle = pact_press(&mut app, false, at(base, 2)).expect("it is pacted now");
        assert!(!toggle.pacted, "the second press takes the pact off");
        assert_eq!(
            app.panel().account().map(Account::line_count),
            Some(2),
            "the last run's account is still on screen"
        );

        // The second pact. One pact, one account: this one starts empty
        // rather than under the last one.
        let toggle = pact_press(&mut app, false, at(base, 3)).expect("it can be pacted again");
        assert!(toggle.pacted);
        assert_eq!(
            app.panel().account().map(Account::line_count),
            Some(0),
            "a new run starts from nothing"
        );
    }

    #[test]
    fn either_key_starts_nothing_while_a_run_is_in_flight() {
        // No filesystem: what a press comes to while something is running is
        // app state and a bit of wording. The run in flight is a refresh, and
        // the account it opened has a section in it, so that a refusal that
        // cleared the panel would show.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedStale,
        ));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.start_account(base);
        app.panel_mut()
            .account_mut()
            .expect("the press that started the run opened one")
            .open_section("engine", base);
        app.set_run_in_flight(Run::Refresh, "/repo/crates/engine", 2, 7);
        app.set_message("something the last key said");
        let before = app.clone();

        // The refresh key during a refresh, and the pact key during the same
        // refresh: one in-flight check, so one answer.
        assert_eq!(
            refresh_press(&mut app, true, at(base, 1)),
            None,
            "no second run"
        );
        assert_eq!(
            pact_press(&mut app, true, at(base, 2)),
            None,
            "and not by the other key either"
        );

        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the presses did more than say so");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("refreshing engine (2/7) — already running"),
            "the refusal is worded onto the line the reader is watching"
        );
        assert_eq!(
            app.message(),
            Some("something the last key said"),
            "the refusal did not go through the message"
        );
        assert_eq!(
            app.panel().account().map(Account::line_count),
            before.panel().account().map(Account::line_count),
            "the running run's account was cleared"
        );

        // And the other direction, which is the same sentence about the other
        // run: a refresh asked for during a pact.
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);
        assert_eq!(refresh_press(&mut app, true, at(base, 3)), None);
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running")
        );
    }

    #[test]
    fn a_second_run_is_refused_in_the_same_words_with_the_thread_showing() {
        // The refusal a conversation changes nothing about. A run started while
        // the reader is looking at the thread fills the account card behind it,
        // so the run key pressed again — from the tree, since the field under
        // the thread has the keyboard otherwise — has to bounce off the same
        // in-flight check, in the same words, without touching either card or
        // the conversation the reader is reading.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut app = App::from_tree(&tree);
        app.panel_mut().set_height(WHOLE_PANEL);
        let base = Instant::now();

        // A question and its answer, then a run started over the top of them.
        app.panel_mut()
            .start_turn("what does the engine do?", at(base, 1));
        app.panel_mut()
            .answer_turn("It walks the tree.", at(base, 2));
        app.start_account(at(base, 3));
        // Through the one call the loop writes a run with.
        app.panel_mut()
            .write_run(|account| account.open_section("engine", at(base, 3)));
        app.set_pact_in_flight("/repo/crates/engine", 3, 12);
        app.set_message("something the last key said");
        assert!(
            app.panel().showing_thread(),
            "the run swapped the card away"
        );
        let before = app.clone();
        // Read at the instant the assertion below reads it at, so what is
        // compared is the lines and not how long the clocks have been running.
        let thread = shown(&app, at(base, 6));

        assert_eq!(
            pact_press(&mut app, true, at(base, 4)),
            None,
            "no second run"
        );
        assert_eq!(
            refresh_press(&mut app, true, at(base, 5)),
            None,
            "and not by the other key either"
        );

        // The same wording as the refusal with no conversation behind it, on the
        // same line, and the flag is the whole of what the presses moved.
        let refused = {
            let mut refused = before.clone();
            refused.set_pact_refused();
            refused
        };
        assert_eq!(app, refused, "the presses did more than say so");
        assert_eq!(
            app.pact_line().as_deref(),
            Some("pacting engine (3/12) — already running"),
            "the refusal is worded onto the line the reader is watching"
        );
        assert_eq!(
            app.message(),
            Some("something the last key said"),
            "the refusal did not go through the message"
        );

        // And the reader is still looking at what they were looking at: the same
        // card, the same lines, and none of the run's among them.
        assert!(app.panel().showing_thread(), "the refusal swapped the card");
        assert_eq!(shown(&app, at(base, 6)), thread, "the refusal moved a line");
        assert!(
            thread.iter().any(|line| line == "what does the engine do?"),
            "the typed turn is not in the thread: {thread:?}"
        );
        assert!(
            !thread.iter().any(|line| line == "engine"),
            "the run's section is in the thread: {thread:?}"
        );
    }

    #[test]
    fn a_refresh_of_a_directory_with_nothing_stale_under_it_starts_no_run() {
        // The good outcome: the reader asked whether anything needed
        // describing again and the answer is no, which costs a sentence rather
        // than a run.
        let fresh = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedFresh,
        ));
        let mut app = App::from_tree(&fresh);
        let base = Instant::now();

        assert_eq!(
            refresh_press(&mut app, false, base),
            None,
            "there is nothing under it to describe again"
        );
        assert!(
            !app.panel().has_account(),
            "a run nobody started has no account"
        );
        assert!(
            app.message()
                .is_some_and(|message| message.contains("already fresh")),
            "the fresh row said nothing: {:?}",
            app.message()
        );
        assert_eq!(app.pact_line(), None, "nothing is running to be refused by");

        // The same key on the same directory gone stale is the press that
        // starts a run, and it starts the panel's account at the moment of the
        // press — which is what the clocks under it are counted from.
        let stale = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::PactedStale,
        ));
        let mut app = App::from_tree(&stale);
        assert_eq!(
            refresh_press(&mut app, false, base),
            Some(PathBuf::from("/repo/crates")),
            "a stale subtree is a thing to refresh"
        );
        let account = app
            .panel_mut()
            .account_mut()
            .expect("the press started an account");
        account.open_section("crates", at(base, 5));
        account.finish(at(base, 5));
        assert_eq!(
            panel_text(&app, at(base, 5)),
            [
                "crates".to_owned(),
                "0:00 waiting".to_owned(),
                // Five seconds of run, and nothing in it had happened yet:
                // the account was opened by the press.
                "pact finished — 1 directory, 0:05, $0.00 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
            ],
            "the run is counted from the press that started it"
        );
    }

    #[test]
    fn a_cancel_stops_the_descent_and_records_only_what_finished() {
        let scratch = two_crates("cancelled");
        let cancel = Cancel::new();
        // One directory that goes wrong on its own, and one that the reader
        // stops the run during: `alpha/src` is refused the way a pass that
        // fails always is, and Esc is pressed while `alpha` is being worked.
        // The engine walks in reverse path order, so `beta` comes before
        // `alpha` and the pact's own root comes last of all.
        let agent = Canned::new(&scratch, ["crates/alpha/src"])
            .cancelling_at("crates/alpha", cancel.clone());

        let events = events_of(&scratch, &toggle(&scratch, "crates", true), &agent, &cancel);

        // Four directories offered a pass out of five, and the fifth — the
        // root of the pact, worked last because parents come after their
        // children — never announced and never run: the descent stopped
        // between directories rather than part way through one.
        let worked = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates/alpha/src"),
            PathBuf::from("crates/alpha"),
        ];
        assert_eq!(announced(&events, &scratch), worked);
        // The refused directory costs two passes — the engine asks once more
        // with the defects listed before giving up — so it is seen twice.
        assert_eq!(
            agent.directories(),
            [
                PathBuf::from("crates/beta/src"),
                PathBuf::from("crates/beta"),
                PathBuf::from("crates/alpha/src"),
                PathBuf::from("crates/alpha/src"),
                PathBuf::from("crates/alpha"),
            ]
        );

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a cancelled pact still saves what it finished: {events:?}");
        };
        assert!(!granted, "a run that was stopped has proved nothing fresh");
        // The refusal above is in this run's failures, and the footer says
        // none of it: what happened is that somebody pressed Esc.
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));

        // What is on disk is what finished, and nothing else. `beta` was
        // documented with nothing missing under it, so it earned a grant and
        // draws green; `alpha` was documented over a child that failed, so it
        // has an entry with no grant and draws yellow; everything the run
        // never reached has no entry at all.
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        for module in ["crates/beta", "crates/beta/src"] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "a subtree that finished is green: {manifest:?}"
            );
        }
        let hash = subtree_hash(scratch.path("crates/alpha")).expect("it hashes");
        assert_eq!(
            decide_state(manifest.entry("crates/alpha"), &hash),
            NodeState::PactedStale,
            "a directory with an unfinished descendant is yellow: {manifest:?}"
        );
        for module in ["crates/alpha/src", "crates"] {
            assert_eq!(
                manifest.entry(module),
                None,
                "`{module}` was never documented, so it is not recorded"
            );
        }

        // And every document written before the cancel is still there.
        for module in ["crates/alpha", "crates/beta", "crates/beta/src"] {
            assert!(
                scratch.path(module).join(DOCUMENT_FILE).is_file(),
                "`{module}`'s document did not survive the cancel"
            );
        }
    }

    #[test]
    fn a_cancel_grants_nothing_even_when_no_pass_went_wrong() {
        let scratch = two_crates("cancelled-clean");
        let cancel = Cancel::new();
        // Nothing refused this time: the run is stopped with every pass it
        // ran having worked, which is what a cancel usually looks like. Esc
        // lands on the last directory before the pact's own root, which the
        // engine works last of all.
        let agent = Canned::new(&scratch, []).cancelling_at("crates/alpha", cancel.clone());

        let events = events_of(&scratch, &toggle(&scratch, "crates", true), &agent, &cancel);

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a cancelled pact still saves what it finished: {events:?}");
        };
        // Not a failure in sight, and still not fresh: the subtree's own
        // root was never documented, so painting the whole of it green —
        // which is all `granted` is for — would be a claim about
        // directories this run never opened.
        assert!(
            !granted,
            "a stopped run grants nothing, whatever it did not trip over"
        );
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));
        assert_eq!(manifest.entry("crates"), None);
        assert!(
            !scratch.path("crates").join(DOCUMENT_FILE).exists(),
            "the root of the pact was never worked"
        );
        // The crates that did finish kept everything they earned.
        for module in [
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` finished before the cancel: {manifest:?}"
            );
        }
    }

    #[test]
    fn a_directory_the_run_carried_is_announced_unchanged_rather_than_documented() {
        // The same fixture as the test below, minus the write into `crates`
        // itself: only `beta/src` has anything new, so `beta` is described
        // because its child's document moved under it, and `crates` is carried
        // — its own files and its children's documents are what they were.
        let scratch = two_crates("carried");
        let before = pacted(&scratch, "crates");
        scratch.write("crates/beta/src/lib.rs", "//! Beta, and something new.\n");

        let agent = Canned::new(&scratch, []);
        let events = events_from(
            &scratch,
            &before,
            &refreshing(&scratch, "crates"),
            &agent,
            &Cancel::new(),
        );

        let carried: Vec<PathBuf> = events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Unchanged { directory } => Some(
                    directory
                        .strip_prefix(&scratch.root)
                        .unwrap_or(directory)
                        .to_path_buf(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(
            carried,
            [PathBuf::from("crates")],
            "the one directory that needed no pass says so in its own word",
        );

        // And it is not also announced as documented: one word per directory,
        // or a front end would draw the write that did not happen.
        let documented: Vec<PathBuf> = events
            .iter()
            .filter_map(|event| match event {
                PactEvent::Documented { directory } => Some(
                    directory
                        .strip_prefix(&scratch.root)
                        .unwrap_or(directory)
                        .to_path_buf(),
                ),
                _ => None,
            })
            .collect();
        assert!(
            !documented.contains(&PathBuf::from("crates")),
            "a carried directory is announced once, and not as documented: \
             {documented:?}",
        );

        // The pass really was skipped, which is the whole point of the word.
        assert!(
            !agent.directories().contains(&PathBuf::from("crates")),
            "no pass was paid for at `crates`",
        );
    }

    #[test]
    fn a_refresh_describes_the_stale_directories_and_leaves_the_fresh_ones_as_they_were() {
        // A pact of everything, then one file moved under `beta`, then the
        // refresh key on the root of it all. Three of the five directories are
        // stale — the one holding the file, its parent and the root above them
        // — and the run is over the three rather than the five.
        let scratch = two_crates("refreshed");
        let before = pacted(&scratch, "crates");
        let kept: Vec<PactEntry> = ["crates/alpha", "crates/alpha/src"]
            .into_iter()
            .map(|module| {
                before
                    .entry(module)
                    .expect("the pact recorded every directory")
                    .clone()
            })
            .collect();
        scratch.write("crates/beta/src/lib.rs", "//! Beta, and something new.\n");
        // And one directly in `crates`, so the root of the run has something
        // new to read too. Without it the early cutoff carries its document
        // forward — `beta`'s document did not move — and the run is two
        // directories rather than the three this is about.
        scratch.write("crates/shared.rs", "//! Shared, and something new.\n");

        let agent = Canned::new(&scratch, []);
        let events = events_from(
            &scratch,
            &before,
            &refreshing(&scratch, "crates"),
            &agent,
            &Cancel::new(),
        );

        // Announced and worked in the same reverse order a pact walks, and
        // `alpha` in neither list: a pass is what a refresh is trying not to
        // spend.
        let stale = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates"),
        ];
        assert_eq!(announced(&events, &scratch), stale);
        assert_eq!(agent.directories(), stale);
        // And the fraction counts the run rather than the subtree: three of
        // three, from the engine's observer, with five directories under the
        // key that was pressed.
        assert_eq!(fractions(&events), [(1, 3), (2, 3), (3, 3)]);

        let Ok(Toggled {
            manifest,
            granted,
            message,
            ..
        }) = outcome_of(&events)
        else {
            panic!("a refresh that walks and saves: {events:?}");
        };
        assert!(granted, "no pass went wrong, so the subtree is fresh");
        assert_eq!(*message, None, "and there is nothing to report");

        // The skipped directories are the entries they were, grant and all —
        // the engine's rule (WAR-39), which is what lets the whole subtree be
        // painted fresh off `granted` alone.
        for entry in &kept {
            assert_eq!(
                manifest.entry(entry.module()),
                Some(entry),
                "a skipped directory was rewritten"
            );
        }
        assert_eq!(
            &saved(&scratch.root).expect("the manifest was written"),
            manifest,
            "the one save at the end is this run's"
        );
        for module in [
            "crates",
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` is not fresh after the refresh: {manifest:?}"
            );
        }
    }

    #[test]
    fn a_cancelled_refresh_saves_what_it_finished_and_says_it_was_stopped() {
        // Both crates move, so every directory under `crates` is stale and the
        // refresh has all five to work; Esc lands while `alpha` is being
        // described, which is the fourth of them.
        let scratch = two_crates("refresh-cancelled");
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        let before = pacted(&scratch, "crates");
        scratch.write("crates/alpha/src/lib.rs", "//! Alpha, edited.\n");
        scratch.write("crates/beta/src/lib.rs", "//! Beta, edited.\n");

        let (mut app, scope) = load(&scratch);
        let mut manifest = before.clone();
        let base = Instant::now();
        app.start_account(base);

        // The event loop's own handle and the clone the run answers to: the
        // pair `spawn_pact` makes, so what the panel reads at the end is the
        // flag the reader's key latched.
        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let work = refreshing(&scratch, "crates");
        let said = recorded_from(&scratch, &before, &work, &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/alpha", cancel.clone())
        });

        // The descent stopped between directories, exactly as a cancelled pact
        // stops: four of the five offered a pass, and the root of the run —
        // worked last, after its children — was never announced and never run.
        let worked = [
            PathBuf::from("crates/beta/src"),
            PathBuf::from("crates/beta"),
            PathBuf::from("crates/alpha/src"),
            PathBuf::from("crates/alpha"),
        ];
        assert_eq!(announced(&said, &scratch), worked);
        assert_eq!(fractions(&said), [(1, 5), (2, 5), (3, 5), (4, 5)]);

        let Ok(Toggled {
            granted, message, ..
        }) = outcome_of(&said)
        else {
            panic!("a cancelled refresh still saves what it finished: {said:?}");
        };
        // Reworded as the stopped run it is, which is the whole of what a
        // refresh needed of `run_pact`: a pact says this and an un-pact cannot.
        assert_eq!(message.as_deref(), Some(PACT_CANCELLED));
        assert!(!granted, "a run that was stopped has proved nothing fresh");

        let progress = replay_work(&mut app, &mut manifest, &scope, guard, &work, said, base);
        assert_eq!(progress.len(), 4, "one line per directory: {progress:?}");

        // What the run finished is on disk. The four directories that did are
        // green again, and the root the cancel came before keeps the entry its
        // pact gave it and stays yellow: a refresh drops nothing it could not
        // re-describe.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest,
            "what the run finished is on disk"
        );
        for module in [
            "crates/alpha",
            "crates/alpha/src",
            "crates/beta",
            "crates/beta/src",
        ] {
            let hash = subtree_hash(scratch.path(module)).expect("it hashes");
            assert_eq!(
                decide_state(manifest.entry(module), &hash),
                NodeState::PactedFresh,
                "`{module}` finished before the cancel: {manifest:?}"
            );
        }
        let hash = subtree_hash(scratch.path("crates")).expect("it hashes");
        assert_eq!(
            decide_state(manifest.entry("crates"), &hash),
            NodeState::PactedStale,
            "the directory the cancel came before is still pacted and still stale"
        );
        assert_eq!(
            state_of(&app, &scratch.path("crates")),
            Some(NodeState::PactedStale),
            "a stopped run painted the subtree green"
        );

        // And the panel says it was stopped, in the section it was stopped in
        // and with what that pass had spent by then — the same ending a
        // cancelled pact leaves, because it is the same code closing it.
        let lines = panel_text(&app, at(base, 10_000));
        assert_eq!(
            &lines[lines.len() - 6..],
            [
                "crates/alpha".to_owned(),
                // What the stopped pass had been handed: no files at all — this
                // directory holds nothing but documents — and the bytes of its
                // child's document. Its own previous document is not among
                // them: no pass is shown its predecessor.
                // Through `account::size` rather than with the unit spelled
                // out: what this line asserts is the count, and a fixture that
                // grew past a kilobyte should not be able to fail a test about
                // where a cancel is recorded.
                format!(
                    "0:20 waiting · 0 files, {}",
                    warlock_tui::size(document_bytes(&scratch, "crates/alpha/src"))
                ),
                "0:30 Read crates/alpha".to_owned(),
                "1:00 thinking".to_owned(),
                "1:00 cancelled — $0.25 spent".to_owned(),
                "pact finished — 4 directories, 4:00, $1.00".to_owned(),
            ],
            "the cancel is recorded in the section it happened in"
        );
        assert_eq!(app.message(), Some(PACT_CANCELLED));
    }

    #[test]
    fn a_run_that_goes_out_of_scope_takes_its_claude_with_it() {
        // What quitting mid-pact comes to: `q` and Ctrl-C return from the
        // event loop, the run goes out of scope with them, and the child it
        // was waiting on is killed on the way — no join, and no `claude`
        // left running against a terminal nobody is looking at any more.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let cancel = CancelGuard::new();
        let watching = cancel.handle();
        let (_events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel,
            work: pact_of("/repo/crates"),
            before: App::from_tree(&tree),
            unchanged: Vec::new(),
        };

        assert!(!watching.is_cancelled(), "a run in flight is not cancelled");

        drop(running);

        assert!(
            watching.is_cancelled(),
            "the run outlived the loop that started it"
        );
    }

    fn a_run_in_flight(base: Instant) -> (App, App, Manifest, mpsc::Sender<PactEvent>, Running) {
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        app.start_account(base);
        let (events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
            unchanged: Vec::new(),
        };
        (app, before, Manifest::new(), events, running)
    }

    #[test]
    fn a_run_says_it_ended_on_one_round_and_no_other() {
        let base = Instant::now();
        let (mut app, _before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Pact::with_run(running);

        // Still going: the worker has named a directory and nothing more.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 1,
            })
            .expect("the loop is still listening");
        assert!(
            pact.keep_up(&mut app, &mut manifest, &nowhere(), base)
                .is_none(),
            "a run that is still going said it had ended"
        );
        assert!(pact.running(), "the run was taken down while still talking");

        // The round the outcome lands on, and the only one that answers.
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        assert!(
            pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 1))
                .is_some(),
            "the run ended without saying so"
        );
        assert!(
            !pact.running(),
            "the run is over and still says it is running"
        );

        // And every round after it, for as long as nothing else is started.
        assert!(
            pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 2))
                .is_none(),
            "the end was reported a second time"
        );
    }

    #[test]
    fn a_pass_fills_the_panel_under_the_directory_it_is_working_on() {
        // The directory the worker names opens a section, everything the
        // pass is then seen doing lands under it one line at a time, a cost
        // is money rather than a line, and the footer goes on saying exactly
        // what it always said.
        let base = Instant::now();
        let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Pact::with_run(running);

        // The first directory, on its own: the run has reached it and the
        // pass has not said anything yet.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);

        assert!(pact.running(), "a run that is talking is still running");
        assert_eq!(
            panel_text(&app, at(base, 2)),
            ["engine", "0:02 waiting"],
            "the section is open, and its clock ticks before the pass says anything"
        );
        let mut in_flight = before.clone();
        in_flight.set_pact_in_flight("/repo/crates/engine", 1, 2);
        assert_eq!(
            app.pact_line(),
            in_flight.pact_line(),
            "and the footer's progress line says what it always said"
        );
        assert_eq!(
            app.message(),
            in_flight.message(),
            "with no message of its own"
        );

        // Then what the pass is doing, four seconds in.
        for event in [
            PactEvent::Doing(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            }),
            PactEvent::Doing(Activity::Thinking),
        ] {
            events.send(event).expect("the loop is still listening");
        }
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 4));

        assert_eq!(
            panel_text(&app, at(base, 4)),
            ["engine", "0:04 Bash cargo test", "0:04 thinking"],
            "one line per activity, under the directory that reported it"
        );

        // A cost is not a thing the pass did, so it draws no line — it is
        // added to what this directory spent.
        events
            .send(PactEvent::Doing(Activity::Cost { usd: 0.21 }))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 6));

        assert_eq!(
            panel_text(&app, at(base, 6)).len(),
            3,
            "the cost added no line: {:?}",
            panel_text(&app, at(base, 6))
        );
        assert_eq!(
            app.panel()
                .account()
                .expect("a run is under way")
                .sections()
                .first()
                .and_then(Section::cost),
            Some(0.21),
            "it was counted instead",
        );

        // The newest line counts up on its own, with nothing arriving: the
        // same app, a later instant, a moving clock. This is what the loop's
        // hundred-millisecond round does for a pass that thinks for a
        // minute.
        assert_eq!(
            panel_text(&app, at(base, 65)),
            ["engine", "0:04 Bash cargo test", "1:05 thinking"],
            "the line beneath the newest one is frozen and the newest is not"
        );
    }

    #[test]
    fn the_request_a_directory_was_handed_draws_a_line_under_it() {
        // What the silence is made of, on the channel and then on screen: the
        // directory is announced, the panel draws the bare placeholder while
        // its files are being read off disk, and the moment the request goes
        // over to the pass a real line lands under that same heading saying
        // what went in it.
        let base = Instant::now();
        let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Pact::with_run(running);

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 1,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);
        assert_eq!(
            panel_text(&app, at(base, 5)),
            ["engine", "0:05 waiting"],
            "the placeholder covers the stretch before the request exists"
        );

        // Twenty seconds of reading files, and then the handover.
        events
            .send(PactEvent::Requesting {
                files: 11,
                bytes: 34 * 1024,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 20));

        // One line, under the heading the `Starting` before it opened, in
        // place of the placeholder that was there a moment ago — and it is the
        // newest line, so it goes on counting while the pass says nothing.
        assert_eq!(
            panel_text(&app, at(base, 20)),
            ["engine", "0:20 waiting · 11 files, 34 KB"],
            "the request the pass was handed is not under its directory"
        );
        assert_eq!(
            panel_text(&app, at(base, 80)),
            ["engine", "1:20 waiting · 11 files, 34 KB"],
            "the line stopped counting the silence it is about"
        );

        // And the footer is untouched by it, as it is by an activity:
        // it still says which directory of how many is being worked, which is
        // the question it answers.
        let mut in_flight = before.clone();
        in_flight.set_pact_in_flight("/repo/crates/engine", 1, 1);
        assert_eq!(
            app.pact_line(),
            in_flight.pact_line(),
            "the footer says exactly what it said before the panel joined in"
        );
    }

    #[test]
    fn a_subtree_pact_reads_as_a_section_per_directory_in_walk_order() {
        // A pass each for two directories: the second heading opens under
        // the first section rather than over it, its clock starts again at
        // nothing, and the account is still whole once the run is over.
        let base = Instant::now();
        let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Pact::with_run(running);

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);
        events
            .send(PactEvent::Doing(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            }))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 4));

        // The next directory. Its clock starts again at nothing, and the
        // section above it stops where the run left it — seventy seconds in,
        // which is where its last line stays however long the run goes on.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/tui"),
                position: 2,
                total: 2,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 70));
        events
            .send(PactEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 71));

        assert_eq!(
            panel_text(&app, at(base, 100)),
            ["engine", "1:10 Bash cargo test", "tui", "0:30 thinking"],
            "the sections are in walk order and each clock counts from its own start"
        );
        let mut in_flight = before.clone();
        in_flight.set_pact_in_flight("/repo/crates/tui", 2, 2);
        assert_eq!(
            app.pact_line(),
            in_flight.pact_line(),
            "and the footer is still the footer"
        );

        // And the outcome behind them lands exactly as it always did.
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 120));

        assert!(!pact.running(), "the run is over");
        assert!(app.pact_line().is_none(), "so nothing is being pacted now");
        // Nothing the run did is on the footer, because nothing went wrong
        // in it. The line that is there belongs to the reload at the bottom
        // of the call, which has no `/repo/crates` on disk to read.
        assert!(
            app.message()
                .is_some_and(|line| line.starts_with(NOT_REFRESHED)),
            "the run reported something of its own: {:?}",
            app.message()
        );
        // The footer stops describing a run that is over; the panel does
        // not. Everything the run said is still there to be read, with each
        // section ended and the run summed up under them. Nothing is on
        // disk at `/repo/crates`, so neither directory has a document to
        // point at and both sections say so rather than claim one.
        assert_eq!(
            panel_text(&app, at(base, 200)),
            [
                "engine",
                "1:10 Bash cargo test",
                "1:10 refused — no document was written",
                "tui",
                "0:50 thinking",
                "0:50 refused — no document was written",
                "pact finished — 2 directories, 2:00, $0.00 \
                     (incomplete: 2 passes reported no cost)",
            ],
            "the account of a finished run stays whole and is closed off"
        );
    }

    #[test]
    fn a_documented_directory_turns_green_the_moment_the_engine_says_so() {
        // The engine's word that a directory and everything under it
        // delivered recolours that subtree there and then, mid-run: a run
        // three directories long shows one green, one flashing and one
        // still yellow, rather than a wall of yellow until the batch is
        // over.
        let stale = NodeState::PactedStale;
        let tree = Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, stale).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, stale)
                    .with_children([Node::new("/repo/crates/engine/src", None::<PathBuf>, stale)]),
                Node::new("/repo/crates/tui", None::<PathBuf>, stale),
            ]),
        );
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: app.clone(),
            unchanged: Vec::new(),
        });

        // The deepest directory's pass delivers, and the run moves on to the
        // one above it without ending.
        for event in [
            PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine/src"),
                position: 1,
                total: 3,
            },
            PactEvent::Documented {
                directory: PathBuf::from("/repo/crates/engine/src"),
            },
            PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 2,
                total: 3,
            },
        ] {
            events.send(event).expect("the loop is still listening");
        }
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);

        assert!(pact.running(), "the run is still going");
        assert_eq!(
            state_of(&app, Path::new("/repo/crates/engine/src")),
            Some(NodeState::PactedFresh),
            "the finished directory is green while the run pays for the rest"
        );
        for still in ["/repo/crates", "/repo/crates/engine", "/repo/crates/tui"] {
            assert_eq!(
                state_of(&app, Path::new(still)),
                Some(stale),
                "{still} has not delivered, so it keeps the keypress's yellow"
            );
        }
    }

    fn one_directory_with_files(state: NodeState) -> Tree {
        Tree::new(
            Node::new("/repo/crates", None::<PathBuf>, state).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, state)
                    .with_files(
                        [
                            "/repo/crates/engine/Cargo.toml",
                            "/repo/crates/engine/build.rs",
                        ]
                        .map(PathBuf::from),
                    )
                    .with_children([Node::new("/repo/crates/engine/src", None::<PathBuf>, state)]),
                Node::new("/repo/crates/tui", None::<PathBuf>, state),
            ]),
        )
    }

    fn running_over(app: &App, work: Work) -> (Sender<PactEvent>, Pact<ClaudeAgent>) {
        let (events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel: CancelGuard::new(),
            work,
            before: app.clone(),
            unchanged: Vec::new(),
        };
        (events, Pact::with_run(running))
    }

    fn drawn(app: &App) -> Vec<(PathBuf, NodeState)> {
        app.rows()
            .iter()
            .map(|row| (row.path.clone(), row.state))
            .collect()
    }

    #[test]
    fn the_document_a_pass_wrote_appears_under_its_directory_as_the_run_goes() {
        // The engine's word that a directory delivered is also the news that
        // there is a `WARLOCK.md` beside it now. The row for it goes in there
        // and then, in the colour the same event just gave the directory,
        // where a fresh load would have put it.
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        // The tally the recolouring moves between states but nothing moves the
        // size of: a document row stands for no node and is counted nowhere.
        let nodes = app.counts().total();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));
        let on_screen: Vec<_> = app.rows().iter().map(|row| row.path.clone()).collect();

        events
            .send(PactEvent::Documented {
                directory: PathBuf::from("/repo/crates/engine"),
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(pact.running(), "the run is still going");
        // Files are hidden — this is the view warlock opens on — and the
        // document is drawn all the same, because the default view keeps each
        // directory's own `WARLOCK.md`. The one row is the whole of the change:
        // the files `engine/` merely holds are still hidden.
        assert_eq!(
            drawn(&app),
            [
                ("/repo/crates", stale),
                ("/repo/crates/engine", NodeState::PactedFresh),
                ("/repo/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/engine/src", NodeState::PactedFresh),
                ("/repo/crates/tui", stale),
            ]
            .map(|(path, state)| (PathBuf::from(path), state)),
            "the document the pass wrote, under its directory and in its colour"
        );
        assert_eq!(
            app.rows().len(),
            on_screen.len() + 1,
            "one row is news and nothing else moved"
        );
        assert_eq!(
            app.counts().total(),
            nodes,
            "a file row is counted nowhere, so the tally cannot have grown"
        );

        app.toggle_files();
        assert_eq!(
            drawn(&app),
            [
                ("/repo/crates", stale),
                ("/repo/crates/engine", NodeState::PactedFresh),
                ("/repo/crates/engine/Cargo.toml", NodeState::PactedFresh),
                ("/repo/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/engine/build.rs", NodeState::PactedFresh),
                ("/repo/crates/engine/src", NodeState::PactedFresh),
                ("/repo/crates/tui", stale),
            ]
            .map(|(path, state)| (PathBuf::from(path), state)),
            "the document is among its directory's files, in path order and \
             before the subdirectory, in the green the run just painted"
        );
    }

    #[test]
    fn a_second_document_for_the_same_directory_inserts_nothing() {
        // A re-pact of an already documented directory says `Documented`
        // about a `WARLOCK.md` that is already a row. One row per path, so
        // the second announcement changes nothing at all.
        let mut app = App::from_tree(&one_directory_with_files(NodeState::PactedStale));
        app.toggle_files();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        for _ in 0..2 {
            events
                .send(PactEvent::Documented {
                    directory: PathBuf::from("/repo/crates/engine"),
                })
                .expect("the loop is still listening");
        }
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(pact.running(), "the run is still going");
        assert_eq!(
            app.rows()
                .iter()
                .filter(|row| row.path == Path::new("/repo/crates/engine/WARLOCK.md"))
                .count(),
            1,
            "the document has exactly one row however often it is announced"
        );
    }

    #[test]
    fn a_documented_directory_with_no_row_changes_nothing() {
        // The event names a directory the tree on screen knows nothing about
        // — a run over a subtree the reader has since loaded away from. There
        // is nothing to hang a document row on, and nothing happens.
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        app.toggle_files();
        let before = drawn(&app);
        let counts = app.counts();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        events
            .send(PactEvent::Documented {
                directory: PathBuf::from("/repo/docs/adr"),
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(pact.running(), "the run is still going");
        assert_eq!(
            drawn(&app),
            before,
            "no row stands for that directory, so none is added under it"
        );
        assert_eq!(app.counts(), counts, "and the tally is where it was");
    }

    #[test]
    fn the_document_rows_a_run_writes_cost_no_reload() {
        // Every directory of the run delivers, one event at a time, and the
        // rows for their documents pile up in a tree that is nowhere on disk.
        // A reload from `nowhere()` would fail and say so on the footer, and a
        // reload that somehow worked would have thrown these rows away — so a
        // tree still holding them, with nothing on the message line and the
        // run still in flight, is the whole of "nothing was re-read".
        let stale = NodeState::PactedStale;
        let mut app = App::from_tree(&one_directory_with_files(stale));
        app.toggle_files();
        let mut manifest = Manifest::new();
        let (events, mut pact) = running_over(&app, pact_of("/repo/crates"));

        for directory in ["/repo/crates/engine/src", "/repo/crates/engine"] {
            events
                .send(PactEvent::Documented {
                    directory: PathBuf::from(directory),
                })
                .expect("the loop is still listening");
        }
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(pact.running(), "the run is still going");
        assert!(
            app.message().is_none(),
            "no reload was attempted, so no reload failed: {:?}",
            app.message()
        );
        assert_eq!(
            drawn(&app),
            [
                ("/repo/crates", stale),
                ("/repo/crates/engine", NodeState::PactedFresh),
                ("/repo/crates/engine/Cargo.toml", NodeState::PactedFresh),
                ("/repo/crates/engine/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/engine/build.rs", NodeState::PactedFresh),
                ("/repo/crates/engine/src", NodeState::PactedFresh),
                ("/repo/crates/engine/src/WARLOCK.md", NodeState::PactedFresh),
                ("/repo/crates/tui", stale),
            ]
            .map(|(path, state)| (PathBuf::from(path), state)),
            "both documents are on screen, each under its own directory"
        );
    }

    #[test]
    fn the_reader_keeps_the_tree_while_the_panel_fills_up() {
        // The run writes to the panel and to nothing else. The reader moves,
        // collapses and filters throughout, and the selection stays where
        // they left it rather than chasing the directory being pacted.
        // Everything is pacted, so the pacted-only filter keeps every row
        // and what it does to the selection is nothing at all.
        let pacted = NodeState::PactedFresh;
        let tree = Tree::new(Node::new("/repo", None::<PathBuf>, pacted).with_children([
            Node::new("/repo/crates", None::<PathBuf>, pacted).with_children([
                Node::new("/repo/crates/engine", None::<PathBuf>, pacted),
                Node::new("/repo/crates/tui", None::<PathBuf>, pacted),
            ]),
            Node::new("/repo/docs", None::<PathBuf>, pacted).with_children([Node::new(
                "/repo/docs/adr",
                None::<PathBuf>,
                pacted,
            )]),
        ]));
        let mut app = App::from_tree(&tree);
        let base = Instant::now();
        app.set_viewport_height(10);
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: app.clone(),
            unchanged: Vec::new(),
        });

        // The reader parks on `docs`, which is nowhere near the subtree
        // being pacted.
        app.select_last();
        app.select_previous();
        let parked = app
            .selected_row()
            .map(|row| row.path.clone())
            .expect("a row is selected");
        assert_eq!(parked, PathBuf::from("/repo/docs"));

        // The first directory, and then the three keys that shape the tree.
        let mut round = |now: Instant, directory: &str, position: usize, app: &mut App| {
            events
                .send(PactEvent::Starting {
                    directory: PathBuf::from(directory),
                    position,
                    total: 2,
                })
                .expect("the loop is still listening");
            events
                .send(PactEvent::Doing(Activity::Thinking))
                .expect("the loop is still listening");
            pact.keep_up(app, &mut manifest, &nowhere(), now);
        };

        round(base, "/repo/crates/engine", 1, &mut app);
        app.toggle_collapsed();
        app.toggle_pacted_only();
        app.toggle_files();

        // The second, and then the movement keys, which end where they
        // started because the reader put them back.
        round(at(base, 10), "/repo/crates/tui", 2, &mut app);
        app.select_previous();
        app.select_next();

        assert!(pact.running(), "the run is still going");
        assert_eq!(
            panel_text(&app, at(base, 10)),
            ["engine", "0:10 thinking", "tui", "0:00 thinking"],
            "the run filled the panel and nothing else"
        );
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(parked),
            "the selection followed the run"
        );
        assert!(app.pacted_only(), "the filter key did nothing");
        assert!(app.show_files(), "the file key did nothing");
        assert!(
            app.is_collapsed("/repo/docs"),
            "the collapse key did nothing"
        );
    }

    #[test]
    fn a_run_that_dies_leaves_the_account_of_what_it_managed_on_screen() {
        // Putting the tree back where it was is the undo for a run that
        // recorded nothing, and it is taken from a copy older than the run
        // itself. The account is not part of that undo: the lines are the
        // record of a pass that really did happen, and this is the run whose
        // reader most wants to see where it got to.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let base = Instant::now();
        app.start_account(base);
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
            unchanged: Vec::new(),
        });

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 2,
            })
            .expect("the loop is still listening");
        events
            .send(PactEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);

        // The worker goes away without an outcome behind it.
        drop(events);
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 5));

        assert!(!pact.running(), "the run is over, however it ended");
        assert_eq!(app.message(), Some(PACT_LOST), "and the footer says so");
        assert_eq!(app.rows(), before.rows(), "the rows match the manifest");
        assert_eq!(
            panel_text(&app, at(base, 5)),
            [
                "engine",
                "0:05 thinking",
                "0:05 refused — no document was written",
                "pact finished — 1 directory, 0:05, $0.00 \
                     (incomplete: 1 pass reported no cost)",
            ],
            "and the panel still holds what the run said before it died"
        );
        assert_eq!(
            panel_text(&app, at(base, 500)),
            panel_text(&app, at(base, 5)),
            "with its clocks stopped where the run stopped"
        );
    }

    const FRAME: u64 = 10;

    fn recorded(
        scratch: &Scratch,
        relative: &str,
        cancel: &Cancel,
        agent: impl FnOnce(&Sender<PactEvent>) -> Canned,
    ) -> Vec<PactEvent> {
        recorded_from(
            scratch,
            &Manifest::new(),
            &toggle(scratch, relative, true),
            cancel,
            agent,
        )
    }

    fn recorded_from(
        scratch: &Scratch,
        manifest: &Manifest,
        work: &Work,
        cancel: &Cancel,
        agent: impl FnOnce(&Sender<PactEvent>) -> Canned,
    ) -> Vec<PactEvent> {
        let (events, received) = mpsc::channel();
        let agent = agent(&events);
        run_pact(manifest, &scratch.root, work, &agent, cancel, &events);
        // Both ends the worker would have held: its own, and the one inside
        // the port attached to its agent.
        drop(events);
        drop(agent);
        received.into_iter().collect()
    }

    fn replay(
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        cancel: CancelGuard,
        said: Vec<PactEvent>,
        base: Instant,
    ) {
        replay_work(
            app,
            manifest,
            scope,
            cancel,
            &pact_of(scope.root.clone()),
            said,
            base,
        );
    }

    fn replay_work(
        app: &mut App,
        manifest: &mut Manifest,
        scope: &Scope,
        cancel: CancelGuard,
        work: &Work,
        said: Vec<PactEvent>,
        base: Instant,
    ) -> Vec<String> {
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel,
            work: work.clone(),
            before: app.clone(),
            unchanged: Vec::new(),
        });
        let mut progress: Vec<String> = Vec::new();
        for (frame, event) in said.into_iter().enumerate() {
            let frame = u64::try_from(frame).expect("a run of fewer than 2^64 events");
            events.send(event).expect("the loop is still listening");
            pact.keep_up(app, manifest, scope, at(base, frame * FRAME));
            if let Some(line) = app.pact_line()
                && progress.last() != Some(&line)
            {
                progress.push(line);
            }
        }
        assert!(!pact.running(), "the run reported its outcome and is over");
        progress
    }

    fn document_bytes(scratch: &Scratch, relative: &str) -> u64 {
        fs::metadata(scratch.path(relative).join(DOCUMENT_FILE))
            .expect("the pass wrote a document")
            .len()
    }

    #[test]
    fn each_section_ends_with_the_document_that_pass_wrote_and_what_it_cost() {
        // A whole run, from the worker's body to the panel: two directories,
        // each with a pass that reads something, thinks, and says what it
        // spent. What closes each section is a fact about disk — the
        // document at `<directory>/WARLOCK.md` and its size — and a fact
        // about the pass, which is the cost the activity port carried.
        let scratch = one_crate_to_load("outcomes");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, []).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        // Read long after the run ended, which is also how it is shown that
        // every clock stopped: a finished account says the same thing for
        // ever.
        assert_eq!(
            panel_text(&app, at(base, 10_000)),
            [
                "crates/engine/src".to_owned(),
                // The request that pass was handed: the one file in
                // `crates/engine/src`, seventeen bytes of it.
                "0:20 waiting · 1 file, 17 bytes".to_owned(),
                "0:30 Read crates/engine/src".to_owned(),
                "1:00 thinking".to_owned(),
                format!(
                    "1:00 wrote crates/engine/src/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "crates/engine".to_owned(),
                // The parent holds no file of its own, and what it carries is
                // its child's document — which is why the count and the weight
                // do not describe the same set.
                format!(
                    "0:20 waiting · 0 files, {} bytes",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "0:30 Read crates/engine".to_owned(),
                "1:00 thinking".to_owned(),
                format!(
                    "1:00 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine")
                ),
                // Thirteen frames of ten seconds: the run started with the
                // first and ended with the thirteenth. The total is the two
                // passes added up, and there is no `incomplete` on it because
                // both of them said what they cost.
                "pact finished — 2 directories, 2:00, $0.50".to_owned(),
            ],
            "each section is closed with its own document and its own cost"
        );

        // And the account of a run that is over is still all there and still
        // all reachable: three lines of panel, and the reader can walk the
        // eleven the run wrote a screenful at a time.
        app.toggle_focus();
        app.panel_mut().set_height(3);
        app.select_first();
        let mut walked = Vec::new();
        loop {
            walked.extend(app.panel().window(at(base, 10_000)));
            if app.panel().lines_below() == 0 {
                break;
            }
            app.select_page_down();
        }
        // Eleven lines do not divide into pages of three, so the last page
        // overlaps the one before it: a panel scrolled to the bottom shows the
        // last three lines whatever it showed a moment ago. A line walked past
        // twice is not a line missed, which is what this is about, so the
        // repeats come out before the comparison.
        walked.dedup();
        assert_eq!(
            walked,
            app.panel()
                .account()
                .expect("the run left one")
                .lines(at(base, 10_000)),
            "every line of the run is still reachable through a short panel"
        );
    }

    #[test]
    fn a_directory_the_run_refused_ends_with_the_reason_it_was_refused() {
        // One directory of the two comes back with an answer the engine will
        // not take. The section for that directory says why, in the words of
        // the failure that named it, and the directory that did write a
        // document is untouched by its neighbour going wrong.
        let scratch = one_crate_to_load("refused-in-the-panel");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, ["crates/engine/src"]).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        let lines = panel_text(&app, at(base, 10_000));
        // The refused directory's section: its first request, the pass, the
        // engine turning the answer down, the second request with the
        // defects listed, that pass, the second rejection, and the refusal.
        let [
            first,
            _,
            _,
            _,
            rejected_once,
            _,
            _,
            _,
            rejected_twice,
            refused,
            second,
            _,
            _,
            _,
            wrote,
            summary,
        ] = lines.as_slice()
        else {
            panic!("a two-directory run reads as two sections and a summary: {lines:?}");
        };
        assert_eq!(first, "crates/engine/src");
        assert_eq!(second, "crates/engine");
        assert!(
            rejected_once
                .starts_with("1:00 rejected · attempt 1/2: the answer is not a JSON object"),
            "the first answer is turned down in the engine's words: {rejected_once}"
        );
        assert!(
            rejected_twice.starts_with("1:50 rejected · attempt 2/2: "),
            "and so is the second: {rejected_twice}"
        );

        let reason = refused
            .strip_prefix("1:50 refused — ")
            .unwrap_or_else(|| panic!("the section says why it was refused: {refused}"));
        assert!(
            reason.contains("crates/engine/src"),
            "and the reason is about that directory: {reason}"
        );
        // The engine's own sentence rather than a second opinion worded
        // here: the footer quotes the same failure, and this is the way to
        // say that the two came out of one place.
        let message = app.message().expect("a partial run reports it");
        assert!(
            message.contains(reason),
            "the panel's reason is the failure's own: {message} / {reason}"
        );

        // Nothing was refused about the parent, which was documented over
        // its failing child, and its section says so in the ordinary way.
        assert_eq!(
            wrote,
            &format!(
                "0:50 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                document_bytes(&scratch, "crates/engine")
            )
        );
        assert!(
            !scratch
                .path("crates/engine/src")
                .join(DOCUMENT_FILE)
                .exists(),
            "the refused directory really has no document"
        );
        // Three passes in all: the refused directory's two and the parent's
        // one, each reporting what it cost.
        assert_eq!(summary, "pact finished — 2 directories, 2:40, $0.75");
    }

    #[test]
    fn a_cancelled_run_says_so_in_the_section_it_was_stopped_in() {
        // Esc during the fourth pass of five. The section being worked when
        // it landed says the run was stopped and what it had spent by then —
        // whatever that pass got as far as answering, because a real cancel
        // kills the `claude` it was waiting on — and the three sections
        // above it keep the endings they earned.
        let scratch = two_crates("cancelled-in-the-panel");
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        // The event loop's own handle, and the clone the run answers to:
        // exactly the pair `spawn_pact` makes, so what the panel reads at
        // the end is the flag the reader's key latched.
        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let said = recorded(&scratch, "crates", &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/alpha", cancel.clone())
        });
        assert!(cancel.is_cancelled(), "the reader stopped this run");
        replay(&mut app, &mut manifest, &scope, guard, said, base);

        let lines = panel_text(&app, at(base, 10_000));
        assert_eq!(
            lines.len(),
            21,
            "four sections of five lines and a summary: {lines:?}"
        );
        assert_eq!(
            &lines[15..],
            [
                "crates/alpha".to_owned(),
                // What the pass that was stopped had been handed: no file of
                // its own and its child's document.
                format!(
                    "0:20 waiting · 0 files, {} bytes",
                    document_bytes(&scratch, "crates/alpha/src")
                ),
                "0:30 Read crates/alpha".to_owned(),
                "1:00 thinking".to_owned(),
                "1:00 cancelled — $0.25 spent".to_owned(),
                // Four directories and not the five the subtree holds: the
                // descent stopped, and the account counts what it reached.
                "pact finished — 4 directories, 4:00, $1.00".to_owned(),
            ],
            "the cancel is recorded in the section it happened in"
        );
        // Everything above it is exactly what a finished pass leaves.
        for (index, directory) in ["crates/beta/src", "crates/beta", "crates/alpha/src"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(&lines[index * 5], directory);
            assert_eq!(
                lines[index * 5 + 4],
                format!(
                    "1:00 wrote {directory}/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, directory)
                ),
                "a section above the cancel keeps the ending it earned"
            );
        }
        // And the footer says what it has always said about a stopped run.
        assert_eq!(app.message(), Some(PACT_CANCELLED));
    }

    const WHOLE_PANEL: u16 = 40;

    fn document_lines() -> Vec<String> {
        (0..4).map(|line| format!("line {line}")).collect()
    }

    fn reading_a_file(app: &mut App) {
        app.panel_mut().set_height(WHOLE_PANEL);
        app.show_document(document_lines(), false);
    }

    fn document_survived(app: &mut App, now: Instant) -> Vec<String> {
        assert_eq!(
            shown(app, now),
            document_lines(),
            "the run took the panel from the reader"
        );
        assert!(
            app.panel().has_document(),
            "the run threw the document away"
        );
        assert!(
            app.panel().has_account(),
            "the run left no account behind it"
        );

        // Past the conversation, which is always a stop of its own: it is where
        // the field is, and a reader is never more than a press from it.
        app.swap_card();
        assert!(
            app.panel().showing_thread(),
            "the cycle skipped the conversation"
        );
        app.swap_card();
        let account = shown(app, now);
        assert_eq!(
            account,
            panel_text(app, now),
            "the account behind the document is not what the run wrote"
        );

        app.swap_card();
        assert_eq!(
            shown(app, now),
            document_lines(),
            "the document did not come back whole"
        );
        account
    }

    #[test]
    fn a_run_that_finishes_under_a_document_leaves_it_showing_and_fills_the_card_behind() {
        // A pact is in flight and the reader opens a file to read while it
        // works. Everything the run says lands on the account's card, one line
        // at a time, and the slot goes on showing what they are reading — line
        // by line through the run, and after the outcome closes it off.
        let base = Instant::now();
        let (mut app, _before, mut manifest, events, running) = a_run_in_flight(base);
        let mut pact = Pact::with_run(running);
        reading_a_file(&mut app);

        // A directory opening a section, and then a line under it.
        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates/engine"),
                position: 1,
                total: 1,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), base);
        assert_eq!(
            shown(&app, at(base, 2)),
            document_lines(),
            "the section that opened took the panel"
        );

        events
            .send(PactEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 4));
        assert!(pact.running(), "a run that is talking is still running");
        assert_eq!(
            shown(&app, at(base, 4)),
            document_lines(),
            "a line appended behind the document took the panel"
        );

        // And the outcome, which is where the run is closed off and summed up.
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 6));
        assert!(!pact.running(), "the run is over");

        let account = document_survived(&mut app, at(base, 6));
        assert_eq!(
            account,
            [
                "engine",
                // The line the run left ticking, stopped where the section was
                // closed rather than where it was appended.
                "0:06 thinking",
                "0:06 refused — no document was written",
                "pact finished — 1 directory, 0:06, $0.00 \
                     (incomplete: 1 pass reported no cost)",
            ],
            "the whole run is there to be read behind the document"
        );
    }

    #[test]
    fn a_cancelled_run_under_a_document_leaves_it_showing_too() {
        // Esc during a run the reader started with a file already up. The
        // account says where it got to and what stopped it; the slot says what
        // they were reading, from before the pact began to after it was
        // stopped.
        let scratch = one_crate_to_load("cancelled-under-a-document");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        reading_a_file(&mut app);
        app.start_account(base);
        assert_eq!(
            shown(&app, base),
            document_lines(),
            "the run starting took the panel"
        );

        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let said = recorded(&scratch, "crates/engine", &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/engine/src", cancel.clone())
        });
        replay(&mut app, &mut manifest, &scope, guard, said, base);

        assert_eq!(app.message(), Some(PACT_CANCELLED), "the run was stopped");
        let account = document_survived(&mut app, at(base, 10_000));
        assert!(
            account.iter().any(|line| line.contains("cancelled")),
            "the account of the stopped run says so: {account:?}"
        );
    }

    #[test]
    fn a_run_a_pass_failed_in_under_a_document_leaves_it_showing_too() {
        // The engine turns down the only answer it got. The failure is on the
        // footer and in the account, which is where a run's failures have always
        // gone — and not in the slot, which is the reader's.
        let scratch = one_crate_to_load("refused-under-a-document");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        reading_a_file(&mut app);
        app.start_account(base);
        assert_eq!(
            shown(&app, base),
            document_lines(),
            "the run starting took the panel"
        );

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, ["crates/engine/src"]).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        let message = app.message().expect("a partial run reports it");
        assert!(
            message.contains("crates/engine/src"),
            "the failing directory is named: {message}"
        );
        let account = document_survived(&mut app, at(base, 10_000));
        assert!(
            account.iter().any(|line| line.contains("refused")),
            "the account of the failed run says why: {account:?}"
        );
    }

    #[test]
    fn a_run_whose_end_puts_the_view_back_leaves_the_document_showing() {
        // The two endings that go through `App::restore_from`: an outcome that
        // recorded nothing, and a worker that died without one. Both roll the
        // rows back to what the manifest on disk still says, and neither may
        // roll back the panel — the account of what the run managed is the thing
        // this reader most wants, and the card they are reading is still theirs.
        for failure in [Some("the manifest could not be saved"), None] {
            let base = Instant::now();
            let (mut app, before, mut manifest, events, running) = a_run_in_flight(base);
            let mut pact = Pact::with_run(running);
            reading_a_file(&mut app);

            events
                .send(PactEvent::Starting {
                    directory: PathBuf::from("/repo/crates/engine"),
                    position: 1,
                    total: 2,
                })
                .expect("the loop is still listening");
            events
                .send(PactEvent::Doing(Activity::Thinking))
                .expect("the loop is still listening");
            pact.keep_up(&mut app, &mut manifest, &nowhere(), base);
            assert_eq!(shown(&app, base), document_lines(), "{failure:?}");

            match failure {
                Some(reason) => events
                    .send(PactEvent::Finished(Err(reason.to_owned())))
                    .expect("the loop is still listening"),
                // The worker goes away with nothing behind it.
                None => drop(events),
            }
            pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 5));

            assert!(!pact.running(), "the run is over, however it ended");
            assert_eq!(app.message(), Some(failure.unwrap_or(PACT_LOST)));
            assert_eq!(app.rows(), before.rows(), "the rows match the manifest");

            let account = document_survived(&mut app, at(base, 5));
            assert_eq!(
                account,
                [
                    "engine",
                    "0:05 thinking",
                    "0:05 refused — no document was written",
                    "pact finished — 1 directory, 0:05, $0.00 \
                         (incomplete: 1 pass reported no cost)",
                ],
                "the restore took the account of the run with the rows: {failure:?}"
            );
        }
    }

    const QUESTION: &str = "what does the engine do?";
    const ANSWER: &str = "It walks the tree and writes what it finds.";

    fn a_conversation(app: &mut App, base: Instant) {
        app.panel_mut().set_height(WHOLE_PANEL);
        app.panel_mut().start_turn(QUESTION, base);
        app.panel_mut().record_turn(
            &Activity::Tool {
                name: "Grep".to_owned(),
                detail: Some("engine".to_owned()),
            },
            base,
        );
        app.panel_mut().answer_turn(ANSWER, base);
        assert!(
            app.panel().showing_thread(),
            "a question brings the conversation to the front by itself"
        );
    }

    fn conversation_untouched(app: &mut App, now: Instant) -> Vec<String> {
        assert!(
            app.panel().showing_thread(),
            "the run took the panel from the reader"
        );
        let thread = app
            .panel()
            .thread()
            .expect("a question was asked before the run");
        assert_eq!(
            thread.turns().len(),
            1,
            "the run took a turn of the conversation"
        );
        assert_eq!(thread.turns()[0].message(), QUESTION);
        assert_eq!(
            thread.turns()[0].answer(),
            Some(ANSWER),
            "the run overwrote the turn above it"
        );
        assert!(
            thread.in_flight().is_none(),
            "a run nobody typed muted the field"
        );

        // On screen: the question, the work under it and the answer, with not
        // one word of the run's vocabulary among them.
        let shown_now = shown(app, now);
        for said in ["wrote", "refused", "cancelled", "pact finished"] {
            assert!(
                !shown_now.iter().any(|line| line.contains(said)),
                "{said:?} is in the conversation: {shown_now:?}"
            );
        }

        // And one swap away is the run's own card, which is where the whole of
        // the run is.
        assert!(
            app.panel().has_account(),
            "the run left no account behind it"
        );
        app.swap_card();
        let card = shown(app, now);
        assert_eq!(
            card,
            panel_text(app, now),
            "the account card is not what the run wrote"
        );

        app.swap_card();
        assert!(
            app.panel().showing_thread(),
            "the conversation did not come back whole"
        );
        assert_eq!(shown(app, now), shown_now);
        card
    }

    #[test]
    fn a_run_started_while_the_thread_shows_fills_its_own_card_and_only_that() {
        // A whole pact, driven a frame at a time through the loop that hears the
        // worker, with a conversation already on screen. After every frame the
        // run's own card has more on it and the conversation has exactly what it
        // had — the run reports into the card built for it, while the reader
        // goes on reading what they asked for.
        let scratch = one_crate_to_load("run-in-a-conversation");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        a_conversation(&mut app, base);
        let asked = shown(&app, base);
        // Everything the press does before the worker starts: one account, and
        // nothing at all to the conversation.
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, []).reporting(activity_port(events))
        });
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of(scope.root.clone()),
            before: app.clone(),
            unchanged: Vec::new(),
        });
        let mut lengths = Vec::new();
        for (frame, event) in said.into_iter().enumerate() {
            let frame = u64::try_from(frame).expect("a run of fewer than 2^64 events");
            let now = at(base, frame * FRAME);
            events.send(event).expect("the loop is still listening");
            pact.keep_up(&mut app, &mut manifest, &scope, now);

            assert!(
                app.panel().showing_thread(),
                "the run swapped the card away"
            );
            let card = app
                .panel()
                .account()
                .expect("the press started the run's own card")
                .lines(now);
            assert_eq!(
                shown(&app, now),
                asked,
                "the run wrote into the conversation at frame {frame}"
            );
            lengths.push(card.len());
        }
        assert!(!pact.running(), "the run reported its outcome and is over");
        // The equality above is about a card that was moving: something arrived
        // over the run, and it ended longer than it started.
        assert!(
            lengths.first() < lengths.last(),
            "nothing arrived over the whole run: {lengths:?}"
        );

        // The conversation is the conversation, and the whole of the run is on
        // the card one swap away.
        let card = conversation_untouched(&mut app, at(base, 10_000));
        assert_eq!(
            shown(&app, at(base, 10_000)),
            asked,
            "the run rewrote the turn somebody typed"
        );
        assert_eq!(
            card,
            [
                "crates/engine/src".to_owned(),
                "0:20 waiting · 1 file, 17 bytes".to_owned(),
                "0:30 Read crates/engine/src".to_owned(),
                "1:00 thinking".to_owned(),
                format!(
                    "1:00 wrote crates/engine/src/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "crates/engine".to_owned(),
                format!(
                    "0:20 waiting · 0 files, {} bytes",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "0:30 Read crates/engine".to_owned(),
                "1:00 thinking".to_owned(),
                format!(
                    "1:00 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine")
                ),
                "pact finished — 2 directories, 2:00, $0.50".to_owned(),
            ],
            "the conversation is not holding the whole run"
        );
    }

    #[test]
    fn a_cancelled_run_says_so_on_its_own_card_and_leaves_the_conversation() {
        // Esc during a run somebody started mid-conversation. The account says
        // where the run got to and what stopped it; the conversation is
        // untouched, and the reader is where they were.
        let scratch = one_crate_to_load("cancelled-in-a-conversation");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        a_conversation(&mut app, base);
        app.start_account(base);

        let guard = CancelGuard::new();
        let cancel = guard.handle();
        let said = recorded(&scratch, "crates/engine", &cancel, |events| {
            Canned::new(&scratch, [])
                .reporting(activity_port(events))
                .cancelling_at("crates/engine/src", cancel.clone())
        });
        replay(&mut app, &mut manifest, &scope, guard, said, base);

        assert_eq!(app.message(), Some(PACT_CANCELLED), "the run was stopped");
        let card = conversation_untouched(&mut app, at(base, 10_000));
        assert!(
            card.iter().any(|line| line.contains("cancelled")),
            "the run's card does not say the reader stopped it: {card:?}"
        );
    }

    #[test]
    fn a_run_a_pass_failed_in_says_so_on_its_card_and_leaves_the_rest_alone() {
        // The engine turns down the only answer it got. The failure goes where
        // a run's failures have always gone — the footer and the account — and
        // the conversation beside it hears none of it.
        let scratch = one_crate_to_load("refused-in-a-conversation");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        a_conversation(&mut app, base);
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            Canned::new(&scratch, ["crates/engine/src"]).reporting(activity_port(events))
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        let message = app.message().expect("a partial run reports it");
        assert!(
            message.contains("crates/engine/src"),
            "the failing directory is named: {message}"
        );
        let card = conversation_untouched(&mut app, at(base, 10_000));
        assert!(
            card.iter().any(|line| line.contains("refused")),
            "the run's card does not say why it failed: {card:?}"
        );
    }

    #[test]
    fn a_run_in_flight_leaves_the_register_the_field_and_the_conversation_alone() {
        // A brief is exactly the conversation somebody is most likely to press a
        // run key during: they are converging on a document about a repository,
        // so they pact or refresh part of it to see what warlock makes of it
        // first. The run is a worker of its own with a card of its own and it
        // shares nothing with the register — so a whole run, driven a frame at a
        // time through the loop that hears the worker, writes not one row into
        // the thread, mutes nothing, and finds the conversation in brief mode at
        // every frame and after the last one.
        //
        // Both keys, because they are two presses of one machine and the mode
        // has to survive either. Nothing here has a `claude`: the run is a
        // recording of a canned agent played back over a channel, and the
        // conversation is asked nothing at all.
        for refreshing_it in [false, true] {
            let scratch = one_crate_to_load(if refreshing_it {
                "refresh-during-a-brief"
            } else {
                "pact-during-a-brief"
            });
            // A refresh is only a question about a subtree pacted once, so it
            // starts from the manifest that pact earned and a file changed
            // under it; a pact starts from nothing, as a first pact does.
            let started = if refreshing_it {
                let earned = pacted(&scratch, "crates/engine");
                scratch.write("crates/engine/src/lib.rs", "//! Core engine, rewritten.\n");
                earned
            } else {
                Manifest::new()
            };
            let work = if refreshing_it {
                refreshing(&scratch, "crates/engine")
            } else {
                toggle(&scratch, "crates/engine", true)
            };

            let (mut app, scope) = load(&scratch);
            let mut manifest = started.clone();
            let base = Instant::now();
            a_conversation(&mut app, base);
            assert!(
                app.panel_mut().set_mode(Mode::Brief),
                "the conversation was in the register before the test put it there"
            );
            let asked = shown(&app, base);
            // The field the run must not reach, held where the loop holds it:
            // the muting is derived from this and from nothing else, and the
            // run is never handed it.
            let chat = Chat::new("/warlock/no/such/repository");
            // Everything the press does before the worker starts: one account,
            // and nothing at all to the conversation.
            app.start_account(base);

            let said = recorded_from(&scratch, &started, &work, &Cancel::new(), |events| {
                Canned::new(&scratch, []).reporting(activity_port(events))
            });
            assert!(
                said.len() > 1,
                "the run had nothing to say, so nothing was driven: {refreshing_it}"
            );
            let (events, received) = mpsc::channel();
            let mut pact = Pact::with_run(Running {
                events: received,
                cancel: CancelGuard::new(),
                work,
                before: app.clone(),
                unchanged: Vec::new(),
            });
            for (frame, event) in said.into_iter().enumerate() {
                let frame = u64::try_from(frame).expect("a run of fewer than 2^64 events");
                let now = at(base, frame * FRAME);
                events.send(event).expect("the loop is still listening");
                pact.keep_up(&mut app, &mut manifest, &scope, now);

                assert_eq!(
                    app.panel().mode(),
                    Mode::Brief,
                    "the run left the register at frame {frame}"
                );
                assert_eq!(
                    shown(&app, now),
                    asked,
                    "the run wrote into the conversation at frame {frame}"
                );
                assert!(
                    !chat.composer().is_muted(),
                    "the run took the field at frame {frame}"
                );
            }

            assert!(!pact.running(), "the run reported its outcome and is over");
            assert_eq!(
                app.panel().mode(),
                Mode::Brief,
                "the register did not survive the run: {refreshing_it}"
            );
            assert!(!chat.composer().is_muted(), "the run left the field muted");
            // And the whole of the run is on the card one swap away, with the
            // conversation — question, work and answer — exactly as it was.
            let card = conversation_untouched(&mut app, at(base, 10_000));
            assert!(
                card.iter().any(|line| line.contains("pact finished")),
                "the run's card does not say it ran: {card:?}"
            );
        }
    }

    fn a_run_in_flight_during_a_conversation(
        base: Instant,
    ) -> (App, App, Manifest, mpsc::Sender<PactEvent>, Running) {
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let mut before = App::from_tree(&tree);
        a_conversation(&mut before, base);
        let mut app = before.clone();
        app.start_account(base);
        let (events, received) = mpsc::channel();
        let running = Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
            unchanged: Vec::new(),
        };
        (app, before, Manifest::new(), events, running)
    }

    #[test]
    fn a_run_whose_end_puts_the_view_back_keeps_the_conversation_beside_it() {
        // The two endings that go through `App::restore_from`: an outcome that
        // recorded nothing, and a worker that died without one. Both roll the
        // rows back to what the manifest on disk still says, and neither may
        // roll back the panel — the conversation, the turn somebody typed in it
        // and the run's own turn all survive, and the loop goes on running.
        for failure in [Some("the manifest could not be saved"), None] {
            let base = Instant::now();
            let (mut app, before, mut manifest, events, running) =
                a_run_in_flight_during_a_conversation(base);
            let mut pact = Pact::with_run(running);

            events
                .send(PactEvent::Starting {
                    directory: PathBuf::from("/repo/crates/engine"),
                    position: 1,
                    total: 2,
                })
                .expect("the loop is still listening");
            events
                .send(PactEvent::Doing(Activity::Thinking))
                .expect("the loop is still listening");
            pact.keep_up(&mut app, &mut manifest, &nowhere(), base);
            assert!(app.panel().showing_thread(), "{failure:?}");

            match failure {
                Some(reason) => events
                    .send(PactEvent::Finished(Err(reason.to_owned())))
                    .expect("the loop is still listening"),
                // The worker goes away with nothing behind it.
                None => drop(events),
            }
            pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 5));

            assert!(!pact.running(), "the run is over, however it ended");
            assert_eq!(app.message(), Some(failure.unwrap_or(PACT_LOST)));
            assert_eq!(app.rows(), before.rows(), "the rows match the manifest");

            let card = conversation_untouched(&mut app, at(base, 5));
            assert_eq!(
                card,
                [
                    "engine",
                    "0:05 thinking",
                    "0:05 refused — no document was written",
                    "pact finished — 1 directory, 0:05, $0.00 \
                         (incomplete: 1 pass reported no cost)",
                ],
                "the restore took the run's account with the rows: {failure:?}"
            );

            // And the frame after the ending is the frame after any other run:
            // nothing drained, nothing reloaded, nothing said. The loop kept
            // running, which is the thing an early return here would cost.
            let quiet = app.clone();
            assert!(
                pact.keep_up(&mut app, &mut manifest, &nowhere(), at(base, 6))
                    .is_none()
            );
            assert_eq!(app, quiet, "a frame with no run in it changed something");
        }
    }

    fn requests_of_a_run(scratch: &Scratch, talked_first: bool) -> Vec<agent::Request> {
        let (mut app, scope) = load(scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        if talked_first {
            a_conversation(&mut app, base);
        }

        let toggle = pact_press(&mut app, false, base).expect("the root row is a directory");
        let agent = Canned::new(scratch, []);
        run_and_apply(
            scratch,
            &mut app,
            &mut manifest,
            &scope,
            &Work::Pact(toggle),
            &agent,
        );
        if talked_first {
            assert!(
                app.panel().showing_thread(),
                "the run was supposed to happen inside the conversation"
            );
        }
        agent.requests()
    }

    fn undo_the_run(scratch: &Scratch) {
        fs::remove_dir_all(scratch.root.join(".warlock")).expect("the run saved a manifest");
        remove_documents(&scratch.root);
        assert!(
            !scratch.path(DOCUMENT_FILE).exists(),
            "a document the run wrote is still there"
        );
    }

    fn remove_documents(directory: &Path) {
        for entry in fs::read_dir(directory).expect("a directory the run walked") {
            let path = entry
                .expect("an entry of a directory the run walked")
                .path();
            if path.is_dir() {
                remove_documents(&path);
            } else if path.file_name().is_some_and(|name| name == DOCUMENT_FILE) {
                fs::remove_file(&path).expect("a document the run wrote");
            }
        }
    }

    #[test]
    fn a_conversation_changes_nothing_about_what_a_run_asks_for() {
        // The whole reason a pact may sit inside a thread: what the engine
        // sends is a function of the directory and nothing else. Two runs over
        // the one repository, the second of them started after a question was
        // asked and answered on the very thread the run then writes its account
        // into, hand the model the same requests — same prompt, same files with
        // the same bytes, same children's documents, in the same order, for the
        // same directories in the same order.
        //
        // A guard against a wiring nobody has written: the engine has no notion
        // of a chat, so the only way this can ever fail is somebody deciding to
        // pass the conversation down. `make` is what a pact has to be — two
        // people with the same repository get the same documents — and green is
        // only a fact for as long as that holds.
        let scratch = one_crate_to_load("a-request-a-chat-cannot-reach");

        let alone = requests_of_a_run(&scratch, false);
        undo_the_run(&scratch);
        let after_a_conversation = requests_of_a_run(&scratch, true);

        assert!(
            !alone.is_empty(),
            "the run described nothing, so nothing was compared"
        );
        assert_eq!(
            alone, after_a_conversation,
            "the conversation reached what the run asks a model for"
        );

        // And said plainly, for a failure that would otherwise be two screens of
        // bytes: neither the question nor the answer is anywhere in what was
        // sent, whether as the prompt itself or smuggled into a file.
        for request in &after_a_conversation {
            for text in [QUESTION, ANSWER] {
                assert!(
                    !request.prompt().contains(text),
                    "the prompt for {} carries the conversation",
                    request.directory().display()
                );
                assert!(
                    !request
                        .files()
                        .iter()
                        .filter_map(agent::File::bytes)
                        .any(|bytes| bytes.windows(text.len()).any(|run| run == text.as_bytes())),
                    "a file sent for {} carries the conversation",
                    request.directory().display()
                );
            }
        }
    }

    fn eight_crates(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        for crate_name in CRATES {
            scratch.write(
                &format!("crates/{crate_name}/src/lib.rs"),
                "//! A crate of its own.\n",
            );
        }
        scratch.write(".git/HEAD", "ref: refs/heads/main\n");
        scratch
    }

    const CRATES: [&str; 8] = ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"];

    #[test]
    fn a_refresh_counts_what_it_will_visit_rather_than_the_subtree_it_was_pointed_at() {
        // Seventeen directories pacted, three files then moved, and a refresh
        // of the lot: seven directories are stale — three that hold a moved
        // file, their three parents, and the root above all of them — and
        // seven is what the reader is counted to.
        let scratch = eight_crates("refresh-fraction");
        let pacted_once = pacted(&scratch, "crates");
        assert_eq!(
            pacted_once.entries().len(),
            17,
            "the subtree the key is pressed on is much bigger than the run"
        );
        for crate_name in ["c1", "c2", "c3"] {
            scratch.write(
                &format!("crates/{crate_name}/src/lib.rs"),
                "//! A crate of its own, and something new.\n",
            );
        }
        // And one in `crates` itself, so the directory above all of them has
        // something new to read rather than being carried forward by the early
        // cutoff — the count this is about is seven passes, not six.
        scratch.write("crates/shared.rs", "//! Shared, and something new.\n");

        let (mut app, scope) = load(&scratch);
        let mut manifest = pacted_once.clone();
        let base = Instant::now();
        // Everything the press does before the worker starts, which is what
        // `refresh_press` does for real: one account, opened at the press.
        app.start_account(base);

        let work = refreshing(&scratch, "crates");
        let said = recorded_from(&scratch, &pacted_once, &work, &Cancel::new(), |_| {
            Canned::new(&scratch, [])
        });
        let progress = replay_work(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            &work,
            said,
            base,
        );

        // The verb is the run's and the fraction is the engine observer's:
        // seven of seven, and not one of seventeen.
        assert_eq!(
            progress,
            [
                "refreshing crates/c3/src (1/7)",
                "refreshing crates/c3 (2/7)",
                "refreshing crates/c2/src (3/7)",
                "refreshing crates/c2 (4/7)",
                "refreshing crates/c1/src (5/7)",
                "refreshing crates/c1 (6/7)",
                "refreshing crates (7/7)",
            ],
            "the footer counted something other than what the run visited"
        );

        // The panel filled the same way, a section per directory the run
        // reached and none for a directory it skipped.
        let lines = panel_text(&app, at(base, 10_000));
        let stale = [
            "crates/c3/src",
            "crates/c3",
            "crates/c2/src",
            "crates/c2",
            "crates/c1/src",
            "crates/c1",
            "crates",
        ];
        let headings: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|line| stale.contains(line))
            .collect();
        assert_eq!(headings, stale, "a section per directory, in walk order");
        assert_eq!(
            lines.last().map(String::as_str),
            Some(
                "pact finished — 7 directories, 3:30, $0.00 (incomplete: 7 passes reported no cost)"
            ),
            "the summary counts the run: {lines:?}"
        );

        // And the run ended the ordinary way: the manifest it computed is on
        // disk, and every directory of the subtree — the described and the
        // skipped alike — is green again.
        assert_eq!(
            saved(&scratch.root).expect("the manifest was written"),
            manifest
        );
        for entry in manifest.entries() {
            let module = entry.module();
            assert_eq!(
                state_of(&app, &scratch.path(module)),
                Some(NodeState::PactedFresh),
                "`{module}` is not green after the refresh"
            );
        }
    }

    #[test]
    fn a_pass_that_never_said_what_it_cost_leaves_the_total_incomplete() {
        // The first pass's result line carries no cost — WAR-24 reports none
        // rather than a zero — and the port drops it on the floor exactly as
        // a result line without the field would. What the run spent is then
        // one pass's worth and the total says so, rather than adding a zero
        // that was never measured and under-reporting the run.
        let scratch = one_crate_to_load("no-cost");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let base = Instant::now();
        app.start_account(base);

        let said = recorded(&scratch, "crates/engine", &Cancel::new(), |events| {
            let events = events.clone();
            let swallowed = AtomicBool::new(false);
            let port = Activities::new(move |activity| {
                if matches!(activity, Activity::Cost { .. })
                    && !swallowed.swap(true, Ordering::Relaxed)
                {
                    return;
                }
                let _ = events.send(PactEvent::Doing(activity));
            });
            Canned::new(&scratch, []).reporting(port)
        });
        replay(
            &mut app,
            &mut manifest,
            &scope,
            CancelGuard::new(),
            said,
            base,
        );

        assert_eq!(
            panel_text(&app, at(base, 10_000)),
            [
                "crates/engine/src".to_owned(),
                // The one file of `one_crate`, seventeen bytes of it, on the
                // line that says what the silence after it was made of.
                "0:20 waiting · 1 file, 17 bytes".to_owned(),
                "0:30 Read crates/engine/src".to_owned(),
                "0:50 thinking".to_owned(),
                format!(
                    "0:50 wrote crates/engine/src/WARLOCK.md — {} bytes, no cost reported",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "crates/engine".to_owned(),
                format!(
                    "0:20 waiting · 0 files, {} bytes",
                    document_bytes(&scratch, "crates/engine/src")
                ),
                "0:30 Read crates/engine".to_owned(),
                "1:00 thinking".to_owned(),
                format!(
                    "1:00 wrote crates/engine/WARLOCK.md — {} bytes, $0.25",
                    document_bytes(&scratch, "crates/engine")
                ),
                "pact finished — 2 directories, 1:50, \
                     $0.25 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
            ],
            "a pass that reported nothing is missing rather than free"
        );
    }

    #[test]
    fn a_worker_that_says_nothing_more_ends_the_run_rather_than_hanging() {
        // The one way the channel closes without an outcome: the worker
        // reports on every path it takes itself, so a silence is a panic in
        // it. The hook has already put the terminal back and printed what
        // happened; what is left to this thread is to stop describing a run
        // that is over, and above all not to wait for a thread that no
        // longer exists.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
            unchanged: Vec::new(),
        });

        events
            .send(PactEvent::Starting {
                directory: PathBuf::from("/repo/crates"),
                position: 1,
                total: 1,
            })
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(pact.running(), "a run that has only started is still on");
        assert!(
            app.pact_line().is_some(),
            "and the footer says where it has got to"
        );

        // The worker's end goes away with no `Finished` behind it.
        drop(events);
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(!pact.running(), "the run is over, however it ended");
        assert!(
            app.pact_line().is_none(),
            "so nothing is being pacted now: {:?}",
            app.pact_line()
        );
        assert_eq!(app.message(), Some(PACT_LOST), "and the footer says so");
        assert_eq!(
            manifest,
            Manifest::new(),
            "nothing new was recorded, because nothing came back to record"
        );
        // The rows are the ones the keystroke painted over, put back: the
        // same app, down to the message that is the only thing this path
        // adds to it.
        let mut restored = before;
        restored.set_message(PACT_LOST);
        assert_eq!(
            app, restored,
            "and the rows go back to matching the manifest"
        );
    }

    #[test]
    fn a_finished_pact_puts_the_documents_it_wrote_on_screen() {
        // The whole point of the reload, in one test: a run writes
        // `WARLOCK.md` into every directory of the subtree, and the app has
        // no way to know that except by reading the tree again. No key is
        // pressed here and nothing is relaunched — the frame after the
        // outcome shows them.
        let scratch = one_crate_to_load("reload-granted");
        let (mut app, scope) = load(&scratch);
        let mut manifest = Manifest::new();
        let agent = Canned::new(&scratch, []);
        let engine = scratch.path("crates/engine");

        // Files on, so a document that appears is a row that appears.
        app.toggle_files();
        assert_eq!(
            documents(&app, &scratch),
            Vec::<PathBuf>::new(),
            "nothing has been pacted yet, so there is nothing to show"
        );
        // What the pact key paints before the run starts: pacted, and not
        // yet proven fresh.
        app.set_subtree_state(&engine, NodeState::PactedStale);

        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "crates/engine", true),
            &agent,
        );

        assert_eq!(
            documents(&app, &scratch),
            [
                PathBuf::from("crates/engine/WARLOCK.md"),
                PathBuf::from("crates/engine/src/WARLOCK.md"),
            ],
            "the documents the run wrote are rows in the tree"
        );
        // And they are green, because the tree they came back in was built
        // from the manifest the run had already saved.
        for relative in ["crates/engine", "crates/engine/src"] {
            assert_eq!(
                state_of(&app, &scratch.path(relative)),
                Some(NodeState::PactedFresh),
                "{relative} did not come back fresh"
            );
        }
        assert_eq!(app.message(), None, "and nothing went wrong to report");
    }

    #[test]
    fn a_reload_leaves_the_reader_exactly_where_they_were() {
        // A tree that collapses to the root and throws the selection to the
        // top every time a pact ends is worse than one that never updates,
        // so: collapse something, filter, scroll, select — then end a pact
        // that really does change the tree, and none of the five moves.
        let scratch = one_crate_to_load("reload-place");
        scratch.write("crates/engine/tests/one.rs", "#[test] fn one() {}\n");
        scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
        scratch.write("docs/adr/one.md", "# One\n");

        // A pact that happened before this reader sat down, so there is
        // something for the pacted-only filter to keep.
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        app.set_viewport_height(4);
        app.toggle_files();
        app.toggle_pacted_only();
        app = app.with_collapsed([scratch.path("crates/tui")]);
        for _ in 0..5 {
            app.select_next();
        }

        let selected = app
            .selected_row()
            .map(|row| row.path.clone())
            .expect("a row is selected");
        let collapsed = app.collapsed().clone();
        let offset = app.scroll_offset();
        assert!(offset > 0, "the reader scrolled off the first row");

        // A pact over a subtree nothing has touched yet: the reload really
        // does bring back a different tree, with `docs` pacted and two more
        // documents in it.
        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "docs", true),
            &agent,
        );

        assert_eq!(
            state_of(&app, &scratch.path("docs")),
            Some(NodeState::PactedFresh),
            "the tree that came back is the new one"
        );
        assert_eq!(
            app.selected_row().map(|row| row.path.clone()),
            Some(selected),
            "the selection moved"
        );
        assert_eq!(app.collapsed(), &collapsed, "the collapsed set moved");
        assert!(app.pacted_only(), "the filter was dropped");
        assert!(app.show_files(), "the file toggle was dropped");
        assert_eq!(app.scroll_offset(), offset, "the window moved");
    }

    #[test]
    fn an_un_pact_reloads_the_tree_by_the_same_one_rule() {
        // One rule for every ending, so an un-pact re-reads too. Nothing in
        // `apply_progress`'s arms recolours anything for an un-pact — it
        // grants nothing and has nothing to say — so a subtree that comes
        // back unpacted here came back from disk.
        let scratch = one_crate_to_load("reload-unpact");
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedFresh),
            "the reader is looking at a pacted subtree"
        );

        run_and_apply(
            &scratch,
            &mut app,
            &mut manifest,
            &scope,
            &toggle(&scratch, "crates/engine", false),
            &agent,
        );

        assert!(
            manifest.entries().is_empty(),
            "the un-pact emptied the manifest"
        );
        for relative in ["crates/engine", "crates/engine/src"] {
            assert_eq!(
                state_of(&app, &scratch.path(relative)),
                Some(NodeState::Unpacted),
                "{relative} is still coloured by a manifest that no longer says so"
            );
        }
    }

    #[test]
    fn a_reload_that_will_not_load_keeps_the_tree_already_on_screen() {
        // Mid-session, a load that fails is not fatal and never was going to
        // be: warlock is up, the documents are on disk and the manifest is
        // saved, so quitting would throw away a run that cost minutes and
        // money. The arm's own result stands and the rows do not move.
        let tree = Tree::new(Node::new(
            "/repo/crates",
            None::<PathBuf>,
            NodeState::Unpacted,
        ));
        let before = App::from_tree(&tree);
        let mut app = before.clone();
        let mut manifest = Manifest::new();
        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of("/repo/crates"),
            before: before.clone(),
            unchanged: Vec::new(),
        });

        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: Manifest::new(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        // There is no repository at `/repo/crates`, so the reload at the
        // bottom of this call fails.
        pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());

        assert!(!pact.running(), "the run is over");
        assert_eq!(
            app.rows().len(),
            before.rows().len(),
            "the tree on screen was thrown away"
        );
        assert_eq!(
            state_of(&app, Path::new("/repo/crates")),
            Some(NodeState::PactedFresh),
            "the outcome the run did report was undone by a load that failed"
        );
        // And the reader is told why the rows did not move, on the one line
        // the footer has, with the load's own reason after it.
        let message = app.message().expect("a refresh that failed says so");
        assert!(
            message.starts_with(NOT_REFRESHED),
            "the footer says nothing about the refresh: {message}"
        );
        assert!(
            message.len() > NOT_REFRESHED.len() + 2,
            "the footer does not say why: {message}"
        );
        assert!(
            !message.contains('\n'),
            "a footer line that wraps is a footer line that hides a row: {message}"
        );
    }

    #[test]
    fn the_pacts_own_message_wins_over_the_reloads() {
        // Both have something to say and there is one line to say it on. The
        // run is the news — it is what the reader asked for, and it cost
        // minutes — so the reload's line waits for a footer nobody else
        // wanted. Both endings are driven here against the same failing
        // reload, so the only difference between them is whether the pact
        // left a message.
        const REFUSED: &str = "the manifest would not save";

        let ending = |message: Option<String>| {
            let tree = Tree::new(Node::new(
                "/repo/crates",
                None::<PathBuf>,
                NodeState::Unpacted,
            ));
            let before = App::from_tree(&tree);
            let mut app = before.clone();
            let mut manifest = Manifest::new();
            let (events, received) = mpsc::channel();
            let mut pact = Pact::with_run(Running {
                events: received,
                cancel: CancelGuard::new(),
                work: pact_of("/repo/crates"),
                before,
                unchanged: Vec::new(),
            });

            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: Manifest::new(),
                    granted: true,
                    message,
                    refusals: Vec::new(),
                })))
                .expect("the loop is still listening");
            // Nothing is on disk at `/repo/crates`, so the reload has its
            // own line to offer in both cases.
            pact.keep_up(&mut app, &mut manifest, &nowhere(), Instant::now());
            app.message().map(str::to_owned)
        };

        assert_eq!(
            ending(Some(REFUSED.to_owned())).as_deref(),
            Some(REFUSED),
            "the reload talked over the run"
        );
        assert!(
            ending(None).is_some_and(|line| line.starts_with(NOT_REFRESHED)),
            "the reload said nothing into a footer nobody else was using"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_reload_with_problems_takes_the_new_tree_and_counts_them() {
        use std::os::unix::fs::PermissionsExt as _;

        // A `Problem` is a per-node fact the engine has already coloured
        // conservatively — the node is stale and says so — so a tree with
        // the run's documents in it beats the stale one it replaces, and it
        // is taken. What the footer adds is the count, in the words the
        // startup load already uses for it.
        let scratch = one_crate_to_load("reload-problems");
        let agent = Canned::new(&scratch, []);
        let Toggled { mut manifest, .. } = apply_toggle(
            &Manifest::new(),
            &scratch.root,
            &toggle(&scratch, "crates/engine", true),
            &agent,
            &mut Unwatched,
        )
        .expect("a subtree that walks and a manifest that writes");

        let (mut app, scope) = load(&scratch);
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedFresh),
            "the reader is looking at a pacted subtree"
        );

        // Something new on disk for the reload to find, and something it
        // cannot read: `crates/engine` and `crates/engine/src` are both
        // pacted, and neither can be hashed with an unreadable file under
        // it, so there are two problems and one of them is counted.
        scratch.write("docs/adr/one.md", "# One\n");
        let unreadable = scratch.path("crates/engine/src/lib.rs");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");

        let (events, received) = mpsc::channel();
        let mut pact = Pact::with_run(Running {
            events: received,
            cancel: CancelGuard::new(),
            work: pact_of(scratch.path("crates/engine")),
            before: app.clone(),
            unchanged: Vec::new(),
        });
        events
            .send(PactEvent::Finished(Ok(Toggled {
                manifest: manifest.clone(),
                granted: true,
                message: None,
                refusals: Vec::new(),
            })))
            .expect("the loop is still listening");
        pact.keep_up(&mut app, &mut manifest, &scope, Instant::now());

        let message = app.message().expect("problems are reported").to_owned();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");

        assert!(
            state_of(&app, &scratch.path("docs")).is_some(),
            "the tree that came back is the old one, without the problems in it"
        );
        assert_eq!(
            state_of(&app, &scratch.path("crates/engine")),
            Some(NodeState::PactedStale),
            "a node with no hash is coloured as if it had one"
        );
        assert!(
            message.contains("could not be hashed"),
            "the footer does not say what went wrong: {message}"
        );
        assert!(
            message.contains("and 1 more like it"),
            "the footer does not say how many there were: {message}"
        );
        assert!(
            !message.contains('\n'),
            "a footer line that wraps is a footer line that hides a row: {message}"
        );
    }

    mod watching {
        use std::time::{Duration, Instant};

        use warlock_engine::manifest_path;
        use warlock_tui::{QUIET_PERIOD, RELOAD_CEILING, WatchPolicy, Watching};

        use super::{
            CancelGuard, Canned, Loaded, Manifest, NodeState, Pact, PactEvent, Reloaded, Running,
            Scope, Toggled, Unwatched, apply_toggle, load, load_tree, mpsc, one_crate_to_load,
            pact_of, state_of, toggle,
        };
        use crate::POLL_INTERVAL;
        use crate::session::{NOT_WATCHING, Watched, note, start_watching};

        fn unwatched(scope: &Scope) -> Watched {
            let Loaded { tree, .. } =
                load_tree(&scope.root).expect("a scratch repository with a `.git/` loads");
            Watched {
                watching: Watching::Off("no watcher in a test".to_owned()),
                policy: WatchPolicy::new(&tree),
                manifest: manifest_path(&scope.repo_root),
            }
        }

        #[test]
        fn a_watcher_driven_reload_leaves_the_reader_exactly_where_they_were() {
            // The same five things a post-pact reload keeps, kept by the
            // same one reload: nobody pressed anything this time, so a
            // tree that threw the reader back to the top here would do it
            // while they were reading, which is worse than a tree that
            // never updates at all.
            let scratch = one_crate_to_load("watch-place");
            scratch.write("crates/engine/tests/one.rs", "#[test] fn one() {}\n");
            scratch.write("crates/tui/src/main.rs", "fn main() {}\n");
            scratch.write("docs/adr/one.md", "# One\n");

            // A pact from before this reader sat down, so the pacted-only
            // filter has something to keep and the reload has a colour to
            // change.
            let agent = Canned::new(&scratch, []);
            apply_toggle(
                &Manifest::new(),
                &scratch.root,
                &toggle(&scratch, "crates", true),
                &agent,
                &mut Unwatched,
            )
            .expect("a subtree that walks and a manifest that writes");

            let (mut app, scope) = load(&scratch);
            let mut watched = unwatched(&scope);
            app.set_viewport_height(4);
            app.toggle_files();
            app.toggle_pacted_only();
            app = app.with_collapsed([scratch.path("crates/tui")]);
            for _ in 0..5 {
                app.select_next();
            }

            let selected = app
                .selected_row()
                .map(|row| row.path.clone())
                .expect("a row is selected");
            let collapsed = app.collapsed().clone();
            let offset = app.scroll_offset();
            assert!(offset > 0, "the reader scrolled off the first row");
            assert_eq!(
                state_of(&app, &scratch.path("crates/engine")),
                Some(NodeState::PactedFresh),
                "the reader is looking at a subtree that was fresh when they sat down"
            );

            // Somebody else saves a file in a directory the last walk
            // produced. Nothing is pressed here and nothing is pressed
            // below: what stands in for the drain is the policy being told
            // the path was accepted.
            scratch.write("crates/engine/src/extra.rs", "//! Late arrival.\n");
            let saved_at = Instant::now();
            watched.policy.accepted(saved_at);

            assert!(
                !watched.round(&mut app, &scope, false, saved_at),
                "read the tree before the disk had gone quiet"
            );
            assert!(
                watched.round(&mut app, &scope, false, saved_at + QUIET_PERIOD),
                "the disk went quiet and the tree was never read"
            );

            assert_eq!(
                state_of(&app, &scratch.path("crates/engine")),
                Some(NodeState::PactedStale),
                "a directory that went stale under the reader is still drawn green"
            );
            assert_eq!(
                app.selected_row().map(|row| row.path.clone()),
                Some(selected),
                "the selection moved"
            );
            assert_eq!(app.collapsed(), &collapsed, "the collapsed set moved");
            assert!(app.pacted_only(), "the filter was dropped");
            assert!(app.show_files(), "the file toggle was dropped");
            assert_eq!(app.scroll_offset(), offset, "the window moved");
            // The header used to be asserted here too. It is not an `App`'s to
            // lose any more — a reload rebuilds the app and never touches the
            // `Chrome` beside it — so the claim moved to `session`, where the
            // thing that could drop it lives.
            assert_eq!(app.message(), None, "and nothing went wrong to report");
        }

        #[test]
        fn a_trigger_during_a_pact_waits_for_the_runs_own_reload() {
            // A run writes documents, the documents are events, and the
            // run ends by re-reading the tree anyway. So the trigger is
            // remembered rather than acted on: one reload at the end, not
            // one during the run and another after it — and none at all
            // over a tree the run is still writing into.

            const ROUNDS: u32 = 12;

            let scratch = one_crate_to_load("watch-in-flight");
            let (mut app, scope) = load(&scratch);
            let mut manifest = Manifest::new();
            let mut watched = unwatched(&scope);

            // What the run put on disk while it was running.
            scratch.write("docs/adr/one.md", "# One\n");
            let base = Instant::now();
            watched.policy.accepted(base);

            // Round after round with the pact in flight, long past the
            // quiet period and past the ceiling too: neither deadline is a
            // reason to read a tree out from under a run.
            for round in 1..=ROUNDS {
                let at = base + QUIET_PERIOD * round;
                assert!(
                    !watched.round(&mut app, &scope, true, at),
                    "the tree was read under a run in flight, {at:?} in"
                );
            }
            assert!(
                QUIET_PERIOD * ROUNDS > RELOAD_CEILING,
                "the rounds above stopped short of the ceiling, so they proved nothing about it"
            );
            assert!(
                state_of(&app, &scratch.path("docs")).is_none(),
                "the tree moved while the run was still going"
            );
            assert!(
                watched.policy.owes_reload(),
                "the trigger was dropped rather than remembered"
            );

            // The run ends the way every run ends — one reload at the
            // bottom of `apply_progress` — and the tree it read is what
            // the loop hands back to the policy.
            let (events, received) = mpsc::channel();
            let mut pact = Pact::with_run(Running {
                events: received,
                cancel: CancelGuard::new(),
                work: pact_of(scratch.path("crates/engine")),
                before: app.clone(),
                unchanged: Vec::new(),
            });
            events
                .send(PactEvent::Finished(Ok(Toggled {
                    manifest: manifest.clone(),
                    granted: true,
                    message: None,
                    refusals: Vec::new(),
                })))
                .expect("the loop is still listening");
            let ended = base + Duration::from_secs(3);
            // The edge itself: `keep_up` says the run ended on this round and
            // on no other, and hands over the tree its own reload read.
            let Some(Reloaded(reloaded)) = pact.keep_up(&mut app, &mut manifest, &scope, ended)
            else {
                panic!("the run ended without saying so");
            };
            assert!(!pact.running(), "the run is over");
            assert!(reloaded.is_some(), "the run's own reload read the tree");
            watched.caught_up(reloaded.as_ref(), ended);

            assert!(
                state_of(&app, &scratch.path("docs")).is_some(),
                "the run's reload did not bring back what moved during it"
            );
            assert!(
                !watched.policy.owes_reload(),
                "the trigger outlived the reload that answered it"
            );
            assert!(
                !watched.round(
                    &mut app,
                    &scope,
                    false,
                    ended + RELOAD_CEILING + QUIET_PERIOD
                ),
                "the run reloaded twice: once at its end and once for the events it caused"
            );
        }

        #[test]
        fn the_line_saying_live_updates_are_off_is_said_once_and_gives_way() {
            // A watcher that would not start costs the noticing and
            // nothing else, so it is one line, said where the watcher was
            // asked for, and it never talks over anything the reader
            // actually asked for.
            const REFUSED: &str = "the manifest would not save";

            let scratch = one_crate_to_load("watch-off");
            let (mut app, scope) = load(&scratch);
            let mut watched = unwatched(&scope);

            let line = watched
                .off_note()
                .expect("a watcher that would not start says so");
            assert!(
                line.starts_with(NOT_WATCHING),
                "the footer says nothing about live updates: {line}"
            );
            assert!(
                line.len() > NOT_WATCHING.len() + 2,
                "the footer does not say why: {line}"
            );
            assert!(
                !line.contains('\n'),
                "a footer line that wraps is a footer line that hides a row: {line}"
            );

            // Said once, before the first frame, into a footer nobody else
            // was using.
            note(&mut app, line.clone());
            assert_eq!(app.message(), Some(line.as_str()));

            // And then a pact has something to say, which is the news: the
            // rounds after it are rounds of a loop that is not watching
            // anything, and not one of them says this again.
            app.set_message(REFUSED);
            let base = Instant::now();
            for round in 0..20 {
                watched.round(&mut app, &scope, false, base + POLL_INTERVAL * round);
            }
            assert_eq!(
                app.message(),
                Some(REFUSED),
                "the line said itself again, over a run's own message"
            );
            note(&mut app, line);
            assert_eq!(
                app.message(),
                Some(REFUSED),
                "the footer's precedence is the other way round for this line"
            );

            // And a watcher that did start says nothing at all. A real one
            // this time, over the scratch repository — started and asked,
            // with nothing waited for, since when this operating system
            // reports a write is not what is being asserted.
            let Loaded { tree, .. } = load_tree(&scope.root).expect("the scratch repository");
            assert_eq!(
                Watched::start(&scope, &tree).off_note(),
                None,
                "a working watcher put a line on the footer"
            );

            // And the two halves in the one call the loop actually makes: the
            // watcher started and the footer left alone, because there was
            // nothing to say about it.
            let (mut fresh, _) = load(&scratch);
            let _watched = start_watching(&mut fresh, &scope, &tree);
            assert_eq!(
                fresh.message(),
                None,
                "starting a working watcher said something"
            );
        }
    }
}
