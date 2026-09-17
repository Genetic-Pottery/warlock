//! Absent, empty and unreadable are three different answers. Absent is the
//! built-in `DEFAULT_TEMPLATE`; empty is an empty template, used as it stands,
//! because a file somebody deliberately emptied says the model is to be given
//! no shape; unreadable is an `Error` naming the file and never the default,
//! because quietly substituting warlock's own shape would put the wrong
//! document at the end of twenty turns of conversation. That last is
//! `ignores.rs`'s "never degraded to no rules" said a third time.
//!
//! Read on every call, with no cache, no watcher and no copy taken at startup,
//! so a template edited between one `/brief` and the next is one that took
//! effect. There is no setting that points at the file or turns it on: it is a
//! markdown file somebody writes with `e` and commits.

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use warlock_engine::manifest_path;

// Only the file name: where `.warlock/` is, is the engine's fact rather than
// this crate's, and `template_path` takes it from the one path the engine
// already builds instead of spelling the directory a second time here.
const TEMPLATE_FILE: &str = "brief-template.md";

/// The shape this project's own twelve briefs in `docs/` converged on, and the
/// only place it is written down. The order is the argument: a document that
/// says what is wrong before it says what to build, and what it will not do
/// before it says how the work is cut, is one somebody can disagree with in the
/// right place.
///
/// Written as instructions rather than as a filled-in example, because an
/// example gets copied — a model handed one produces a brief about the example's
/// subject in the example's words, and the sections stop being questions the
/// conversation has to answer.
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

// Built from `manifest_path` rather than by joining `.warlock` here, because the
// name of that directory is the engine's to spell and a second copy of the
// string in this crate is a second place to change it. `with_file_name` rather
// than a parent and a join: the manifest path always has a file name, so there
// is no absent case to invent an answer for.
fn template_path(root: &Path) -> PathBuf {
    manifest_path(root).with_file_name(TEMPLATE_FILE)
}

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
/// [`Error`] if the file is there and cannot be read or decoded. A template that
/// exists is never quietly replaced by the built-in default — the caller says so
/// and asks for nothing.
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

// A struct rather than an enum, because there is exactly one way this fails and
// no second case to grow into: absent is not an error and the contents are never
// parsed, so there is no syntax and no wrong shape to report. Non-UTF-8 arrives
// here too, since the read that decodes is the read that fails.
#[derive(Debug)]
pub struct Error {
    pub path: PathBuf,
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

/// The one place in warlock the brief shape is *checked*, and it is here because
/// a model that has been arguing about a change for twenty turns writes the
/// document it has been thinking about and quietly drops a section it stopped
/// thinking about — a brief missing its `## Scope` reads perfectly well right up
/// until somebody goes looking for the slices, days later.
///
/// A section is a `## ` line of the template and only that level: the `# ` line
/// is an instruction about the title rather than a heading to reproduce, and a
/// `### ` line is inside a section rather than one of them, which is what the
/// numbered slices under `## Scope` are.
///
/// Matching is generous in both directions a strict reading would refuse — a
/// `### Outcome` is not a dropped outcome, and neither is an `## OUTCOME` —
/// because every false refusal throws away a document that took twenty turns to
/// arrive at, and neither is the failure this guards. Order is not checked for
/// the same reason: sections in an odd order is a document a reader can see is
/// odd and fix by asking, while a section that is not there is the one they
/// cannot see.
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

// `### ` does not match `"## "` at all, since its third byte is a `#` where the
// prefix wants a space. A heading with nothing after it is dropped rather than
// becoming a section no document could ever have.
fn sections_of(template: &str) -> Vec<&str> {
    template
        .lines()
        .filter_map(|line| line.trim().strip_prefix("## "))
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect()
}

// Asks the line and not the text: a document merely *mentioning* `## Scope` in
// its prose is not a document with a scope section. The level is thrown away
// before the comparison, so `#`, `##` and `###` all count — the generosity
// argued at `missing_sections`.
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
#[path = "tests/template.rs"]
mod tests;
