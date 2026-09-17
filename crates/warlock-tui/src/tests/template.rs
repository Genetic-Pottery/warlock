use std::fs;
use std::path::{Path, PathBuf};

use warlock_engine::manifest_path;

use std::fmt::Write as _;

use super::{
    DEFAULT_TEMPLATE, Error, brief_template, missing_sections, sections_of, template_path,
};

fn document_with(sections: &[&str]) -> String {
    let mut document = String::from("# A change\n\nWhat is wrong now.\n");
    for section in sections {
        let _ = write!(document, "\n{section}\n\nSomething under it.\n");
    }
    document
}

const BUILT_IN: [&str; 5] = [
    "## Outcome",
    "## Success criteria",
    "## Constraints",
    "## Out of scope",
    "## Scope",
];

#[test]
fn the_built_in_shape_asks_for_its_five_sections_in_the_order_it_writes_them() {
    assert_eq!(
        sections_of(DEFAULT_TEMPLATE),
        [
            "Outcome",
            "Success criteria",
            "Constraints",
            "Out of scope",
            "Scope"
        ],
    );

    // The title line is an instruction about a title rather than a heading
    // to reproduce, and the slice headings live inside a section rather
    // than being one. Neither is a section, and both are in the template.
    assert!(DEFAULT_TEMPLATE.starts_with("# A title line"));
    assert!(DEFAULT_TEMPLATE.contains("### N."));
    assert!(
        !sections_of(DEFAULT_TEMPLATE)
            .iter()
            .any(|section| section.starts_with("A title line") || section.starts_with("N."))
    );
}

#[test]
fn a_document_with_every_section_is_missing_none() {
    assert!(missing_sections(DEFAULT_TEMPLATE, &document_with(&BUILT_IN)).is_empty());
}

#[test]
fn a_dropped_section_is_named_and_the_rest_are_not() {
    // The failure this whole check exists for: twenty turns of argument, a
    // document that reads perfectly, and no slices at the end of it.
    let dropped = document_with(&BUILT_IN[..4]);

    assert_eq!(missing_sections(DEFAULT_TEMPLATE, &dropped), ["Scope"]);
}

#[test]
fn several_dropped_sections_come_back_in_the_shapes_own_order() {
    let document = document_with(&["## Success criteria", "## Out of scope"]);

    assert_eq!(
        missing_sections(DEFAULT_TEMPLATE, &document),
        ["Outcome", "Constraints", "Scope"],
    );
}

#[test]
fn a_section_written_at_another_level_or_in_another_case_is_still_there() {
    // Both are generous on purpose: neither is a dropped section, and a
    // refusal here would throw away a document twenty turns in the making.
    for spelling in [
        "### Scope",
        "# Scope",
        "## SCOPE",
        "## scope",
        "##   Scope   ",
        "  ## Scope",
    ] {
        let mut sections: Vec<&str> = BUILT_IN[..4].to_vec();
        sections.push(spelling);

        assert!(
            missing_sections(DEFAULT_TEMPLATE, &document_with(&sections)).is_empty(),
            "{spelling:?} was read as a dropped section",
        );
    }
}

#[test]
fn a_section_named_in_prose_is_not_a_section_the_document_has() {
    // The line is asked, not the text: a document apologising for leaving
    // the scope out has not thereby got one.
    let mentioned = format!(
        "{}\nThere was no room for a ## Scope section here.\n",
        document_with(&BUILT_IN[..4]),
    );

    assert_eq!(missing_sections(DEFAULT_TEMPLATE, &mentioned), ["Scope"]);
}

#[test]
fn a_template_that_asks_for_nothing_holds_a_document_to_nothing() {
    // What an emptied `brief-template.md` already meant everywhere else,
    // said once more here: no shape is no sections, not five.
    for shape in [
        "",
        "Just prose, and not one heading in it.\n",
        "# Only a title\n",
    ] {
        assert!(
            missing_sections(shape, "# A change\n").is_empty(),
            "{shape:?}"
        );
    }
}

#[test]
fn a_repositorys_own_shape_is_the_one_a_document_is_held_to() {
    // The check follows the template rather than warlock's own idea of a
    // brief, so a repository that wrote its own sections gets those.
    let shape = "# A title\n\n## Background\n\n## Rollout\n";

    assert_eq!(
        missing_sections(shape, &document_with(&BUILT_IN)),
        ["Background", "Rollout"],
    );
    assert!(missing_sections(shape, &document_with(&["## Background", "## Rollout"])).is_empty());
}

fn a_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn write_template(root: &Path, text: &str) -> PathBuf {
    let path = template_path(root);
    fs::create_dir_all(path.parent().expect("a `.warlock` directory"))
        .expect("a `.warlock` directory");
    fs::write(&path, text).expect("a template file");
    path
}

#[test]
fn a_template_sits_beside_the_manifest_it_shares_a_directory_with() {
    let root = Path::new("/repo");

    assert_eq!(
        template_path(root),
        manifest_path(root).with_file_name("brief-template.md"),
    );
    assert_eq!(
        template_path(root),
        root.join(".warlock").join("brief-template.md"),
    );
}

#[test]
fn the_built_in_shape_states_the_problem_then_the_six_sections_in_order() {
    // The order is the argument: what is wrong, what it looks like fixed,
    // how that is checked, under what rules, what is refused, and only then
    // how the work is cut.
    let headings = [
        "\n## Outcome\n",
        "\n## Success criteria\n",
        "\n## Constraints\n",
        "\n## Out of scope\n",
        "\n## Scope\n",
    ];

    let mut previous = 0;
    for heading in headings {
        let at = DEFAULT_TEMPLATE
            .find(heading)
            .unwrap_or_else(|| panic!("the default template says {heading:?}"));
        assert!(at > previous, "{heading:?} is out of order");
        previous = at;
    }

    // The opening: a title line, and prose about the problem before any
    // heading at all.
    let opening = &DEFAULT_TEMPLATE[..DEFAULT_TEMPLATE
        .find("\n## Outcome\n")
        .expect("an outcome heading")];
    assert!(opening.starts_with("# "), "no title line: {opening}");
    assert!(opening.contains("problem"), "no problem stated: {opening}");

    // The scope section asks for numbered slices carrying their
    // dependencies, which is what makes a brief cuttable into tickets.
    assert!(DEFAULT_TEMPLATE.contains("depends_on"));
}

#[test]
fn success_criteria_are_asked_for_in_bolded_groups() {
    assert!(DEFAULT_TEMPLATE.contains("bolded groups"));
    assert!(DEFAULT_TEMPLATE.contains("`**One part of the change**`"));
}

#[test]
fn a_repository_that_has_written_no_template_gets_the_built_in_one() {
    let root = a_root();

    assert_eq!(
        brief_template(root.path()).expect("the built-in template"),
        DEFAULT_TEMPLATE,
    );
}

#[test]
fn a_warlock_directory_with_no_template_in_it_is_still_absent() {
    // The manifest's directory exists in every repository warlock has ever
    // pacted anything in; only the file is missing.
    let root = a_root();
    fs::create_dir_all(root.path().join(".warlock")).expect("a `.warlock` directory");

    assert_eq!(
        brief_template(root.path()).expect("the built-in template"),
        DEFAULT_TEMPLATE,
    );
}

#[test]
fn a_template_the_repository_wrote_is_used_exactly_as_it_stands() {
    let root = a_root();
    // Deliberately nothing like the default, and deliberately untidy:
    // nothing here trims, wraps or validates.
    let written = "  # our shape\n\nsay the thing.\n\n\n";
    write_template(root.path(), written);

    assert_eq!(
        brief_template(root.path()).expect("the written template"),
        written,
    );
}

#[test]
fn an_empty_template_is_a_template_that_says_nothing() {
    let root = a_root();
    write_template(root.path(), "");

    assert_eq!(brief_template(root.path()).expect("an empty template"), "");
}

#[test]
fn a_second_read_sees_what_the_file_now_says() {
    // The whole of "read fresh every time": no cache, no copy taken on the
    // first call, so editing the file between two briefs changes the shape.
    let root = a_root();
    write_template(root.path(), "first");

    assert_eq!(brief_template(root.path()).expect("the first"), "first");

    write_template(root.path(), "second");

    assert_eq!(brief_template(root.path()).expect("the second"), "second");
}

#[test]
fn a_template_that_cannot_be_read_names_the_file_and_the_reason() {
    let error = Error {
        path: PathBuf::from("/repo/.warlock/brief-template.md"),
        source: std::io::Error::other("permission denied"),
    };

    let message = error.to_string();

    assert_eq!(
        message,
        "could not read `/repo/.warlock/brief-template.md`: permission denied",
    );
    assert!(!message.contains('\n'), "wrapped: {message}");
}

#[test]
fn absent_and_unreadable_are_different_answers() {
    // Bytes that are not UTF-8 are a file that exists and cannot be had:
    // the read that decodes is the read that fails. Portable, unlike a
    // permission bit, and the same case as far as this is concerned.
    let (absent, unreadable) = (a_root(), a_root());
    let path = write_template(unreadable.path(), "");
    fs::write(&path, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");

    assert_eq!(
        brief_template(absent.path()).expect("the built-in template"),
        DEFAULT_TEMPLATE,
    );

    let error = brief_template(unreadable.path()).expect_err("a refusal");
    let message = error.to_string();

    assert!(
        message.starts_with(&format!("could not read `{}`: ", path.display())),
        "did not name the file: {message}",
    );
    assert!(!message.contains('\n'), "wrapped: {message}");
    assert_ne!(error.path.as_path(), absent.path());
}

#[test]
fn a_directory_where_the_template_should_be_is_unreadable_and_not_absent() {
    let root = a_root();
    let path = template_path(root.path());
    fs::create_dir_all(&path).expect("a directory in the template's place");

    let error = brief_template(root.path()).expect_err("a refusal");

    assert_eq!(error.path, path);
    assert!(!error.to_string().contains('\n'));
}
