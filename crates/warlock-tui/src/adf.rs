use std::str::Lines;

use serde_json::{Value, json};

// ADF's own wire field, not warlock's: every document Jira's v3 API accepts
// carries 1, and a tree sent with anything else is refused whole.
const VERSION: u8 = 1;

const BULLET: &str = "bulletList";
const ORDERED: &str = "orderedList";

const STRONG: &str = "**";
const CODE: &str = "`";
const FENCE: &str = "```";

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
                "content": inline(text),
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
// fallback's text is the source itself, so a `**` or a `[a](b)` inside a table
// row or a quote stays the characters that were written rather than becoming a
// mark over part of a line nothing else on the board understands.
fn literal(line: &str) -> Value {
    json!({ "type": "paragraph", "content": [text_node(line)] })
}

fn paragraph(text: &str) -> Value {
    json!({ "type": "paragraph", "content": inline(text) })
}

// `text` split into the marked and unmarked runs it is written as. A construct
// left open is none: its characters stay in the run around it, which is what
// keeps the conversion total.
fn inline(text: &str) -> Vec<Value> {
    let mut nodes = Vec::new();
    let mut plain = 0;
    let mut at = 0;

    while at < text.len() {
        if let Some((width, node)) = marked(&text[at..]) {
            if plain < at {
                nodes.push(text_node(&text[plain..at]));
            }
            nodes.push(node);
            at += width;
            plain = at;
        } else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }

    if plain < text.len() {
        nodes.push(text_node(&text[plain..]));
    }
    nodes
}

// A span's body is one text node under one mark and is not scanned again, so
// marks never nest and never combine: `**a `b`**` is strong over all of it.
fn marked(rest: &str) -> Option<(usize, Value)> {
    if let Some((width, body)) = delimited(rest, STRONG) {
        return Some((width, marked_text(body, &json!({ "type": "strong" }))));
    }
    if let Some((width, body)) = delimited(rest, CODE) {
        return Some((width, marked_text(body, &json!({ "type": "code" }))));
    }
    link(rest)
}

// The delimiters are ASCII, so every offset `find` hands back is a character
// boundary.
fn delimited<'a>(rest: &'a str, delimiter: &str) -> Option<(usize, &'a str)> {
    let body = rest.strip_prefix(delimiter)?;
    let end = body.find(delimiter)?;
    // An ADF text node may not be empty, so `****` is four characters somebody
    // typed rather than a mark over nothing.
    if end == 0 {
        return None;
    }
    Some((delimiter.len() * 2 + end, &body[..end]))
}

fn link(rest: &str) -> Option<(usize, Value)> {
    let body = rest.strip_prefix('[')?;
    // `](` together, which leaves a criterion's `[ ] ` box and a bare `[]` as
    // the characters they are.
    let close = body.find("](")?;
    let target = &body[close + 2..];
    let end = target.find(')')?;

    let (text, href) = (&body[..close], &target[..end]);
    if text.is_empty() || href.is_empty() {
        return None;
    }
    let mark = json!({ "type": "link", "attrs": { "href": href } });
    Some((close + end + "[]()".len(), marked_text(text, &mark)))
}

fn text_node(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn marked_text(text: &str, mark: &Value) -> Value {
    json!({ "type": "text", "text": text, "marks": [mark] })
}

/// `document` as Markdown.
///
/// Total, and that is the point: every field is read defensively because the
/// tree may have been edited where warlock cannot see it, and a node type this
/// does not model contributes the text under it and nothing else. So a brief
/// somebody wrapped in a panel comes back plainer rather than not at all.
///
/// ```
/// use warlock_tui::body_for;
///
/// let body = body_for(&serde_json::json!({
///     "type": "doc",
///     "version": 1,
///     "content": [{
///         "type": "heading",
///         "attrs": { "level": 2 },
///         "content": [{ "type": "text", "text": "Scope" }],
///     }],
/// }));
///
/// assert_eq!(body, "## Scope\n");
/// ```
#[must_use]
pub fn body_for(document: &Value) -> String {
    // The root's own type is not checked, and a tree rooted at anything is read
    // for its children: refusing a root that does not say `doc` would turn one
    // wrong field into a brief with no headings.
    let mut written: Vec<String> = Vec::new();
    for block in content_of(document) {
        let block = block_of(block);
        if !block.is_empty() {
            written.push(block);
        }
    }

    if written.is_empty() {
        return String::new();
    }
    // Blank lines between blocks and a newline at the end, because the one
    // consumer is a Markdown parse: a heading on the line after a paragraph is
    // still a heading, but a list item on the line after one is that
    // paragraph's own text.
    written.join("\n\n") + "\n"
}

fn block_of(block: &Value) -> String {
    match kind_of(block) {
        "heading" => {
            let text = text_of(block);
            if text.is_empty() {
                // A heading's markers with nothing after them are not a
                // heading, here or in the writer above.
                String::new()
            } else {
                format!("{} {text}", "#".repeat(level_of(block)))
            }
        }
        BULLET => listed(block, false),
        ORDERED => listed(block, true),
        "codeBlock" => fence_of(block),
        // `paragraph` and every type warlock does not model land here together:
        // one line of the text underneath, carrying no trace of the node it came
        // out of.
        _ => text_of(block),
    }
}

// A `N.` marker is the reader's own count of the items it kept, not a number
// read out of the tree: ADF carries where a list starts and nothing per item,
// and the one consumer reads a slice's number from its heading.
fn listed(list: &Value, numbered: bool) -> String {
    let mut items: Vec<String> = Vec::new();
    for item in content_of(list) {
        let text = text_of(item);
        if text.is_empty() {
            continue;
        }
        if numbered {
            items.push(format!("{}. {text}", items.len() + 1));
        } else {
            items.push(format!("- {text}"));
        }
    }
    items.join("\n")
}

fn fence_of(block: &Value) -> String {
    let language = attribute_of(block, "language").unwrap_or_default();
    let fence = format!("{FENCE}{language}");
    // Text under a code block is what somebody typed, so no mark is applied to
    // it: a `code` mark in here would come back as backticks inside the block.
    let inside = source_of(block);
    if inside.is_empty() {
        format!("{fence}\n{FENCE}")
    } else {
        format!("{fence}\n{inside}\n{FENCE}")
    }
}

fn source_of(node: &Value) -> String {
    let mut text = String::new();
    for child in content_of(node) {
        match child.get("text").and_then(Value::as_str) {
            Some(part) => text.push_str(part),
            None => text.push_str(&source_of(child)),
        }
    }
    text
}

fn text_of(node: &Value) -> String {
    if kind_of(node) == "text" {
        return inlined(node);
    }

    let mut text = String::new();
    for child in content_of(node) {
        let part = text_of(child);
        if part.is_empty() {
            continue;
        }
        // Text nodes are runs of one line and join as they were written. A child
        // that is not one was a block of its own, and joining those bare would
        // run the last word of a panel's first paragraph into its second.
        if !text.is_empty() && kind_of(child) != "text" {
            text.push(' ');
        }
        text.push_str(&part);
    }
    text
}

fn inlined(node: &Value) -> String {
    let mut text = node
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if text.is_empty() {
        return text;
    }

    // The writer puts one mark on a node, so the order only decides how a tree
    // from elsewhere nests: innermost first, as the marks are listed.
    for mark in node
        .get("marks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        text = match kind_of(mark) {
            "strong" => format!("{STRONG}{text}{STRONG}"),
            "code" => format!("{CODE}{text}{CODE}"),
            "link" => match attribute_of(mark, "href") {
                Some(href) if !href.is_empty() => format!("[{text}]({href})"),
                _ => text,
            },
            _ => text,
        };
    }
    text
}

fn kind_of(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn level_of(heading: &Value) -> usize {
    heading
        .get("attrs")
        .and_then(|attrs| attrs.get("level"))
        .and_then(Value::as_u64)
        .map_or(1, |level| {
            usize::try_from(level).unwrap_or(LEVELS).clamp(1, LEVELS)
        })
}

fn attribute_of<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    node.get("attrs")
        .and_then(|attrs| attrs.get(name))
        .and_then(Value::as_str)
}

fn content_of(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

#[cfg(test)]
#[path = "tests/adf.rs"]
mod tests;
