use std::time::{Duration, Instant};

use warlock_engine::agent;

use super::{Ending, Thread, Turn, ending_for};
use crate::account::{Account, Line, Outcome};
use crate::claude::Activity;

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

fn tool(name: &str, detail: &str) -> Activity {
    Activity::Tool {
        name: name.to_owned(),
        detail: Some(detail.to_owned()),
    }
}

fn said(thread: &Thread, now: Instant) -> Vec<String> {
    thread
        .lines(now)
        .into_iter()
        .map(|line| match line {
            // Never a directory heading, which is an account's row: it is
            // here so this helper words every row of the panel and not
            // most of them.
            Line::Directory { path } => path.display().to_string(),
            Line::Clocked { clock, text } => format!("{clock} {text}"),
            // A thread holds a conversation, not rows: a wrapped row is
            // made on the way to a panel of some width.
            Line::Summary { text }
            | Line::Text { text }
            | Line::Said { text }
            | Line::Note { text }
            | Line::Wrapped { text, .. } => text,
        })
        .collect()
}

fn work(thread: &Thread, now: Instant) -> Vec<String> {
    thread
        .lines(now)
        .into_iter()
        .filter_map(|line| match line {
            Line::Clocked { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn every_row_says_which_of_the_threads_own_texts_it_was_drawn_from() {
    let base = Instant::now();
    let mut thread = Thread::new();
    thread.ask("what does\nthe engine do?", base);
    thread.record(&Activity::Thinking, at(base, 2));
    thread.answer("It pacts a tree.\n\nSlowly.", at(base, 42));
    thread.note("no such command", at(base, 43));
    thread.ask("a question with no answer yet", at(base, 44));

    let now = at(base, 50);
    let pieces = thread.pieces();
    let sourced = thread.sourced(now);

    // Row for row the rows the panel is drawn from, in the same order: the
    // tags are what `lines` throws away and nothing else.
    assert_eq!(
        sourced
            .iter()
            .map(|row| row.line.clone())
            .collect::<Vec<_>>(),
        thread.lines(now),
    );

    for row in sourced {
        let text = pieces[row.piece];
        if row.work {
            // A work row is nobody's text and stands for the end of the
            // question above it.
            assert_eq!(row.offset, text.len(), "{row:?}");
            continue;
        }
        let drawn = crate::wrap::shape(&row.line).text;
        assert_eq!(
            text[row.offset..row.offset + drawn.len()],
            *drawn,
            "{row:?}"
        );
    }
}

#[test]
fn a_conversation_nobody_has_started_draws_nothing() {
    let base = Instant::now();
    let thread = Thread::new();

    assert!(thread.is_empty());
    assert_eq!(thread.line_count(), 0);
    assert_eq!(thread.lines(at(base, 30)), Vec::new());
    assert!(thread.turns().is_empty());
    assert!(thread.in_flight().is_none());
}

#[test]
fn a_whole_turn_reads_as_question_then_work_then_answer() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("what does the engine do?", base);
    thread.record(&Activity::Thinking, at(base, 2));
    thread.record(&tool("grep", "pact_directory"), at(base, 8));
    thread.record(
        &tool("read", "crates/warlock-engine/src/pact.rs"),
        at(base, 11),
    );
    thread.record(&Activity::Writing { bytes: 0 }, at(base, 30));
    thread.record(&Activity::Cost { usd: 0.02 }, at(base, 31));
    thread.answer("It pacts a tree.", at(base, 42));

    assert_eq!(
        said(&thread, at(base, 90)),
        vec![
            "what does the engine do?".to_owned(),
            // Each clock is the instant the next line arrived, measured
            // from the question rather than from the session.
            "0:08 thinking".to_owned(),
            "0:11 grep pact_directory".to_owned(),
            "0:30 read crates/warlock-engine/src/pact.rs".to_owned(),
            "0:42 writing".to_owned(),
            "It pacts a tree.".to_owned(),
        ],
    );
    assert_eq!(thread.line_count(), 6);
    assert_eq!(thread.lines(at(base, 90)).len(), thread.line_count());
}

#[test]
fn a_turn_that_has_heard_nothing_ticks_waiting_until_the_model_speaks() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("why?", base);

    // On screen from the moment the question is submitted: the clock is
    // already moving with nothing recorded, which is what says the silence
    // is a model and not a hang.
    assert_eq!(thread.line_count(), 2);
    assert_eq!(
        said(&thread, at(base, 12)),
        vec!["why?".to_owned(), "0:12 waiting".to_owned()],
    );

    // The first real line takes its place, on the same clock.
    thread.record(&Activity::Thinking, at(base, 58));
    assert_eq!(
        said(&thread, at(base, 60)),
        vec!["why?".to_owned(), "1:00 thinking".to_owned()],
    );
}

#[test]
fn the_newest_work_line_ticks_with_now_and_freezes_when_the_answer_lands() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("go on", base);
    thread.record(&Activity::Thinking, at(base, 1));

    assert_eq!(work(&thread, at(base, 5)), vec!["thinking".to_owned()]);
    assert_eq!(
        said(&thread, at(base, 5))[1],
        "0:05 thinking",
        "a live turn's newest line counts up with the caller's now",
    );
    assert_eq!(said(&thread, at(base, 9))[1], "0:09 thinking");

    thread.answer("done", at(base, 20));

    // Frozen where the answer landed, however long the session runs on.
    assert_eq!(said(&thread, at(base, 600))[1], "0:20 thinking");
    assert!(thread.in_flight().is_none());
}

#[test]
fn every_turn_starts_its_clock_again_at_zero() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("first", at(base, 10));
    thread.record(&Activity::Thinking, at(base, 12));
    thread.ask("second", at(base, 70));
    thread.record(&Activity::Thinking, at(base, 71));

    // The first turn's line froze when the second question was asked, one
    // minute in; the second's counts from its own question.
    assert_eq!(
        said(&thread, at(base, 75)),
        vec![
            "first".to_owned(),
            "1:00 thinking".to_owned(),
            "second".to_owned(),
            "0:05 thinking".to_owned(),
        ],
    );
    assert_eq!(thread.turns().len(), 2);
    assert_eq!(thread.turns()[0].message(), "first");
    assert_eq!(thread.in_flight().map(Turn::started), Some(at(base, 70)));
}

#[test]
fn a_stretch_of_thinking_is_one_line_however_often_it_is_reported() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("hello", base);
    for second in [2, 5, 9, 40] {
        thread.record(&Activity::Thinking, at(base, second));
    }

    assert_eq!(
        said(&thread, at(base, 55)),
        vec!["hello".to_owned(), "0:55 thinking".to_owned()],
        "one line, counting from when thinking started",
    );

    // Something else happening ends the stretch, and thinking after it is a
    // new one — the two are separated by what came between them.
    thread.record(&tool("read", "src/lib.rs"), at(base, 60));
    thread.record(&Activity::Thinking, at(base, 61));
    thread.record(&Activity::Thinking, at(base, 65));

    assert_eq!(
        work(&thread, at(base, 70)),
        vec![
            "thinking".to_owned(),
            "read src/lib.rs".to_owned(),
            "thinking".to_owned(),
        ],
    );
}

#[test]
fn a_tool_without_a_detail_is_its_bare_name() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("look around", base);
    thread.record(
        &Activity::Tool {
            name: "glob".to_owned(),
            detail: None,
        },
        at(base, 3),
    );

    assert_eq!(work(&thread, at(base, 4)), vec!["glob".to_owned()]);
}

#[test]
fn what_a_turn_cost_is_heard_and_never_drawn() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("what is this?", base);
    thread.record(&Activity::Cost { usd: 0.01 }, at(base, 1));
    thread.record(&Activity::Cost { usd: 0.014 }, at(base, 2));

    // Not a work line, and not a line anywhere else either: a report of
    // what a chat turn spent belongs to no total a reader can use, so the
    // card is the same card whether it arrives or not.
    assert_eq!(work(&thread, at(base, 3)), vec!["waiting".to_owned()]);

    thread.answer("this.", at(base, 4));
    assert_eq!(
        said(&thread, at(base, 5)),
        vec![
            "what is this?".to_owned(),
            "0:04 waiting".to_owned(),
            "this.".to_owned(),
        ],
    );

    // The same rows a turn that reported nothing draws.
    let mut silent = Thread::new();
    silent.ask("what is this?", base);
    silent.answer("this.", at(base, 4));
    assert_eq!(silent.lines(at(base, 5)), thread.lines(at(base, 5)));
}

#[test]
fn no_money_reaches_this_card_however_much_a_turn_reports() {
    let base = Instant::now();

    let mut thread = Thread::new();
    thread.ask("how much?", base);
    thread.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
    thread.answer("that much.", at(base, 2));

    // The same twenty-one cents on an account, which is the card money is
    // said on: what a pass produced and what it cost, totalled.
    let mut account = Account::new(base);
    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
    account.close_section(
        &Outcome::Wrote {
            document: "crates/engine/WARLOCK.md".into(),
            bytes: 2341,
        },
        at(base, 2),
    );
    account.finish(at(base, 3));

    let asked = said(&thread, at(base, 4));
    let pact_money: Vec<String> = account
        .lines(at(base, 4))
        .into_iter()
        .filter_map(|line| match line {
            Line::Clocked { text, .. } | Line::Summary { text } => {
                text.contains("$0.21").then_some(text)
            }
            _ => None,
        })
        .collect();

    // The account says what a pass produced and totals those; the question
    // says nothing about money at all, so there are not two numbers on one
    // screen for a reader to add up.
    assert_eq!(
        pact_money,
        vec![
            "wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21".to_owned(),
            "pact finished — 1 directory, 0:03, $0.21".to_owned(),
        ],
    );
    assert_eq!(
        asked,
        vec![
            "how much?".to_owned(),
            "0:02 waiting".to_owned(),
            "that much.".to_owned(),
        ],
    );
    assert!(!asked.iter().any(|line| line.contains('$')));
}

#[test]
fn no_tool_result_and_no_model_prose_ever_reaches_a_work_line() {
    let base = Instant::now();
    let mut thread = Thread::new();

    // Everything a turn can be told, including a tool whose detail is the
    // only thing it carries: none of it is a result, and none of it is
    // reasoning. The answer is the one piece of prose, and it is a text row
    // rather than a clocked one.
    thread.ask("summarise the engine", base);
    thread.record(&Activity::Thinking, at(base, 1));
    thread.record(&tool("grep", "fn pact_directory"), at(base, 4));
    thread.record(&tool("read", "src/pact.rs"), at(base, 6));
    thread.record(&Activity::Writing { bytes: 0 }, at(base, 9));
    thread.answer(
        "The engine walks a tree and writes a document per directory.",
        at(base, 20),
    );

    assert_eq!(
        work(&thread, at(base, 21)),
        vec![
            "thinking".to_owned(),
            "grep fn pact_directory".to_owned(),
            "read src/pact.rs".to_owned(),
            "writing".to_owned(),
        ],
    );
    for line in work(&thread, at(base, 21)) {
        assert!(
            !line.contains("engine walks"),
            "the answer's prose never appears among the work lines: {line}",
        );
    }
}

#[test]
fn an_answer_is_kept_whole_and_unwrapped() {
    let base = Instant::now();
    let mut thread = Thread::new();

    let long = "a ".repeat(120);
    thread.ask("say something long", base);
    thread.answer(format!("{long}\n\nand a second paragraph"), at(base, 5));

    // Stored exactly as it arrived: wrapping is the frame's business, and a
    // terminal made narrower should re-flow this rather than ask again.
    let answer = thread.turns()[0]
        .answer()
        .expect("the turn answered")
        .to_owned();
    assert_eq!(answer, format!("{long}\n\nand a second paragraph"));

    // Its own newlines are rows, though, because they are the answer's.
    assert_eq!(
        thread
            .lines(at(base, 6))
            .into_iter()
            .filter(|line| matches!(line, Line::Text { .. }))
            .count(),
        3,
    );
}

#[test]
fn a_cancelled_turn_keeps_what_arrived_and_gains_one_line() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("read the tree", base);
    thread.record(&tool("read", "a.rs"), at(base, 2));
    thread.record(&tool("read", "b.rs"), at(base, 4));
    thread.end(&Ending::Cancelled, at(base, 6));

    assert_eq!(
        said(&thread, at(base, 60)),
        vec![
            "read the tree".to_owned(),
            "0:04 read a.rs".to_owned(),
            "0:06 read b.rs".to_owned(),
            "0:06 the turn was cancelled".to_owned(),
        ],
    );
    assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
    assert!(thread.turns()[0].answer().is_none());
    assert!(thread.in_flight().is_none());
}

#[test]
fn every_failure_ends_the_turn_in_exactly_one_line() {
    let base = Instant::now();
    let endings = [
        (
            Ending::NoModel {
                program: "claude".to_owned(),
            },
            "the turn could not run — no claude on PATH",
        ),
        (
            Ending::Failed {
                code: Some(2),
                stderr: "not logged in\n  try `claude login`".to_owned(),
            },
            "the turn failed — exit status 2: not logged in try `claude login`",
        ),
        (
            Ending::Failed {
                code: None,
                stderr: String::new(),
            },
            "the turn failed — stopped before it exited",
        ),
        (
            Ending::TimedOut {
                after: Duration::from_secs(90),
            },
            "the turn ran past 90s and was stopped",
        ),
        (Ending::NothingSaid, "the turn ended with nothing to say"),
        (
            Ending::Broke {
                reason: "broken pipe".to_owned(),
            },
            "the turn could not run — broken pipe",
        ),
    ];

    for (ending, wording) in endings {
        let mut thread = Thread::new();
        thread.ask("go", base);
        thread.end(&ending, at(base, 3));

        assert_eq!(
            said(&thread, at(base, 30)),
            vec!["go".to_owned(), format!("0:03 {wording}")],
            "one question, one line, and nothing else",
        );
        assert_eq!(thread.turns()[0].ending(), Some(&ending));
        assert!(thread.in_flight().is_none(), "the session is free again");
    }
}

#[test]
fn a_model_that_finished_with_nothing_to_say_ends_rather_than_answers() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("well?", base);
    thread.answer("   \n  ", at(base, 4));

    // A blank turn on screen is indistinguishable from one still going, so
    // an empty answer is a failure with a line of its own.
    assert!(thread.turns()[0].answer().is_none());
    assert_eq!(thread.turns()[0].ending(), Some(&Ending::NothingSaid));
    assert_eq!(
        said(&thread, at(base, 30)),
        vec![
            "well?".to_owned(),
            "0:04 the turn ended with nothing to say".to_owned()
        ],
    );
}

#[test]
fn the_first_ending_wins_and_nothing_lands_on_a_closed_turn() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("stop", base);
    thread.end(&Ending::Cancelled, at(base, 2));

    // Everything after the cancel is dropped: a second ending, an answer
    // that was already on its way, and any activity still in the channel.
    thread.end(&Ending::NothingSaid, at(base, 3));
    thread.answer("too late", at(base, 4));
    thread.record(&Activity::Thinking, at(base, 5));
    thread.record(&Activity::Cost { usd: 9.99 }, at(base, 6));

    assert_eq!(
        said(&thread, at(base, 30)),
        vec!["stop".to_owned(), "0:02 the turn was cancelled".to_owned()],
    );
    assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
}

#[test]
fn nothing_can_be_filed_before_the_first_question() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.record(&Activity::Thinking, base);
    thread.answer("out of nowhere", at(base, 1));
    thread.end(&Ending::Cancelled, at(base, 2));

    assert!(thread.is_empty());
    assert_eq!(thread.lines(at(base, 3)), Vec::new());
}

#[test]
fn a_cancelled_turn_ends_in_the_line_that_cancelled_it_and_nothing_after() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("expensive?", base);
    thread.record(&Activity::Cost { usd: 0.4 }, at(base, 1));
    thread.end(&Ending::Cancelled, at(base, 2));

    assert_eq!(
        said(&thread, at(base, 30)),
        vec![
            "expensive?".to_owned(),
            "0:02 the turn was cancelled".to_owned(),
        ],
    );
}

#[test]
fn a_message_with_newlines_in_it_is_the_rows_it_was_typed_as() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("first line\nsecond line", base);

    assert_eq!(
        thread.lines(at(base, 1))[..2],
        [
            Line::Said {
                text: "first line".to_owned()
            },
            Line::Said {
                text: "second line".to_owned()
            },
        ],
    );
    assert_eq!(thread.line_count(), 3);
}

#[test]
fn a_window_gives_back_what_is_there_rather_than_what_was_asked_for() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("one", base);
    thread.answer("uno", at(base, 1));
    thread.ask("two", at(base, 2));
    thread.answer("dos", at(base, 3));

    assert_eq!(thread.line_count(), 6);
    assert_eq!(
        thread.window(3, 2, at(base, 4)),
        vec![
            Line::Said {
                text: "two".to_owned()
            },
            Line::Clocked {
                clock: "0:01".to_owned(),
                text: "waiting".to_owned()
            },
        ],
    );
    assert_eq!(thread.window(0, 99, at(base, 4)).len(), 6);
    assert_eq!(thread.window(99, 4, at(base, 4)), Vec::new());
}

#[test]
fn a_run_is_no_part_of_this_card_however_it_is_driven() {
    // The account is the run's card and this one has no idea it exists:
    // nothing here opens, feeds or closes a run, and there is no call that
    // could. What the conversation holds is what was typed into it.
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("what is in crates?", base);
    thread.record(&Activity::Thinking, at(base, 1));
    thread.answer("The engine and the front end.", at(base, 2));

    let mut account = Account::new(base);
    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 2));
    account.close_section(
        &Outcome::Wrote {
            document: "crates/engine/WARLOCK.md".into(),
            bytes: 2341,
        },
        at(base, 3),
    );
    account.finish(at(base, 4));

    // A whole run happened beside the conversation, and the conversation is
    // the one turn somebody asked for.
    assert_eq!(thread.turns().len(), 1);
    assert_eq!(
        said(&thread, at(base, 900)),
        vec![
            "what is in crates?".to_owned(),
            "0:02 thinking".to_owned(),
            "The engine and the front end.".to_owned(),
        ],
    );
    assert!(
        thread.in_flight().is_none(),
        "a run cannot mute a conversation it is no part of"
    );
}

#[test]
fn a_note_lands_between_the_turns_exactly_where_it_was_said() {
    let base = Instant::now();
    let mut thread = Thread::new();

    // Note, turn, note: warlock says something before anybody has asked
    // anything, and again after the answer.
    thread.note("commands are /brief, /write and /chat", base);
    thread.ask("what does the engine do?", at(base, 4));
    thread.record(&Activity::Thinking, at(base, 5));
    thread.answer("It walks the tree.", at(base, 8));
    thread.note(
        "crates/engine/WARLOCK.md is older than that answer",
        at(base, 9),
    );

    // One sequence, read in the order it happened: a note read out of
    // order is a note about the wrong thing.
    assert_eq!(
        thread.lines(at(base, 30)),
        vec![
            Line::Note {
                text: "commands are /brief, /write and /chat".to_owned(),
            },
            Line::Said {
                text: "what does the engine do?".to_owned(),
            },
            Line::Clocked {
                clock: "0:04".to_owned(),
                text: "thinking".to_owned(),
            },
            Line::Text {
                text: "It walks the tree.".to_owned(),
            },
            Line::Note {
                text: "crates/engine/WARLOCK.md is older than that answer".to_owned(),
            },
        ],
    );
    assert_eq!(thread.line_count(), 5);
    assert_eq!(thread.lines(at(base, 30)).len(), thread.line_count());

    // And the window cuts that same sequence, so a reader scrolled to the
    // middle sees the note where it belongs and not at either end.
    assert_eq!(
        thread.window(3, 2, at(base, 30)),
        vec![
            Line::Text {
                text: "It walks the tree.".to_owned(),
            },
            Line::Note {
                text: "crates/engine/WARLOCK.md is older than that answer".to_owned(),
            },
        ],
    );
}

#[test]
fn a_note_is_one_unclocked_row_and_never_a_turn() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.note("that is not a command", at(base, 3));

    // A card with a note on it has something to draw, so it is not empty —
    // a refusal before the first question is exactly that case.
    assert!(!thread.is_empty());
    assert_eq!(thread.line_count(), 1);
    assert_eq!(thread.started(), Some(at(base, 3)));
    assert!(thread.turns().is_empty(), "a note is nobody's question");
    assert!(thread.in_flight().is_none(), "and it asked nothing");

    // Its own row: not a work line, so there is no clock beside it at any
    // `now`, and not a `Said`, so warlock's voice is not the reader's.
    let row = Line::Note {
        text: "that is not a command".to_owned(),
    };
    assert_eq!(thread.lines(at(base, 3)), vec![row.clone()]);
    assert_eq!(thread.lines(at(base, 900)), vec![row]);
    assert_eq!(
        said(&thread, at(base, 900)),
        vec!["that is not a command".to_owned()]
    );
    assert!(work(&thread, at(base, 900)).is_empty());

    // One row however it is worded: a note that arrived with newlines in it
    // is flattened rather than spending two rows of the card.
    let mut wordy = Thread::new();
    wordy.note("that is not a command\n  try /brief", base);
    assert_eq!(wordy.line_count(), 1);
    assert_eq!(
        wordy.lines(base),
        vec![Line::Note {
            text: "that is not a command try /brief".to_owned(),
        }]
    );
}

#[test]
fn a_note_neither_opens_closes_nor_freezes_a_turn() {
    let base = Instant::now();
    let mut thread = Thread::new();

    thread.ask("read the tree", base);
    thread.record(&Activity::Thinking, at(base, 1));
    thread.note("crates/engine/WARLOCK.md changed under you", at(base, 4));

    // The turn is still the live one, and its clock is still moving: the
    // note happened beside the turn, not to it.
    assert_eq!(thread.in_flight().map(Turn::started), Some(base));
    assert_eq!(thread.turns().len(), 1);
    assert!(!thread.turns()[0].is_closed());
    assert_eq!(
        said(&thread, at(base, 20)),
        vec![
            "read the tree".to_owned(),
            "0:20 thinking".to_owned(),
            "crates/engine/WARLOCK.md changed under you".to_owned(),
        ],
        "the work line ticks on under a note that came after it",
    );

    // Everything still files under that turn, over the note's head.
    thread.answer("It is a tree.", at(base, 30));
    assert_eq!(thread.turns()[0].answer(), Some("It is a tree."));
    assert!(thread.in_flight().is_none());
    assert_eq!(
        said(&thread, at(base, 60)),
        vec![
            "read the tree".to_owned(),
            "0:30 thinking".to_owned(),
            "It is a tree.".to_owned(),
            "crates/engine/WARLOCK.md changed under you".to_owned(),
        ],
        "the note stays where it was said, under the turn it interrupted",
    );

    // And a question asked after it freezes the turn, not the note.
    thread.ask("what else?", at(base, 70));
    assert_eq!(thread.turns().len(), 2);
    assert_eq!(thread.in_flight().map(Turn::started), Some(at(base, 70)));
}

#[test]
fn the_seams_failures_become_the_panels_endings() {
    assert_eq!(
        ending_for(&agent::Error::NotFound {
            program: "claude".to_owned()
        }),
        Ending::NoModel {
            program: "claude".to_owned()
        },
    );
    assert_eq!(
        ending_for(&agent::Error::Failed {
            code: Some(1),
            stderr: "boom".to_owned(),
        }),
        Ending::Failed {
            code: Some(1),
            stderr: "boom".to_owned(),
        },
    );
    assert_eq!(
        ending_for(&agent::Error::TimedOut {
            after: Duration::from_secs(30)
        }),
        Ending::TimedOut {
            after: Duration::from_secs(30)
        },
    );
    assert_eq!(ending_for(&agent::Error::EmptyOutput), Ending::NothingSaid);

    // Anything the seam grows later arrives as whatever it says about
    // itself, rather than as a panic or a silent nothing.
    let broken = agent::Error::Io {
        source: std::io::Error::other("broken pipe"),
    };
    assert_eq!(
        ending_for(&broken),
        Ending::Broke {
            reason: broken.to_string()
        },
    );
}
