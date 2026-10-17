use std::str::Lines;

use serde_json::{Value, json};

// ADF's own wire field, not warlock's: every document Jira's v3 API accepts
// carries 1, and a tree sent with anything else is refused whole.
const VERSION: u8 = 1;

const BULLET: &str = "bulletList";
const ORDERED: &str = "orderedList";

// A heading deeper than this is not one: ADF has six levels, and `#######` is a
// paragraph in Markdown too.
const LEVELS: usize = 6;

/// `body` as an ADF document tree.
///
/// Total, and that is the point: a construct this does not model becomes a
/// paragraph holding the source line as it was written, so no body is refused
/// and nothing is dropped.
///
/// ```
/// use warlock_tui::document_for;
///
/// let document = document_for("## Scope\n\nOne slice.\n");
///
/// assert_eq!(document["version"], 1);
/// assert_eq!(document["content"][0]["attrs"]["level"], 2);
/// ```
#[must_use]
pub fn document_for(body: &str) -> Value {
    json!({ "type": "doc", "version": VERSION, "content": blocks(body) })
}

fn blocks(body: &str) -> Vec<Value> {
    let mut content = Vec::new();
    let mut list: Option<(&'static str, Vec<Value>)> = None;
    let mut lines = body.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if let Some((marker, language)) = fenced(trimmed) {
            closed(&mut content, &mut list);
            content.push(code_block(marker, language, &mut lines));
            continue;
        }

        // A blank line does not close a list: a blank between two items is one
        // loose list in Markdown and reads as one list on the board, where
        // closing here would file a brief's criteria as a column of one-item
        // lists.
        if trimmed.is_empty() {
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            closed(&mut content, &mut list);
            content.push(json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": [text_node(text)],
            }));
            continue;
        }

        if let Some((kind, item)) = item(trimmed) {
            if list.as_ref().is_some_and(|(open, _)| *open != kind) {
                closed(&mut content, &mut list);
            }
            let (_, items) = list.get_or_insert_with(|| (kind, Vec::new()));
            items.push(json!({
                "type": "listItem",
                "content": [paragraph(item)],
            }));
            continue;
        }

        closed(&mut content, &mut list);
        content.push(if source_text(trimmed) {
            literal(trimmed)
        } else {
            paragraph(trimmed)
        });
    }

    closed(&mut content, &mut list);
    content
}

fn closed(content: &mut Vec<Value>, list: &mut Option<(&'static str, Vec<Value>)>) {
    if let Some((kind, items)) = list.take() {
        content.push(json!({ "type": kind, "content": items }));
    }
}

// The marker is held rather than its length, so a block opened with backticks
// is closed by backticks and a `~~~` inside a ``` block is content.
fn fenced(trimmed: &str) -> Option<(&'static str, Option<&str>)> {
    let (marker, fence) = [("```", '`'), ("~~~", '~')]
        .into_iter()
        .find(|(marker, _)| trimmed.starts_with(marker))?;
    // Past every marker character, so a four-backtick fence is a fence with no
    // info string rather than one in a language called `.
    let info = trimmed.trim_start_matches(fence).trim();
    Some((marker, info.split_whitespace().next()))
}

fn code_block(marker: &str, language: Option<&str>, lines: &mut Lines<'_>) -> Value {
    let mut inside = Vec::new();
    for line in lines.by_ref() {
        if line.trim().starts_with(marker) {
            break;
        }
        inside.push(line);
    }

    let text = inside.join("\n");
    // An ADF text node may not be empty, so a block with nothing in it carries
    // no content rather than an empty string.
    let content = if text.is_empty() {
        Vec::new()
    } else {
        vec![text_node(&text)]
    };
    match language {
        Some(language) => json!({
            "type": "codeBlock",
            "attrs": { "language": language },
            "content": content,
        }),
        None => json!({ "type": "codeBlock", "content": content }),
    }
}

// A space after the markers is required, which leaves `#` alone and `#tag` as
// paragraphs: an empty heading would need an empty text node, which ADF refuses.
fn heading(trimmed: &str) -> Option<(usize, &str)> {
    let rest = trimmed.trim_start_matches('#');
    let level = trimmed.len() - rest.len();
    if level == 0 || level > LEVELS {
        return None;
    }
    Some((level, rest.strip_prefix(' ')?.trim()))
}

// The three markers Markdown takes, and a numbered item, which is what a
// `### 1.` slice's prose sits under. A `- [ ] ` box is nothing special here:
// the box stays in the item's own text, because a `taskItem` on the board is
// something a person ticks and a criterion is read rather than worked.
fn item(trimmed: &str) -> Option<(&'static str, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some((BULLET, rest));
        }
    }
    let digits = trimmed.trim_start_matches(|character: char| character.is_ascii_digit());
    if digits.len() < trimmed.len() {
        return digits.strip_prefix(". ").map(|rest| (ORDERED, rest));
    }
    None
}

// A table row, a quote, raw HTML and an image: everything this writer does not
// model goes up as the line somebody typed.
fn source_text(trimmed: &str) -> bool {
    trimmed.starts_with(['|', '>', '<']) || trimmed.contains("![")
}

// Separate from `paragraph` on purpose, and not to be folded back into it: the
// fallback's text is the source itself, so whatever is read out of a paragraph's
// text is never read out of this one.
fn literal(line: &str) -> Value {
    json!({ "type": "paragraph", "content": [text_node(line)] })
}

fn paragraph(text: &str) -> Value {
    json!({ "type": "paragraph", "content": [text_node(text)] })
}

fn text_node(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

#[cfg(test)]
#[path = "tests/adf.rs"]
mod tests;
