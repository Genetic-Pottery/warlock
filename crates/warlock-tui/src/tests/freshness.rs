use std::fs;
use std::path::Path;

use tempfile::TempDir;
use warlock_engine::{Manifest, PactEntry, from_manifest_path, subtree_hash};

use super::made_stale;

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
        PactEntry::new(self.root(), &directory, directory.join("WARLOCK.md"))
            .expect("a module inside the root")
    }
}

// Two crates and a docs directory, and not one `WARLOCK.md`: no document is
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
