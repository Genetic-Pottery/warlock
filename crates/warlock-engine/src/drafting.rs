use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::document::{Defect, turned_down};

// The drafting road asks again exactly as often as the document road does, and
// a second bound would be a number to keep in step with this one for no gain.
pub use crate::document::ATTEMPTS;

pub const DRAFTS_PER_SLICE: usize = 12;

pub const TITLE_CHARS: usize = 120;

pub const TITLE_MINIMUM: usize = 12;

pub const BODY_CHARS: usize = 4000;

// A reference is an index into this slice's own drafts, so a list longer than
// the array cap can only be repeating itself or pointing outside the slice —
// and a reference outside the slice is dropped rather than resolved.
pub const REFERENCES_PER_LIST: usize = DRAFTS_PER_SLICE;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    #[serde(default)]
    pub drafts: Vec<Draft>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stated")]
pub struct Draft {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub blocked_by: Vec<usize>,
    #[serde(default)]
    pub blocks: Vec<usize>,
}

// Answering a draft with a bare title rather than the object asked for is the
// commonest shape a model gets wrong, and reading it strictly would answer that
// slip with `document::Defect::NotJson` — the one defect with nothing to repair
// from, which throws the whole slice's answer away over one entry. Read
// leniently and the same slip is a draft with an empty body, which the repair
// road already handles: asked about once, and filled from the slice's own text
// if the answer comes back no better.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stated {
    Title(String),
    Draft {
        #[serde(default)]
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        blocked_by: Vec<usize>,
        #[serde(default)]
        blocks: Vec<usize>,
    },
}

impl From<Stated> for Draft {
    fn from(stated: Stated) -> Self {
        match stated {
            Stated::Title(title) => Self {
                title,
                body: String::new(),
                blocked_by: Vec::new(),
                blocks: Vec::new(),
            },
            Stated::Draft {
                title,
                body,
                blocked_by,
                blocks,
            } => Self {
                title,
                body,
                blocked_by,
                blocks,
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

#[cfg(test)]
impl Draft {
    fn of(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            blocked_by: Vec::new(),
            blocks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    Filled(Fill),
    Defective { fill: Fill, defects: Vec<Defect> },
    Unparsed(Defect),
}

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

/// Every slot of a filled drafting answer, in [`Defect`]'s own vocabulary, with
/// `field` spelt as the path to the slot: `drafts`, `drafts[2].title`,
/// `drafts[2].blocked_by`.
#[must_use]
pub fn check(fill: &Fill) -> Vec<Defect> {
    let mut defects = Vec::new();

    // `Missing` rather than `Empty` for a slice nobody drafted, because
    // `#[serde(default)]` reads an absent `drafts` key and an empty array as the
    // same value and there is no way back to which was written. The repair has
    // to build a draft either way, and `Missing` is what a slot with nothing in
    // it to work from is called on the document road.
    if fill.drafts.is_empty() {
        defects.push(Defect::Missing {
            field: "drafts".to_owned(),
        });
    }
    if fill.drafts.len() > DRAFTS_PER_SLICE {
        defects.push(Defect::TooMany {
            field: "drafts".to_owned(),
            count: fill.drafts.len(),
            cap: DRAFTS_PER_SLICE,
        });
    }

    for (index, draft) in fill.drafts.iter().enumerate() {
        line(
            &format!("drafts[{index}].title"),
            &draft.title,
            TITLE_MINIMUM,
            TITLE_CHARS,
            &mut defects,
        );
        prose(&format!("drafts[{index}].body"), &draft.body, &mut defects);
        references(
            &format!("drafts[{index}].blocked_by"),
            &draft.blocked_by,
            &mut defects,
        );
        references(
            &format!("drafts[{index}].blocks"),
            &draft.blocks,
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

// A body is the ticket's own description and runs to paragraphs, so it is held
// to a cap and to being written at all — never to one line, and to no minimum:
// the title carries the floor, and a one-sentence ticket body is a short ticket
// rather than a defective one.
fn prose(field: &str, value: &str, defects: &mut Vec<Defect>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        defects.push(Defect::Empty {
            field: field.to_owned(),
        });
        return;
    }
    let chars = trimmed.chars().count();
    if chars > BODY_CHARS {
        defects.push(Defect::TooLong {
            field: field.to_owned(),
            chars,
            cap: BODY_CHARS,
        });
    }
}

// Only the length. An index pointing outside this slice's own drafts — past the
// end of the array, or at the draft carrying it — goes unreported here, and the
// repair drops it: there is no defect in this vocabulary that says it without
// inventing one, `UnknownTarget` being the document road's word for a claim its
// evidence cannot vouch for, and this contract has no evidence witness at all.
// Re-asking would buy nothing either, since dropping the index is the whole of
// the repair and a re-ask cannot make a reference to a ticket that does not
// exist right. Whichever way that is decided, `check` over a mended fill is
// empty, because nothing here reports it.
fn references(field: &str, given: &[usize], defects: &mut Vec<Defect>) {
    if given.len() > REFERENCES_PER_LIST {
        defects.push(Defect::TooMany {
            field: field.to_owned(),
            count: given.len(),
            cap: REFERENCES_PER_LIST,
        });
    }
}

// The text warlock writes into a slot the pass left empty, built out of the
// only two things this contract holds: the slice's own title and the brief's
// prose about it. Nothing here says what the ticket should do, because that is
// exactly what a pass which did not answer never said, and nothing here is
// phrased as if a model wrote it — a filled-in draft that reads like a drafted
// one is worse than an obviously empty one, because it gets filed. So every
// line opens by saying it was not drafted. Do not dress these up.
mod fallback {
    use super::{BODY_CHARS, TITLE_CHARS, TITLE_MINIMUM, flattened};

    pub(super) fn title(slice: &str, index: usize) -> String {
        fit(
            &format!("Unwritten draft {} of {}", index + 1, named(slice)),
            "(no title was drafted)",
        )
    }

    pub(super) fn body(slice: &str, prose: &str, index: usize) -> String {
        let prose = prose.trim();
        let mut text = format!(
            "No body was drafted for this ticket. It is draft {} of {}, and warlock filled this \
             in rather than leave the ticket empty.",
            index + 1,
            named(slice),
        );
        if prose.is_empty() {
            text.push_str(" The brief says nothing further about the slice.");
        } else {
            text.push_str(" The brief says of the slice:\n\n");
            text.push_str(prose);
        }
        cut(&text, BODY_CHARS)
    }

    fn named(slice: &str) -> String {
        let slice = flattened(slice);
        if slice.is_empty() {
            "an unnamed slice".to_owned()
        } else {
            format!("the slice `{slice}`")
        }
    }

    // The shape a fallback title has to hold: one line, at least
    // `TITLE_MINIMUM` characters and at most `TITLE_CHARS`, counted as
    // characters and cut on a character boundary so a multibyte slice title
    // cannot split. A value out of here is never defective, which is what lets
    // the fixpoint below settle.
    pub(super) fn fit(line: &str, pad: &str) -> String {
        let mut line = flattened(line);
        // A non-empty pad adds at least one character a turn, so this ends. In
        // practice it never runs: the shortest line built here clears the floor
        // on its own.
        while line.chars().count() < TITLE_MINIMUM && !pad.is_empty() {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(pad);
        }
        cut(&line, TITLE_CHARS)
    }

    fn cut(text: &str, cap: usize) -> String {
        let cut: String = text.chars().take(cap).collect();
        cut.trim_end().to_owned()
    }
}

// Three, and the shape of the worst chain is why. A repair can make a slot
// defective in a new way — a multiline title cut back to its first line can
// land under `TITLE_MINIMUM` — so the mend is a fixpoint and not a single
// sweep: repair, check again, repair what the repair left. The longest chain a
// rule here can start is two links (keep the first line, then fall back), and a
// third pass is the margin. It is a stop and not a schedule: the loop leaves as
// soon as `check` comes back empty, and if it ever did not, an unbounded
// version would spin between two rules instead of drafting tickets.
pub const MEND_PASSES: usize = 3;

// `field` is the slot in [`Defect`]'s own spelling — `drafts`,
// `drafts[2].title`, `drafts[2].blocked_by` — so a caller can line a mend up
// against the defect it answers without parsing prose.
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
    // References pointing outside this slice's own drafts, or at the draft
    // carrying them, and so gone.
    Dropped { count: usize },
    // The slot was never answered and fell to warlock's own line about the
    // slice. See [`fallback`].
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
            Mended::Dropped { count } => write!(
                f,
                "{field} named {count} position(s) outside this slice's drafts and lost them"
            ),
            Mended::Supplied => write!(
                f,
                "{field} was not answered and was filled in from the slice's own text"
            ),
        }
    }
}

/// The mechanical mend: the floor under an exhausted attempt loop. Every
/// [`Defect`] but `NotJson` has a repair here, and none of them reaches a model
/// — the evidence is the answer's own text and the slice's title and prose. A
/// fill that comes back from here is not defective: [`check`] over it is empty,
/// and every reference it still carries points at another draft of this slice.
#[must_use]
pub fn mend(fill: &Fill, title: &str, prose: &str) -> (Fill, Vec<Mend>) {
    let (fill, mends, _) = mended(fill, title, prose);
    (fill, mends)
}

// The same operation, saying how many passes it took. Private because the count
// is a fact about this function and not about the drafts; the test for the
// bound is the only caller that has any use for it.
fn mended(fill: &Fill, title: &str, prose: &str) -> (Fill, Vec<Mend>, usize) {
    let mut fill = fill.clone();
    let mut mends = Vec::new();
    let mut passes = 0;
    for _ in 0..MEND_PASSES {
        let defects = check(&fill);
        if defects.is_empty() {
            break;
        }
        passes += 1;
        sweep(&mut fill, &defects, title, prose, &mut mends);
    }
    prune(&mut fill, &mut mends);

    (fill, mends, passes)
}

// The one repair no defect asks for: `check` reports nothing about where a
// reference points, so this runs whether or not the fill was defective, and it
// runs after the loop because the loop is what settles how long the array is.
// An index past the end of the array and an index naming the draft that carries
// it are both dropped rather than resolved: this slice's drafts are the only
// tickets this contract knows about, so there is nowhere else such an index
// could be looking, and a ticket that blocks itself is an order no filing road
// could carry out.
fn prune(fill: &mut Fill, mends: &mut Vec<Mend>) {
    let held = fill.drafts.len();
    for (index, draft) in fill.drafts.iter_mut().enumerate() {
        for (name, list) in [
            ("blocked_by", &mut draft.blocked_by),
            ("blocks", &mut draft.blocks),
        ] {
            let before = list.len();
            list.retain(|target| *target < held && *target != index);
            let dropped = before - list.len();
            if dropped > 0 {
                mends.push(Mend {
                    field: format!("drafts[{index}].{name}"),
                    done: Mended::Dropped { count: dropped },
                });
            }
        }
    }
}

// One pass of the fixpoint. The order inside it is what keeps the indices
// meaning what the defects say they mean: what to fill and what to cut is
// decided first, then the values that survive are rewritten in place, and only
// then does the array itself move — so a defect naming `drafts[3]` is never
// applied to whatever slid into position 3.
fn sweep(fill: &mut Fill, defects: &[Defect], title: &str, prose: &str, mends: &mut Vec<Mend>) {
    let mut plan = Plan::default();
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
            // measures the trimmed value, so a title that opens with a blank
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
    plan.carry_out(fill, title, prose);
}

#[derive(Debug, Default)]
struct Plan {
    drafts: bool,
    cut_drafts: bool,
    filled_titles: BTreeSet<usize>,
    filled_bodies: BTreeSet<usize>,
    cut_blocked_by: BTreeSet<usize>,
    cut_blocks: BTreeSet<usize>,
}

impl Plan {
    fn note_cut(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let Defect::TooMany { field, count, cap } = defect else {
            return;
        };
        let recorded = match slot(field) {
            Slot::Drafts => !std::mem::replace(&mut self.cut_drafts, true),
            Slot::BlockedBy(index) => self.cut_blocked_by.insert(index),
            Slot::Blocks(index) => self.cut_blocks.insert(index),
            Slot::Title(_) | Slot::Body(_) | Slot::Unknown => false,
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

    // What was never answered: an array with nothing in it, a title or body
    // left empty, a title too short to name a ticket. Warlock says what the
    // slice says instead.
    fn note_fill(&mut self, defect: &Defect, mends: &mut Vec<Mend>) {
        let (Defect::Missing { field } | Defect::Empty { field } | Defect::TooShort { field, .. }) =
            defect
        else {
            return;
        };
        let recorded = match slot(field) {
            Slot::Drafts => !std::mem::replace(&mut self.drafts, true),
            Slot::Title(index) => !self.cut_off(index) && self.filled_titles.insert(index),
            Slot::Body(index) => !self.cut_off(index) && self.filled_bodies.insert(index),
            Slot::BlockedBy(_) | Slot::Blocks(_) | Slot::Unknown => false,
        };
        if recorded {
            mends.push(Mend {
                field: field.clone(),
                done: Mended::Supplied,
            });
        }
    }

    fn covers(&self, field: &str) -> bool {
        match slot(field) {
            Slot::Drafts => self.drafts,
            Slot::Title(index) => self.filled_titles.contains(&index) || self.cut_off(index),
            Slot::Body(index) => self.filled_bodies.contains(&index) || self.cut_off(index),
            Slot::BlockedBy(_) | Slot::Blocks(_) | Slot::Unknown => false,
        }
    }

    // Whether a draft is past the cap of an array this pass cuts back, and so
    // about to go anyway.
    fn cut_off(&self, index: usize) -> bool {
        self.cut_drafts && index >= DRAFTS_PER_SLICE
    }

    fn carry_out(self, fill: &mut Fill, title: &str, prose: &str) {
        if self.drafts {
            fill.drafts.push(Draft {
                title: fallback::title(title, fill.drafts.len()),
                body: fallback::body(title, prose, fill.drafts.len()),
                blocked_by: Vec::new(),
                blocks: Vec::new(),
            });
        }
        for index in &self.filled_titles {
            if let Some(draft) = fill.drafts.get_mut(*index) {
                draft.title = fallback::title(title, *index);
            }
        }
        for index in &self.filled_bodies {
            if let Some(draft) = fill.drafts.get_mut(*index) {
                draft.body = fallback::body(title, prose, *index);
            }
        }
        for index in &self.cut_blocked_by {
            if let Some(draft) = fill.drafts.get_mut(*index) {
                draft.blocked_by.truncate(REFERENCES_PER_LIST);
            }
        }
        for index in &self.cut_blocks {
            if let Some(draft) = fill.drafts.get_mut(*index) {
                draft.blocks.truncate(REFERENCES_PER_LIST);
            }
        }

        // Last, and off the end. Every index above was read against the
        // pre-pass array, and the array is cut rather than thinned from the
        // middle: nothing slides, so a reference that survives still means the
        // draft it always meant, and a reference into the part that went is out
        // of range and `prune` drops it.
        if self.cut_drafts {
            fill.drafts.truncate(DRAFTS_PER_SLICE);
        }
    }
}

// The slot a defect's `field` names, read back out of the spelling `check`
// wrote it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Drafts,
    Title(usize),
    Body(usize),
    BlockedBy(usize),
    Blocks(usize),
    Unknown,
}

fn slot(field: &str) -> Slot {
    if field == "drafts" {
        return Slot::Drafts;
    }
    let Some(rest) = field.strip_prefix("drafts[") else {
        return Slot::Unknown;
    };
    let Some((inside, tail)) = rest.split_once(']') else {
        return Slot::Unknown;
    };
    let Ok(index) = inside.parse::<usize>() else {
        return Slot::Unknown;
    };
    match tail {
        ".title" => Slot::Title(index),
        ".body" => Slot::Body(index),
        ".blocked_by" => Slot::BlockedBy(index),
        ".blocks" => Slot::Blocks(index),
        _ => Slot::Unknown,
    }
}

// The value a rewrite writes over.
fn target<'f>(fill: &'f mut Fill, field: &str) -> Option<&'f mut String> {
    match slot(field) {
        Slot::Title(index) => fill.drafts.get_mut(index).map(|draft| &mut draft.title),
        Slot::Body(index) => fill.drafts.get_mut(index).map(|draft| &mut draft.body),
        Slot::Drafts | Slot::BlockedBy(_) | Slot::Blocks(_) | Slot::Unknown => None,
    }
}

// The caps are written into the prose rather than asked about, and the numbers
// themselves are arbitrary: what is permanent is that there is a ceiling at
// all. A pass told the number answers inside it; a pass asked to be brief
// answers at whatever length it likes and is then repaired, which costs an
// attempt and loses whatever it wrote past the cut. The test below reads the
// constants, so moving one and leaving the prose behind fails rather than
// silently instructing the model to break the cap it is checked against.
pub const DRAFTING_PROMPT: &str = "\
Fill in the JSON object at the end of these instructions, cutting the one \
slice of work described below it into tickets, and output the filled object \
and nothing else.

You are shown the brief the work was planned in and one slice of that brief. \
Draft the tickets that slice is worth and no others: a ticket is a piece of \
work one person can pick up, finish and check. Everything you write comes from \
the brief and the slice — do not plan work neither of them asked for, and do \
not draft the rest of the brief.

\"drafts\": one entry per ticket, in the order the work would be done, at most \
12 entries. Each entry is {\"title\": ..., \"body\": ..., \"blocked_by\": \
[...], \"blocks\": [...]}.

\"title\": one line, between 12 and 120 characters, saying what the ticket does \
in the words the brief uses for it.

\"body\": what the ticket asks for, what would show it was done, and what it \
leaves alone. It may run to several paragraphs, at most 4000 characters.

\"blocked_by\" and \"blocks\": the order the drafts have to be done in, given \
as positions in the array above, counting from 0, at most 12 positions per \
list. A draft refers only to the other drafts of this slice: never to itself, \
never to a position the array does not hold, and never to work outside the \
slice. Where nothing is ordered, both lists are empty.

Write each ticket in its own voice: no first person, and nothing about this \
request or about what you were or were not shown.";

/// The drafting prompt with the brief, the one slice and the shape to fill
/// appended, in the layout [`crate::document::synthesis_instructions`] uses.
///
/// The slice arrives as its plain title and prose, so nothing about where a
/// slice came from or where its drafts are going reaches the engine.
#[must_use]
pub fn drafting_instructions(brief: &str, title: &str, prose: &str, rejected: &[Defect]) -> String {
    let mut text = DRAFTING_PROMPT.to_owned();
    turned_down(&mut text, rejected);
    let _ = write!(
        text,
        "\n\nThe brief:\n\n{}\n\nThe one slice to cut into tickets is `{}`, and the brief says \
         of it:\n\n{}",
        brief.trim(),
        title.trim(),
        prose.trim(),
    );
    let shape = serde_json::json!({
        "drafts": [{"title": "", "body": "", "blocked_by": [], "blocks": []}],
    });
    let _ = write!(
        text,
        "\n\nReturn an object of exactly this shape, carrying one entry per ticket with every \
         empty string filled in, as JSON, with no code fence and nothing before or after \
         it:\n\n{shape}",
    );
    text
}

/// The answer a test double hands back for a drafting pass, as
/// [`crate::document::stub_answer`] does for the document road.
///
/// The slice arrives as plain text and nothing else, so the stub is two drafts
/// built out of that text: one blocking the other, both inside every cap, so a
/// double's answer is accepted rather than repaired.
#[must_use]
pub fn stub_answer(slice: &str) -> String {
    const BODY: &str = "A stand-in ticket body, written by a test double that read no repository \
                        and made no plan. It says what the slice said and nothing more.";
    let named = flattened(slice);
    let named = if named.is_empty() {
        "an unnamed slice"
    } else {
        &named
    };
    Fill {
        drafts: vec![
            Draft {
                title: fallback::fit(&format!("Stand in for {named}"), "(stand-in)"),
                body: BODY.to_owned(),
                blocked_by: Vec::new(),
                blocks: vec![1],
            },
            Draft {
                title: fallback::fit(&format!("Follow on from {named}"), "(stand-in)"),
                body: BODY.to_owned(),
                blocked_by: vec![0],
                blocks: Vec::new(),
            },
        ],
    }
    .to_json()
}

fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/drafting.rs"]
mod tests;
