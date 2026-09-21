use serde::{Deserialize, Serialize};

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
