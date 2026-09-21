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
                title: fitted(&format!("Stand in for {named}")),
                body: BODY.to_owned(),
                blocked_by: Vec::new(),
                blocks: vec![1],
            },
            Draft {
                title: fitted(&format!("Follow on from {named}")),
                body: BODY.to_owned(),
                blocked_by: vec![0],
                blocks: Vec::new(),
            },
        ],
    }
    .to_json()
}

fn fitted(title: &str) -> String {
    let mut title = flattened(title);
    // The pad is non-empty, so this ends; in practice it never runs.
    while title.chars().count() < TITLE_MINIMUM {
        title.push_str(" (stand-in)");
    }
    let cut: String = title.chars().take(TITLE_CHARS).collect();
    cut.trim_end().to_owned()
}

fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/drafting.rs"]
mod tests;
