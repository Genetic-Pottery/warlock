use std::fmt;
use std::fmt::Write as _;

use serde::de::DeserializeOwned;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Defect<S> {
    NotJson {
        detail: String,
    },
    Missing {
        field: S,
    },
    Empty {
        field: S,
    },
    Multiline {
        field: S,
    },
    TooShort {
        field: S,
        chars: usize,
        minimum: usize,
    },
    TooLong {
        field: S,
        chars: usize,
        cap: usize,
    },
    TooMany {
        field: S,
        count: usize,
        cap: usize,
    },
    UnknownTarget {
        field: S,
        name: String,
    },
    ToolNamed {
        field: S,
    },
}

// Sent back to the model word for word by `turned_down`, so a change here is a
// change to every prompt that asks again.
impl<S: fmt::Display> fmt::Display for Defect<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson { detail } => write!(f, "the answer is not a JSON object: {detail}"),
            Self::Missing { field } => write!(f, "{field} is missing"),
            Self::Empty { field } => write!(f, "{field} is empty"),
            Self::Multiline { field } => write!(f, "{field} runs to more than one line"),
            Self::TooShort {
                field,
                chars,
                minimum,
            } => write!(
                f,
                "{field} is {chars} characters, under the {minimum} it has to reach"
            ),
            Self::TooLong { field, chars, cap } => {
                write!(f, "{field} is {chars} characters, over the cap of {cap}")
            }
            Self::TooMany { field, count, cap } => {
                write!(f, "{field} has {count} entries, over the cap of {cap}")
            }
            Self::ToolNamed { field } => write!(
                f,
                "{field} names warlock, which is the tool writing this document and not \
                 something the files mention"
            ),
            Self::UnknownTarget { field, name } => write!(
                f,
                "{field} names `{name}`, which is not a file, a subdirectory, or a name \
                 declared in one of them"
            ),
        }
    }
}

impl<S: fmt::Debug + fmt::Display> std::error::Error for Defect<S> {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted<F, S> {
    Filled(F),
    Defective { fill: F, defects: Vec<Defect<S>> },
    Unparsed(Defect<S>),
}

impl<F, S> Accepted<F, S> {
    #[must_use]
    pub fn judged(fill: F, defects: Vec<Defect<S>>) -> Self {
        if defects.is_empty() {
            Self::Filled(fill)
        } else {
            Self::Defective { fill, defects }
        }
    }
}

#[must_use]
pub fn accept<F: DeserializeOwned, S>(
    answer: &str,
    check: impl FnOnce(&F) -> Vec<Defect<S>>,
) -> Accepted<F, S> {
    match parse(answer) {
        Ok(fill) => {
            let defects = check(&fill);
            Accepted::judged(fill, defects)
        }
        Err(defect) => Accepted::Unparsed(defect),
    }
}

/// Which defects on an answer that parsed are worth another turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reask {
    /// Every one.
    Any,
    /// Only the ones the mend answers by throwing away what the model wrote — a
    /// value cut to its cap, a list cut to its first entries. Every other
    /// defect the mend fixes without losing anything the model said, so a turn
    /// spent on it buys nothing; a cut is different, because a body cut at its
    /// cap drops whatever the model wrote last, and that was once the end of a
    /// ticket's "left alone" list.
    Cut,
}

impl Reask {
    fn rejected<S: Clone>(self, defects: &[Defect<S>]) -> Vec<Defect<S>> {
        match self {
            Self::Any => defects.to_vec(),
            Self::Cut => defects
                .iter()
                .filter(|defect| matches!(defect, Defect::TooLong { .. } | Defect::TooMany { .. }))
                .cloned()
                .collect(),
        }
    }
}

/// How a road asks: how many turns in all, the first included, and which
/// defects earn another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asking {
    pub attempts: usize,
    pub reask: Reask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled<F, S> {
    /// An answer with nothing left to ask about: clean, or defective only in
    /// ways [`Reask`] lets the mend have.
    Taken(F),
    /// The attempts ran out, and this is the last answer that parsed.
    Spent(F),
    /// The attempts ran out and no answer parsed; the last one's defect.
    Unusable(Defect<S>),
}

/// The attempt loop. `turn` is one model turn, handed the defects the last
/// attempt is being refused for — empty the first time — and `refused` hears
/// each refusal, with its 1-based attempt number, before the next turn is
/// asked or the loop gives up.
///
/// A turn that fails ends the loop at once: a pass that produced no answer is
/// not a pass that produced a wrong one, and asking a missing `claude` again
/// finds it still missing.
///
/// # Errors
///
/// Whatever `turn` failed with.
pub fn settle<F, S: Clone, E>(
    asking: Asking,
    mut turn: impl FnMut(&[Defect<S>]) -> Result<Accepted<F, S>, E>,
    mut refused: impl FnMut(&[Defect<S>], usize),
) -> Result<Settled<F, S>, E> {
    let mut rejected = Vec::new();
    let mut held = None;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let spent = match turn(&rejected)? {
            Accepted::Filled(fill) => return Ok(Settled::Taken(fill)),
            Accepted::Defective { fill, defects } => {
                rejected = asking.reask.rejected(&defects);
                if rejected.is_empty() {
                    return Ok(Settled::Taken(fill));
                }
                Settled::Spent(fill)
            }
            Accepted::Unparsed(defect) => {
                rejected = vec![defect.clone()];
                match held.take() {
                    Some(Settled::Spent(fill)) => Settled::Spent(fill),
                    _ => Settled::Unusable(defect),
                }
            }
        };
        refused(&rejected, attempt);
        if attempt >= asking.attempts {
            return Ok(spent);
        }
        held = Some(spent);
    }
}

/// One repair the mend made, `field` the same slot a [`Defect`] names, so a
/// caller can line a mend up against the defect it answers. Each road says what
/// `done` means in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mend<M, S> {
    pub field: S,
    pub done: M,
}

/// A repair as a road records it: the four every road makes, and `Own` for the
/// ones only one road makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mended<X> {
    FirstLine,
    Cut { from: usize, to: usize },
    Shortened { from: usize, to: usize },
    // The slot was never answered and fell to warlock's own text, which
    // `Wording::SUPPLIED` names.
    Supplied,
    Own(X),
}

impl<X> From<Rewrite> for Mended<X> {
    fn from(rewrite: Rewrite) -> Self {
        match rewrite {
            Rewrite::FirstLine => Self::FirstLine,
            Rewrite::Cut { from, to } => Self::Cut { from, to },
        }
    }
}

/// A road's side of a [`Mend`]'s sentence.
pub trait Wording {
    /// What a `Supplied` slot was filled in from.
    const SUPPLIED: &'static str;

    fn word(&self, field: &dyn fmt::Display, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<X: Wording, S: fmt::Display> fmt::Display for Mend<Mended<X>, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = &self.field;
        match &self.done {
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
            Mended::Supplied => write!(
                f,
                "{field} was not answered and was filled in from {}",
                X::SUPPLIED
            ),
            Mended::Own(own) => own.word(field, f),
        }
    }
}

/// The two repairs every road makes the same way, rewriting a one-line value
/// in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewrite {
    FirstLine,
    Cut { from: usize, to: usize },
}

/// One fill schema's side of the mend: its check and its repairs by slot.
pub trait Schema {
    type Fill;
    /// Where in the fill a defect is. Its `Display` is the path the model is
    /// shown in a turned-down answer, such as `drafts[2].title`, so a change to
    /// it is a change to every prompt that asks again.
    type Slot: Clone;
    type Mended: From<Rewrite>;
    type Plan;

    const MEND_PASSES: usize;

    fn check(&self, fill: &Self::Fill) -> Vec<Defect<Self::Slot>>;

    /// What this pass drops, fills in whole and cuts back, decided from the
    /// defects alone and recorded as it is decided.
    fn plan(
        &self,
        defects: &[Defect<Self::Slot>],
        mends: &mut Vec<Mend<Self::Mended, Self::Slot>>,
    ) -> Self::Plan;

    /// The value a [`Rewrite`] of `slot` writes over, or `None` for a slot the
    /// plan is already dropping, filling in whole or cutting off the end of its
    /// list: the record would name work the same pass undoes.
    fn rewritable<'f>(
        plan: &Self::Plan,
        fill: &'f mut Self::Fill,
        slot: &Self::Slot,
    ) -> Option<&'f mut String>;

    fn carry_out(&self, plan: Self::Plan, fill: &mut Self::Fill);
}

type Mends<S> = Vec<Mend<<S as Schema>::Mended, <S as Schema>::Slot>>;

/// The mechanical mend, and how many passes it took: repair, check again,
/// repair what the repair left, until the check is empty or
/// [`Schema::MEND_PASSES`] is spent. The bound is a stop and not a schedule —
/// an unbounded version would spin between two rules instead of writing
/// anything.
///
/// The order inside a pass is what keeps a list's indices meaning what the
/// defects say they mean: the plan is made first, then the values that survive
/// it are rewritten in place, and only then does anything move — so a defect
/// naming `structure[3]` is never applied to whatever slid into position 3.
pub fn mend<S: Schema>(schema: &S, mut fill: S::Fill) -> (S::Fill, Mends<S>, usize) {
    let mut mends = Vec::new();
    let mut passes = 0;
    for _ in 0..S::MEND_PASSES {
        let defects = schema.check(&fill);
        if defects.is_empty() {
            break;
        }
        passes += 1;
        let plan = schema.plan(&defects, &mut mends);
        for defect in &defects {
            let (field, rewrite) = match defect {
                Defect::Multiline { field } => (field, Rewrite::FirstLine),
                Defect::TooLong { field, chars, cap } => (
                    field,
                    Rewrite::Cut {
                        from: *chars,
                        to: *cap,
                    },
                ),
                _ => continue,
            };
            let Some(value) = S::rewritable(&plan, &mut fill, field) else {
                continue;
            };
            match rewrite {
                // The first line of the value as the check reads it: `line`
                // measures the trimmed value, so a value that opens with a
                // blank line keeps the first line of what was actually written.
                Rewrite::FirstLine => {
                    *value = value.trim().lines().next().unwrap_or_default().to_owned();
                }
                // Characters, not bytes, and so on a character boundary. No
                // trim: the cut lands where it lands.
                Rewrite::Cut { to, .. } => *value = value.chars().take(to).collect(),
            }
            mends.push(Mend {
                field: field.clone(),
                done: rewrite.into(),
            });
        }
        schema.carry_out(plan, &mut fill);
    }
    (fill, mends, passes)
}

pub(crate) fn turned_down<S: fmt::Display>(text: &mut String, rejected: &[Defect<S>]) {
    if rejected.is_empty() {
        return;
    }
    text.push_str(
        "\n\nA previous answer to exactly this request was turned down. Do not repeat \
         these defects:",
    );
    for defect in rejected {
        let _ = write!(text, "\n- {defect}");
    }
}

pub(crate) fn parse<T: DeserializeOwned, S>(answer: &str) -> Result<T, Defect<S>> {
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

pub(crate) fn line<S: Clone>(
    field: &S,
    value: &str,
    minimum: usize,
    cap: usize,
    defects: &mut Vec<Defect<S>>,
) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        defects.push(Defect::Empty {
            field: field.clone(),
        });
        return;
    }
    if trimmed.contains(['\n', '\r']) {
        defects.push(Defect::Multiline {
            field: field.clone(),
        });
    }
    let chars = trimmed.chars().count();
    if chars < minimum {
        defects.push(Defect::TooShort {
            field: field.clone(),
            chars,
            minimum,
        });
    }
    if chars > cap {
        defects.push(Defect::TooLong {
            field: field.clone(),
            chars,
            cap,
        });
    }
}

// The shape a one-line value has to hold: one line, at least `minimum`
// characters and at most `cap`, counted as characters and cut on a character
// boundary so a multibyte name or title cannot split. A value out of
// here is never defective, which is what lets the mend's fixpoint settle.
pub(crate) fn fit(line: &str, pad: &str, minimum: usize, cap: usize) -> String {
    let mut line = flattened(line);
    // A non-empty pad adds at least one character a turn, so this ends.
    while line.chars().count() < minimum && !pad.is_empty() {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(pad);
    }
    cut(&line, cap)
}

pub(crate) fn cut(text: &str, cap: usize) -> String {
    let cut: String = text.chars().take(cap).collect();
    cut.trim_end().to_owned()
}

pub(crate) fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/fill.rs"]
mod tests;
