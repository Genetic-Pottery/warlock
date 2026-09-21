use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::DEFAULT_TEMPLATE;

use super::{Brief, Error, ScopeBlockError, brief_at, scope_block_in};

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

// Reading a project back is a question about bytes, so every document below is
// a literal: no root, no template, no file and no socket.
const PROJECT: &str = "The problem, in prose.\n\n\
                       ## Outcome\n\nWhat somebody sees.\n\n\
                       ## Scope\n\n\
                       ### 1. Read the file\n\ndepends_on: []\n\n\
                       What this slice decides.\n\n\
                       ### 2) Write it back\n\ndepends_on: [1, 3]\n\n\
                       And what this one decides.\n\n\
                       ### An unnumbered slice\n\n\
                       Nothing depends on it.\n";

#[test]
fn a_project_is_the_brief_above_the_scope_heading_and_the_slices_under_it() {
    let block = scope_block_in(PROJECT).expect("a scope block");

    assert_eq!(
        block.brief(),
        "The problem, in prose.\n\n## Outcome\n\nWhat somebody sees."
    );
    assert_eq!(block.unreadable(), 0);

    let slices = block.slices();
    assert_eq!(slices.len(), 3);
    // Both numbering spellings and none at all: a slice is its position first,
    // and the number it was written with is only what `depends_on` speaks.
    assert_eq!(
        slices
            .iter()
            .map(|slice| (slice.position(), slice.number(), slice.heading()))
            .collect::<Vec<_>>(),
        [
            (1, Some(1), "Read the file"),
            (2, Some(2), "Write it back"),
            (3, None, "An unnumbered slice"),
        ]
    );
    // Positions, not the numbers the lines were written with: `3` is nobody's
    // number here — the third slice is unnumbered — so that reference is gone
    // and the one beside it is kept.
    assert!(slices[0].depends_on().is_empty());
    assert_eq!(slices[1].depends_on(), [1]);
    assert!(slices[2].depends_on().is_empty());

    assert_eq!(slices[0].prose(), "What this slice decides.");
    assert_eq!(slices[1].prose(), "And what this one decides.");
    assert_eq!(slices[2].prose(), "Nothing depends on it.");
}

#[test]
fn a_paragraph_that_says_the_words_is_not_a_scope_heading() {
    // The defect this exists for: a substring search for `## Scope` splits a
    // project in half at a sentence about it, and every slice is lost.
    let mentioned = "A brief about warlock writes its slices under ## Scope, which is \
                     what this one is about.\n\n\
                     ## Scope\n\n### 1. The only slice\n\nProse.\n";

    let block = scope_block_in(mentioned).expect("a scope block");

    assert!(
        block.brief().contains("under ## Scope, which is"),
        "{:?}",
        block.brief()
    );
    assert_eq!(block.slices().len(), 1);
    assert_eq!(block.slices()[0].heading(), "The only slice");
}

#[test]
fn a_heading_inside_a_fence_is_a_document_being_quoted() {
    let quoted = "It writes this:\n\n\
                  ```md\n## Scope\n\n### 1. Not a slice\n```\n\n\
                  ## Scope\n\n\
                  ### 1. The only slice\n\n\
                  It writes this too:\n\n\
                  ```md\n### 2. Also not a slice\n```\n";

    let block = scope_block_in(quoted).expect("a scope block");

    assert!(
        block.brief().contains("### 1. Not a slice"),
        "split inside a fence: {:?}",
        block.brief()
    );
    assert_eq!(block.slices().len(), 1);
    assert!(
        block.slices()[0]
            .prose()
            .contains("```md\n### 2. Also not a slice\n```"),
        "{:?}",
        block.slices()[0].prose()
    );
}

#[test]
fn depends_on_is_only_the_first_line_under_a_heading() {
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. No line of its own\n\n\
                    The first line of prose.\n\n\
                    depends_on: [2]\n\n\
                    More prose.\n\n\
                    ### 2. One that has one\n\ndepends_on: [1]\n\nProse.\n";

    let block = scope_block_in(document).expect("a scope block");

    // Deeper in the body it is a sentence about dependencies, and it stays in
    // the prose rather than being lifted out of the middle of a paragraph.
    assert!(block.slices()[0].depends_on().is_empty());
    assert_eq!(
        block.slices()[0].prose(),
        "The first line of prose.\n\ndepends_on: [2]\n\nMore prose."
    );
    // And the line that is a declaration is consumed rather than left to
    // become the first paragraph of a ticket.
    assert_eq!(block.slices()[1].depends_on(), [1]);
    assert_eq!(block.slices()[1].prose(), "Prose.");
}

#[test]
fn a_heading_with_nothing_in_it_is_counted_rather_than_dropped() {
    // Red's parser drops these in silence, and its own docstring says the
    // cost: a project that fails to parse gives the person no way to see why.
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. A real slice\n\nProse.\n\n\
                    ###\n\n### 4.\n\n### 5)\n";

    let block = scope_block_in(document).expect("a scope block");

    assert_eq!(block.slices().len(), 1);
    assert_eq!(block.unreadable(), 3);
    assert_eq!(block.slices()[0].prose(), "Prose.");
}

#[test]
fn a_project_with_no_scope_heading_and_one_with_no_slices_are_two_refusals() {
    let none = scope_block_in("A document about a change, and no scope section.\n")
        .expect_err("a refusal");
    let empty = scope_block_in("Why.\n\n## Scope\n\nA sentence where the slices go.\n")
        .expect_err("a refusal");

    assert!(matches!(none, ScopeBlockError::NoScope));
    assert!(matches!(empty, ScopeBlockError::NoSlices));

    let none = none.to_string();
    let empty = empty.to_string();
    assert!(!none.contains('\n'), "wrapped: {none}");
    assert!(!empty.contains('\n'), "wrapped: {empty}");
    // Each says which of the two it is, because the fix is a different edit.
    assert!(none.contains("## Scope"), "{none}");
    assert!(empty.contains("### "), "{empty}");
    assert_ne!(none, empty);
}

fn cut_in_order(document: &str) -> Vec<usize> {
    let block = scope_block_in(document).expect("a scope block");
    block
        .ordered()
        .iter()
        .map(|slice| slice.position())
        .collect()
}

fn scope_refusal(document: &str) -> String {
    let message = scope_block_in(document).expect_err("a refusal").to_string();
    assert!(!message.contains('\n'), "wrapped: {message}");
    message
}

#[test]
fn a_reference_is_read_through_the_numbers_the_document_was_written_with() {
    // Numbered 1, 3, 7 — a scope somebody deleted two slices out of and never
    // renumbered. `depends_on: [3]` is the second slice, not a third one.
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. First\n\ndepends_on: []\n\n\
                    ### 3. Second\n\ndepends_on: [1]\n\n\
                    ### 7. Third\n\ndepends_on: [3, 1]\n";

    let block = scope_block_in(document).expect("a scope block");

    let slices = block.slices();
    assert_eq!(slices[1].depends_on(), [1]);
    assert_eq!(slices[2].depends_on(), [2, 1]);
    assert_eq!(cut_in_order(document), [1, 2, 3]);
}

#[test]
fn a_reference_to_nothing_and_a_reference_to_itself_are_both_dropped() {
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. First\n\ndepends_on: [1, 9]\n\n\
                    ### 2. Second\n\ndepends_on: [2, 1]\n";

    let block = scope_block_in(document).expect("a scope block");

    // `9` is nobody's, and each slice's reference to itself would be a circle
    // of one. Dropped rather than refused: the document is still cuttable.
    assert!(block.slices()[0].depends_on().is_empty());
    assert_eq!(block.slices()[1].depends_on(), [1]);
    assert_eq!(cut_in_order(document), [1, 2]);
}

#[test]
fn two_slices_written_with_one_number_are_answered_by_the_earlier_one() {
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. First\n\ndepends_on: []\n\n\
                    ### 2. Second\n\ndepends_on: []\n\n\
                    ### 2. Renumbered by nobody\n\ndepends_on: [2]\n";

    let block = scope_block_in(document).expect("a scope block");

    // A reader counting down the document for `2.` stops at the first one, so
    // this does too — and the third slice is a slice, not a second second.
    assert_eq!(block.slices().len(), 3);
    assert_eq!(block.slices()[2].depends_on(), [2]);
}

#[test]
fn the_slices_come_back_dependency_first_rather_than_in_document_order() {
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. Written first, cut last\n\ndepends_on: [3]\n\n\
                    ### 2. On its own\n\ndepends_on: []\n\n\
                    ### 3. What the first one waits on\n\ndepends_on: []\n";

    let block = scope_block_in(document).expect("a scope block");

    assert_eq!(cut_in_order(document), [2, 3, 1]);
    // The slices themselves stay as the document has them, so a position still
    // counts down the page.
    assert_eq!(block.slices()[0].heading(), "Written first, cut last");
}

#[test]
fn a_tenth_slice_is_not_cut_before_the_ninth() {
    // Positions compared as numbers and never as text: sorted as strings, `10`
    // comes before `9` and the order is wrong in exactly the scope big enough
    // for nobody to notice.
    let mut document =
        String::from("Why.\n\n## Scope\n\n### 1. Waits for the last one\n\ndepends_on: [10]\n\n");
    document.extend(
        (2..=10).map(|number| format!("### {number}. Slice {number}\n\ndepends_on: []\n\n")),
    );

    assert_eq!(cut_in_order(&document), [2, 3, 4, 5, 6, 7, 8, 9, 10, 1]);
}

#[test]
fn two_slices_waiting_on_each_other_refuse_and_are_both_named() {
    // Red's parser falls back to document order here. Refused instead: the
    // order is what the tickets are filed and blocked in, so a guess is a
    // wrong board rather than a wrong line.
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. Read it back\n\ndepends_on: [2]\n\n\
                    ### 2. Write it out\n\ndepends_on: [1]\n";

    let error = scope_block_in(document).expect_err("a refusal");
    assert!(matches!(error, ScopeBlockError::Circle { .. }));

    let message = scope_refusal(document);
    assert!(message.contains("slice 1 `Read it back`"), "{message}");
    assert!(message.contains("slice 2 `Write it out`"), "{message}");
}

#[test]
fn a_three_slice_circle_names_the_three_and_not_the_slice_that_is_fine() {
    let document = "Why.\n\n## Scope\n\n\
                    ### 1. On its own\n\ndepends_on: []\n\n\
                    ### 2. Second\n\ndepends_on: [4]\n\n\
                    ### 3. Third\n\ndepends_on: [2]\n\n\
                    ### 4. Fourth\n\ndepends_on: [3]\n";

    let message = scope_refusal(document);

    assert!(
        message.contains("slice 2 `Second`, slice 3 `Third` and slice 4 `Fourth`"),
        "{message}"
    );
    assert!(
        !message.contains("On its own"),
        "a slice with an order was named: {message}"
    );
}

#[test]
fn a_red_scope_marker_is_neither_required_nor_consumed() {
    // The markers in `docs/warlock-brief-23` sit immediately above the
    // headings. Tolerated and nothing more: reading one as warlock's would
    // make a foreign tool's comment part of this format.
    let marked = "Why.\n\n## Scope\n\n\
                  <!-- red:scope -->\n### 1. A slice\n\ndepends_on: []\n\nProse.\n\n\
                  <!-- red:scope -->\n### 2. Another\n\ndepends_on: [1]\n\nMore prose.\n";

    let block = scope_block_in(marked).expect("a scope block");

    assert_eq!(block.slices().len(), 2);
    assert_eq!(block.unreadable(), 0);
    assert_eq!(block.slices()[1].depends_on(), [1]);
    assert_eq!(
        block.slices()[0].prose(),
        "Prose.\n\n<!-- red:scope -->",
        "a marker was read as warlock's",
    );
}
