use serde_json::{Value, json};

use super::{body_for, document_for};

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

// The reader's side. Every assertion goes through the one public conversion
// again, so the tree read here is a tree a board could hand back.
fn read(content: &Value) -> String {
    body_for(&json!({ "type": "doc", "version": 1, "content": content }))
}

fn text(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn marked(text: &str, mark: &Value) -> Value {
    json!({ "type": "text", "text": text, "marks": [mark] })
}

#[test]
fn a_paragraph_is_one_line_and_two_are_separated_by_a_blank_one() {
    assert_eq!(
        read(&json!([
            paragraph("One slice lands on its own."),
            paragraph("The next needs it."),
        ])),
        "One slice lands on its own.\n\nThe next needs it.\n"
    );
}

#[test]
fn a_headings_level_is_read_back_as_that_many_markers() {
    for level in 1..=6 {
        assert_eq!(
            read(&json!([{
                "type": "heading",
                "attrs": { "level": level },
                "content": [text("Scope")],
            }])),
            format!("{} Scope\n", "#".repeat(level)),
            "{level}"
        );
    }
}

#[test]
fn a_heading_level_outside_the_six_is_pulled_back_inside_them() {
    // Not a refusal and not seven markers: a level ADF cannot hold becomes the
    // nearest one it can, so the heading is still a heading.
    for (level, markers) in [(json!(0), "#"), (json!(9), "######"), (json!("two"), "#")] {
        assert_eq!(
            read(&json!([{
                "type": "heading",
                "attrs": { "level": level },
                "content": [text("Scope")],
            }])),
            format!("{markers} Scope\n"),
            "{level}"
        );
    }
}

#[test]
fn a_heading_with_no_attrs_is_a_first_level_heading() {
    assert_eq!(
        read(&json!([{ "type": "heading", "content": [text("Scope")] }])),
        "# Scope\n"
    );
}

#[test]
fn a_bullet_list_reads_back_as_dashed_items_on_consecutive_lines() {
    assert_eq!(
        read(&json!([{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [paragraph("sees a URL")] },
                { "type": "listItem", "content": [paragraph("sees a key")] },
            ],
        }])),
        "- sees a URL\n- sees a key\n"
    );
}

#[test]
fn an_ordered_list_reads_back_numbered_from_one() {
    assert_eq!(
        read(&json!([{
            "type": "orderedList",
            "content": [
                { "type": "listItem", "content": [paragraph("Read the file")] },
                { "type": "listItem", "content": [paragraph("File the Epic")] },
                { "type": "listItem", "content": [paragraph("Cut the slices")] },
            ],
        }])),
        "1. Read the file\n2. File the Epic\n3. Cut the slices\n"
    );
}

#[test]
fn a_code_block_reads_back_fenced_under_its_language() {
    assert_eq!(
        read(&json!([{
            "type": "codeBlock",
            "attrs": { "language": "rust" },
            "content": [text("fn main() {\n    let brief = 1;\n\n}")],
        }])),
        "```rust\nfn main() {\n    let brief = 1;\n\n}\n```\n"
    );
}

#[test]
fn a_code_block_with_no_language_and_no_content_is_still_a_fence() {
    assert_eq!(
        read(&json!([{ "type": "codeBlock", "content": [text("warlock push")] }])),
        "```\nwarlock push\n```\n"
    );
    assert_eq!(read(&json!([{ "type": "codeBlock" }])), "```\n```\n");
}

#[test]
fn a_code_blocks_text_is_not_marked_up_again() {
    // A mark in here is somebody else's edit, and backticks around a line
    // inside a fence would be characters the block did not hold.
    assert_eq!(
        read(&json!([{
            "type": "codeBlock",
            "content": [marked("warlock push", &json!({ "type": "code" }))],
        }])),
        "```\nwarlock push\n```\n"
    );
}

#[test]
fn the_three_marks_read_back_as_the_characters_they_were_written_as() {
    assert_eq!(
        read(&json!([{
            "type": "paragraph",
            "content": [
                text("The brief is "),
                marked("the document", &json!({ "type": "strong" })),
                text(", and "),
                marked("push.rs", &json!({ "type": "code" })),
                text(" files it; see "),
                marked(
                    "the brief",
                    &json!({ "type": "link", "attrs": { "href": "docs/brief.md" } }),
                ),
                text("."),
            ],
        }])),
        "The brief is **the document**, and `push.rs` files it; see [the brief](docs/brief.md).\n"
    );
}

#[test]
fn a_mark_the_reader_does_not_know_leaves_its_text_alone() {
    for mark in [
        json!({ "type": "underline" }),
        json!({ "type": "link" }),
        json!({ "type": "link", "attrs": { "href": "" } }),
    ] {
        assert_eq!(
            read(&json!([{ "type": "paragraph", "content": [marked("plain", &mark)] }])),
            "plain\n",
            "{mark}"
        );
    }
}

#[test]
fn marks_on_one_node_nest_in_the_order_they_are_listed() {
    assert_eq!(
        read(&json!([{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "cut",
                "marks": [{ "type": "code" }, { "type": "strong" }],
            }],
        }])),
        "**`cut`**\n"
    );
}

// A tree no warlock code path writes: panels, a table, media, a mention and an
// emoji are what an edit in the board's own UI leaves behind.
fn edited_elsewhere() -> Value {
    json!([
        {
            "type": "panel",
            "attrs": { "panelType": "info" },
            "content": [
                paragraph("A brief is the document."),
                paragraph("Everything else follows from it."),
            ],
        },
        {
            "type": "table",
            "attrs": { "isNumberColumnEnabled": false },
            "content": [{
                "type": "tableRow",
                "content": [
                    { "type": "tableHeader", "content": [paragraph("slice")] },
                    { "type": "tableCell", "content": [paragraph("waits on")] },
                ],
            }],
        },
        {
            "type": "mediaSingle",
            "attrs": { "layout": "center" },
            "content": [{ "type": "media", "attrs": { "id": "a1", "collection": "b2" } }],
        },
        {
            "type": "paragraph",
            "content": [
                text("asked "),
                { "type": "mention", "attrs": { "id": "u1", "text": "@someone" } },
                text("to look again"),
                { "type": "emoji", "attrs": { "shortName": ":white_check_mark:", "text": "y" } },
            ],
        },
    ])
}

#[test]
fn an_unknown_node_contributes_the_text_under_it_and_nothing_else() {
    assert_eq!(
        read(&edited_elsewhere()),
        "A brief is the document. Everything else follows from it.\n\nslice waits on\n\nasked to look again\n"
    );
}

#[test]
fn an_unknown_node_leaves_no_name_no_attribute_and_no_marker_behind() {
    let body = read(&edited_elsewhere());

    for name in [
        "panel",
        "table",
        "tableRow",
        "tableHeader",
        "tableCell",
        "mediaSingle",
        "media",
        "mention",
        "emoji",
        "paragraph",
        "type",
        "attrs",
        "content",
        "panelType",
        "layout",
        "shortName",
        "collection",
        "isNumberColumnEnabled",
    ] {
        assert!(!body.contains(name), "{name:?} survived into {body:?}");
    }
    for marker in ["|", "#", "```", "- ", "1. ", "[", "*"] {
        assert!(!body.contains(marker), "{marker:?} survived into {body:?}");
    }
}

#[test]
fn a_tree_that_is_not_a_document_is_read_for_whatever_it_holds() {
    // The root's `type` is not checked, so a paragraph handed back as a whole
    // document is still that paragraph's line.
    assert_eq!(
        body_for(&json!({
            "type": "paragraph",
            "content": [text("Just the one line.")],
        })),
        "Just the one line.\n"
    );
    assert_eq!(
        body_for(&json!({ "content": [paragraph("No type at all.")] })),
        "No type at all.\n"
    );
}

#[test]
fn a_malformed_tree_reads_back_as_nothing_at_all() {
    for tree in [
        json!({}),
        json!({ "type": "doc" }),
        json!({ "type": "doc", "version": 1, "content": [] }),
        json!({ "type": "doc", "content": "## Scope" }),
        json!(null),
        json!("doc"),
        json!(7),
        json!([paragraph("in an array, not a document")]),
    ] {
        assert_eq!(body_for(&tree), "", "{tree}");
    }
}

#[test]
fn a_node_with_nothing_under_it_contributes_no_line() {
    // Including a heading: markers with nothing after them are not a heading,
    // and an empty line between two blocks is one the writer would read as a
    // separator.
    assert_eq!(
        read(&json!([
            paragraph("above"),
            { "type": "heading", "attrs": { "level": 2 } },
            { "type": "paragraph" },
            { "type": "paragraph", "content": [] },
            { "type": "bulletList" },
            { "type": "orderedList", "content": [{ "type": "listItem" }] },
            { "type": "panel" },
            { "type": "text", "text": "" },
            "a string where a node should be",
            json!(null),
            json!([]),
            paragraph("below"),
        ])),
        "above\n\nbelow\n"
    );
}

#[test]
fn an_item_with_nothing_in_it_does_not_take_a_number() {
    assert_eq!(
        read(&json!([{
            "type": "orderedList",
            "content": [
                { "type": "listItem", "content": [paragraph("first")] },
                { "type": "listItem" },
                { "type": "listItem", "content": [paragraph("second")] },
            ],
        }])),
        "1. first\n2. second\n"
    );
}

// Both conversions, in the order a brief meets them: out to the board as a tree
// and back off it as the text `cut` parses. A body written one block to a
// paragraph, with a blank line between blocks, is what the reader emits, so the
// round trip of one is the text itself.
fn round_trip(body: &str) -> String {
    body_for(&document_for(body))
}

#[test]
fn every_construct_in_the_subset_comes_back_as_the_markdown_it_went_out_as() {
    let body = "\
# Problem

Nothing converts **Markdown** to `ADF`, so see [the API](https://example.test/v3) first.

## Acceptance criteria

- sees a URL
- sees a key

### Out of scope

1. Read the file
2. File the Epic

```rust
fn main() {}
```

#### A fourth level

##### A fifth

###### And a sixth
";

    assert_eq!(round_trip(body), body);
}

#[test]
fn each_of_the_three_marks_comes_back_as_the_characters_it_was_written_as() {
    for line in [
        "The brief is **the document**.",
        "`push.rs` hands it over",
        "see [the API](https://example.test/v3) first",
        "**one** and `two` and [three](https://example.test)",
        "## The `doc` node",
        "- [ ] `adf.rs` is **declared**",
    ] {
        assert_eq!(
            round_trip(&format!("{line}\n")),
            format!("{line}\n"),
            "{line}"
        );
    }
}

#[test]
fn a_fence_comes_back_with_its_markers_once_and_its_own_lines_unchanged() {
    // The backticks inside are the point of counting them: a `code` mark
    // applied to a block's text, or a second fence around a block that already
    // carried one, is how a block comes back holding characters it never held.
    let inside = "let marked = `not code`;\n\nwarlock push --scope pay";
    let body = format!("```rust\n{inside}\n```\n");

    let read = round_trip(&body);

    assert_eq!(read, body);
    assert_eq!(read.matches("```").count(), 2, "{read:?}");
    assert_eq!(read.lines().next(), Some("```rust"));
    assert_eq!(read.lines().next_back(), Some("```"));
    for line in inside.lines() {
        assert!(read.contains(line), "{line:?} changed in {read:?}");
    }
}

#[test]
fn a_criterion_comes_back_holding_its_box_and_one_marker() {
    let body = "- [ ] A new module is declared.\n- [x] It is tested.\n";

    assert_eq!(round_trip(body), body);
}

// A brief holding the three things `cut` reads off the board: the scope
// heading, a `### N. ` heading per slice, and a slice's dependency line.
fn brief_body() -> String {
    [
        "Nothing walks an ADF tree back to Markdown.",
        "",
        "## Acceptance criteria",
        "",
        "- [ ] `body_for` does not return `Result`.",
        "",
        "## Scope",
        "",
        "### 1. Walk the tree back",
        "",
        "Read `heading`, `paragraph` and both list kinds.",
        "",
        "### 2. Round-trip a brief",
        "",
        "depends_on: [1]",
        "",
        "Assert that **both** directions compose.",
        "",
    ]
    .join("\n")
}

#[test]
fn a_brief_still_parses_after_both_conversions() {
    let body = brief_body();
    let read = round_trip(&body);
    assert_eq!(read, body);

    let block = crate::brief::scope_block_in(&read).expect("a scope block");

    assert!(
        block.brief().ends_with("does not return `Result`."),
        "{:?}",
        block.brief()
    );
    let headings: Vec<&str> = block
        .slices()
        .iter()
        .map(crate::brief::Slice::heading)
        .collect();
    assert_eq!(headings, vec!["Walk the tree back", "Round-trip a brief"]);
    assert_eq!(block.slices()[0].depends_on(), [0usize; 0]);
    assert_eq!(block.slices()[1].depends_on(), [1]);
    assert_eq!(block.unreadable(), 0);
}

#[test]
fn a_briefs_criterion_and_its_dependency_line_keep_their_brackets() {
    let read = round_trip(&brief_body());

    // No link mark fired on either, and neither bracket came back escaped: a
    // `\[` here is a Linear storage quirk that `depended` undoes, and nothing
    // this conversion is allowed to introduce.
    assert!(
        read.contains("- [ ] `body_for` does not return `Result`."),
        "{read:?}"
    );
    assert!(read.contains("\ndepends_on: [1]\n"), "{read:?}");
    assert!(!read.contains('\\'), "{read:?}");
}
