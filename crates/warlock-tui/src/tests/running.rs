use std::cell::RefCell;
use std::fs;
use std::iter;
use std::path::{Path, PathBuf};

use warlock_engine::{
    Agent, Manifest, PactEntry, PactedSubtree, agent, document, manifest_path, save_sigils,
    stub_answer,
};

use warlock_tui::Cancel;

use super::{Descent, Report, descended, named};
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

fn owned(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
    save_sigils(home, repo_root, &owned(sigils)).expect("a config that writes");
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
    blanked: Vec<String>,
    cancel_at: Option<(String, Cancel)>,
    seen: RefCell<Vec<(PathBuf, bool)>>,
}

impl Canned {
    fn refusing(root: &Path, refused: &[&str]) -> Self {
        Self {
            root: root.to_path_buf(),
            refused: owned(refused),
            blanked: Vec::new(),
            cancel_at: None,
            seen: RefCell::new(Vec::new()),
        }
    }

    // The other way an answer can be wrong, and the one this module has to
    // print: an object of the right shape with `purpose` blank. It is
    // answered that way on every attempt — the answer never changes, so
    // every repair pass finds the same slot empty — which runs the asking
    // out and leaves the engine to mend the fill itself rather than refuse
    // the directory. Built by blanking one field of the engine's own stub
    // answer, so the only thing wrong with it is the thing under test.
    fn blanking(mut self, directories: &[&str]) -> Self {
        self.blanked = owned(directories);
        self
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
        let module = named(&self.root, &directory);
        self.seen
            .borrow_mut()
            .push((directory, manifest_path(&self.root).is_file()));
        // Pressed while this pass is running, and the pass still answers:
        // the press beats the engine to the *next* directory rather than to
        // this one's answer. A real Ctrl-C kills the child as well, and the
        // pass it kills comes back as a failure — which is what `refused`
        // already produces, so a test that wants both asks for both.
        if let Some((at, cancel)) = &self.cancel_at
            && *at == module
        {
            cancel.cancel();
        }
        if self.refused.contains(&module) {
            return Err(agent::Error::NotFound {
                program: CLAUDE.to_owned(),
            });
        }
        let answer = stub_answer(request);
        if self.blanked.contains(&module) {
            return Ok(agent::Response::new(without_a_purpose(&answer)));
        }
        Ok(agent::Response::new(answer))
    }
}

// The engine's stub answer with one slot emptied. Through `serde_json`
// rather than by hand-writing an object, so this stays an answer of the
// shape the engine asked for however that shape moves, and the defect is
// the blank field alone.
fn without_a_purpose(answer: &str) -> String {
    let mut fill: serde_json::Value =
        serde_json::from_str(answer).expect("a document request is answered with JSON");
    fill["purpose"] = serde_json::Value::String(String::new());
    fill.to_string()
}

// The sentence the engine makes of the slot it had to fill in, named rather
// than retyped in three tests.
fn supplied(module: &str) -> String {
    format!(
        "warlock: {module} — purpose was not answered and was filled in from what warlock \
             measured"
    )
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

// Both kinds of wrong answer in one run: the directories that refuse
// outright, and the ones that answer with a blank slot until the asking
// runs out. `refused` empty is the repaired run on its own, which is the
// case the clean status is asked of.
fn run_repairing(
    repo_root: &Path,
    home: &Path,
    descent: Descent,
    path: &str,
    refused: &[&str],
    blanked: &[&str],
) -> Result<Run, Error> {
    let cancel = Cancel::new();
    let agent = Canned::refusing(repo_root, refused).blanking(blanked);
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

    // Two passes for a directory holding one file — the file's own, then
    // the synthesis over the line it came to — and one for the root, which
    // has no file of its own to pay for and is written from its children's
    // documents. One document beside each of the three.
    assert_eq!(
        run.agent.directories(),
        ["beta", "beta", "alpha", "alpha", "."]
    );
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
fn a_repaired_directory_says_what_was_mended_between_entering_it_and_documenting_it() {
    let repo = a_repository();
    let home = a_dir();

    // `alpha` answers with a blank `purpose` every time it is asked, so the
    // attempt loop runs out and the engine fills the slot in itself.
    let run = run_repairing(
        repo.path(),
        home.path(),
        Descent::Pact,
        ".",
        &[],
        &["alpha"],
    )
    .expect("nothing is scoped");

    // The exact run, in order. The repair sits between the line that
    // entered the directory and the line that finished it, because that is
    // when it happened; the two directories that answered properly say
    // nothing extra, so a reader can tell a repaired entry from a written
    // one by reading the log and nothing else.
    assert_eq!(
        run.lines,
        [
            "warlock: [1/3] documenting beta".to_owned(),
            "warlock: documented beta".to_owned(),
            "warlock: [2/3] documenting alpha".to_owned(),
            supplied("alpha"),
            "warlock: documented alpha".to_owned(),
            "warlock: [3/3] documenting .".to_owned(),
            "warlock: documented .".to_owned(),
        ]
    );
    // A mended directory is documented and granted like any other: the
    // repair cost the run passes, not its manifest.
    assert!(repo.path().join("alpha").join("WARLOCK.md").is_file());
    assert_eq!(stored_modules(repo.path()), [".", "alpha", "beta"]);
}

#[test]
fn a_run_that_only_repaired_leaves_with_the_clean_status_and_nothing_to_report() {
    let repo = a_repository();
    let home = a_dir();

    let run = run_repairing(
        repo.path(),
        home.path(),
        Descent::Pact,
        ".",
        &[],
        &["alpha"],
    )
    .expect("nothing is scoped");

    // There were repairs, so this is a real case rather than a clean run
    // asserted twice.
    assert!(
        !run.subtree.repairs.is_empty(),
        "nothing was mended, so the run this asserts about did not happen"
    );
    // A directory that answered badly enough to be mended is still a
    // directory that was described, so it costs the run a pass per attempt
    // and nothing else.
    assert!(
        run.subtree.failures.is_empty(),
        "{:?}",
        run.subtree.failures
    );
    // `alpha`'s file pass answers once and is taken; it is the synthesis
    // over that line that comes back blank, and that is asked once per
    // attempt until the asking runs out — so the count follows
    // `document::ATTEMPTS` rather than pinning it. The blank is a slot of
    // the directory's own fill, which is why the file pass never sees it.
    let mut offered = vec!["beta", "beta", "alpha"];
    offered.extend(iter::repeat_n("alpha", document::ATTEMPTS));
    offered.push(".");
    assert_eq!(run.agent.directories(), offered);
    // No report, nothing on stderr and the status of a run that worked: a
    // repair is not a thing to go and look at, so it is not in the list of
    // them.
    assert!(run.report().is_none());
    assert!(run.stderr().is_empty(), "{:?}", run.stderr());
    assert!(run.outcome.is_ok(), "{:?}", run.outcome);
    assert_eq!(status_for(&Ok(())), 0);
}

#[test]
fn a_run_that_failed_and_repaired_reports_the_failure_and_not_the_repair() {
    let repo = a_repository();
    let home = a_dir();

    // One directory refuses and another is mended, which is the run where
    // the two could be confused for each other.
    let run = run_repairing(
        repo.path(),
        home.path(),
        Descent::Pact,
        ".",
        &["alpha"],
        &["beta"],
    )
    .expect("a refused pass fails one directory, not the run");

    assert!(
        !run.subtree.repairs.is_empty(),
        "nothing was mended, so the run this asserts about did not happen"
    );
    let report = run.report().expect("one directory failed");
    // Only the failure is named, and the arithmetic is the arithmetic of
    // the same run without the repairs in it — a mended directory is
    // neither a failure nor an extra denominator.
    assert_eq!(report.lines.len(), 1, "{:?}", report.lines);
    assert!(
        report.lines[0].starts_with("alpha — "),
        "{:?}",
        report.lines[0]
    );
    assert_eq!((report.failed, report.total), (1, 3));
    assert_eq!(status_for(&Err(report.status())), 4);
    // And the repair is on stdout with the progress, where it belongs:
    // stderr is the list of directories to go and look at.
    assert!(
        !run.stderr().iter().any(|line| line.contains("purpose")),
        "a repair was reported as a failure: {:?}",
        run.stderr()
    );
    assert!(
        run.lines.contains(&supplied("beta")),
        "the repair went unannounced: {:?}",
        run.lines
    );
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

    assert_eq!(run.agent.directories(), ["alpha", "alpha"]);
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
    // unwidened: `alpha` costs no pass. `beta` costs two — the file that
    // moved, then the synthesis — and the root one, having no file of its
    // own.
    assert_eq!(run.agent.directories(), ["beta", "beta", "."]);
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
    // model answered have their documents. `alpha` is asked once and not
    // twice — its file pass is the one that refuses, and a refusal ends the
    // directory before there is anything to synthesise.
    assert_eq!(run.agent.directories(), ["beta", "beta", "alpha", "."]);
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

    // Two directories of the three the run offered, both paid for in full
    // — a file pass and a synthesis each — and the third never asked for:
    // the descent ended at a directory boundary rather than part way
    // through a directory.
    assert_eq!(run.agent.directories(), ["beta", "beta", "alpha", "alpha"]);
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

    // `alpha` once: the pass the cancel killed is its file pass, and the
    // synthesis it would have fed never ran.
    assert_eq!(run.agent.directories(), ["beta", "beta", "alpha"]);
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
