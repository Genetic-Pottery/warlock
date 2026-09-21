use crate::document::Defect;

use super::{
    Accepted, BODY_CHARS, DRAFTS_PER_SLICE, Draft, Fill, REFERENCES_PER_LIST, TITLE_CHARS,
    TITLE_MINIMUM, accept, check, stub_answer,
};

const BODY: &str = "What the ticket asks for, and what it does not.";

fn drafted(drafts: Vec<Draft>) -> Fill {
    Fill { drafts }
}

fn sound(which: usize) -> Draft {
    Draft::of(format!("Draft number {which} of this slice"), BODY)
}

// Three sound drafts with the one under test third, so every field string a
// defect carries is `drafts[2]...` and an index read off the wrong list shows
// up as a mismatch rather than passing by luck.
fn third(draft: Draft) -> Fill {
    drafted(vec![sound(0), sound(1), draft])
}

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

#[test]
fn a_sound_answer_is_filled_and_a_defective_one_carries_its_defects() {
    let sound = drafted(vec![sound(0), sound(1)]).to_json();
    assert!(
        matches!(accept(&sound), Accepted::Filled(_)),
        "{:?}",
        accept(&sound)
    );

    let answer = format!(
        "Here is the object:\n\n{}\n\nThat is all.",
        third(Draft::of("", BODY)).to_json()
    );
    let Accepted::Defective { fill, defects } = accept(&answer) else {
        panic!("a defective fill is accepted with its defects, not thrown away");
    };
    assert_eq!(fill.drafts.len(), 3, "the fill survives its defects");
    assert_eq!(
        defects,
        vec![Defect::Empty {
            field: "drafts[2].title".to_owned()
        }]
    );

    let Accepted::Unparsed(defect) = accept("I could not do this.") else {
        panic!("an answer with no object in it is unparsed");
    };
    assert!(matches!(defect, Defect::NotJson { .. }), "{defect:?}");
}

#[test]
fn the_stub_answer_is_accepted_whole() {
    assert!(
        matches!(
            accept(&stub_answer("The drafting contract")),
            Accepted::Filled(_)
        ),
        "a test double's answer needs no repair"
    );
}

#[test]
fn an_empty_title_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(Draft::of("   ", BODY))),
        vec![Defect::Empty {
            field: "drafts[2].title".to_owned()
        }]
    );
}

#[test]
fn a_title_running_to_two_lines_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(Draft::of(
            "Parse the scope section\nand file it",
            BODY
        ))),
        vec![Defect::Multiline {
            field: "drafts[2].title".to_owned()
        }]
    );
}

#[test]
fn a_title_over_its_cap_is_reported_at_its_slot() {
    let title = "t".repeat(TITLE_CHARS + 1);
    assert_eq!(
        check(&third(Draft::of(title, BODY))),
        vec![Defect::TooLong {
            field: "drafts[2].title".to_owned(),
            chars: TITLE_CHARS + 1,
            cap: TITLE_CHARS,
        }]
    );
}

#[test]
fn a_title_under_the_minimum_is_reported_at_its_slot() {
    let title = "t".repeat(TITLE_MINIMUM - 1);
    assert_eq!(
        check(&third(Draft::of(title, BODY))),
        vec![Defect::TooShort {
            field: "drafts[2].title".to_owned(),
            chars: TITLE_MINIMUM - 1,
            minimum: TITLE_MINIMUM,
        }]
    );
}

#[test]
fn a_body_over_its_cap_is_reported_at_its_slot() {
    let body = "b".repeat(BODY_CHARS + 1);
    assert_eq!(
        check(&third(Draft::of("A draft with too much body", body))),
        vec![Defect::TooLong {
            field: "drafts[2].body".to_owned(),
            chars: BODY_CHARS + 1,
            cap: BODY_CHARS,
        }]
    );
}

#[test]
fn a_body_of_several_lines_is_no_defect() {
    let draft = Draft::of(
        "A draft with a written body",
        "One paragraph.\n\nAnd a second.",
    );
    assert_eq!(
        check(&third(draft)),
        Vec::new(),
        "only the title is one line"
    );
}

#[test]
fn a_slice_nobody_drafted_is_a_missing_array() {
    assert_eq!(
        check(&drafted(Vec::new())),
        vec![Defect::Missing {
            field: "drafts".to_owned()
        }]
    );
}

#[test]
fn more_drafts_than_the_cap_are_reported_against_the_array() {
    let fill = drafted((0..=DRAFTS_PER_SLICE).map(sound).collect());
    assert_eq!(
        check(&fill),
        vec![Defect::TooMany {
            field: "drafts".to_owned(),
            count: DRAFTS_PER_SLICE + 1,
            cap: DRAFTS_PER_SLICE,
        }]
    );
}

#[test]
fn more_references_than_the_list_cap_are_reported_against_the_list() {
    let mut draft = sound(2);
    draft.blocked_by = (0..=REFERENCES_PER_LIST).collect();
    draft.blocks = vec![0];
    assert_eq!(
        check(&third(draft)),
        vec![Defect::TooMany {
            field: "drafts[2].blocked_by".to_owned(),
            count: REFERENCES_PER_LIST + 1,
            cap: REFERENCES_PER_LIST,
        }],
        "the list over the cap is named, and an index outside the slice is the repair's business"
    );
}
