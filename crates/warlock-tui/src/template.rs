//! The shape a brief takes: warlock's own skeleton, and the repository's if it
//! has written one down.
//!
//! One function, [`brief_template`], and one fact behind it — the shape a brief
//! mode conversation is aimed at is either the built-in [`DEFAULT_TEMPLATE`] or
//! whatever `<root>/.warlock/brief-template.md` says instead. Nothing has to be
//! configured to get a shape, and nothing has to be configured to change one:
//! the override is a markdown file somebody writes with `e` like any other file
//! in the repository, committed with it, and there is no setting anywhere that
//! points at it or turns it on.
//!
//! ## The default is this project's own skeleton
//!
//! Twelve briefs in `docs/` converged on the same six moves — the problem,
//! `## Outcome`, `## Success criteria`, `## Constraints`, `## Out of scope`,
//! `## Scope` — so that is what is compiled in, written as the instructions for
//! filling each section rather than as an example of one filled in. It is a
//! shape to aim at, and it is the only place in warlock where that shape is
//! written down.
//!
//! ## Read every time, cached never
//!
//! This reads the file on every call, and the accessor is the whole of the
//! mechanism: no cache, no watcher, no copy taken at startup. A template edited
//! between one `/brief` and the next is a template that took effect, which is
//! what makes editing it worth doing while warlock is running.
//!
//! ## Absent, empty and unreadable are three different answers
//!
//! * **Absent** is the built-in default. A repository that has said nothing
//!   about the shape gets warlock's.
//! * **Empty** is an empty template, used as it stands. A file somebody
//!   deliberately emptied is a statement that the model is to be given no
//!   shape, and nothing here validates or lints what a template *says* — a
//!   template full of nonsense is the same kind of user's business. Its `## `
//!   lines are read back out of it by [`missing_sections`], which is a
//!   different thing from judging them: whatever a template asks for is what a
//!   document is held to, and a template asking for nothing holds a document to
//!   nothing.
//! * **Unreadable** — permissions, a directory in the file's place, bytes that
//!   are not UTF-8 — is an [`Error`] naming the file, and never the default. A
//!   file that exists is a file somebody meant, and quietly using warlock's own
//!   shape in its place would put the wrong document at the end of twenty turns
//!   of conversation. This is `ignores.rs`'s "never degraded to no rules" and
//!   `sigils.rs`'s refusal to read an unreadable config as an unrestricted one,
//!   said a third time about the same kind of file.

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use warlock_engine::manifest_path;

/// The file a repository states its own brief shape in, inside `.warlock/`.
///
/// Only the file name: where `.warlock/` is, is the engine's fact rather than
/// this crate's, and [`template_path`] takes it from the one path the engine
/// already builds instead of spelling the directory a second time here.
const TEMPLATE_FILE: &str = "brief-template.md";

/// The shape a brief takes when the repository has not said otherwise.
///
/// This project's own skeleton, because twelve briefs in `docs/` arrived at it:
/// a `# ` title, prose stating the problem, and then `## Outcome`,
/// `## Success criteria`, `## Constraints`, `## Out of scope` and `## Scope` in
/// that order. The order is the argument — a document that says what is wrong
/// before it says what to build, and what it will not do before it says how the
/// work is cut, is a document somebody can disagree with in the right place.
///
/// Written as instructions rather than as a filled-in example. An example gets
/// copied: a model handed one produces a brief about the example's subject in
/// the example's words, and the sections stop being questions the conversation
/// has to answer. So each heading here is followed by what belongs under it and
/// nothing that could be mistaken for content.
///
/// Public because a caller that has no repository root in its hand still has to
/// be able to state a shape — [`brief_template`] is how a repository's own is
/// reached, and this is the answer that function gives when there is none.
pub const DEFAULT_TEMPLATE: &str = "# A title line naming the change\n\n\
Open with the problem, in prose and before any heading: what is wrong now, in \
this repository, naming the files and the behaviour. Say what it costs to \
leave it alone. Do not describe the document itself.\n\n\
## Outcome\n\n\
What somebody sees once the change is made, written as something a reader can \
watch happen — a session, a screen, a command and what it prints. Not a list \
of the work; the work is further down.\n\n\
## Success criteria\n\n\
Facts that can be checked as done or not done, gathered into bolded groups: \
`**One part of the change**` on a line of its own, then its bullets. One group \
per part. A criterion nobody could mark done is a wish, and belongs in the \
outcome or nowhere.\n\n\
## Constraints\n\n\
What must not change and what the work may not reach for: dependencies, \
architecture, the things earlier decisions already settled. Each said as a rule \
the work is made under rather than as a preference.\n\n\
## Out of scope\n\n\
What is deliberately not being done, each one named with the reasoning for the \
refusal. Something refused with a reason is a decision a reader can argue with; \
something merely left out is an oversight nobody can tell from one.\n\n\
## Scope\n\n\
The work as numbered slices, each `### N. What the slice does` followed by a \
line reading `depends_on: [<the numbers it needs first>]`, then what that slice \
decides and why. A slice is a piece of work that lands on its own; the \
dependencies say what order they can land in.";

/// Where the brief template would be under `root`:
/// `<root>/.warlock/brief-template.md`.
///
/// Built from [`manifest_path`] rather than by joining `.warlock` here, because
/// the name of that directory is the engine's to spell — it is the engine that
/// creates it, and a second copy of the string in this crate is a second place
/// to change it. `with_file_name` rather than a parent and a join: the manifest
/// path always has a file name, so there is no absent case to invent an answer
/// for.
fn template_path(root: &Path) -> PathBuf {
    manifest_path(root).with_file_name(TEMPLATE_FILE)
}

/// The shape a brief for the repository at `root` has to take: the file at
/// `<root>/.warlock/brief-template.md`, or the built-in default when there is
/// none.
///
/// Read from disk on every call, so a template edited while warlock is running
/// takes effect on the next brief without a restart. The file's contents come
/// back verbatim — not trimmed, not parsed, not checked for headings — and an
/// empty file is an empty template, which is a repository saying the model is
/// to be given no shape.
///
/// ```no_run
/// use warlock_tui::brief_template;
///
/// // Nothing configured, nothing written: the built-in shape.
/// let shape = brief_template("/repo")?;
///
/// assert!(shape.contains("## Success criteria"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`Error`] if the file is there and cannot be read or decoded: no permission,
/// a directory in its place, or bytes that are not UTF-8. A template that
/// exists is never quietly replaced by the built-in default — the caller says
/// so and asks for nothing.
pub fn brief_template(root: impl AsRef<Path>) -> Result<String, Error> {
    let path = template_path(root.as_ref());
    match fs::read_to_string(&path) {
        Ok(template) => Ok(template),
        // The only case that is not a failure: no file is the repository having
        // said nothing about the shape, which warlock has an answer for.
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(DEFAULT_TEMPLATE.to_owned()),
        Err(source) => Err(Error { path, source }),
    }
}

/// A brief template that is there and cannot be had: the file, and what the
/// filesystem said about it.
///
/// A struct rather than an enum, because there is exactly one way this fails
/// and there is no second case to grow into: absent is not an error, and the
/// contents are never parsed, so there is no missing file, no syntax and no
/// wrong shape to report. Non-UTF-8 arrives here too — the read that decodes is
/// the read that fails — which is why the reason is worth printing rather than
/// summarising.
///
/// One line, like everything the binary prints: the file that was found and, in
/// the filesystem's own words, why it could not be read.
#[derive(Debug)]
pub struct Error {
    /// The template file that was found and could not be read.
    pub path: PathBuf,
    /// What the read said — permission denied, is a directory, not UTF-8.
    pub source: io::Error,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "could not read `{}`: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Which of `template`'s sections `document` has not got, in the order the
/// template asks for them.
///
/// The shape is a shape to aim at everywhere else in warlock — written into the
/// instruction, argued over for twenty turns, and then not looked at again. This
/// is the one place it is *checked*, and it exists because a model that has been
/// arguing about a change for twenty turns writes the document it has been
/// thinking about and quietly drops a section it stopped thinking about. That
/// failure is silent: a brief missing its `## Scope` reads perfectly well right
/// up until somebody goes looking for the slices, which is days later and in
/// another conversation.
///
/// # What counts as a section, and what counts as having one
///
/// A section is a `## ` line of the template. Only that level: the `# ` line is
/// an *instruction* about the title rather than a heading to reproduce — a
/// document is supposed to name its own change there — and a `### ` line is
/// inside a section rather than one of them, which is what the numbered slices
/// under `## Scope` are.
///
/// A document has a section when some heading line of it, at any level, carries
/// that text. Deliberately generous in both directions that a strict reading
/// would refuse, because every false refusal here throws away a document that
/// took twenty turns to arrive at, and neither of them is the failure this
/// guards: a `### Outcome` is not a dropped outcome, and neither is an
/// `## OUTCOME`. What is not generous is presence itself — a section that is not
/// there in any spelling is a section the document has not got.
///
/// Order is not checked, though the template's own docs argue the order is the
/// argument. Sections in an odd order is a document a reader can see is odd and
/// fix by asking; a section that is not there is the one they cannot see.
/// Folding is ASCII-only, which is the whole of what the built-in shape and any
/// heading a model writes in English needs.
///
/// ```
/// use warlock_tui::{DEFAULT_TEMPLATE, missing_sections};
///
/// let dropped = "# A change\n\nThe problem.\n\n## Outcome\n\n## Success criteria\n\n\
///                ## Constraints\n\n## Out of scope\n";
///
/// assert_eq!(missing_sections(DEFAULT_TEMPLATE, dropped), ["Scope"]);
/// // A template that asks for nothing holds a document to nothing.
/// assert!(missing_sections("", dropped).is_empty());
/// ```
#[must_use]
pub fn missing_sections<'a>(template: &'a str, document: &str) -> Vec<&'a str> {
    sections_of(template)
        .into_iter()
        .filter(|section| !carries(document, section))
        .collect()
}

/// The `## ` lines of `template`, trimmed, in the order they appear.
///
/// Leading whitespace is trimmed before the prefix is looked for, so an indented
/// heading counts; `### ` does not match `"## "` at all, since its third byte is
/// a `#` where the prefix wants a space. A heading with nothing after it is
/// dropped rather than becoming a section no document could ever have.
fn sections_of(template: &str) -> Vec<&str> {
    template
        .lines()
        .filter_map(|line| line.trim().strip_prefix("## "))
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect()
}

/// Whether any heading line of `document` carries `section`.
///
/// The level is thrown away before the comparison — see
/// [`missing_sections`], where that generosity is argued — so `#`, `##` and
/// `###` all count, and a line that is not a heading at all never does. A
/// document merely *mentioning* `## Scope` in its prose is not a document with a
/// scope section, which is why this asks the line and not the text.
fn carries(document: &str, section: &str) -> bool {
    document.lines().any(|line| {
        let line = line.trim();
        line.starts_with('#')
            && line
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case(section)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::manifest_path;

    use std::fmt::Write as _;

    use super::{
        DEFAULT_TEMPLATE, Error, brief_template, missing_sections, sections_of, template_path,
    };

    /// A document carrying `sections` as `## ` headings, with the title and the
    /// problem prose every brief opens with.
    fn document_with(sections: &[&str]) -> String {
        let mut document = String::from("# A change\n\nWhat is wrong now.\n");
        for section in sections {
            let _ = write!(document, "\n{section}\n\nSomething under it.\n");
        }
        document
    }

    /// Every section the built-in shape asks for, spelled as its headings.
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
        assert!(
            missing_sections(shape, &document_with(&["## Background", "## Rollout"])).is_empty()
        );
    }

    /// A throwaway repository root. Nothing in this module reads or writes a
    /// path the developer has anything in.
    fn a_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// Put `text` at `<root>/.warlock/brief-template.md`, creating the
    /// directory the way anything writing under `.warlock/` has to.
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
}
