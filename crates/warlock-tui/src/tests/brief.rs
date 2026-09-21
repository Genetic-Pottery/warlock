use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::DEFAULT_TEMPLATE;

use super::{Brief, Error, brief_at};

// Every section the built-in shape asks for, so a repository that has written
// no template of its own holds this document to something it satisfies.
const BRIEF: &str = "# Push a brief to the board\n\n\
                     Nothing turns a document on disk into a project.\n\n\
                     ## Outcome\n\n`warlock push docs/brief.md` files it.\n\n\
                     ## Success criteria\n\n**The reader**\n\n- sees a URL\n\n\
                     ## Constraints\n\nNo new dependency.\n\n\
                     ## Out of scope\n\nPulling anything back.\n\n\
                     ## Scope\n\n### 1. Read the file\n\ndepends_on: []\n";

fn a_root() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// One name for every document a test writes, so two refusals about two
// different documents differ only where the wording differs.
fn write_brief(root: &Path, document: &str) -> PathBuf {
    let path = root.join("docs").join("brief.md");
    fs::create_dir_all(path.parent().expect("a `docs` directory")).expect("a `docs` directory");
    fs::write(&path, document).expect("a brief file");
    path
}

fn read_back(root: &Path, document: &str) -> Brief {
    let path = write_brief(root, document);
    brief_at(root, &path).expect("a brief")
}

fn refusal(root: &Path, document: &str) -> String {
    let path = write_brief(root, document);
    let message = brief_at(root, &path).expect_err("a refusal").to_string();
    assert!(!message.contains('\n'), "wrapped: {message}");
    message
}

#[test]
fn a_brief_is_its_title_line_and_the_document_under_it() {
    let root = a_root();

    let brief = read_back(root.path(), BRIEF);

    assert_eq!(brief.name(), "Push a brief to the board");
    // The words are the document's, the headings are where they were, and the
    // blank line that followed the title is still there. What the body is not
    // is the file byte for byte: see the two tests below for the two things
    // `for_the_board` does and the reasons it does them.
    assert!(brief.content().starts_with('\n'));
    assert!(
        brief
            .content()
            .contains("Nothing turns a document on disk into a project."),
        "{:?}",
        brief.content()
    );
    for kept in [
        "## Outcome",
        "## Success criteria",
        "**The reader**",
        "## Scope",
        "### 1. Read the file",
        "depends_on: []",
    ] {
        assert!(
            brief.content().lines().any(|line| line == kept),
            "{kept} is not a line of its own: {:?}",
            brief.content()
        );
    }
}

#[test]
fn a_title_is_the_text_and_not_the_spaces_around_it() {
    let root = a_root();
    let document = BRIEF.replacen(
        "# Push a brief to the board",
        "#   Push a brief to the board \t",
        1,
    );

    let brief = read_back(root.path(), &document);
    // Asserted against the same document with an ordinary title rather than
    // against a second spelling of what the body should be: the title line is
    // trimmed and nothing below it is read differently for it.
    let ordinary = read_back(root.path(), BRIEF);

    assert_eq!(brief.name(), "Push a brief to the board");
    assert_eq!(brief.content(), ordinary.content());
}

#[test]
fn a_wrapped_paragraph_goes_up_as_one_line() {
    // The defect this exists for: a brief is hard wrapped for a terminal, and
    // an editor that reads a single newline as a line break renders every
    // paragraph of it ragged. The words and the paragraph breaks are the
    // document's; the column width was never part of it.
    let root = a_root();
    let document = BRIEF.replacen(
        "Nothing turns a document on disk into a project.",
        "Nothing turns a document\non disk into a project.\n\nAnd a second\nparagraph.",
        1,
    );

    let brief = read_back(root.path(), &document);

    assert!(
        brief.content().contains(
            "\nNothing turns a document on disk into a project.\n\nAnd a second paragraph.\n"
        ),
        "{:?}",
        brief.content()
    );
}

#[test]
fn a_fenced_block_keeps_every_line_it_was_written_with() {
    // Inside a fence a line break is content, so the rule that joins prose has
    // to stop at one — a shell session joined into a paragraph is a session
    // nobody can run.
    let root = a_root();
    let session = "```sh\n$ warlock push docs/brief.md\nwarlock: filed to WAR\n```";
    let document = BRIEF.replacen(
        "`warlock push docs/brief.md` files it.",
        &format!("It files it:\n\n{session}"),
        1,
    );

    let brief = read_back(root.path(), &document);

    assert!(brief.content().contains(session), "{:?}", brief.content());
}

#[test]
fn only_the_success_criteria_are_made_checkable() {
    // A criterion is ticked off on the board. A constraint and an out-of-scope
    // line are decisions somebody made once, and a box beside either would ask
    // a reader to do something about it.
    let root = a_root();
    let document = BRIEF
        .replacen(
            "## Constraints\n\nNo new dependency.",
            "## Constraints\n\n- No new dependency.",
            1,
        )
        .replacen(
            "## Out of scope\n\nPulling anything back.",
            "## Out of scope\n\n- Pulling anything back.",
            1,
        );

    let brief = read_back(root.path(), &document);

    assert!(
        brief.content().contains("- [ ] sees a URL"),
        "the criterion is not checkable: {:?}",
        brief.content()
    );
    assert!(
        brief.content().contains("\n- No new dependency."),
        "a constraint was made a task: {:?}",
        brief.content()
    );
    assert!(
        brief.content().contains("\n- Pulling anything back."),
        "an out-of-scope line was made a task: {:?}",
        brief.content()
    );
}

#[test]
fn a_criterion_written_with_a_box_keeps_the_one_it_has() {
    // Written by a hand that already knew, or pushed a second time from a
    // document somebody edited on the board and pasted back.
    let root = a_root();
    let document = BRIEF.replacen("- sees a URL", "- [ ] sees a URL\n- [x] and a done one", 1);

    let brief = read_back(root.path(), &document);

    assert!(
        brief.content().contains("- [ ] sees a URL"),
        "{:?}",
        brief.content()
    );
    assert!(
        brief.content().contains("- [x] and a done one"),
        "{:?}",
        brief.content()
    );
    assert!(
        !brief.content().contains("[ ] [ ]"),
        "a box grew a second box: {:?}",
        brief.content()
    );
}

#[test]
fn a_title_is_read_the_way_a_filename_was_proposed_from_one() {
    // `slug_of` in `writing.rs`, to the letter: the prefix is `# ` exactly, so
    // none of these is a title and all of them are the same refusal.
    let root = a_root();
    let first = refusal(root.path(), &BRIEF.replacen("# Push", "#Push", 1));

    for document in [
        // A hash with no space after it.
        BRIEF.replacen("# Push", "#Push", 1),
        // A section heading, which is a level down and not a title.
        BRIEF.replacen("# Push", "## Push", 1),
        // An indented heading, which markdown would still draw as one.
        BRIEF.replacen("# Push", "  # Push", 1),
        BRIEF.replacen("# Push", "\t# Push", 1),
        // No heading of any kind.
        BRIEF.replacen("# Push a brief to the board\n", "", 1),
    ] {
        assert_eq!(refusal(root.path(), &document), first, "for {document:?}");
    }

    assert!(
        first.contains("no `# ` title line"),
        "says nothing about the rule: {first}"
    );
}

#[test]
fn a_title_that_says_nothing_is_the_same_refusal_as_no_title_at_all() {
    // The document is otherwise perfect, which is exactly when an `untitled`
    // fallback would look harmless: a project on somebody's board called
    // `untitled` is worse than a refusal a person can act on.
    let root = a_root();
    let blank = refusal(
        root.path(),
        &BRIEF.replacen("# Push a brief to the board", "#  \t ", 1),
    );
    let none = refusal(
        root.path(),
        &BRIEF.replacen("# Push a brief to the board\n", "", 1),
    );

    assert_eq!(blank, none);
    assert!(!blank.contains("untitled"), "a fallback leaked: {blank}");
}

#[test]
fn a_blank_first_heading_is_the_title_rather_than_a_search_for_a_better_one() {
    // The first `# ` line is the title even when it is empty, because the
    // heading under it is a section: reading past a blank title would file the
    // project under a section name.
    let root = a_root();

    let message = refusal(root.path(), "# \n\n# A real title\n\nProse.\n");

    assert!(message.contains("no `# ` title line"), "{message}");
}

#[test]
fn a_dropped_section_is_named_and_the_title_refusal_is_not_borrowed_for_it() {
    let root = a_root();
    let document = BRIEF.replacen("## Scope", "## Slices", 1);

    let message = refusal(root.path(), &document);

    assert!(message.contains("## Scope"), "{message}");
    assert!(!message.contains("## Outcome"), "{message}");
    // A second, differently-worded refusal on purpose: what is wrong with the
    // document is not what is wrong with a titleless one, and a reader who is
    // told about a title goes looking at the wrong line.
    assert!(!message.contains("title"), "{message}");
}

#[test]
fn two_dropped_sections_are_both_named_in_one_sentence() {
    let root = a_root();
    let document =
        BRIEF
            .replacen("## Outcome", "## What happens", 1)
            .replacen("## Scope", "## Slices", 1);

    let message = refusal(root.path(), &document);

    assert!(message.contains("## Outcome and ## Scope"), "{message}");
}

#[test]
fn the_shape_is_the_repositorys_own_when_it_has_written_one() {
    // The check is `missing_sections` against `brief_template`, so a template
    // the repository wrote is the one a push is held to — not warlock's.
    let root = a_root();
    write_template(root.path(), "# A title\n\n## Rationale\n");

    let message = refusal(root.path(), BRIEF);
    assert!(message.contains("## Rationale"), "{message}");

    let brief = read_back(root.path(), "# Small\n\nWhy.\n\n## Rationale\n\nBecause.\n");
    assert_eq!(brief.name(), "Small");
}

#[test]
fn a_path_nobody_wrote_a_file_at_is_its_own_refusal() {
    let root = a_root();
    let path = root.path().join("docs").join("missing.md");

    let error = brief_at(root.path(), &path).expect_err("a refusal");

    assert!(matches!(error, Error::Absent { .. }));
    let message = error.to_string();
    assert!(message.contains("missing.md"), "{message}");
    assert!(!message.contains('\n'), "wrapped: {message}");
}

#[test]
fn a_file_that_will_not_decode_is_a_different_refusal_from_one_that_is_not_there() {
    // Bytes that are not UTF-8 are a file that exists and cannot be had: the
    // read that decodes is the read that fails. Portable, unlike a permission
    // bit, and the same case as far as this is concerned.
    let root = a_root();
    let path = write_brief(root.path(), "");
    fs::write(&path, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a brief file");

    let error = brief_at(root.path(), &path).expect_err("a refusal");

    assert!(matches!(error, Error::Unreadable { .. }));
    let message = error.to_string();
    assert!(message.contains("brief.md"), "{message}");
    assert!(!message.contains('\n'), "wrapped: {message}");
    assert_ne!(
        message,
        brief_at(root.path(), root.path().join("docs").join("gone.md"))
            .expect_err("a refusal")
            .to_string(),
    );
}

#[test]
fn a_template_that_will_not_read_is_a_refusal_and_never_the_built_in_shape() {
    // `template.rs`'s rule carried through: an unreadable template stays an
    // error, because pushing a document held to warlock's own shape instead of
    // the repository's is the wrong document on somebody's board.
    let root = a_root();
    let path = write_template(root.path(), "");
    fs::write(&path, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");
    let brief = write_brief(root.path(), BRIEF);

    let error = brief_at(root.path(), &brief).expect_err("a refusal");

    assert!(matches!(error, Error::Shape { .. }));
    let message = error.to_string();
    // Names the file, so a reader knows it is the template and not the brief.
    assert!(message.contains("brief-template.md"), "{message}");
    assert!(!message.contains('\n'), "wrapped: {message}");
    // The document satisfies the built-in shape, so a pass here would be the
    // default having been substituted behind the reader's back.
    assert!(crate::missing_sections(DEFAULT_TEMPLATE, BRIEF).is_empty());
}

fn write_template(root: &Path, text: &str) -> PathBuf {
    let path = root.join(".warlock").join("brief-template.md");
    fs::create_dir_all(path.parent().expect("a `.warlock` directory"))
        .expect("a `.warlock` directory");
    fs::write(&path, text).expect("a template file");
    path
}
