use crate::document::Defect;

use super::{
    Accepted, DEPENDS_ON_PER_SUBTASK, DONE_CHARS, DONE_PER_SUBTASK, FILE_CHARS, FILES_PER_SUBTASK,
    Fill, GOAL_CHARS, GOAL_MINIMUM, NOTES_CHARS, SPLIT_PROMPT, SUBTASKS_PER_TICKET, Subtask,
    TEST_PLAN_CHARS, accept, check, split_instructions, stub_answer,
};

const DONE: &str = "The module exists and its tests pass.";

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
