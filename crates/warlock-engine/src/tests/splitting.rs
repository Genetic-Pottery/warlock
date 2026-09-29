use std::collections::BTreeSet;

use crate::document::Defect;

use super::{
    Accepted, Caught, DEPENDS_ON_PER_SUBTASK, DONE_CHARS, DONE_PER_SUBTASK, FILE_CHARS,
    FILES_PER_SUBTASK, Fill, GOAL_CHARS, GOAL_MINIMUM, MEND_PASSES, Mend, Mended, NOTES_CHARS,
    Numbered, SPLIT_PROMPT, SUBTASKS_PER_TICKET, Subtask, TEST_PLAN_CHARS, accept, check, mend,
    mended, number, split_instructions, stub_answer,
};

const DONE: &str = "The module exists and its tests pass.";

const TICKET: &str = "Split a pulled ticket into numbered sub-tasks";

const DESCRIPTION: &str = "A pulled ticket is one description, and the run needs an ordered set \
                           of small sub-tasks.";

fn split(subtasks: Vec<Subtask>) -> Fill {
    Fill { subtasks }
}

fn sound(which: usize) -> Subtask {
    Subtask {
        goal: format!("Sub-task number {which} of this ticket"),
        ..Subtask::default()
    }
}

// Three sound sub-tasks with the one under test third, so every field string a
// defect carries is `subtasks[2]...` and an index read off the wrong list shows
// up as a mismatch rather than passing by luck.
fn third(subtask: Subtask) -> Fill {
    split(vec![sound(0), sound(1), subtask])
}

fn goal(goal: impl Into<String>) -> Subtask {
    Subtask {
        goal: goal.into(),
        ..Subtask::default()
    }
}

#[test]
fn a_bare_string_a_lone_line_and_a_null_are_all_read_as_answers() {
    let answer = r#"{
        "subtasks": [
            {
                "goal": "Add the split contract to the engine",
                "depends_on": [],
                "definition_of_done": ["The module exists."],
                "likely_files": ["crates/warlock-engine/src/splitting.rs"],
                "test_plan": "cargo test -p warlock-engine",
                "notes": "Read drafting.rs first."
            },
            "Number the sub-tasks in a topological order",
            {
                "goal": "Write one brief per sub-task",
                "depends_on": [1, 2],
                "definition_of_done": "A brief exists for every sub-task",
                "likely_files": null,
                "test_plan": null,
                "notes": null
            },
            {"goal": "Comment on the ticket when the split fails"}
        ]
    }"#;

    let fill: Fill = serde_json::from_str(answer)
        .expect("a bare string, a lone line and a null are repairable and never Defect::NotJson");

    assert_eq!(fill.subtasks.len(), 4);
    assert_eq!(
        fill.subtasks[1],
        goal("Number the sub-tasks in a topological order"),
        "the string carries the goal and every other field comes back empty"
    );
    assert_eq!(
        fill.subtasks[2].definition_of_done,
        vec!["A brief exists for every sub-task"],
        "one line where a list was asked for is a list of one"
    );
    assert!(fill.subtasks[2].likely_files.is_empty());
    assert!(fill.subtasks[2].test_plan.is_empty());
    assert!(fill.subtasks[2].notes.is_empty());
    assert_eq!(fill.subtasks[2].depends_on, vec![1, 2]);
    assert_eq!(
        fill.subtasks[3],
        goal("Comment on the ticket when the split fails"),
        "an absent field reads as an empty one"
    );
    assert_eq!(
        check(&fill),
        [],
        "only the goal is required, so a sub-task with nothing else is sound"
    );
}

#[test]
fn a_sound_answer_is_filled_and_a_defective_one_carries_its_defects() {
    let sound = split(vec![sound(0), sound(1)]).to_json();
    assert!(
        matches!(accept(&sound), Accepted::Filled(_)),
        "{:?}",
        accept(&sound)
    );

    let answer = format!(
        "Here is the object:\n\n{}\n\nThat is all.",
        third(goal("")).to_json()
    );
    let Accepted::Defective { fill, defects } = accept(&answer) else {
        panic!("a defective fill is accepted with its defects, not thrown away");
    };
    assert_eq!(fill.subtasks.len(), 3, "the fill survives its defects");
    assert_eq!(
        defects,
        vec![Defect::Empty {
            field: "subtasks[2].goal".to_owned()
        }]
    );

    let Accepted::Unparsed(defect) = accept("I could not split this ticket.") else {
        panic!("an answer with no object in it is unparsed");
    };
    assert!(matches!(defect, Defect::NotJson { .. }), "{defect:?}");
}

#[test]
fn the_stub_answer_is_a_fill_inside_every_cap_and_is_accepted_whole() {
    let answer = stub_answer("Split a pulled ticket into numbered sub-tasks");
    let fill: Fill = serde_json::from_str(&answer).expect("the stub is the contract's own shape");

    assert!((1..=SUBTASKS_PER_TICKET).contains(&fill.subtasks.len()));
    for subtask in &fill.subtasks {
        let goal = subtask.goal.chars().count();
        assert!((GOAL_MINIMUM..=GOAL_CHARS).contains(&goal), "{goal}");
        assert!(!subtask.goal.contains(['\n', '\r']));
        assert!(subtask.depends_on.len() <= DEPENDS_ON_PER_SUBTASK);
        assert!(
            subtask
                .depends_on
                .iter()
                .all(|position| (1..=fill.subtasks.len()).contains(position)),
            "a dependency is a 1-based position in this ticket's own sub-tasks"
        );
        assert!(subtask.definition_of_done.len() <= DONE_PER_SUBTASK);
        assert!(subtask.likely_files.len() <= FILES_PER_SUBTASK);
        assert!(subtask.test_plan.chars().count() <= TEST_PLAN_CHARS);
        assert!(subtask.notes.chars().count() <= NOTES_CHARS);
    }
    assert!(fill.subtasks.iter().any(|subtask| {
        subtask
            .goal
            .contains("Split a pulled ticket into numbered sub-tasks")
    }));
    assert!(
        matches!(accept(&answer), Accepted::Filled(_)),
        "a test double's answer needs no repair"
    );
    assert!(
        stub_answer("   ").contains("an unnamed ticket"),
        "an unnamed ticket still fills a goal inside every cap"
    );
    assert!(matches!(accept(&stub_answer("   ")), Accepted::Filled(_)));
}

#[test]
fn a_ticket_nobody_split_is_a_missing_array() {
    assert_eq!(
        check(&split(Vec::new())),
        vec![Defect::Missing {
            field: "subtasks".to_owned()
        }]
    );
}

#[test]
fn more_subtasks_than_the_cap_are_reported_against_the_array() {
    let fill = split((0..=SUBTASKS_PER_TICKET).map(sound).collect());
    assert_eq!(
        check(&fill),
        vec![Defect::TooMany {
            field: "subtasks".to_owned(),
            count: SUBTASKS_PER_TICKET + 1,
            cap: SUBTASKS_PER_TICKET,
        }]
    );
}

#[test]
fn an_empty_goal_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(goal("   "))),
        vec![Defect::Empty {
            field: "subtasks[2].goal".to_owned()
        }]
    );
}

#[test]
fn a_goal_running_to_two_lines_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(goal("Add the split contract\nand its repair"))),
        vec![Defect::Multiline {
            field: "subtasks[2].goal".to_owned()
        }]
    );
}

#[test]
fn a_goal_under_the_minimum_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(goal("g".repeat(GOAL_MINIMUM - 1)))),
        vec![Defect::TooShort {
            field: "subtasks[2].goal".to_owned(),
            chars: GOAL_MINIMUM - 1,
            minimum: GOAL_MINIMUM,
        }]
    );
}

#[test]
fn a_goal_over_its_cap_is_reported_at_its_slot() {
    assert_eq!(
        check(&third(goal("g".repeat(GOAL_CHARS + 1)))),
        vec![Defect::TooLong {
            field: "subtasks[2].goal".to_owned(),
            chars: GOAL_CHARS + 1,
            cap: GOAL_CHARS,
        }]
    );
}

#[test]
fn more_dependencies_than_the_list_cap_are_reported_against_the_list() {
    let mut subtask = sound(2);
    subtask.depends_on = (1..=DEPENDS_ON_PER_SUBTASK + 1).collect();
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooMany {
            field: "subtasks[2].depends_on".to_owned(),
            count: DEPENDS_ON_PER_SUBTASK + 1,
            cap: DEPENDS_ON_PER_SUBTASK,
        }],
        "the list over the cap is named, and a position outside the ticket is the repair's business"
    );
}

#[test]
fn a_dependency_outside_the_ticket_is_no_defect_here() {
    let mut subtask = sound(2);
    subtask.depends_on = vec![0, 3, 99];
    assert_eq!(
        check(&third(subtask)),
        [],
        "zero, itself and past the end all go unreported: dropping them is the repair's job"
    );
}

#[test]
fn more_definition_of_done_entries_than_the_cap_are_reported_against_the_list() {
    let mut subtask = sound(2);
    subtask.definition_of_done = vec![DONE.to_owned(); DONE_PER_SUBTASK + 1];
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooMany {
            field: "subtasks[2].definition_of_done".to_owned(),
            count: DONE_PER_SUBTASK + 1,
            cap: DONE_PER_SUBTASK,
        }]
    );
}

#[test]
fn a_definition_of_done_entry_is_held_to_one_line_and_to_its_cap() {
    let mut subtask = sound(2);
    subtask.definition_of_done = vec![
        DONE.to_owned(),
        "  ".to_owned(),
        "d".repeat(DONE_CHARS + 1),
        "One fact.\nAnd a second.".to_owned(),
    ];
    assert_eq!(
        check(&third(subtask)),
        vec![
            Defect::Empty {
                field: "subtasks[2].definition_of_done[1]".to_owned()
            },
            Defect::TooLong {
                field: "subtasks[2].definition_of_done[2]".to_owned(),
                chars: DONE_CHARS + 1,
                cap: DONE_CHARS,
            },
            Defect::Multiline {
                field: "subtasks[2].definition_of_done[3]".to_owned()
            },
        ],
        "each entry is reported at its own position, and a short entry is no defect"
    );
}

#[test]
fn more_likely_files_than_the_cap_are_reported_against_the_list() {
    let mut subtask = sound(2);
    subtask.likely_files = vec!["src/lib.rs".to_owned(); FILES_PER_SUBTASK + 1];
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooMany {
            field: "subtasks[2].likely_files".to_owned(),
            count: FILES_PER_SUBTASK + 1,
            cap: FILES_PER_SUBTASK,
        }]
    );
}

#[test]
fn a_likely_file_over_its_cap_is_reported_at_its_position() {
    let mut subtask = sound(2);
    subtask.likely_files = vec!["src/lib.rs".to_owned(), "f".repeat(FILE_CHARS + 1)];
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooLong {
            field: "subtasks[2].likely_files[1]".to_owned(),
            chars: FILE_CHARS + 1,
            cap: FILE_CHARS,
        }],
        "a path short enough to be one word is no defect"
    );
}

#[test]
fn a_test_plan_over_its_cap_is_reported_at_its_slot() {
    let mut subtask = sound(2);
    subtask.test_plan = "t".repeat(TEST_PLAN_CHARS + 1);
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooLong {
            field: "subtasks[2].test_plan".to_owned(),
            chars: TEST_PLAN_CHARS + 1,
            cap: TEST_PLAN_CHARS,
        }]
    );
}

#[test]
fn notes_over_their_cap_are_reported_at_their_slot() {
    let mut subtask = sound(2);
    subtask.notes = "n".repeat(NOTES_CHARS + 1);
    assert_eq!(
        check(&third(subtask)),
        vec![Defect::TooLong {
            field: "subtasks[2].notes".to_owned(),
            chars: NOTES_CHARS + 1,
            cap: NOTES_CHARS,
        }]
    );
}

#[test]
fn an_unwritten_test_plan_or_note_is_no_defect() {
    let mut blank = sound(2);
    blank.test_plan = "  ".to_owned();
    blank.notes = "\n \n".to_owned();
    assert_eq!(
        check(&third(blank)),
        [],
        "both are optional, so blank is how a pass says it had nothing to add"
    );

    let mut written = sound(2);
    written.test_plan = "cargo fmt\ncargo test -p warlock-engine".to_owned();
    written.notes = "One paragraph.\n\nAnd a second.".to_owned();
    assert_eq!(
        check(&third(written)),
        [],
        "only the goal and the list entries are held to one line"
    );
}

#[test]
fn the_prompt_states_every_cap_it_is_checked_against() {
    // Each number in the phrase that carries it, not on its own: three of the
    // caps are 8 and two are 200, so a bare `contains("8")` would pass for all
    // of them with one written into the prose and the rest forgotten.
    for stated in [
        format!("between 1 and {SUBTASKS_PER_TICKET} entries"),
        format!("between {GOAL_MINIMUM} and {GOAL_CHARS} characters"),
        format!("at most {DEPENDS_ON_PER_SUBTASK} positions per list"),
        format!("at most {DONE_PER_SUBTASK} entries of at most {DONE_CHARS} characters"),
        format!("at most {FILES_PER_SUBTASK} entries of at most {FILE_CHARS} characters"),
        format!("at most {TEST_PLAN_CHARS} characters"),
        format!("At most {NOTES_CHARS} characters"),
    ] {
        assert!(
            SPLIT_PROMPT.contains(&stated),
            "the prompt does not say {stated:?}:\n{SPLIT_PROMPT}"
        );
    }
    assert!(
        SPLIT_PROMPT.contains("counting from 1"),
        "the 1-based dependency is stated, not assumed"
    );
}

#[test]
fn the_prompt_keeps_a_change_and_its_test_in_one_sub_task() {
    // GEN-7 split into "add `reverse`" and "add its tests"; the first session
    // wrote both, and the second ran to find nothing left.
    assert!(SPLIT_PROMPT.contains("A change and the test that covers it are one sub-task"));
}

#[test]
fn the_prompt_binds_every_sub_task_to_the_tickets_rules() {
    // A split that paraphrased "unexported, exactly one route" into "exported,
    // populate the table" handed the worker a brief that broke the ticket.
    assert!(SPLIT_PROMPT.contains("The ticket's rules bind every sub-task"));
    assert!(SPLIT_PROMPT.contains("Never loosen, invert or drop one"));
    assert!(SPLIT_PROMPT.contains("copy the ticket's words"));
}

#[test]
fn the_instructions_carry_the_ticket_and_the_shape() {
    let text = split_instructions(
        "  Split a pulled ticket into numbered sub-tasks  ",
        "A pulled ticket is one description, and the run needs an ordered set of small \
         sub-tasks.\n",
        &[],
    );

    assert!(text.starts_with(SPLIT_PROMPT));
    assert!(
        text.contains("`Split a pulled ticket into numbered sub-tasks`"),
        "{text}"
    );
    assert!(text.contains("the run needs an ordered set of small sub-tasks."));
    assert!(
        text.ends_with(
            "{\"subtasks\":[{\"goal\":\"\",\"depends_on\":[],\"definition_of_done\":[],\
             \"likely_files\":[],\"test_plan\":\"\",\"notes\":\"\"}]}"
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
            field: "subtasks[2].goal".to_owned(),
        },
        Defect::TooMany {
            field: "subtasks".to_owned(),
            count: SUBTASKS_PER_TICKET + 4,
            cap: SUBTASKS_PER_TICKET,
        },
    ];

    let text = split_instructions("A ticket", "What it asks for.", &rejected);

    assert!(text.contains("Do not repeat these defects:"), "{text}");
    for defect in &rejected {
        assert!(text.contains(&format!("\n- {defect}")), "{text}");
    }
}

#[test]
fn a_position_outside_the_ticket_is_dropped_by_the_repair() {
    let mut subtask = sound(2);
    subtask.depends_on = vec![0, 1, 9];

    let (repaired, mends) = mend(&third(subtask), TICKET, DESCRIPTION);

    assert_eq!(
        repaired.subtasks[2].depends_on,
        vec![1],
        "zero and a position the ticket does not hold are gone, and the one it holds stays"
    );
    assert_eq!(
        mends,
        vec![Mend {
            field: "subtasks[2].depends_on".to_owned(),
            done: Mended::Dropped { count: 2 },
        }]
    );
    assert_eq!(
        mends[0].to_string(),
        "subtasks[2].depends_on named 2 position(s) outside this ticket's sub-tasks and lost them"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_subtask_that_waits_on_itself_loses_the_position() {
    let mut subtask = sound(2);
    // Third of three, so 1-based this sub-task is position 3.
    subtask.depends_on = vec![3, 1];

    let (repaired, mends) = mend(&third(subtask), TICKET, DESCRIPTION);

    assert_eq!(repaired.subtasks[2].depends_on, vec![1]);
    assert_eq!(
        mends,
        vec![Mend {
            field: "subtasks[2].depends_on".to_owned(),
            done: Mended::Dropped { count: 1 },
        }]
    );
}

#[test]
fn a_sound_fill_is_left_alone() {
    let (repaired, mends) = mend(&good(), TICKET, DESCRIPTION);
    assert_eq!(repaired, good());
    assert_eq!(mends, []);
}

#[test]
fn subtasks_past_the_cap_go_from_the_end_and_the_positions_into_them_go_too() {
    let mut fill = split((0..SUBTASKS_PER_TICKET + 2).map(sound).collect());
    fill.subtasks[0].depends_on = vec![2, SUBTASKS_PER_TICKET + 2];

    let (repaired, mends) = mend(&fill, TICKET, DESCRIPTION);

    assert_eq!(repaired.subtasks.len(), SUBTASKS_PER_TICKET);
    assert_eq!(repaired.subtasks[7].goal, sound(7).goal, "the tail went");
    assert_eq!(
        repaired.subtasks[0].depends_on,
        vec![2],
        "the array is cut from the end and never thinned from the middle, so a surviving \
         position still names the sub-task it always named"
    );
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "subtasks".to_owned(),
                done: Mended::Shortened {
                    from: SUBTASKS_PER_TICKET + 2,
                    to: SUBTASKS_PER_TICKET,
                },
            },
            Mend {
                field: "subtasks[0].depends_on".to_owned(),
                done: Mended::Dropped { count: 1 },
            },
        ]
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_list_over_its_cap_keeps_its_first_entries() {
    let mut subtask = sound(2);
    subtask.depends_on = vec![1; DEPENDS_ON_PER_SUBTASK + 1];
    subtask.definition_of_done = (0..=DONE_PER_SUBTASK)
        .map(|which| format!("{DONE} ({which})"))
        .collect();
    subtask.likely_files = (0..=FILES_PER_SUBTASK)
        .map(|which| format!("src/file{which}.rs"))
        .collect();

    let (repaired, mends) = mend(&third(subtask), TICKET, DESCRIPTION);

    let kept = &repaired.subtasks[2];
    assert_eq!(kept.depends_on.len(), DEPENDS_ON_PER_SUBTASK);
    assert_eq!(kept.definition_of_done.len(), DONE_PER_SUBTASK);
    assert_eq!(kept.definition_of_done[0], format!("{DONE} (0)"));
    assert_eq!(kept.likely_files.len(), FILES_PER_SUBTASK);
    assert_eq!(kept.likely_files[0], "src/file0.rs");
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "subtasks[2].depends_on".to_owned(),
                done: Mended::Shortened {
                    from: DEPENDS_ON_PER_SUBTASK + 1,
                    to: DEPENDS_ON_PER_SUBTASK,
                },
            },
            Mend {
                field: "subtasks[2].definition_of_done".to_owned(),
                done: Mended::Shortened {
                    from: DONE_PER_SUBTASK + 1,
                    to: DONE_PER_SUBTASK,
                },
            },
            Mend {
                field: "subtasks[2].likely_files".to_owned(),
                done: Mended::Shortened {
                    from: FILES_PER_SUBTASK + 1,
                    to: FILES_PER_SUBTASK,
                },
            },
        ]
    );
    assert_eq!(
        mends[1].to_string(),
        format!(
            "subtasks[2].definition_of_done had {} entries and was cut to the first {}",
            DONE_PER_SUBTASK + 1,
            DONE_PER_SUBTASK,
        )
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_blank_list_entry_is_dropped_and_the_drop_is_named() {
    let mut subtask = sound(2);
    subtask.definition_of_done = vec![DONE.to_owned(), "  ".to_owned(), "\n".to_owned()];
    subtask.likely_files = vec!["   ".to_owned(), "src/lib.rs".to_owned()];

    let (repaired, mends) = mend(&third(subtask), TICKET, DESCRIPTION);

    assert_eq!(
        repaired.subtasks[2].definition_of_done,
        vec![DONE.to_owned()],
        "the blanks are gone and the written entry stays"
    );
    assert_eq!(repaired.subtasks[2].likely_files, vec!["src/lib.rs"]);
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "subtasks[2].definition_of_done[1]".to_owned(),
                done: Mended::Blank,
            },
            Mend {
                field: "subtasks[2].definition_of_done[2]".to_owned(),
                done: Mended::Blank,
            },
            Mend {
                field: "subtasks[2].likely_files[0]".to_owned(),
                done: Mended::Blank,
            },
        ],
        "each blank is named at the position it was read at, before the list shortened"
    );
    assert_eq!(
        mends[0].to_string(),
        "subtasks[2].definition_of_done[1] was left blank and was dropped from its list"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_multiline_goal_keeps_its_first_line_and_over_cap_prose_is_cut() {
    let mut subtask = sound(2);
    subtask.goal = "Repair a defective split\nrather than refusing it".to_owned();
    subtask.test_plan = "t".repeat(TEST_PLAN_CHARS + 5);
    subtask.notes = "n".repeat(NOTES_CHARS + 5);
    subtask.definition_of_done = vec![
        "One fact.\nAnd a second.".to_owned(),
        "d".repeat(DONE_CHARS + 1),
    ];
    subtask.likely_files = vec!["f".repeat(FILE_CHARS + 1)];

    let (repaired, mends) = mend(&third(subtask), TICKET, DESCRIPTION);

    let repaired_subtask = &repaired.subtasks[2];
    assert_eq!(repaired_subtask.goal, "Repair a defective split");
    assert_eq!(repaired_subtask.test_plan.chars().count(), TEST_PLAN_CHARS);
    assert_eq!(repaired_subtask.notes.chars().count(), NOTES_CHARS);
    assert_eq!(repaired_subtask.definition_of_done[0], "One fact.");
    assert_eq!(
        repaired_subtask.definition_of_done[1].chars().count(),
        DONE_CHARS
    );
    assert_eq!(repaired_subtask.likely_files[0].chars().count(), FILE_CHARS);
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "subtasks[2].goal".to_owned(),
                done: Mended::FirstLine,
            },
            Mend {
                field: "subtasks[2].definition_of_done[0]".to_owned(),
                done: Mended::FirstLine,
            },
            Mend {
                field: "subtasks[2].definition_of_done[1]".to_owned(),
                done: Mended::Cut {
                    from: DONE_CHARS + 1,
                    to: DONE_CHARS,
                },
            },
            Mend {
                field: "subtasks[2].likely_files[0]".to_owned(),
                done: Mended::Cut {
                    from: FILE_CHARS + 1,
                    to: FILE_CHARS,
                },
            },
            Mend {
                field: "subtasks[2].test_plan".to_owned(),
                done: Mended::Cut {
                    from: TEST_PLAN_CHARS + 5,
                    to: TEST_PLAN_CHARS,
                },
            },
            Mend {
                field: "subtasks[2].notes".to_owned(),
                done: Mended::Cut {
                    from: NOTES_CHARS + 5,
                    to: NOTES_CHARS,
                },
            },
        ]
    );
    assert_eq!(
        mends[0].to_string(),
        "subtasks[2].goal ran to more than one line and keeps its first"
    );
    assert_eq!(
        mends[4].to_string(),
        format!(
            "subtasks[2].test_plan was {} characters and was cut to {TEST_PLAN_CHARS}",
            TEST_PLAN_CHARS + 5,
        )
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn an_over_cap_goal_is_cut_on_a_character_boundary() {
    let (repaired, mends) = mend(
        &third(goal("é".repeat(GOAL_CHARS + 9))),
        TICKET,
        DESCRIPTION,
    );

    assert_eq!(repaired.subtasks[2].goal.chars().count(), GOAL_CHARS);
    assert_eq!(
        mends[0].done,
        Mended::Cut {
            from: GOAL_CHARS + 9,
            to: GOAL_CHARS,
        },
        "characters, not bytes"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_goal_nobody_wrote_is_filled_from_the_ticket_itself() {
    let (repaired, mends) = mend(&third(goal("  ")), TICKET, DESCRIPTION);

    let filled = &repaired.subtasks[2].goal;
    assert!(filled.contains(TICKET), "{filled}");
    assert!(
        filled.starts_with("Unwritten sub-task 3"),
        "warlock's own line says it is warlock's: {filled}"
    );
    assert_eq!(
        mends,
        vec![Mend {
            field: "subtasks[2].goal".to_owned(),
            done: Mended::Supplied,
        }]
    );
    assert_eq!(
        mends[0].to_string(),
        "subtasks[2].goal was not answered and was filled in from the ticket's own text"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn a_ticket_nobody_split_becomes_one_sub_task_built_from_the_ticket() {
    for empty in [split(Vec::new()), serde_json::from_str("{}").unwrap()] {
        let (repaired, mends) = mend(&empty, TICKET, DESCRIPTION);

        assert!(
            (1..=SUBTASKS_PER_TICKET).contains(&repaired.subtasks.len()),
            "a split always yields between 1 and {SUBTASKS_PER_TICKET}"
        );
        assert!(repaired.subtasks[0].goal.contains(TICKET));
        assert!(
            repaired.subtasks[0]
                .notes
                .starts_with("No sub-task was split"),
            "{}",
            repaired.subtasks[0].notes
        );
        assert!(
            repaired.subtasks[0].notes.contains(DESCRIPTION),
            "the ticket's own description is what warlock has to write from"
        );
        assert!(repaired.subtasks[0].depends_on.is_empty());
        assert_eq!(
            mends,
            vec![Mend {
                field: "subtasks".to_owned(),
                done: Mended::Supplied,
            }]
        );
        assert_eq!(check(&repaired), []);
    }
}

#[test]
fn a_goal_cut_back_to_one_line_that_is_then_too_short_falls_to_the_ticket() {
    let subtask = goal("tiny\nbut the second line of it runs on and on");

    let (repaired, mends, passes) = mended(&third(subtask), TICKET, DESCRIPTION);

    assert_eq!(passes, 2, "the repair of a repair is a second pass");
    assert!(repaired.subtasks[2].goal.contains(TICKET));
    assert_eq!(
        mends,
        vec![
            Mend {
                field: "subtasks[2].goal".to_owned(),
                done: Mended::FirstLine,
            },
            Mend {
                field: "subtasks[2].goal".to_owned(),
                done: Mended::Supplied,
            },
        ],
        "one record per repair, including the one the first repair made necessary"
    );
    assert_eq!(check(&repaired), []);
}

#[test]
fn an_unnamed_ticket_still_fills_a_slot_inside_every_cap() {
    let (repaired, _) = mend(&split(Vec::new()), "  ", "");

    assert_eq!(check(&repaired), []);
    assert!(repaired.subtasks[0].goal.contains("an unnamed ticket"));
    assert!(
        repaired.subtasks[0]
            .notes
            .contains("The ticket says nothing further.")
    );
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
    let mut fill = split((0..4).map(sound).collect());
    fill.subtasks[1].depends_on = vec![1];
    fill.subtasks[1].definition_of_done = vec![DONE.to_owned()];
    fill.subtasks[2].likely_files = vec!["crates/warlock-engine/src/splitting.rs".to_owned()];
    fill.subtasks[3].depends_on = vec![1, 2];
    fill.subtasks[3].test_plan = "cargo test -p warlock-engine".to_owned();
    fill.subtasks[3].notes = "Read drafting.rs first.".to_owned();
    fill
}

type Mutation = (&'static str, fn(&mut Fill));

// One per repairable defect at every slot that can carry it, plus the two
// position slips no defect reports, and a few that collide on purpose: two
// mutations over the same slot are how a repair comes to answer a slot another
// repair already moved. Every one of them tolerates an array another mutation
// emptied or replaced.
fn mutations() -> [Mutation; 15] {
    [
        ("Empty", |fill| {
            if let Some(subtask) = fill.subtasks.first_mut() {
                subtask.goal = "  ".to_owned();
            }
        }),
        ("Multiline", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(1) {
                subtask.goal = "tiny\nbut the second line of it runs on and on".to_owned();
            }
        }),
        ("TooShort", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(1) {
                subtask.goal = "short".to_owned();
            }
        }),
        ("TooLong", |fill| {
            if let Some(subtask) = fill.subtasks.last_mut() {
                subtask.goal = "é".repeat(GOAL_CHARS + 7);
            }
        }),
        ("TooMany", |fill| {
            fill.subtasks = (0..SUBTASKS_PER_TICKET + 3).map(sound).collect();
            fill.subtasks[0].depends_on = vec![2, SUBTASKS_PER_TICKET + 3];
        }),
        ("TooMany", |fill| {
            if let Some(subtask) = fill.subtasks.first_mut() {
                subtask.depends_on = (1..=DEPENDS_ON_PER_SUBTASK + 1).collect();
            }
        }),
        ("TooMany", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(2) {
                subtask.definition_of_done = (0..DONE_PER_SUBTASK + 2)
                    .map(|which| format!("{DONE} ({which})"))
                    .collect();
                subtask.definition_of_done[DONE_PER_SUBTASK + 1] = "  ".to_owned();
            }
        }),
        ("Empty", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(2) {
                subtask.definition_of_done.insert(0, " \n ".to_owned());
            }
        }),
        ("TooLong", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(2) {
                subtask.definition_of_done.push("d".repeat(DONE_CHARS + 40));
            }
        }),
        ("Multiline", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(2) {
                subtask
                    .definition_of_done
                    .push("One fact.\nAnd a second.".to_owned());
            }
        }),
        ("TooMany", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(2) {
                subtask.likely_files = (0..FILES_PER_SUBTASK + 2)
                    .map(|which| format!("src/file{which}.rs"))
                    .collect();
                subtask.likely_files.push("  ".to_owned());
            }
        }),
        ("TooLong", |fill| {
            if let Some(subtask) = fill.subtasks.last_mut() {
                subtask.likely_files.push("f".repeat(FILE_CHARS + 3));
                subtask.test_plan = "t".repeat(TEST_PLAN_CHARS + 30);
                subtask.notes = "n".repeat(NOTES_CHARS + 30);
            }
        }),
        ("Missing", |fill| fill.subtasks.clear()),
        ("out of ticket", |fill| {
            if let Some(subtask) = fill.subtasks.get_mut(1) {
                subtask.depends_on = vec![99, 0, 1];
            }
        }),
        ("at itself", |fill| {
            let last = fill.subtasks.len();
            if let Some(subtask) = fill.subtasks.last_mut() {
                subtask.depends_on = vec![last];
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

        let (repaired, mends, passes) = mended(&fill, TICKET, DESCRIPTION);

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
        let held = repaired.subtasks.len();
        assert!(
            (1..=SUBTASKS_PER_TICKET).contains(&held),
            "{applied:?} left {held} sub-tasks"
        );
        for (index, subtask) in repaired.subtasks.iter().enumerate() {
            assert!(
                subtask
                    .depends_on
                    .iter()
                    .all(|position| (1..=held).contains(position) && *position != index + 1),
                "{applied:?} left a position outside the ticket: {subtask:?}"
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

// A sub-task of `TICKET` that waits on the 1-based positions given.
fn waiting(which: usize, on: &[usize]) -> Subtask {
    Subtask {
        depends_on: on.to_vec(),
        ..sound(which)
    }
}

fn numbers(numbered: &[Numbered]) -> Vec<&str> {
    numbered.iter().map(|subtask| subtask.id.as_str()).collect()
}

fn goals(numbered: &[Numbered]) -> Vec<&str> {
    numbered
        .iter()
        .map(|subtask| subtask.goal.as_str())
        .collect()
}

// The property the numbering exists for: read top to bottom, every dependency
// is already finished.
fn legal(numbered: &[Numbered]) {
    let mut behind: Vec<&str> = Vec::new();
    for subtask in numbered {
        for dependency in &subtask.depends_on {
            assert!(
                behind.contains(&dependency.as_str()),
                "{} waits on {dependency}, which does not come before it in {:?}",
                subtask.id,
                numbers(numbered),
            );
        }
        behind.push(&subtask.id);
    }
}

#[test]
fn a_split_already_in_order_is_numbered_in_that_order_from_01() {
    let fill = split(vec![
        waiting(0, &[]),
        waiting(1, &[1]),
        waiting(2, &[2]),
        waiting(3, &[1, 3]),
    ]);

    let numbered = number(&fill, "WAR-138").expect("a chain in order holds no circle");

    assert_eq!(
        numbers(&numbered),
        ["WAR-138.01", "WAR-138.02", "WAR-138.03", "WAR-138.04"],
        "the numbering counts from 01, padded to two places"
    );
    assert_eq!(
        goals(&numbered),
        [
            sound(0).goal.as_str(),
            sound(1).goal.as_str(),
            sound(2).goal.as_str(),
            sound(3).goal.as_str(),
        ],
        "a split that arrived workable is left in the order the pass wrote it"
    );
    assert_eq!(numbered[3].depends_on, ["WAR-138.01", "WAR-138.03"]);
    legal(&numbered);
}

#[test]
fn a_chain_written_backwards_is_sorted_and_its_positions_become_names() {
    // Last first: 3 waits on 2, 2 waits on 1, 1 waits on nothing.
    let fill = split(vec![waiting(0, &[2]), waiting(1, &[3]), waiting(2, &[])]);

    let numbered = number(&fill, "WAR-138").expect("a chain holds no circle whichever way it runs");

    assert_eq!(
        goals(&numbered),
        [
            sound(2).goal.as_str(),
            sound(1).goal.as_str(),
            sound(0).goal.as_str(),
        ],
        "the order is turned around so every dependency is finished first"
    );
    assert_eq!(
        numbers(&numbered),
        ["WAR-138.01", "WAR-138.02", "WAR-138.03"]
    );
    assert_eq!(numbered[1].depends_on, ["WAR-138.01"]);
    assert_eq!(numbered[2].depends_on, ["WAR-138.02"]);
    legal(&numbered);
}

#[test]
fn a_diamond_puts_both_middles_after_the_first_and_before_the_last() {
    // 1 -> 2 and 1 -> 3, then 4 waits on both.
    let fill = split(vec![
        waiting(0, &[]),
        waiting(1, &[1]),
        waiting(2, &[1]),
        waiting(3, &[2, 3]),
    ]);

    let numbered = number(&fill, "WAR-138").expect("a diamond holds no circle");

    legal(&numbered);
    assert_eq!(numbered[0].goal, sound(0).goal);
    assert_eq!(numbered[3].goal, sound(3).goal);
    assert_eq!(numbered[3].depends_on, ["WAR-138.02", "WAR-138.03"]);
}

#[test]
fn sub_tasks_nothing_separates_keep_the_order_the_pass_gave_them() {
    // Nothing orders 1, 2 and 4 against each other, and 3 waits on 4. Only 3
    // moves, and it moves the least it can: everything else stays where the
    // pass put it.
    let fill = split(vec![
        waiting(0, &[]),
        waiting(1, &[]),
        waiting(2, &[4]),
        waiting(3, &[]),
    ]);

    let numbered = number(&fill, "WAR-138").expect("one dependency and no circle");

    assert_eq!(
        goals(&numbered),
        [
            sound(0).goal.as_str(),
            sound(1).goal.as_str(),
            sound(3).goal.as_str(),
            sound(2).goal.as_str(),
        ],
    );
    legal(&numbered);
    assert_eq!(
        number(&fill, "WAR-138").unwrap(),
        numbered,
        "two calls over one answer agree"
    );
}

#[test]
fn a_repeated_position_is_one_name_and_the_ticket_is_taken_as_written() {
    let fill = split(vec![waiting(0, &[]), waiting(1, &[1, 1])]);

    let numbered = number(&fill, "  WAR-138\n").expect("a repeat is not a circle");

    assert_eq!(numbers(&numbered), ["WAR-138.01", "WAR-138.02"]);
    assert_eq!(
        numbered[1].depends_on,
        ["WAR-138.01"],
        "one dependency written twice is one dependency"
    );
}

#[test]
fn a_mended_split_is_numbered_whole_and_carries_every_field_over() {
    let (mended, _) = mend(&good(), TICKET, DESCRIPTION);

    let numbered = number(&mended, "WAR-138").expect("the stub split holds no circle");

    assert_eq!(numbered.len(), mended.subtasks.len());
    legal(&numbered);
    let last = numbered.last().expect("four sub-tasks");
    assert_eq!(
        last.definition_of_done,
        mended.subtasks[3].definition_of_done
    );
    assert_eq!(last.likely_files, mended.subtasks[3].likely_files);
    assert_eq!(last.test_plan, mended.subtasks[3].test_plan);
    assert_eq!(last.notes, mended.subtasks[3].notes);
}

#[test]
fn two_sub_tasks_waiting_on_each_other_halt_the_split() {
    let fill = split(vec![waiting(0, &[2]), waiting(1, &[1])]);

    let cycle = number(&fill, "WAR-138").expect_err("neither sub-task can go first");

    assert_eq!(
        cycle.caught,
        vec![
            Caught {
                position: 1,
                goal: sound(0).goal,
            },
            Caught {
                position: 2,
                goal: sound(1).goal,
            },
        ],
    );
    let said = cycle.to_string();
    assert!(
        said.contains(
            "The split of `WAR-138` could not be ordered: sub-tasks 1 and 2 wait on one \
                       another"
        ),
        "{said}"
    );
    assert!(
        said.contains("No dependency was dropped to break the circle."),
        "{said}"
    );
    assert!(
        said.contains(&format!("Sub-task 1 is `{}`.", sound(0).goal)),
        "{said}"
    );
    assert!(
        said.contains(&format!("Sub-task 2 is `{}`.", sound(1).goal)),
        "{said}"
    );
}

#[test]
fn a_longer_circle_names_every_sub_task_in_it_and_nothing_else() {
    // 1 is orderable, 2 -> 4 -> 3 -> 2 is the circle, and 5 only waits on it.
    let fill = split(vec![
        waiting(0, &[]),
        waiting(1, &[4]),
        waiting(2, &[2]),
        waiting(3, &[3]),
        waiting(4, &[2]),
    ]);

    let cycle = number(&fill, "WAR-138").expect_err("three sub-tasks wait on each other");

    assert_eq!(
        cycle
            .caught
            .iter()
            .map(|caught| caught.position)
            .collect::<Vec<_>>(),
        [2, 3, 4],
        "the circle itself, not the sub-task that merely waits on it"
    );
    let said = cycle.to_string();
    assert!(
        said.contains("sub-tasks 2, 3 and 4 wait on one another"),
        "{said}"
    );
    assert!(
        !said.contains(&format!("Sub-task 5 is `{}`", sound(4).goal)),
        "a sub-task waiting on the circle is not in it: {said}"
    );
}

#[test]
fn an_unnamed_ticket_still_says_which_sub_tasks_are_in_the_circle() {
    let fill = split(vec![waiting(0, &[2]), waiting(1, &[1])]);

    let said = number(&fill, "   ").expect_err("a circle whatever the ticket is called");

    assert!(
        said.to_string().starts_with("The split of this ticket"),
        "{said}"
    );
}

// The repair drops a sub-task that waits on itself, so this only reaches
// `number` from a fill nobody mended. It is still a circle, and it still halts.
#[test]
fn a_sub_task_waiting_on_itself_is_a_circle_of_one() {
    let fill = split(vec![sound(0), waiting(1, &[2])]);

    let cycle = number(&fill, "WAR-138").expect_err("a sub-task cannot go after itself");

    assert_eq!(
        cycle.caught,
        vec![Caught {
            position: 2,
            goal: sound(1).goal,
        }],
    );
    assert!(
        cycle.to_string().contains("sub-task 2 waits on itself"),
        "{cycle}"
    );
}

#[test]
fn nothing_to_split_is_nothing_to_number() {
    assert_eq!(number(&Fill::default(), "WAR-138"), Ok(Vec::new()));
}

#[test]
fn the_module_speaks_no_git_http_or_linear_vocabulary() {
    // The contract is the engine's, and the engine reaches nothing: a word from
    // the transport in here is how a cap or a prompt ends up depending on where
    // the ticket came from.
    let source = include_str!("../splitting.rs").to_lowercase();
    for word in ["git", "http", "linear", "branch"] {
        assert!(
            !source.contains(word),
            "the splitting contract mentions {word:?}"
        );
    }
}
