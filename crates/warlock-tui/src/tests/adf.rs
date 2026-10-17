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

// The marks of the one paragraph a body of `text` converts to.
fn marks(text: &str) -> Value {
    blocks(text).remove(0)["content"].clone()
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
    let document = document_for("- [ ] A new module is declared.\n- [x] It is tested.\n");

    assert_eq!(
        document["content"],
        json!([{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [paragraph("[ ] A new module is declared.")],
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
fn doubled_asterisks_are_one_text_node_under_the_strong_mark() {
    assert_eq!(
        marks("The brief is **the document**."),
        json!([
            { "type": "text", "text": "The brief is " },
            {
                "type": "text",
                "text": "the document",
                "marks": [{ "type": "strong" }],
            },
            { "type": "text", "text": "." },
        ])
    );
}

#[test]
fn backticked_text_is_one_text_node_under_the_code_mark() {
    assert_eq!(
        marks("`push.rs` hands it over"),
        json!([
            { "type": "text", "text": "push.rs", "marks": [{ "type": "code" }] },
            { "type": "text", "text": " hands it over" },
        ])
    );
}

#[test]
fn a_link_is_one_text_node_under_a_link_mark_carrying_its_href() {
    assert_eq!(
        marks("see [the API](https://example.test/v3) first"),
        json!([
            { "type": "text", "text": "see " },
            {
                "type": "text",
                "text": "the API",
                "marks": [{
                    "type": "link",
                    "attrs": { "href": "https://example.test/v3" },
                }],
            },
            { "type": "text", "text": " first" },
        ])
    );
}

#[test]
fn a_heading_and_a_list_item_carry_marks_of_their_own() {
    assert_eq!(
        blocks("## The `doc` node\n- [ ] `adf.rs` is declared\n"),
        vec![
            json!({
                "type": "heading",
                "attrs": { "level": 2 },
                "content": [
                    { "type": "text", "text": "The " },
                    { "type": "text", "text": "doc", "marks": [{ "type": "code" }] },
                    { "type": "text", "text": " node" },
                ],
            }),
            json!({
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "[ ] " },
                            {
                                "type": "text",
                                "text": "adf.rs",
                                "marks": [{ "type": "code" }],
                            },
                            { "type": "text", "text": " is declared" },
                        ],
                    }],
                }],
            }),
        ]
    );
}

#[test]
fn several_marks_on_one_line_are_each_their_own_node() {
    let nodes = marks("**one** and `two` and [three](https://example.test)");
    let written: Vec<&Value> = nodes
        .as_array()
        .expect("the paragraph's text nodes")
        .iter()
        .collect();

    assert_eq!(written.len(), 5, "{nodes}");
    assert_eq!(written[0]["marks"][0]["type"], "strong");
    assert_eq!(written[2]["marks"][0]["type"], "code");
    assert_eq!(written[4]["marks"][0]["type"], "link");
}

#[test]
fn a_marks_body_is_not_scanned_again() {
    // One mark to a node: a code span inside strong is strong over the whole
    // run, backticks and all.
    assert_eq!(
        marks("**the `doc` node**"),
        json!([{
            "type": "text",
            "text": "the `doc` node",
            "marks": [{ "type": "strong" }],
        }])
    );
}

#[test]
fn an_unclosed_or_empty_construct_is_the_characters_it_was_written_as() {
    for line in [
        "**unclosed and alone",
        "a stray ` backtick",
        "an *emphasis* is not a mark",
        "****",
        "``",
        "[](https://example.test)",
        "[no target]()",
        "[a reference] and (a parenthesis)",
        "100% of **",
    ] {
        assert_eq!(blocks(line), vec![paragraph(line)], "{line}");
    }
}

#[test]
fn a_fence_and_the_literal_fallback_are_not_scanned_for_marks() {
    let fenced = blocks("```\n**not strong** and `not code`\n```\n").remove(0);

    assert_eq!(fenced["type"], "codeBlock");
    assert_eq!(
        fenced["content"],
        json!([{ "type": "text", "text": "**not strong** and `not code`" }])
    );

    for line in [
        "| **a** | `b` |",
        "> see [the API](https://example.test)",
        "<kbd>**Enter**</kbd>",
    ] {
        assert_eq!(blocks(line), vec![paragraph(line)], "{line}");
    }
}

#[test]
fn a_mark_around_text_that_is_not_ascii_keeps_its_characters() {
    assert_eq!(
        marks("a **brief — ünchanged** ✓"),
        json!([
            { "type": "text", "text": "a " },
            {
                "type": "text",
                "text": "brief — ünchanged",
                "marks": [{ "type": "strong" }],
            },
            { "type": "text", "text": " ✓" },
        ])
    );
}

#[test]
fn the_writer_emits_six_node_types_and_three_marks_and_no_others() {
    let document = document_for(
        "# Problem\n\nNothing converts **Markdown** to `ADF`.\n\n- a bullet\n1. a number\n\n```rust\nfn main() {}\n```\n\n| a | table |\n\nsee [the API](https://example.test).\n",
    );

    let mut kinds: Vec<String> = Vec::new();
    collect(&document, &mut kinds);
    kinds.sort_unstable();
    kinds.dedup();

    assert_eq!(
        kinds,
        vec![
            "bulletList",
            "code",
            "codeBlock",
            "doc",
            "heading",
            "link",
            "listItem",
            "orderedList",
            "paragraph",
            "strong",
            "text",
        ]
    );
}

// Every `type` anywhere in the tree, so a node or a mark added past the six and
// the three fails the count above wherever it is written.
fn collect(value: &Value, kinds: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::String(kind)) = fields.get("type") {
                kinds.push(kind.clone());
            }
            for field in fields.values() {
                collect(field, kinds);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, kinds);
            }
        }
        _ => {}
    }
}

#[test]
fn a_body_with_nothing_in_it_is_a_doc_with_nothing_in_it() {
    for body in ["", "\n", "\n\n", "   ", "   \n\n\t\n"] {
        assert_eq!(blocks(body), Vec::<Value>::new(), "{body:?}");
    }
}

// The scanner walks byte offsets, where a slice taken mid-character panics and
// an offset past the end panics. Every short arrangement of the characters the
// marks are made of is cheaper than reasoning about which ones can.
#[test]
fn no_arrangement_of_the_mark_characters_panics() {
    const ALPHABET: [char; 11] = ['*', '`', '[', ']', '(', ')', '#', '-', 'a', '—', '\n'];

    let mut bodies: Vec<String> = vec![String::new()];
    for _ in 0..4 {
        let mut longer = Vec::with_capacity(bodies.len() * ALPHABET.len());
        for body in &bodies {
            for character in ALPHABET {
                longer.push(format!("{body}{character}"));
            }
        }
        bodies = longer;

        for body in &bodies {
            let document = document_for(body);
            assert_eq!(document["version"], 1, "{body:?}");
        }
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
