use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::document::{Defect, cut, fit, flattened, line, parse, turned_down};

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

pub type Accepted = crate::document::Accepted<Fill>;

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

// The text warlock writes into a slot the pass left unanswered, built out of
// the only two things this contract holds: the ticket's own title and its
// description. Nothing here says what a sub-task should do, because that is
// exactly what a pass which did not answer never said, and nothing here is
// phrased as if a model wrote it — a filled-in sub-task that reads like a split
// one is worse than an obviously empty one, because a fresh session picks it up
// and works from it. So every line opens by saying it was not split out. Do not
// dress these up.
mod fallback {
    use super::{GOAL_CHARS, GOAL_MINIMUM, NOTES_CHARS, cut, fit, flattened};

    pub(super) fn goal(title: &str, index: usize) -> String {
        fit(
            &format!("Unwritten sub-task {} of {}", index + 1, named(title)),
            "(no goal was split out)",
            GOAL_MINIMUM,
            GOAL_CHARS,
        )
    }

    pub(super) fn notes(title: &str, description: &str, index: usize) -> String {
        let description = description.trim();
        let mut text = format!(
            "No sub-task was split out of this ticket. This is sub-task {} of {}, and warlock \
             filled it in rather than leave the split empty.",
            index + 1,
            named(title),
        );
        if description.is_empty() {
            text.push_str(" The ticket says nothing further.");
        } else {
            text.push_str(" The ticket says:\n\n");
            text.push_str(description);
        }
        cut(&text, NOTES_CHARS)
    }

    fn named(title: &str) -> String {
        let title = flattened(title);
        if title.is_empty() {
            "an unnamed ticket".to_owned()
        } else {
            format!("the ticket `{title}`")
        }
    }
}

// The splitting road repairs a repair exactly as often as the drafting road
// does, and the worst chain here is the same two links: a goal cut back to its
// first line can land under `GOAL_MINIMUM` and then fall to warlock's own text,
// and a list entry cut to its cap can come back blank and then be dropped. A
// second bound would be a number to keep in step with that one for no gain.
pub use crate::drafting::MEND_PASSES;

// `field` is the slot in [`Defect`]'s own spelling — `subtasks`,
// `subtasks[2].goal`, `subtasks[2].definition_of_done[1]` — so a caller can line
// a mend up against the defect it answers without parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mend {
    pub field: String,
    pub done: Mended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mended {
    // The value spanned lines and keeps its first.
    FirstLine,
    // The value ran over its cap and was cut to it, counting characters.
    Cut { from: usize, to: usize },
    // The list ran over its cap and keeps its first entries.
    Shortened { from: usize, to: usize },
    // The entry was left blank, and a blank line in a list is nothing a session
    // could work from, so it is gone from the list.
    Blank,
    // Positions pointing outside this ticket's own sub-tasks, or at the
    // sub-task carrying them, and so gone.
    Dropped { count: usize },
    // The slot was never answered and fell to warlock's own line about the
    // ticket. See [`fallback`].
    Supplied,
}

impl fmt::Display for Mend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = &self.field;
        match self.done {
            Mended::FirstLine => {
                write!(f, "{field} ran to more than one line and keeps its first")
            }
            Mended::Cut { from, to } => {
                write!(f, "{field} was {from} characters and was cut to {to}")
            }
            Mended::Shortened { from, to } => {
                write!(
                    f,
                    "{field} had {from} entries and was cut to the first {to}"
                )
            }
            Mended::Blank => write!(f, "{field} was left blank and was dropped from its list"),
            Mended::Dropped { count } => write!(
                f,
                "{field} named {count} position(s) outside this ticket's sub-tasks and lost them"
            ),
            Mended::Supplied => write!(
                f,
                "{field} was not answered and was filled in from the ticket's own text"
            ),
        }
    }
}

/// The mechanical mend: the floor under an exhausted attempt loop. Every
/// [`Defect`] but `NotJson` has a repair here, and none of them reaches a model
/// — the evidence is the answer's own text and the ticket's title and
/// description. A fill that comes back from here is not defective: [`check`]
/// over it is empty, it holds between 1 and [`SUBTASKS_PER_TICKET`] sub-tasks,
/// and every position it still carries names another sub-task of this ticket.
#[must_use]
pub fn mend(fill: &Fill, title: &str, description: &str) -> (Fill, Vec<Mend>) {
    let (fill, mends, _) = mended(fill, title, description);
    (fill, mends)
}

// The same operation, saying how many passes it took. Private because the count
// is a fact about this function and not about the split; the test for the bound
// is the only caller that has any use for it.
fn mended(fill: &Fill, title: &str, description: &str) -> (Fill, Vec<Mend>, usize) {
    let mut fill = fill.clone();
    let mut mends = Vec::new();
    let mut passes = 0;
    for _ in 0..MEND_PASSES {
        let defects = check(&fill);
        if defects.is_empty() {
            break;
        }
        passes += 1;
        sweep(&mut fill, &defects, title, description, &mut mends);
    }
    prune(&mut fill, &mut mends);

    (fill, mends, passes)
}

// The one repair no defect asks for: `check` reports nothing about where a
// position points, so this runs whether or not the fill was defective, and it
// runs after the loop because the loop is what settles how long the array is.
// Zero, a position past the end of the array and a position naming the sub-task
// that carries it are all dropped rather than resolved: this ticket's sub-tasks
// are the only work this contract knows about, so there is nowhere else such a
// position could be looking, and a sub-task that waits on itself is an order no
// run could carry out.
fn prune(fill: &mut Fill, mends: &mut Vec<Mend>) {
    let held = fill.subtasks.len();
    for (index, subtask) in fill.subtasks.iter_mut().enumerate() {
        let before = subtask.depends_on.len();
        // Positions count from 1, so the sub-task carrying the list is at
        // `index + 1` and the last sub-task of the ticket is at `held`.
        subtask
            .depends_on
            .retain(|position| (1..=held).contains(position) && *position != index + 1);
        let dropped = before - subtask.depends_on.len();
        if dropped > 0 {
            mends.push(Mend {
                field: format!("subtasks[{index}].depends_on"),
                done: Mended::Dropped { count: dropped },
            });
        }
    }
}

// One pass of the fixpoint. The order inside it is what keeps the indices
// meaning what the defects say they mean: what the array cuts off is decided
// first, then what to fill and what to drop, then the values that survive are
// rewritten in place, and only then do the arrays themselves move — so a defect
// naming `subtasks[3]` is never applied to whatever slid into position 3.
fn sweep(
    fill: &mut Fill,
    defects: &[Defect],
    title: &str,
    description: &str,
    mends: &mut Vec<Mend>,
) {
    let mut plan = Plan::default();
    for defect in defects {
        plan.note_array(defect, mends);
    }
    for defect in defects {
        plan.note_cut(defect, mends);
    }
    for defect in defects {
        plan.note_fill(defect, mends);
    }
    for defect in defects {
        let (field, done) = match defect {
            Defect::Multiline { field } => (field, Mended::FirstLine),
            Defect::TooLong { field, chars, cap } => (
                field,
                Mended::Cut {
                    from: *chars,
                    to: *cap,
                },
            ),
            _ => continue,
        };
        // A slot already being filled in whole, or hanging off the end of an
        // array this pass cuts back, is not worth rewriting first: the record
        // would name work the same pass undoes.
        if plan.covers(field) {
            continue;
        }
        let Some(value) = target(fill, field) else {
            continue;
        };
        match done {
            // The first line of the value as the check reads it: `line`
            // measures the trimmed value, so a goal that opens with a blank
            // line keeps the first line of what was actually written.
            Mended::FirstLine => {
                *value = value.trim().lines().next().unwrap_or_default().to_owned();
            }
            // Characters, not bytes, and so on a character boundary. No trim:
            // the cut lands where it lands.
            Mended::Cut { to, .. } => *value = value.chars().take(to).collect(),
            _ => {}
        }
        mends.push(Mend {
            field: field.clone(),
            done,
        });
    }
    plan.carry_out(fill, title, description);
}

#[derive(Debug, Default)]
struct Plan {
    subtasks: bool,
    cut_subtasks: bool,
    filled_goals: BTreeSet<usize>,
    cut_depends_on: BTreeSet<usize>,
    cut_done: BTreeSet<usize>,
    cut_files: BTreeSet<usize>,
    blank_done: BTreeSet<(usize, usize)>,
    blank_files: BTreeSet<(usize, usize)>,
}

impl Plan {
    // The array itself: cut back to the cap, or filled with one sub-task where
    // the pass split none out. Taken first and on its own, because what the cut
    // takes off the end decides which of the slots below are worth repairing at
    // all, and nothing here may depend on the order `check` reported them in.
    fn note_array(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let done = match defect {
            Defect::Missing { field } if slot(field) == Slot::Subtasks => {
                if std::mem::replace(&mut self.subtasks, true) {
                    return;
                }
                Mended::Supplied
            }
            Defect::TooMany { field, count, cap } if slot(field) == Slot::Subtasks => {
                if std::mem::replace(&mut self.cut_subtasks, true) {
                    return;
                }
                Mended::Shortened {
                    from: *count,
                    to: *cap,
                }
            }
            _ => return,
        };
        mends.push(Mend {
            field: "subtasks".to_owned(),
            done,
        });
    }

    fn note_cut(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let Defect::TooMany { field, count, cap } = defect else {
            return;
        };
        let recorded = match slot(field) {
            Slot::DependsOn(index) => !self.cut_off(index) && self.cut_depends_on.insert(index),
            Slot::Done(index, None) => !self.cut_off(index) && self.cut_done.insert(index),
            Slot::Files(index, None) => !self.cut_off(index) && self.cut_files.insert(index),
            Slot::Subtasks
            | Slot::Goal(_)
            | Slot::Done(_, Some(_))
            | Slot::Files(_, Some(_))
            | Slot::TestPlan(_)
            | Slot::Notes(_)
            | Slot::Unknown => false,
        };
        if recorded {
            mends.push(Mend {
                field: field.clone(),
                done: Mended::Shortened {
                    from: *count,
                    to: *cap,
                },
            });
        }
    }

    // What was never answered: a goal left empty or too short to say anything,
    // and an entry of a list left blank. Warlock says what the ticket says for
    // the first and drops the second — a blank line in a list is not a fact
    // anything could be built out of, and only the goal is required.
    fn note_fill(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let (Defect::Empty { field } | Defect::TooShort { field, .. }) = defect else {
            return;
        };
        let (done, recorded) = match slot(field) {
            Slot::Goal(index) => (
                Mended::Supplied,
                !self.cut_off(index) && self.filled_goals.insert(index),
            ),
            Slot::Done(index, Some(entry)) => (
                Mended::Blank,
                !self.dropped_done(index, entry) && self.blank_done.insert((index, entry)),
            ),
            Slot::Files(index, Some(entry)) => (
                Mended::Blank,
                !self.dropped_file(index, entry) && self.blank_files.insert((index, entry)),
            ),
            Slot::Subtasks
            | Slot::DependsOn(_)
            | Slot::Done(_, None)
            | Slot::Files(_, None)
            | Slot::TestPlan(_)
            | Slot::Notes(_)
            | Slot::Unknown => (Mended::Supplied, false),
        };
        if recorded {
            mends.push(Mend {
                field: field.clone(),
                done,
            });
        }
    }

    fn covers(&self, field: &str) -> bool {
        match slot(field) {
            Slot::Subtasks => self.subtasks,
            Slot::Goal(index) => self.cut_off(index) || self.filled_goals.contains(&index),
            Slot::Done(index, Some(entry)) => {
                self.dropped_done(index, entry) || self.blank_done.contains(&(index, entry))
            }
            Slot::Files(index, Some(entry)) => {
                self.dropped_file(index, entry) || self.blank_files.contains(&(index, entry))
            }
            Slot::DependsOn(index)
            | Slot::Done(index, None)
            | Slot::Files(index, None)
            | Slot::TestPlan(index)
            | Slot::Notes(index) => self.cut_off(index),
            Slot::Unknown => false,
        }
    }

    // Whether a sub-task is past the cap of an array this pass cuts back, and
    // so about to go anyway.
    fn cut_off(&self, index: usize) -> bool {
        self.cut_subtasks && index >= SUBTASKS_PER_TICKET
    }

    // The same question for one entry of a list this pass cuts back.
    fn dropped_done(&self, index: usize, entry: usize) -> bool {
        self.cut_off(index) || (self.cut_done.contains(&index) && entry >= DONE_PER_SUBTASK)
    }

    fn dropped_file(&self, index: usize, entry: usize) -> bool {
        self.cut_off(index) || (self.cut_files.contains(&index) && entry >= FILES_PER_SUBTASK)
    }

    fn carry_out(self, fill: &mut Fill, title: &str, description: &str) {
        if self.subtasks {
            let index = fill.subtasks.len();
            fill.subtasks.push(Subtask {
                goal: fallback::goal(title, index),
                notes: fallback::notes(title, description, index),
                ..Subtask::default()
            });
        }
        for index in &self.filled_goals {
            if let Some(subtask) = fill.subtasks.get_mut(*index) {
                subtask.goal = fallback::goal(title, *index);
            }
        }
        for index in &self.cut_depends_on {
            if let Some(subtask) = fill.subtasks.get_mut(*index) {
                subtask.depends_on.truncate(DEPENDS_ON_PER_SUBTASK);
            }
        }
        for index in &self.cut_done {
            if let Some(subtask) = fill.subtasks.get_mut(*index) {
                subtask.definition_of_done.truncate(DONE_PER_SUBTASK);
            }
        }
        for index in &self.cut_files {
            if let Some(subtask) = fill.subtasks.get_mut(*index) {
                subtask.likely_files.truncate(FILES_PER_SUBTASK);
            }
        }

        // The blank entries next, highest position first, so each position
        // still names the entry it was read against as the list shortens
        // underneath it. A blank past a cap this pass cut was never recorded,
        // and the length check is the belt on that brace.
        for (index, entry) in self.blank_done.iter().rev() {
            let Some(subtask) = fill.subtasks.get_mut(*index) else {
                continue;
            };
            if *entry < subtask.definition_of_done.len() {
                subtask.definition_of_done.remove(*entry);
            }
        }
        for (index, entry) in self.blank_files.iter().rev() {
            let Some(subtask) = fill.subtasks.get_mut(*index) else {
                continue;
            };
            if *entry < subtask.likely_files.len() {
                subtask.likely_files.remove(*entry);
            }
        }

        // Last, and off the end. Every index above was read against the
        // pre-pass array, and the array is cut rather than thinned from the
        // middle: nothing slides, so a surviving position still names the
        // sub-task it always named, and a position into the part that went is
        // out of range and `prune` drops it.
        if self.cut_subtasks {
            fill.subtasks.truncate(SUBTASKS_PER_TICKET);
        }
    }
}

// The slot a defect's `field` names, read back out of the spelling `check`
// wrote it in. The two lists are named either whole or one entry at a time —
// `subtasks[2].likely_files` and `subtasks[2].likely_files[1]` — and the entry
// is what the second member carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Subtasks,
    Goal(usize),
    DependsOn(usize),
    Done(usize, Option<usize>),
    Files(usize, Option<usize>),
    TestPlan(usize),
    Notes(usize),
    Unknown,
}

fn slot(field: &str) -> Slot {
    if field == "subtasks" {
        return Slot::Subtasks;
    }
    let Some(rest) = field.strip_prefix("subtasks[") else {
        return Slot::Unknown;
    };
    let Some((inside, tail)) = rest.split_once(']') else {
        return Slot::Unknown;
    };
    let Ok(index) = inside.parse::<usize>() else {
        return Slot::Unknown;
    };
    match tail {
        ".goal" => Slot::Goal(index),
        ".depends_on" => Slot::DependsOn(index),
        ".test_plan" => Slot::TestPlan(index),
        ".notes" => Slot::Notes(index),
        _ => listed(index, tail),
    }
}

fn listed(index: usize, tail: &str) -> Slot {
    let (name, entry) = match tail.split_once('[') {
        None => (tail, None),
        Some((name, rest)) => {
            let Some(inside) = rest.strip_suffix(']') else {
                return Slot::Unknown;
            };
            let Ok(entry) = inside.parse::<usize>() else {
                return Slot::Unknown;
            };
            (name, Some(entry))
        }
    };
    match name {
        ".definition_of_done" => Slot::Done(index, entry),
        ".likely_files" => Slot::Files(index, entry),
        _ => Slot::Unknown,
    }
}

// The value a rewrite writes over.
fn target<'f>(fill: &'f mut Fill, field: &str) -> Option<&'f mut String> {
    match slot(field) {
        Slot::Goal(index) => fill
            .subtasks
            .get_mut(index)
            .map(|subtask| &mut subtask.goal),
        Slot::Done(index, Some(entry)) => fill
            .subtasks
            .get_mut(index)
            .and_then(|subtask| subtask.definition_of_done.get_mut(entry)),
        Slot::Files(index, Some(entry)) => fill
            .subtasks
            .get_mut(index)
            .and_then(|subtask| subtask.likely_files.get_mut(entry)),
        Slot::TestPlan(index) => fill
            .subtasks
            .get_mut(index)
            .map(|subtask| &mut subtask.test_plan),
        Slot::Notes(index) => fill
            .subtasks
            .get_mut(index)
            .map(|subtask| &mut subtask.notes),
        Slot::Subtasks
        | Slot::DependsOn(_)
        | Slot::Done(_, None)
        | Slot::Files(_, None)
        | Slot::Unknown => None,
    }
}

/// One sub-task of a split after it has been ordered and named: the same
/// answer, with an identifier of its own and with `depends_on` spelt in those
/// identifiers rather than in positions.
///
/// The positions are gone on purpose. They meant places in the array the pass
/// wrote, the sort moves those places, and a record carrying both would be two
/// spellings of one ordering that drift apart the first time anything is
/// reordered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Numbered {
    pub id: String,
    pub goal: String,
    pub depends_on: Vec<String>,
    pub definition_of_done: Vec<String>,
    pub likely_files: Vec<String>,
    pub test_plan: String,
    pub notes: String,
}

/// One sub-task caught in a circle, named the way the pass named it: its
/// 1-based place in the array it answered with, and its goal. There is no
/// identifier here because nothing was numbered — the numbering is what the
/// circle stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caught {
    pub position: usize,
    pub goal: String,
}

/// The one split defect with no repair: sub-tasks that wait on one another, so
/// no order puts every dependency before its dependant.
///
/// It is not repairable because every repair would be a guess at which
/// dependency the pass did not mean, and a dropped edge is invisible in the
/// manifest afterwards — the run would go on in an order nobody chose. So this
/// is a halt, and [`fmt::Display`] writes the sentence that says why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    pub ticket: String,
    pub caught: Vec<Caught>,
}

impl fmt::Display for Cycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ticket = self.ticket.trim();
        let ticket = if ticket.is_empty() {
            "this ticket".to_owned()
        } else {
            format!("`{ticket}`")
        };
        let places: Vec<String> = self
            .caught
            .iter()
            .map(|caught| caught.position.to_string())
            .collect();
        // One sub-task on its own is a sub-task waiting on itself, which `mend`
        // drops and only an unmended fill can still carry. It is a circle all
        // the same, and saying "wait on one another" of one sub-task would read
        // as a sentence warlock got wrong rather than a split that is.
        let circle = if let [only] = places.as_slice() {
            format!("sub-task {only} waits on itself")
        } else {
            format!(
                "sub-tasks {} wait on one another, directly or by way of another sub-task",
                named(&places),
            )
        };
        write!(
            f,
            "The split of {ticket} could not be ordered: {circle}, so nothing in the split can \
             start. No dependency was dropped to break the circle.",
        )?;
        for caught in &self.caught {
            write!(f, " Sub-task {} is `{}`.", caught.position, caught.goal)?;
        }
        Ok(())
    }
}

// `2`, `2 and 3`, `2, 3 and 4`. Plain prose, because the sentence goes on a
// ticket for a person to read.
fn named(items: &[String]) -> String {
    match items {
        [] => "none".to_owned(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Order a mended split so every sub-task comes after everything it waits on,
/// then name each one `<TICKET>.01`, `<TICKET>.02` and so on, rewriting
/// `depends_on` into those names. A manifest read top to bottom is then a legal
/// order to work in.
///
/// Sub-tasks nothing separates keep the order the pass gave them: of the
/// sub-tasks whose dependencies are all placed, the earliest in the answer goes
/// next, so a split that arrived in a workable order is numbered in exactly
/// that order and two calls over one answer agree.
///
/// ```
/// use warlock_engine::splitting::{Fill, Subtask, number};
///
/// let fill = Fill {
///     subtasks: vec![
///         Subtask { goal: "Write the briefs".to_owned(), depends_on: vec![2], ..Subtask::default() },
///         Subtask { goal: "Split the ticket".to_owned(), ..Subtask::default() },
///     ],
/// };
///
/// let numbered = number(&fill, "WAR-138").expect("nothing here waits on itself");
///
/// assert_eq!(numbered[0].id, "WAR-138.01");
/// assert_eq!(numbered[0].goal, "Split the ticket");
/// assert_eq!(numbered[1].id, "WAR-138.02");
/// assert_eq!(numbered[1].depends_on, ["WAR-138.01"]);
/// ```
///
/// # Errors
///
/// [`Cycle`] when sub-tasks wait on one another, naming every sub-task in the
/// circle. That is the one split defect [`mend`] does not answer.
pub fn number(fill: &Fill, ticket: &str) -> Result<Vec<Numbered>, Cycle> {
    let subtasks = &fill.subtasks;
    let order = match ordered(subtasks) {
        Ok(order) => order,
        Err(tangled) => {
            return Err(Cycle {
                ticket: ticket.trim().to_owned(),
                caught: tangled
                    .into_iter()
                    .map(|index| Caught {
                        position: index + 1,
                        goal: subtasks[index].goal.clone(),
                    })
                    .collect(),
            });
        }
    };

    // Where each sub-task of the answer ended up, so a position can be read
    // straight through to the name of the sub-task it points at.
    let mut placed_at = vec![0; order.len()];
    for (at, index) in order.iter().enumerate() {
        placed_at[*index] = at;
    }
    let name = |at: usize| format!("{}.{:02}", ticket.trim(), at + 1);

    Ok(order
        .iter()
        .enumerate()
        .map(|(at, index)| {
            let subtask = &subtasks[*index];
            let mut depends_on: Vec<String> = Vec::new();
            for position in &subtask.depends_on {
                let Some(dependency) = resolved(*position, &placed_at) else {
                    continue;
                };
                let spelt = name(dependency);
                // A position the pass repeated is one dependency, and a name
                // written twice in a manifest reads as two.
                if !depends_on.contains(&spelt) {
                    depends_on.push(spelt);
                }
            }
            Numbered {
                id: name(at),
                goal: subtask.goal.clone(),
                depends_on,
                definition_of_done: subtask.definition_of_done.clone(),
                likely_files: subtask.likely_files.clone(),
                test_plan: subtask.test_plan.clone(),
                notes: subtask.notes.clone(),
            }
        })
        .collect())
}

// The sub-task a 1-based position names, as a place in the ordered answer.
// `mend`'s `prune` has already dropped every position that does not resolve —
// zero, past the end, and the sub-task carrying it — so this never misses on a
// mended fill, and the assertion says so. On an unmended one it ignores the
// position rather than panicking or re-implementing the drop: dropping a
// dependency is a repair, repairs are named in the output, and this function
// has nowhere to name one.
fn resolved(position: usize, placed_at: &[usize]) -> Option<usize> {
    let at = position
        .checked_sub(1)
        .and_then(|index| placed_at.get(index))
        .copied();
    debug_assert!(
        at.is_some(),
        "position {position} does not name a sub-task of this ticket: `mend` drops those, so \
         `number` was handed a fill that was never mended"
    );
    at
}

// The answer's sub-tasks in an order where every dependency comes first, as
// places in the answer, or the sub-tasks that could not be ordered.
fn ordered(subtasks: &[Subtask]) -> Result<Vec<usize>, Vec<usize>> {
    let held = subtasks.len();
    let mut placed = vec![false; held];
    let mut order = Vec::with_capacity(held);
    // The lowest-numbered sub-task that could go next, every turn: that is what
    // keeps the sort stable, since a sub-task is only overtaken by one it waits
    // on. Held to `held` turns because each turn places one.
    for _ in 0..held {
        let Some(next) =
            (0..held).find(|index| !placed[*index] && ready(&subtasks[*index], &placed))
        else {
            break;
        };
        placed[next] = true;
        order.push(next);
    }
    if order.len() == held {
        Ok(order)
    } else {
        Err(tangled(subtasks, &placed))
    }
}

fn ready(subtask: &Subtask, placed: &[bool]) -> bool {
    subtask
        .depends_on
        .iter()
        .all(|position| placed_yet(*position, placed))
}

// A position that resolves is ready when the sub-task it names is placed; one
// that does not resolve cannot hold anything up, for the reason `resolved`
// gives.
fn placed_yet(position: usize, placed: &[bool]) -> bool {
    position
        .checked_sub(1)
        .and_then(|index| placed.get(index))
        .copied()
        .unwrap_or(true)
}

// Which of the sub-tasks left over from the sort to name in the halt. The
// leftovers are the circle plus whatever waits on it, and a sub-task that
// merely waits on a circle is not in one — so anything nothing else in the
// leftovers waits on is dropped, over and over until the set stops shrinking.
// What is left is the sub-tasks that both wait and are waited on, which is the
// circle itself and any run of sub-tasks between two circles.
fn tangled(subtasks: &[Subtask], placed: &[bool]) -> Vec<usize> {
    let over: Vec<usize> = (0..subtasks.len())
        .filter(|index| !placed[*index])
        .collect();
    let mut caught = over.clone();
    loop {
        let held = caught.clone();
        caught.retain(|index| {
            held.iter()
                .any(|other| subtasks[*other].depends_on.contains(&(index + 1)))
        });
        if caught.len() == held.len() {
            break;
        }
    }
    // Every leftover waits on another leftover, so a leftover set always holds
    // a circle and this cannot empty. If it ever did, naming every leftover
    // would still be true and a halt that names nothing would not be.
    if caught.is_empty() { over } else { caught }
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
that alone. Cut the ticket into as few sub-tasks as it honestly takes. A \
change and the test that covers it are one sub-task, and so are a change and \
the edit that keeps the build compiling after it: split apart, the first \
cannot pass its own checks. Everything you write comes from the ticket and from what you read in the \
repository — do not plan work the ticket did not ask for.

Cut only the work this ticket describes: the brief it came from and any other \
ticket are context, never scope, so work they name that this ticket does not \
belongs to their sub-tasks and not to these. Read the repository before you \
cut. Where it already does part of what the ticket asks, leave that part out. \
Where it already does all of it, cut one sub-task that checks the ticket's \
definition of done against the code as it stands and changes only what \
fails that check — never one that restates the ticket as if nothing were \
there.

The ticket's rules bind every sub-task: what it fixes exactly — a value, a \
name, a count, whether something is exported — what it forbids, and what it \
says to leave alone. Never loosen, invert or drop one. Where a sub-task touches \
something the ticket fixes, copy the ticket's words for it into that \
sub-task's \"definition_of_done\" rather than paraphrasing them.

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
                goal: fit(
                    &format!("Stand in for {named}"),
                    "(stand-in)",
                    GOAL_MINIMUM,
                    GOAL_CHARS,
                ),
                depends_on: Vec::new(),
                definition_of_done: vec![DONE.to_owned()],
                likely_files: vec!["src/lib.rs".to_owned()],
                test_plan: TESTS.to_owned(),
                notes: NOTES.to_owned(),
            },
            Subtask {
                goal: fit(
                    &format!("Follow on from {named}"),
                    "(stand-in)",
                    GOAL_MINIMUM,
                    GOAL_CHARS,
                ),
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

#[cfg(test)]
#[path = "tests/splitting.rs"]
mod tests;
