use serde_json::{Value, json};

use super::document_for;

// Every assertion goes through the one public conversion, so a node shape read
// here is the node shape a board would be sent.
fn blocks(body: &str) -> Vec<Value> {
    let document = document_for(body);
    assert_eq!(document["type"], "doc");
    document["content"]
        .as_array()
        .expect("a doc node's content")
        .clone()
}

fn paragraph(text: &str) -> Value {
    json!({ "type": "paragraph", "content": [{ "type": "text", "text": text }] })
}

#[test]
fn the_root_is_a_doc_node_carrying_a_version_and_its_content() {
    let document = document_for("Nothing converts Markdown yet.\n");

    assert_eq!(document["type"], "doc");
    assert_eq!(document["version"], 1);
    assert_eq!(
        document["content"],
        json!([paragraph("Nothing converts Markdown yet.")])
    );
}

#[test]
fn a_plain_line_is_a_paragraph_and_a_blank_line_is_nothing() {
    assert_eq!(
        blocks("One slice lands on its own.\n\nThe next needs it.\n"),
        vec![
            paragraph("One slice lands on its own."),
            paragraph("The next needs it."),
        ]
    );
}

#[test]
fn a_heading_carries_its_marker_count_as_its_level() {
    for level in 1..=6 {
        let body = format!("{} Scope\n", "#".repeat(level));

        assert_eq!(
            blocks(&body),
            vec![json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": [{ "type": "text", "text": "Scope" }],
            })],
            "{body}"
        );
    }
}

#[test]
fn a_line_that_is_not_a_heading_is_a_paragraph_of_its_own_text() {
    // Seven markers is past every level ADF has, and neither `#tag` nor a bare
    // marker is a heading with anything in it.
    for line in ["####### Scope", "#tag", "#", "###"] {
        assert_eq!(blocks(line), vec![paragraph(line)], "{line}");
    }
}

#[test]
fn consecutive_items_of_one_marker_are_one_bullet_list() {
    for marker in ['-', '*', '+'] {
        let body = format!("{marker} sees a URL\n{marker} sees a key\n");

        assert_eq!(
            blocks(&body),
            vec![json!({
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [paragraph("sees a URL")] },
                    { "type": "listItem", "content": [paragraph("sees a key")] },
                ],
            })],
            "{body}"
        );
    }
}

#[test]
fn a_numbered_item_is_an_ordered_list() {
    assert_eq!(
        blocks("1. Read the file\n2. File the Epic\n"),
        vec![json!({
            "type": "orderedList",
            "content": [
                { "type": "listItem", "content": [paragraph("Read the file")] },
                { "type": "listItem", "content": [paragraph("File the Epic")] },
            ],
        })]
    );
}

#[test]
fn a_blank_line_between_two_items_leaves_them_in_one_list() {
    let joined = blocks("- sees a URL\n\n- sees a key\n");

    assert_eq!(joined.len(), 1, "{joined:?}");
    assert_eq!(joined[0]["type"], "bulletList");
    assert_eq!(
        joined[0]["content"]
            .as_array()
            .expect("the list's items")
            .len(),
        2
    );
}

#[test]
fn a_list_closes_at_anything_that_is_not_an_item_of_the_same_kind() {
    let kinds: Vec<Value> = blocks(
        "- a bullet\n1. a number\n- another bullet\n## Scope\n- a last bullet\nprose\n- and one more\n",
    )
    .into_iter()
    .map(|block| block["type"].clone())
    .collect();

    assert_eq!(
        kinds,
        vec![
            json!("bulletList"),
            json!("orderedList"),
            json!("bulletList"),
            json!("heading"),
            json!("bulletList"),
            json!("paragraph"),
            json!("bulletList"),
        ]
    );
}

#[test]
fn a_fence_carries_its_lines_verbatim_under_its_info_strings_language() {
    assert_eq!(
        blocks("```rust ignore\nfn main() {\n    let brief = 1;\n\n}\n```\n"),
        vec![json!({
            "type": "codeBlock",
            "attrs": { "language": "rust" },
            "content": [{
                "type": "text",
                "text": "fn main() {\n    let brief = 1;\n\n}",
            }],
        })]
    );
}

#[test]
fn a_fence_with_no_info_string_carries_no_language() {
    for body in [
        "```\nwarlock push\n```\n",
        "~~~\nwarlock push\n~~~\n",
        "````\nwarlock push\n````\n",
    ] {
        let block = blocks(body).remove(0);

        assert_eq!(block["type"], "codeBlock", "{body}");
        assert!(block.get("attrs").is_none(), "{block:?}");
        assert_eq!(
            block["content"],
            json!([{ "type": "text", "text": "warlock push" }]),
            "{body}"
        );
    }
}

#[test]
fn a_fence_is_closed_only_by_its_own_marker() {
    assert_eq!(
        blocks("```\n~~~\nstill inside\n```\nafter\n"),
        vec![
            json!({
                "type": "codeBlock",
                "content": [{ "type": "text", "text": "~~~\nstill inside" }],
            }),
            paragraph("after"),
        ]
    );
}

#[test]
fn an_unterminated_fence_is_the_rest_of_the_body() {
    assert_eq!(
        blocks("## Scope\n\n```sh\nwarlock push pay docs/brief.md\n"),
        vec![
            json!({
                "type": "heading",
                "attrs": { "level": 2 },
                "content": [{ "type": "text", "text": "Scope" }],
            }),
            json!({
                "type": "codeBlock",
                "attrs": { "language": "sh" },
                "content": [{ "type": "text", "text": "warlock push pay docs/brief.md" }],
            }),
        ]
    );
}

#[test]
fn an_empty_fence_holds_no_text_node() {
    // An ADF text node may not be empty, so the block is empty instead.
    assert_eq!(
        blocks("```\n```\n"),
        vec![json!({ "type": "codeBlock", "content": [] })]
    );
}

#[test]
fn a_construct_outside_the_subset_keeps_its_source_text() {
    for line in [
        "| slice | depends_on |",
        "|---|---|",
        "> A brief is the document.",
        "![the panel](assets/panel.png)",
        "<kbd>Enter</kbd>",
        "see ![the panel](assets/panel.png) above",
    ] {
        assert_eq!(blocks(line), vec![paragraph(line)], "{line}");
    }
}

#[test]
fn every_row_of_a_table_is_a_paragraph_of_its_own() {
    assert_eq!(
        blocks("| slice | depends_on |\n|---|---|\n| 1 | [] |\n"),
        vec![
            paragraph("| slice | depends_on |"),
            paragraph("|---|---|"),
            paragraph("| 1 | [] |"),
        ]
    );
}

#[test]
fn a_criterion_keeps_its_box_and_no_task_node_is_written() {
    let document =
        document_for("- [ ] A new module is declared in `lib.rs`.\n- [x] It is tested.\n");

    assert_eq!(
        document["content"],
        json!([{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [paragraph("[ ] A new module is declared in `lib.rs`.")],
                },
                { "type": "listItem", "content": [paragraph("[x] It is tested.")] },
            ],
        }])
    );
    let written = document.to_string();
    assert!(!written.contains("taskList"), "{written}");
    assert!(!written.contains("taskItem"), "{written}");
}

#[test]
fn a_body_with_nothing_in_it_is_a_doc_with_nothing_in_it() {
    for body in ["", "\n", "\n\n", "   ", "   \n\n\t\n"] {
        assert_eq!(blocks(body), Vec::<Value>::new(), "{body:?}");
    }
}

#[test]
fn the_conversion_names_no_board_and_no_transport() {
    // The tree is a function of the text and of nothing else: a type from a
    // board or from the wire in here is how which documents warlock can convert
    // starts depending on where they are being filed.
    let source = include_str!("../adf.rs").to_lowercase();
    for word in ["ureq", "http", "crate::", "client", "newbrief"] {
        assert!(!source.contains(word), "adf.rs mentions {word:?}");
    }
}
