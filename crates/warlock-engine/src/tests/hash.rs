use std::fs;
use std::path::Path;

use super::{Error, subtree_hash};

/// Built in a temporary directory, so no test here says anything about the
/// warlock repository itself.
fn fixture() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("a temporary directory");
    write(repo.path(), "WARLOCK.md", "# repo\n");
    write(repo.path(), ".gitignore", "/target\n");
    write(repo.path(), ".warlockignore", "notes/\n");
    write(repo.path(), "notes/scratch.md", "thinking out loud\n");
    write(repo.path(), "crates/engine/WARLOCK.md", "# engine\n");
    write(repo.path(), "crates/engine/src/lib.rs", "pub fn one() {}\n");
    write(repo.path(), "crates/engine/src/deep/WARLOCK.md", "# deep\n");
    write(repo.path(), "crates/tui/WARLOCK.md", "# tui\n");
    write(repo.path(), "crates/tui/README.md", "# for people\n");
    write(repo.path(), "target/debug/junk.bin", "build output\n");
    write(repo.path(), ".warlock/pacts.toml", "version = 1\n");
    repo
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
    fs::write(path, contents).expect("writes a file");
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("creates the destination");
    for entry in fs::read_dir(from).expect("reads a directory") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copies a file");
        }
    }
}

fn hash(root: &Path, relative: &str) -> String {
    subtree_hash(root.join(relative)).expect("hashes")
}

const SPINE: [&str; 4] = ["", "crates", "crates/engine", "crates/engine/src"];

#[test]
fn the_same_tree_hashes_the_same_twice() {
    let repo = fixture();
    let first = hash(repo.path(), "");

    assert_eq!(first, hash(repo.path(), ""));
    assert_eq!(first.len(), 64, "blake3 as lowercase hex: {first}");
    assert!(
        first
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "{first}"
    );
}

#[test]
fn a_copy_at_a_different_absolute_path_hashes_the_same() {
    let repo = fixture();
    let elsewhere = tempfile::tempdir().expect("a second temporary directory");
    let copy = elsewhere.path().join("under/another/name");
    copy_dir(repo.path(), &copy);

    assert_ne!(repo.path(), copy, "two genuinely different absolute paths");
    for node in SPINE {
        assert_eq!(
            hash(repo.path(), node),
            hash(&copy, node),
            "`{node}` holds the same bytes in both, so it hashes the same"
        );
    }
}

#[test]
fn editing_a_leaf_changes_that_leaf_and_every_ancestor() {
    let repo = fixture();
    let before: Vec<String> = SPINE.iter().map(|node| hash(repo.path(), node)).collect();
    let untouched = hash(repo.path(), "crates/tui");

    write(repo.path(), "crates/engine/src/lib.rs", "pub fn two() {}\n");

    for (node, was) in SPINE.iter().zip(&before) {
        assert_ne!(
            hash(repo.path(), node),
            *was,
            "`{node}` is at or above the edited file, so its hash moves"
        );
    }
    assert_eq!(
        hash(repo.path(), "crates/tui"),
        untouched,
        "a sibling subtree holding none of the edited file is unmoved"
    );
}

#[test]
fn editing_inside_an_ignored_or_warlock_directory_changes_nothing() {
    let repo = fixture();
    let before: Vec<String> = SPINE.iter().map(|node| hash(repo.path(), node)).collect();

    write(
        repo.path(),
        "target/debug/junk.bin",
        "different build output\n",
    );
    write(
        repo.path(),
        "target/debug/extra.bin",
        "a whole new artefact\n",
    );
    write(
        repo.path(),
        ".warlock/pacts.toml",
        "version = 1\n\n[[pact]]\n",
    );
    write(repo.path(), ".warlock/WARLOCK.md", "# not a module\n");
    // The third exclusion, the repository's own `.warlockignore`.
    write(repo.path(), "notes/scratch.md", "thought better of it\n");
    write(repo.path(), "notes/plan.md", "a whole new note\n");
    fs::remove_file(repo.path().join("notes/plan.md")).expect("removes");
    write(repo.path(), "notes/plan.md", "and a different one again\n");

    for (node, was) in SPINE.iter().zip(&before) {
        assert_eq!(
            hash(repo.path(), node),
            *was,
            "`target/` is gitignored, `notes/` is `.warlockignore`d and \
                 `.warlock/` is ours, so `{node}` sees none of them"
        );
    }
}

#[test]
fn a_directory_the_repository_excluded_hashes_as_an_empty_one() {
    let repo = fixture();
    let nothing =
        subtree_hash(tempfile::tempdir().expect("a temporary directory").path()).expect("hashes");

    assert_eq!(
        hash(repo.path(), "notes"),
        nothing,
        "handing an excluded directory over directly is not a way round \
             the rules: it holds nothing Warlock covers, so it hashes as an \
             empty directory does"
    );

    write(repo.path(), "notes/scratch.md", "rewritten\n");
    write(
        repo.path(),
        "notes/deep/further.md",
        "and one further down\n",
    );
    assert_eq!(hash(repo.path(), "notes"), nothing, "still nothing");

    assert_ne!(
        hash(repo.path(), "crates/tui"),
        nothing,
        "a directory the rules did not name is hashed as ever"
    );
}

#[test]
fn the_rules_are_read_where_they_sit_not_only_at_the_root() {
    let repo = fixture();
    write(repo.path(), "crates/tui/.warlockignore", "sketches/\n");
    write(repo.path(), "crates/tui/sketches/one.svg", "<svg/>\n");
    let before = hash(repo.path(), "crates/tui");
    let root_before = hash(repo.path(), "");

    write(
        repo.path(),
        "crates/tui/sketches/one.svg",
        "<svg>two</svg>\n",
    );
    write(repo.path(), "crates/tui/sketches/two.svg", "<svg/>\n");

    assert_eq!(
        hash(repo.path(), "crates/tui"),
        before,
        "a `.warlockignore` in a subdirectory governs that subdirectory"
    );
    assert_eq!(
        hash(repo.path(), ""),
        root_before,
        "and it still governs it when the hash is taken from above"
    );
}

#[test]
fn a_negated_rule_puts_a_file_back_in_the_digest() {
    let repo = fixture();
    write(repo.path(), ".warlockignore", "*.log\n!keep.log\n");
    write(repo.path(), "crates/engine/noisy.log", "one\n");
    write(repo.path(), "crates/engine/keep.log", "one\n");
    let before = hash(repo.path(), "crates/engine");

    write(repo.path(), "crates/engine/noisy.log", "two\n");
    assert_eq!(
        hash(repo.path(), "crates/engine"),
        before,
        "`*.log` excludes it"
    );

    write(repo.path(), "crates/engine/keep.log", "two\n");
    assert_ne!(
        hash(repo.path(), "crates/engine"),
        before,
        "`!keep.log` puts that one back, exactly as git would"
    );
}

#[test]
fn an_anchored_rule_names_one_directory_and_not_its_namesakes() {
    let repo = fixture();
    write(repo.path(), ".warlockignore", "/docs\n");
    write(repo.path(), "docs/plan.md", "the plan\n");
    write(
        repo.path(),
        "crates/engine/docs/plan.md",
        "the engine's plan\n",
    );
    let root_before = hash(repo.path(), "");
    let engine_before = hash(repo.path(), "crates/engine");

    write(repo.path(), "docs/plan.md", "the plan, revised\n");
    assert_eq!(
        hash(repo.path(), ""),
        root_before,
        "`/docs` is anchored to the directory the rules sit in"
    );

    write(
        repo.path(),
        "crates/engine/docs/plan.md",
        "the engine's plan, revised\n",
    );
    assert_ne!(
        hash(repo.path(), "crates/engine"),
        engine_before,
        "a `docs` further down is a different directory and is covered"
    );
}

#[test]
fn a_directory_only_rule_leaves_a_file_of_the_same_name_alone() {
    let repo = fixture();
    write(repo.path(), ".warlockignore", "assets/\n");
    write(repo.path(), "crates/engine/assets/logo.png", "one\n");
    write(
        repo.path(),
        "crates/tui/assets",
        "a file, not a directory\n",
    );
    let engine_before = hash(repo.path(), "crates/engine");
    let tui_before = hash(repo.path(), "crates/tui");

    write(repo.path(), "crates/engine/assets/logo.png", "two\n");
    assert_eq!(
        hash(repo.path(), "crates/engine"),
        engine_before,
        "`assets/` excludes the directory"
    );

    write(repo.path(), "crates/tui/assets", "still a file\n");
    assert_ne!(
        hash(repo.path(), "crates/tui"),
        tui_before,
        "and the trailing slash means it excludes only directories, so a \
             file called `assets` is covered"
    );
}

#[test]
fn editing_the_rules_moves_the_hash_twice_over() {
    let repo = fixture();
    let before = hash(repo.path(), "");

    write(
        repo.path(),
        ".warlockignore",
        "notes/\ncrates/tui/README.md\n",
    );
    let excluding_more = hash(repo.path(), "");
    assert_ne!(
        excluding_more, before,
        "the rules changed, and the rules are also an ordinary file in \
             the digest — either alone would move it"
    );

    write(repo.path(), ".warlockignore", "notes/\n");
    assert_eq!(
        hash(repo.path(), ""),
        before,
        "and putting them back puts the digest back"
    );
}

#[test]
fn rules_that_cannot_be_parsed_are_an_error_not_an_absence_of_rules() {
    let repo = fixture();
    // A range that runs backwards: a glob the matcher will not compile.
    write(repo.path(), "crates/engine/.warlockignore", "a[z-a]\n");

    for node in ["", "crates", "crates/engine", "crates/engine/src"] {
        let error = subtree_hash(repo.path().join(node))
            .expect_err("rules that cannot be read are not no rules");
        assert!(matches!(error, Error::Walk { .. }), "{node}: {error:?}");
        assert!(
            error.to_string().contains(".warlockignore"),
            "the failure names the file to go and fix: {error}"
        );
    }

    write(repo.path(), "crates/engine/.warlockignore", "notes/\n");
    hash(repo.path(), "crates/engine");
}

#[test]
fn renaming_a_file_with_identical_bytes_changes_the_hash() {
    let repo = fixture();
    let before = hash(repo.path(), "crates/engine");

    fs::rename(
        repo.path().join("crates/engine/src/lib.rs"),
        repo.path().join("crates/engine/src/main.rs"),
    )
    .expect("renames");

    assert_ne!(
        hash(repo.path(), "crates/engine"),
        before,
        "the path is part of the input, not just the bytes"
    );
}

#[test]
fn editing_the_nodes_own_document_changes_its_hash() {
    let repo = fixture();
    let before = hash(repo.path(), "crates/engine");
    let root_before = hash(repo.path(), "");

    write(
        repo.path(),
        "crates/engine/WARLOCK.md",
        "# engine\n\nHand-edited, which is exactly what section 9 expects.\n",
    );

    assert_ne!(hash(repo.path(), "crates/engine"), before);
    assert_ne!(hash(repo.path(), ""), root_before);
}

#[test]
fn a_plain_readme_goes_into_the_digest_like_any_other_file() {
    let repo = fixture();
    let before = hash(repo.path(), "crates/tui");

    write(
        repo.path(),
        "crates/tui/README.md",
        "# for people, revised\n",
    );

    assert_ne!(
        hash(repo.path(), "crates/tui"),
        before,
        "a `README.md` is not Warlock's document and is not special to the \
             hash either: it is a file at or below the node, so it counts"
    );
}

#[test]
fn a_file_added_or_deleted_changes_the_hash() {
    let repo = fixture();
    let before = hash(repo.path(), "crates/engine");

    write(repo.path(), "crates/engine/src/extra.rs", "");
    let with_extra = hash(repo.path(), "crates/engine");
    assert_ne!(with_extra, before, "an empty new file is still a new path");

    fs::remove_file(repo.path().join("crates/engine/src/extra.rs")).expect("removes");
    assert_eq!(hash(repo.path(), "crates/engine"), before);
}

#[test]
fn a_directory_with_no_files_hashes_and_holds_no_bytes() {
    let empty = tempfile::tempdir().expect("a temporary directory");
    fs::create_dir_all(empty.path().join("a/b/c")).expect("creates empty directories");

    let other = tempfile::tempdir().expect("a second temporary directory");

    assert_eq!(
        subtree_hash(empty.path()).expect("hashes"),
        subtree_hash(other.path()).expect("hashes"),
        "directories are not part of the input, only files are"
    );
}

/// Unix-only because the fixture needs `std::os::unix::fs::symlink` to
/// build the cycle; what it covers is not platform-specific.
#[cfg(unix)]
#[test]
fn a_symlinked_directory_cycle_terminates() {
    let repo = fixture();
    let before = hash(repo.path(), "crates/engine");

    std::os::unix::fs::symlink(repo.path(), repo.path().join("crates/engine/up"))
        .expect("links back to the root");
    std::os::unix::fs::symlink(
        repo.path().join("crates"),
        repo.path().join("crates/engine/sideways"),
    )
    .expect("links back to an ancestor");

    assert_eq!(
        hash(repo.path(), "crates/engine"),
        before,
        "a symlink is neither followed nor hashed as its target, so the \
             hash terminates and is unchanged"
    );
}

/// Unix-only: it needs `chmod`, which cannot deny root anything, so the
/// test checks the file really is unreadable before asserting on it.
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_an_error_not_a_hash() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = fixture();
    let path = repo.path().join("crates/engine/src/lib.rs");
    let good = hash(repo.path(), "crates/engine");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmods");
    if fs::read(&path).is_ok() {
        // Running as root: there is no such thing as an unreadable file
        // here, so there is nothing to assert against.
        return;
    }

    let error = subtree_hash(repo.path().join("crates/engine"))
        .expect_err("a file that cannot be read has no hash");
    assert!(matches!(error, Error::Read { .. }), "{error:?}");
    assert!(error.to_string().contains("lib.rs"), "{error}");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmods back");
    assert_eq!(hash(repo.path(), "crates/engine"), good);

    let deleted = fixture();
    fs::remove_file(deleted.path().join("crates/engine/src/lib.rs")).expect("removes");
    assert_ne!(
        hash(deleted.path(), "crates/engine"),
        good,
        "an unreadable file must not hash like a deleted one — and it \
             cannot, because it does not hash at all"
    );
}

#[test]
fn a_directory_that_vanished_is_an_error() {
    let repo = fixture();
    let gone = repo.path().join("crates/engine/src/deep");
    fs::remove_dir_all(&gone).expect("removes");

    let error = subtree_hash(&gone).expect_err("nothing there to hash");
    assert!(matches!(error, Error::Walk { .. }), "{error:?}");
    assert!(error.to_string().contains("could not walk"), "{error}");
}

#[test]
fn every_error_variant_says_what_happened_and_where() {
    let read = Error::Read {
        path: Path::new("/repo/src/lib.rs").to_path_buf(),
        source: std::io::Error::other("boom"),
    };
    assert_eq!(
        read.to_string(),
        "could not read `/repo/src/lib.rs`, so the subtree has no hash: boom"
    );

    let named = Error::Path {
        path: Path::new("/elsewhere/x").to_path_buf(),
        source: Box::new(crate::manifest::Error::NonUtf8Path {
            path: Path::new("/elsewhere/x").to_path_buf(),
        }),
    };
    assert!(
        named
            .to_string()
            .starts_with("could not name `/elsewhere/x`")
    );
}
