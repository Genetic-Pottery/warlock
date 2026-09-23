use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use warlock_engine::drafting::{Draft, stub_answer};
use warlock_engine::{
    CutRecord, Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, agent, filed_path, save_key,
    save_key_binding, save_sigils,
};
use warlock_tui::{
    Activities, Cancel, Converses, DRAFTING_ONE_SHOT_CONTRACT, FetchedProject, Opens,
    ScopeBlockError, Wired,
};

use super::{Planned, Settled, cut_with, prepare};
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;
use crate::stubs::{Boarding, Call, Op};

// Not a key, and named so that nothing reading this file mistakes it for one.
// It is stored only so that a bound name resolves and a cut can reach the
// client, which is the one line that sees it.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "work";

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const BRIEF_PATH: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

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

// The model a read may never turn. A refusal opens no session and a dry run
// opens none at all, so being asked for a turn is the failure rather than a
// flag.
#[derive(Debug, Clone, Copy)]
struct Unasked;

impl Wired for Unasked {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        *self
    }
}

impl Converses for Unasked {
    fn turn(&self, _message: &str) -> Result<String, agent::Error> {
        panic!("a drafting session was opened");
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        *self
    }
}

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
}

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// A repository that has filed one brief, which is the finished state a cut
// starts from.
fn a_repository() -> TempDir {
    let repo = a_dir();
    recording(repo.path(), BRIEF_PATH);
    repo
}

// The same, with a `[[scope]]` record for the board a cut reads back off:
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

fn recording(root: &Path, path: &str) {
    Filed::with_records([a_filed_record(root, path)])
        .save(root)
        .expect("a record file that saves");
}

// The same record with a cut already on it, which is what a slice that has been
// filed once looks like on disk.
fn recording_cuts(root: &Path, cuts: impl IntoIterator<Item = CutRecord>) {
    let mut record = a_filed_record(root, BRIEF_PATH);
    for cut in cuts {
        record.push_cut(cut);
    }
    Filed::with_records([record])
        .save(root)
        .expect("a record file that saves");
}

fn a_filed_record(root: &Path, path: &str) -> FiledRecord {
    FiledRecord::new(
        root,
        root.join(path),
        PROJECT_ID,
        URL,
        SCOPE,
        TEAM,
        "2026-09-21T09:14:00Z",
    )
    .expect("a path inside the repository")
}

fn a_cut(title: &str, issues: &[&str]) -> CutRecord {
    CutRecord::new(title, issues.iter().copied(), "2026-09-21T10:00:00Z")
}

// A workspace holding the one project the record names, which answers
// everything a run then asks for as well.
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

fn fetched_by_the_record() -> [Call; 1] {
    [Call::FetchProject(PROJECT_ID.to_owned())]
}

// The module's first step, less the environment: the repository root and the
// home are this test's temporary directories, and the seam is whatever `open`
// is.
fn preparing<O: Opens>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    open: &O,
) -> Result<Planned, Error> {
    let standing = Standing::at(repo.to_path_buf(), repo.to_path_buf());
    prepare(
        &standing.manifest()?,
        repo,
        home,
        &standing.target(path),
        scope,
        open,
    )
}

// A cut that has to come to something, over the scoped repository and home a
// test has already built.
fn prepared(repo: &Path, home: &Path, linear: &Boarding) -> Planned {
    preparing(repo, home, BRIEF_PATH, None, linear).expect("a draft to walk")
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
        },
        Draft {
            title: format!("Follow on from {title}"),
            body,
            blocked_by: vec![0],
            blocks: Vec::new(),
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
// are this test's temporary directories, the socket is whatever `open` is, and
// the model is one that panics when it is turned.
fn cut_to<O: Opens>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    dry_run: bool,
    open: &O,
) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = cut_with(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        home,
        Path::new(path),
        scope,
        dry_run,
        open,
        &Unasked,
        &mut out,
    );

    (
        outcome,
        String::from_utf8(out).expect("warlock writes its own text"),
    )
}

// The whole subcommand with a model in it, which `cut_to` cannot drive: its
// agent panics when it is turned. Never a dry run — a dry run with a model in
// reach is the thing the reading tests are about.
fn cut_running<A: Converses>(
    repo: &Path,
    home: &Path,
    linear: &Boarding,
    agent: &A,
) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = cut_with(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        home,
        Path::new(BRIEF_PATH),
        None,
        false,
        linear,
        agent,
        &mut out,
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

// The cuts `.warlock/filed.toml` holds for the brief, as they are on disk: the
// record is the product of a cut, so it is read back off the file rather than
// off anything the run handed over.
fn recorded(root: &Path) -> Vec<(String, Vec<String>)> {
    Filed::load(root)
        .expect("a record file that reads")
        .record(BRIEF_PATH)
        .expect("the brief is recorded")
        .cuts()
        .iter()
        .map(|cut| (cut.title().to_owned(), cut.issues().to_vec()))
        .collect()
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
// a filing per slice, and the one comment.
mod preparing {
    use super::*;

    #[test]
    fn a_recorded_brief_is_fetched_by_the_id_that_record_holds() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Planned"));

        let planned = prepared(repo.path(), home.path(), &linear);

        assert_eq!(planned.name(), NAME);
        assert_eq!(planned.status(), "Planned");
        assert_eq!(planned.total(), 3);
        assert_eq!(planned.left(), 3);
        assert_eq!(planned.destination().team(), TEAM);
        // The id out of `.warlock/filed.toml` and no other selector, in one
        // request.
        assert_eq!(linear.calls(), fetched_by_the_record());
        assert_eq!(linear.requests(), 1, "one call per operation");
        assert_eq!(linear.opened_with(), [NOT_A_KEY.to_owned()]);
        assert!(
            !format!("{planned:?}").contains(NOT_A_KEY),
            "the draft renders the key"
        );
    }

    #[test]
    fn a_brief_named_from_a_subdirectory_resolves_to_the_same_record() {
        // The spelling a push records is `to_manifest_path`'s, so a path handed
        // in with a `./` in front of it is the same record rather than a second
        // one.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Planned"));

        preparing(repo.path(), home.path(), "./docs/brief.md", None, &linear)
            .expect("the same brief, spelled twice");

        assert_eq!(linear.calls(), fetched_by_the_record());
    }

    #[test]
    fn a_brief_no_record_names_is_refused_before_the_key_is_read() {
        // `.warlock/filed.toml` is what turns a path into a project id, so a
        // brief nothing filed is answered on this machine: no board is opened,
        // which the seam asserts by panicking if one is.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            "docs/other.md",
            None,
            &Boarding::unopened(),
        );

        assert!(
            matches!(&error, Error::NoRecord { path } if path == "docs/other.md"),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains("docs/other.md"), "{message}");
        assert!(message.contains("warlock push"), "{message}");
    }

    #[test]
    fn a_repository_that_has_filed_nothing_at_all_is_that_same_refusal() {
        // No `.warlock/filed.toml` on disk, which `records` reads as an empty
        // one: a repository that has never pushed records no project, and that
        // is an answer rather than a failure to reach one.
        let repo = a_dir();
        saving(repo.path(), &a_manifest([a_record(SCOPE, TEAM)]));
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
            None,
            &Boarding::unopened(),
        );

        assert!(matches!(error, Error::NoRecord { .. }), "{error:?}");
        assert!(!filed_path(repo.path()).exists());
    }

    #[test]
    fn a_project_id_the_api_does_not_know_names_the_id_and_the_file_it_is_written_in() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
            None,
            &Boarding::filing(""),
        );

        assert!(
            matches!(&error, Error::UnknownProject { id, .. } if id == PROJECT_ID),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains(PROJECT_ID), "{message}");
        assert!(
            message.contains(".warlock/filed.toml"),
            "the file the id is written in is not named: {message}"
        );
    }

    #[test]
    fn a_project_that_is_not_planned_names_the_status_it_found_and_the_one_it_wanted() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_project(Some("Backlog"));

        let error = refused(repo.path(), home.path(), BRIEF_PATH, None, &linear);

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
        assert_eq!(linear.calls(), fetched_by_the_record());
    }

    #[test]
    fn a_project_with_no_status_at_all_is_refused_and_says_it_has_none() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(repo.path(), home.path(), BRIEF_PATH, None, &a_project(None));

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
                BRIEF_PATH,
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
                BRIEF_PATH,
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
    fn nothing_on_this_path_writes_to_the_board_or_to_the_record_file() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");
        let linear = a_project(Some("Planned"));

        prepared(repo.path(), home.path(), &linear);

        assert_eq!(
            linear.calls(),
            fetched_by_the_record(),
            "something other than the read was asked"
        );
        assert_eq!(
            fs::read_to_string(filed_path(repo.path())).expect("a record file"),
            before,
            "a read rewrote the record file"
        );
    }

    #[test]
    fn a_record_file_that_will_not_read_is_a_failure_rather_than_an_empty_one() {
        // The reading half of `push.rs`'s rule: an unreadable record file is a
        // repository whose filings are unknown, not one with none, and
        // answering "nothing is filed" here would send somebody back to
        // `warlock push` for a brief that already has a project.
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        fs::write(filed_path(repo.path()), "version = 2\nnot toml {{{")
            .expect("a broken record file");

        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
            None,
            &Boarding::unopened(),
        );

        assert!(matches!(error, Error::Filed { .. }), "{error:?}");
    }

    #[test]
    fn a_project_whose_every_slice_is_cut_is_refused_naming_the_brief_and_the_file() {
        let repo = a_scoped_repository();
        recording_cuts(
            repo.path(),
            [
                a_cut(FIRST, &["WAR-1"]),
                a_cut(SECOND, &["WAR-2"]),
                a_cut(THIRD, &["WAR-3"]),
            ],
        );
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");

        let error = refused(repo.path(), home.path(), BRIEF_PATH, None, &linear);

        assert!(
            matches!(&error, Error::AllCut { path } if path == BRIEF_PATH),
            "{error:?}"
        );
        let message = said(&error);
        assert!(message.contains(BRIEF_PATH), "{message}");
        assert!(message.contains(".warlock/filed.toml"), "{message}");
        // Refused where the answer that said so arrived: the fetch and nothing
        // after it, and the record file as it was.
        assert_eq!(linear.calls(), fetched_by_the_record());
        assert_eq!(
            fs::read_to_string(filed_path(repo.path())).expect("a record file"),
            before
        );
    }

    #[test]
    fn a_project_with_no_scope_block_is_refused_in_the_parser_s_own_words() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());

        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
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
            BRIEF_PATH,
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
            let error = refused(
                repo.path(),
                home.path(),
                BRIEF_PATH,
                None,
                &Boarding::unopened(),
            );

            assert!(matches!(error, Error::Filing { .. }), "{error:?}");
            assert!(said(&error).contains(expected), "{}", said(&error));
        }
    }

    #[test]
    fn a_machine_that_can_cut_from_several_boards_names_them_all_and_asks_for_one() {
        let repo = a_repository();
        saving(
            repo.path(),
            &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
        );
        let home = a_home_holding(repo.path(), &[SCOPE, "web"]);

        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
            None,
            &Boarding::unopened(),
        );

        let message = said(&error);
        assert!(matches!(error, Error::Filing { .. }), "{error:?}");
        assert!(
            message.contains(SCOPE) && message.contains("web"),
            "{message}"
        );
        assert!(message.contains("--scope"), "{message}");
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
            BRIEF_PATH,
            Some("web"),
            &a_sliced_project(SLICED),
        )
        .expect("a named candidate is a board");
        assert_eq!(planned.destination().team(), "WEB");
        assert_eq!(planned.destination().scope(), "web");

        // And a name that is not one of them is refused with both of them
        // named, with no key read and nothing sent.
        let error = refused(
            repo.path(),
            home.path(),
            BRIEF_PATH,
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

mod walking {
    use super::*;

    #[test]
    fn every_slice_is_walked_in_the_cut_order_with_what_a_record_already_names() {
        // The last slice is what the first two wait on, and the first is
        // already cut: the walk is the order tickets would be filed in, the
        // fraction is the place in that order, and the position still finds the
        // slice in the document.
        let repo = a_scoped_repository();
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let home = a_home(repo.path());
        let mut planned = prepared(repo.path(), home.path(), &a_sliced_project(OUT_OF_ORDER));

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
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1"])]);
        let home = a_home(repo.path());
        let mut planned = prepared(repo.path(), home.path(), &a_sliced_project(SLICED));
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
                Settled::Filed {
                    issues: vec!["WAR-1".to_owned(), "WAR-2".to_owned()],
                    reported: Vec::new(),
                },
                Settled::Filed {
                    issues: vec!["WAR-3".to_owned(), "WAR-4".to_owned()],
                    reported: Vec::new(),
                },
                Settled::Filed {
                    issues: vec!["WAR-5".to_owned(), "WAR-6".to_owned()],
                    reported: Vec::new(),
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
    fn a_slice_waiting_on_one_that_was_already_cut_is_blocked_by_the_issues_its_record_names() {
        // The one thing a resumed cut does with a slice it cut last time: the
        // identifiers off the record are what the relation names, because a cut
        // record keeps nothing else of an issue.
        let repo = a_scoped_repository();
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).numbering_from(3);
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

        let Settled::Filed { issues, reported } = &settled[0] else {
            panic!("the first slice was not filed: {settled:?}");
        };
        assert_eq!(issues, &["WAR-1".to_owned(), "WAR-2".to_owned()]);
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert!(
            reported[0].contains("was not written as blocking")
                && reported[0].contains("the workspace would not"),
            "{:?}",
            reported[0]
        );
        assert_eq!(recorded(repo.path()).len(), 3);
    }

    #[test]
    fn no_key_value_is_in_a_filing_or_the_comment_it_leads_to() {
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

        let announcement = planned.finish().expect("a comment is owed");
        assert!(
            !format!("{announcement:?}").contains(NOT_A_KEY),
            "{announcement:?}"
        );
        // The key reached the one place it is for: every board opened.
        assert!(
            linear.opened_with().iter().all(|key| key == NOT_A_KEY),
            "{:?}",
            linear.opened_with()
        );
    }
}

mod finishing {
    use super::*;

    #[test]
    fn a_cut_that_created_nothing_owes_no_comment() {
        // Every slice but one already cut, and that one never filed: nothing was
        // created, so there is nothing to say on the project.
        let repo = a_scoped_repository();
        recording_cuts(
            repo.path(),
            [a_cut(FIRST, &["WAR-1"]), a_cut(SECOND, &["WAR-2"])],
        );
        let home = a_home(repo.path());
        let mut planned = prepared(repo.path(), home.path(), &a_sliced_project(SLICED));
        while planned.next().is_some() {}

        assert!(planned.finish().is_none(), "a comment was owed for nothing");
    }

    #[test]
    fn the_comment_names_only_what_this_cut_created_and_is_said_on_the_project() {
        // An earlier run's issues were named by an earlier run's comment.
        let repo = a_scoped_repository();
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).numbering_from(3);
        let mut planned = prepared(repo.path(), home.path(), &linear);
        filing_each(&mut planned, &linear);

        let line = planned.finish().expect("a comment is owed").post(&linear);

        assert_eq!(line, None, "a comment that was said reported a line");
        let comments = linear.comments();
        assert_eq!(comments.len(), 1, "{comments:?}");
        let (project, body) = &comments[0];
        assert_eq!(project, PROJECT_ID);
        for issue in ["WAR-3", "WAR-4", "WAR-5", "WAR-6"] {
            assert!(body.contains(issue), "{body}");
        }
        for issue in ["WAR-1`", "WAR-2`"] {
            assert!(!body.contains(issue), "{body}");
        }
        assert!(body.contains("status was not moved"), "{body}");
    }

    #[test]
    fn a_comment_the_api_turns_down_is_a_line_rather_than_a_failure() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Comment, "the workspace would not");
        let mut planned = prepared(repo.path(), home.path(), &linear);
        filing_each(&mut planned, &linear);

        let line = planned.finish().expect("a comment is owed").post(&linear);

        let line = line.expect("a refusal is a line");
        assert!(line.contains("the project was not commented on"), "{line}");
        assert!(line.contains("the workspace would not"), "{line}");
        assert_eq!(recorded(repo.path()).len(), 3, "the cut did not stand");
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

        let (outcome, printed) = cut_to(repo.path(), home.path(), BRIEF_PATH, None, false, &linear);

        assert!(printed.is_empty(), "a refusal printed something: {printed}");
        let error = refusal(outcome);
        assert!(matches!(error, Error::NotPlanned { .. }), "{error:?}");
    }

    #[test]
    fn a_dry_run_reports_the_project_its_status_and_every_slice_then_stops() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");

        let (outcome, printed) = cut_to(repo.path(), home.path(), BRIEF_PATH, None, true, &linear);

        outcome.expect("a dry run answers");
        // The one request that got the project, and no second one: a dry run
        // reads what it reports and spends nothing else.
        assert_eq!(
            linear.calls(),
            fetched_by_the_record(),
            "a dry run sent more than the fetch"
        );
        assert_eq!(
            fs::read_to_string(filed_path(repo.path())).expect("a record file"),
            before,
            "a dry run wrote a record of a cut nobody made"
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

        let (outcome, printed) = cut_to(repo.path(), home.path(), BRIEF_PATH, None, true, &linear);

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
    fn a_dry_run_reports_a_slice_a_record_already_names_with_the_issues_it_became() {
        let repo = a_scoped_repository();
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        let (outcome, printed) = cut_to(repo.path(), home.path(), BRIEF_PATH, None, true, &linear);

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

        let (outcome, printed) = cut_to(
            repo.path(),
            home.path(),
            BRIEF_PATH,
            Some("web"),
            true,
            &linear,
        );

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

        // One line per slice as it is drafted and one naming what it filed, in
        // the order the slices are cut in and with nothing else between them.
        assert_eq!(
            lines,
            [
                format!("[1/3] slice 1 `{FIRST}` — drafting"),
                format!("cut `{FIRST}` into `WAR-1`, `WAR-2`"),
                format!("[2/3] slice 2 `{SECOND}` — drafting"),
                format!("cut `{SECOND}` into `WAR-3`, `WAR-4`"),
                format!("[3/3] slice 3 `{THIRD}` — drafting"),
                format!("cut `{THIRD}` into `WAR-5`, `WAR-6`"),
            ],
            "{lines:?}"
        );
        // One session per slice and one turn in each of them: no question was
        // relayed, because a one-shot session has no round to relay one with.
        assert_eq!(agent.turns(), 3, "{:?}", agent.said());
        assert_eq!(linear.issues_created().len(), 6);
    }

    #[test]
    fn every_slice_is_drafted_under_the_one_shot_contract_and_carries_its_own_words() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
        let agent = Sketching::drafting();

        cut_filing(repo.path(), home.path(), &linear, &agent);

        let said = agent.said();
        for (turn, heading) in said.iter().zip([FIRST, SECOND, THIRD]) {
            assert!(
                turn.contains(DRAFTING_ONE_SHOT_CONTRACT),
                "a session opened without the one-shot contract: {turn}"
            );
            // Its own slice and not the whole scope block: one session is aimed
            // at one slice, so the next slice's heading is nowhere in its
            // opening.
            assert!(turn.contains(heading), "{turn}");
        }
        assert!(!said[0].contains(SECOND), "{}", said[0]);
    }

    #[test]
    fn a_cut_record_is_on_disk_for_every_slice_the_run_filed() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        assert_eq!(
            recorded(repo.path()),
            [
                (
                    FIRST.to_owned(),
                    vec!["WAR-1".to_owned(), "WAR-2".to_owned()]
                ),
                (
                    SECOND.to_owned(),
                    vec!["WAR-3".to_owned(), "WAR-4".to_owned()]
                ),
                (
                    THIRD.to_owned(),
                    vec!["WAR-5".to_owned(), "WAR-6".to_owned()]
                ),
            ]
        );
    }

    #[test]
    fn a_run_that_dies_partway_keeps_the_record_of_every_slice_it_had_already_filed() {
        // The record is saved after each slice and not at the end: two issues
        // are created and the third is turned down, and what is on disk
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
            recorded(repo.path()),
            [(
                FIRST.to_owned(),
                vec!["WAR-1".to_owned(), "WAR-2".to_owned()]
            )]
        );
        // And what it did file was said before it stopped: the identifiers are
        // the one thing that must not be lost.
        assert!(lines(&printed).contains(&format!("cut `{FIRST}` into `WAR-1`, `WAR-2`")));
    }

    #[test]
    fn a_slice_the_record_already_names_is_said_and_nothing_at_all_is_sent_for_it() {
        let repo = a_scoped_repository();
        recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);
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
        // And its record is the one the earlier run wrote, unchanged and still
        // first.
        assert_eq!(
            recorded(repo.path())[0],
            (
                FIRST.to_owned(),
                vec!["WAR-1".to_owned(), "WAR-2".to_owned()]
            )
        );
    }

    #[test]
    fn a_team_with_no_backlog_state_refuses_the_run_with_nothing_created() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).without_backlog_state();
        let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");

        let (outcome, _printed) =
            cut_running(repo.path(), home.path(), &linear, &Sketching::drafting());

        let error = refusal(outcome);
        assert!(
            matches!(&error, Error::NoBacklog { team } if team == TEAM),
            "{error:?}"
        );
        assert!(said(&error).contains(TEAM), "{}", said(&error));
        // Nothing on the board and nothing on disk: the refusal happens while
        // the slice is still nothing rather than half filed.
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert_eq!(
            fs::read_to_string(filed_path(repo.path())).expect("a record file"),
            before
        );
    }

    #[test]
    fn a_slice_that_never_parsed_is_reported_and_left_uncut_rather_than_filed_as_a_stand_in() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        let lines = cut_filing(repo.path(), home.path(), &linear, &Sketching::talking());

        for (place, heading) in [(1, FIRST), (2, SECOND), (3, THIRD)] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(heading) && line.contains("was not drafted")),
                "slice {place} `{heading}` was not reported: {lines:?}"
            );
        }
        // Nothing was created, nothing was recorded, and the project was not
        // commented on: a run that filed nothing has nothing to say about it.
        assert!(
            linear.issues_created().is_empty(),
            "{:?}",
            linear.issues_created()
        );
        assert!(recorded(repo.path()).is_empty());
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
    fn the_project_is_commented_on_once_after_the_last_slice_naming_every_issue_filed() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        let comments = linear.comments();
        assert_eq!(comments.len(), 1, "{comments:?}");
        let (project, body) = &comments[0];
        assert_eq!(project, PROJECT_ID);
        for issue in ["WAR-1", "WAR-2", "WAR-3", "WAR-4", "WAR-5", "WAR-6"] {
            assert!(body.contains(issue), "{body}");
        }
        // After the last slice settled, which is after the last create.
        let commented = linear.positions_of(Op::Comment)[0];
        let last = *linear
            .positions_of(Op::CreateIssue)
            .last()
            .expect("issues were created");
        assert!(commented > last, "{commented} is not after {last}");
    }

    #[test]
    fn a_comment_the_api_turns_down_is_the_last_line_printed() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED).refuse(Op::Comment, "the workspace would not");

        let lines = cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        assert!(
            lines
                .last()
                .expect("something was printed")
                .contains("the project was not commented on"),
            "{lines:?}"
        );
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
        assert_eq!(recorded(repo.path()).len(), 3);
    }

    #[test]
    fn nothing_the_run_sends_moves_a_status_or_writes_a_field_warlock_would_have_to_invent() {
        let repo = a_scoped_repository();
        let home = a_home(repo.path());
        let linear = a_sliced_project(SLICED);

        cut_filing(repo.path(), home.path(), &linear, &Sketching::drafting());

        // A board has no operation that moves a status, and an issue create has
        // no field beyond the six a draft and its slice resolve: what the latter
        // puts on the wire is held by `linear.rs`'s own tests. What a run can
        // still get wrong is asking for something a cut has no business asking.
        for op in linear.ops() {
            assert!(
                matches!(
                    op,
                    Op::FetchProject
                        | Op::Team
                        | Op::BacklogState
                        | Op::IssueLabel
                        | Op::CreateIssue
                        | Op::Relation
                        | Op::Comment
                ),
                "a run asked for {op:?}"
            );
        }
    }
}
