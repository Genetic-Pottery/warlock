use std::collections::BTreeSet;

use crate::document::Defect;

use super::{
    Accepted, BODY_CHARS, DRAFTING_PROMPT, DRAFTS_PER_SLICE, Draft, Fill, MEND_PASSES, Mend,
    Mended, REFERENCES_PER_LIST, TITLE_CHARS, TITLE_MINIMUM, accept, check, drafting_instructions,
    mend, mended, stub_answer,
};

const BODY: &str = "What the ticket asks for, and what it does not.";

const SLICE: &str = "The drafting contract";

const PROSE: &str = "A slice's drafts, filled by a pass and checked by warlock.";

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

#[test]
fn the_prompt_states_every_cap_it_is_checked_against() {
    // Each number in the phrase that carries it, not on its own: three of the
    // caps are 12, so a bare `contains("12")` would pass for all three with one
    // of them written into the prose and the other two forgotten.
    for stated in [
        format!("at most {DRAFTS_PER_SLICE} entries"),
        format!("between {TITLE_MINIMUM} and {TITLE_CHARS} characters"),
        format!("at most {BODY_CHARS} characters"),
        format!("at most {REFERENCES_PER_LIST} positions"),
    ] {
        assert!(
            DRAFTING_PROMPT.contains(&stated),
            "the prompt does not say {stated:?}:\n{DRAFTING_PROMPT}"
        );
    }
}

#[test]
fn the_instructions_carry_the_brief_the_slice_and_the_shape() {
    let text = drafting_instructions(
        "# The brief\n\nCut a planned project into tickets.\n",
        "  The drafting contract  ",
        "A slice's drafts, filled by a pass and checked by warlock.",
        &[],
    );

    assert!(text.starts_with(DRAFTING_PROMPT));
    assert!(text.contains("Cut a planned project into tickets."));
    assert!(text.contains("`The drafting contract`"), "{text}");
    assert!(text.contains("A slice's drafts, filled by a pass and checked by warlock."));
    assert!(
        text.ends_with(
            "{\"drafts\":[{\"title\":\"\",\"body\":\"\",\"blocked_by\":[],\"blocks\":[]}]}"
        ),
        "{text}"
    );
    assert!(
        !text.contains("turned down"),
        "nothing was rejected, so nothing is listed back"
    );
}

#[test]
fn a_rejected_defect_is_listed_back_as_one_not_to_repeat() {
    let rejected = vec![
        Defect::Empty {
            field: "drafts[2].title".to_owned(),
        },
        Defect::TooLong {
            field: "drafts[0].body".to_owned(),
            chars: BODY_CHARS + 40,
            cap: BODY_CHARS,
        },
    ];

    let text = drafting_instructions("The brief.", "A slice", "What it asks for.", &rejected);

    assert!(text.contains("Do not repeat these defects:"), "{text}");
    for defect in &rejected {
        assert!(text.contains(&format!("\n- {defect}")), "{text}");
    }
}

#[test]
fn a_reference_beyond_the_array_is_dropped_by_the_repair() {
    let mut draft = sound(2);
    draft.blocked_by = vec![0, 7];
    draft.blocks = vec![1];

    let (repaired, mends) = mend(&third(draft), SLICE, PROSE);

    assert_eq!(
        repaired.drafts[2].blocked_by,
        vec![0],
        "an index the array does not hold is gone, and the one it does stays"
    );
    assert_eq!(repaired.drafts[2].blocks, vec![1]);
    assert_eq!(
        mends,
        vec![Mend {
            field: "drafts[2].blocked_by".to_owned(),
            done: Mended::Dropped { count: 1 },
        }]
    );
    assert_eq!(
        mends[0].to_string(),
        "drafts[2].blocked_by named 1 position(s) outside this slice's drafts and lost them"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_draft_that_refers_to_itself_loses_the_reference() {
    let mut draft = sound(2);
    draft.blocks = vec![2, 0];

    let (repaired, mends) = mend(&third(draft), SLICE, PROSE);

    assert_eq!(repaired.drafts[2].blocks, vec![0]);
    assert_eq!(
        mends,
        vec![Mend {
            field: "drafts[2].blocks".to_owned(),
            done: Mended::Dropped { count: 1 },
        }]
    );
}

#[test]
fn a_sound_fill_is_left_alone() {
    let (repaired, mends) = mend(&good(), SLICE, PROSE);
    assert_eq!(repaired, good());
    assert_eq!(mends, []);
}

#[test]
fn drafts_past_the_cap_go_from_the_end_and_the_references_into_them_go_too() {
    let mut fill = drafted((0..DRAFTS_PER_SLICE + 2).map(sound).collect());
    fill.drafts[0].blocks = vec![1, DRAFTS_PER_SLICE + 1];

    let (repaired, mends) = mend(&fill, SLICE, PROSE);

    assert_eq!(repaired.drafts.len(), DRAFTS_PER_SLICE);
    assert_eq!(repaired.drafts[11].title, sound(11).title, "the tail went");
    assert_eq!(
        repaired.drafts[0].blocks,
        vec![1],
        "the array is cut from the end and never thinned from the middle, so a surviving \
         reference still names the draft it always named"
    );
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "drafts".to_owned(),
                done: Mended::Shortened {
                    from: DRAFTS_PER_SLICE + 2,
                    to: DRAFTS_PER_SLICE,
                },
            },
            Mend {
                field: "drafts[0].blocks".to_owned(),
                done: Mended::Dropped { count: 1 },
            },
        ]
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_reference_list_over_its_cap_keeps_its_first_entries() {
    let mut draft = sound(2);
    draft.blocked_by = vec![0; REFERENCES_PER_LIST + 1];

    let (repaired, mends) = mend(&third(draft), SLICE, PROSE);

    assert_eq!(repaired.drafts[2].blocked_by.len(), REFERENCES_PER_LIST);
    assert_eq!(
        mends,
        vec![Mend {
            field: "drafts[2].blocked_by".to_owned(),
            done: Mended::Shortened {
                from: REFERENCES_PER_LIST + 1,
                to: REFERENCES_PER_LIST,
            },
        }]
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_multiline_title_keeps_its_first_line_and_an_over_cap_body_is_cut() {
    let draft = Draft::of(
        "Parse the scope section\nand file what it holds",
        "b".repeat(BODY_CHARS + 5),
    );

    let (repaired, mends) = mend(&third(draft), SLICE, PROSE);

    assert_eq!(repaired.drafts[2].title, "Parse the scope section");
    assert_eq!(repaired.drafts[2].body.chars().count(), BODY_CHARS);
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "drafts[2].title".to_owned(),
                done: Mended::FirstLine,
            },
            Mend {
                field: "drafts[2].body".to_owned(),
                done: Mended::Cut {
                    from: BODY_CHARS + 5,
                    to: BODY_CHARS,
                },
            },
        ]
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn an_over_cap_title_is_cut_on_a_character_boundary() {
    let draft = Draft::of("é".repeat(TITLE_CHARS + 9), BODY);

    let (repaired, mends) = mend(&third(draft), SLICE, PROSE);

    assert_eq!(repaired.drafts[2].title.chars().count(), TITLE_CHARS);
    assert_eq!(
        mends[0].done,
        Mended::Cut {
            from: TITLE_CHARS + 9,
            to: TITLE_CHARS,
        },
        "characters, not bytes"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_title_and_body_nobody_wrote_are_filled_from_the_slice_itself() {
    let (repaired, mends) = mend(&third(Draft::of("", "  ")), SLICE, PROSE);

    let filled = &repaired.drafts[2];
    assert!(filled.title.contains(SLICE), "{}", filled.title);
    assert!(filled.title.contains("Unwritten"), "{}", filled.title);
    assert!(filled.body.contains(PROSE), "{}", filled.body);
    assert!(
        filled.body.starts_with("No body was drafted"),
        "warlock's own line says it is warlock's: {}",
        filled.body
    );
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "drafts[2].title".to_owned(),
                done: Mended::Supplied,
            },
            Mend {
                field: "drafts[2].body".to_owned(),
                done: Mended::Supplied,
            },
        ]
    );
    assert_eq!(
        mends[0].to_string(),
        "drafts[2].title was not answered and was filled in from the slice's own text"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_slice_nobody_drafted_is_filled_with_one_draft() {
    let (repaired, mends) = mend(&drafted(Vec::new()), SLICE, PROSE);

    assert_eq!(repaired.drafts.len(), 1);
    assert!(repaired.drafts[0].title.contains(SLICE));
    assert_eq!(
        mends,
        vec![Mend {
            field: "drafts".to_owned(),
            done: Mended::Supplied,
        }]
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_title_cut_back_to_one_line_that_is_then_too_short_falls_to_the_slice() {
    let draft = Draft::of("tiny\nbut the second line of it runs on and on", BODY);

    let (repaired, mends, passes) = mended(&third(draft), SLICE, PROSE);

    assert_eq!(passes, 2, "the repair of a repair is a second pass");
    assert!(repaired.drafts[2].title.contains(SLICE));
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "drafts[2].title".to_owned(),
                done: Mended::FirstLine,
            },
            Mend {
                field: "drafts[2].title".to_owned(),
                done: Mended::Supplied,
            },
        ],
        "one record per repair, including the one the first repair made necessary"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn an_unnamed_slice_still_fills_a_slot_inside_every_cap() {
    let (repaired, _) = mend(&drafted(Vec::new()), "  ", "");

    assert_eq!(check(&repaired), []);
    assert!(repaired.drafts[0].title.contains("an unnamed slice"));
}

fn variant(defect: &Defect) -> &'static str {
    match defect {
        Defect::NotJson { .. } => "NotJson",
        Defect::Missing { .. } => "Missing",
        Defect::Empty { .. } => "Empty",
        Defect::Multiline { .. } => "Multiline",
        Defect::TooShort { .. } => "TooShort",
        Defect::TooLong { .. } => "TooLong",
        Defect::TooMany { .. } => "TooMany",
        Defect::UnknownTarget { .. } => "UnknownTarget",
        Defect::ToolNamed { .. } => "ToolNamed",
    }
}

fn good() -> Fill {
    let mut fill = drafted((0..4).map(sound).collect());
    fill.drafts[0].blocks = vec![1, 3];
    fill.drafts[1].blocked_by = vec![0];
    fill
}

type Mutation = (&'static str, fn(&mut Fill));

// One per repairable defect, plus the two reference slips no defect reports,
// and a few that collide on purpose: two mutations over the same slot are how a
// repair comes to answer a slot another repair already moved. Every one of them
// tolerates an array another mutation emptied or replaced.
fn mutations() -> [Mutation; 11] {
    [
        ("Empty", |fill| {
            if let Some(draft) = fill.drafts.first_mut() {
                draft.title = "  ".to_owned();
            }
        }),
        ("Multiline", |fill| {
            if let Some(draft) = fill.drafts.get_mut(1) {
                draft.title = "tiny\nbut the second line of it runs on and on".to_owned();
            }
        }),
        ("TooShort", |fill| {
            if let Some(draft) = fill.drafts.get_mut(1) {
                draft.title = "short".to_owned();
            }
        }),
        ("TooLong", |fill| {
            if let Some(draft) = fill.drafts.last_mut() {
                draft.title = "é".repeat(TITLE_CHARS + 7);
            }
        }),
        ("Empty", |fill| {
            if let Some(draft) = fill.drafts.get_mut(2) {
                draft.body = "\n \n".to_owned();
            }
        }),
        ("TooLong", |fill| {
            if let Some(draft) = fill.drafts.get_mut(2) {
                draft.body = "b".repeat(BODY_CHARS + 40);
            }
        }),
        ("TooMany", |fill| {
            fill.drafts = (0..DRAFTS_PER_SLICE + 3).map(sound).collect();
            fill.drafts[0].blocks = vec![1, DRAFTS_PER_SLICE + 2];
        }),
        ("TooMany", |fill| {
            if let Some(draft) = fill.drafts.first_mut() {
                draft.blocked_by = (0..=REFERENCES_PER_LIST).collect();
            }
        }),
        ("Missing", |fill| fill.drafts.clear()),
        ("out of slice", |fill| {
            if let Some(draft) = fill.drafts.get_mut(1) {
                draft.blocks = vec![99, 0];
            }
        }),
        ("at itself", |fill| {
            let last = fill.drafts.len().saturating_sub(1);
            if let Some(draft) = fill.drafts.last_mut() {
                draft.blocked_by = vec![last];
            }
        }),
    ]
}

#[test]
fn a_mended_fill_is_never_defective_whatever_was_wrong_with_it() {
    let mutations = mutations();
    let mut covered: BTreeSet<&'static str> = BTreeSet::new();
    // A fixed seed and a plain congruential generator: this crate takes no
    // dependency for a coin toss, and a property test that cannot be reproduced
    // from its own source is not much of one.
    let mut state: u64 = 0x5eed_1234_5678_9abc;
    for _ in 0..512 {
        let mut fill = good();
        let mut applied: Vec<&str> = Vec::new();
        for (name, mutate) in &mutations {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (state >> 60) & 1 == 1 {
                mutate(&mut fill);
                applied.push(name);
            }
        }
        let defects = check(&fill);
        for defect in &defects {
            covered.insert(variant(defect));
        }

        let (repaired, mends, passes) = mended(&fill, SLICE, PROSE);

        assert_eq!(
            check(&repaired),
            [],
            "{applied:?} left {mends:?} and still a defect"
        );
        assert!(passes <= MEND_PASSES, "{applied:?} took {passes} passes");
        assert!(
            defects.is_empty() || !mends.is_empty(),
            "{applied:?}: a defect is answered by a mend"
        );
        let held = repaired.drafts.len();
        for (index, draft) in repaired.drafts.iter().enumerate() {
            assert!(
                draft
                    .blocked_by
                    .iter()
                    .chain(&draft.blocks)
                    .all(|target| *target < held && *target != index),
                "{applied:?} left a reference outside the slice: {draft:?}"
            );
        }
    }
    assert_eq!(
        covered,
        BTreeSet::from([
            "Missing",
            "Empty",
            "Multiline",
            "TooShort",
            "TooLong",
            "TooMany"
        ]),
        "every defect this contract reports was generated. `NotJson` cannot be — the mend is \
         handed a fill, not an answer — and `UnknownTarget` and `ToolNamed` belong to the \
         document road, which has an evidence witness this one has not"
    );
}
