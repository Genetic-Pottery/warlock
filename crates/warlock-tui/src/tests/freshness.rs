use std::fs;
use std::path::Path;

use tempfile::TempDir;
use warlock_engine::pact::Event;
use warlock_engine::{
    Agent, Manifest, NodeState, PactEntry, agent, decide_state, from_manifest_path, subtree_hash,
};

use super::{Freshened, Freshening, freshened, made_stale};
use crate::boundary::closed_scope_message;
use crate::claude::Cancel;
use crate::descent::{Descent, descend};
use crate::git::Dirty;
use crate::stubs::{Checkout, GitCall, Passing};

// Never read back, and no clock is consulted: the grant's timestamp plays no
// part in the rule under test.
const GRANTED_AT: &str = "2026-09-28T09:00:00Z";

// A real repository in a real temporary directory, hashed by the engine's own
// digest over the bytes actually on disk. A fixed-up hash would let these tests
// pass while `decide_state` and `subtree_hash` disagreed with each other, which
// is the one thing the selection rule leans on entirely.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("a file has a directory above it"))
            .expect("the directories above the file");
        fs::write(path, contents).expect("a file under the temporary repository");
    }

    fn remove(&self, relative: &str) {
        fs::remove_file(self.root().join(relative)).expect("a file the test just wrote");
    }

    fn remove_directory(&self, relative: &str) {
        fs::remove_dir_all(self.root().join(relative)).expect("a directory the test just wrote");
    }

    // A `[[pact]]` row granted against the subtree as it stands, so the entry is
    // fresh until something at or below it moves — which is how the branch's
    // edits below make it stale, rather than a hand-written wrong hash.
    fn fresh(&self, module: &str) -> PactEntry {
        let entry = self.pact(module);
        let hash =
            subtree_hash(entry.module_path(self.root())).expect("a directory on disk hashes");
        entry.with_grant(hash, GRANTED_AT)
    }

    fn pact(&self, module: &str) -> PactEntry {
        let directory = from_manifest_path(self.root(), module);
        PactEntry::new(self.root(), &directory, directory.join(".warlock.md"))
            .expect("a module inside the root")
    }
}

// Two crates and a docs directory, and not one `.warlock.md`: no document is
// read by the rule, only the directories' own bytes.
fn repository() -> Repo {
    let repo = Repo::new();
    repo.write("crates/engine/src/lib.rs", "pub fn read_one() {}\n");
    repo.write("crates/tui/src/main.rs", "fn main() {}\n");
    repo.write("docs/notes.md", "# Notes\n");
    repo
}

// Spelled the way `git diff --name-only` spells them: repository-root-relative,
// forward slashes.
fn changed<const N: usize>(paths: [&str; N]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

#[test]
fn a_pacted_directory_the_branch_made_stale_is_selected() {
    let repo = repository();
    let manifest = Manifest::with_entries([
        repo.fresh("."),
        repo.fresh("crates"),
        repo.fresh("crates/engine"),
        repo.fresh("crates/tui"),
    ]);

    // The branch's edit, made after the grants, so the staleness is real.
    repo.write(
        "crates/engine/src/lib.rs",
        "pub fn read_one(at: usize) {}\n",
    );

    assert_eq!(
        made_stale(
            repo.root(),
            &manifest,
            &changed(["crates/engine/src/lib.rs"])
        ),
        ["crates/engine", "crates", "."],
        "the pacted directories holding the changed file, deepest first; \
         `crates/engine/src` holds it too and is not pacted"
    );
}

#[test]
fn a_stale_directory_the_branch_did_not_touch_is_left_alone() {
    let repo = repository();
    let manifest = Manifest::with_entries([
        repo.fresh("."),
        repo.fresh("crates"),
        repo.fresh("crates/engine"),
        repo.fresh("crates/tui"),
    ]);

    // Both are stale on disk. Only one of them is this branch's doing — the
    // other is somebody's uncommitted work, or a pact left ungranted.
    repo.write(
        "crates/engine/src/lib.rs",
        "pub fn read_one(at: usize) {}\n",
    );
    repo.write("crates/tui/src/main.rs", "fn main() { warlock() }\n");

    let selected = made_stale(
        repo.root(),
        &manifest,
        &changed(["crates/engine/src/lib.rs"]),
    );

    assert!(
        !selected.iter().any(|module| module == "crates/tui"),
        "stale is not enough: the branch changed nothing at or below it, so \
         refreshing it would put a document in this review that has nothing to \
         do with it — got {selected:?}"
    );
    assert_eq!(selected, ["crates/engine", "crates", "."]);
}

#[test]
fn a_touched_directory_that_is_fresh_is_left_alone() {
    let repo = repository();
    // Granted against the bytes that are there, and nothing is written
    // afterwards: the branch touched `docs/notes.md` and left it as it found it.
    let manifest = Manifest::with_entries([repo.fresh("."), repo.fresh("docs")]);

    assert!(
        made_stale(repo.root(), &manifest, &changed(["docs/notes.md"])).is_empty(),
        "touched is not enough either: a fresh directory has a document that \
         already describes what is there, and a pass over it would be paid for \
         nothing"
    );
}

#[test]
fn a_touched_directory_that_is_unpacted_is_left_alone() {
    let repo = repository();
    // No `[[pact]]` row for `assets`, and none for the root's other children.
    let manifest = Manifest::with_entries([repo.fresh(".")]);

    repo.write("assets/logo.svg", "<svg/>\n");

    assert_eq!(
        made_stale(repo.root(), &manifest, &changed(["assets/logo.svg"])),
        ["."],
        "an unpacted directory is outside warlock's management however much the \
         branch changed in it; the pacted root above it is stale and is refreshed"
    );
}

#[test]
fn children_come_before_their_ancestors() {
    let repo = repository();
    let manifest = Manifest::with_entries([
        repo.fresh("."),
        repo.fresh("crates"),
        repo.fresh("crates/engine"),
        repo.fresh("crates/engine/src"),
        repo.fresh("crates/tui"),
    ]);

    repo.write(
        "crates/engine/src/lib.rs",
        "pub fn read_one(at: usize) {}\n",
    );
    repo.write("crates/tui/src/main.rs", "fn main() { warlock() }\n");

    let selected = made_stale(
        repo.root(),
        &manifest,
        &changed(["crates/engine/src/lib.rs", "crates/tui/src/main.rs"]),
    );

    assert_eq!(
        selected,
        [
            "crates/engine/src",
            "crates/engine",
            "crates/tui",
            "crates",
            "."
        ],
        "deepest first, alphabetical among equal depths so the list does not \
         wander between runs"
    );
    for (child, ancestor) in [
        ("crates/engine/src", "crates/engine"),
        ("crates/engine", "crates"),
        ("crates/tui", "crates"),
        ("crates", "."),
    ] {
        let at = |module: &str| {
            selected
                .iter()
                .position(|selected| selected == module)
                .expect("every stale, touched pact is in the list")
        };
        assert!(
            at(child) < at(ancestor),
            "a parent's document is written from its children's, so {child} has \
             to be refreshed before {ancestor}"
        );
    }
}

#[test]
fn a_deleted_file_still_touches_the_directory_it_was_in() {
    let repo = repository();
    repo.write("crates/tui/src/old.rs", "pub fn gone() {}\n");
    let manifest = Manifest::with_entries([
        repo.fresh("."),
        repo.fresh("crates"),
        repo.fresh("crates/tui"),
    ]);

    repo.remove("crates/tui/src/old.rs");

    assert_eq!(
        made_stale(repo.root(), &manifest, &changed(["crates/tui/src/old.rs"])),
        ["crates/tui", "crates", "."],
        "a path the branch deleted is a change to the directory it was in, and \
         the document still names the file"
    );
}

#[test]
fn a_pacted_directory_the_branch_deleted_is_stale_rather_than_dropped() {
    let repo = repository();
    repo.write("crates/gone/src/lib.rs", "pub fn leaving() {}\n");
    let manifest = Manifest::with_entries([
        repo.fresh("."),
        repo.fresh("crates"),
        repo.fresh("crates/gone"),
    ]);

    repo.remove_directory("crates/gone");

    assert_eq!(
        made_stale(repo.root(), &manifest, &changed(["crates/gone/src/lib.rs"])),
        ["crates/gone", "crates", "."],
        "a hash that cannot be taken is the stale side of the rule: the pass \
         over it fails and is reported, where dropping it here would hide a \
         `[[pact]]` row that needs un-pacting"
    );
}

#[test]
fn a_branch_that_changed_nothing_selects_nothing() {
    let repo = repository();
    let manifest = Manifest::with_entries([repo.fresh("."), repo.fresh("crates")]);

    // Stale on disk, and none of it this branch's.
    repo.write(
        "crates/engine/src/lib.rs",
        "pub fn read_one(at: usize) {}\n",
    );

    assert!(
        made_stale(repo.root(), &manifest, &[]).is_empty(),
        "with no changed path there is nothing at or below any directory, not \
         even the root"
    );
}

#[test]
fn a_path_from_outside_the_repository_is_dropped() {
    let repo = repository();
    let manifest = Manifest::with_entries([repo.fresh(".")]);

    repo.write(
        "crates/engine/src/lib.rs",
        "pub fn read_one(at: usize) {}\n",
    );

    assert!(
        made_stale(
            repo.root(),
            &manifest,
            &changed(["/elsewhere/crates/engine/src/lib.rs", "../sibling/file.rs"])
        )
        .is_empty(),
        "a path with no spelling relative to this root names no directory in it, \
         and the root is not refreshed on the strength of one"
    );
}

// The pass. Everything below drives the whole of `freshened` against a real
// temporary repository, a stub agent and the scripted `Checkout`: no `git` runs,
// no socket is opened and no model is reached, and what the passes wrote is read
// back off the disk they wrote it to.

const TICKET: &str = "WAR-141";
const DEFAULT: &str = "main";
const SCOPE: &str = "data-plane";

// An agent that would be a bug to reach. A panic rather than a count read
// afterwards, because a count says a pass was spent and not which directory paid
// for it.
struct Never;

impl Agent for Never {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        panic!("this run was owed no model pass")
    }
}

// No `claude` on the machine: a pass that fails for every directory it is asked
// about, which is the ordinary way a refresh fails on a real checkout.
struct Refusing;

impl Agent for Refusing {
    fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
        Err(agent::Error::NotFound {
            program: "claude".to_owned(),
        })
    }
}

// A repository as a branch finds it: two directories and the root pacted, a
// document on disk for each, and a grant against the bytes beside it.
//
// Pacted by the very `descend` the pass runs rather than by a hand-built
// manifest, so the documents, the grants and the hashes agree the way they do on
// a checkout — which is what makes "still stale afterwards" and "fresh
// afterwards" below mean anything.
fn pacted() -> (Repo, Manifest) {
    let repo = Repo::new();
    repo.write("src/lib.rs", "//! a module\n");
    repo.write("docs/notes.md", "# Notes\n");

    let subtree = descend(
        Descent::Pact,
        repo.root(),
        repo.root(),
        &Manifest::new(),
        &Passing::filling(),
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a pact of a readable subtree");
    assert!(subtree.failures.is_empty(), "{:?}", subtree.failures);
    (repo, subtree.manifest)
}

fn asked<'a>(
    repo: &'a Repo,
    checkout: &'a Checkout,
    manifest: &'a Manifest,
    held: &'a [String],
) -> Freshening<'a> {
    Freshening {
        ticket: TICKET,
        repo: checkout,
        root: repo.root(),
        manifest,
        held,
    }
}

// A checkout that answers one diff and one status, which is all one pass asks it
// for before the commit.
fn checkout(changed: Vec<&'static str>, tree: Vec<Dirty>) -> Checkout {
    Checkout::clean(DEFAULT).changed([changed]).trees([tree])
}

// What `git status` says about the documents a refresh rewrote.
fn modified<const N: usize>(paths: [&str; N]) -> Vec<Dirty> {
    paths
        .iter()
        .map(|path| Dirty {
            code: "M".to_owned(),
            path: (*path).to_owned(),
            from: None,
        })
        .collect()
}

fn commits(checkout: &Checkout) -> Vec<GitCall> {
    checkout
        .calls()
        .into_iter()
        .filter(|call| matches!(call, GitCall::CommitPaths { .. }))
        .collect()
}

// The engine's own judgement of a directory, read back off the disk the pass
// wrote to: `PactedFresh` is a document that describes what is there now.
fn state(repo: &Repo, module: &str) -> NodeState {
    let manifest = Manifest::load(repo.root()).expect("the manifest the descent saved");
    let entry = manifest.entry(module).expect("a pacted module");
    let hash = subtree_hash(entry.module_path(repo.root())).expect("a directory on disk hashes");
    decide_state(Some(entry), &hash)
}

fn document(repo: &Repo, relative: &str) -> Vec<u8> {
    fs::read(repo.root().join(relative)).expect("a document a pact wrote")
}

// Every directory a pass was spent on, in the order the passes ran, as the
// descents themselves report it.
fn described(events: &[Event], root: &Path) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Starting { directory, .. } => Some(
                warlock_engine::to_manifest_path(root, directory)
                    .expect("a directory inside the root"),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn a_branch_that_left_nothing_stale_runs_no_pass_and_asks_the_checkout_nothing_more() {
    let (repo, manifest) = pacted();
    // The branch touched a pacted directory and left it as it found it, so every
    // grant still stands.
    let checkout = checkout(vec!["docs/notes.md"], Vec::new());

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Never,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a checkout that answers");

    assert_eq!(
        outcome,
        Freshened::default(),
        "nothing was stale to refresh"
    );
    assert_eq!(
        checkout.calls(),
        [
            GitCall::DefaultBranch,
            GitCall::ChangedAgainst(DEFAULT.to_owned())
        ],
        "with nothing selected there is no status to read and no commit to make"
    );
}

#[test]
fn a_stale_directory_is_refreshed_and_the_documents_are_one_commit() {
    let (repo, manifest) = pacted();
    repo.write("src/lib.rs", "//! a module, revised\npub fn read() {}\n");
    // The code file is in the tree beside the documents, because the refresh runs
    // after the sub-task commits on a branch that changed it.
    let checkout = checkout(
        vec!["src/lib.rs"],
        modified([
            "src/.warlock.md",
            ".warlock.md",
            ".warlock/pacts.toml",
            "src/lib.rs",
        ]),
    );

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Passing::filling(),
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a checkout that answers");

    assert_eq!(outcome.refreshed, ["src", "."]);
    assert!(outcome.left_stale.is_empty(), "{:?}", outcome.left_stale);
    for module in ["src", "."] {
        assert_eq!(
            state(&repo, module),
            NodeState::PactedFresh,
            "`{module}` is still stale after the pass that was supposed to put it back"
        );
    }
    assert_eq!(
        commits(&checkout),
        [GitCall::CommitPaths {
            message: format!("{TICKET}: refresh .warlock.md"),
            paths: vec![
                "src/.warlock.md".to_owned(),
                ".warlock.md".to_owned(),
                ".warlock/pacts.toml".to_owned(),
            ],
        }],
        "one commit, of the documents and the manifest and of no code file beside them"
    );
}

#[test]
fn the_passes_run_children_before_parents() {
    let (repo, manifest) = pacted();
    repo.write("src/lib.rs", "//! a module, revised\n");
    repo.write("docs/notes.md", "# Notes, revised\n");
    let checkout = checkout(
        vec!["src/lib.rs", "docs/notes.md"],
        modified(["docs/.warlock.md", "src/.warlock.md", ".warlock.md"]),
    );

    let mut events = Vec::new();
    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Passing::filling(),
        &Cancel::new(),
        &mut |event| events.push(event),
    )
    .expect("a checkout that answers");

    assert_eq!(outcome.refreshed, ["docs", "src", "."]);
    assert_eq!(
        described(&events, repo.root()),
        ["docs", "src", "."],
        "a parent's document is written from its children's, so the root is \
         described last — and each child costs exactly one pass, because the \
         root's own descent finds them fresh"
    );
}

#[test]
fn a_refresh_that_changed_nothing_on_disk_asks_for_no_commit() {
    let (repo, manifest) = pacted();
    repo.write("src/lib.rs", "//! a module, revised\n");
    // The pass ran and wrote the documents it already had, word for word: the
    // tree is clean, and `git commit` over an empty diff would refuse.
    let checkout = checkout(vec!["src/lib.rs"], Vec::new());

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Passing::filling(),
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a checkout that answers");

    assert_eq!(outcome.refreshed, ["src", "."]);
    assert!(
        checkout.calls().contains(&GitCall::Dirty),
        "the commit decision is the tree's to make, so the tree is read"
    );
    assert!(
        commits(&checkout).is_empty(),
        "nothing changed on disk, so there is nothing to commit: {:?}",
        checkout.calls()
    );
}

#[test]
fn a_closed_scope_runs_no_pass_and_is_left_stale_with_the_boundarys_sentence() {
    let (repo, pacted) = pacted();
    // `src` alone, under a scope, and the root left unpacted: with nothing pacted
    // above it there is no ancestor whose own pass would descend into it, so what
    // this test asserts about is the gate and nothing else.
    let scoped = pacted
        .entry("src")
        .expect("the pact wrote an entry for `src`")
        .clone()
        .with_scope(SCOPE);
    let manifest = Manifest::with_entries([scoped]);
    repo.write("src/lib.rs", "//! a module, revised\n");
    let before = document(&repo, "src/.warlock.md");
    let checkout = checkout(vec!["src/lib.rs"], Vec::new());

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Never,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a checkout that answers");

    assert!(outcome.refreshed.is_empty(), "{:?}", outcome.refreshed);
    assert_eq!(
        outcome
            .left_stale
            .iter()
            .map(|left| (left.directory.clone(), left.reason.clone()))
            .collect::<Vec<_>>(),
        [("src".to_owned(), closed_scope_message("src", SCOPE))],
        "the boundary's own sentence, said about the directory the manifest names"
    );
    assert_eq!(
        document(&repo, "src/.warlock.md"),
        before,
        "a directory this machine's sigils do not open had its document rewritten"
    );
    assert!(
        commits(&checkout).is_empty(),
        "a run that wrote nothing asked for a commit"
    );
}

#[test]
fn a_pass_the_model_failed_leaves_the_directory_stale_and_does_not_end_the_run() {
    let (repo, manifest) = pacted();
    repo.write("src/lib.rs", "//! a module, revised\n");
    let checkout = checkout(vec!["src/lib.rs"], Vec::new());

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Refusing,
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a failed pass is an outcome, not an error that ends the pull");

    assert!(outcome.refreshed.is_empty(), "{:?}", outcome.refreshed);
    assert_eq!(
        outcome
            .left_stale
            .iter()
            .map(|left| left.directory.clone())
            .collect::<Vec<_>>(),
        ["src", "."],
        "both selected directories are still stale: the pass over `src` failed, \
         and a refresh skips everything above a failure"
    );
    for left in &outcome.left_stale {
        assert!(
            left.reason.contains("the refresh pass failed")
                && left.reason.contains("`src`")
                && left.reason.contains("claude"),
            "the reason has to name the directory that failed and what failed \
             about it: {}",
            left.reason
        );
    }
    assert_eq!(
        state(&repo, "src"),
        NodeState::PactedStale,
        "a directory reported as left stale is fresh on disk"
    );
}

#[test]
fn a_pacted_directory_that_is_gone_is_left_stale_and_the_run_carries_on() {
    let (repo, manifest) = pacted();
    // The branch deleted a pacted directory outright, which is a `[[pact]]` row
    // that wants un-pacting: its subtree cannot be walked, so the descent refuses.
    repo.remove_directory("docs");
    let checkout = checkout(
        vec!["docs/notes.md"],
        modified([".warlock.md", ".warlock/pacts.toml"]),
    );

    let outcome = freshened(
        &asked(&repo, &checkout, &manifest, &[]),
        &Passing::filling(),
        &Cancel::new(),
        &mut |_| {},
    )
    .expect("a descent that refused is an outcome, not an error that ends the pull");

    assert_eq!(
        outcome.refreshed,
        ["."],
        "the directory below refused and the root was still refreshed"
    );
    assert_eq!(
        outcome
            .left_stale
            .iter()
            .map(|left| left.directory.clone())
            .collect::<Vec<_>>(),
        ["docs"]
    );
    let reason = &outcome.left_stale[0].reason;
    assert!(
        reason.contains("docs") && !reason.contains('\n'),
        "the descent's own sentence, on one line: {reason}"
    );
    assert_eq!(
        commits(&checkout),
        [GitCall::CommitPaths {
            message: format!("{TICKET}: refresh .warlock.md"),
            paths: vec![".warlock.md".to_owned(), ".warlock/pacts.toml".to_owned()],
        }],
        "what the pass did put back is committed, and the failure does not stop it"
    );
}
