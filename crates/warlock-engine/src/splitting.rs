use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::document::{Defect, turned_down};

// The splitting road asks again exactly as often as the document and drafting
// roads do, and a third bound would be a number to keep in step with this one
// for no gain.
pub use crate::document::ATTEMPTS;

// One to eight. The floor is what makes a split a split — a ticket nobody could
// cut into even one sub-task is a ticket nothing can work — and the ceiling is
// there because each sub-task costs a whole session: a split into thirty is a
// ticket that should have been several tickets, and the run would spend a day
// finding that out.
pub const SUBTASKS_PER_TICKET: usize = 8;

pub const GOAL_MINIMUM: usize = 12;

pub const GOAL_CHARS: usize = 160;

// A dependency is a position in this ticket's own sub-tasks, so a list longer
// than the array can only be repeating itself or pointing outside the ticket —
// and a position outside the ticket is dropped rather than resolved.
pub const DEPENDS_ON_PER_SUBTASK: usize = SUBTASKS_PER_TICKET;

pub const DONE_PER_SUBTASK: usize = 8;

pub const DONE_CHARS: usize = 200;

pub const FILES_PER_SUBTASK: usize = 12;

pub const FILE_CHARS: usize = 200;

pub const TEST_PLAN_CHARS: usize = 600;

pub const NOTES_CHARS: usize = 1200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    #[serde(default)]
    pub subtasks: Vec<Subtask>,
}

/// One sub-task of a split. Only [`Self::goal`] is required: a sub-task with
/// nothing else answered is a thin brief, which is a fact about the split worth
/// recording, where a refusal over an absent `notes` would cost the whole
/// session.
///
/// `depends_on` holds 1-based positions in the split's own array, because the
/// numbering the manifest is read in — `<TICKET>.01`, `.02` — counts from one,
/// and two numbering schemes for the same ordering is how a dependency ends up
/// off by one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stated")]
pub struct Subtask {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub depends_on: Vec<usize>,
    #[serde(default)]
    pub definition_of_done: Vec<String>,
    #[serde(default)]
    pub likely_files: Vec<String>,
    #[serde(default)]
    pub test_plan: String,
    #[serde(default)]
    pub notes: String,
}

// Three slips a model makes here, and reading any of them strictly would answer
// it with `document::Defect::NotJson` — the one defect with nothing to repair
// from, which throws a whole read-only session's work away over one key. Read
// leniently and each becomes a slot the repair road already handles: a sub-task
// answered as a bare goal string, a list answered as one line, and a field the
// model had nothing to put in answered as `null` rather than left out.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stated {
    Goal(String),
    Subtask {
        #[serde(default)]
        goal: Option<String>,
        #[serde(default)]
        depends_on: Option<Vec<usize>>,
        #[serde(default)]
        definition_of_done: Listed,
        #[serde(default)]
        likely_files: Listed,
        #[serde(default)]
        test_plan: Option<String>,
        #[serde(default)]
        notes: Option<String>,
    },
}

// `Absent` first: an untagged read tries the variants in order, and `()` is what
// matches a `null`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Listed {
    Absent(()),
    One(String),
    Many(Vec<String>),
}

impl Default for Listed {
    fn default() -> Self {
        Self::Absent(())
    }
}

impl From<Listed> for Vec<String> {
    fn from(listed: Listed) -> Self {
        match listed {
            Listed::Absent(()) => Self::new(),
            Listed::One(line) => vec![line],
            Listed::Many(lines) => lines,
        }
    }
}

impl From<Stated> for Subtask {
    fn from(stated: Stated) -> Self {
        match stated {
            Stated::Goal(goal) => Self {
                goal,
                ..Self::default()
            },
            Stated::Subtask {
                goal,
                depends_on,
                definition_of_done,
                likely_files,
                test_plan,
                notes,
            } => Self {
                goal: goal.unwrap_or_default(),
                depends_on: depends_on.unwrap_or_default(),
                definition_of_done: definition_of_done.into(),
                likely_files: likely_files.into(),
                test_plan: test_plan.unwrap_or_default(),
                notes: notes.unwrap_or_default(),
            },
        }
    }
}

impl Fill {
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a fill is plain strings and numbers")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    Filled(Fill),
    Defective { fill: Fill, defects: Vec<Defect> },
    Unparsed(Defect),
}

/// Read a splitting answer, in the layout [`crate::drafting::accept`] uses: the
/// object lifted out of whatever prose surrounds it, then [`check`].
///
/// ```
/// use warlock_engine::splitting::{Accepted, accept, stub_answer};
///
/// let accepted = accept(&stub_answer("Split a pulled ticket into sub-tasks"));
/// assert!(matches!(accepted, Accepted::Filled(_)));
///
/// assert!(matches!(accept("I could not split this."), Accepted::Unparsed(_)));
/// ```
#[must_use]
pub fn accept(answer: &str) -> Accepted {
    let fill = match parse(answer) {
        Ok(fill) => fill,
        Err(defect) => return Accepted::Unparsed(defect),
    };
    let defects = check(&fill);
    if defects.is_empty() {
        Accepted::Filled(fill)
    } else {
        Accepted::Defective { fill, defects }
    }
}

fn parse(answer: &str) -> Result<Fill, Defect> {
    let object = match (answer.find('{'), answer.rfind('}')) {
        (Some(start), Some(end)) if start <= end => &answer[start..=end],
        _ => {
            return Err(Defect::NotJson {
                detail: "no object found in the answer".to_owned(),
            });
        }
    };
    serde_json::from_str(object).map_err(|error| Defect::NotJson {
        detail: error.to_string(),
    })
}

/// Every slot of a filled splitting answer, in [`Defect`]'s own vocabulary, with
/// `field` spelt as the path to the slot: `subtasks`, `subtasks[2].goal`,
/// `subtasks[2].depends_on`, `subtasks[2].definition_of_done[1]`.
///
/// ```
/// use warlock_engine::document::Defect;
/// use warlock_engine::splitting::{Fill, check};
///
/// assert_eq!(
///     check(&Fill::default()),
///     vec![Defect::Missing { field: "subtasks".to_owned() }],
/// );
/// ```
#[must_use]
pub fn check(fill: &Fill) -> Vec<Defect> {
    let mut defects = Vec::new();

    // `Missing` rather than `Empty` for a ticket nobody split, because
    // `#[serde(default)]` reads an absent `subtasks` key and an empty array as
    // the same value and there is no way back to which was written. The repair
    // has to build a sub-task either way, and `Missing` is what a slot with
    // nothing in it to work from is called on the document road.
    if fill.subtasks.is_empty() {
        defects.push(Defect::Missing {
            field: "subtasks".to_owned(),
        });
    }
    if fill.subtasks.len() > SUBTASKS_PER_TICKET {
        defects.push(Defect::TooMany {
            field: "subtasks".to_owned(),
            count: fill.subtasks.len(),
            cap: SUBTASKS_PER_TICKET,
        });
    }

    for (index, subtask) in fill.subtasks.iter().enumerate() {
        line(
            &format!("subtasks[{index}].goal"),
            &subtask.goal,
            GOAL_MINIMUM,
            GOAL_CHARS,
            &mut defects,
        );
        depends_on(
            &format!("subtasks[{index}].depends_on"),
            &subtask.depends_on,
            &mut defects,
        );
        lines(
            &format!("subtasks[{index}].definition_of_done"),
            &subtask.definition_of_done,
            DONE_PER_SUBTASK,
            DONE_CHARS,
            &mut defects,
        );
        lines(
            &format!("subtasks[{index}].likely_files"),
            &subtask.likely_files,
            FILES_PER_SUBTASK,
            FILE_CHARS,
            &mut defects,
        );
        prose(
            &format!("subtasks[{index}].test_plan"),
            &subtask.test_plan,
            TEST_PLAN_CHARS,
            &mut defects,
        );
        prose(
            &format!("subtasks[{index}].notes"),
            &subtask.notes,
            NOTES_CHARS,
            &mut defects,
        );
    }

    defects
}

fn line(field: &str, value: &str, minimum: usize, cap: usize, defects: &mut Vec<Defect>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        defects.push(Defect::Empty {
            field: field.to_owned(),
        });
        return;
    }
    if trimmed.contains(['\n', '\r']) {
        defects.push(Defect::Multiline {
            field: field.to_owned(),
        });
    }
    let chars = trimmed.chars().count();
    if chars < minimum {
        defects.push(Defect::TooShort {
            field: field.to_owned(),
            chars,
            minimum,
        });
    }
    if chars > cap {
        defects.push(Defect::TooLong {
            field: field.to_owned(),
            chars,
            cap,
        });
    }
}

// The length of the list, and then each entry as its own line with no floor
// under it: a done entry or a path is held to being written at all and to one
// line, but `src/lib.rs` is a perfectly good `likely_files` entry and a floor
// would make it defective. An empty list is no defect either — only the goal is
// required, and a sub-task the pass had nothing more to say about is a thin
// brief rather than a broken answer.
fn lines(field: &str, given: &[String], entries: usize, cap: usize, defects: &mut Vec<Defect>) {
    if given.len() > entries {
        defects.push(Defect::TooMany {
            field: field.to_owned(),
            count: given.len(),
            cap: entries,
        });
    }
    for (index, entry) in given.iter().enumerate() {
        line(&format!("{field}[{index}]"), entry, 0, cap, defects);
    }
}

// A test plan and a notes block run to lines and are both optional, so they are
// held to a cap and to nothing else: blank is how a pass says there was nothing
// to add, and reporting that as `Empty` would send the repair road off to invent
// a test plan out of a ticket title.
fn prose(field: &str, value: &str, cap: usize, defects: &mut Vec<Defect>) {
    let chars = value.trim().chars().count();
    if chars > cap {
        defects.push(Defect::TooLong {
            field: field.to_owned(),
            chars,
            cap,
        });
    }
}

// Only the length. A position outside this ticket's own sub-tasks — zero, past
// the end of the array, or the sub-task carrying it — goes unreported here, and
// the repair drops it: there is no defect in this vocabulary that says it
// without inventing one, `UnknownTarget` being the document road's word for a
// claim its evidence cannot vouch for, and this contract has no evidence witness
// at all. Re-asking would buy nothing either, since dropping the position is the
// whole of the repair. So `check` over a mended fill is empty, because nothing
// here reports what the mend dropped.
fn depends_on(field: &str, given: &[usize], defects: &mut Vec<Defect>) {
    if given.len() > DEPENDS_ON_PER_SUBTASK {
        defects.push(Defect::TooMany {
            field: field.to_owned(),
            count: given.len(),
            cap: DEPENDS_ON_PER_SUBTASK,
        });
    }
}

// The caps are written into the prose rather than asked about, and the numbers
// themselves are arbitrary: what is permanent is that there is a ceiling at all.
// A pass told the number answers inside it; a pass asked to be brief answers at
// whatever length it likes and is then repaired, which costs an attempt and
// loses whatever it wrote past the cut. The test below reads the constants, so
// moving one and leaving the prose behind fails rather than silently instructing
// the model to break the cap it is checked against.
pub const SPLIT_PROMPT: &str = "\
Fill in the JSON object at the end of these instructions, cutting the one \
ticket described below it into sub-tasks, and output the filled object and \
nothing else.

Each sub-task is picked up by a fresh session that has read none of the others, \
holds no memory of this one, and has nobody to ask: it is handed its own \
sub-task and the ticket for context, and has to finish and check the work from \
that alone. Cut the ticket into as few sub-tasks as it honestly takes. \
Everything you write comes from the ticket and from what you read in the \
repository — do not plan work the ticket did not ask for.

\"subtasks\": one entry per sub-task, in the order the work would be done, \
between 1 and 8 entries. Each entry is {\"goal\": ..., \"depends_on\": [...], \
\"definition_of_done\": [...], \"likely_files\": [...], \"test_plan\": ..., \
\"notes\": ...}.

\"goal\": one line, between 12 and 160 characters, saying what this sub-task \
does in the words the ticket uses for it.

\"depends_on\": the sub-tasks that have to be finished before this one, given \
as positions in the array above, counting from 1, at most 8 positions per list. \
A sub-task refers only to the other sub-tasks of this ticket: never to itself, \
never to a position the array does not hold, and never to work outside the \
ticket. Where nothing is ordered, the list is empty.

\"definition_of_done\": what would show this sub-task is finished, one \
checkable statement per entry, each on one line, at most 8 entries of at most \
200 characters.

\"likely_files\": the paths in this repository the work is expected to touch, \
one path per entry, at most 12 entries of at most 200 characters.

\"test_plan\": the commands that would show the sub-task works, at most 600 \
characters.

\"notes\": what the session needs and the ticket does not say — a convention to \
follow, a file to read first, a decision already taken. At most 1200 \
characters.

Only \"goal\" is required. Leave a field empty where there is nothing to put in \
it rather than padding it out.

Write each sub-task in its own voice: no first person, and nothing about this \
request or about what you were or were not shown.";

/// The splitting prompt with the ticket and the shape to fill appended, in the
/// layout [`crate::document::synthesis_instructions`] uses.
///
/// The ticket arrives as its plain title and description, so nothing about
/// where it came from or where its sub-tasks are going reaches the engine.
///
/// ```
/// use warlock_engine::splitting::{SPLIT_PROMPT, split_instructions};
///
/// let text = split_instructions("Split a pulled ticket", "One session, no writes.", &[]);
/// assert!(text.starts_with(SPLIT_PROMPT));
/// assert!(text.contains("One session, no writes."));
/// ```
#[must_use]
pub fn split_instructions(title: &str, description: &str, rejected: &[Defect]) -> String {
    let mut text = SPLIT_PROMPT.to_owned();
    turned_down(&mut text, rejected);
    let _ = write!(
        text,
        "\n\nThe ticket to cut into sub-tasks is `{}`, and it says:\n\n{}",
        title.trim(),
        description.trim(),
    );
    let shape = serde_json::json!({
        "subtasks": [{
            "goal": "",
            "depends_on": [],
            "definition_of_done": [],
            "likely_files": [],
            "test_plan": "",
            "notes": "",
        }],
    });
    let _ = write!(
        text,
        "\n\nReturn an object of exactly this shape, carrying one entry per sub-task with every \
         empty string filled in, as JSON, with no code fence and nothing before or after \
         it:\n\n{shape}",
    );
    text
}

/// The answer a test double hands back for a splitting pass, as
/// [`crate::drafting::stub_answer`] does for a drafting pass.
///
/// The ticket arrives as plain text and nothing else, so the stub is two
/// sub-tasks built out of that text, the second depending on the first, both
/// inside every cap — so a double's answer is accepted rather than repaired.
#[must_use]
pub fn stub_answer(ticket: &str) -> String {
    const DONE: &str = "A stand-in statement of what would show this sub-task finished, written by \
                        a test double that read no repository.";
    const TESTS: &str = "No commands: a test double planned this sub-task and ran nothing.";
    const NOTES: &str = "A stand-in note. It says what the ticket said and nothing more.";
    let named = flattened(ticket);
    let named = if named.is_empty() {
        "an unnamed ticket"
    } else {
        &named
    };
    Fill {
        subtasks: vec![
            Subtask {
                goal: fit(&format!("Stand in for {named}"), "(stand-in)"),
                depends_on: Vec::new(),
                definition_of_done: vec![DONE.to_owned()],
                likely_files: vec!["src/lib.rs".to_owned()],
                test_plan: TESTS.to_owned(),
                notes: NOTES.to_owned(),
            },
            Subtask {
                goal: fit(&format!("Follow on from {named}"), "(stand-in)"),
                depends_on: vec![1],
                definition_of_done: vec![DONE.to_owned()],
                likely_files: vec!["src/lib.rs".to_owned()],
                test_plan: TESTS.to_owned(),
                notes: NOTES.to_owned(),
            },
        ],
    }
    .to_json()
}

// The shape a goal has to hold: one line, at least `GOAL_MINIMUM` characters and
// at most `GOAL_CHARS`, counted as characters and cut on a character boundary so
// a multibyte ticket title cannot split. A value out of here is never defective.
fn fit(line: &str, pad: &str) -> String {
    let mut line = flattened(line);
    // A non-empty pad adds at least one character a turn, so this ends. In
    // practice it never runs: the shortest line built here clears the floor on
    // its own.
    while line.chars().count() < GOAL_MINIMUM && !pad.is_empty() {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(pad);
    }
    let cut: String = line.chars().take(GOAL_CHARS).collect();
    cut.trim_end().to_owned()
}

fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/splitting.rs"]
mod tests;
