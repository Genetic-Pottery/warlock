use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use serde_json::{Value, json};
use tempfile::TempDir;
use warlock_engine::{
    CutRecord, Filed, FiledRecord, Manifest, PactEntry, ScopeRecord, agent, filed_path, save_key,
    save_key_binding, save_sigils,
};
use warlock_tui::{
    Activities, Cancel, Converses, FetchedProject, LinearError, Posts, ScopeBlockError, Wired,
};

use super::{planned, pulled};
use crate::error::Error;
use crate::standing::Standing;
use crate::status_for;

// Not a key, and named so that nothing reading this file mistakes it for one.
// It is stored only so that a bound name resolves and a pull can reach the
// client, which is the one line that sees it.
const NOT_A_KEY: &str = "not-a-real-key-value";

const KEY_NAME: &str = "work";

const SCOPE: &str = "warlock-team";

const TEAM: &str = "WAR";

const LABEL: &str = "warlock";

const BRIEF_PATH: &str = "docs/brief.md";

const PROJECT_ID: &str = "b229262b-22aa-444a-a8af-0a2a3f4ef100";

const URL: &str = "https://linear.app/acme/project/pull-a-brief-1a2b3c";

const NAME: &str = "Cut a planned project into tickets";

const PLANNED: &str = "Planned";

const CONTENT: &str = "# Cut a planned project into tickets\n\n## Scope\n";

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

// A Linear that answers from memory, in the order it was given. Held by handle,
// because the closure `pulled` opens its client through has to hand one *over*
// and the test keeps a copy to read afterwards. `Rc<RefCell<_>>` rather than the
// `Mutex` the client's own stand-in uses, because nothing here crosses a thread.
#[derive(Debug, Clone)]
struct Posting {
    answers: Rc<RefCell<VecDeque<Result<Value, LinearError>>>>,
    asked: Rc<RefCell<Vec<(String, Value)>>>,
}

impl Posting {
    fn answering(answers: impl IntoIterator<Item = Result<Value, LinearError>>) -> Self {
        Self {
            answers: Rc::new(RefCell::new(answers.into_iter().collect())),
            asked: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn documents(&self) -> Vec<String> {
        self.asked
            .borrow()
            .iter()
            .map(|(document, _)| document.clone())
            .collect()
    }

    fn variables(&self) -> Vec<Value> {
        self.asked
            .borrow()
            .iter()
            .map(|(_, variables)| variables.clone())
            .collect()
    }
}

impl Posts for Posting {
    fn post(&self, document: &str, variables: Value) -> Result<Value, LinearError> {
        self.asked
            .borrow_mut()
            .push((document.to_owned(), variables));

        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("the stand-in was asked more times than it was answered"))
    }
}

// The seam a refusal may never reach. A brief nothing records is refused with
// the file on this machine and nothing else, so being posted to at all is the
// failure the test is about — which is why it is a panic and not a flag.
#[derive(Debug, Clone, Copy)]
struct Unreachable;

impl Posts for Unreachable {
    fn post(&self, document: &str, _variables: Value) -> Result<Value, LinearError> {
        panic!("a request was sent: {document}");
    }
}

// The client a board refusal may never build. Every refusal decided before the
// fetch happens with no key read at all, so reaching this is the failure the
// test is about.
fn no_socket(_key: &str) -> Unreachable {
    panic!("the key was read and a client was built");
}

// The model a read may never turn. Nothing above the drafting half opens a
// session, and a dry run opens none at all, so being asked for a turn is the
// failure rather than a flag.
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

fn a_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// A repository that has filed one brief, which is the finished state a pull
// starts from.
fn a_repository() -> TempDir {
    let repo = a_dir();
    recording(repo.path(), BRIEF_PATH);
    repo
}

// The same, with a `[[scope]]` record for the board a pull reads back off:
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
// under it: everything a pull needs, under a directory of this test's own, so
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

fn a_project(status: Option<&str>) -> Value {
    a_project_of(CONTENT, status)
}

fn a_project_of(content: &str, status: Option<&str>) -> Value {
    json!({
        "project": {
            "name": NAME,
            "content": content,
            "url": URL,
            "status": status.map(|name| json!({ "name": name })),
        },
    })
}

// A planned project with slices in it, which is the one answer a whole pull
// gets before it starts drafting.
fn a_sliced_project(content: &str) -> Posting {
    Posting::answering([Ok(a_project_of(content, Some(PLANNED)))])
}

// The reading half, less the environment: the repository root is this test's
// temporary directory and the seam is whatever was handed in.
fn pull_from(repo: &Path, linear: &impl Posts, path: &str) -> Result<FetchedProject, Error> {
    planned(
        &Standing::at(repo.to_path_buf(), repo.to_path_buf()),
        linear,
        Path::new(path),
    )
}

// The whole subcommand, less the environment: the repository root and the home
// are this test's temporary directories, the socket is whatever `open` is, and
// the model is one that panics when it is turned.
fn pull_to<P: Posts, O: FnOnce(&str) -> P>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    dry_run: bool,
    open: O,
) -> (Result<(), Error>, String) {
    let mut out = Vec::new();
    let outcome = pulled(
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

// What every refusal here promises, checked in one place: the ordinary exit
// status rather than the boundary's, and one line to print.
fn refusal<T>(outcome: Result<T, Error>) -> Error {
    let outcome = outcome.map(drop);

    assert_eq!(status_for(&outcome), 1, "a pull refusal is the ordinary 1");
    assert_ne!(
        status_for(&outcome),
        3,
        "a pull refusal took the boundary's status"
    );

    let error = outcome.expect_err("a refusal");
    let message = error.to_string();
    assert!(!message.contains('\n'), "`main` prints one line: {message}");
    error
}

// The socket as `pulled` takes it: a closure handing over the stand-in the test
// keeps a handle on, so what was asked can be read after the run. The key is
// dropped here because this is the one place a pull would spend it, and nothing
// below it ever sees a key at all.
fn opening(linear: &Posting) -> impl FnOnce(&str) -> Posting {
    let linear = linear.clone();
    move |_key| linear
}

// A pull that must refuse, with the seam whatever the case allows: a board
// refusal is handed `no_socket`, and one decided after the fetch is handed the
// stand-in that answered it. Nothing printed is part of the promise — a refusal
// is `main`'s one line and not a report with a failure at the end of it.
fn refusing<P: Posts, O: FnOnce(&str) -> P>(
    repo: &Path,
    home: &Path,
    path: &str,
    scope: Option<&str>,
    open: O,
) -> Error {
    let (outcome, printed) = pull_to(repo, home, path, scope, false, open);

    assert!(printed.is_empty(), "a refusal printed something: {printed}");

    refusal(outcome)
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

#[test]
fn a_recorded_brief_is_fetched_by_the_id_that_record_holds() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    let project = pull_from(repo.path(), &linear, BRIEF_PATH).expect("a planned project is read");

    assert_eq!(project.name(), NAME);
    assert_eq!(project.content(), CONTENT);
    assert_eq!(project.url(), URL);
    assert_eq!(project.status(), Some("Planned"));
    // The id out of `.warlock/filed.toml` and no other selector, in one request.
    assert_eq!(linear.variables(), [json!({ "id": PROJECT_ID })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_brief_named_from_a_subdirectory_resolves_to_the_same_record() {
    // The spelling a push records is `Standing::spelled`'s, so a path handed in
    // with the root's own separators and a `./` in front of it is the same
    // record rather than a second one.
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    pull_from(repo.path(), &linear, "./docs/brief.md").expect("the same brief, spelled twice");

    assert_eq!(linear.variables(), [json!({ "id": PROJECT_ID })]);
}

#[test]
fn a_brief_no_record_names_is_refused_with_the_path_and_the_command_that_files_it() {
    let repo = a_repository();

    let error = refusal(pull_from(repo.path(), &Unreachable, "docs/other.md"));

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
    // No `.warlock/filed.toml` on disk, which `records` reads as an empty one:
    // a repository that has never pushed records no project, and that is an
    // answer rather than a failure to reach one.
    let repo = a_dir();

    let error = refusal(pull_from(repo.path(), &Unreachable, BRIEF_PATH));

    assert!(matches!(error, Error::NoRecord { .. }), "{error:?}");
    assert!(!filed_path(repo.path()).exists());
}

#[test]
fn a_project_id_the_api_does_not_know_names_the_id_and_the_file_it_is_written_in() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(json!({ "project": null }))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

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
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Backlog")))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert!(
        matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some("Backlog")),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains("Backlog"), "{message}");
    assert!(message.contains("Planned"), "{message}");
}

#[test]
fn a_project_with_no_status_at_all_is_refused_and_says_it_has_none() {
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(None))]);

    let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

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
    let repo = a_repository();

    for accepted in ["Planned", "planned", " Planned ", "PLANNED", "\tplanned\n"] {
        let linear = Posting::answering([Ok(a_project(Some(accepted)))]);

        pull_from(repo.path(), &linear, BRIEF_PATH)
            .unwrap_or_else(|error| panic!("`{accepted}` is `Planned`: {error:?}"));
    }

    for refused in [
        "Backlog",
        "In Progress",
        "Plan",
        "Planned later",
        "",
        "unplanned",
    ] {
        let linear = Posting::answering([Ok(a_project(Some(refused)))]);

        let error = refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

        assert!(
            matches!(&error, Error::NotPlanned { status, .. } if status.as_deref() == Some(refused)),
            "`{refused}` was read as `Planned`: {error:?}"
        );
    }
}

#[test]
fn a_status_that_is_not_planned_stops_the_sequence_where_it_stands() {
    // One answer in the stand-in and one request made: anything read or sent
    // after the status said `Backlog` would be a second `post`, which this
    // stand-in panics on.
    let repo = a_repository();
    let linear = Posting::answering([Ok(a_project(Some("Backlog")))]);

    refusal(pull_from(repo.path(), &linear, BRIEF_PATH));

    assert_eq!(linear.documents().len(), 1, "{:?}", linear.documents());
}

#[test]
fn nothing_on_this_path_writes_to_the_board_or_to_the_record_file() {
    let repo = a_repository();
    let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");
    let linear = Posting::answering([Ok(a_project(Some("Planned")))]);

    pull_from(repo.path(), &linear, BRIEF_PATH).expect("a planned project is read");

    for asked in linear.documents() {
        assert!(
            !asked.contains("mutation"),
            "a mutation was issued: {asked}"
        );
        assert!(!asked.contains("Create"), "a mutation was issued: {asked}");
        assert!(!asked.contains("Update"), "a mutation was issued: {asked}");
    }
    assert_eq!(
        fs::read_to_string(filed_path(repo.path())).expect("a record file"),
        before,
        "a read rewrote the record file"
    );
}

#[test]
fn a_record_file_that_will_not_read_is_a_failure_rather_than_an_empty_one() {
    // The reading half of `push.rs`'s rule: an unreadable record file is a
    // repository whose filings are unknown, not one with none, and answering
    // "nothing is filed" here would send somebody back to `warlock push` for a
    // brief that already has a project.
    let repo = a_repository();
    fs::write(filed_path(repo.path()), "version = 2\nnot toml {{{").expect("a broken record file");

    let error = refusal(pull_from(repo.path(), &Unreachable, BRIEF_PATH));

    assert!(matches!(error, Error::Filed { .. }), "{error:?}");
}

#[test]
fn a_dry_run_reports_the_project_its_status_and_every_slice_then_stops() {
    let repo = a_scoped_repository();
    let home = a_home(repo.path());
    let linear = a_sliced_project(SLICED);
    let before = fs::read_to_string(filed_path(repo.path())).expect("a record file");

    let (outcome, printed) = pull_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        None,
        true,
        opening(&linear),
    );

    outcome.expect("a dry run answers");
    // The one request that got the project, and no second one: a dry run reads
    // what it reports and spends nothing else.
    assert_eq!(
        linear.documents().len(),
        1,
        "a dry run sent more than the fetch: {:?}",
        linear.documents()
    );
    assert_eq!(linear.variables(), [json!({ "id": PROJECT_ID })]);
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
    // The key value reaches the client and nothing else, here as everywhere.
    assert!(!printed.contains(NOT_A_KEY), "{printed}");
}

#[test]
fn a_dry_run_lists_the_slices_in_the_order_they_would_be_cut_in() {
    // A scope whose last slice is what the first two wait on: the cut order and
    // the document order differ, and what is printed is the order the tickets
    // would be filed in.
    let repo = a_scoped_repository();
    let home = a_home(repo.path());
    let linear = a_sliced_project(OUT_OF_ORDER);

    let (outcome, printed) = pull_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        None,
        true,
        opening(&linear),
    );

    outcome.expect("a dry run answers");
    let lines = lines(&printed);
    assert!(
        placed(&lines, THIRD) < placed(&lines, FIRST),
        "the document order was printed rather than the cut order: {lines:?}"
    );
    assert!(placed(&lines, FIRST) < placed(&lines, SECOND), "{lines:?}");
    // The position in the document travels with each of them, so a reader can
    // find the slice in the brief the project was filed from.
    assert!(lines[1].contains("slice 3"), "{lines:?}");
    assert!(lines[1].contains("[1/3]"), "{lines:?}");
}

#[test]
fn a_slice_a_record_already_names_is_reported_as_cut_with_the_issues_it_became() {
    let repo = a_scoped_repository();
    recording_cuts(repo.path(), [a_cut(FIRST, &["WAR-1", "WAR-2"])]);
    let home = a_home(repo.path());
    let linear = a_sliced_project(SLICED);

    let (outcome, printed) = pull_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        None,
        true,
        opening(&linear),
    );

    outcome.expect("two slices are left to cut");
    let lines = lines(&printed);
    assert!(lines[0].contains("1 already cut"), "{lines:?}");
    let cut = &lines[placed(&lines, FIRST)];
    assert!(cut.contains("already cut"), "{cut}");
    assert!(cut.contains("WAR-1") && cut.contains("WAR-2"), "{cut}");
    // And the slices with no record are named without one.
    assert!(
        !lines[placed(&lines, SECOND)].contains("already cut"),
        "{lines:?}"
    );
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

    let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, opening(&linear));

    assert!(
        matches!(&error, Error::AllCut { path } if path == BRIEF_PATH),
        "{error:?}"
    );
    let message = said(&error);
    assert!(message.contains(BRIEF_PATH), "{message}");
    assert!(message.contains(".warlock/filed.toml"), "{message}");
    // Refused where the answer that said so arrived: the fetch and nothing
    // after it, and the record file as it was.
    assert_eq!(linear.documents().len(), 1, "{:?}", linear.documents());
    assert_eq!(
        fs::read_to_string(filed_path(repo.path())).expect("a record file"),
        before
    );
}

#[test]
fn a_project_with_no_scope_block_is_refused_in_the_parser_s_own_words() {
    let repo = a_scoped_repository();
    let home = a_home(repo.path());
    let linear = a_sliced_project(NO_SCOPE);

    let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, opening(&linear));

    assert!(matches!(error, Error::ScopeBlock { .. }), "{error:?}");
    // The parser's sentence rather than one of this module's, which is what
    // carrying the source is for.
    assert_eq!(said(&error), ScopeBlockError::NoScope.to_string());
}

#[test]
fn slices_that_wait_on_each_other_are_refused_with_both_of_them_named() {
    let repo = a_scoped_repository();
    let home = a_home(repo.path());
    let linear = a_sliced_project(CIRCLE);

    let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, opening(&linear));

    assert!(matches!(error, Error::ScopeBlock { .. }), "{error:?}");
    let message = said(&error);
    assert!(
        message.contains(FIRST) && message.contains(SECOND),
        "{message}"
    );
    assert!(message.contains("wait on each other"), "{message}");
}

#[test]
fn a_project_that_is_not_planned_stops_the_whole_verb_where_it_stands() {
    let repo = a_scoped_repository();
    let home = a_home(repo.path());
    let linear = Posting::answering([Ok(a_project_of(SLICED, Some("Backlog")))]);

    let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, opening(&linear));

    assert!(matches!(error, Error::NotPlanned { .. }), "{error:?}");
    // Nothing is read or sent after the status: the scope block in the content
    // that came back is never parsed, and no second request is made.
    assert_eq!(linear.documents().len(), 1, "{:?}", linear.documents());
}

#[test]
fn a_brief_no_record_names_refuses_before_the_key_is_read() {
    // The whole verb rather than its reading half: the board is resolved first,
    // and the record refusal still happens with nothing sent.
    let repo = a_scoped_repository();
    let home = a_home(repo.path());

    let error = refusing(repo.path(), home.path(), "docs/other.md", None, |_key| {
        Unreachable
    });

    assert!(matches!(error, Error::NoRecord { .. }), "{error:?}");
    assert!(said(&error).contains("warlock push"), "{}", said(&error));
}

#[test]
fn a_machine_with_no_board_to_pull_from_is_refused_before_anything_is_read() {
    // The three ways there is no candidate, which `push` refuses in the same
    // three sentences: this asserts they are that command's words and not a
    // second set worded here.
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
        let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, no_socket);

        assert!(matches!(error, Error::Filing { .. }), "{error:?}");
        assert!(said(&error).contains(expected), "{}", said(&error));
    }
}

#[test]
fn a_machine_that_can_pull_from_several_boards_names_them_all_and_asks_for_one() {
    let repo = a_repository();
    saving(
        repo.path(),
        &a_manifest([a_record(SCOPE, TEAM), a_record("web", "WEB")]),
    );
    let home = a_home_holding(repo.path(), &[SCOPE, "web"]);

    let error = refusing(repo.path(), home.path(), BRIEF_PATH, None, no_socket);

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

    // Honoured: the named candidate's team is the board the dry run reports,
    // and the other one is nowhere in it.
    let linear = a_sliced_project(SLICED);
    let (outcome, printed) = pull_to(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        Some("web"),
        true,
        opening(&linear),
    );
    outcome.expect("a named candidate is a board");
    assert!(printed.contains("WEB"), "{printed}");
    assert!(!printed.contains(SCOPE), "{printed}");

    // And a name that is not one of them is refused with both of them named,
    // with no key read and nothing sent.
    let error = refusing(
        repo.path(),
        home.path(),
        BRIEF_PATH,
        Some("billing"),
        no_socket,
    );

    let message = said(&error);
    assert!(matches!(error, Error::Filing { .. }), "{error:?}");
    assert!(message.contains("billing"), "{message}");
    assert!(
        message.contains(SCOPE) && message.contains("web"),
        "{message}"
    );
}
