use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use warlock_engine::drafting::{Draft, stub_answer};
use warlock_engine::{
    Destination, Manifest, PactEntry, ScopeRecord, agent, save_key, save_key_binding, save_sigils,
};

use super::{Planned, Settled, cut_with, listing, listing_lines, prepare};

// Two of Forman's review words, spelled out in full so a test reads as the
// answer it gives.
const ACCEPT: &str = "create";

const SKIP: &str = "quit";
use crate::asking::Asks;
use crate::brief::ScopeBlockError;
use crate::claude::{
    Activities, Cancel, Converses, DRAFTING_CONTRACT, DRAFTING_ROUNDS, NOTHING_SETTLES_IT, Wired,
};
use crate::cut::noted;
use crate::error::Error;
use crate::error::status_for;
use crate::linear::{FetchedProject, Listing, Opens};
use crate::standing::Standing;
use crate::stubs::{Answering, Boarding, Call, Op, QueueAsked, Saying, Scripted, Typing, VIEWER};

// Not a key, and named so that nothing reading this file mistakes it for one.
// It is stored only so that a bound name resolves and a cut can reach the
// client, which is the one line that sees it.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "work";

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const SLUG: &str = "1a2b3c4d5e6f";

// The id `FetchedProject::new` gives the project it stands in for, which is what
// issues are created in and notes are said on.
const PROJECT_ID: &str = "project-1";

const MOVED: &str = "every slice is settled, so the project moved to `In Progress`";

const URL: &str = "https://linear.app/acme/project/draft-a-brief-1a2b3c";

const NAME: &str = "Cut a planned project into tickets";

const PLANNED: &str = "Planned";

// Three slices in document order, each waiting on the one before it, which is
// the ordinary shape a brief is written in.
const SLICED: &str = "Nothing cuts a planned project into tickets.\n\n## Scope\n\n\
                      ### 1. Read the project back\n\ndepends_on: []\n\n\
                      What it resolves.\n\n\
                      ### 2. Parse the scope block\n\ndepends_on: [1]\n\n\
                      What it parses.\n\n\
                      ### 3. File the drafts\n\ndepends_on: [2]\n\n\
                      What it writes.\n";

const FIRST: &str = "Read the project back";

const SECOND: &str = "Parse the scope block";

const THIRD: &str = "File the drafts";

// The same three, with the first waiting on the last: the cut order and the
// document order differ, which is the one thing `ordered` is for.
const OUT_OF_ORDER: &str = "Nothing cuts a planned project into tickets.\n\n## Scope\n\n\
                            ### 1. Read the project back\n\ndepends_on: [3]\n\n\
                            What it resolves.\n\n\
                            ### 2. Parse the scope block\n\ndepends_on: [3]\n\n\
                            What it parses.\n\n\
                            ### 3. File the drafts\n\ndepends_on: []\n\n\
                            What it writes.\n";

// Two slices waiting on each other, which is the one thing the parser will not
// guess its way out of.
const CIRCLE: &str = "Nothing cuts a planned project into tickets.\n\n## Scope\n\n\
                      ### 1. Read the project back\n\ndepends_on: [2]\n\n\
                      What it resolves.\n\n\
                      ### 2. Parse the scope block\n\ndepends_on: [1]\n\n\
                      What it parses.\n";

const NO_SCOPE: &str = "Nothing cuts a planned project into tickets.\n\n\
                        ## Out of scope\n\nEverything.\n";

// What a drafting session stops to ask about, in the tests that relay one.
const A_QUESTION: &str = "Which of the two spellings of the status is the gate?";

// Warlock's own attempt at it, out of the proposing conversation: offered over
// the prompt, and sent only when the line read is empty.
const PROPOSED: &str = "The board's own spelling, which is what the record names.";

// And what somebody types instead, which is the answer whatever it says.
const TYPED: &str = "Neither: the gate folds case.";

// What a review offers, less the slice it names: the two words that are answers,
// and the third that is whatever the reader has to say.
// Forman's review prompt, for a slice the stand-in drafts two tickets for.
const REVIEWED: &str = "[c]reate 2 ticket(s), [e]dit, [q]uit, or type feedback to redraft: ";

// Red's carry-on question after a skip.
const CARRY: &str = "carry on with the remaining slices? [y/N] ";

// A line that is none of the two words, which is feedback whatever it says.
const FEEDBACK: &str = "Two tickets is one too many; say it in one.";

// A model that answers every turn with the drafting road's own stub object,
// modelled on `stubs.rs`'s `Saying` and keeping what it was asked so that the
// contract a session opened with, and the number of turns it took, are
// assertions rather than readings.
//
// The answer names the turn it was given on, so one slice's two drafts are told
// apart from the next slice's on the board: nothing here reads the slice it was
// handed, which is exactly what a stand-in should not pretend to do.
//
// `Arc<Mutex<_>>` because `Wired` is `Send`: an agent is a thing the panel
// hands to a worker thread, and a stand-in that could not cross one would not
// stand in for it.
#[derive(Debug, Clone)]
struct Sketching {
    said: Arc<Mutex<Vec<String>>>,
    // Prose rather than the object, from the first turn on: what a session with
    // no rounds left does with an answer that is not JSON is ask again, and what
    // it does when every attempt is prose is hand back nothing usable.
    prose: bool,
}

impl Sketching {
    fn drafting() -> Self {
        Self {
            said: Arc::new(Mutex::new(Vec::new())),
            prose: false,
        }
    }

    fn talking() -> Self {
        Self {
            prose: true,
            ..Self::drafting()
        }
    }

    fn turns(&self) -> usize {
        self.said().len()
    }

    fn said(&self) -> Vec<String> {
        self.said
            .lock()
            .expect("a stand-in nothing poisoned")
            .clone()
    }
}

impl Wired for Sketching {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Sketching {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        let turn = {
            let mut said = self.said.lock().expect("a stand-in nothing poisoned");
            said.push(message.to_owned());
            said.len()
        };

        if self.prose {
            return Ok("I would rather talk about it first.".to_owned());
        }

        Ok(stub_answer(&format!("slice {turn}")))
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }

    fn fresh(&self) -> Self {
        self.clone()
    }
}

// The stdin nothing may read from. A refusal asks nothing, a dry run drafts
// nothing, and a model that never asks a question puts none to anybody — so
// being read is the failure rather than a flag, and no test holding this can
// block on a terminal.
#[derive(Debug, Clone, Copy)]
struct Unprompted;

impl Asks for Unprompted {
    fn ask(&mut self, prompt: &str) -> Result<Option<String>, Error> {
        panic!("a line was read from stdin at `{prompt}`");
    }
}

// A model no `claude` on the machine answers for: every turn is the failure a
// missing binary is, which is one of the three a session's turn can end in.
#[derive(Debug, Clone, Copy)]
struct Missing;

impl Wired for Missing {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        *self
    }
}

impl Converses for Missing {
    fn turn(&self, _message: &str) -> Result<String, agent::Error> {
        Err(agent::Error::NotFound {
            program: "claude".into(),
        })
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        *self
    }

    fn fresh(&self) -> Self {
        *self
    }
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn a_repository() -> TempDir {
    a_dir()
}

// A repository with a `[[scope]]` record for the board a cut reads back off:
// everything `resolve_filing` needs on the repository's side.
fn a_scoped_repository() -> TempDir {
    let repo = a_repository();
    saving(repo.path(), &a_manifest([a_record(SCOPE, TEAM)]));
    repo
}

fn a_manifest(scopes: impl IntoIterator<Item = ScopeRecord>) -> Manifest {
    Manifest::new().with_scopes(scopes)
}

// A manifest whose pact carries a scope with no `[[scope]]` record behind it,
// which is the third way there is no board: the manifest is one line short
// rather than the machine being wrong.
fn a_pacted_manifest() -> Manifest {
    Manifest::with_entries([PactEntry::new(".", "docs", "docs/WARLOCK.md")
        .expect("a relative module path is inside the root")
        .with_scope(SCOPE)])
}

fn a_record(name: &str, team: &str) -> ScopeRecord {
    ScopeRecord::new(name, team, "In Review", LABEL)
}

fn saving(root: &Path, manifest: &Manifest) {
    manifest.save(root).expect("a manifest that saves");
}

// The home of a machine that holds the sigil, has bound a name and stores a key
// under it: everything a cut needs, under a directory of this test's own, so
// that nothing here can reach the developer's real key store.
fn a_home(root: &Path) -> TempDir {
    a_home_holding(root, &[SCOPE])
}

// The same machine holding whichever sigils a test names, which is how a
// checkout standing at two boards is built: that is the ambiguity `--scope`
// settles, and it is a fact about the home rather than about the repository.
fn a_home_holding(root: &Path, sigils: &[&str]) -> TempDir {
    let home = a_dir();
    holding(home.path(), root, sigils);
    save_key_binding(home.path(), root, KEY_NAME).expect("a binding that writes");
    save_key(home.path(), KEY_NAME, NOT_A_KEY).expect("a key store that writes");
    home
}

fn holding(home: &Path, root: &Path, sigils: &[&str]) {
    let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
    save_sigils(home, root, &sigils).expect("a config that writes");
}

// A note as a cut leaves it on the project, which is the whole of what makes a
// slice cut for the next run.
fn a_cut(title: &str, issues: &[&str]) -> String {
    let issues: Vec<String> = issues.iter().map(|issue| format!("`{issue}`")).collect();
    format!("Warlock cut slice `{title}` into {}.", issues.join(", "))
}

fn a_skip(title: &str) -> String {
    format!("Warlock skipped slice `{title}`.")
}

// A planned project whose comments already hold these notes.
fn a_noted_project<const N: usize>(content: &str, notes: [String; N]) -> Boarding {
    Boarding::filing("")
        .reading(FetchedProject::new(NAME, content, URL, Some(PLANNED)).with_notes(notes))
}

// A workspace holding the one project the slug names, which answers everything
// a run then asks for as well.
fn a_project_of(content: &str, status: Option<&str>) -> Boarding {
    Boarding::filing("").reading(FetchedProject::new(NAME, content, URL, status))
}

// A planned project with slices in it, which is the one answer a whole cut
// gets before it starts drafting.
fn a_sliced_project(content: &str) -> Boarding {
    a_project_of(content, Some(PLANNED))
}

// The same slices in whatever status a case needs.
fn a_project(status: Option<&str>) -> Boarding {
    a_project_of(SLICED, status)
}

// What a refusal on the project's own state reads: the user the key belongs
// to, then the project the slug names. The queue is only asked for once there is
// a draft to show it to.
fn read_before_refusing() -> [Call; 2] {
    [Call::Viewer, Call::FetchProject(SLUG.to_owned())]
}

fn read_by_prepare() -> [Call; 3] {
    [
        Call::Viewer,
        Call::FetchProject(SLUG.to_owned()),
        Call::ScopeQueue(QueueAsked {
            team: TEAM.to_owned(),
            label: LABEL.to_owned(),
            assignee: VIEWER.to_owned(),
        }),
    ]
}

// The module's first step, less the environment: the repository root and the
// home are this test's temporary directories, and the seam is whatever `open`
// is. `None` is the scope every other test files under.
fn preparing<O: Opens>(
    repo: &Path,
    home: &Path,
    slug: &str,
    scope: Option<&str>,
    open: &O,
) -> Result<Planned, Error> {
    let standing = Standing::at(repo.to_path_buf(), repo.to_path_buf());
    prepare(
        &standing.manifest()?,
        repo,
        home,
        scope.unwrap_or(SCOPE),
        slug,
        open,
    )
}

// A cut that has to come to something, over the scoped repository and home a
// test has already built.
fn prepared(repo: &Path, home: &Path, linear: &Boarding) -> Planned {
    preparing(repo, home, SLUG, None, linear).expect("a draft to walk")
}

// The drafting road's own stub pair for a slice — one blocking the other — as
// the drafts rather than the answer that carries them: the module is handed
// drafts, and what a session said to produce them is the door's business.
fn drafts_for(title: &str) -> Vec<Draft> {
    let body = "A stand-in ticket body.".to_owned();
    vec![
        Draft {
            title: format!("Stand in for {title}"),
            body: body.clone(),
            blocked_by: Vec::new(),
            blocks: vec![1],
            waits_on: Vec::new(),
        },
        Draft {
            title: format!("Follow on from {title}"),
            body,
            blocked_by: vec![0],
            blocks: Vec::new(),
            waits_on: Vec::new(),
        },
    ]
}

// Every slice the cut has left, filed in the order the walk offers them and
// settled, as a door that says Create to everything would.
fn filing_each(planned: &mut Planned, linear: &Boarding) -> Vec<Settled> {
    let mut settled = Vec::new();
    while let Some(next) = planned.next_uncut() {
        let cut = planned
            .filing(&next, drafts_for(next.slice().heading()))
            .file(linear, &mut io::sink())
            .expect("a slice that files");
        settled.push(planned.settle(&next, cut));
    }
    settled
}

// The whole subcommand, less the environment: the repository root and the home
// are this test's temporary directories, the socket is whatever `open` is, the
// models are ones that panic when they are turned, and the stdin is one that
// panics when it is read.
fn cut_to<O: Opens>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    dry_run: bool,
    open: &O,
) -> (Result<(), Error>, String) {
    cutting(
        repo,
        home,
        path,
        scope,
        dry_run,
        open,
        &Scripted::saying([]),
        &Scripted::saying([]),
        &mut Unprompted,
    )
}

// Every review answered `accept`, which is what a run that files everything is
// told. More lines than any run here can reach, because a script that ran out
// would be EOF at a prompt — a skipped slice, which is the opposite of the
// filing these drive.
fn accepting() -> Typing {
    Typing::lines(vec![ACCEPT; 9])
}

// The whole subcommand with a model in it, which `cut_to` cannot drive: its
// agent panics when it is turned. Never a dry run — a dry run with a model in
// reach is the thing the reading tests are about. Nothing proposes, because a
// model that asks nothing puts no question to anybody and a run that reached
// that stand-in panics rather than passing; every slice's drafts are accepted,
// because nothing reaches the board until somebody says so.
fn cut_running<A: Converses>(
    repo: &Path,
    home: &Path,
    linear: &Boarding,
    agent: &A,
) -> (Result<(), Error>, String) {
    cutting(
        repo,
        home,
        SLUG,
        None,
        false,
        linear,
        agent,
        &Scripted::saying([]),
        &mut accepting(),
    )
}

// The same run with the reviews written down rather than all accepted, and still
// nothing proposing: what a test says `skip` or feedback through.
fn cut_reviewing<A: Converses, K: Asks>(
    repo: &Path,
    home: &Path,
    linear: &Boarding,
    agent: &A,
    ask: &mut K,
) -> (Result<(), Error>, String) {
    cutting(
        repo,
        home,
        SLUG,
        None,
        false,
        linear,
        agent,
        &Scripted::saying([]),
        ask,
    )
}

// The same run with a proposing model and a written-down line for every
// question and every review in it: the whole relay, driven with no `claude` on
// the machine and nothing attached to stdin.
fn cut_answering<A: Converses, P: Converses, K: Asks>(
    repo: &Path,
    home: &Path,
    linear: &Boarding,
    agent: &A,
    proposer: &P,
    ask: &mut K,
) -> (Result<(), Error>, String) {
    cutting(repo, home, SLUG, None, false, linear, agent, proposer, ask)
}

// The one call into the module, with everything the two drivers above differ
// over as parameters.
#[expect(
    clippy::too_many_arguments,
    reason = "the subcommand's own seams, one per parameter, so a test hands in \
              the stand-in it is about and panicking ones for the rest"
)]
fn cutting<O: Opens, A: Converses, P: Converses, K: Asks>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    dry_run: bool,
    open: &O,
    agent: &A,
    proposer: &P,
    ask: &mut K,
) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = cut_with(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        home,
        scope.unwrap_or(SCOPE),
        path,
        dry_run,
        open,
        agent,
        proposer,
        ask,
        &mut out,
        Activities::none,
    );

    (
        outcome,
        String::from_utf8(out).expect("warlock writes its own text"),
    )
}

// A whole run that has to come to something, with the lines it printed less
// their prefix.
fn cut_filing<A: Converses>(repo: &Path, home: &Path, linear: &Boarding, agent: &A) -> Vec<String> {
    let (outcome, printed) = cut_running(repo, home, linear, agent);

    outcome.expect("a run that files");
    lines(&printed)
}

// The slices the run noted on the project, as the folded title and the issues:
// the note is the product of a cut, so it is read back off what was said to the
// board rather than off anything the run handed over.
fn recorded(linear: &Boarding) -> Vec<(String, Vec<String>)> {
    let bodies: Vec<String> = linear
        .comments()
        .into_iter()
        .map(|(project, body)| {
            assert_eq!(project, PROJECT_ID, "a note went to another project");
            body
        })
        .collect();
    noted(&bodies)
}

fn folded(title: &str) -> String {
    crate::cut::fold_title(title)
}

// What every refusal here promises, checked in one place: the ordinary exit
// status rather than the boundary's, and one line to print.
fn refusal<T>(outcome: Result<T, Error>) -> Error {
    let outcome = outcome.map(drop);

    assert_eq!(status_for(&outcome), 1, "a draft refusal is the ordinary 1");
    assert_ne!(
        status_for(&outcome),
        3,
        "a draft refusal took the boundary's status"
    );

    let error = outcome.expect_err("a refusal");
    let message = error.to_string();
    assert!(!message.contains('\n'), "`main` prints one line: {message}");
    error
}

// A refusal out of the module's first step, with the seam whatever the case
// allows: a board refusal is handed one that cannot be opened, and one decided
// after the fetch is handed the stand-in that answered it.
fn refused<O: Opens>(repo: &Path, home: &Path, path: &str, scope: Option<&str>, open: &O) -> Error {
    refusal(preparing(repo, home, path, scope, open))
}

fn said(error: &Error) -> String {
    error.to_string()
}

// The lines off the writer, less the prefix every one of them carries: what the
// assertions below are about is what a reader is told, and `warlock: ` on the
// front of each is asserted once, here.
fn lines(printed: &str) -> Vec<String> {
    printed
        .lines()
        .map(|line| {
            line.strip_prefix("warlock: ")
                .unwrap_or_else(|| panic!("every line warlock prints is prefixed: {line}"))
                .to_owned()
        })
        .collect()
}

// Where a name sits in what was printed, so the cut order can be asserted as an
// order rather than as a set of lines that each happen to be present.
fn placed(lines: &[String], heading: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(heading))
        .unwrap_or_else(|| panic!("`{heading}` is in none of {lines:?}"))
}

// The module's interface, driven the way both doors drive it: `prepare`, a walk,
// and a filing per slice.
mod preparing {
    use super::*;

    #[test]
    fn a_project_is_fetched_by_the_slug_it_was_named_with() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Planned"));

        let planned = prepared(repo.path(), home.path(), &linear);

        assert_eq!(planned.name(), NAME);
        assert_eq!(planned.status(), "Planned");
        assert_eq!(planned.total(), 3);
        assert_eq!(planned.left(), 3);
        assert_eq!(planned.destination().team_key(), TEAM);
        // The slug as typed and no other selector, in one request, alongside
        // the one that resolved who the run files for.
        assert_eq!(linear.calls(), read_by_prepare());
        assert_eq!(linear.requests(), 3, "one call per operation");
        assert_eq!(linear.opened_with(), [NOT_A_KEY.to_owned()]);
        assert!(
            !format!("{planned:?}").contains(NOT_A_KEY),
            "the draft renders the key"
        );
    }

    #[test]
    fn a_slug_the_api_does_not_know_names_the_slug_and_the_command_that_lists_them() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(repo.path(), home.path(), SLUG, None, &Boarding::filing(""));

        assert!(
            matches!(&error, Error::UnknownProject { slug, scope } if slug == SLUG && scope == SCOPE),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains(SLUG), "{message}");
        assert!(
            message.contains("warlock draft warlock-team"),
            "the command that lists the slugs is not named: {message}"
        );
    }

    #[test]
    fn a_project_that_is_not_planned_names_the_status_it_found_and_the_one_it_wanted() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Backlog"));

        let error = refused(repo.path(), home.path(), SLUG, None, &linear);

        assert!(
            matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some("Backlog")),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains("Backlog"), "{message}");
        assert!(message.contains("Planned"), "{message}");
        // Nothing is read or sent after the status: the scope block in the
        // content that came back is never parsed, and no second request is
        // made.
        assert_eq!(linear.calls(), read_before_refusing());
    }

    #[test]
    fn a_project_with_no_status_at_all_is_refused_and_says_it_has_none() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(repo.path(), home.path(), SLUG, None, &a_project(None));

        assert!(
            matches!(&error, Error::NotPlanned { status: None, .. }),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains("no status"), "{message}");
        assert!(message.contains("Planned"), "{message}");
    }

    #[test]
    fn the_status_is_trimmed_and_case_folded_and_planned_is_the_only_spelling_accepted() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        for accepted in ["Planned", "planned", " Planned ", "PLANNED", "\tplanned\n"] {
            let planned = preparing(
                repo.path(),
                home.path(),
                SLUG,
                None,
                &a_project(Some(accepted)),
            )
            .unwrap_or_else(|error| panic!("`{accepted}` is `Planned`: {error:?}"));
            // The board's own spelling is what is reported, not this module's.
            assert_eq!(planned.status(), accepted);
        }

        for refused_as in [
            "Backlog",
            "In Progress",
            "Plan",
            "Planned later",
            "",
            "unplanned",
        ] {
            let error = refused(
                repo.path(),
                home.path(),
                SLUG,
                None,
                &a_project(Some(refused_as)),
            );

            assert!(
                matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some(refused_as)),
                "`{refused_as}` was read as `Planned`: {error:?}"
            );
        }
    }

    #[test]
    fn nothing_on_this_path_writes_to_the_board() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Planned"));

        prepared(repo.path(), home.path(), &linear);

        assert_eq!(
            linear.calls(),
            read_by_prepare(),
            "something other than the read was asked"
        );
    }

    #[test]
    fn a_project_whose_every_slice_is_noted_is_refused_naming_the_project() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(
            SLICED,
            [
                a_cut(FIRST, &["WAR-1"]),
                a_skip(SECOND),
                a_cut(THIRD, &["WAR-3"]),
            ],
        );

        let error = refused(repo.path(), home.path(), SLUG, None, &linear);

        assert!(
            matches!(&error, Error::AllCut { name } if name == NAME),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains(NAME), "{message}");
        assert!(message.contains("comments"), "{message}");
        // Refused where the answer that said so arrived: the fetch and nothing
        // after it.
        assert_eq!(linear.calls(), read_before_refusing());
    }

    #[test]
    fn a_note_is_matched_on_the_folded_title_and_a_cut_beats_a_skip() {
        // A brief edited only in spacing or case still finds its notes, and a
        // slice noted both ways counts as cut: its issues exist whatever was
        // said about it afterwards.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(
            SLICED,
            [
                a_skip(FIRST),
                a_cut("  READ the   project BACK ", &["WAR-7"]),
                "A person's comment about the project.".to_owned(),
            ],
        );

        let mut planned = prepared(repo.path(), home.path(), &linear);

        assert_eq!((planned.total(), planned.left()), (3, 2));
        let first = planned.next().expect("a first slice");
        assert_eq!(first.already(), Some(&["WAR-7".to_owned()][..]));
    }

    #[test]
    fn a_skip_note_is_settled_with_no_issues() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_skip(SECOND)]);

        let mut planned = prepared(repo.path(), home.path(), &linear);

        assert_eq!(planned.left(), 2);
        planned.next();
        let second = planned.next().expect("a second slice");
        assert_eq!(second.already(), Some(&[][..]));
    }

    #[test]
    fn a_project_with_no_scope_block_is_refused_in_the_parser_s_own_words() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            SLUG,
            None,
            &a_sliced_project(NO_SCOPE),
        );

        assert!(matches!(error, Error::ScopeBlock { .. }), "{error:?}");
        // The parser's sentence rather than one of this module's, which is what
        // carrying the source is for.
        assert_eq!(said(&error), ScopeBlockError::NoScope.to_string());
    }

    #[test]
    fn slices_that_wait_on_each_other_are_refused_with_both_of_them_named() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            SLUG,
            None,
            &a_sliced_project(CIRCLE),
        );

        assert!(matches!(error, Error::ScopeBlock { .. }), "{error:?}");
        let message = said(&error);
        assert!(
            message.contains(FIRST) && message.contains(SECOND),
            "{message}"
        );
        assert!(message.contains("wait on each other"), "{message}");
    }

    #[test]
    fn a_machine_with_no_board_to_cut_from_is_refused_before_anything_is_read() {
        // The three ways there is no candidate, which `push` refuses in the
        // same three sentences: this asserts they are that command's words and
        // not a second set worded here.
        let holds_nothing = a_scoped_repository();
        let nothing = a_dir();

        let unmatched = a_scoped_repository();
        let elsewhere = a_dir();
        holding(elsewhere.path(), unmatched.path(), &["billing"]);

        let unrecorded = a_repository();
        saving(unrecorded.path(), &a_pacted_manifest());
        let no_record = a_dir();
        holding(no_record.path(), unrecorded.path(), &[SCOPE]);

        for (repo, home, expected) in [
            (&holds_nothing, &nothing, "warlock config"),
            (&unmatched, &elsewhere, "billing"),
            (&unrecorded, &no_record, "[[scope]]"),
        ] {
            let error = refused(repo.path(), home.path(), SLUG, None, &Boarding::unopened());

            assert!(matches!(error, Error::Filing { .. }), "{error:?}");
            assert!(said(&error).contains(expected), "{}", said(&error));
        }
    }

    #[test]
    fn a_scope_that_is_a_candidate_is_honoured_and_one_that_is_not_names_the_candidates() {
        let repo = a_repository();
        saving(
            repo.path(),
            &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
        );
        let home = a_home_holding(repo.path(), &[SCOPE, "web"]);

        let planned = preparing(
            repo.path(),
            home.path(),
            SLUG,
            Some("web"),
            &a_sliced_project(SLICED),
        )
        .expect("a named candidate is a board");
        assert_eq!(planned.destination().team_key(), "WEB");
        assert_eq!(planned.destination().scope(), "web");

        // And a name that is not one of them is refused with both of them
        // named, with no key read and nothing sent.
        let error = refused(
            repo.path(),
            home.path(),
            SLUG,
            Some("billing"),
            &Boarding::unopened(),
        );

        let message = said(&error);
        assert!(matches!(error, Error::Filing { .. }), "{error:?}");
        assert!(message.contains("billing"), "{message}");
        assert!(
            message.contains(SCOPE) && message.contains("web"),
            "{message}"
        );
    }
}

mod listing {
    use super::*;

    fn destination() -> Destination {
        Destination::new(SCOPE, TEAM, LABEL, KEY_NAME)
    }

    #[test]
    fn the_planned_projects_are_asked_of_the_scope_s_team_and_label() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = Boarding::filing("").listing(Listing::new(&[(SLUG, NAME)]));
        let standing = Standing::at(repo.path().to_path_buf(), repo.path().to_path_buf());

        let (destination, listed) = listing(
            &standing.manifest().expect("a manifest"),
            repo.path(),
            home.path(),
            SCOPE,
            &linear,
        )
        .expect("a listing");

        assert_eq!(destination.team_key(), TEAM);
        assert_eq!(listed, Listing::new(&[(SLUG, NAME)]));
        assert_eq!(
            linear.calls(),
            [Call::PlannedProjects {
                team: TEAM.to_owned(),
                label: LABEL.to_owned(),
            }]
        );
    }

    #[test]
    fn a_scope_this_machine_cannot_file_to_is_refused_before_a_board_is_opened() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let standing = Standing::at(repo.path().to_path_buf(), repo.path().to_path_buf());

        let error = listing(
            &standing.manifest().expect("a manifest"),
            repo.path(),
            home.path(),
            "billing",
            &Boarding::unopened(),
        )
        .expect_err("a scope that is not a board");

        assert!(matches!(error, Error::Filing { .. }), "{error:?}");
    }

    #[test]
    fn each_project_is_its_slug_then_its_name_on_a_bare_line() {
        let lines = listing_lines(
            &destination(),
            &Listing::new(&[(SLUG, NAME), ("9e41c07a2b13", "Another one")]),
        );

        assert_eq!(
            lines,
            [
                format!("{SLUG}  {NAME}"),
                "9e41c07a2b13  Another one".to_owned()
            ]
        );
    }

    #[test]
    fn nothing_to_list_is_one_line_naming_the_team_and_the_label() {
        let lines = listing_lines(&destination(), &Listing::new(&[]));

        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(TEAM), "{}", lines[0]);
        assert!(lines[0].contains(LABEL), "{}", lines[0]);
        assert!(lines[0].contains("Planned"), "{}", lines[0]);
    }
}

mod walking {
    use super::*;

    #[test]
    fn every_slice_is_walked_in_the_cut_order_with_what_a_note_already_names() {
        // The last slice is what the first two wait on, and the first is
        // already cut: the walk is the order tickets would be filed in, the
        // fraction is the place in that order, and the position still finds the
        // slice in the document.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let mut planned = prepared(
            repo.path(),
            home.path(),
            &a_noted_project(OUT_OF_ORDER, [a_cut(FIRST, &["WAR-1", "WAR-2"])]),
        );

        let mut walked = Vec::new();
        while let Some(next) = planned.next() {
            walked.push((next.heading(), next.already().map(<[String]>::to_vec)));
        }

        assert_eq!(
            walked,
            [
                (format!("[1/3] slice 3 `{THIRD}`"), None),
                (
                    format!("[2/3] slice 1 `{FIRST}`"),
                    Some(vec!["WAR-1".to_owned(), "WAR-2".to_owned()])
                ),
                (format!("[3/3] slice 2 `{SECOND}`"), None),
            ]
        );
    }

    #[test]
    fn the_uncut_walk_counts_only_what_is_left_and_skips_what_is_cut() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let mut planned = prepared(
            repo.path(),
            home.path(),
            &a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1"])]),
        );
        assert_eq!((planned.total(), planned.left()), (3, 2));

        let mut walked = Vec::new();
        while let Some(next) = planned.next_uncut() {
            walked.push((next.heading(), next.left()));
        }

        assert_eq!(
            walked,
            [
                (format!("[1/2] slice 2 `{SECOND}`"), 1),
                (format!("[2/2] slice 3 `{THIRD}`"), 0),
            ]
        );
    }
}

mod filing {
    use super::*;

    #[test]
    fn a_slice_is_blocked_by_the_issues_the_slices_it_depends_on_became_this_run() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let settled = filing_each(&mut planned, &linear);

        assert_eq!(
            settled,
            [
                Settled {
                    issues: vec!["WAR-1".to_owned(), "WAR-2".to_owned()],
                    reported: Vec::new(),
                },
                Settled {
                    issues: vec!["WAR-3".to_owned(), "WAR-4".to_owned()],
                    reported: Vec::new(),
                },
                Settled {
                    issues: vec!["WAR-5".to_owned(), "WAR-6".to_owned()],
                    reported: vec![MOVED.to_owned()],
                },
            ]
        );
        let relations = linear.relations();
        // Every issue of the first slice blocks every issue of the second, and
        // the same again between the second and the third — beside the one edge
        // inside each slice that the drafts themselves asked for.
        for edge in [
            ("issue-1", "issue-2"),
            ("issue-1", "issue-3"),
            ("issue-1", "issue-4"),
            ("issue-2", "issue-3"),
            ("issue-2", "issue-4"),
            ("issue-3", "issue-5"),
            ("issue-4", "issue-6"),
        ] {
            assert!(
                relations.contains(&(edge.0.to_owned(), edge.1.to_owned())),
                "the edge {edge:?} is in none of {relations:?}"
            );
        }
        // And nothing across a slice nobody said it waits on.
        assert!(
            !relations.contains(&("issue-1".to_owned(), "issue-5".to_owned())),
            "{relations:?}"
        );
    }

    #[test]
    fn a_slice_waiting_on_one_that_was_already_cut_is_blocked_by_the_issues_its_note_names() {
        // The one thing a resumed cut does with a slice it cut last time: the
        // identifiers off the note are what the relation names, because a note
        // keeps nothing else of an issue.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1", "WAR-2"])]).numbering_from(3);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        filing_each(&mut planned, &linear);

        let relations = linear.relations();
        for blocker in ["WAR-1", "WAR-2"] {
            for waiting in ["issue-3", "issue-4"] {
                assert!(
                    relations.contains(&(blocker.to_owned(), waiting.to_owned())),
                    "`{blocker}` does not block `{waiting}` in {relations:?}"
                );
            }
        }
    }

    #[test]
    fn an_edge_the_api_turns_down_is_reported_beside_the_issues_it_did_not_take_down() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Relation, "the workspace would not");
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let settled = filing_each(&mut planned, &linear);

        let Settled { issues, reported } = &settled[0];
        assert_eq!(issues, &["WAR-1".to_owned(), "WAR-2".to_owned()]);
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert!(
            reported[0].contains("was not written as blocking")
                && reported[0].contains("the workspace would not"),
            "{:?}",
            reported[0]
        );
        assert_eq!(recorded(&linear).len(), 3);
    }

    #[test]
    fn every_slice_filed_is_one_note_naming_its_issues() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        filing_each(&mut planned, &linear);

        assert_eq!(
            linear
                .comments()
                .into_iter()
                .map(|(_, body)| body)
                .collect::<Vec<_>>(),
            [
                a_cut(FIRST, &["WAR-1", "WAR-2"]),
                a_cut(SECOND, &["WAR-3", "WAR-4"]),
                a_cut(THIRD, &["WAR-5", "WAR-6"]),
            ]
        );
    }

    #[test]
    fn a_skip_is_noted_on_the_project_by_its_own_title() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);
        let next = planned.next_uncut().expect("a slice to skip");

        let skipping = planned.skipping(&next);
        assert!(!format!("{skipping:?}").contains(NOT_A_KEY), "{skipping:?}");
        skipping.post(&linear).expect("a skip that is noted");

        assert_eq!(linear.comments(), [(PROJECT_ID.to_owned(), a_skip(FIRST))]);
    }

    #[test]
    fn no_key_value_is_in_a_filing() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);
        let next = planned.next_uncut().expect("a slice to file");

        let filing = planned.filing(&next, drafts_for(FIRST));
        assert!(!format!("{filing:?}").contains(NOT_A_KEY), "{filing:?}");
        let cut = filing
            .file(&linear, &mut io::sink())
            .expect("a slice that files");
        planned.settle(&next, cut);

        // The key reached the one place it is for: every board opened.
        assert!(
            linear.opened_with().iter().all(|key| key == NOT_A_KEY),
            "{:?}",
            linear.opened_with()
        );
    }
}

// `warlock draft` itself: the exit status, and what is printed.
mod headless {
    use super::*;

    #[test]
    fn a_refusal_prints_nothing_and_is_the_ordinary_exit_status() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Backlog"));

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, None, false, &linear);

        assert!(printed.is_empty(), "a refusal printed something: {printed}");
        let error = refusal(outcome);
        assert!(matches!(error, Error::NotPlanned { .. }), "{error:?}");
    }

    #[test]
    fn a_dry_run_reports_the_project_its_status_and_every_slice_then_stops() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, None, true, &linear);

        outcome.expect("a dry run answers");
        // The one request that got the project, and no second one: a dry run
        // reads what it reports and spends nothing else.
        assert_eq!(
            linear.calls(),
            read_by_prepare(),
            "a dry run sent more than the fetch"
        );

        let lines = lines(&printed);
        // One line for the run and one per slice, and nothing else.
        assert_eq!(lines.len(), 4, "{lines:?}");
        for said in [NAME, PLANNED, TEAM, SCOPE, "3 slices", "0 already cut"] {
            assert!(lines[0].contains(said), "{said} is not in: {}", lines[0]);
        }
        assert!(
            lines[0].contains("nothing was drafted"),
            "a dry run has to say it was one: {}",
            lines[0]
        );
        // The key value reaches the client and nothing else, here as
        // everywhere.
        assert!(!printed.contains(NOT_A_KEY), "{printed}");
    }

    #[test]
    fn a_dry_run_lists_the_slices_in_the_order_they_would_be_cut_in() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(OUT_OF_ORDER);

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, None, true, &linear);

        outcome.expect("a dry run answers");
        let lines = lines(&printed);
        assert!(
            placed(&lines, THIRD) < placed(&lines, FIRST),
            "the document order was printed rather than the cut order: {lines:?}"
        );
        assert!(placed(&lines, FIRST) < placed(&lines, SECOND), "{lines:?}");
        assert!(lines[1].contains("slice 3"), "{lines:?}");
        assert!(lines[1].contains("[1/3]"), "{lines:?}");
    }

    #[test]
    fn a_dry_run_reports_a_slice_a_note_already_names_with_the_issues_it_became() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1", "WAR-2"])]);

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, None, true, &linear);

        outcome.expect("two slices are left to cut");
        let lines = lines(&printed);
        assert!(lines[0].contains("1 already cut"), "{lines:?}");
        let cut = &lines[placed(&lines, FIRST)];
        assert!(cut.contains("already cut"), "{cut}");
        assert!(cut.contains("WAR-1") && cut.contains("WAR-2"), "{cut}");
        assert!(
            !lines[placed(&lines, SECOND)].contains("already cut"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_dry_run_under_a_named_scope_reports_that_board_and_not_the_other() {
        let repo = a_repository();
        saving(
            repo.path(),
            &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
        );
        let home = a_home_holding(repo.path(), &[SCOPE, "web"]);
        let linear = a_sliced_project(SLICED);

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, Some("web"), true, &linear);

        outcome.expect("a named candidate is a board");
        assert!(printed.contains("WEB"), "{printed}");
        assert!(!printed.contains(SCOPE), "{printed}");
    }

    #[test]
    fn a_whole_run_drafts_each_uncut_slice_in_the_cut_order_and_files_what_it_drafted() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = Sketching::drafting();

        let lines = cut_filing(repo.path(), home.path(), &linear, &agent);

        // Per slice: the walk's own line as it is drafted, the drafts that came
        // back by title and then whole, what can be said about them, and what
        // saying `accept` filed them as — in the order the slices are cut in and
        // with nothing else between them.
        let body = "A stand-in ticket body, written by a test double that read no repository \
                    and made no plan. It says what the slice said and nothing more.";
        let mut expected: Vec<String> = [(1, FIRST, 1, 2), (2, SECOND, 3, 4), (3, THIRD, 5, 6)]
            .into_iter()
            .flat_map(|(place, heading, first, second)| {
                [
                    format!("[{place}/3] slice {place} `{heading}` — drafting"),
                    format!(
                        "slice {place} `{heading}` — drafted `Stand in for slice {place}`, \
                         `Follow on from slice {place}`"
                    ),
                    format!("# slice {place} `{heading}`"),
                    String::new(),
                    format!("## 1. Stand in for slice {place}"),
                    String::new(),
                    body.to_owned(),
                    String::new(),
                    "Blocks #2".to_owned(),
                    String::new(),
                    format!("## 2. Follow on from slice {place}"),
                    String::new(),
                    body.to_owned(),
                    String::new(),
                    "Blocked by #1".to_owned(),
                    format!("cut `{heading}` into `WAR-{first}`, `WAR-{second}`"),
                ]
            })
            .collect();
        expected.push(MOVED.to_owned());
        assert_eq!(lines, expected, "{lines:?}");
        // One session per slice and one turn in each of them: this model asked
        // nothing, so nothing was put to anybody — the stand-ins for the
        // proposing model and for stdin both panic if they are reached.
        assert_eq!(agent.turns(), 3, "{:?}", agent.said());
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn every_slice_is_drafted_under_the_interactive_contract_and_carries_its_own_words() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = Sketching::drafting();

        cut_filing(repo.path(), home.path(), &linear, &agent);

        let said = agent.said();
        for (turn, heading) in said.iter().zip([FIRST, SECOND, THIRD]) {
            // The contract with somebody in front of it, and not the one-shot
            // one: there is a shell to put a question to, so the session is
            // told the rounds it has rather than told it has none.
            assert!(
                turn.contains(DRAFTING_CONTRACT),
                "a session opened without the interactive contract: {turn}"
            );
            // Its own slice and not the whole scope block: one session is aimed
            // at one slice, so the next slice's heading is nowhere in its
            // opening.
            assert!(turn.contains(heading), "{turn}");
        }
        assert!(!said[0].contains(SECOND), "{}", said[0]);
    }

    #[test]
    fn a_note_is_on_the_project_for_every_slice_the_run_filed() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        assert_eq!(
            recorded(&linear),
            [
                (folded(FIRST), vec!["WAR-1".to_owned(), "WAR-2".to_owned()]),
                (folded(SECOND), vec!["WAR-3".to_owned(), "WAR-4".to_owned()]),
                (folded(THIRD), vec!["WAR-5".to_owned(), "WAR-6".to_owned()]),
            ]
        );
    }

    #[test]
    fn a_run_that_dies_partway_keeps_the_note_of_every_slice_it_had_already_filed() {
        // The note is said after each slice and not at the end: two issues are
        // created and the third is turned down, and what is on the project
        // afterwards is the first slice — so the next run files the second and
        // third rather than all three again.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear =
            a_sliced_project(SLICED).refuse_from(Op::CreateIssue, 2, "that is enough issues");

        let (outcome, printed) =
            cut_running(repo.path(), home.path(), &linear, &Sketching::drafting());

        let error = refusal(outcome);
        assert!(matches!(error, Error::Linear { .. }), "{error:?}");
        assert_eq!(
            recorded(&linear),
            [(folded(FIRST), vec!["WAR-1".to_owned(), "WAR-2".to_owned()])]
        );
        // And what it did file was said before it stopped: the identifiers are
        // the one thing that must not be lost.
        assert!(lines(&printed).contains(&format!("cut `{FIRST}` into `WAR-1`, `WAR-2`")));
    }

    #[test]
    fn a_slice_a_note_already_names_is_said_and_nothing_at_all_is_sent_for_it() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let agent = Sketching::drafting();

        let lines = cut_filing(repo.path(), home.path(), &linear, &agent);

        assert_eq!(
            lines[0],
            format!(
                "[1/3] slice 1 `{FIRST}` — already cut as `WAR-1`, `WAR-2`, so nothing was sent"
            )
        );
        // Not drafted and not filed: two sessions for three slices, and nothing
        // on the board carrying the skipped slice's stand-in titles.
        assert_eq!(agent.turns(), 2, "{:?}", agent.said());
        assert_eq!(linear.issues_created().len(), 4);
        for said in agent.said() {
            assert!(
                !said.contains(FIRST),
                "the skipped slice was drafted: {said}"
            );
        }
        // And no second note for it: the run noted only what it filed.
        let noted: Vec<String> = recorded(&linear)
            .into_iter()
            .map(|(title, _)| title)
            .collect();
        assert_eq!(noted, [folded(SECOND), folded(THIRD)]);
    }

    #[test]
    fn a_team_with_no_backlog_state_refuses_the_run_with_nothing_created() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).without_backlog_state();

        let (outcome, _printed) =
            cut_running(repo.path(), home.path(), &linear, &Sketching::drafting());

        let error = refusal(outcome);
        assert!(
            matches!(&error, Error::NoBacklog { team } if team == TEAM),
            "{error:?}"
        );
        assert!(said(&error).contains(TEAM), "{}", said(&error));
        // Nothing on the board: the refusal happens while the slice is still
        // nothing rather than half filed.
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert!(linear.comments().is_empty(), "{:?}", linear.comments());
    }

    #[test]
    fn a_slice_that_never_parsed_is_reported_and_left_uncut_rather_than_filed_as_a_stand_in() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        // A model that only ever talks spends its three rounds as questions and
        // is then out of them, and what it says after that is an attempt that
        // never parsed: one answer per question per slice, and the run reaches
        // the same ending it always did.
        let mut typing = Typing::lines(vec!["say more"; DRAFTING_ROUNDS * 3]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &Sketching::talking(),
            &Saying::answering(PROPOSED),
            &mut typing,
        );

        outcome.expect("a run that files nothing still answers");
        let lines = lines(&printed);
        for (place, heading) in [(1, FIRST), (2, SECOND), (3, THIRD)] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(heading) && line.contains("was not drafted")),
                "slice {place} `{heading}` was not reported: {lines:?}"
            );
        }
        // Nothing was created and nothing was noted: a run that filed nothing
        // has nothing to say on the project.
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert!(linear.comments().is_empty(), "{:?}", linear.comments());
    }

    #[test]
    fn a_session_whose_turn_fails_is_a_line_and_the_next_slice_is_still_cut() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        let lines = cut_filing(repo.path(), home.path(), &linear, &Missing);

        assert!(
            lines.iter().any(|line| line.contains("was not drafted")),
            "{lines:?}"
        );
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
    }

    #[test]
    fn each_slice_is_noted_after_its_own_issues_and_before_the_next_slice_s() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        let notes = linear.positions_of(Op::Comment);
        let creates = linear.positions_of(Op::CreateIssue);
        assert_eq!(notes.len(), 3, "{:?}", linear.ops());
        for (slice, note) in notes.iter().enumerate() {
            assert!(*note > creates[slice * 2 + 1], "{:?}", linear.ops());
            if let Some(next) = creates.get(slice * 2 + 2) {
                assert!(note < next, "{:?}", linear.ops());
            }
        }
    }

    #[test]
    fn a_note_the_api_turns_down_stops_the_run_naming_the_issues_it_left_unnoted() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Comment, "the workspace would not");

        let (outcome, printed) =
            cut_running(repo.path(), home.path(), &linear, &Sketching::drafting());

        let error = refusal(outcome);
        assert!(
            matches!(&error, Error::Uncut { issues, .. } if issues == &["WAR-1", "WAR-2"]),
            "{error:?}"
        );
        assert!(said(&error).contains("the workspace would not"), "{error}");
        assert!(lines(&printed).contains(&format!("cut `{FIRST}` into `WAR-1`, `WAR-2`")));
        assert_eq!(linear.issues_created().len(), 2, "the run went on");
    }

    #[test]
    fn an_edge_the_api_turns_down_is_a_printed_line_and_does_not_fail_the_slice() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Relation, "the workspace would not");

        let lines = cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        assert!(
            lines
                .iter()
                .any(|line| line.contains("was not written as blocking")),
            "{lines:?}"
        );
        assert_eq!(recorded(&linear).len(), 3);
    }

    #[test]
    fn nothing_the_run_sends_writes_a_field_warlock_would_have_to_invent() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        // An issue create has no field beyond the seven a draft, its slice and
        // the run's own viewer resolve: what it puts on the wire is held by
        // `linear.rs`'s own tests. What a run can still get wrong is asking for
        // something a cut has no business asking, and the one status it moves
        // is the project's, once.
        for op in linear.ops() {
            assert!(
                matches!(
                    op,
                    Op::Viewer
                        | Op::FetchProject
                        | Op::ScopeQueue
                        | Op::Team
                        | Op::BacklogState
                        | Op::IssueLabel
                        | Op::CreateIssue
                        | Op::Relation
                        | Op::Comment
                        | Op::ProjectStatus
                        | Op::MoveProject
                ),
                "a run asked for {op:?}"
            );
        }
        assert_eq!(
            linear.moves(),
            [(PROJECT_ID.to_owned(), "status-in-progress".to_owned())]
        );
    }

    #[test]
    fn the_whole_run_asks_once_who_it_files_for_and_asks_before_the_first_issue() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        // Three slices with two drafts in each: six issues, and one answer about
        // who they are all for. A request per slice — or per draft — would be
        // bought again for an answer that cannot have changed.
        assert_eq!(linear.issues_created().len(), 6);
        let asked = linear.positions_of(Op::Viewer);
        assert_eq!(asked.len(), 1, "{:?}", linear.ops());
        // And before anything exists to assign: the id is resolved while the run
        // is still nothing, not found out once issues are on the board.
        let first = linear.positions_of(Op::CreateIssue)[0];
        assert!(
            asked[0] < first,
            "the viewer was asked at {} and the first issue created at {first}",
            asked[0]
        );
    }

    #[test]
    fn every_issue_the_run_files_is_assigned_to_the_user_the_viewer_answered() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        // One id for the run, the one the board said the key belongs to: the
        // slices are cut one at a time and the assignee is not a thing a later
        // slice can drift on.
        let issues = linear.issues_created();
        assert_eq!(issues.len(), 6, "{issues:?}");
        for issue in &issues {
            assert_eq!(issue.assignee, VIEWER, "{issue:?}");
        }
    }

    #[test]
    fn a_viewer_request_the_api_turns_down_refuses_the_run_before_anything_exists() {
        // A board that will not say who the key belongs to has nothing to file
        // for. A timeout is the same failure by the same road — one `LinearError`
        // through the one line that maps it — so the refusal stands for both.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Viewer, "the workspace would not");

        let (outcome, printed) = cut_to(repo.path(), home.path(), SLUG, None, false, &linear);

        let error = refusal(outcome);
        assert!(matches!(error, Error::Linear { .. }), "{error:?}");
        assert!(
            said(&error).contains("the workspace would not"),
            "{}",
            said(&error)
        );
        // The one request and nothing after it: the project was never even
        // fetched, no session was opened — the model panics if one is — and
        // nothing was created.
        assert_eq!(linear.calls(), [Call::Viewer], "{:?}", linear.calls());
        assert!(printed.is_empty(), "a refusal printed something: {printed}");
        assert!(recorded(&linear).is_empty(), "a refusal noted a cut");
    }
}

// The question a slice asks, put to whoever ran `warlock draft` and answered at
// the prompt. Every one of these drives the whole subcommand: a scripted model
// that asks, a scripted proposing model for warlock's own attempt, and a
// written-down line for the read — so no test here reaches a `claude`, a
// terminal or the stdin of whatever ran the suite.
mod relaying {
    use super::*;

    // A model that asks once on the first slice and then drafts every slice:
    // the shortest run that puts a question to the shell. A turn it was not
    // scripted for panics, so the number of sessions and turns is asserted by
    // the script rather than counted afterwards.
    fn asking_once() -> Scripted {
        Scripted::saying([
            Answering::says(A_QUESTION),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
    }

    // The same first question, with the slice it was asked about left uncut:
    // the two slices after it are the ones that draft.
    fn asking_and_abandoned() -> Scripted {
        Scripted::saying([
            Answering::says(A_QUESTION),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
    }

    #[test]
    fn a_question_with_lettered_options_is_printed_a_line_per_option() {
        // The contract asks for the options on lines of their own; flattening
        // them into one line would bury the choice being offered.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = Scripted::saying([
            Answering::says(
                "The brief does not say where the gate lives.\n\n\
                 a) one ticket that adds it to the walk\n\
                 b) two tickets, the gate first\n\n\
                 Or tell me something else.",
            ),
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let mut typing = Typing::lines(["", ACCEPT, ACCEPT, ACCEPT]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Saying::answering(PROPOSED),
            &mut typing,
        );

        outcome.expect("a run that files");
        let lines = lines(&printed);
        let asked = placed(&lines, "asked:");
        assert_eq!(
            lines[asked..asked + 4],
            [
                format!("slice 1 `{FIRST}` asked: The brief does not say where the gate lives."),
                "a) one ticket that adds it to the walk".to_owned(),
                "b) two tickets, the gate first".to_owned(),
                "Or tell me something else.".to_owned(),
            ],
            "{lines:?}"
        );
    }

    #[test]
    fn a_question_is_put_to_the_shell_with_warlocks_own_answer_over_the_prompt() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_once();
        // Enter at the question, which is the proposal accepted, and then the
        // three reviews the three slices' drafts come up for.
        let mut typing = Typing::lines(["", ACCEPT, ACCEPT, ACCEPT]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Saying::answering(PROPOSED),
            &mut typing,
        );

        outcome.expect("a run that files");
        let lines = lines(&printed);
        // The question in the words it was asked, and the slice named the way
        // every other line about a slice names it.
        let asked = &lines[placed(&lines, "asked:")];
        assert!(asked.contains(A_QUESTION), "{asked}");
        assert!(asked.contains(&format!("slice 1 `{FIRST}`")), "{asked}");
        // And warlock's own attempt at it, over the prompt.
        let offered = &lines[placed(&lines, "warlock's answer:")];
        assert!(offered.contains(PROPOSED), "{offered}");
        assert!(offered.contains(&format!("slice 1 `{FIRST}`")), "{offered}");
        assert!(
            placed(&lines, "asked:") < placed(&lines, "warlock's answer:"),
            "the answer was offered before the question was put: {lines:?}"
        );
        // The cursor stopped at the question and at each of the three reviews,
        // on the same bare mark every time: what is being answered is on the
        // lines above it rather than in the prompt.
        assert_eq!(
            typing.asked(),
            ["> ", REVIEWED, REVIEWED, REVIEWED],
            "{:?}",
            typing.asked()
        );
        // And type-ahead thrown away before each of those four, since a model
        // had just been working before every one of them.
        assert_eq!(typing.discards(), 4);
        // What reached the session is the proposal, in the words it was
        // offered in, and it is on the thread as what was sent.
        assert!(
            agent.said().iter().any(|said| said == PROPOSED),
            "{:?}",
            agent.said()
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("was answered") && line.contains(PROPOSED)),
            "{lines:?}"
        );
        // A question is a pause and not an ending: every slice was cut.
        assert_eq!(linear.issues_created().len(), 6);
        // The key value reaches the client and nothing else, here as
        // everywhere.
        assert!(!printed.contains(NOT_A_KEY), "{printed}");
    }

    #[test]
    fn a_line_somebody_types_is_what_reaches_the_session_and_the_proposal_is_not_sent() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_once();
        // As a pipe hands it over, newline and all: what is sent is what was
        // typed and nothing of how it arrived. The three reviews after it are
        // accepted, so the run files what it drafted.
        let mut typing = Typing::lines([
            format!("{TYPED}\n"),
            ACCEPT.to_owned(),
            ACCEPT.to_owned(),
            ACCEPT.to_owned(),
        ]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Saying::answering(PROPOSED),
            &mut typing,
        );

        outcome.expect("a run that files");
        assert!(
            agent.said().iter().any(|said| said == TYPED),
            "{:?}",
            agent.said()
        );
        for said in agent.said() {
            assert!(
                !said.contains(PROPOSED),
                "warlock's own answer was sent over a typed one: {said}"
            );
        }
        let lines = lines(&printed);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("was answered") && line.contains(TYPED)),
            "{lines:?}"
        );
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn a_question_nothing_settles_says_so_and_the_line_typed_is_still_read() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_once();
        let mut typing = Typing::lines([TYPED, ACCEPT, ACCEPT, ACCEPT]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            // The sentence the proposing session hands back when the brief, the
            // slice and the repository do not settle the question.
            &Saying::answering(NOTHING_SETTLES_IT),
            &mut typing,
        );

        outcome.expect("a run that files");
        let lines = lines(&printed);
        let settled = &lines[placed(&lines, NOTHING_SETTLES_IT)];
        assert!(settled.contains(&format!("slice 1 `{FIRST}`")), "{settled}");
        // Nothing was offered as an answer, and the read happened anyway.
        assert!(
            !printed.contains("warlock's answer:"),
            "a refusal was offered as an answer: {printed}"
        );
        assert_eq!(
            typing.asked(),
            ["> ", REVIEWED, REVIEWED, REVIEWED],
            "{:?}",
            typing.asked()
        );
        assert!(
            agent.said().iter().any(|said| said == TYPED),
            "{:?}",
            agent.said()
        );
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn a_proposal_that_never_came_back_says_so_and_the_line_typed_is_still_read() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_once();
        let mut typing = Typing::lines([TYPED, ACCEPT, ACCEPT, ACCEPT]);

        // No `claude` for the proposing conversation, which is one of the three
        // ways that attempt ends in a failure rather than an answer. The
        // slice's own session is untouched by it.
        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Missing,
            &mut typing,
        );

        outcome.expect("a run that files");
        let lines = lines(&printed);
        let unproposed = &lines[placed(&lines, "no answer was proposed")];
        assert!(
            unproposed.contains(&format!("slice 1 `{FIRST}`")),
            "{unproposed}"
        );
        assert_eq!(
            typing.asked(),
            ["> ", REVIEWED, REVIEWED, REVIEWED],
            "{:?}",
            typing.asked()
        );
        assert!(
            agent.said().iter().any(|said| said == TYPED),
            "{:?}",
            agent.said()
        );
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn enter_at_a_question_with_nothing_proposed_leaves_the_slice_uncut() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_and_abandoned();
        // Enter at the question, and then the two reviews the slices that did
        // draft come up for: the slice this is about never reaches one.
        let mut typing = Typing::lines(["", ACCEPT, ACCEPT]);

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Saying::answering(NOTHING_SETTLES_IT),
            &mut typing,
        );

        outcome.expect("a slice that came to nothing is a line and the next slice");
        let lines = lines(&printed);
        let uncut = &lines[placed(&lines, "was not drafted")];
        assert!(uncut.contains(&format!("slice 1 `{FIRST}`")), "{uncut}");
        assert!(uncut.contains("nothing was typed"), "{uncut}");
        // The slice is uncut and the two after it were cut: nothing was filed
        // for it and no note names it, so the next run offers it again.
        assert_eq!(linear.issues_created().len(), 4);
        for (title, _) in recorded(&linear) {
            assert_ne!(title, folded(FIRST), "the uncut slice was noted");
        }
    }

    #[test]
    fn a_pipe_with_nothing_in_it_leaves_every_slice_uncut_and_sends_nothing_in_its_place() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = asking_and_abandoned();
        // EOF at every read: nobody is there. A read waits for a line rather
        // than deciding anything, so this is the one thing that ends the
        // waiting without one — at the question the first slice asked, and at
        // the review the two that drafted came up for.
        let mut typing = Typing::nothing();

        let (outcome, printed) = cut_answering(
            repo.path(),
            home.path(),
            &linear,
            &agent,
            &Saying::answering(PROPOSED),
            &mut typing,
        );

        outcome.expect("a slice that came to nothing is a line and the next slice");
        assert!(
            printed.contains("nobody answered its question"),
            "{printed}"
        );
        assert!(printed.contains(&format!("slice 1 `{FIRST}`")), "{printed}");
        // Warlock's own attempt was offered and never sent: an unanswered
        // question is not a licence to answer it.
        for said in agent.said() {
            assert!(
                !said.contains(PROPOSED),
                "warlock answered its own question: {said}"
            );
        }
        // And the two that did draft were left uncut as well: a pipe that ends
        // at the review is nobody saying what to do with the drafts, which files
        // nothing.
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert!(recorded(&linear).is_empty());
    }
}

// One slice's drafts put up before anything is sent, and what the line read
// makes of them. Every one of these drives the whole subcommand off a
// written-down script, so no test here reaches a `claude`, a terminal or the
// stdin of whatever ran the suite.
mod reviewing {
    use super::*;

    // What the second drafts of a redrafted slice are named for, so the tickets
    // that were filed are told from the ones that were thought better of.
    const AGAIN: &str = "Read the project back, in one ticket";

    // What a reader is told one slice's drafts are, in the titles the drafting
    // road's own stub gives them.
    fn drafted(position: usize, heading: &str, stub: &str) -> String {
        format!(
            "slice {position} `{heading}` — drafted `Stand in for {stub}`, \
             `Follow on from {stub}`"
        )
    }

    // A model that drafts every slice and asks nothing: the shortest run that
    // puts three reviews to the shell and no question at all. A turn it was not
    // scripted for panics, so the number of sessions is asserted by the script
    // rather than counted afterwards.
    fn drafting_each() -> Scripted {
        Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
    }

    // The whole run over that model, with the reviews written down.
    fn reviewing(repo: &Path, home: &Path, linear: &Boarding, ask: &mut Typing) -> String {
        let (outcome, printed) = cut_reviewing(repo, home, linear, &drafting_each(), ask);

        outcome.expect("a reviewed slice is a line and the next slice");
        printed
    }

    #[test]
    fn the_drafts_are_printed_and_a_line_is_read_before_the_slice_is_filed() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::lines([ACCEPT, ACCEPT, ACCEPT]);

        let printed = reviewing(repo.path(), home.path(), &linear, &mut typing);

        let lines = lines(&printed);
        // The drafts by title, then whole, then the filing: the order is the
        // promise, because a reader answers about drafts they have been shown.
        let shown = placed(&lines, &drafted(1, FIRST, FIRST));
        let offered = placed(&lines, &format!("cut `{FIRST}` into"));
        // And whole: every draft numbered with its body and its edges by those
        // numbers, between the titles and the prompt, so what is accepted is
        // what was read.
        let first = placed(&lines, &format!("## 1. Stand in for {FIRST}"));
        let second = placed(&lines, &format!("## 2. Follow on from {FIRST}"));
        assert!(
            shown < first && first < second && second < offered,
            "{lines:?}"
        );
        assert!(
            lines[first..second]
                .iter()
                .any(|line| line.starts_with("A stand-in ticket body")),
            "{lines:?}"
        );
        assert!(
            lines[first..second].iter().any(|line| line == "Blocks #2"),
            "{lines:?}"
        );
        assert!(
            lines[second..offered]
                .iter()
                .any(|line| line == "Blocked by #1"),
            "{lines:?}"
        );
        // One read per slice, at Forman's own prompt.
        assert_eq!(typing.asked(), [REVIEWED; 3], "{:?}", typing.asked());
        assert_eq!(typing.discards(), 3);
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn skip_leaves_the_slice_uncut_and_the_run_goes_on_to_the_next_one() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::lines([SKIP, "y", ACCEPT, ACCEPT]);

        let printed = reviewing(repo.path(), home.path(), &linear, &mut typing);

        let lines = lines(&printed);
        let skipped = &lines[placed(&lines, "was skipped")];
        assert!(skipped.contains(&format!("slice 1 `{FIRST}`")), "{skipped}");
        assert!(skipped.contains("nothing was created for it"), "{skipped}");
        // Red's question, asked once, after the skip and before the next slice.
        assert_eq!(
            typing.asked(),
            [REVIEWED, CARRY, REVIEWED, REVIEWED],
            "{:?}",
            typing.asked()
        );
        // The two after it were drafted and filed, and nothing of the skipped
        // slice reached the board.
        assert_eq!(linear.issues_created().len(), 4);
        for issue in linear.issues_created() {
            assert!(!issue.title.contains(FIRST), "{issue:?}");
        }
    }

    #[test]
    fn an_empty_line_files_the_drafts_as_forman_reads_it() {
        // Forman's `parse_decision`: somebody who has read the drafts and
        // pressed Enter has agreed with them, and a line of spaces is the same
        // line.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::lines(["", "   ", "Y"]);

        let printed = reviewing(repo.path(), home.path(), &linear, &mut typing);

        assert!(!printed.contains("was skipped"), "{printed}");
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn a_no_at_the_carry_on_question_stops_the_run_and_leaves_the_rest() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        // Red's default: an empty answer is no.
        let mut typing = Typing::lines([SKIP, ""]);

        let printed = reviewing(repo.path(), home.path(), &linear, &mut typing);

        assert!(
            printed.contains("the run stopped; 2 slices left"),
            "{printed}"
        );
        assert!(linear.issues_created().is_empty());
        assert_eq!(recorded(&linear), [(folded(FIRST), Vec::<String>::new())]);
    }

    #[test]
    fn a_skipped_slice_costs_nothing_but_its_note() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::lines([SKIP, "y", SKIP, "y", SKIP]);

        reviewing(repo.path(), home.path(), &linear, &mut typing);

        // The reads `prepare` makes and one note per skip: the review is asked
        // before a filing is built at all, so a slice nobody accepted reaches no
        // issue create and no edge.
        let mut expected = read_by_prepare().to_vec();
        expected.extend([FIRST, SECOND, THIRD].map(|title| Call::Comment {
            project: PROJECT_ID.to_owned(),
            body: a_skip(title),
        }));
        assert_eq!(linear.calls(), expected, "{:?}", linear.calls());
    }

    #[test]
    fn a_skipped_slice_is_noted_so_a_later_run_passes_it_over() {
        // Red's rule: a skip is a human saying no at the gate, and is never
        // retried on its own.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::lines([SKIP, "y", ACCEPT, ACCEPT]);

        reviewing(repo.path(), home.path(), &linear, &mut typing);

        // A note that names no issue is how the project says so.
        assert_eq!(
            recorded(&linear),
            [
                (folded(FIRST), Vec::<String>::new()),
                (folded(SECOND), vec!["WAR-1".to_owned(), "WAR-2".to_owned()]),
                (folded(THIRD), vec!["WAR-3".to_owned(), "WAR-4".to_owned()]),
            ]
        );

        // And a second run over the project those notes are on has nothing to
        // offer: the skipped slice is settled like the filed ones.
        let notes: [String; 3] = linear
            .comments()
            .into_iter()
            .map(|(_, body)| body)
            .collect::<Vec<_>>()
            .try_into()
            .expect("three notes");
        let again = a_noted_project(SLICED, notes);
        let (outcome, _) = cut_running(repo.path(), home.path(), &again, &Scripted::saying([]));

        let error = outcome.expect_err("every slice is settled");
        assert!(
            error.to_string().contains("already cut or skipped"),
            "{error}"
        );
        assert!(again.issues_created().is_empty());
    }

    #[test]
    fn anything_else_is_feedback_to_the_same_session_and_the_slice_is_drafted_again() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        // Four answers for three slices: the first slice drafts, is told to
        // think again, and drafts something else.
        let agent = Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(AGAIN),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ]);
        let mut typing = Typing::lines([FEEDBACK, ACCEPT, ACCEPT, ACCEPT]);

        let (outcome, printed) =
            cut_reviewing(repo.path(), home.path(), &linear, &agent, &mut typing);

        outcome.expect("a redrafted slice is still filed");
        // What was typed reached the session that drafted them, in the words it
        // was typed in: feedback is a turn of that same conversation and never a
        // second one.
        assert_eq!(
            agent.said().iter().filter(|said| *said == FEEDBACK).count(),
            1,
            "{:?}",
            agent.said()
        );
        let lines = lines(&printed);
        let redrafting = &lines[placed(&lines, "is being redrafted")];
        assert!(redrafting.contains(FEEDBACK), "{redrafting}");
        assert!(
            redrafting.contains(&format!("slice 1 `{FIRST}`")),
            "{redrafting}"
        );
        // The drafts that came back were put up the same way — two readings for
        // the one slice, four for the run — and what was filed is the second
        // set rather than the one the feedback was about.
        assert_eq!(typing.asked(), [REVIEWED; 4], "{:?}", typing.asked());
        assert!(lines.contains(&drafted(1, FIRST, AGAIN)), "{lines:?}");
        let filed: Vec<String> = linear
            .issues_created()
            .into_iter()
            .map(|issue| issue.title)
            .collect();
        assert!(
            filed.contains(&format!("Stand in for {AGAIN}")),
            "{filed:?}"
        );
        assert!(
            !filed.contains(&format!("Stand in for {FIRST}")),
            "the drafts the feedback was about were filed: {filed:?}"
        );
        // And the key value reaches the client and nothing else, here as
        // everywhere.
        assert!(!printed.contains(NOT_A_KEY), "{printed}");
    }

    #[test]
    fn a_pipe_that_ends_at_the_review_leaves_the_slice_uncut_and_sends_nothing() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut typing = Typing::nothing();

        let printed = reviewing(repo.path(), home.path(), &linear, &mut typing);

        assert!(
            printed.contains("nobody said what to do with its drafts"),
            "{printed}"
        );
        assert!(printed.contains(&format!("slice 1 `{FIRST}`")), "{printed}");
        // Every slice was drafted and offered, and none was filed: nothing was
        // created, nothing was recorded, and a run that filed nothing has
        // nothing to say on the project.
        assert_eq!(typing.asked(), [REVIEWED; 3], "{:?}", typing.asked());
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert!(recorded(&linear).is_empty());
        assert!(linear.comments().is_empty(), "{:?}", linear.comments());
    }
}

// What a slice's session is seen doing at the shell. The printer itself writes
// to stdout, which no test can read, so what is asserted is the two halves on
// either side of it: the session is wired to the port, and the port's lines
// collapse a stretch of thinking or writing.
mod watching {
    use std::sync::Mutex;

    use super::*;
    use crate::claude::Activity;
    use crate::planned::Watched;

    // A `fn` and not a closure, because that is what `cut_with` takes: a fresh
    // port per session. Read by the one test below and by nothing else, so the
    // suite running in parallel cannot put another test's activity in it.
    static SEEN: Mutex<Vec<Activity>> = Mutex::new(Vec::new());

    fn recording() -> Activities {
        Activities::new(|activity| {
            SEEN.lock()
                .expect("nothing panics holding it")
                .push(activity);
        })
    }

    #[test]
    fn every_slice_session_reports_into_the_port_the_run_was_given() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let read = Activity::Tool {
            name: "Read".to_owned(),
            detail: Some("src/lib.rs".to_owned()),
        };
        let agent = Scripted::saying([
            Answering::drafts(FIRST),
            Answering::drafts(SECOND),
            Answering::drafts(THIRD),
        ])
        .doing([read.clone()]);
        let mut typing = Typing::lines([ACCEPT, ACCEPT, ACCEPT]);
        let mut out = Vec::new();

        cut_with(
            &Standing::at(repo.path().to_path_buf(), repo.path().to_path_buf()),
            home.path(),
            SCOPE,
            SLUG,
            false,
            &linear,
            &agent,
            &Scripted::saying([]),
            &mut typing,
            &mut out,
            recording,
        )
        .expect("a run that files");

        assert_eq!(
            *SEEN.lock().expect("nothing panics holding it"),
            [read.clone(), read.clone(), read],
            "one report per slice's one turn"
        );
    }

    #[test]
    fn a_stretch_of_thinking_or_writing_is_one_line_and_a_tool_is_every_time() {
        let mut watched = Watched::default();
        let read = Activity::Tool {
            name: "Read".to_owned(),
            detail: Some("src/lib.rs".to_owned()),
        };

        let said: Vec<Option<String>> = [
            Activity::Thinking,
            Activity::Thinking,
            read.clone(),
            read,
            Activity::Writing { bytes: 10 },
            Activity::Writing { bytes: 20 },
            Activity::Cost { usd: 0.01 },
            Activity::Thinking,
        ]
        .iter()
        .map(|activity| watched.line(activity))
        .collect();

        assert_eq!(
            said,
            [
                Some("thinking".to_owned()),
                None,
                Some("Read src/lib.rs".to_owned()),
                Some("Read src/lib.rs".to_owned()),
                Some("writing".to_owned()),
                None,
                None,
                Some("thinking".to_owned()),
            ]
        );
    }
}

// Forman's `[e]dit`: the drafts written out as the panel shows them, changed in
// an editor, and read back.
mod editing {
    use super::*;
    use crate::brief::scope_block_in;
    use crate::planned::{drafts_document, drafts_from_document, edited_drafts};

    fn slice() -> crate::brief::Slice {
        scope_block_in("Brief.\n\n## Scope\n\n### 1. Rename the field\n\nThe prose.\n")
            .expect("a scope block")
            .slices()[0]
            .clone()
    }

    fn drafts() -> Vec<Draft> {
        vec![
            Draft {
                title: "Rename the field on the record".to_owned(),
                body: "## Problem\nThe field says team.\n\n## Acceptance criteria\n- [ ] Renamed"
                    .to_owned(),
                blocked_by: Vec::new(),
                blocks: vec![1],
                waits_on: vec!["WAR-142".to_owned()],
            },
            Draft {
                title: "Fix every call site of the field".to_owned(),
                body: "Every caller.".to_owned(),
                blocked_by: vec![0],
                blocks: Vec::new(),
                waits_on: Vec::new(),
            },
        ]
    }

    #[test]
    fn the_document_reads_back_into_the_drafts_it_was_written_from() {
        // A body's own `## Problem` sections stay in the body: a draft's
        // heading starts with its number, and theirs does not.
        let text = drafts_document(&slice(), &drafts()).join("\n");

        assert_eq!(drafts_from_document(&text), Ok(drafts()));
    }

    #[test]
    fn what_is_saved_in_the_editor_is_what_comes_back() {
        let edited = edited_drafts(&slice(), &drafts(), |path| {
            let text = fs::read_to_string(path).expect("the drafts were written");
            let text = text.replace("Fix every call site", "Fix the call sites");
            fs::write(path, text).expect("the edit was saved");
            None
        })
        .expect("an edit that reads back");

        assert_eq!(edited[1].title, "Fix the call sites of the field");
        assert_eq!(edited[1].blocked_by, [0]);
    }

    #[test]
    fn an_edit_that_leaves_no_draft_or_a_wrong_edge_changes_nothing() {
        assert!(drafts_from_document("# slice 1\n\nNothing here.").is_err());
        assert!(
            drafts_from_document("## 1. A title long enough\n\nBody.\n\nBlocks #7")
                .is_err_and(|why| why.contains("#7"))
        );
        let failed = edited_drafts(&slice(), &drafts(), |_| Some("`vim` exited".to_owned()));
        assert_eq!(failed, Err("`vim` exited".to_owned()));
    }
}

// Forman's backlog digest: the open tickets a session can name in `blocked_by`.
mod backlog {
    use super::*;
    use crate::linear::{Priority, Queue, QueuedIssue, StateType};

    fn open(identifier: &str, title: &str) -> QueuedIssue {
        QueuedIssue::new(
            format!("id-{identifier}"),
            identifier,
            title,
            "Todo",
            StateType::new("unstarted"),
            Priority::None,
            Vec::new(),
        )
    }

    #[test]
    fn the_brief_a_session_is_given_lists_the_open_tickets_in_forman_s_words() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).queueing(Queue::new(vec![
            open("WAR-10", "The tenth ticket"),
            open("WAR-9", "The ninth ticket"),
        ]));

        let brief = prepared(repo.path(), home.path(), &linear).drafting_brief();

        // Ordered as a person counts, WAR-9 before WAR-10.
        assert!(
            brief.ends_with(
                "These tickets already exist and are still open. If what you are drafting \
                 cannot start until one of them has landed, put that identifier in \
                 blocked_by:\n\nWAR-9  [Todo]  The ninth ticket\nWAR-10  [Todo]  The tenth ticket"
            ),
            "{brief}"
        );
    }

    #[test]
    fn with_nothing_open_the_brief_is_the_brief() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        let brief = prepared(repo.path(), home.path(), &linear).drafting_brief();

        assert!(
            !brief.contains("already exist and are still open"),
            "{brief}"
        );
    }
}

mod finishing {
    use super::*;

    // Every uncut slice skipped, in order.
    fn skipping_each(planned: &mut Planned, linear: &Boarding) -> Vec<Option<String>> {
        let mut said = Vec::new();
        while let Some(next) = planned.next_uncut() {
            said.push(
                planned
                    .skipping(&next)
                    .post(linear)
                    .expect("a skip that is noted"),
            );
        }
        said
    }

    #[test]
    fn a_project_moves_only_when_its_last_slice_settles() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let first = planned.next_uncut().expect("a first slice");
        let cut = planned
            .filing(&first, drafts_for(FIRST))
            .file(&linear, &mut io::sink())
            .expect("a slice that files");
        planned.settle(&first, cut);

        assert!(linear.moves().is_empty(), "two slices are still to draft");
    }

    #[test]
    fn a_skip_that_settles_the_last_slice_moves_a_project_with_issues() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1"]), a_skip(SECOND)]);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let said = skipping_each(&mut planned, &linear);

        assert_eq!(said, [Some(MOVED.to_owned())]);
        assert_eq!(
            linear.moves(),
            [(PROJECT_ID.to_owned(), "status-in-progress".to_owned())]
        );
    }

    #[test]
    fn a_project_every_slice_of_which_was_skipped_stays_planned() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let said = skipping_each(&mut planned, &linear);

        assert_eq!(said, [None, None, None]);
        assert!(
            linear.moves().is_empty(),
            "no issue exists to be in progress"
        );
    }

    #[test]
    fn a_workspace_with_no_in_progress_status_is_a_line_and_not_a_failure() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(SLICED, [a_cut(FIRST, &["WAR-1"]), a_skip(SECOND)])
            .without_project_status();
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let said = skipping_each(&mut planned, &linear);

        assert_eq!(
            said,
            [Some(
                "every slice is settled, and the workspace has no project status called \
                 `In Progress`, so the project was not moved"
                    .to_owned()
            )]
        );
        assert!(linear.moves().is_empty());
    }

    #[test]
    fn a_move_linear_turns_down_is_a_line_and_the_issues_stand() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_noted_project(
            SLICED,
            [a_cut(FIRST, &["WAR-1"]), a_cut(SECOND, &["WAR-2"])],
        )
        .refuse(Op::MoveProject, "the workspace would not");
        let mut planned = prepared(repo.path(), home.path(), &linear);

        let settled = filing_each(&mut planned, &linear);

        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].issues.len(), 2);
        assert!(
            settled[0].reported[0].contains("the project was not moved to `In Progress`"),
            "{:?}",
            settled[0].reported
        );
    }
}
