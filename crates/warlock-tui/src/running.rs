//! The headless run: `warlock pact <path>` and `warlock refresh <path>`.
//!
//! The seventh and eighth subcommands, and the first two that spend anything.
//! Everything before them is arithmetic — a listing walks a tree, a check walks
//! up a manifest, an un-pact and a scope write rewrite one file — and all of it
//! is over before the reader's hand leaves the keyboard. These two descend a
//! subtree, hand one `claude --print` per directory to a model, write a
//! `WARLOCK.md` beside each of them and save the manifest once at the end. A run
//! is minutes long, it costs tokens, and what it leaves behind is somebody's
//! prose rewritten.
//!
//! That is the whole reason this module exists as its own file rather than as
//! two more arms of [`mod@crate::edits`]. It is not that the work is bigger; it
//! is that the work is *spent*. Every rule below follows from it.
//!
//! # Nothing new happens here
//!
//! This module invents no operation. `pact` is
//! [`pact_subtree`](warlock_engine::pact_subtree) and `refresh` is
//! [`refresh_subtree`](warlock_engine::refresh_subtree), called with exactly the
//! arguments the `p` and `r` keys call them with, through exactly the agent the
//! `p` and `r` keys use — [`ClaudeAgent`], which is the one place in this crate
//! that spawns a process. Which directories a refresh describes is the engine's
//! judgement and is not re-decided, narrowed or widened here: it describes the
//! directories that are not fresh, and a front end holding a second opinion
//! about staleness would be a second answer waiting to disagree with the colour
//! the tree is drawn in.
//!
//! What this module *is*, then, is four things: the boundary, an observer that
//! prints, one save, and an exit status.
//!
//! # The boundary is asked first, and here that ordering is the money
//!
//! [`Opened`] is the gate, borrowed whole from [`mod@crate::edits`] rather than
//! written again — [`opened`] resolves the working directory, the repository
//! root, the manifest and this machine's sigils, and refuses at the covering
//! scope before it hands anything back. A run cannot begin without one, because
//! [`ran`] takes one and there is no other road to the engine call.
//!
//! The cheap writes ask the boundary first so that a refusal discloses nothing
//! about the inside of a manifest. This asks it first for that reason *and* for
//! a blunter one: past the gate, the next thing that happens is a model pass.
//! A boundary asked after the walk would have listed somebody else's directories
//! before refusing; asked after the first directory it would have spent a pass
//! and overwritten a `WARLOCK.md` that was not this machine's to touch, and no
//! exit status puts that back. So the refusal is one line on stderr, **exit
//! status 3**, no `claude` spawned, and `.warlock/pacts.toml` byte-identical to
//! what was read.
//!
//! The sentence is [`closed_scope_message`](crate::session::closed_scope_message)'s,
//! reached through [`Error::ClosedScope`] exactly as the un-pact reaches it, so
//! the footer, `warlock unpact` and `warlock pact` refuse one boundary in one
//! wording. There is no `--force`, no environment variable and no per-run flag
//! past it: `warlock config` is the one road, here as everywhere.
//!
//! # Progress is on stdout, and it is lines rather than a screen
//!
//! No terminal is entered, no raw mode, no alternate screen and no panic hook —
//! the hook exists to restore a terminal this path never takes. What a reader
//! watching a five-minute descent gets instead is [`Progress`]: one line as each
//! directory is entered, naming it and saying which of how many it is, and one
//! as it comes out documented. Directories are named relative to the repository
//! root, the way the manifest spells them, so a line of output and a line of
//! `.warlock/pacts.toml` say the same word about the same directory.
//!
//! The two announcements the engine makes *inside* a directory — the summarising
//! passes over over-cap files, and the request handed over — are deliberately
//! left silent. They are the TUI's, where there is a footer to overwrite ten
//! times a second and a panel to keep the history in; on a pipe they would be
//! tens of lines per directory competing with the two that say where the run has
//! got to. The default bodies on
//! [`Observer`](warlock_engine::PactObserver) are what makes not writing them
//! the same thing as saying nothing.
//!
//! # One save, at the end
//!
//! The engine saves nothing: [`pact_subtree`](warlock_engine::pact_subtree)
//! hands back the manifest that *should* be written and leaves the writing to
//! whoever asked. So [`ran`] writes it once, after the descent, through
//! [`Manifest::save`](warlock_engine::Manifest::save) — the same single write
//! the worker thread behind the `p` key performs. Once rather than per directory
//! because the file is written beside and renamed over: a save between every
//! pass would be N renames to record one event, and a reader watching the file
//! would see a manifest that claims a run finished while it is still going.
//!
//! # What the environment touches, and where
//!
//! In exactly one place, [`started`], which resolves the [`Opened`] and builds
//! the [`ClaudeAgent`] and the [`Progress`] writing to stdout. Everything under
//! it — [`ran`] — takes the repository, the agent and the observer as
//! parameters, which is the seam the tests below run through: a scratch
//! repository, a throwaway home, a fake agent and a `Vec<u8>` for stdout, so
//! nothing reads the developer's real home and nothing spawns `claude`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use warlock_engine::{
    Agent, PactObserver, PactedSubtree, Pacting, pact_subtree, refresh_subtree, to_manifest_path,
};
use warlock_tui::ClaudeAgent;

use crate::edits::{Opened, opened};
use crate::error::Error;

/// What a pact wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. Every other subcommand's tail is spelled
/// beside it, each where its subcommand is written.
const FOR_PACT: &str = "pact anything under";

/// What a refresh wants one for, in the same shape.
const FOR_REFRESH: &str = "refresh anything under";

/// Which of the two descents this is.
///
/// The whole of the difference between `warlock pact` and `warlock refresh`,
/// and it is two lines wide: which engine entry point [`ran`] calls, and what
/// the sentence about a missing `.git` says the root was wanted for. Everything
/// else — the gate, the agent, the observer, the save, the status — is one road
/// travelled twice, which is exactly the shape [`Work`](crate::pacting::Work)
/// gives the same pair of keys inside the TUI.
///
/// A carried value rather than two copies of the function, because the two
/// commands must not be able to drift into asking the boundary in a different
/// order or saving a different number of times. A [`Copy`] enum, because it is a
/// tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Descent {
    /// `warlock pact`: describe every directory of the subtree, whatever state
    /// it was in.
    Pact,
    /// `warlock refresh`: describe the ones the engine finds stale, and leave
    /// the fresh ones exactly as they are — grant, document and all.
    Refresh,
}

impl Descent {
    /// What this descent wanted a repository root for, as the tail of the
    /// sentence a missing `.git` is refused with.
    const fn wanted(self) -> &'static str {
        match self {
            Self::Pact => FOR_PACT,
            Self::Refresh => FOR_REFRESH,
        }
    }
}

/// The engine's progress port, writing lines.
///
/// The headless counterpart of [`Reporting`](crate::pacting), and a much smaller
/// thing than it: there is no channel, no thread and no screen, so where that
/// one forwards five kinds of event to an event loop that decides what to draw,
/// this one writes the two that a person watching a pipe can act on and answers
/// [`Pacting::Continue`] to everything.
///
/// It always continues, and today that is the only answer it has: nobody is
/// holding a key down. Stopping a headless run is the signal handler's, and it
/// arrives here later.
///
/// Generic over the writer rather than reaching for [`io::stdout`] itself, for
/// the reason every other seam in this crate is a parameter: the tests below
/// assert on the exact lines a run produces, in order, and a function that
/// printed could only be tested by spawning a process to read the output of.
///
/// A write that fails is ignored — the same shrug [`Reporting`](crate::pacting)
/// gives a send into a closed channel, for a stronger reason. A closed stdout is
/// `warlock pact . | head -1`, and failing a run of model passes because the
/// thing reading its progress went away would be spending minutes of somebody's
/// tokens on the state of a pipe. What the run leaves behind is the manifest and
/// the documents; the lines are a courtesy.
struct Progress<W: Write> {
    /// The repository root every directory is named against, so a line of
    /// output spells a directory the way `.warlock/pacts.toml` spells it.
    root: PathBuf,
    /// Where the lines go: stdout on the real road, a `Vec<u8>` under test.
    out: W,
}

impl<W: Write> Progress<W> {
    /// A port naming directories against `root` and writing to `out`.
    const fn new(root: PathBuf, out: W) -> Self {
        Self { root, out }
    }

    /// One line, in the shape every subcommand prints on stdout: `warlock: `
    /// and then the fact.
    fn say(&mut self, fact: &str) {
        // Ignored on purpose; see the type's doc.
        let _ = writeln!(self.out, "warlock: {fact}");
    }
}

impl<W: Write> PactObserver for Progress<W> {
    /// Say which directory is being entered and where it sits in the run, and
    /// let it go ahead.
    ///
    /// The fraction is the engine's own, unaltered and one-based, and its
    /// denominator does not move for the length of the run — so `[3/12]` is a
    /// thing a reader can watch rather than a running total that redefines
    /// itself. It is on this line and not the completion line because this is
    /// where it means something: it counts the directories offered, and the one
    /// being offered is the one it is about.
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        let named = named(&self.root, directory);
        self.say(&format!("[{position}/{total}] documenting {named}"));
        Pacting::Continue
    }

    /// Say that the directory came out documented.
    ///
    /// The engine's own word and the engine's own moment: this is announced when
    /// the directory's document *and* every document below it are written, which
    /// is the point at which nothing phase one can still do will take its grant
    /// away. A directory whose pass failed is never announced here, so the
    /// absence of a completion line under an entry line is itself the news —
    /// and naming that failure properly is the run's report rather than this
    /// port's.
    fn documented(&mut self, directory: &Path) {
        let named = named(&self.root, directory);
        self.say(&format!("documented {named}"));
    }
}

/// `directory` as the manifest spells it: relative to `root`, forward slashes,
/// and `.` for the root itself.
///
/// [`spelled`](crate::query::spelled)'s rule without its refusal. A query
/// refuses a path it cannot spell, because an answer with a directory quietly
/// left out would be a lie about that directory; a progress line is not an
/// answer, and a run that stopped part way through because one directory under
/// the root has a non-UTF-8 component in its name would be a run refused over
/// its own cosmetics. So an unspellable directory is named as it stands, which
/// is [`section_label`](crate::pacting)'s decision in the panel and is taken
/// here for the same reason: a line that says something odd beats one that says
/// nothing.
fn named(root: &Path, directory: &Path) -> String {
    to_manifest_path(root, directory).unwrap_or_else(|_| directory.display().to_string())
}

/// Descend the subtree past an open boundary, save the manifest once, and hand
/// back what happened.
///
/// The whole of `warlock pact` and `warlock refresh` below the gate, and it is
/// two engine calls and a save. `opened` is the proof the boundary was asked —
/// it cannot be built any other way — so this function does not ask it again and
/// there is no arrangement of the arguments in which it could be skipped.
///
/// The agent and the observer are parameters rather than reached for, which is
/// the seam this crate is built on: the tests below drive the real engine
/// operations over a real scratch repository with a hand-written fake in place
/// of the model, so what they assert is what a run does rather than what a mock
/// was told to say.
///
/// The manifest is saved once, here, after the descent, and it is saved whatever
/// the descent came to: a run in which some directories failed still earned the
/// grants of the ones that did not, and throwing them away would mean paying for
/// them again. What comes back is the [`PactedSubtree`] whole — the saved
/// manifest, the failures and the byte caps' problems — as a value rather than
/// as something printed, so that the report a run's failures deserve is a thing
/// a test can hold up rather than something only a subprocess could observe.
///
/// # Errors
///
/// [`Error::Pact`] when the subtree cannot be walked, which is the one thing
/// that fails a run as a whole and happens before any pass is spent; and
/// [`Error::Manifest`] when the manifest will not save, which happens after
/// every document is already on disk. Nothing else: everything else that goes
/// wrong goes wrong for one directory and comes back in
/// [`PactedSubtree::failures`].
fn ran(
    opened: &Opened,
    descent: Descent,
    agent: &dyn Agent,
    observer: &mut dyn PactObserver,
) -> Result<PactedSubtree, Error> {
    let (target, repo_root, manifest) = (opened.target(), opened.repo_root(), opened.manifest());
    let subtree = match descent {
        // Every directory in the subtree, whatever state it was in.
        Descent::Pact => pact_subtree(target, repo_root, manifest, agent, observer),
        // Only the stale ones, and which those are is the engine's judgement
        // from the same manifest handed in here: it keeps the grant of
        // everything it skipped, so a fresh directory costs no pass and loses
        // nothing.
        Descent::Refresh => refresh_subtree(target, repo_root, manifest, agent, observer),
    }
    .map_err(|source| Error::Pact { source })?;

    subtree
        .manifest
        .save(repo_root)
        .map_err(|source| Error::Manifest { source })?;
    Ok(subtree)
}

/// The subcommand: resolve the environment, ask the boundary, run, and print as
/// it goes.
///
/// The one place in this module that reads the working directory, the home
/// directory or stdout, kept to three lines so that everything worth testing is
/// underneath it in [`ran`]. [`opened`] is the resolution and the gate together
/// — see [`mod@crate::edits`] for why those are one step and not two — and a
/// closed boundary leaves through the `?` with nothing spawned and nothing
/// written.
///
/// [`ClaudeAgent::new`] is built here and nowhere else in this module: one
/// `claude --print` per directory, on the terms a pass is always asked on, from
/// the one file in this crate that spawns a process.
///
/// What the run came to is not read yet. A run whose failures are empty is the
/// case this slice is about — every directory documented, the manifest saved,
/// exit 0 — and naming the directories that failed, counting them and spending
/// an exit status on them is the report that comes next.
///
/// # Errors
///
/// Everything [`opened`] and [`ran`] refuse, unchanged and unwrapped: this adds
/// no sentence of its own.
fn started(descent: Descent, path: &Path) -> Result<(), Error> {
    let opened = opened(descent.wanted(), path)?;
    let mut progress = Progress::new(opened.repo_root().to_path_buf(), io::stdout());
    ran(&opened, descent, &ClaudeAgent::new(), &mut progress)?;
    Ok(())
}

/// `warlock pact <path>`: describe every directory at or below that one, and
/// save the manifest it earned.
///
/// One line, for the reason [`unpact`](crate::edits::unpact) is two: the whole
/// of the command is [`started`], and what tells it from the refresh below is
/// the tag it is handed.
///
/// # Errors
///
/// Everything [`started`] refuses, unchanged.
pub(crate) fn pact(path: &Path) -> Result<(), Error> {
    started(Descent::Pact, path)
}

/// `warlock refresh <path>`: describe the stale directories at or below that
/// one, leave the fresh ones alone, and save the manifest it earned.
///
/// # Errors
///
/// Everything [`started`] refuses, unchanged.
pub(crate) fn refresh(path: &Path) -> Result<(), Error> {
    started(Descent::Refresh, path)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::{
        Agent, AgentError, AgentRequest, AgentResponse, Manifest, PactEntry, PactedSubtree,
        manifest_path, save_sigils,
    };

    use super::{Descent, Progress, ran};
    use crate::edits::Opened;
    use crate::error::Error;
    // The sentence itself, asked of the one function that writes it rather than
    // retyped: the footer, `warlock unpact` and a refused run say one thing
    // about one boundary, and a test holding its own copy of those words would
    // go on passing while the doors drifted apart.
    use crate::session::{closed_scope_message, load_manifest};
    use crate::status_for;

    /// A throwaway directory. Every test below builds both its repository and
    /// its home out of these, so nothing reads the developer's real home and no
    /// leftover outlives the test.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// A repository with two directories of source under the root, and the
    /// `.git/` that makes it one.
    ///
    /// Three pactable directories, then: the root, `alpha` and `beta`. Small
    /// enough that a run over it is three passes of a fake, and shaped enough
    /// that "only the stale one" is a claim about a real sibling.
    fn a_repository() -> tempfile::TempDir {
        let repo = a_dir();
        write(repo.path(), ".git/HEAD", "ref: refs/heads/main\n");
        write(repo.path(), "alpha/lib.rs", "//! Alpha.\n");
        write(repo.path(), "beta/lib.rs", "//! Beta.\n");
        repo
    }

    /// Write `contents` at `relative` under `root`, making every directory above
    /// it. Directories are made by writing files into them, because a directory
    /// with nothing in it is not a thing this repository has.
    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("a file has a parent"))
            .expect("the directories above a file");
        fs::write(&path, contents).expect("a file");
    }

    /// Write `sigils` as what this machine holds for the repository at
    /// `repo_root`, under `home`.
    fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, repo_root, &sigils).expect("a config that writes");
    }

    /// Put a boundary on `module` and save it, so the tests below have one to
    /// be inside and outside of.
    ///
    /// An entry with a scope and no grant, which is the state
    /// `warlock scope add` leaves a directory nobody has pacted in: the
    /// boundary is a property of the entry, and a run's business with it is
    /// settled before anything is described.
    fn scoped(repo_root: &Path, module: &str, scope: &str) {
        Manifest::with_entries([PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
            .with_scope(scope)])
        .save(repo_root)
        .expect("a manifest that saves");
    }

    /// The bytes of the manifest on disk, or `None` when there is none.
    ///
    /// Bytes rather than a parsed [`Manifest`], because what a refusal promises
    /// is that the file did not change — not that it still parses to something
    /// equal.
    fn manifest_bytes(repo_root: &Path) -> Option<Vec<u8>> {
        fs::read(manifest_path(repo_root)).ok()
    }

    /// A model that always answers with a document long enough to be accepted,
    /// and remembers what it was asked.
    ///
    /// [`pacting`](crate::pacting)'s `Canned` with everything this module does
    /// not test taken out: no activities, because there is no panel; no cancel,
    /// because there is no key to press. What is kept is the pair of facts these
    /// tests turn on — which directories were offered a pass, in order, and
    /// whether a manifest was on disk while the passes were running, which is
    /// how "saved once, at the end" is asked of a run rather than of a mock.
    #[derive(Debug)]
    struct Canned {
        /// The root each remembered directory is named against.
        root: PathBuf,
        /// One entry per pass: the directory, and whether the manifest existed
        /// on disk at the moment the pass ran.
        seen: RefCell<Vec<(PathBuf, bool)>>,
    }

    impl Canned {
        /// A model that answers every pass, for a repository at `root`.
        fn new(root: &Path) -> Self {
            Self {
                root: root.to_path_buf(),
                seen: RefCell::new(Vec::new()),
            }
        }

        /// The directories a pass ran for, in call order, named from the root.
        fn directories(&self) -> Vec<String> {
            self.seen
                .borrow()
                .iter()
                .map(|(directory, _)| named(&self.root, directory))
                .collect()
        }

        /// Whether a manifest was on disk while any pass was running.
        fn saw_a_manifest(&self) -> bool {
            self.seen.borrow().iter().any(|(_, saved)| *saved)
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &AgentRequest) -> Result<AgentResponse, AgentError> {
            self.seen.borrow_mut().push((
                request.directory().to_path_buf(),
                manifest_path(&self.root).is_file(),
            ));
            Ok(AgentResponse::new(document()))
        }
    }

    /// A document long enough for the engine to accept. The rule is a byte
    /// count and nothing here reads what it says, so this is filler.
    fn document() -> String {
        format!("# module\n\n{}\n", "What it does, at length. ".repeat(20))
    }

    /// `directory` as the manifest spells it, for the assertions. The
    /// production spelling, asked of the production function.
    fn named(root: &Path, directory: &Path) -> String {
        super::named(root, directory)
    }

    /// What a run left behind: what the model was asked, and what the observer
    /// printed.
    #[derive(Debug)]
    struct Run {
        /// The subtree the engine handed back, saved.
        subtree: PactedSubtree,
        /// The model, with every pass it was given still in it.
        agent: Canned,
        /// The lines that would have gone to stdout, in order and without their
        /// newlines.
        lines: Vec<String>,
    }

    /// `warlock pact <path>` or `warlock refresh <path>` in the repository at
    /// `repo_root`, run by a machine whose sigils are under `home`, with
    /// everything the environment would have settled handed in instead.
    ///
    /// The production road exactly: the boundary through [`Opened::new`], the
    /// descent through [`ran`], in that order and with no way to reach the
    /// second without the first. What [`started`](super::started) adds on top of
    /// this is three lines — the working directory, the real home and
    /// [`ClaudeAgent`](warlock_tui::ClaudeAgent) — and they are exactly the
    /// three things a test must not have.
    fn run(repo_root: &Path, home: &Path, descent: Descent, path: &str) -> Result<Run, Error> {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        let opened = Opened::new(
            repo_root.to_path_buf(),
            Some(home),
            manifest,
            repo_root.join(path),
        )?;
        let agent = Canned::new(repo_root);
        let mut progress = Progress::new(repo_root.to_path_buf(), Vec::new());
        let subtree = ran(&opened, descent, &agent, &mut progress)?;
        let lines = String::from_utf8(progress.out)
            .expect("the lines warlock writes are its own text")
            .lines()
            .map(str::to_owned)
            .collect();
        Ok(Run {
            subtree,
            agent,
            lines,
        })
    }

    /// The modules the manifest on disk holds, in its own order.
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
}
