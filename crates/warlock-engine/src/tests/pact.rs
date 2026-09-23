use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    DOCUMENT_FILE, Failure, Observer, PactedSubtree, Pacting, Refusal, Unwatched, pact_directory,
    pact_subtree, pactable_directories, refresh_subtree, unpact_ignored, unpact_subtree,
};
use crate::document::{self, STAMP};
use crate::fitting::{Omission, Snapshot};
use crate::ignores;
use crate::{
    Agent, Loaded, Manifest, NodeState, PactEntry, ScopeRecord, agent, decide_state,
    from_manifest_path, load_tree, manifest, subtree_hash,
};

struct Canned {
    seen: std::cell::RefCell<Vec<agent::Request>>,
}

impl Canned {
    fn filling() -> Self {
        Self {
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Agent for Canned {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        self.seen.borrow_mut().push(request.clone());
        Ok(agent::Response::new(document::stub_answer(request)))
    }
}

struct Fails(fn() -> agent::Error);

impl Agent for Fails {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        Err(self.0())
    }
}

fn written(dir: &Path) -> Option<Vec<u8>> {
    fs::read(dir.join("WARLOCK.md")).ok()
}

fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
    fs::write(&path, contents).expect("writes a file");
    path
}

// The pass that happens once per directory, whatever its files cost. A
// directory is described by one synthesis pass and as many per-file passes
// as it has files that moved, so counting anything else counts files.
fn is_document_pass(request: &agent::Request) -> bool {
    request.prompt().starts_with(document::SYNTHESIS_PROMPT)
}

fn modules(manifest: &Manifest) -> Vec<&str> {
    manifest.entries().iter().map(PactEntry::module).collect()
}

fn scopes(manifest: &Manifest) -> Vec<(&str, Option<&str>)> {
    manifest
        .entries()
        .iter()
        .map(|entry| (entry.module(), entry.scope()))
        .collect()
}

fn with_scopes(manifest: &Manifest, scoped: &[(&str, &str)]) -> Manifest {
    for (module, _) in scoped {
        assert!(
            manifest.entry(module).is_some(),
            "`{module}` is not pacted, so nothing can scope it",
        );
    }
    manifest.rebuilt_with(manifest.entries().iter().map(|entry| {
        match scoped.iter().find(|(module, _)| *module == entry.module()) {
            Some((_, scope)) => entry.clone().with_scope(*scope),
            None => entry.clone(),
        }
    }))
}

fn file_paths(request: &agent::Request) -> Vec<&str> {
    request.files().iter().map(agent::File::path).collect()
}

#[test]
fn a_pact_leaves_nothing_behind_but_the_document() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "lib.rs", "//! Core engine.\n");

    pact_directory(dir.path(), &Canned::filling()).expect("pacts");

    let mut left = fs::read_dir(dir.path())
        .expect("lists")
        .map(|entry| entry.expect("an entry").file_name())
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(
        left,
        ["WARLOCK.md", "lib.rs"],
        "no temporary file leaks into the directory the pact just described",
    );
}

#[cfg(unix)]
#[test]
fn a_document_that_cannot_be_written_names_itself_and_leaves_the_old_one() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("a temporary directory");
    let before = "# engine\n\nWhat it used to say.\n";
    write(dir.path(), DOCUMENT_FILE, before);
    // Readable and listable, so the gather still works, but nothing new can
    // be created in it — neither the temporary nor a rename over the
    // document.
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("chmods");
    if fs::write(dir.path().join("probe"), "").is_ok() {
        // Running as root: no directory is unwritable, so there is nothing
        // here to assert against.
        fs::remove_file(dir.path().join("probe")).expect("removes the probe");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmods back");
        return;
    }

    let error = pact_directory(dir.path(), &Canned::filling())
        .expect_err("a read-only directory takes no document");

    match &error {
        super::Error::Write { path, .. } => {
            assert_eq!(
                path,
                &dir.path().join(DOCUMENT_FILE),
                "the document, not the temporary"
            );
        }
        other => panic!("expected a write failure, got {other:?}"),
    }
    assert_eq!(error.directory(), dir.path());
    assert_eq!(
        written(dir.path()).as_deref(),
        Some(before.as_bytes()),
        "the write is atomic, so a failure leaves the old document whole",
    );

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmods back");
    let mut left = fs::read_dir(dir.path())
        .expect("lists")
        .map(|entry| entry.expect("an entry").file_name())
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(
        left,
        [DOCUMENT_FILE],
        "and no temporary behind on the failure path"
    );
}

#[test]
fn an_agent_that_fails_writes_nothing_and_names_the_directory() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "lib.rs", "//! Core engine.\n");
    let agent = Fails(|| agent::Error::Failed {
        code: Some(2),
        stderr: "Invalid API key\n".to_owned(),
    });

    let error = pact_directory(dir.path(), &agent).expect_err("a failed pass is no document");

    assert!(
        matches!(
            error,
            super::Error::Refused {
                cause: Refusal::Agent {
                    source: agent::Error::Failed { code: Some(2), .. }
                },
                ..
            }
        ),
        "{error:?}",
    );
    assert_eq!(error.directory(), dir.path());
    assert_eq!(
        written(dir.path()),
        None,
        "a directory with no document still has none",
    );
}

// The mend: the floor under an exhausted attempt loop.

// A pass that answers with the right shape and the same slot wrong every
// time, however often it is asked. Four attempts that change nothing is
// the only road to the mend: it is what is left when the asking has run
// out, not a substitute for asking.
struct Defective {
    break_it: fn(&mut serde_json::Map<String, serde_json::Value>),
    seen: std::cell::RefCell<Vec<agent::Request>>,
}

impl Defective {
    fn with(break_it: fn(&mut serde_json::Map<String, serde_json::Value>)) -> Self {
        Self {
            break_it,
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Agent for Defective {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        self.seen.borrow_mut().push(request.clone());
        let answer = document::stub_answer(request);
        if !is_document_pass(request) {
            return Ok(agent::Response::new(answer));
        }
        let mut parsed: serde_json::Value =
            serde_json::from_str(&answer).expect("a document pass is answered with an object");
        (self.break_it)(parsed.as_object_mut().expect("a fill is an object"));
        Ok(agent::Response::new(parsed.to_string()))
    }
}

fn blank_purpose(answer: &mut serde_json::Map<String, serde_json::Value>) {
    answer.insert(
        "purpose".to_owned(),
        serde_json::Value::String(String::new()),
    );
}

const OVERLONG: usize = document::ENTRY_CHARS + 120;

// Two wrong slots of two different kinds: one value warlock can cut back
// to its cap out of the answer's own text, and one it has to fill in from
// what it measured itself.
fn blank_purpose_and_overlong_entries(answer: &mut serde_json::Map<String, serde_json::Value>) {
    blank_purpose(answer);
    let files = answer
        .get_mut("files")
        .and_then(serde_json::Value::as_object_mut)
        .expect("a fill holds an entry per file");
    for value in files.values_mut() {
        *value = serde_json::Value::String("x".repeat(OVERLONG));
    }
}

fn named(root: &Path, path: &Path) -> String {
    relative_to(root, std::slice::from_ref(&path.to_path_buf()))
        .pop()
        .expect("one directory in, one name out")
}

#[derive(Default)]
struct Mending {
    rejections: Vec<(PathBuf, usize)>,
    repairs: Vec<(PathBuf, document::Mend)>,
}

impl Mending {
    fn turned_down(&self, root: &Path) -> Vec<(String, usize)> {
        self.rejections
            .iter()
            .map(|(directory, attempt)| (named(root, directory), *attempt))
            .collect()
    }
}

impl Observer for Mending {
    fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
        Pacting::Continue
    }

    fn rejected(
        &mut self,
        directory: &Path,
        _defects: &[document::Defect],
        attempt: usize,
        _attempts: usize,
    ) {
        self.rejections.push((directory.to_path_buf(), attempt));
    }

    fn repaired(&mut self, directory: &Path, mend: &document::Mend) {
        self.repairs.push((directory.to_path_buf(), mend.clone()));
    }
}

struct Lining {
    answer: String,
    passes: std::cell::Cell<usize>,
}

impl Lining {
    fn saying(answer: impl Into<String>) -> Self {
        Self {
            answer: answer.into(),
            passes: std::cell::Cell::new(0),
        }
    }
}

impl Agent for Lining {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        self.passes.set(self.passes.get() + 1);
        Ok(agent::Response::new(self.answer.clone()))
    }
}

fn taken(dir: &Path) -> Snapshot {
    Snapshot::take(dir).expect("walks")
}

#[test]
fn a_line_is_reused_only_where_the_file_has_not_moved() {
    struct Counting {
        passes: std::cell::Cell<usize>,
    }

    impl Agent for Counting {
        fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
            self.passes.set(self.passes.get() + 1);
            Ok(agent::Response::new(
                r#"{"line": "A line about one file alone."}"#,
            ))
        }
    }

    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("reading.rs"), "pub fn read_one() {}\n").expect("writes");
    fs::write(dir.path().join("writing.rs"), "fn scratch() {}\n").expect("writes");
    let agent = Counting {
        passes: std::cell::Cell::new(0),
    };

    // Nothing recorded: every file is asked about.
    let first = taken(dir.path())
        .assemble(None, &agent, &mut Unwatched)
        .expect("lines");
    assert_eq!(agent.passes.get(), 2);
    assert_eq!(first.asked, ["reading.rs", "writing.rs"]);

    // The page and the hashes from that run, and one file changed under them.
    fs::write(dir.path().join("writing.rs"), "fn scratch(at: usize) {}\n").expect("writes");
    let page = page_of(
        &first
            .lines
            .iter()
            .map(|(path, line)| (path.as_str(), line.as_str()))
            .collect::<Vec<_>>(),
    );

    let again = taken(dir.path())
        .assemble(Some((&page, &first.hashes)), &agent, &mut Unwatched)
        .expect("lines");
    assert_eq!(agent.passes.get(), 3, "one changed file, one pass");
    assert_eq!(again.asked, ["writing.rs"]);
    assert_eq!(again.lines["reading.rs"], first.lines["reading.rs"]);
    assert_eq!(
        again.hashes["writing.rs"],
        crate::hash::line_hash(
            &crate::file_hash(dir.path().join("writing.rs")).expect("hashes"),
            &again.lines["writing.rs"],
        ),
        "the file as it stands and the line now on the page, together",
    );
}

#[test]
fn one_file_is_one_pass_and_one_line() {
    struct Lined;

    impl Agent for Lined {
        fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
            assert!(request.prompt().starts_with(document::FILE_PROMPT));
            assert_eq!(request.files().len(), 1, "one file, one pass");
            Ok(agent::Response::new(
                r#"{"line": "The reading half: one entry point and the type it hands back."}"#,
            ))
        }
    }

    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("reading.rs"), "pub fn read_one() {}\n").expect("writes");

    let described = taken(dir.path())
        .line("reading.rs", &Lined, &mut Unwatched)
        .expect("a line");
    assert_eq!(
        described.line,
        "The reading half: one entry point and the type it hands back."
    );
    assert!(
        !described.mended,
        "the pass answered, so warlock wrote nothing"
    );
}

fn one_file_directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "reading.rs",
        "pub fn read_one() -> Reader { Reader }\n",
    );
    dir
}

fn page_of(lines: &[(&str, &str)]) -> String {
    let rows: Vec<String> = lines
        .iter()
        .map(|(path, line)| format!("- `{path}` (1 B) — {line}"))
        .collect();
    format!("\n## Files\n\n{}\n", rows.join("\n"))
}

#[test]
fn synthesis_is_shown_the_lines_and_checked_against_the_directory() {
    // The pass sees no source at all, so every name it uses is one the
    // request cannot vouch for. What makes the answer checkable is
    // warlock's own walk, and what makes this test worth having is that the
    // answer names `read_one` — a symbol in the file and in no line.
    let dir = one_file_directory();
    let agent = Lining::saying(
        r#"{"purpose": "A directory of one reading file, for the tests below it.",
                "structure": [{"line": "`reading.rs` is the only file here.", "names": ["reading.rs"]},
                              {"line": "`read_one` reads a single record.", "names": ["read_one"]}]}"#,
    );
    let lines = [(
        "reading.rs".to_owned(),
        "The reading half of the fixture.".to_owned(),
    )]
    .into_iter()
    .collect();

    let synthesised = taken(dir.path())
        .fill(&lines, &agent, &mut Unwatched)
        .expect("a fill either way");

    assert_eq!(agent.passes.get(), 1, "a clean answer is taken at once");
    assert!(synthesised.mends.is_empty(), "{:?}", synthesised.mends);
    assert_eq!(
        synthesised.fill.files, lines,
        "the lines are the caller's and pass through untouched",
    );
    assert_eq!(
        synthesised.fill.structure[1].names,
        ["read_one"],
        "a symbol no line spells is still checkable against the walk",
    );
}

#[test]
fn synthesis_that_cannot_be_got_right_is_mended_rather_than_lost() {
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"purpose": "", "structure": []}"#);
    let lines = [(
        "reading.rs".to_owned(),
        "The reading half of the fixture.".to_owned(),
    )]
    .into_iter()
    .collect();

    let synthesised = taken(dir.path())
        .fill(&lines, &agent, &mut Unwatched)
        .expect("a fill either way");

    assert_eq!(agent.passes.get(), document::ATTEMPTS);
    assert_eq!(
        synthesised
            .mends
            .iter()
            .map(|mend| mend.field.as_str())
            .collect::<Vec<_>>(),
        ["purpose"],
        "{:?}",
        synthesised.mends,
    );
    assert!(!synthesised.fill.purpose.is_empty());
}

#[test]
fn a_hash_with_no_line_on_the_page_is_asked_about_again() {
    // The document was edited by hand, or written by a warlock that did not
    // record lines. The hash says the file has not moved and there is still
    // nothing to reuse, so the pass runs.
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
    let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
    let recorded = [("reading.rs".to_owned(), hash)].into_iter().collect();

    let assembled = taken(dir.path())
        .assemble(Some(("", &recorded)), &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(assembled.asked, ["reading.rs"]);
    assert!(assembled.kept.is_empty());
    assert_eq!(agent.passes.get(), 1);
}

#[test]
fn a_line_with_no_hash_behind_it_is_asked_about_again() {
    // The other half of the same rule: a page says what the file was, and
    // nothing says the file still is that. Trusting the page here is how a
    // document outlives the code it describes.
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
    let page = page_of(&[("reading.rs", "The line already on the page.")]);

    let assembled = taken(dir.path())
        .assemble(Some((&page, &BTreeMap::new())), &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(assembled.asked, ["reading.rs"]);
    assert_eq!(agent.passes.get(), 1);
    assert_eq!(
        assembled.lines["reading.rs"], "A line about one file alone.",
        "the answer, not the page",
    );
}

#[test]
fn a_file_the_page_and_the_hashes_agree_on_costs_nothing() {
    const LINE: &str = "The line already on the page.";

    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "A line no pass should be asked for."}"#);
    let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
    let recorded = [("reading.rs".to_owned(), crate::hash::line_hash(&hash, LINE))]
        .into_iter()
        .collect();
    let page = page_of(&[("reading.rs", LINE)]);

    let assembled = taken(dir.path())
        .assemble(Some((&page, &recorded)), &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(agent.passes.get(), 0, "the run paid for nothing");
    assert_eq!(assembled.kept, ["reading.rs"]);
    assert!(assembled.asked.is_empty());
    assert_eq!(assembled.lines["reading.rs"], LINE);
    assert_eq!(
        assembled.hashes["reading.rs"],
        crate::hash::line_hash(&hash, LINE),
        "recorded again as it stands"
    );
}

#[test]
fn a_line_edited_on_the_page_is_described_again_however_still_its_file_is() {
    // The reason the digest binds the two. The file has not moved, so the
    // old rule would have kept whatever the page said; the line is not the
    // one warlock recorded, so it is bought again and the edit is gone from
    // the document without a word about it.
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "The line a pass wrote."}"#);
    let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
    let recorded = [(
        "reading.rs".to_owned(),
        crate::hash::line_hash(&hash, "The line a pass wrote."),
    )]
    .into_iter()
    .collect();
    let page = page_of(&[("reading.rs", "maintained by a unicorn, actually")]);

    let assembled = taken(dir.path())
        .assemble(Some((&page, &recorded)), &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(agent.passes.get(), 1, "the edited line costs one pass");
    assert_eq!(assembled.asked, ["reading.rs"]);
    assert!(assembled.kept.is_empty());
    assert_eq!(assembled.lines["reading.rs"], "The line a pass wrote.");
}

// The end-to-end half of `a_line_edited_on_the_page_is_described_again`:
// that one settles `Snapshot::assemble`, this one settles that a real refresh
// reaches it. A refresh is the path that reuses lines at all — a pact runs
// under `AboveFailure::Describe` and re-describes everything regardless —
// so a refresh that kept the edit is the way this could regress without a
// single other test noticing.
#[test]
fn a_hand_edited_line_survives_neither_a_refresh_nor_a_pact() {
    const LIE: &str = "maintained by a unicorn that files its own taxes";

    fn tamper(src: &Path) {
        let page = String::from_utf8(written(src).expect("a document")).expect("utf-8");
        let edited: Vec<String> = page
            .lines()
            .map(|line| match line.split_once(" — ") {
                Some((head, _)) if head.starts_with("- `lib.rs`") => {
                    format!("{head} — {LIE}")
                }
                _ => line.to_owned(),
            })
            .collect();
        let edited = edited.join("\n") + "\n";
        assert!(edited.contains(LIE), "the planted line went in: {edited}");
        fs::write(src.join("WARLOCK.md"), edited).expect("writes the edited document");
    }

    let repo = project();
    let src = repo.path().join("crates/engine/src");

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &src,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");
    assert!(failures.is_empty(), "{failures:?}");

    tamper(&src);

    let PactedSubtree {
        manifest: refreshed,
        failures,
        ..
    } = refresh_subtree(
        &src,
        repo.path(),
        &manifest,
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("refreshes");
    assert!(failures.is_empty(), "{failures:?}");

    let after_refresh = String::from_utf8(written(&src).expect("a document")).expect("utf-8");
    assert!(
        !after_refresh.contains(LIE),
        "the recorded digest covers the line as well as the file, so an \
             edited line does not match and is described again: {after_refresh}",
    );
    assert_eq!(
        state(&refreshed, repo.path(), "crates/engine/src"),
        NodeState::PactedFresh,
        "and what is granted fresh is a document warlock wrote every line of",
    );

    tamper(&src);

    let PactedSubtree { failures, .. } = pact_subtree(
        &src,
        repo.path(),
        &refreshed,
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts again");
    assert!(failures.is_empty(), "{failures:?}");

    let after_pact = String::from_utf8(written(&src).expect("a document")).expect("utf-8");
    assert!(
        !after_pact.contains(LIE),
        "a pact reuses nothing, so the planted line is written over: \
             {after_pact}",
    );
}

#[test]
fn a_line_for_a_file_that_is_gone_is_left_off_the_page() {
    // The page outlives the directory, so a deleted file's line is still
    // sitting in it with a hash still recorded against it. Assembly walks
    // the snapshot and not the page, which is what keeps a document from
    // describing a file that is not there — and the whole point of a map is
    // that everything on it can be opened.
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "A line about the file that is left."}"#);
    let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
    let gone = "writing.rs";
    let recorded = [
        (
            "reading.rs".to_owned(),
            crate::hash::line_hash(&hash, "The line already on the page."),
        ),
        (
            gone.to_owned(),
            "a hash for a file nobody deleted it with".to_owned(),
        ),
    ]
    .into_iter()
    .collect();
    let page = page_of(&[
        ("reading.rs", "The line already on the page."),
        (gone, "A line about a file that has since been deleted."),
    ]);

    let assembled = taken(dir.path())
        .assemble(Some((&page, &recorded)), &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(agent.passes.get(), 0, "the file that is left was reused");
    assert_eq!(assembled.kept, ["reading.rs"]);
    assert!(
        !assembled.lines.contains_key(gone),
        "the deleted file's line is gone from the page: {:?}",
        assembled.lines,
    );
    assert!(
        !assembled.hashes.contains_key(gone),
        "and its hash is gone from the manifest with it: {:?}",
        assembled.hashes,
    );
}

#[test]
fn a_file_warlock_had_to_write_itself_is_named_as_such() {
    let dir = one_file_directory();
    let agent = Lining::saying("prose where an object was asked for");

    let assembled = taken(dir.path())
        .assemble(None, &agent, &mut Unwatched)
        .expect("lines");

    assert_eq!(assembled.asked, ["reading.rs"]);
    assert_eq!(assembled.mended, ["reading.rs"]);
    assert!(assembled.lines["reading.rs"].contains("reading.rs"));
}

#[test]
fn a_file_whose_line_is_never_usable_is_written_by_warlock_rather_than_refused() {
    let dir = one_file_directory();
    let over = "x".repeat(document::ENTRY_CHARS + 40);
    let agent = Lining::saying(format!("{{\"line\": \"{over}\"}}"));

    let described = taken(dir.path())
        .line("reading.rs", &agent, &mut Unwatched)
        .expect("a line either way");

    assert_eq!(
        agent.passes.get(),
        document::ATTEMPTS,
        "asked in full first"
    );
    assert!(described.mended);
    assert!(
        described.line.contains("reading.rs"),
        "{:?}",
        described.line
    );
    assert!(
        described.line.contains("read_one"),
        "the fallback is the file's own declared names: {:?}",
        described.line,
    );
}

#[test]
fn a_file_answered_with_prose_is_mended_where_a_directory_would_be_refused() {
    // `pact_directory` refuses an answer that was never an object, because
    // there is no slot in prose to repair from and the whole document is at
    // stake. One file is not: warlock knows its name, its size and what it
    // declares, so losing the directory over one file's punctuation is the
    // trade brief 16 was written against.
    let dir = one_file_directory();
    let agent = Lining::saying("Here is some prose instead of the object you asked for.");

    let described = taken(dir.path())
        .line("reading.rs", &agent, &mut Unwatched)
        .expect("a line either way");

    assert!(described.mended);
    assert!(
        described.line.contains("reading.rs"),
        "{:?}",
        described.line
    );
}

#[test]
fn a_file_that_is_not_there_is_an_error_and_not_a_line_about_nothing() {
    let dir = one_file_directory();
    let agent = Lining::saying(r#"{"line": "a line about a file that does not exist"}"#);

    let error = taken(dir.path())
        .line("writing.rs", &agent, &mut Unwatched)
        .expect_err("the caller walked the directory to get this name");

    assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
    assert_eq!(agent.passes.get(), 0, "nothing was asked");
}

#[test]
fn a_mended_directory_is_not_a_failure_and_the_subtree_is_still_pacted() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let agent = Defective::with(blank_purpose);

    let PactedSubtree {
        manifest,
        failures,
        repairs,
        ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut Unwatched,
    )
    .expect("a subtree of mended passes is a pacted subtree");

    assert!(
        failures.is_empty(),
        "a directory warlock mended was documented, hashed and granted, so it \
             is a note about the run and not a failure in it: {failures:?}",
    );
    let mended: Vec<PathBuf> = repairs
        .iter()
        .map(|repaired| repaired.directory.clone())
        .collect();
    for module in [
        "crates/engine/tests",
        "crates/engine/src/inner",
        "crates/engine/src",
        "crates/engine",
    ] {
        let directory = from_manifest_path(repo.path(), module);
        assert!(written(&directory).is_some(), "`{module}` is documented");
        assert!(
            manifest
                .entry(module)
                .and_then(PactEntry::granted_hash)
                .is_some(),
            "`{module}` is granted, the same as a clean pass would leave it",
        );
        assert!(
            failures
                .iter()
                .all(|failure| failure.directory() != directory),
            "and `{module}` is in no failure: {failures:?}",
        );
        assert!(
            mended.contains(&directory),
            "`{module}` was mended, which is why this is worth asserting: {repairs:?}",
        );
    }
}

#[test]
fn every_mend_is_carried_out_of_the_run_and_announced_as_it_is_made() {
    let repo = project();
    let src = repo.path().join("crates/tui/src");
    let agent = Defective::with(blank_purpose_and_overlong_entries);
    let mut observer = Mending::default();

    let PactedSubtree {
        failures, repairs, ..
    } = pact_subtree(&src, repo.path(), &Manifest::new(), &agent, &mut observer).expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        observer.turned_down(repo.path()),
        (1..=document::ATTEMPTS)
            .map(|attempt| ("crates/tui/src".to_owned(), attempt))
            .collect::<Vec<_>>(),
        "the loop ran out first, and said so each time",
    );

    let carried: Vec<(String, String, document::Mended)> = repairs
        .iter()
        .map(|repaired| {
            (
                named(repo.path(), &repaired.directory),
                repaired.mend.field.clone(),
                repaired.mend.done,
            )
        })
        .collect();
    assert_eq!(
        carried,
        [(
            "crates/tui/src".to_owned(),
            "purpose".to_owned(),
            document::Mended::Supplied,
        )],
        "one value per slot, naming the directory and the slot in `Defect`'s own spelling",
    );
}

#[test]
fn every_document_opens_by_saying_it_is_a_map_and_not_a_specification() {
    // The misreading this exists to head off: a document treated as the
    // specification of a directory, so that what it does not mention is
    // taken not to exist. Warlock writes the correction rather than asking
    // a pass for it, so it is the same in every document and cannot be
    // reworded, shortened or dropped under a long request.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "lib.rs", "//! Core engine.\n");

    pact_directory(dir.path(), &Canned::filling()).expect("pacts");

    let written = String::from_utf8(written(dir.path()).expect("a document")).expect("text");
    assert!(written.starts_with(STAMP), "{written}");
    assert!(
        written.contains("check anything you are about to rely on against the files"),
        "the reader is told to verify: {written}"
    );
    assert!(
        written.contains("the code is right"),
        "and told which side wins when it does not match: {written}"
    );
    assert_eq!(
        written.matches("<!-- warlock -->").count(),
        1,
        "and it is there exactly once"
    );
}

#[test]
fn a_pass_is_sent_its_childrens_documents_and_none_of_their_source() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "Cargo.toml", "[package]\n");
    write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");
    write(
        dir.path(),
        "src/lib.rs",
        "//! Not for the parent to read.\n",
    );
    write(dir.path(), "tests/it.rs", "#[test] fn works() {}\n");
    let agent = Canned::filling();

    pact_directory(dir.path(), &agent).expect("pacts");

    let seen = agent.seen.borrow();
    // The synthesis pass and not the first: a per-file pass is shown one
    // file and no child at all, and `## Directories` is written from a
    // child's own document by the pass that writes the directory's slots.
    let synthesis = seen
        .iter()
        .find(|request| is_document_pass(request))
        .expect("a directory is synthesised");
    assert_eq!(
        synthesis
            .child_documents()
            .iter()
            .map(|child| (child.directory(), child.text()))
            .collect::<Vec<_>>(),
        [("src", "# src\n\nThe code.\n")],
        "the child describes itself; `tests/` has no document and \
             contributes no entry, which is not an error",
    );
    assert_eq!(
        file_paths(&seen[0]),
        ["Cargo.toml"],
        "and the child's source is not a file of the parent",
    );
    assert!(
        !format!("{:?}", seen[0]).contains("Not for the parent to read"),
        "nor is it anywhere else in the request",
    );
}

#[test]
fn a_directory_that_cannot_be_gathered_never_reaches_the_agent() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("nowhere");
    let agent = Canned::filling();

    let error = pact_directory(&missing, &agent).expect_err("there is nothing to walk");

    assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
    assert_eq!(
        error.directory(),
        missing,
        "a walk that failed still says which directory it was",
    );
    assert!(
        agent.seen.borrow().is_empty(),
        "no request, no pass: the expensive half never runs",
    );
}

fn repository() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    write(repo.path(), ".git/config", "[core]\n");
    write(repo.path(), ".gitignore", "/target\ngenerated/\n");
    write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
    for dir in [
        "crates/engine/src/inner",
        "crates/engine/tests",
        "crates/engine/generated/schema",
        "crates/engine/.hidden/cache",
        "crates/engine/.warlock",
        "crates/tui/src",
        "target/debug",
    ] {
        fs::create_dir_all(repo.path().join(dir)).expect("creates a directory");
    }
    repo
}

fn relative_to(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .expect("every directory sits under the root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn a_subtree_is_exactly_the_directories_the_loader_makes_nodes_of() {
    let repo = repository();
    let subtree = repo.path().join("crates/engine");

    let pacted = pactable_directories(&subtree).expect("walks");

    // The loader is the authority on which directories exist, because it is
    // what the user is looking at when they press the key. Compared as sets,
    // since the two orders are deliberately opposite.
    let Loaded { tree, problems, .. } = load_tree(&subtree).expect("loads");
    assert!(problems.is_empty(), "{problems:?}");
    let mut walked: Vec<PathBuf> = tree.walk().map(|(node, _)| node.path.clone()).collect();
    let mut sorted = pacted.clone();
    walked.sort();
    sorted.sort();
    assert_eq!(
        sorted, walked,
        "a pact covers the nodes of the subtree, no more and no fewer",
    );

    assert_eq!(
        relative_to(repo.path(), &sorted),
        [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
        ],
        "the selected directory and every ordinary directory below it; \
             `generated/` is gitignored, `.hidden/` is hidden and `.warlock/` \
             is ours, so none of them — nor anything inside them — is pactable",
    );
}

#[test]
fn every_child_comes_before_its_parent_and_the_selected_directory_is_last() {
    let repo = repository();
    let subtree = repo.path().join("crates/engine");

    let pacted = pactable_directories(&subtree).expect("walks");

    assert_eq!(
        relative_to(repo.path(), &pacted),
        [
            "crates/engine/tests",
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ],
        "deepest first, and the directory the pact was asked for last",
    );
    // Said again as the property rather than the listing: a parent's request
    // carries its children's documents, so no directory may be pacted before
    // anything below it has written one.
    for (index, directory) in pacted.iter().enumerate() {
        for (other, descendant) in pacted.iter().enumerate() {
            if descendant != directory && descendant.starts_with(directory) {
                assert!(
                    other < index,
                    "`{}` is below `{}` and has to come first",
                    descendant.display(),
                    directory.display(),
                );
            }
        }
    }
    assert_eq!(
        pacted.last().map(PathBuf::as_path),
        Some(subtree.as_path()),
        "and the last pass is the one the whole subtree was gathered for",
    );
}

#[test]
fn a_directory_with_nothing_below_it_is_a_subtree_of_one() {
    let repo = repository();
    let leaf = repo.path().join("crates/engine/src/inner");

    assert_eq!(
        pactable_directories(&leaf).expect("walks"),
        [leaf],
        "a pact always covers the directory it was asked for, documented \
             or not, empty or not",
    );
}

#[test]
fn a_subtree_that_cannot_be_walked_says_which_directory_it_was() {
    let repo = repository();
    let missing = repo.path().join("crates/engine/nowhere");

    let error = pactable_directories(&missing).expect_err("there is nothing to walk");

    assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
    assert_eq!(error.directory(), missing);
}

struct FailsFor {
    directory: PathBuf,
    asked: std::cell::RefCell<Vec<PathBuf>>,
}

impl FailsFor {
    fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            asked: std::cell::RefCell::new(Vec::new()),
        }
    }

    // Every directory a request went out for, refused or not — which is
    // what a skip is measured against: a directory nobody paid for is a
    // directory that is not in here.
    fn asked(&self, root: &Path) -> Vec<String> {
        relative_to(root, &self.asked.borrow())
    }
}

impl Agent for FailsFor {
    fn run(&self, request: &agent::Request) -> Result<agent::Response, agent::Error> {
        self.asked
            .borrow_mut()
            .push(request.directory().to_path_buf());
        if request.directory() == self.directory {
            return Err(agent::Error::EmptyOutput);
        }
        Ok(agent::Response::new(document::stub_answer(request)))
    }
}

#[derive(Default)]
struct Watching {
    stop_after: Option<usize>,
    calls: Vec<(PathBuf, usize, usize)>,
    documented: Vec<PathBuf>,
    skipped: Vec<(PathBuf, PathBuf)>,
}

impl Watching {
    fn patient() -> Self {
        Self::default()
    }

    fn stopping_after(directories: usize) -> Self {
        Self {
            stop_after: Some(directories),
            ..Self::default()
        }
    }

    fn calls(&self, root: &Path) -> Vec<(String, usize, usize)> {
        self.calls
            .iter()
            .map(|(directory, position, total)| (named(root, directory), *position, *total))
            .collect()
    }

    fn offered(&self) -> Vec<PathBuf> {
        self.calls
            .iter()
            .map(|(directory, ..)| directory.clone())
            .collect()
    }

    fn done(&self, root: &Path) -> Vec<String> {
        relative_to(root, &self.documented)
    }

    // Both halves, because the pair is the announcement: the directory that
    // got no pass is only half an answer without the one that cost it.
    fn passed_over(&self, root: &Path) -> Vec<(String, String)> {
        self.skipped
            .iter()
            .map(|(directory, below)| (named(root, directory), named(root, below)))
            .collect()
    }
}

impl Observer for Watching {
    fn starting(&mut self, directory: &Path, position: usize, total: usize) -> Pacting {
        self.calls.push((directory.to_path_buf(), position, total));
        match self.stop_after {
            Some(limit) if position > limit => Pacting::Stop,
            _ => Pacting::Continue,
        }
    }

    fn documented(&mut self, directory: &Path) {
        self.documented.push(directory.to_path_buf());
    }

    fn skipped(&mut self, directory: &Path, below: &Path) {
        self.skipped
            .push((directory.to_path_buf(), below.to_path_buf()));
    }
}

fn project() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    write(repo.path(), ".git/config", "[core]\n");
    write(repo.path(), ".gitignore", "/target\n");
    write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
    write(repo.path(), "Cargo.toml", "[workspace]\n");
    write(repo.path(), "crates/engine/Cargo.toml", "[package]\n");
    write(
        repo.path(),
        "crates/engine/src/lib.rs",
        "//! Core engine.\n",
    );
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deep() {}\n",
    );
    write(
        repo.path(),
        "crates/engine/tests/it.rs",
        "#[test] fn works() {}\n",
    );
    write(repo.path(), "crates/tui/src/main.rs", "fn main() {}\n");
    write(repo.path(), "target/debug/build.log", "noise\n");
    repo
}

fn state(manifest: &Manifest, root: &Path, module: &str) -> NodeState {
    let hash = subtree_hash(from_manifest_path(root, module)).expect("the subtree hashes");
    decide_state(manifest.entry(module), &hash)
}

#[test]
fn every_directory_is_pacted_before_the_one_above_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let agent = Canned::filling();

    pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut Unwatched,
    )
    .expect("pacts");

    let seen: Vec<PathBuf> = agent
        .seen
        .borrow()
        .iter()
        .filter(|request| is_document_pass(request))
        .map(|request| request.directory().to_path_buf())
        .collect();
    assert_eq!(
        relative_to(repo.path(), &seen),
        [
            "crates/engine/tests",
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ],
        "one pass per directory, deepest first, the selected directory last",
    );
    // Said again as the property, since the listing above is one fixture and
    // this is the rule: no request may be issued for a directory before
    // every request below it has been.
    for (index, directory) in seen.iter().enumerate() {
        for (other, descendant) in seen.iter().enumerate() {
            if descendant != directory && descendant.starts_with(directory) {
                assert!(
                    other < index,
                    "`{}` is below `{}` and has to be pacted first",
                    descendant.display(),
                    directory.display(),
                );
            }
        }
    }
    // And this is what the ordering is *for*: the last pass was handed the
    // documents the earlier ones had already written.
    let seen = agent.seen.borrow();
    let parent = seen.last().expect("the selected directory was pacted");
    assert_eq!(
        parent
            .child_documents()
            .iter()
            .map(agent::ChildDocument::directory)
            .collect::<Vec<_>>(),
        ["src", "tests"],
        "a parent reads its children's finished documents, not their source",
    );
}

#[test]
fn a_directory_the_repository_excluded_is_no_part_of_a_pact_above_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    write(&engine, ".warlockignore", "tests/\n");
    let excluded = engine.join("tests");

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        modules(&manifest),
        [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
        ],
        "the excluded directory earns no entry, and the rest of the \
             subtree is pacted exactly as it always was",
    );
    assert_eq!(
        written(&excluded),
        None,
        "and no document was written into it: a pact of an ancestor is not \
             a way round what the repository excluded",
    );
}

#[test]
fn rules_that_cannot_be_parsed_fail_the_pact_rather_than_meaning_no_rules() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    // A range that runs backwards: a glob the matcher will not compile.
    write(&engine, ".warlockignore", "a[z-a]\n");
    let agent = Canned::filling();

    let error = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut Unwatched,
    )
    .expect_err("a pact that cannot tell what is excluded must not run");

    assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
    assert!(
        error.to_string().contains(".warlockignore"),
        "the one line back names the file to go and fix: {error}"
    );
    assert!(
        agent.seen.borrow().is_empty(),
        "and it fails before a single pass is spent",
    );
}

#[test]
fn a_directory_with_no_document_gets_no_entry_and_costs_its_ancestors_their_grants() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let failing = engine.join("src").join("inner");
    let agent = FailsFor::at(failing.clone());

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut Unwatched,
    )
    .expect("one directory failing is not the pact failing");

    assert!(
        manifest.entry("crates/engine/src/inner").is_none(),
        "a directory this run could not describe is not one it pacted",
    );
    assert_eq!(
        written(&failing),
        None,
        "and nothing was written for it either",
    );

    for module in ["crates/engine/src", "crates/engine"] {
        let entry = manifest.entry(module).expect("pacted, if not judged");
        assert_eq!(
            entry.granted_hash(),
            None,
            "`{module}` has an incomplete subtree below it, so it earned no grant",
        );
        assert_eq!(entry.granted_at(), None, "and no timestamp for one");
        assert_eq!(
            state(&manifest, repo.path(), module),
            NodeState::PactedStale,
            "which renders yellow, by the existing freshness rule",
        );
    }

    let sibling = manifest
        .entry("crates/engine/tests")
        .expect("a completed subtree is still pacted");
    assert_eq!(
        sibling.granted_hash(),
        Some(subtree_hash(engine.join("tests")).expect("hashes").as_str()),
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine/tests"),
        NodeState::PactedFresh,
        "one failure elsewhere does not take a finished subtree's grant away",
    );

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        matches!(&failures[0], Failure::Document { .. }),
        "{:?}",
        failures[0],
    );
    assert_eq!(failures[0].directory(), failing);
    assert!(
        failures[0]
            .to_string()
            .contains(&failing.display().to_string()),
        "a failure says which directory it is about: {}",
        failures[0],
    );
}

#[test]
fn the_repository_root_is_a_module_like_any_other_and_stores_as_a_dot() {
    let repo = project();

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        repo.path(),
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        modules(&manifest),
        [
            ".",
            "crates",
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
            "crates/tui",
            "crates/tui/src",
        ],
        "the root stores as `.`, and `target/`, `.git/` and `.warlock/` are \
             not modules",
    );

    let root = manifest.entry(".").expect("the root is pacted too");
    assert_eq!(
        root.document(),
        "WARLOCK.md",
        "documented by the `WARLOCK.md` sitting in the root itself",
    );
    assert_eq!(root.module_path(repo.path()), repo.path());
    assert_eq!(
        state(&manifest, repo.path(), "."),
        NodeState::PactedFresh,
        "and a whole-repository pact leaves the whole repository green",
    );
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_hashed_is_pacted_without_a_grant() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = project();
    let engine = repo.path().join("crates/engine");
    let unreadable = engine.join("tests").join("it.rs");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
    if fs::read(&unreadable).is_ok() {
        // Running as root: no file is unreadable, so there is nothing here
        // to assert against.
        return;
    }

    let PactedSubtree {
        manifest,
        failures,
        problems,
        ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("a file nobody can read never fails the pact");

    // Both directories whose hash would have covered the unreadable file.
    for module in ["crates/engine/tests", "crates/engine"] {
        let entry = manifest.entry(module).expect("documented, so pacted");
        assert_eq!(
            entry.granted_hash(),
            None,
            "`{module}` has no hash, and a hash nobody computed is never invented",
        );
    }
    assert_eq!(
        manifest
            .entry("crates/engine/src")
            .and_then(PactEntry::granted_hash),
        Some(subtree_hash(engine.join("src")).expect("hashes").as_str()),
        "the part of the subtree that can be hashed is still granted",
    );

    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Failure::Hash { .. })),
        "a document that was written is never reported as one that was not: \
             {failures:?}",
    );
    assert!(
        problems.iter().any(|problem| problem.path == unreadable
            && matches!(problem.cause, Omission::Unreadable { .. })),
        "and the request that could not read it said so, non-fatally: {problems:?}",
    );

    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
}

// Progress and cancellation: the observer port.

#[test]
fn every_directory_is_announced_once_before_it_is_pacted() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let agent = Canned::filling();
    let mut observer = Watching::patient();

    let PactedSubtree { failures, .. } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut observer,
    )
    .expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        observer.calls(repo.path()),
        [
            ("crates/engine/tests".to_owned(), 1, 4),
            ("crates/engine/src/inner".to_owned(), 2, 4),
            ("crates/engine/src".to_owned(), 3, 4),
            ("crates/engine".to_owned(), 4, 4),
        ],
        "once per directory, in the pact's own order, 1-based, out of a \
             total that does not move while the pact runs",
    );
    assert_eq!(
        observer.offered(),
        agent
            .seen
            .borrow()
            .iter()
            // The pass that happens once per directory: a directory also
            // costs one pass per file that moved, and those are not what an
            // offer is counted against.
            .filter(|request| is_document_pass(request))
            .map(|request| request.directory().to_path_buf())
            .collect::<Vec<_>>(),
        "and each one names the directory whose pass runs next, not the one \
             that has just finished",
    );
}

#[test]
fn a_directory_is_announced_documented_the_moment_its_pass_delivers() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let mut observer = Watching::patient();

    let PactedSubtree { failures, .. } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut observer,
    )
    .expect("pacts");

    // One announcement per directory, in the order the passes finish —
    // which on a clean run is the order they were offered in, children
    // before parents. Each lands before the next directory is offered,
    // which is what lets a front end colour work done while the run is
    // still paying for the rest.
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        observer.done(repo.path()),
        [
            "crates/engine/tests",
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ],
    );
}

#[test]
fn a_directory_above_a_failure_is_never_announced_documented() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let failing = engine.join("src").join("inner");
    let agent = FailsFor::at(failing);
    let mut observer = Watching::patient();

    let PactedSubtree { failures, .. } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut observer,
    )
    .expect("one refused pass does not fail the pact");

    // `tests` is a whole subtree this run documented, so it is announced.
    // `src/inner` failed, `src` and `engine` sit above the failure, and
    // all three are headed for an entry with no grant or none at all —
    // the announcement stays honest by saying nothing about any of them,
    // even though `src` and `engine` did write documents.
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(observer.done(repo.path()), ["crates/engine/tests"]);
}

#[test]
fn a_cancelled_pact_stops_between_directories_and_keeps_what_it_wrote() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let mut observer = Watching::stopping_after(2);

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut observer,
    )
    .expect("a pact somebody stopped is not a pact that failed");

    assert_eq!(
        observer.calls(repo.path()).len(),
        3,
        "the third directory was offered and turned down, and there was no \
             fourth question: {:?}",
        observer.calls(repo.path()),
    );
    assert!(
        failures.is_empty(),
        "nothing went wrong — fewer directories were asked for: {failures:?}",
    );

    for documented in ["crates/engine/tests", "crates/engine/src/inner"] {
        let directory = from_manifest_path(repo.path(), documented);
        assert!(
            written(&directory).is_some(),
            "`{documented}` was pacted before the cancel, so its document stays on disk",
        );
    }
    for untouched in ["crates/engine/src", "crates/engine"] {
        let directory = from_manifest_path(repo.path(), untouched);
        assert_eq!(
            written(&directory),
            None,
            "`{untouched}` is at or past the cancel, so no pass ran for it",
        );
    }

    assert_eq!(
        modules(&manifest),
        ["crates/engine/src/inner", "crates/engine/tests"],
        "a directory the pact never reached is undocumented by this run, \
             and an undocumented directory gets no entry",
    );
    for module in modules(&manifest) {
        assert_eq!(
            state(&manifest, repo.path(), module),
            NodeState::PactedFresh,
            "`{module}` is a whole subtree this run documented, so it is \
                 granted like any other",
        );
    }
}

#[test]
fn a_cancel_leaves_a_documented_ancestor_of_a_failure_pacted_without_a_grant() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let failing = engine.join("src").join("inner");
    let agent = FailsFor::at(failing.clone());
    // Everything but the selected directory itself, so the run holds all
    // three cases at once: `crates/engine/tests` finished, `crates/engine/src`
    // is documented above a directory that is not, and `crates/engine` is
    // never reached.
    let mut observer = Watching::stopping_after(3);

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &agent,
        &mut observer,
    )
    .expect("neither a failure nor a cancel fails the pact");

    assert!(
        manifest.entry("crates/engine/src/inner").is_none(),
        "no document, no entry — the cancel changes none of that rule",
    );
    let src = manifest
        .entry("crates/engine/src")
        .expect("documented, so pacted");
    assert_eq!(
        src.granted_hash(),
        None,
        "it has an undocumented descendant, so it earned no grant",
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine/src"),
        NodeState::PactedStale,
        "which renders yellow, by the existing freshness rule",
    );
    assert!(
        manifest.entry("crates/engine").is_none(),
        "and the directory the cancel landed on was never pacted at all",
    );

    let finished = manifest
        .entry("crates/engine/tests")
        .expect("a completed subtree is still pacted");
    assert_eq!(
        finished.granted_hash(),
        Some(subtree_hash(engine.join("tests")).expect("hashes").as_str()),
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine/tests"),
        NodeState::PactedFresh,
        "what finished before the cancel keeps what it earned",
    );

    assert_eq!(
        failures.len(),
        1,
        "the directory that failed is reported; the ones nobody asked for \
             are not: {failures:?}",
    );
    assert_eq!(failures[0].directory(), failing);
}

#[test]
fn an_unwatched_pact_is_the_pact_that_never_stops() {
    let repo = project();
    let engine = repo.path().join("crates/engine");

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        modules(&manifest),
        [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
        ],
        "the caller that watches nothing gets every directory pacted",
    );
    assert_eq!(Unwatched.starting(&engine, 1, 4), Pacting::Continue);
}

// Announcing the passes themselves: a file at a time, then the handover to
// the one that fits them together.

#[derive(Default)]
struct Weighing {
    described: Vec<(String, u64, usize, usize)>,
    requested: Vec<(usize, u64)>,
}

impl Observer for Weighing {
    fn starting(&mut self, _directory: &Path, _position: usize, _total: usize) -> Pacting {
        Pacting::Continue
    }

    fn describing(
        &mut self,
        _directory: &Path,
        name: &str,
        bytes: u64,
        position: usize,
        total: usize,
    ) {
        self.described
            .push((name.to_owned(), bytes, position, total));
    }

    fn requesting(&mut self, files: usize, bytes: u64) {
        self.requested.push((files, bytes));
    }
}

#[test]
fn every_file_a_directory_pays_for_is_announced_once_and_counted_to_the_same_total() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "reading.rs", "pub fn read() {}\n");
    write(dir.path(), "writing.rs", "pub fn write() {}\n");
    let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
    let mut watched = Weighing::default();

    taken(dir.path())
        .assemble(None, &agent, &mut watched)
        .expect("lines");

    assert_eq!(
        watched.described,
        [
            ("reading.rs".to_owned(), 17, 1, 2),
            ("writing.rs".to_owned(), 18, 2, 2),
        ],
        "a file, its size, and where it is in the files being paid for",
    );
}

#[test]
fn a_file_taken_off_the_page_is_never_announced_and_never_counted() {
    const LINE: &str = "The line already on the page.";

    // The denominator is the run that is left, which is what makes it worth
    // drawing a bar against: a directory of two files with one moved counts
    // to one, not to two with the first already behind it.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "reading.rs", "pub fn read() {}\n");
    write(dir.path(), "writing.rs", "pub fn write() {}\n");
    let agent = Lining::saying(r#"{"line": "A line about one file alone."}"#);
    let hash = crate::hash::file_hash(dir.path().join("reading.rs")).expect("hashes");
    let recorded = [("reading.rs".to_owned(), crate::hash::line_hash(&hash, LINE))]
        .into_iter()
        .collect();
    let page = page_of(&[("reading.rs", LINE)]);
    let mut watched = Weighing::default();

    taken(dir.path())
        .assemble(Some((&page, &recorded)), &agent, &mut watched)
        .expect("lines");

    assert_eq!(watched.described, [("writing.rs".to_owned(), 18, 1, 1)]);
}

#[test]
fn a_file_asked_about_twice_is_announced_once() {
    // Attempts are `rejected`, not a second `describing`: a bar that moved
    // on a retry would be counting the asking rather than the work.
    let dir = one_file_directory();
    let agent = Lining::saying("prose where an object was asked for");
    let mut watched = Weighing::default();

    let assembled = taken(dir.path())
        .assemble(None, &agent, &mut watched)
        .expect("lines");

    assert_eq!(assembled.mended, ["reading.rs"], "every attempt was spent");
    assert!(agent.passes.get() > 1, "the retries this is about happened");
    assert_eq!(watched.described.len(), 1);
}

#[test]
fn the_handover_counts_the_lines_and_the_documents_below_and_not_the_files() {
    // A directory with no file of its own still carries its children's
    // documents, and a handover reporting no bytes over a pass that is
    // about to wait on a model is the panel's clock labelled with a lie.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "src/lib.rs", "pub fn one() {}\n");
    write(dir.path(), "src/WARLOCK.md", "# src\n\nA document below.\n");
    let agent = Lining::saying("prose where an object was asked for");
    let mut watched = Weighing::default();

    taken(dir.path())
        .fill(&BTreeMap::new(), &agent, &mut watched)
        .expect("a fill");

    assert_eq!(
        watched.requested,
        [(0, 25)],
        "no lines of its own, and the bytes of the document below it",
    );
}

// Un-pacting: dropping the entries and keeping the documents.

fn pacted(modules: &[&str]) -> Manifest {
    Manifest::with_entries(modules.iter().map(|module| {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative path inside the root is storable")
            .with_grant(format!("hash-of-{module}"), "2026-08-21T09:00:00Z")
    }))
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(dir)
                    .expect("under the root")
                    .to_path_buf();
                files.insert(relative, fs::read(&path).expect("a readable file"));
            }
        }
    }
    files
}

#[test]
fn an_un_pact_drops_the_directory_and_everything_below_it() {
    let manifest = pacted(&[
        ".",
        "crates/engine",
        "crates/engine/src",
        "crates/engine/src/inner",
        "crates/engine/tests",
        "crates/engine-tools",
        "crates/tui",
    ]);

    let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

    assert_eq!(
        modules(&left),
        [".", "crates/engine-tools", "crates/tui"],
        "the directory and its descendants go, and nothing else does",
    );
}

#[test]
fn a_sibling_that_shares_a_prefix_is_not_a_descendant() {
    // The whole reason the match is by path segment: `engine-tools` sorts
    // right next to `engine` and starts with every character of it.
    let manifest = pacted(&[
        "crates/engine",
        "crates/engine-tools",
        "crates/engine-tools/src",
        "crates/engineering",
    ]);

    let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

    assert_eq!(
        modules(&left),
        [
            "crates/engine-tools",
            "crates/engine-tools/src",
            "crates/engineering"
        ],
    );
}

#[test]
fn the_repository_root_is_below_nothing_but_itself() {
    let manifest = pacted(&[".", "crates/engine/src"]);

    let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

    assert_eq!(
        modules(&left),
        ["."],
        "a pact on the repository as a whole is not a pact on the subtree, \
             so un-pacting the subtree leaves it alone",
    );
}

#[test]
fn un_pacting_something_that_was_never_pacted_changes_nothing() {
    let manifest = pacted(&["crates/engine", "crates/engine/src"]);

    let left = unpact_subtree("docs/adr", ".", &manifest).expect("un-pacts");
    assert_eq!(left, manifest);

    // And doing it twice says the same thing as doing it once.
    let once = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");
    let twice = unpact_subtree("crates/engine", ".", &once).expect("un-pacts again");
    assert_eq!(twice, once);
}

#[test]
fn a_directory_with_no_manifest_relative_form_is_an_error() {
    let manifest = pacted(&["crates/engine"]);
    assert!(matches!(
        unpact_subtree("/elsewhere/crates", "/repo", &manifest),
        Err(manifest::Error::PathOutsideRoot { .. })
    ));
}

// The directories are made for real, because `is_ignored` reads an absent
// path as not excluded: a fixture of names alone would have every one of
// these tests pass without a rule ever being consulted.
fn ignoring(rules: &str, directories: &[&str]) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    for directory in directories {
        fs::create_dir_all(repo.path().join(directory)).expect("creates a fixture directory");
    }
    write(repo.path(), ignores::FILENAME, rules);
    repo
}

#[test]
fn an_excluded_entry_goes_and_takes_its_scope_with_it() {
    let repo = ignoring("vendor/\n", &["vendor", "crates/engine"]);
    let manifest = with_scopes(
        &pacted(&["crates/engine", "vendor"]),
        &[("vendor", "third-party")],
    );

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(
        scopes(&left),
        [("crates/engine", None)],
        "the repository said the content is out, and a scope is no reason \
             to keep the entry that held it",
    );
}

#[test]
fn an_entry_below_an_excluded_directory_goes_though_that_directory_has_no_entry() {
    let repo = ignoring("vendor/\n", &["vendor/acme/src", "crates"]);
    let manifest = pacted(&["crates", "vendor/acme", "vendor/acme/src"]);

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(
        modules(&left),
        ["crates"],
        "the rule names an ancestor nothing pacted, so the ancestors are \
             what gets asked, not the entries alone",
    );
}

#[test]
fn a_negated_rule_keeps_the_directory_it_re_includes() {
    // The negation sits beside the exclusion rather than under it: gitignore
    // will not re-include anything below a directory already excluded.
    let repo = ignoring("vendor/*\n!vendor/keep\n", &["vendor/acme", "vendor/keep"]);
    let manifest = pacted(&["vendor/acme", "vendor/keep"]);

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(
        modules(&left),
        ["vendor/keep"],
        "exclusion is whatever the matcher says, negation included",
    );
}

#[test]
fn a_manifest_with_nothing_excluded_comes_back_equal() {
    let repo = ignoring("target/\n", &["crates/engine", "crates/tui", "target"]);
    let manifest = with_scopes(
        &pacted(&["crates/engine", "crates/tui"]),
        &[("crates/engine", "data-plane")],
    );

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(
        left, manifest,
        "grant and scope intact: a rule that matches nothing pacted is not \
             an edit to the manifest",
    );
}

#[test]
fn an_excluded_entry_outside_the_loaded_root_is_left_alone() {
    let repo = ignoring("vendor/\n", &["vendor", "crates/engine"]);
    let manifest = pacted(&["crates/engine", "vendor"]);

    let left = unpact_ignored(&manifest, repo.path(), repo.path().join("crates"))
        .expect("the rules are read");

    assert_eq!(
        modules(&left),
        ["crates/engine", "vendor"],
        "`vendor` is excluded and stays anyway: this session loaded \
             `crates` and never read the rules above it",
    );
}

#[test]
fn removing_entries_writes_nothing_to_disk() {
    let repo = ignoring("vendor/\n", &["vendor", "crates"]);
    let manifest = pacted(&["crates", "vendor"]);
    manifest.save(repo.path()).expect("saves");
    let path = repo.path().join(".warlock").join("pacts.toml");
    let before = fs::read(&path).expect("a readable manifest");

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(modules(&left), ["crates"], "an entry really was removed");
    assert_eq!(
        fs::read(&path).expect("a readable manifest"),
        before,
        "and the file on disk is byte-identical: the caller owns the write",
    );
}

#[test]
fn rules_that_cannot_be_used_fail_rather_than_guessing() {
    // A range that runs backwards: a glob the matcher will not compile.
    let repo = ignoring("a[z-a]\n", &["crates/engine"]);
    let manifest = pacted(&["crates/engine"]);

    let error = unpact_ignored(&manifest, repo.path(), repo.path())
        .expect_err("a manifest cannot be cleaned against rules that cannot be read");

    assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
    assert!(
        error.to_string().contains(ignores::FILENAME),
        "the one line back names the file to go and fix: {error}",
    );
}

// The `[[scope]]` records across every rebuild in this module. `third-party` is
// named by no entry in any fixture below, and `data-plane` loses the only entry
// that named it in two of these tests: a record is written down before anything
// is pacted under it and outlives the last pact that spelled it, so neither is
// ever pruned.
fn records() -> Vec<ScopeRecord> {
    vec![
        ScopeRecord::new("data-plane", "Data Plane", "In Review", "area/data-plane"),
        ScopeRecord::new("third-party", "Vendor", "Triage", "area/vendor"),
    ]
}

// The written bytes and not the parsed records, because the order the tables
// come back in and the order they are written in are two different claims.
fn record_bytes(manifest: &Manifest) -> String {
    let text = manifest.to_toml_string().expect("serialises");
    let at = text.find("[[scope]]").unwrap_or(text.len());
    text[at..].to_owned()
}

#[test]
fn an_un_pact_keeps_every_record_including_the_one_it_orphaned() {
    let manifest = with_scopes(
        &pacted(&["crates/engine", "crates/tui"]),
        &[("crates/engine", "data-plane")],
    )
    .with_scopes(records());

    let left = unpact_subtree("crates/engine", ".", &manifest).expect("un-pacts");

    assert_eq!(modules(&left), ["crates/tui"], "the entry really did go");
    assert_eq!(
        record_bytes(&left),
        record_bytes(&manifest),
        "the records are the same records, in the same order, written the same way",
    );
}

#[test]
fn an_un_pact_of_the_whole_repository_still_leaves_the_records() {
    let manifest = pacted(&[".", "crates/engine"]).with_scopes(records());

    let left = unpact_subtree(".", ".", &manifest).expect("un-pacts");

    assert!(left.entries().is_empty(), "nothing is pacted any more");
    assert_eq!(
        left.scopes(),
        records(),
        "and the records are all still here"
    );
}

#[test]
fn cleaning_an_ignored_subtree_keeps_the_records() {
    let repo = ignoring("vendor/\n", &["vendor", "crates/engine"]);
    let manifest = with_scopes(
        &pacted(&["crates/engine", "vendor"]),
        &[("vendor", "third-party")],
    )
    .with_scopes(records());

    let left = unpact_ignored(&manifest, repo.path(), repo.path()).expect("the rules are read");

    assert_eq!(
        scopes(&left),
        [("crates/engine", None)],
        "the excluded entry went, and took the boundary it stated with it",
    );
    assert_eq!(
        record_bytes(&left),
        record_bytes(&manifest),
        "`third-party` routes nothing now and is still written down",
    );
}

#[test]
fn a_run_that_rewrites_the_entries_leaves_the_records_alone() {
    let repo = project();
    let before = pacted(&["crates/tui"]).with_scopes(records());

    let PactedSubtree { manifest, .. } = pact_subtree(
        repo.path().join("crates/engine"),
        repo.path(),
        &before,
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");

    assert!(
        manifest.entries().len() > before.entries().len(),
        "the run really did add entries",
    );
    assert_eq!(
        record_bytes(&manifest),
        record_bytes(&before),
        "a pass writes documents and grants, and no record is its to move",
    );
}

#[test]
fn un_pacting_a_real_subtree_leaves_every_document_on_disk_untouched() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let PactedSubtree { manifest, .. } = pact_subtree(
        &engine,
        repo.path(),
        &pacted(&["crates/tui"]),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");
    assert_eq!(
        modules(&manifest),
        [
            "crates/tui",
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
        ],
    );

    let before = snapshot(repo.path());
    assert_eq!(
        before
            .keys()
            .filter(|path| path.ends_with(DOCUMENT_FILE))
            .count(),
        4,
        "four documents were written, and they are what must survive",
    );

    let left = unpact_subtree(&engine, repo.path(), &manifest).expect("un-pacts");

    assert_eq!(modules(&left), ["crates/tui"]);
    assert_eq!(
        snapshot(repo.path()),
        before,
        "un-pacting deletes no file, writes no file and changes no byte — \
             the documents stay, the manifest on disk is the caller's to save",
    );
    for module in [
        "crates/engine",
        "crates/engine/src",
        "crates/engine/src/inner",
        "crates/engine/tests",
    ] {
        let document = from_manifest_path(repo.path(), module).join(DOCUMENT_FILE);
        assert!(document.is_file(), "`{}` was deleted", document.display());
    }
}

// Refreshing a subtree: describing what has gone stale and passing over
// what has not.

fn restale_the_path(repo: &Path) {
    write(repo, "crates/engine/src/inner/deep.rs", "fn deeper() {}\n");
    write(repo, "crates/engine/src/wider.rs", "fn wider() {}\n");
    write(repo, "crates/engine/outer.rs", "fn outer() {}\n");
}

fn refreshable(repo: &Path) -> Manifest {
    let PactedSubtree {
        manifest,
        failures,
        problems,
        ..
    } = pact_subtree(
        repo.join("crates/engine"),
        repo,
        &Manifest::new(),
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");

    assert!(failures.is_empty(), "{failures:?}");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        modules(&manifest),
        [
            "crates/engine",
            "crates/engine/src",
            "crates/engine/src/inner",
            "crates/engine/tests",
        ],
    );
    for module in modules(&manifest) {
        assert_eq!(
            state(&manifest, repo, module),
            NodeState::PactedFresh,
            "`{module}` starts green, or there is nothing here to skip",
        );
    }
    manifest
}

fn described_by(agent: &Canned, root: &Path) -> Vec<String> {
    let asked: Vec<PathBuf> = agent
        .seen
        .borrow()
        .iter()
        .filter(|request| is_document_pass(request))
        .map(|request| request.directory().to_path_buf())
        .collect();
    relative_to(root, &asked)
}

#[test]
fn a_refresh_describes_every_stale_directory_and_none_it_calls_fresh() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());

    // Two ways of being stale and one of being fresh, in one subtree: a
    // directory whose content changed, a directory nobody ever pacted, and
    // a `src/` nothing has touched since it was granted.
    write(
        repo.path(),
        "crates/engine/tests/it.rs",
        "#[test] fn works_differently() {}\n",
    );
    write(
        repo.path(),
        "crates/engine/benches/speed.rs",
        "fn bench() {}\n",
    );
    let agent = Canned::filling();

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
        .expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        described_by(&agent, repo.path()),
        [
            "crates/engine/tests",
            "crates/engine/benches",
            "crates/engine",
        ],
        "every directory `decide_state` calls stale — changed, unpacted, \
             and the directory above both — and no directory it calls fresh",
    );
    // Said again from the other side: what was fresh is exactly what was
    // never handed to a pass.
    for skipped in ["crates/engine/src", "crates/engine/src/inner"] {
        assert!(
            !described_by(&agent, repo.path()).contains(&skipped.to_owned()),
            "`{skipped}` hashes to what it was granted for, so it is not \
                 described",
        );
    }
    for module in modules(&manifest) {
        assert_eq!(
            state(&manifest, repo.path(), module),
            NodeState::PactedFresh,
            "`{module}` ends green: what was described earned a grant, what \
                 was skipped kept one",
        );
    }
}

#[test]
fn a_refresh_leaves_the_entry_of_every_directory_it_skipped_as_it_found_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = refreshable(repo.path());

    restale_the_path(repo.path());
    let agent = Canned::filling();

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut Unwatched).expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    let described = [
        "crates/engine/src/inner",
        "crates/engine/src",
        "crates/engine",
    ];
    assert_eq!(described_by(&agent, repo.path()), described);

    let skipped: Vec<&str> = modules(&before)
        .into_iter()
        .filter(|module| !described.contains(module))
        .collect();
    assert_eq!(skipped, ["crates/engine/tests"], "the fixture skips one");
    for module in skipped {
        let was = before.entry(module).expect("pacted before the refresh");
        let now = manifest.entry(module).expect("still pacted after it");
        assert_eq!(
            (
                now.module(),
                now.document(),
                now.granted_hash(),
                now.granted_at()
            ),
            (
                was.module(),
                was.document(),
                was.granted_hash(),
                was.granted_at()
            ),
            "`{module}` was skipped, so its entry keeps its module, its \
                 document, its hash and its timestamp",
        );
        assert_eq!(now, was, "and the whole entry with them");
    }

    for module in described {
        assert_eq!(
            state(&manifest, repo.path(), module),
            NodeState::PactedFresh,
            "`{module}` was described and hashed afterwards, so it ends green",
        );
    }
}

#[test]
fn a_change_in_every_directory_costs_one_pass_for_each_of_them_and_no_others() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());

    restale_the_path(repo.path());
    let agent = Canned::filling();

    let PactedSubtree { failures, .. } =
        refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
            .expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        described_by(&agent, repo.path()),
        [
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ],
        "the path from the changed files up to the refreshed root, deepest \
             first, and nothing beside it",
    );
    assert_eq!(
        agent
            .seen
            .borrow()
            .iter()
            .filter(|request| is_document_pass(request))
            .count(),
        3,
        "one synthesis pass per directory that had something new to read — \
             `crates/engine/tests` is a quarter of the subtree and costs nothing",
    );
}

#[test]
fn a_hand_edited_document_is_never_carried_forward_by_the_cutoff() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());

    // Nothing about the code moves. Somebody edits the document instead,
    // which is the one thing the request never sees: it is prose, so it
    // reaches no pass and moves no other digest. If the cutoff went on the
    // request alone, this would be carried forward and stamped granted —
    // a person's sentences recorded as a pass's work.
    let document = engine.join("src").join(DOCUMENT_FILE);
    let edited = format!(
        "{}\n\nA sentence a person added by hand.\n",
        fs::read_to_string(&document).expect("the pact wrote a document"),
    );
    fs::write(&document, &edited).expect("writes the edit");

    let agent = Canned::filling();
    let PactedSubtree { failures, .. } =
        refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
            .expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert!(
        described_by(&agent, repo.path()).contains(&"crates/engine/src".to_string()),
        "the edited directory is described again rather than carried: the \
             only road back to fresh is a pass",
    );
    assert_ne!(
        fs::read_to_string(&document).expect("still a document"),
        edited,
        "and the pass overwrote the hand-written sentence rather than \
             granting it",
    );
}

#[test]
fn a_change_below_an_unmoved_request_costs_no_pass_at_the_directory_above_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());

    // One file, at the bottom. Every directory from it to the root is
    // stale — the subtree hash says so and that has not changed — but only
    // the directories whose *request* moved are worth a pass.
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deeper() {}\n",
    );
    let agent = Canned::filling();

    let PactedSubtree { failures, .. } =
        refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut Unwatched)
            .expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        described_by(&agent, repo.path()),
        ["crates/engine/src/inner", "crates/engine/src"],
        "the pass runs where the file changed, and at the parent whose \
             child document changed under it — and stops at `crates/engine`, \
             whose own files and children's documents are what they were",
    );

    // The cutoff is a saving, never a downgrade: the directory that paid
    // for no pass is as green as the ones that did, because its document
    // was granted against the subtree as it now stands.
    for module in [
        "crates/engine",
        "crates/engine/src",
        "crates/engine/src/inner",
    ] {
        assert_eq!(
            state(&manifest, repo.path(), module),
            NodeState::PactedStale,
            "`{module}` is stale before the refresh",
        );
    }
}

#[test]
fn a_refresh_with_nothing_stale_runs_no_pass_and_changes_no_entry() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = refreshable(repo.path());
    let agent = Canned::filling();
    let mut observer = Watching::patient();

    let PactedSubtree {
        manifest,
        failures,
        problems,
        ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer).expect("refreshes");

    assert!(
        agent.seen.borrow().is_empty(),
        "nothing is stale, so nothing is described and no pass is bought",
    );
    assert!(
        observer.calls(repo.path()).is_empty(),
        "and there is no directory to announce: {:?}",
        observer.calls(repo.path()),
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        manifest, before,
        "the manifest comes back with every entry, every grant and every \
             timestamp exactly as it went in",
    );
}

#[test]
fn the_total_announced_counts_the_directories_a_refresh_will_describe() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());
    write(
        repo.path(),
        "crates/engine/tests/it.rs",
        "#[test] fn works_differently() {}\n",
    );
    let mut observer = Watching::patient();

    let PactedSubtree { failures, .. } = refresh_subtree(
        &engine,
        repo.path(),
        &manifest,
        &Canned::filling(),
        &mut observer,
    )
    .expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        observer.calls(repo.path()),
        [
            ("crates/engine/tests".to_owned(), 1, 2),
            ("crates/engine".to_owned(), 2, 2),
        ],
        "two of two: the directories this run will actually describe, not \
             the four in the subtree",
    );
    assert_eq!(
        observer.done(repo.path()),
        ["crates/engine/tests", "crates/engine"],
        "and each is announced documented as its pass delivers, exactly as \
             in a pact",
    );
}

#[cfg(unix)]
#[derive(Default)]
struct Probing {
    said: Vec<String>,
}

impl Observer for Probing {
    fn starting(&mut self, directory: &Path, _position: usize, _total: usize) -> Pacting {
        self.said.push(format!("starting {}", directory.display()));
        Pacting::Continue
    }

    fn unchanged(&mut self, directory: &Path) {
        self.said.push(format!("unchanged {}", directory.display()));
    }

    fn skipped(&mut self, directory: &Path, below: &Path) {
        self.said.push(format!(
            "skipped {} below {}",
            directory.display(),
            below.display()
        ));
    }

    fn documented(&mut self, directory: &Path) {
        self.said
            .push(format!("documented {}", directory.display()));
    }
}

#[test]
fn a_directory_whose_hash_fails_while_staleness_is_decided_is_described_anyway() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = project();
    let engine = repo.path().join("crates/engine");
    let manifest = refreshable(repo.path());

    let unreadable = engine.join("tests").join("it.rs");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
    if fs::read(&unreadable).is_ok() {
        // Running as root: no file is unreadable, so there is nothing here
        // to assert against.
        return;
    }

    let agent = Canned::filling();
    let mut probe = Probing::default();
    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &manifest, &agent, &mut probe)
        .expect("a hash nobody can take is a directory to describe, not an error");

    assert_eq!(
        described_by(&agent, repo.path()),
        ["crates/engine/tests"],
        "no hash is no answer to `is this still the content it was granted \
             for`, so the directory holding the unreadable file is described",
    );
    assert!(
        probe
            .said
            .contains(&format!("unchanged {}", engine.display())),
        "and the one above it is offered and then cut off: its own files and \
             its children's documents are where they were, so re-describing it \
             would buy the same document twice. Being unhashable is what keeps \
             it from a grant, not what earns it a pass: {:?}",
        probe.said,
    );
    // And then it plays out exactly as the module docs say it does: phase
    // two hashes them again, that hash fails again, and each lands as a
    // `Failure::Hash` with an ungranted entry — yellow, which is the honest
    // outcome for a directory something is really wrong with, whether or
    // not this run paid for a pass over it.
    for module in ["crates/engine/tests", "crates/engine"] {
        let entry = manifest.entry(module).expect("described, so pacted");
        assert_eq!(
            entry.granted_hash(),
            None,
            "`{module}` was described and still has no hash to grant against",
        );
    }
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Failure::Hash { .. })),
        "the documents were written; only the hashes failed: {failures:?}",
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine/src"),
        NodeState::PactedFresh,
        "the part of the subtree nothing is wrong with is skipped and stays \
             green",
    );

    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
}

#[test]
fn a_refresh_whose_passes_all_fail_removes_no_entry_and_drops_no_grant() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = refreshable(repo.path());
    restale_the_path(repo.path());

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(
        &engine,
        repo.path(),
        &before,
        &Fails(|| agent::Error::EmptyOutput),
        &mut Unwatched,
    )
    .expect("a refused pass does not fail the refresh");

    assert_eq!(
        failures.len(),
        1,
        "the deepest stale directory is the only one that reached the \
             agent: the two above it are ancestors of its failure and were \
             skipped rather than paid for: {failures:?}",
    );
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Failure::Document { .. })),
        "{failures:?}",
    );
    assert_eq!(
        manifest, before,
        "a refresh that could not re-describe anything leaves the manifest \
             exactly as stale as it found it: no entry removed, no grant \
             dropped",
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine"),
        NodeState::PactedStale,
        "still yellow, which is what a stale directory nobody managed to \
             re-describe should be",
    );
}

#[test]
fn a_cancelled_refresh_keeps_what_it_described_and_leaves_the_rest_alone() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = refreshable(repo.path());
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deeper() {}\n",
    );
    let agent = Canned::filling();
    let mut observer = Watching::stopping_after(1);

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
        .expect("a refresh somebody stopped is not a refresh that failed");

    assert_eq!(
        observer.calls(repo.path()),
        [
            ("crates/engine/src/inner".to_owned(), 1, 3),
            ("crates/engine/src".to_owned(), 2, 3),
        ],
        "the second directory was offered and turned down, and there was no \
             third question",
    );
    assert!(
        failures.is_empty(),
        "nothing went wrong — fewer directories were asked for: {failures:?}",
    );
    assert_eq!(
        described_by(&agent, repo.path()),
        ["crates/engine/src/inner"],
        "and the cancel cost no pass at all",
    );

    assert_eq!(
        modules(&manifest),
        modules(&before),
        "a cancelled refresh drops no entry either",
    );
    assert_eq!(
        state(&manifest, repo.path(), "crates/engine/src/inner"),
        NodeState::PactedFresh,
        "what finished before the cancel is granted like any other",
    );
    for untouched in ["crates/engine/src", "crates/engine"] {
        assert_eq!(
            manifest.entry(untouched),
            before.entry(untouched),
            "`{untouched}` is at or past the cancel, so it keeps the entry \
                 the refresh found",
        );
        assert_eq!(
            state(&manifest, repo.path(), untouched),
            NodeState::PactedStale,
            "which is to say it is exactly as stale as it was",
        );
    }
}

// The whole point of the skip, and the reason it is worth a test of its
// own: a refresh that hits a failure used to walk on up the tree, pay for a
// pass at every directory above it, and record each one without a grant —
// documents the next run has to write again, because the directory below
// still has to be described and a described child is a moved parent
// request. The spend was real and nothing survived it.
#[test]
fn a_refresh_above_a_failed_pass_skips_the_ancestor_rather_than_paying_for_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = refreshable(repo.path());
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deeper() {}\n",
    );
    let agent = FailsFor::at(engine.join("src").join("inner"));
    let mut observer = Watching::patient();

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
        .expect("one refused pass does not fail the refresh");

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].directory(), engine.join("src").join("inner"));
    assert_eq!(
        agent.asked(repo.path()),
        ["crates/engine/src/inner"],
        "one request went out, for the one stale directory that was not \
             above a failure — the two above it cost nothing",
    );
    assert_eq!(
        observer.passed_over(repo.path()),
        [
            (
                "crates/engine/src".to_owned(),
                "crates/engine/src/inner".to_owned()
            ),
            (
                "crates/engine".to_owned(),
                "crates/engine/src/inner".to_owned()
            ),
        ],
        "and each one says which failure below it took it down, so a run \
             that describes fewer directories than it started does not look \
             like one that finished",
    );
    for untouched in [
        "crates/engine/src/inner",
        "crates/engine/src",
        "crates/engine",
        "crates/engine/tests",
    ] {
        assert_eq!(
            manifest.entry(untouched),
            before.entry(untouched),
            "`{untouched}` kept the entry it had, grant and all: a refresh \
                 that did not re-describe a directory has nothing to say about \
                 it, and the one it could not re-describe is not un-pacted",
        );
    }
    for stale in [
        "crates/engine/src/inner",
        "crates/engine/src",
        "crates/engine",
    ] {
        assert_eq!(
            state(&manifest, repo.path(), stale),
            NodeState::PactedStale,
            "`{stale}` reads stale on the grant it kept, because the hash \
                 under it moved and no pass has been granted since",
        );
    }
}

// The bytes themselves. Everything above asserts about entries; this
// asserts about the file, so that a change of shape — a key that moves, a
// blank line that appears, an entry that is appended where it used to be
// replaced in place — fails the build instead of passing quietly.

fn hash_of(repo: &Path, module: &str) -> String {
    subtree_hash(from_manifest_path(repo, module)).expect("the subtree hashes")
}

// Read back off the written document rather than passed in: what a
// `[pact.lines]` entry records is the file and the line together, so the
// expected value cannot be derived from the source alone.
fn line_of(repo: &Path, module: &str, file: &str) -> String {
    let directory = repo.join(module);
    let page = String::from_utf8(written(&directory).expect("a document")).expect("utf-8");
    let line = document::lines_of(&page)
        .remove(file)
        .unwrap_or_else(|| panic!("`{file}` has a line on `{module}`'s page"));

    crate::hash::line_hash(
        &crate::hash::file_hash(directory.join(file)).expect("the fixture is readable"),
        &line,
    )
}

fn carry_of(repo: &Path, module: &str) -> String {
    super::carry_hash(&from_manifest_path(repo, module)).expect("the directory digests")
}

fn granted_at_of(manifest: &Manifest, module: &str) -> String {
    manifest
        .entry(module)
        .unwrap_or_else(|| panic!("`{module}` is pacted"))
        .granted_at()
        .unwrap_or_else(|| panic!("`{module}` is granted"))
        .to_owned()
}

struct Stamps<'at> {
    engine: &'at str,
    src: &'at str,
    inner: &'at str,
    tests: &'at str,
}

fn expected_manifest(repo: &Path, at: &Stamps<'_>) -> String {
    format!(
        "version = 1\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src\"\n\
             document = \"crates/engine/src/WARLOCK.md\"\n\
             granted_hash = \"{src}\"\n\
             granted_at = \"{src_at}\"\n\
             carry_hash = \"{src_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"lib.rs\" = \"{src_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/tui\"\n\
             document = \"crates/tui/WARLOCK.md\"\n\
             granted_hash = \"othercrate\"\n\
             granted_at = \"2026-02-02T00:00:00Z\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine\"\n\
             document = \"crates/engine/WARLOCK.md\"\n\
             granted_hash = \"{root}\"\n\
             granted_at = \"{engine_at}\"\n\
             carry_hash = \"{root_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"Cargo.toml\" = \"{root_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/src/inner\"\n\
             document = \"crates/engine/src/inner/WARLOCK.md\"\n\
             granted_hash = \"{inner}\"\n\
             granted_at = \"{inner_at}\"\n\
             carry_hash = \"{inner_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"deep.rs\" = \"{inner_line}\"\n\
             \n\
             [[pact]]\n\
             module = \"crates/engine/tests\"\n\
             document = \"crates/engine/tests/WARLOCK.md\"\n\
             granted_hash = \"{tests}\"\n\
             granted_at = \"{tests_at}\"\n\
             carry_hash = \"{tests_carry}\"\n\
             \n\
             [pact.lines]\n\
             \"it.rs\" = \"{tests_line}\"\n",
        engine_at = at.engine,
        src_at = at.src,
        inner_at = at.inner,
        tests_at = at.tests,
        root = hash_of(repo, "crates/engine"),
        src = hash_of(repo, "crates/engine/src"),
        inner = hash_of(repo, "crates/engine/src/inner"),
        tests = hash_of(repo, "crates/engine/tests"),
        root_carry = carry_of(repo, "crates/engine"),
        root_line = line_of(repo, "crates/engine", "Cargo.toml"),
        src_carry = carry_of(repo, "crates/engine/src"),
        src_line = line_of(repo, "crates/engine/src", "lib.rs"),
        inner_carry = carry_of(repo, "crates/engine/src/inner"),
        inner_line = line_of(repo, "crates/engine/src/inner", "deep.rs"),
        tests_carry = carry_of(repo, "crates/engine/tests"),
        tests_line = line_of(repo, "crates/engine/tests", "it.rs"),
    )
}

#[test]
fn a_pact_and_a_refresh_over_a_granted_manifest_write_these_exact_bytes() {
    let repo = project();
    let engine = repo.path().join("crates/engine");

    // A manifest that already says something: one entry for a directory the
    // pact will cover, carrying a grant from a run that is not this one, and
    // one entry for a directory it will not cover at all. The covered one is
    // written first so that keeping its position is visible in the bytes —
    // it must stay at the top with the newly gained entries below it, not be
    // dropped and re-appended in sorted order.
    let entry = |module: &str| {
        PactEntry::new(repo.path(), module, format!("{module}/{DOCUMENT_FILE}"))
            .expect("the fixture's paths are spellable")
    };
    let before = Manifest::with_entries([
        entry("crates/engine/src").with_grant("stalehash", "2026-01-01T00:00:00Z"),
        entry("crates/tui").with_grant("othercrate", "2026-02-02T00:00:00Z"),
    ]);

    let PactedSubtree {
        manifest,
        failures,
        problems,
        ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &before,
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("pacts");
    assert!(failures.is_empty(), "{failures:?}");
    assert!(problems.is_empty(), "{problems:?}");

    // The one thing a run does not decide for itself: the clock. Read back
    // off the manifest rather than guessed at, and read back once, so the
    // literal below still insists that every entry the run granted carries
    // the same timestamp.
    let pacted_at = granted_at_of(&manifest, "crates/engine");

    assert_eq!(
        manifest.to_toml_string().expect("serialises"),
        expected_manifest(
            repo.path(),
            &Stamps {
                engine: &pacted_at,
                src: &pacted_at,
                inner: &pacted_at,
                tests: &pacted_at,
            },
        ),
        "the whole file, not a fragment of it",
    );

    // Now a refresh over that manifest, with one file moved under `tests/`.
    // Two directories go stale — `tests` and the `crates/engine` above it —
    // and everything else, covered or not, is carried through byte for byte,
    // grants and positions and all.
    write(
        repo.path(),
        "crates/engine/tests/it.rs",
        "#[test] fn works_differently() {}\n",
    );

    let PactedSubtree {
        manifest,
        failures,
        problems,
        ..
    } = refresh_subtree(
        &engine,
        repo.path(),
        &manifest,
        &Canned::filling(),
        &mut Unwatched,
    )
    .expect("refreshes");
    assert!(failures.is_empty(), "{failures:?}");
    assert!(problems.is_empty(), "{problems:?}");

    // Read the same way, and deliberately not asserted to differ from
    // `pacted_at`: the clock is only to the second, so two runs in one test
    // very often mint the same string. What the literal below pins is which
    // entries got the refresh's timestamp and which kept the pact's, and
    // that reads the same either way.
    let refreshed_at = granted_at_of(&manifest, "crates/engine");

    assert_eq!(
        manifest.to_toml_string().expect("serialises"),
        expected_manifest(
            repo.path(),
            &Stamps {
                engine: &refreshed_at,
                src: &pacted_at,
                inner: &pacted_at,
                tests: &refreshed_at,
            },
        ),
        "the whole file again: two entries re-granted where they sat, three \
             carried through untouched",
    );
}

// Scopes: what a run may not touch, and what un-pacting takes with it.
//
// Every test below passes by construction — a scope lives on an entry, a
// run hands over `Outcome`s, and an `Outcome` has nowhere to put one — so
// these are regression guards rather than proofs of new code. They are what
// fails loudly if somebody ever widens the outcome type, which is the
// mistake worth catching early: a run that can write a scope is a run that
// can quietly move a boundary somebody drew on purpose.

#[test]
fn a_refresh_leaves_every_scope_exactly_as_it_found_it() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = with_scopes(
        &refreshable(repo.path()),
        &[
            ("crates/engine", "engine"),
            ("crates/engine/src", "data-plane"),
            ("crates/engine/tests", "harness"),
        ],
    );

    // A change in each directory on the path up, so the refresh describes
    // all three and skips `tests/` — both kinds of directory in one run.
    restale_the_path(repo.path());
    let agent = Canned::filling();

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut Unwatched).expect("refreshes");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        described_by(&agent, repo.path()),
        [
            "crates/engine/src/inner",
            "crates/engine/src",
            "crates/engine",
        ],
        "two scoped directories were described and one scoped directory was \
             skipped, or this proves nothing about either",
    );
    assert_eq!(
        scopes(&manifest),
        scopes(&before),
        "a refresh rewrites documents, hashes and timestamps, and no scope: \
             described and skipped directories alike keep the boundary somebody \
             drew on them",
    );
}

#[test]
fn a_cancelled_run_keeps_every_scope() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = with_scopes(
        &refreshable(repo.path()),
        &[
            ("crates/engine", "engine"),
            ("crates/engine/src", "data-plane"),
            ("crates/engine/src/inner", "deep"),
            ("crates/engine/tests", "harness"),
        ],
    );
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deeper() {}\n",
    );
    let agent = Canned::filling();
    // The same cancellation the refresh tests above use: one directory
    // described, the next offered and turned down, the rest never asked.
    let mut observer = Watching::stopping_after(1);

    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(&engine, repo.path(), &before, &agent, &mut observer)
        .expect("a refresh somebody stopped is not a refresh that failed");

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        described_by(&agent, repo.path()),
        ["crates/engine/src/inner"],
        "one scoped directory got its pass, and three scoped directories \
             were cut off mid-run",
    );
    assert_eq!(
        scopes(&manifest),
        scopes(&before),
        "stopping a run part way through takes out documents and grants \
             nobody asked for, and no boundary anybody drew",
    );
}

fn scoped_path(repo: &Path) -> Manifest {
    with_scopes(
        &refreshable(repo),
        &[
            ("crates/engine", "engine"),
            ("crates/engine/src", "data-plane"),
            ("crates/engine/src/inner", "deep"),
            ("crates/engine/tests", "harness"),
        ],
    )
}

#[test]
fn a_partly_completed_refresh_keeps_every_scope() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = scoped_path(repo.path());
    write(
        repo.path(),
        "crates/engine/src/inner/deep.rs",
        "fn deeper() {}\n",
    );

    // One pass refuses, so the two scoped directories above it are skipped
    // and the scoped directory that refused is left where it was.
    let PactedSubtree {
        manifest, failures, ..
    } = refresh_subtree(
        &engine,
        repo.path(),
        &before,
        &FailsFor::at(engine.join("src").join("inner")),
        &mut Unwatched,
    )
    .expect("one refused pass does not fail the refresh");

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        scopes(&manifest),
        scopes(&before),
        "a refresh that got part way holds every boundary somebody drew: \
             it did not describe these directories, so it has nothing to say \
             about them",
    );
}

// The other half, on the caller that still describes above a failure.
// `AboveFailure::Describe` is now the only route to an ungranted entry
// written by a run that meant to grant it, and a scope has to survive it.
#[test]
fn a_partly_completed_pact_keeps_the_scope_of_every_entry_it_keeps() {
    let repo = project();
    let engine = repo.path().join("crates/engine");
    let before = scoped_path(repo.path());

    let PactedSubtree {
        manifest, failures, ..
    } = pact_subtree(
        &engine,
        repo.path(),
        &before,
        &FailsFor::at(engine.join("src").join("inner")),
        &mut Unwatched,
    )
    .expect("one refused pass does not fail the pact");

    assert_eq!(failures.len(), 1, "{failures:?}");
    for above in ["crates/engine/src", "crates/engine"] {
        let entry = manifest.entry(above).expect("documented, so pacted");
        assert_eq!(
            entry.granted_hash(),
            None,
            "`{above}` really did come out of the run ungranted, or the \
                 partial-completion path was never taken",
        );
    }
    assert_eq!(
        scopes(&manifest),
        scopes(&before)
            .into_iter()
            .filter(|(module, _)| *module != "crates/engine/src/inner")
            .collect::<Vec<_>>(),
        "the grant is a field a run owns and clears; the scope is not, so \
             the two ungranted directories keep their boundaries. The one that \
             earned nothing loses its entry, and a scope has no home outside an \
             entry — a pact is the direction that can take a boundary away",
    );
}
