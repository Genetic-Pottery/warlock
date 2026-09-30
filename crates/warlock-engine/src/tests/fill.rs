use std::cell::RefCell;
use std::collections::VecDeque;

use super::{Accepted, Asking, Defect, Mend, Reask, Rewrite, Schema, Settled, mend, settle};

fn too_long() -> Defect {
    Defect::TooLong {
        field: "body".to_owned(),
        chars: 9,
        cap: 4,
    }
}

fn empty() -> Defect {
    Defect::Empty {
        field: "title".to_owned(),
    }
}

fn not_json() -> Defect {
    Defect::NotJson {
        detail: "prose".to_owned(),
    }
}

// A model that answers from a script and remembers what it was told each turn,
// with the refusals the loop reported beside it.
struct Scripted {
    answers: RefCell<VecDeque<Accepted<&'static str>>>,
    told: RefCell<Vec<Vec<Defect>>>,
    refused: RefCell<Vec<(Vec<Defect>, usize)>>,
}

impl Scripted {
    fn new(answers: Vec<Accepted<&'static str>>) -> Self {
        Self {
            answers: RefCell::new(answers.into()),
            told: RefCell::new(Vec::new()),
            refused: RefCell::new(Vec::new()),
        }
    }

    fn run(&self, reask: Reask) -> Settled<&'static str> {
        let asking = Asking { attempts: 3, reask };
        settle(
            asking,
            |rejected| {
                self.told.borrow_mut().push(rejected.to_vec());
                Ok::<_, ()>(
                    self.answers
                        .borrow_mut()
                        .pop_front()
                        .expect("asked more often than scripted"),
                )
            },
            |defects, attempt| self.refused.borrow_mut().push((defects.to_vec(), attempt)),
        )
        .expect("the script never fails")
    }
}

#[test]
fn a_clean_answer_is_taken_on_the_turn_it_arrives() {
    for reask in [Reask::Any, Reask::Cut] {
        let model = Scripted::new(vec![Accepted::Filled("clean")]);
        assert_eq!(model.run(reask), Settled::Taken("clean"));
        assert_eq!(*model.told.borrow(), [Vec::<Defect>::new()]);
        assert!(model.refused.borrow().is_empty());
    }
}

#[test]
fn the_any_road_asks_again_about_every_defect_and_reports_each_refusal() {
    let model = Scripted::new(vec![
        Accepted::Defective {
            fill: "first",
            defects: vec![empty()],
        },
        Accepted::Unparsed(not_json()),
        Accepted::Filled("third"),
    ]);
    assert_eq!(model.run(Reask::Any), Settled::Taken("third"));
    assert_eq!(
        *model.told.borrow(),
        [vec![], vec![empty()], vec![not_json()]]
    );
    assert_eq!(
        *model.refused.borrow(),
        [(vec![empty()], 1), (vec![not_json()], 2)]
    );
}

#[test]
fn the_cut_road_takes_a_fill_whose_defects_the_mend_loses_nothing_over() {
    let model = Scripted::new(vec![Accepted::Defective {
        fill: "short title",
        defects: vec![empty()],
    }]);
    assert_eq!(model.run(Reask::Cut), Settled::Taken("short title"));
    assert_eq!(model.told.borrow().len(), 1);
}

#[test]
fn the_cut_road_asks_again_about_the_cut_alone() {
    let model = Scripted::new(vec![
        Accepted::Defective {
            fill: "long",
            defects: vec![empty(), too_long()],
        },
        Accepted::Filled("fits"),
    ]);
    assert_eq!(model.run(Reask::Cut), Settled::Taken("fits"));
    assert_eq!(*model.told.borrow(), [vec![], vec![too_long()]]);
}

#[test]
fn spent_attempts_keep_the_last_answer_that_parsed() {
    for reask in [Reask::Any, Reask::Cut] {
        let model = Scripted::new(vec![
            Accepted::Defective {
                fill: "older",
                defects: vec![too_long()],
            },
            Accepted::Defective {
                fill: "newer",
                defects: vec![too_long()],
            },
            Accepted::Unparsed(not_json()),
        ]);
        assert_eq!(model.run(reask), Settled::Spent("newer"));
        assert_eq!(
            model.refused.borrow().len(),
            3,
            "the last refusal is heard too"
        );
    }
}

#[test]
fn spent_attempts_with_nothing_parsed_carry_the_last_defect() {
    let last = Defect::NotJson {
        detail: "the last".to_owned(),
    };
    let model = Scripted::new(vec![
        Accepted::Unparsed(not_json()),
        Accepted::Unparsed(not_json()),
        Accepted::Unparsed(last.clone()),
    ]);
    assert_eq!(model.run(Reask::Cut), Settled::Unusable(last));
}

#[test]
fn a_turn_that_fails_ends_the_loop_at_once() {
    let mut turns = 0;
    let settled = settle::<(), _>(
        Asking {
            attempts: 4,
            reask: Reask::Any,
        },
        |_| {
            turns += 1;
            Err("no claude")
        },
        |_, _| panic!("a failed turn is not a refusal"),
    );
    assert_eq!(settled, Err("no claude"));
    assert_eq!(turns, 1);
}

// Lines of exactly four characters, one line each: enough schema to drive
// every step of the mend without any road's rules.
struct Lines {
    // What a line too short to keep is filled in with.
    pad: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mended {
    FirstLine,
    Cut,
    Padded,
}

impl From<Rewrite> for Mended {
    fn from(rewrite: Rewrite) -> Self {
        match rewrite {
            Rewrite::FirstLine => Self::FirstLine,
            Rewrite::Cut { .. } => Self::Cut,
        }
    }
}

fn index(field: &str) -> usize {
    field.parse().expect("a field here is an index")
}

impl Schema for Lines {
    type Fill = Vec<String>;
    type Mended = Mended;
    type Plan = Vec<usize>;

    const MEND_PASSES: usize = 3;

    fn check(&self, fill: &Vec<String>) -> Vec<Defect> {
        let mut defects = Vec::new();
        for (at, value) in fill.iter().enumerate() {
            super::line(&at.to_string(), value, 4, 4, &mut defects);
        }
        defects
    }

    fn plan(&self, defects: &[Defect], mends: &mut Vec<Mend<Mended>>) -> Vec<usize> {
        let mut padded = Vec::new();
        for defect in defects {
            if let Defect::Empty { field } | Defect::TooShort { field, .. } = defect {
                padded.push(index(field));
                mends.push(Mend {
                    field: field.clone(),
                    done: Mended::Padded,
                });
            }
        }
        padded
    }

    fn rewritable<'f>(
        plan: &Vec<usize>,
        fill: &'f mut Vec<String>,
        field: &str,
    ) -> Option<&'f mut String> {
        let at = index(field);
        if plan.contains(&at) {
            return None;
        }
        fill.get_mut(at)
    }

    fn carry_out(&self, plan: Vec<usize>, fill: &mut Vec<String>) {
        for at in plan {
            fill[at] = self.pad.to_owned();
        }
    }
}

fn mends(fill: &[&str], pad: &'static str) -> (Vec<String>, Vec<(String, Mended)>, usize) {
    let fill = fill.iter().map(|value| (*value).to_owned()).collect();
    let (fill, mends, passes) = mend(&Lines { pad }, fill);
    let mends = mends
        .into_iter()
        .map(|mend| (mend.field, mend.done))
        .collect();
    (fill, mends, passes)
}

#[test]
fn a_clean_fill_takes_no_pass() {
    assert_eq!(
        mends(&["fine"], "pads"),
        (vec!["fine".to_owned()], vec![], 0)
    );
}

#[test]
fn a_value_is_rewritten_in_place_to_its_first_line_and_then_cut_to_its_cap() {
    let (fill, done, passes) = mends(&["\nfirst line\nsecond", "fine"], "pads");
    assert_eq!(fill, ["firs", "fine"]);
    assert_eq!(
        done,
        [
            ("0".to_owned(), Mended::FirstLine),
            ("0".to_owned(), Mended::Cut)
        ]
    );
    assert_eq!(passes, 1);
}

#[test]
fn a_slot_the_plan_covers_is_not_rewritten_first() {
    // Too short and on two lines: the plan fills it in whole, so keeping its
    // first line would be a repair the same pass undoes.
    let (fill, done, passes) = mends(&["a\nb"], "pads");
    assert_eq!(fill, ["pads"]);
    assert_eq!(done, [("0".to_owned(), Mended::Padded)]);
    assert_eq!(passes, 1);
}

#[test]
fn a_repair_the_check_still_refuses_is_repaired_again_on_the_next_pass() {
    // Cut to its first line, and that line is too short to keep.
    let (fill, done, passes) = mends(&["ab\nc"], "pads");
    assert_eq!(fill, ["pads"]);
    assert_eq!(
        done,
        [
            ("0".to_owned(), Mended::FirstLine),
            ("0".to_owned(), Mended::Padded)
        ]
    );
    assert_eq!(passes, 2);
}

#[test]
fn the_bound_is_a_stop_when_a_repair_never_settles() {
    // A pad too short to keep fills a value with another value too short to
    // keep, so only the pass count ends it.
    let (fill, _, passes) = mends(&["a"], "b");
    assert_eq!(passes, Lines::MEND_PASSES);
    assert_eq!(fill, ["b"]);
}
