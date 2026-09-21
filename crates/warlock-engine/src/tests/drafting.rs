use super::{
    BODY_CHARS, DRAFTS_PER_SLICE, Draft, Fill, REFERENCES_PER_LIST, TITLE_CHARS, TITLE_MINIMUM,
    stub_answer,
};

#[test]
fn a_bare_string_where_a_draft_was_asked_for_is_read_as_a_draft() {
    let answer = r#"{
        "drafts": [
            {
                "title": "Add the Linear client",
                "body": "One request, one timeout, no retry.",
                "blocked_by": [],
                "blocks": [2]
            },
            "Parse the scope section",
            {"title": "File the drafts"}
        ]
    }"#;

    let fill: Fill = serde_json::from_str(answer)
        .expect("a bare string is a repairable draft and never Defect::NotJson");

    assert_eq!(fill.drafts.len(), 3);
    assert_eq!(
        fill.drafts[1],
        Draft {
            title: "Parse the scope section".to_owned(),
            body: String::new(),
            blocked_by: Vec::new(),
            blocks: Vec::new(),
        },
        "the string carries the title and the lists come back empty"
    );
    assert_eq!(fill.drafts[0].blocks, vec![2]);
    assert_eq!(fill.drafts[2].title, "File the drafts");
    assert!(fill.drafts[2].body.is_empty());
}

#[test]
fn the_stub_answer_is_a_fill_inside_every_cap() {
    let answer = stub_answer("The drafting contract");
    let fill: Fill = serde_json::from_str(&answer).expect("the stub is the contract's own shape");

    assert!(fill.drafts.len() <= DRAFTS_PER_SLICE);
    for draft in &fill.drafts {
        let title = draft.title.chars().count();
        assert!((TITLE_MINIMUM..=TITLE_CHARS).contains(&title), "{title}");
        assert!(!draft.title.contains(['\n', '\r']));
        assert!(!draft.body.is_empty() && draft.body.chars().count() <= BODY_CHARS);
        assert!(draft.blocked_by.len() <= REFERENCES_PER_LIST);
        assert!(draft.blocks.len() <= REFERENCES_PER_LIST);
        assert!(
            draft.blocked_by.iter().chain(&draft.blocks).all(|index| {
                *index < fill.drafts.len() && fill.drafts[*index].title != draft.title
            }),
            "a reference points at another draft of this slice"
        );
    }
    assert!(
        fill.drafts
            .iter()
            .any(|draft| draft.title.contains("The drafting contract"))
    );
}
