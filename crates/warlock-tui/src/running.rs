//! `warlock pact <path>` and `warlock refresh <path>`: the two subcommands that
//! spend anything, hence their own file rather than two more arms of
//! [`mod@crate::edits`].
//!
//! [`ran`] takes an [`Opened`], so the boundary is asked before the walk. Asked
//! afterwards it would have listed somebody else's directories before refusing;
//! asked after the first directory it would have spent a pass and overwritten a
//! `WARLOCK.md` that no exit status puts back. Which directories a refresh
//! describes stays the engine's judgement, because a second opinion about
//! staleness here would disagree with the colour the tree is drawn in.
//!
//! Failures are named on stderr one line each, unlike the footer's one-line
//! shape: the reader here is often a script or a log read tomorrow, and "and 99
//! more" throws away the only list of what to go and look at. A cancelled run
//! names none of its own, because nothing tells the killed pass apart from a
//! real failure. The environment is read in [`started`] alone; everything under
//! it takes the repository, agent, observer and say-when as parameters, which
//! is the seam the tests run through.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};

use warlock_engine::{Agent, PactedSubtree, Pacting, document, pact, to_manifest_path};
use warlock_tui::{Cancel, ClaudeAgent};

use crate::CANCELLED;
use crate::descent::{Descent, carry_on, descend};
use crate::edits::{Opened, opened};
use crate::error::{Error, one_line};

// The headless counterpart of [`Reporting`](crate::pacting), and much smaller:
// no channel, no thread and no screen, so where that one forwards five kinds of
// event to an event loop, this writes the two a person watching a pipe can act
// on.
//
// Generic over the writer rather than reaching for `io::stdout` itself, for the
// reason every other seam in this crate is a parameter: the tests assert on the
// exact lines a run produces, in order, and a function that printed could only
// be tested by spawning a process to read the output of.
//
// A write that fails is ignored — the same shrug `Reporting` gives a send into a
// closed channel, for a stronger reason. A closed stdout is
// `warlock pact . | head -1`, and failing a run of model passes because the
// thing reading its progress went away would be spending minutes of somebody's
// tokens on the state of a pipe.
struct Progress<W: Write> {
    root: PathBuf,
    out: W,
    // Read rather than written here: this port has no opinion about when a run
    // should end, it only carries somebody else's to the one place the engine
    // asks.
    cancel: Cancel,
    // Kept because the failure report needs it and nothing else has it: the
    // `PactedSubtree` carries the failures but not the size of the run they
    // happened in, and "3 failed" without "of 100" is the illegible half of the
    // report. The engine's own denominator, unaltered, and it does not move for
    // the length of a run — `starting` is handed `directories.len()` every time
    // — so reading it after the descent reads the number every progress line was
    // counting against.
    //
    // A field rather than a count of the lines written, because the run's size
    // is a thing the engine states and this port's arithmetic about it would be
    // a second opinion waiting to disagree.
    total: usize,
}

impl<W: Write> Progress<W> {
    const fn new(root: PathBuf, out: W, cancel: Cancel) -> Self {
        Self {
            root,
            out,
            cancel,
            total: 0,
        }
    }

    const fn total(&self) -> usize {
        self.total
    }

    fn say(&mut self, fact: &str) {
        // Ignored on purpose; see the type's doc.
        let _ = writeln!(self.out, "warlock: {fact}");
    }
}

impl<W: Write> pact::Observer for Progress<W> {
    // The say-when is read before anything is printed, so a cancelled run
    // neither announces a directory it will not describe nor describes it. This
    // is the only question the engine asks that a run can be stopped at, which
    // is what makes a stop leave whole documents behind rather than half of one.
    //
    // The fraction is the engine's own, unaltered and one-based, and its
    // denominator does not move for the length of the run — so `[3/12]` is a
    // thing a reader can watch rather than a running total that redefines
    // itself. It is on this line and not the completion line because this is
    // where it means something: it counts the directories offered.
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        if carry_on(&self.cancel) == Pacting::Stop {
            return Pacting::Stop;
        }
        // Remembered as well as printed, and remembered every time rather than
        // only the first: the engine states one denominator for a run, so the
        // last thing it said and the first are the same number, and a port that
        // only recorded one of them would be a port with a rule about which.
        self.total = total;
        let named = named(&self.root, directory);
        self.say(&format!("[{position}/{total}] documenting {named}"));
        Pacting::Continue
    }

    // On stdout with the progress and not on stderr with the report, because a
    // mended slot is a document that was written, not a directory that was
    // missed: the failure report is the list of things to go and look at, and a
    // repair belongs to the story of the run. It costs the run nothing — no
    // status, no `failed`, no `total` — but a log read tomorrow should still be
    // able to tell a repaired entry from a written one, so it says which slot
    // and what was done to it.
    fn repaired(&mut self, directory: &Path, mend: &document::Mend) {
        let named = named(&self.root, directory);
        self.say(&format!("{named} — {mend}"));
    }

    fn documented(&mut self, directory: &Path) {
        let named = named(&self.root, directory);
        self.say(&format!("documented {named}"));
    }

    fn unchanged(&mut self, directory: &Path) {
        let named = named(&self.root, directory);
        self.say(&format!("unchanged {named}"));
    }

    // Both names on the line. A headless run is read in a log after the fact,
    // often by whoever has to explain why a directory is still yellow, and
    // `skipped crates/tui` on its own is the half of the answer that does not
    // help.
    fn skipped(&mut self, directory: &Path, below: &Path) {
        let (named, below) = (named(&self.root, directory), named(&self.root, below));
        self.say(&format!(
            "skipped {named} — {below} below it was not documented"
        ));
    }
}

fn named(root: &Path, directory: &Path) -> String {
    to_manifest_path(root, directory).unwrap_or_else(|_| directory.display().to_string())
}

// A value rather than a printing function, for the reason `Opened::unpacted`
// hands its success line back instead of printing it: the report is the
// interesting half of this module, and a test that could only read it by
// spawning a process would be a test of the shell.
//
// Only ever built for a run that had failures — [`report`] answers `None` for a
// clean run — so there is no empty report to be printed as a row of nothing and
// no `0 of 12 directories failed` line.
#[derive(Debug)]
struct Report {
    lines: Vec<String>,
    failed: usize,
    total: usize,
}

impl Report {
    fn onto<W: Write>(&self, err: &mut W) {
        for line in &self.lines {
            // Ignored on purpose; see above.
            let _ = writeln!(err, "warlock: {line}");
        }
    }

    fn status(&self) -> Error {
        Error::Failures {
            failed: self.failed,
            total: self.total,
        }
    }
}

// One line per failing directory and no line for any other. Directories are
// named against `root` the way [`Progress`] names them, so the failure line for
// a directory and the progress line that entered it say the same word; the
// engine's sentence is flattened because a report is read a line at a time and a
// TOML diagnostic wrapping over four of them would be four directories' worth of
// screen for one.
//
// A directory is named once however many ways it went wrong. Today it can only
// go wrong once, so this is a promise about the report rather than a filter that
// fires: `failed` counts directories, `total` counts directories, and the two
// have to be countable against each other.
fn report(root: &Path, failures: &[pact::Failure], total: usize) -> Option<Report> {
    let mut named_already: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for failure in failures {
        let directory = named(root, failure.directory());
        if named_already.contains(&directory) {
            continue;
        }
        lines.push(format!("{directory} — {}", one_line(&failure.to_string())));
        named_already.push(directory);
    }

    if lines.is_empty() {
        return None;
    }
    Some(Report {
        failed: lines.len(),
        lines,
        total,
    })
}

// The whole of `warlock pact` and `warlock refresh` below the gate. `opened` is
// the proof the boundary was asked — it cannot be built any other way — so this
// does not ask it again, and there is no arrangement of the arguments in which
// it could be skipped.
//
// The manifest is saved inside `descend`, once, after the descent and whatever
// it came to: a run in which some directories failed still earned the grants of
// the ones that did not, and throwing them away would mean paying for them
// again. What comes back is the `PactedSubtree` whole, as a value rather than as
// something printed, so the report a run's failures deserve is a thing a test
// can hold up.
fn ran(
    opened: &Opened,
    descent: Descent,
    agent: &dyn Agent,
    observer: &mut dyn pact::Observer,
) -> Result<PactedSubtree, Error> {
    descend(
        descent,
        opened.target(),
        opened.repo_root(),
        opened.manifest(),
        agent,
        observer,
    )
}

// The whole of the signal handling, and it is a flag and two lines. Two presses
// rather than one because they are two different questions. The first is "stop
// when you can", and what it buys is the manifest: the directories already
// documented are hashed, granted and saved by the save at the end of the
// descent, which cannot happen if the process dies here. The second is "stop
// now", honoured literally with nothing saved and nothing printed — nothing is
// corrupted by taking it, because every file is written beside and renamed over
// and the descent's thread is simply abandoned.
//
// No `unsafe`, and none is needed: `ctrlc::set_handler` does the registration
// and calls this closure on an ordinary thread of its own, where latching an
// `AtomicBool`, taking a mutex and killing a child are ordinary rather than the
// undefined behaviour they would be in signal context. That is the whole reason
// the dependency is in the manifest.
//
// `pressed` is this function's own flag and not `Cancel::is_cancelled`, because
// the two say different things: a handle can be latched by the run itself, and
// "has anybody pressed Ctrl-C" is a question about the keyboard.
//
// Called once per process, after the boundary is asked and before the first
// pass, so a run refused here has spent nothing and written nothing.
fn listening() -> Result<Cancel, Error> {
    let cancel = Cancel::new();
    let latch = cancel.clone();
    let pressed = AtomicBool::new(false);
    ctrlc::set_handler(move || {
        if pressed.swap(true, Ordering::SeqCst) {
            // The second press, honoured as asked: no save, no report, and the
            // status the run was going to leave with anyway.
            process::exit(i32::from(CANCELLED));
        }
        latch.cancel();
    })
    .map_err(|source| Error::Signal { source })?;
    Ok(cancel)
}

// The three endings a run can have, decided in one place and in this order.
// `cancelled` wins because a stopped run's failures are the stopping — the
// killed pass comes back in `PactedSubtree::failures` like any other, and
// nothing in that list says which entries the reader caused — so a cancelled run
// prints no per-directory lines at all. What finished is on stdout, where it has
// been all along.
//
// Takes the stderr to write to rather than being three lines inside [`started`],
// for the reason [`Progress`] is generic over its writer: this is the decision
// the endings of a run are made by, and a test that could only read it by
// spawning a process would be a test of the shell.
fn ending<W: Write>(cancelled: bool, report: Option<&Report>, err: &mut W) -> Result<(), Error> {
    if cancelled {
        return Err(Error::Cancelled);
    }
    match report {
        None => Ok(()),
        Some(report) => {
            report.onto(err);
            Err(report.status())
        }
    }
}

// The one place in this module that reads the working directory, the home
// directory, stdout or the keyboard, kept to five lines so everything worth
// testing is underneath it. A closed boundary leaves through the `?` with
// nothing spawned, nothing written and no handler installed.
//
// The order is the load-bearing part: a manifest that would not save leaves
// through `descended` as `Error::Manifest` and a 1, failures or cancel or
// neither, because a run whose record never reached the disk is warlock unable
// to do the thing rather than a run that completed imperfectly.
fn started(descent: Descent, path: &Path) -> Result<(), Error> {
    let opened = opened(descent.wanted(), path)?;
    let cancel = listening()?;
    let agent = ClaudeAgent::new().with_cancel(cancel.clone());

    descended(
        &opened,
        descent,
        &agent,
        &cancel,
        io::stdout(),
        &mut io::stderr(),
    )?
    .outcome
}

// The observer is handed back still holding what it wrote and the denominator it
// counted against, because a caller that gave it a `Vec<u8>` wants both. On the
// real road the writer is stdout and nobody reads either again. No `Debug`,
// because [`Progress`] has none: a writer is not a value to print.
//
// The fields are the point: [`started`] reads only `outcome`, while the suite
// reads all four off the same value. Dropping the three would make the
// composition untestable again, which is the thing this type exists to fix.
#[allow(dead_code, reason = "read by the tests that drive this composition")]
struct Descended<W: Write> {
    subtree: PactedSubtree,
    progress: Progress<W>,
    report: Option<Report>,
    outcome: Result<(), Error>,
}

// The composition, and one function rather than four lines in [`started`]
// because the order is the load-bearing part. Four things have to be true
// together, and are true here:
//
// * The observer and the agent answer to one `Cancel`. The handle handed in is
//   cloned into `Progress`, and the caller has already given the same one to the
//   agent — so a single Ctrl-C kills the pass in flight *and* stops the descent
//   at the next directory, rather than doing one and not the other.
// * The manifest is saved before anything has an opinion. That happens inside
//   `descend`, which is what makes `Error::Failures` "completed with failures"
//   rather than a failure, and a cancel a run that recorded what it finished.
// * The denominator is read after the descent. `Progress::total` is `0` until
//   the engine states the run's size, so the report has to count against it
//   afterwards; counting before would report every failure out of nothing.
// * `ending` is last, so a manifest that would not save leaves through the `?`
//   above as the bigger news.
fn descended<O: Write, E: Write>(
    opened: &Opened,
    descent: Descent,
    agent: &dyn Agent,
    cancel: &Cancel,
    out: O,
    err: &mut E,
) -> Result<Descended<O>, Error> {
    let mut progress = Progress::new(opened.repo_root().to_path_buf(), out, cancel.clone());
    let subtree = ran(opened, descent, agent, &mut progress)?;

    let report = report(opened.repo_root(), &subtree.failures, progress.total());
    let outcome = ending(cancel.is_cancelled(), report.as_ref(), err);

    Ok(Descended {
        subtree,
        progress,
        report,
        outcome,
    })
}

// One line, for the reason `edits::unpact` is two: the whole of the command is
// [`started`], and what tells it from the refresh below is the tag it is handed.
pub(crate) fn pact(path: &Path) -> Result<(), Error> {
    started(Descent::Pact, path)
}

pub(crate) fn refresh(path: &Path) -> Result<(), Error> {
    started(Descent::Refresh, path)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::{
        Agent, Manifest, PactEntry, PactedSubtree, agent, manifest_path, save_sigils, stub_answer,
    };

    use warlock_tui::Cancel;

    use super::{Descent, Report, descended};
    use crate::edits::Opened;
    use crate::error::Error;
    use crate::session::load_manifest;
    // The sentence itself, asked of the one function that writes it rather than
    // retyped: the footer, `warlock unpact` and a refused run say one thing
    // about one boundary, and a test holding its own copy of those words would
    // go on passing while the doors drifted apart.
    use crate::boundary::closed_scope_message;
    use crate::status_for;

    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    // Three pactable directories — the root, `alpha` and `beta`. Small enough
    // that a run over it is three passes of a fake, and shaped enough that "only
    // the stale one" is a claim about a real sibling.
    fn a_repository() -> tempfile::TempDir {
        let repo = a_dir();
        write(repo.path(), ".git/HEAD", "ref: refs/heads/main\n");
        write(repo.path(), "alpha/lib.rs", "//! Alpha.\n");
        write(repo.path(), "beta/lib.rs", "//! Beta.\n");
        repo
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("a file has a parent"))
            .expect("the directories above a file");
        fs::write(&path, contents).expect("a file");
    }

    fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, repo_root, &sigils).expect("a config that writes");
    }

    fn scoped(repo_root: &Path, module: &str, scope: &str) {
        Manifest::with_entries([PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
            .with_scope(scope)])
        .save(repo_root)
        .expect("a manifest that saves");
    }

    fn manifest_bytes(repo_root: &Path) -> Option<Vec<u8>> {
        fs::read(manifest_path(repo_root)).ok()
    }

    const CLAUDE: &str = "claude";

    #[derive(Debug)]
    // `pacting`'s `Canned` with everything this module does not test taken out:
    // no activities, because there is no panel. What is kept is the pair of
    // facts these tests turn on — which directories were offered a pass, in
    // order, and whether a manifest was on disk while the passes were running,
    // which is how "saved once, at the end" is asked of a run rather than of a
    // mock — and the say-when, because a cancel arrives during a pass.
    //
    // The refusal it can be given is `agent::Error::NotFound`, which is what
    // every directory of every run gets on a machine with no `claude` on `PATH`:
    // the case the failure report is shaped around.
    struct Canned {
        root: PathBuf,
        refused: Vec<String>,
        cancel_at: Option<(String, Cancel)>,
        seen: RefCell<Vec<(PathBuf, bool)>>,
    }

    impl Canned {
        fn refusing(root: &Path, refused: &[&str]) -> Self {
            Self {
                root: root.to_path_buf(),
                refused: refused.iter().map(|module| (*module).to_owned()).collect(),
                cancel_at: None,
                seen: RefCell::new(Vec::new()),
            }
        }

        // Latching from inside a pass is where a press really lands, and it is
        // the one way to have one arrive mid-run without a test sending its own
        // process a signal.
        fn cancelling_at(mut self, directory: &str, cancel: Cancel) -> Self {
            self.cancel_at = Some((directory.to_owned(), cancel));
            self
        }

        fn directories(&self) -> Vec<String> {
            self.seen
                .borrow()
                .iter()
                .map(|(directory, _)| named(&self.root, directory))
                .collect()
        }

        fn saw_a_manifest(&self) -> bool {
            self.seen.borrow().iter().any(|(_, saved)| *saved)
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            let directory = request.directory().to_path_buf();
            self.seen
                .borrow_mut()
                .push((directory.clone(), manifest_path(&self.root).is_file()));
            // Pressed while this pass is running, and the pass still answers:
            // the press beats the engine to the *next* directory rather than to
            // this one's answer. A real Ctrl-C kills the child as well, and the
            // pass it kills comes back as a failure — which is what `refused`
            // already produces, so a test that wants both asks for both.
            if let Some((at, cancel)) = &self.cancel_at
                && *at == named(&self.root, &directory)
            {
                cancel.cancel();
            }
            if self.refused.contains(&named(&self.root, &directory)) {
                return Err(agent::Error::NotFound {
                    program: CLAUDE.to_owned(),
                });
            }
            Ok(agent::Response::new(stub_answer(request)))
        }
    }

    fn named(root: &Path, directory: &Path) -> String {
        super::named(root, directory)
    }

    #[derive(Debug)]
    struct Run {
        subtree: PactedSubtree,
        agent: Canned,
        lines: Vec<String>,
        err: Vec<String>,
        report: Option<Report>,
        outcome: Result<(), Error>,
    }

    impl Run {
        fn ended(&self) -> (&Result<(), Error>, Vec<String>) {
            (&self.outcome, self.err.clone())
        }

        const fn report(&self) -> Option<&Report> {
            self.report.as_ref()
        }

        fn stderr(&self) -> Vec<String> {
            let (outcome, mut lines) = self.ended();
            if let Err(error) = outcome {
                lines.push(format!("warlock: {error}"));
            }
            lines
        }
    }

    // [`driven`] with a model that answers everything and a say-when nobody
    // pulls, which is the ordinary run.
    fn run(repo_root: &Path, home: &Path, descent: Descent, path: &str) -> Result<Run, Error> {
        run_refusing(repo_root, home, descent, path, &[])
    }

    fn run_refusing(
        repo_root: &Path,
        home: &Path,
        descent: Descent,
        path: &str,
        refused: &[&str],
    ) -> Result<Run, Error> {
        let cancel = Cancel::new();
        let agent = Canned::refusing(repo_root, refused);
        driven(repo_root, home, descent, path, agent, &cancel)
    }

    fn run_cancelling(
        repo_root: &Path,
        home: &Path,
        descent: Descent,
        path: &str,
        refused: &[&str],
        at: &str,
    ) -> Result<Run, Error> {
        let cancel = Cancel::new();
        let agent = Canned::refusing(repo_root, refused).cancelling_at(at, cancel.clone());
        driven(repo_root, home, descent, path, agent, &cancel)
    }

    // The production road exactly: the boundary through `Opened::new`, then the
    // same `descended` call `started` makes, with the same arguments in the same
    // order. This used to re-assemble that composition by hand, which meant the
    // suite proved that *a* correct order worked rather than that the subcommand
    // used it.
    //
    // What `started` still has that this does not is the environment and nothing
    // else: the working directory, the real home, a real `ClaudeAgent`, the
    // signal handler, and stdout and stderr in place of two `Vec<u8>`s.
    fn driven(
        repo_root: &Path,
        home: &Path,
        descent: Descent,
        path: &str,
        agent: Canned,
        cancel: &Cancel,
    ) -> Result<Run, Error> {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        let opened = Opened::new(
            repo_root.to_path_buf(),
            Some(home),
            manifest,
            repo_root.join(path),
        )?;
        // The production composition, not a second one assembled here: whatever
        // order `descended` puts these in is the order under test.
        let mut err = Vec::new();
        let done = descended(&opened, descent, &agent, cancel, Vec::new(), &mut err)?;

        Ok(Run {
            subtree: done.subtree,
            agent,
            lines: written(done.progress.out),
            err: written(err),
            report: done.report,
            outcome: done.outcome,
        })
    }

    fn written(bytes: Vec<u8>) -> Vec<String> {
        String::from_utf8(bytes)
            .expect("the lines warlock writes are its own text")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn stored_modules(repo_root: &Path) -> Vec<String> {
        load_manifest(repo_root)
            .expect("a manifest that reads")
            .entries()
            .iter()
            .map(|entry| entry.module().to_owned())
            .collect()
    }

    #[test]
    fn an_open_boundary_documents_every_directory_and_saves_the_manifest_once() {
        let repo = a_repository();
        let home = a_dir();

        let run = run(repo.path(), home.path(), Descent::Pact, ".").expect("nothing is scoped");

        // One pass per directory, and one document beside each of them.
        assert_eq!(run.agent.directories(), ["beta", "alpha", "."]);
        assert!(
            run.subtree.failures.is_empty(),
            "{:?}",
            run.subtree.failures
        );
        for module in [".", "alpha", "beta"] {
            let document = repo.path().join(module).join("WARLOCK.md");
            assert!(document.is_file(), "{} is missing", document.display());
        }
        // Saved, and saved once: no manifest existed while the passes were
        // running, so the file on disk is the work of the single write after
        // the descent.
        assert!(
            !run.agent.saw_a_manifest(),
            "the manifest was written while the run was still going"
        );
        assert_eq!(stored_modules(repo.path()), [".", "alpha", "beta"]);
        assert_eq!(status_for(&Ok(())), 0);
        // And nothing on stderr: a run with nothing wrong with it has no
        // report, so there is no `0 of 3 directories failed` for a script to
        // read as news.
        assert!(run.report().is_none());
        assert!(run.stderr().is_empty());
    }

    #[test]
    fn progress_names_every_directory_from_the_root_as_it_is_entered_and_as_it_completes() {
        let repo = a_repository();
        let home = a_dir();

        let run = run(repo.path(), home.path(), Descent::Pact, ".").expect("nothing is scoped");

        // The two announcements a reader on the far end of a pipe can act on,
        // in the order the run made them: every directory is entered before it
        // is documented, and the fraction counts the directories offered.
        assert_eq!(
            run.lines,
            [
                "warlock: [1/3] documenting beta",
                "warlock: documented beta",
                "warlock: [2/3] documenting alpha",
                "warlock: documented alpha",
                "warlock: [3/3] documenting .",
                "warlock: documented .",
            ]
        );
        // Named the way `.warlock/pacts.toml` names them, so a line of output
        // and a line of the manifest say one word about one directory.
        for module in stored_modules(repo.path()) {
            assert!(
                run.lines
                    .iter()
                    .any(|line| line.ends_with(&format!("documenting {module}"))),
                "nothing announced {module}: {:?}",
                run.lines
            );
        }
    }

    #[test]
    fn a_closed_boundary_refuses_before_a_pass_is_spent_and_leaves_the_manifest_alone() {
        let repo = a_repository();
        let home = a_dir();
        // `alpha` is somebody else's, and this machine holds nothing at all —
        // the ordinary state of a fresh checkout.
        scoped(repo.path(), "alpha", "platform");
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let refused = run(repo.path(), home.path(), Descent::Pact, "alpha");

        let error = refused.expect_err("a scope this machine does not hold refuses");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the boundary was refused as something else: {error:?}"
        );
        // The footer's own sentence, named rather than copied.
        assert_eq!(error.to_string(), closed_scope_message("alpha", "platform"));
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        // Refused, not failed: nothing was spent, so this is not the 1 warlock
        // spends on something it could not do.
        assert_eq!(status_for(&Err(error)), 3);
        // And nothing was spent: no document beside the directory, and the
        // manifest is the bytes that were read.
        assert!(!repo.path().join("alpha").join("WARLOCK.md").exists());
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn a_scope_this_machine_holds_lets_the_run_through() {
        // The same repository and the same boundary as the refusal above, with
        // the one thing that decides it changed: this machine holds the sigil.
        // A gate that refused both would pass that test and be useless.
        let repo = a_repository();
        let home = a_dir();
        scoped(repo.path(), "alpha", "platform");
        holding(home.path(), repo.path(), &["platform"]);

        let run = run(repo.path(), home.path(), Descent::Pact, "alpha")
            .expect("this machine holds the boundary over `alpha`");

        assert_eq!(run.agent.directories(), ["alpha"]);
        assert!(repo.path().join("alpha").join("WARLOCK.md").is_file());
        // And the boundary the run passed through is still on the entry it was
        // written on: a run describes a directory, it does not re-decide whose
        // it is.
        assert_eq!(
            load_manifest(repo.path())
                .expect("a manifest that reads")
                .entry("alpha")
                .and_then(|entry| entry.scope()),
            Some("platform")
        );
    }

    #[test]
    fn a_refresh_describes_the_stale_directory_and_leaves_the_fresh_one_as_it_found_it() {
        let repo = a_repository();
        let home = a_dir();
        run(repo.path(), home.path(), Descent::Pact, ".").expect("nothing is scoped");
        let alpha = load_manifest(repo.path())
            .expect("a manifest that reads")
            .entry("alpha")
            .expect("the pact granted `alpha`")
            .clone();
        // One directory moves under warlock's feet, which makes it and the root
        // above it stale and leaves its sibling exactly where it was.
        write(repo.path(), "beta/lib.rs", "//! Beta, rewritten.\n");

        let run = run(repo.path(), home.path(), Descent::Refresh, ".").expect("nothing is scoped");

        // The engine's own judgement of what is stale, unnarrowed and
        // unwidened: `alpha` costs no pass.
        assert_eq!(run.agent.directories(), ["beta", "."]);
        assert!(
            run.subtree.failures.is_empty(),
            "{:?}",
            run.subtree.failures
        );
        assert!(
            !run.lines.iter().any(|line| line.contains("alpha")),
            "a directory nothing was spent on was announced: {:?}",
            run.lines
        );
        // And it keeps everything it had — the grant included, which is the
        // whole point of not describing it again.
        assert_eq!(
            load_manifest(repo.path())
                .expect("a manifest that reads")
                .entry("alpha"),
            Some(&alpha)
        );
        assert_eq!(stored_modules(repo.path()), [".", "alpha", "beta"]);
    }

    #[test]
    fn one_directory_failing_names_it_counts_it_and_leaves_the_rest_of_the_run_granted() {
        let repo = a_repository();
        let home = a_dir();

        let run = run_refusing(repo.path(), home.path(), Descent::Pact, ".", &["alpha"])
            .expect("a refused pass fails one directory, not the run");

        // The run happened: every directory was offered a pass, and the two the
        // model answered have their documents.
        assert_eq!(run.agent.directories(), ["beta", "alpha", "."]);
        assert!(repo.path().join("beta").join("WARLOCK.md").is_file());
        assert!(!repo.path().join("alpha").join("WARLOCK.md").exists());
        // And the manifest was saved anyway, holding what the rest of the
        // subtree earned: `beta` is granted, `alpha` has no entry at all, and
        // the root sits above a directory with no document so it is pacted
        // without a grant. Throwing that away would mean paying for it twice.
        assert_eq!(stored_modules(repo.path()), [".", "beta"]);
        let manifest = load_manifest(repo.path()).expect("a manifest that reads");
        assert!(
            manifest
                .entry("beta")
                .expect("`beta` was documented")
                .granted_hash()
                .is_some()
        );

        let report = run.report().expect("one directory failed");
        // One line, naming the directory the manifest's way and then saying
        // what happened to it in the engine's own words.
        assert_eq!(report.lines.len(), 1, "{:?}", report.lines);
        assert!(
            report.lines[0].starts_with("alpha — "),
            "the failing directory is not named root-relative first: {:?}",
            report.lines[0]
        );
        assert!(
            report.lines[0].contains(CLAUDE),
            "the reason went missing: {:?}",
            report.lines[0]
        );
        assert!(
            !report.lines[0].contains('\n'),
            "a report is read a line at a time: {:?}",
            report.lines[0]
        );
        // The pair of counts: one directory of the three the run offered.
        assert_eq!((report.failed, report.total), (1, 3));
        assert_eq!(
            report.status().to_string(),
            "1 of 3 directories failed — the manifest holds what the rest earned"
        );
        // Completed with failures, which is neither the 0 of a run that worked
        // nor the 1 of a warlock that could not do the thing.
        assert_eq!(status_for(&Err(report.status())), 4);
        // Every failure and the count go to stderr; the progress stays on
        // stdout, so `warlock pact . > run.log` still shows what went wrong.
        assert_eq!(
            run.stderr(),
            [
                format!("warlock: {}", report.lines[0]),
                format!("warlock: {}", report.status()),
            ]
        );
        assert!(
            !run.lines.iter().any(|line| line.contains(CLAUDE)),
            "a failure was announced on stdout: {:?}",
            run.lines
        );
        // The directory that failed was entered and never completed, which is
        // what makes the report the only place its name is bad news.
        assert!(
            run.lines
                .contains(&"warlock: [2/3] documenting alpha".to_owned())
        );
        assert!(!run.lines.contains(&"warlock: documented alpha".to_owned()));
    }

    #[test]
    fn a_refresh_says_which_failure_below_cost_a_directory_its_pass() {
        let repo = a_repository();
        let home = a_dir();
        run(repo.path(), home.path(), Descent::Pact, ".").expect("nothing is scoped");
        let root = load_manifest(repo.path())
            .expect("a manifest that reads")
            .entry(".")
            .expect("the pact granted the root")
            .clone();
        // `beta` moves, so `beta` and the root above it are stale, and the pass
        // over `beta` is the one that refuses.
        write(repo.path(), "beta/lib.rs", "//! Beta, rewritten.\n");

        let run = run_refusing(repo.path(), home.path(), Descent::Refresh, ".", &["beta"])
            .expect("nothing is scoped");

        // One request went out, for the directory that refused. The root above
        // it was never going to be granted, so it was never paid for.
        assert_eq!(run.agent.directories(), ["beta"]);
        assert!(
            run.lines
                .iter()
                .any(|line| line == "warlock: skipped . — beta below it was not documented"),
            "the run says which directory it passed over and why: {:?}",
            run.lines
        );
        // And the entry it did not touch is the entry it found, grant and all —
        // stale on a hash that moved, which is what the next run will fix.
        assert_eq!(
            load_manifest(repo.path())
                .expect("a manifest that reads")
                .entry("."),
            Some(&root),
        );
    }

    #[test]
    fn a_machine_with_no_model_names_every_directory_once_and_says_how_many_of_how_many() {
        let repo = a_repository();
        let home = a_dir();

        // Every pass refused, which is what a machine with no `claude` on PATH
        // does to every directory of every run.
        let run = run_refusing(
            repo.path(),
            home.path(),
            Descent::Pact,
            ".",
            &[".", "alpha", "beta"],
        )
        .expect("a run where everything fails is still a run");

        assert_eq!(run.subtree.failures.len(), 3, "{:?}", run.subtree.failures);
        // Saved even so: the file did not exist before the run, so its being
        // there is the save having happened.
        assert!(manifest_bytes(repo.path()).is_some());
        assert!(stored_modules(repo.path()).is_empty());

        let report = run.report().expect("every directory failed");
        // Every one of them named, and each of them once: a run over a hundred
        // directories is a hundred names to go and look at, not "and 99 more".
        assert_eq!(report.lines.len(), 3);
        for module in [".", "alpha", "beta"] {
            let named: Vec<&String> = report
                .lines
                .iter()
                .filter(|line| line.starts_with(&format!("{module} — ")))
                .collect();
            assert_eq!(named.len(), 1, "{module} is not named once: {named:?}");
        }
        // And one line of arithmetic under them, which is the whole of what
        // stops a hundred identical sentences being illegible.
        assert_eq!((report.failed, report.total), (3, 3));
        assert_eq!(
            report.status().to_string(),
            "3 of 3 directories failed — the manifest holds what the rest earned"
        );
        assert_eq!(status_for(&Err(report.status())), 4);
        assert_eq!(run.stderr().len(), 4, "{:?}", run.stderr());
        // Nothing was documented, so nothing said so.
        assert!(
            !run.lines.iter().any(|line| line.contains("documented")),
            "{:?}",
            run.lines
        );
    }

    #[test]
    fn a_cancel_stops_the_descent_between_directories_and_saves_what_finished() {
        let repo = a_repository();
        let home = a_dir();

        // Ctrl-C during the pass over `alpha`. The engine walks children before
        // parents in reverse path order, so the run is `beta`, `alpha`, and
        // then the root — which is the directory the press arrives in time for.
        let run = run_cancelling(repo.path(), home.path(), Descent::Pact, ".", &[], "alpha")
            .expect("a cancelled run is still a run");

        // Two passes of the three the run offered, and the third never asked
        // for: the descent ended at a directory boundary rather than part way
        // through a directory.
        assert_eq!(run.agent.directories(), ["beta", "alpha"]);
        assert!(
            !run.lines
                .iter()
                .any(|line| line.contains("documenting .") || line == "warlock: documented ."),
            "the directory the run stopped before was announced: {:?}",
            run.lines
        );
        assert!(!repo.path().join("WARLOCK.md").exists());
        // What did finish is on disk and in the manifest, hashed and granted:
        // the whole point of stopping between directories rather than dying
        // where the key was pressed.
        for module in ["alpha", "beta"] {
            assert!(repo.path().join(module).join("WARLOCK.md").is_file());
        }
        let manifest = load_manifest(repo.path()).expect("a manifest that reads");
        assert_eq!(stored_modules(repo.path()), ["alpha", "beta"]);
        for module in ["alpha", "beta"] {
            assert!(
                manifest
                    .entry(module)
                    .expect("a documented directory is in the manifest")
                    .granted_hash()
                    .is_some(),
                "{module} was documented and not granted"
            );
        }

        // And the ending: one line, and the status a shell already reads as
        // interrupted.
        let (outcome, lines) = run.ended();
        let error = outcome
            .as_ref()
            .expect_err("a cancelled run is not a run that worked");
        assert!(
            matches!(error, Error::Cancelled),
            "the cancel was reported as something else: {error:?}"
        );
        assert!(lines.is_empty(), "a cancel named directories: {lines:?}");
        // The very value the subcommand hands `main`, not a rebuilt one.
        assert_eq!(status_for(outcome), 130);
        assert_eq!(
            run.stderr(),
            ["warlock: the run was cancelled; what it finished first is recorded"]
        );
    }

    #[test]
    fn a_cancelled_run_says_it_was_cancelled_rather_than_naming_the_pass_it_killed() {
        let repo = a_repository();
        let home = a_dir();

        // The shape a real Ctrl-C makes: the pass in flight is killed, so the
        // directory it was over fails the way any dead `claude` fails, and the
        // press that killed it is the reason.
        let run = run_cancelling(
            repo.path(),
            home.path(),
            Descent::Pact,
            ".",
            &["alpha"],
            "alpha",
        )
        .expect("a cancelled run is still a run");

        assert_eq!(run.agent.directories(), ["beta", "alpha"]);
        // There is a failure to report, and the run has the report — this is
        // not a run that happens to have nothing to say.
        assert_eq!(run.subtree.failures.len(), 1, "{:?}", run.subtree.failures);
        let report = run.report().expect("the killed pass failed its directory");
        assert!(report.lines[0].starts_with("alpha — "));

        // And it is not printed. Nothing in the engine's list says which
        // failures the reader caused, so naming them would send somebody to
        // look at a directory whose only problem was Ctrl-C. One line instead,
        // and `beta` is on stdout where it has been all along.
        let (outcome, lines) = run.ended();
        assert!(matches!(outcome, Err(Error::Cancelled)));
        assert!(lines.is_empty(), "a cancel named directories: {lines:?}");
        assert_eq!(run.stderr().len(), 1, "{:?}", run.stderr());
        assert!(run.lines.contains(&"warlock: documented beta".to_owned()));
        // The manifest still holds what the run earned before the press.
        assert_eq!(stored_modules(repo.path()), ["beta"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_manifest_that_will_not_save_is_the_one_warlock_could_not_do_rather_than_a_four() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = a_repository();
        let home = a_dir();
        // A manifest to read, and then a `.warlock/` nothing new can be created
        // in — so the run reads what it always reads and the save at the end of
        // it is the thing that fails.
        Manifest::new()
            .save(repo.path())
            .expect("a manifest that saves");
        let warlock = repo.path().join(".warlock");
        fs::set_permissions(&warlock, fs::Permissions::from_mode(0o555)).expect("chmods");
        if fs::write(warlock.join("probe"), "").is_ok() {
            // Running as root: no directory is unwritable, so there is nothing
            // here to assert against.
            fs::remove_file(warlock.join("probe")).expect("removes the probe");
            fs::set_permissions(&warlock, fs::Permissions::from_mode(0o755)).expect("chmods");
            return;
        }

        // Failures as well, so this is the case where the two endings compete.
        let refused = run_refusing(repo.path(), home.path(), Descent::Pact, ".", &["alpha"]);

        let error = refused.expect_err("the manifest could not be saved");
        assert!(
            matches!(error, Error::Manifest { .. }),
            "the save failure was reported as something else: {error:?}"
        );
        // 1, not 4: a run whose record never reached the disk is warlock unable
        // to do the thing, and that is the news rather than which directories
        // failed inside it.
        assert_eq!(status_for(&Err(error)), 1);

        fs::set_permissions(&warlock, fs::Permissions::from_mode(0o755)).expect("chmods back");
    }
}
