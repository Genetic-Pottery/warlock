use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{SLUG_MAX, next_number, proposed_path, slug_of, unfenced};

fn a_repo() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

const DOCS: &str = "docs";

fn holding(root: &Path, directory: &str, names: &[&str]) {
    let output = root.join(directory);
    fs::create_dir_all(&output).expect("makes the output directory");
    for name in names {
        fs::write(output.join(name), "").expect("writes a file into it");
    }
}

fn docs_holding(root: &Path, names: &[&str]) {
    holding(root, DOCS, names);
}

fn titled(title: &str) -> String {
    format!("{title}\n\nSome prose about it.\n")
}

#[test]
fn a_title_is_lowercased_and_hyphenated_at_every_run_of_anything_else() {
    for (title, slug) in [
        ("# Freshness", "freshness"),
        ("# The Brief Workflow", "the-brief-workflow"),
        // Runs collapse to one hyphen however long they are, and whatever
        // they are made of.
        ("# Read   what  it wrote", "read-what-it-wrote"),
        ("# Scopes & sigils", "scopes-sigils"),
        ("# back  --  to  green", "back-to-green"),
        // Both ends trimmed, so no filename starts or ends in a separator.
        ("# --Big files!--", "big-files"),
        ("#    At work   ", "at-work"),
        // Digits are alphanumeric and stay where they are.
        ("# 12 factor apps", "12-factor-apps"),
        // A script warlock has never heard of keeps its own letters rather
        // than folding away to nothing.
        ("# Дом and home", "дом-and-home"),
    ] {
        assert_eq!(slug_of(&titled(title)), slug, "the slug for {title:?}");
    }
}

#[test]
fn nothing_before_a_colon_is_stripped() {
    // The rule that is easiest to get wrong by being helpful: a title's
    // first word is usually its subject, and a filename that throws it away
    // is a filename nobody can find again.
    assert_eq!(
        slug_of(&titled("# Scopes: the boundary problem")),
        "scopes-the-boundary-problem"
    );
}

#[test]
fn the_first_hash_space_line_is_the_title_and_nothing_else_is() {
    // A heading is a line beginning with `# ` exactly. A deeper heading, an
    // indented one and a hash with no space after it are all not one, so
    // the first *real* heading wins however far down it is.
    let reply = "## Not this\n   # Nor this\n#NorThis\n\n# The real title\n\n# A later one\n";

    assert_eq!(slug_of(reply), "the-real-title");
}

#[test]
fn a_title_capped_at_about_sixty_characters_breaks_at_a_hyphen() {
    let slug = slug_of(&titled(
        "# The quick brown fox jumps over the lazy dog and keeps on running for a while",
    ));

    // Cut back to the last whole word inside the cap: never `…-run`.
    assert_eq!(
        slug,
        "the-quick-brown-fox-jumps-over-the-lazy-dog-and-keeps-on"
    );
    assert!(slug.chars().count() <= SLUG_MAX, "{slug} is over the cap");
    assert!(!slug.ends_with('-'), "{slug} ends on a separator");
}

#[test]
fn a_slug_that_fits_is_left_exactly_as_it_is() {
    // Sixty characters on the nose, and the word after it: the first is
    // never touched, and the second is cut at the separator that is already
    // there rather than one character earlier.
    let sixty = "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee fffff";
    let folded = "aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-fffff";
    assert_eq!(folded.chars().count(), SLUG_MAX);

    assert_eq!(slug_of(&titled(&format!("# {sixty}"))), folded);
    assert_eq!(slug_of(&titled(&format!("# {sixty} ggggg"))), folded);
}

#[test]
fn one_word_longer_than_the_cap_is_kept_whole_rather_than_cut_in_half() {
    // "About sixty" is the whole of the promise: there is nowhere to break
    // this, and half a word is not a name.
    let long = "a".repeat(SLUG_MAX + 20);

    assert_eq!(slug_of(&titled(&format!("# {long}"))), long);
    assert_eq!(slug_of(&titled(&format!("# {long} and more"))), long);
}

#[test]
fn a_reply_that_is_entirely_one_fenced_block_is_unwrapped() {
    for reply in [
        "```\n# Freshness\n\nProse.\n```",
        // An info string, which is what a model actually writes.
        "```markdown\n# Freshness\n\nProse.\n```\n",
        // Whitespace either side of the whole thing, and a closing fence
        // longer than the one that opened it.
        "\n\n~~~\n# Freshness\n\nProse.\n~~~~\n\n",
    ] {
        assert_eq!(unfenced(reply), "# Freshness\n\nProse.", "{reply:?}");
    }
}

#[test]
fn a_document_that_merely_contains_fenced_blocks_is_left_exactly_as_it_is() {
    for reply in [
        // The ordinary case: prose with an example in it.
        "# Freshness\n\n```rust\nlet x = 1;\n```\n\nMore prose.\n",
        // Two blocks and nothing else, which opens and closes twice.
        "```\none\n```\n\n```\ntwo\n```\n",
        // A block that is never closed.
        "```\n# Freshness\n\nProse.\n",
        // A fence made of a different character than the one that opened
        // it, and a closing fence shorter than the opening one.
        "````\n# Freshness\n```\n",
        "~~~\n# Freshness\n```\n",
        // Not a fence at all.
        "# Freshness\n\nProse.\n",
        "`code` and ```more```\n# Freshness\n```\n",
        // One line, so there is no last line for a fence to close on.
        "# Freshness",
    ] {
        assert_eq!(unfenced(reply), reply, "{reply:?} was interfered with");
    }
}

#[test]
fn the_title_is_looked_for_after_the_unwrap() {
    // The whole reason the two are ordered: a fenced reply's first line is
    // a fence, and a slug taken before the unwrap would be `untitled` for a
    // document that plainly has a title.
    let repo = a_repo();

    assert_eq!(
        proposed_path(
            repo.path(),
            DOCS,
            "```markdown\n# Scopes and sigils\n\nProse.\n```\n"
        ),
        "docs/warlock-brief-01-scopes-and-sigils.md"
    );
}

#[test]
fn a_reply_with_no_heading_is_untitled() {
    let repo = a_repo();

    // The literal word, in the whole path, so the field opens on something
    // that reads as an invitation to type rather than as a bug.
    assert_eq!(
        proposed_path(repo.path(), DOCS, "Prose with no title anywhere in it.\n"),
        "docs/warlock-brief-01-untitled.md"
    );
}

#[test]
fn a_title_that_folds_away_to_nothing_is_untitled_too() {
    let repo = a_repo();

    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# ---")),
        "docs/warlock-brief-01-untitled.md"
    );
}

#[test]
fn the_number_is_one_above_the_highest_in_any_name_in_the_directory() {
    let repo = a_repo();
    // What `docs/` actually holds: documents written by another hand, in
    // another shape, which a rule that only counted warlock's own filenames
    // would ignore.
    docs_holding(
        repo.path(),
        &[
            "red-brief-01-foundations.md",
            "red-brief-12-the-brief-workflow.md",
            "red-brief-09-scopes-and-sigils.md",
            "warlock-design-doc.md",
        ],
    );

    assert_eq!(next_number(&repo.path().join("docs")), 13);
    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# Scopes and sigils")),
        "docs/warlock-brief-13-scopes-and-sigils.md"
    );
}

#[test]
fn an_empty_or_absent_directory_proposes_01() {
    // Absent: nothing has ever been written here, which is every repository
    // the first time.
    let repo = a_repo();
    assert_eq!(next_number(&repo.path().join("docs")), 1);
    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# Foundations")),
        "docs/warlock-brief-01-foundations.md"
    );

    // Empty: the directory is there and holds nothing, which is the same
    // answer.
    docs_holding(repo.path(), &[]);
    assert_eq!(next_number(&repo.path().join("docs")), 1);
    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# Foundations")),
        "docs/warlock-brief-01-foundations.md"
    );
}

#[test]
fn a_number_counts_wherever_in_the_name_it_sits() {
    let repo = a_repo();
    // Named for what this costs as much as for what it buys: a year in a
    // filename is a number in a filename, and the answer lands in an
    // editable field rather than in a file.
    docs_holding(repo.path(), &["notes-2026-08-31.md", "red-brief-03-x.md"]);

    assert_eq!(next_number(&repo.path().join("docs")), 2027);
}

#[test]
fn a_directory_is_a_name_in_the_directory_too() {
    let repo = a_repo();
    docs_holding(repo.path(), &["red-brief-02-x.md"]);
    fs::create_dir(repo.path().join("docs/07-archive")).expect("makes a subdirectory");

    assert_eq!(next_number(&repo.path().join("docs")), 8);
}

#[test]
fn nothing_outside_the_output_directory_is_counted() {
    let repo = a_repo();
    docs_holding(repo.path(), &["red-brief-04-x.md"]);
    // Beside `docs/`, not in it.
    fs::write(repo.path().join("release-99.md"), "").expect("writes a file at the root");

    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# At work")),
        "docs/warlock-brief-05-at-work.md"
    );
}

#[test]
fn the_directory_handed_in_is_both_the_path_and_what_the_number_is_counted_in() {
    // A repository that keeps its briefs somewhere else: the whole of what
    // the caller's directory changes, in one assertion. It is the first
    // segment of the proposal, and it is the directory whose names the
    // number comes off — so `plans/` holding an eighth brief proposes the
    // ninth there, and the four documents sitting in `docs/` count for
    // nothing because nothing is proposed into `docs/`.
    let repo = a_repo();
    docs_holding(repo.path(), &["red-brief-12-the-brief-workflow.md"]);
    holding(repo.path(), "plans", &["warlock-brief-08-x.md"]);

    assert_eq!(
        proposed_path(repo.path(), "plans", &titled("# Scopes and sigils")),
        "plans/warlock-brief-09-scopes-and-sigils.md"
    );
    // And a directory that is not there at all is the empty one's answer,
    // wherever it is: nothing about the default is special.
    assert_eq!(
        proposed_path(repo.path(), "notes/briefs", &titled("# Foundations")),
        "notes/briefs/warlock-brief-01-foundations.md"
    );
}

#[test]
fn past_ninety_nine_the_number_gets_wider_rather_than_wrapping() {
    // The padding exists to keep `01` ahead of `10`; it is not a promise
    // about width, and a hundredth brief is `100` rather than `00`.
    let repo = a_repo();
    docs_holding(repo.path(), &["warlock-brief-99-x.md"]);

    assert_eq!(
        proposed_path(repo.path(), DOCS, &titled("# Keeping up")),
        "docs/warlock-brief-100-keeping-up.md"
    );
}

#[test]
fn a_run_of_digits_too_long_to_be_a_number_is_ignored() {
    // A checksum in a filename is not somebody's numbering, and clamping to
    // the largest number there is would propose the same path forever
    // after.
    let repo = a_repo();
    docs_holding(
        repo.path(),
        &["sha-99999999999999999999999999.md", "red-brief-06-x.md"],
    );

    assert_eq!(next_number(&repo.path().join("docs")), 7);
}

#[test]
fn proposing_a_path_writes_nothing_and_creates_nothing() {
    // The whole module in one assertion: it reads one directory and answers
    // with a string. Nothing here makes the directory it names, and the
    // repository is exactly as empty afterwards as it was before.
    let repo = a_repo();

    let path = proposed_path(repo.path(), DOCS, &titled("# Foundations"));

    assert_eq!(path, "docs/warlock-brief-01-foundations.md");
    assert!(!repo.path().join("docs").exists(), "the directory was made");
    assert_eq!(
        fs::read_dir(repo.path())
            .expect("the repository reads back")
            .count(),
        0,
        "something was written into the repository"
    );
}
