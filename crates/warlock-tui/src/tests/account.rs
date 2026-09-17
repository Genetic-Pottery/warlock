use std::path::Path;
use std::time::{Duration, Instant};

use super::{Account, Line, Outcome, Section, clock, size};
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

fn said(account: &Account, now: Instant) -> Vec<String> {
    account
        .lines(now)
        .into_iter()
        .map(|line| match line {
            Line::Directory { path } => path.display().to_string(),
            Line::Clocked { clock, text } => format!("{clock} {text}"),
            // An account never yields a document's line, a question or a
            // note; they are here so this helper words every row of the
            // panel and not most of them. Nor a wrapped row: an account
            // holds what happened, and how many rows that takes is the
            // panel's question.
            Line::Summary { text }
            | Line::Text { text }
            | Line::Said { text }
            | Line::Note { text }
            | Line::Wrapped { text, .. } => text,
        })
        .collect()
}

#[test]
fn a_run_that_has_done_nothing_draws_nothing() {
    let base = Instant::now();
    let account = Account::new(base);

    assert_eq!(account.line_count(), 0);
    assert_eq!(account.lines(at(base, 30)), Vec::new());
    assert!(account.sections().is_empty());
}

#[test]
fn every_section_starts_its_clock_again_at_zero() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", at(base, 10));
    account.record(&Activity::Thinking, at(base, 12));
    account.open_section("crates/tui", at(base, 70));
    account.record(&Activity::Thinking, at(base, 71));

    // The first section's line froze when the second section opened, at one
    // minute in; the second's counts from its own start, not the run's.
    assert_eq!(
        said(&account, at(base, 75)),
        vec![
            "crates/engine".to_owned(),
            "1:00 thinking".to_owned(),
            "crates/tui".to_owned(),
            "0:05 thinking".to_owned(),
        ],
    );
}

#[test]
fn a_section_that_has_heard_nothing_ticks_waiting_until_the_pass_speaks() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);

    // On screen from the moment the section opens: the clock is already
    // moving with nothing recorded, which is what says the silence is a
    // pass and not a hang.
    assert_eq!(account.line_count(), 2);
    assert_eq!(
        said(&account, at(base, 12)),
        vec!["crates/engine".to_owned(), "0:12 waiting".to_owned()],
    );

    // The first real line takes its place, on the same clock: the pass
    // was quiet for fifty-eight seconds and then it was thinking.
    account.record(&Activity::Thinking, at(base, 58));
    assert_eq!(
        said(&account, at(base, 60)),
        vec!["crates/engine".to_owned(), "1:00 thinking".to_owned()],
    );
}

#[test]
fn a_stretch_of_thinking_is_one_line_however_often_it_is_reported() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    // The stream says "still thinking" every few seconds; the panel says it
    // once, and lets the clock do the rest.
    for second in [2, 5, 9, 40] {
        account.record(&Activity::Thinking, at(base, second));
    }

    assert_eq!(
        said(&account, at(base, 55)),
        vec!["crates/engine".to_owned(), "0:55 thinking".to_owned()],
        "one line, counting from when thinking started",
    );

    // Something else happening ends the stretch, and thinking after it is a
    // new one — the two are separated by what came between them.
    account.record(&tool("Read", "src/lib.rs"), at(base, 60));
    account.record(&Activity::Thinking, at(base, 61));
    account.record(&Activity::Thinking, at(base, 65));

    assert_eq!(
        said(&account, at(base, 70)),
        vec![
            "crates/engine".to_owned(),
            "1:00 thinking".to_owned(),
            "1:01 Read src/lib.rs".to_owned(),
            "1:10 thinking".to_owned(),
        ],
    );
}

#[test]
fn a_pass_reads_as_thinking_and_then_as_writing() {
    // The whole of a toolless pass, which is what a pact runs now: a few
    // seconds of thinking, then the rest of the time producing the
    // document. Two lines, each clocked from when its own stretch began, so
    // the panel says which half the wait is being spent in.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 2));
    account.record(&Activity::Thinking, at(base, 3));
    account.record(&Activity::Writing { bytes: 0 }, at(base, 4));
    account.record(&Activity::Writing { bytes: 0 }, at(base, 9));

    assert_eq!(
        said(&account, at(base, 25)),
        vec![
            "crates/engine".to_owned(),
            // Thinking froze when writing began, four seconds in.
            "0:04 thinking".to_owned(),
            // And writing is the live line, counting from when it started.
            "0:25 writing".to_owned(),
        ],
    );
}

#[test]
fn the_writing_line_is_reworded_as_the_answer_arrives() {
    // The long half of a toolless pass, with the count on it: one line,
    // reworded where it stands, so the panel moves while the answer is
    // being produced instead of holding one motionless word for minutes.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 2));
    // The text block opening, before a single delta: the bare word, which
    // is what the panel showed at the first token before there was a count
    // to show.
    account.record(&Activity::Writing { bytes: 0 }, at(base, 8));
    assert_eq!(
        said(&account, at(base, 8)),
        vec![
            "crates/engine".to_owned(),
            "0:08 thinking".to_owned(),
            "0:08 writing".to_owned(),
        ],
    );

    for (bytes, second) in [(212, 20), (1_843, 60), (48_000, 200)] {
        account.record(&Activity::Writing { bytes }, at(base, second));
    }

    assert_eq!(
        said(&account, at(base, 260)),
        vec![
            "crates/engine".to_owned(),
            // Thinking still froze when writing began, and nothing since
            // has moved it: the rewrites are not arrivals.
            "0:08 thinking".to_owned(),
            // One line, the newest count, and a clock counting from the
            // first report rather than from the last one.
            "4:20 writing · 47 KB".to_owned(),
        ],
    );
}

#[test]
fn a_writing_line_under_a_kilobyte_counts_in_bytes() {
    // The short answer, worded exactly rather than rounded to `0.9 KB`.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Writing { bytes: 0 }, at(base, 3));
    account.record(&Activity::Writing { bytes: 934 }, at(base, 5));

    assert_eq!(said(&account, at(base, 6))[1], "0:06 writing · 934 bytes");
}

#[test]
fn writing_after_something_else_opens_a_line_with_its_own_count() {
    // A stretch that ends and begins again — a tool call between two text
    // blocks — counts its own block's bytes from its own instant, rather
    // than continuing the earlier line's total.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Writing { bytes: 2_048 }, at(base, 10));
    account.record(&tool("Read", "src/lib.rs"), at(base, 30));
    account.record(&Activity::Writing { bytes: 512 }, at(base, 40));
    account.record(&tool("Read", "src/big.rs"), at(base, 50));
    account.record(&Activity::Writing { bytes: 100 }, at(base, 60));

    assert_eq!(
        said(&account, at(base, 90)),
        vec![
            "crates/engine".to_owned(),
            "0:30 writing · 2.0 KB".to_owned(),
            "0:40 Read src/lib.rs".to_owned(),
            "0:50 writing · 512 bytes".to_owned(),
            "1:00 Read src/big.rs".to_owned(),
            "1:30 writing · 100 bytes".to_owned(),
        ],
        "each stretch its own line, its own count and its own clock",
    );
}

#[test]
fn a_waiting_line_freezes_with_the_section_it_waits_in() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.open_section("crates/tui", at(base, 30));

    // The first section froze when the second opened, still with nothing
    // heard, so its waiting line stops where the pass stopped — however
    // late the frame is drawn.
    for now in [at(base, 30), at(base, 900)] {
        assert_eq!(said(&account, now)[1], "0:30 waiting");
    }
    // While the live section's own waiting line goes on ticking.
    assert_eq!(said(&account, at(base, 45))[3], "0:15 waiting");
}

#[test]
fn the_newest_line_counts_up_with_the_now_it_is_asked_about() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 1));

    let ticking: Vec<_> = [1, 9, 65, 3_600]
        .into_iter()
        .map(|seconds| said(&account, at(base, seconds)).join("|"))
        .collect();

    assert_eq!(
        ticking,
        vec![
            "crates/engine|0:01 thinking".to_owned(),
            "crates/engine|0:09 thinking".to_owned(),
            "crates/engine|1:05 thinking".to_owned(),
            "crates/engine|60:00 thinking".to_owned(),
        ],
    );
}

#[test]
fn a_line_freezes_at_the_instant_the_line_beneath_it_arrived() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 2));
    account.record(&tool("Read", "src/lib.rs"), at(base, 20));

    // The thinking line stopped where it had got to — 0:20, where the read
    // arrived — and stays there however late the frame is drawn.
    for now in [at(base, 20), at(base, 40), at(base, 900)] {
        assert_eq!(said(&account, now)[1], "0:20 thinking");
    }

    // While the line beneath it, being the newest, goes on ticking.
    assert_eq!(said(&account, at(base, 40))[2], "0:40 Read src/lib.rs");
    assert_eq!(said(&account, at(base, 41))[2], "0:41 Read src/lib.rs");
}

#[test]
fn a_carried_section_says_unchanged_and_names_no_write_and_no_cost() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // A directory the run carried forward: it opened a section, because the
    // engine announced it like any other, and then closed it having spent
    // nothing and written nothing.
    account.open_section("crates/engine", base);
    account.close_section(
        &Outcome::Unchanged {
            document: "crates/engine/WARLOCK.md".into(),
        },
        at(base, 0),
    );

    let line = said(&account, at(base, 5))
        .into_iter()
        .find(|line| line.contains("crates/engine/WARLOCK.md"))
        .expect("the section closed with a line naming its document");
    assert!(
        line.contains("unchanged"),
        "the word a reader learns this from, and the one `wrote` would \
             have lied about: {line}",
    );
    assert!(
        !line.contains("wrote"),
        "nothing was written, so nothing may say it was: {line}",
    );
    assert!(
        !line.contains('$') && !line.contains("cost"),
        "and a carried directory has no spend to report at all: {line}",
    );
}

#[test]
fn a_skipped_section_names_the_failure_below_it_and_claims_no_write() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.close_section(
        &Outcome::Skipped {
            below: "crates/engine/src".into(),
        },
        at(base, 0),
    );

    let line = said(&account, at(base, 5))
        .into_iter()
        .find(|line| line.contains("skipped"))
        .expect("the section closed saying it was skipped");
    assert!(
        line.contains("crates/engine/src"),
        "on its own `skipped` reads as the run losing interest; the \
             directory below it is the half that explains it: {line}",
    );
    assert!(
        !line.contains("wrote") && !line.contains('$'),
        "no pass ran, so there is no write and no spend to report: {line}",
    );
}

#[test]
fn a_section_that_ran_no_pass_is_no_part_of_the_incomplete_count() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // One pass that said what it cost, and two directories no pass ran for
    // at all: one carried forward, one skipped over a failure below it.
    account.open_section("crates/alpha", base);
    account.record(&Activity::Cost { usd: 0.25 }, at(base, 1));
    account.close_section(
        &Outcome::Wrote {
            document: "crates/alpha/WARLOCK.md".into(),
            bytes: 1_200,
        },
        at(base, 10),
    );
    account.open_section("crates/beta", at(base, 10));
    account.close_section(
        &Outcome::Unchanged {
            document: "crates/beta/WARLOCK.md".into(),
        },
        at(base, 10),
    );
    account.open_section("crates", at(base, 10));
    account.close_section(
        &Outcome::Skipped {
            below: "crates/gamma".into(),
        },
        at(base, 10),
    );
    account.finish(at(base, 60));

    assert_eq!(
        said(&account, at(base, 60)).last().map(String::as_str),
        Some("pact finished — 3 directories, 1:00, $0.25"),
        "a directory no pass ran for has no cost to have gone missing, so \
             the total is whole rather than `incomplete`",
    );
}

#[test]
fn a_closed_section_stops_ticking_for_good() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 2));
    account.close_section(
        &Outcome::Wrote {
            document: "crates/engine/WARLOCK.md".into(),
            bytes: 2_341,
        },
        at(base, 30),
    );

    let early = said(&account, at(base, 30));
    assert_eq!(early, said(&account, at(base, 4_000)));
    assert_eq!(early[1], "0:30 thinking");
    assert!(early[2].starts_with("0:30 wrote"));
    assert!(account.sections()[0].is_closed());
}

#[test]
fn ending_a_run_freezes_the_section_that_was_still_live() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 1));
    account.finish(at(base, 12));

    assert_eq!(said(&account, at(base, 999))[1], "0:12 thinking");
}

#[test]
fn a_tool_is_its_name_and_the_one_detail_it_carries() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&tool("Read", "src/lib.rs"), at(base, 1));
    account.record(
        &Activity::Tool {
            name: "WebFetch".to_owned(),
            detail: None,
        },
        at(base, 2),
    );
    account.record(&Activity::Thinking, at(base, 3));

    assert_eq!(
        said(&account, at(base, 3)),
        vec![
            "crates/engine".to_owned(),
            "0:02 Read src/lib.rs".to_owned(),
            "0:03 WebFetch".to_owned(),
            "0:03 thinking".to_owned(),
        ],
    );
}

#[test]
fn a_pushed_line_freezes_and_ticks_like_any_other() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 1));

    // Its arrival freezes the line above it, at the instant it arrived.
    account.record_waiting(11, 34 * 1024, at(base, 20));
    for now in [at(base, 20), at(base, 60), at(base, 900)] {
        assert_eq!(said(&account, now)[1], "0:20 thinking");
    }

    // And, being the newest, it counts up until something newer lands.
    assert_eq!(
        said(&account, at(base, 40))[2],
        "0:40 waiting · 11 files, 34 KB"
    );
    assert_eq!(
        said(&account, at(base, 41))[2],
        "0:41 waiting · 11 files, 34 KB"
    );
    account.record(&Activity::Writing { bytes: 0 }, at(base, 50));
    for now in [at(base, 50), at(base, 900)] {
        assert_eq!(said(&account, now)[2], "0:50 waiting · 11 files, 34 KB");
    }

    // A section frozen by the next one opening stops with a pushed line
    // under it exactly as it stops with anything else.
    account.open_section("crates/tui", at(base, 80));
    assert_eq!(said(&account, at(base, 4_000))[3], "1:20 writing");
}

#[test]
fn the_files_of_one_directory_are_one_line_that_counts_up() {
    // A directory of eighteen moved files would be eighteen near-identical
    // lines in the run's record if each were filed. One line instead,
    // reworded where it stands, so the clock on it counts the whole stretch
    // of file passes rather than restarting at every one.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine/src", base);
    account.record_describing(1, 3, 1_024, at(base, 10));
    // Ticking, because nothing newer has landed.
    assert_eq!(
        said(&account, at(base, 25))[1],
        "0:25 describing · 1/3 files, 1.0 KB"
    );

    // The rewording keeps the entry that is already on screen, instant and
    // all, and the bytes are the ones this directory has been handed so
    // far rather than the newest file's alone.
    account.record_describing(2, 3, 1_024, at(base, 40));
    account.record_describing(3, 3, 2_048, at(base, 70));
    assert_eq!(
        said(&account, at(base, 90)),
        [
            "crates/engine/src".to_owned(),
            "1:30 describing · 3/3 files, 4.0 KB".to_owned(),
        ],
        "the stretch is one line, clocked from the first file"
    );

    // And the handover to the synthesis is a line of its own, not a fourth
    // rewording: it is a different kind of wait, and it freezes the stretch
    // above it where that stretch stopped.
    account.record_waiting(3, 194, at(base, 100));
    assert_eq!(
        said(&account, at(base, 300)),
        [
            "crates/engine/src".to_owned(),
            "1:40 describing · 3/3 files, 4.0 KB".to_owned(),
            "5:00 waiting · 3 files, 194 bytes".to_owned(),
        ]
    );
}

#[test]
fn the_files_of_the_next_directory_open_a_line_of_their_own() {
    // The running total is the section's, not the account's: a new section
    // starts its count at nothing, or the second directory would report the
    // first one's bytes as well as its own.
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine/src", base);
    account.record_describing(1, 1, 4_096, at(base, 10));
    account.open_section("crates/tui/src", at(base, 30));
    account.record_describing(1, 1, 1_024, at(base, 40));

    assert_eq!(
        said(&account, at(base, 60))[3],
        "0:30 describing · 1/1 file, 1.0 KB",
        "the second directory counted the first one's bytes"
    );
}

#[test]
fn a_size_is_spelled_with_the_digits_its_magnitude_deserves() {
    // Exact under a kilobyte, one decimal under ten of a unit, whole units
    // above that.
    assert_eq!(size(0), "0 bytes");
    assert_eq!(size(1), "1 byte");
    assert_eq!(size(934), "934 bytes");
    assert_eq!(size(1_843), "1.8 KB");
    assert_eq!(size(34 * 1024), "34 KB");
    assert_eq!(size(403 * 1024), "403 KB");
    assert_eq!(size(1_677_722), "1.6 MB");
    assert_eq!(size(12 * 1024 * 1024), "12 MB");
}

#[test]
fn a_size_never_rounds_itself_across_a_boundary() {
    // The unit is chosen by magnitude and the rounding happens inside it,
    // so no number ever claims to have reached the next unit or the next
    // tier by being rounded into it.
    assert_eq!(size(1_023), "1023 bytes");
    assert_eq!(size(1_024), "1.0 KB");
    assert_eq!(size(10_239), "10.0 KB", "one decimal still, not 10 KB");
    assert_eq!(size(10_240), "10 KB");
    assert_eq!(size(1_048_575), "1024 KB", "not 1.0 MB");
    assert_eq!(size(1_048_576), "1.0 MB");
}

#[test]
fn the_handed_over_request_replaces_the_waiting_placeholder() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // The section opens when the directory comes up, and the placeholder
    // covers reading it off disk: one second of ticking with nothing filed.
    account.open_section("crates/warlock-tui/src", base);
    assert_eq!(
        said(&account, at(base, 1)),
        vec![
            "crates/warlock-tui/src".to_owned(),
            "0:01 waiting".to_owned(),
        ],
    );

    // Then the request is handed over, and the wait that follows is the
    // pass's own rather than the disk's. The placeholder is drawn rather
    // than stored, so it is not left above the entry: there is an entry now,
    // and `entries.is_empty()` being false is the whole of what stops it
    // being drawn. Nothing was stored for it and nothing was deleted — the
    // account is the same two rows it always was.
    account.record_waiting(11, 1_677_722, at(base, 1));
    assert_eq!(
        said(&account, at(base, 60)),
        vec![
            "crates/warlock-tui/src".to_owned(),
            "1:00 waiting · 11 files, 1.6 MB".to_owned(),
        ],
        "one row, the entry, ticking by the ordinary rule",
    );
    assert_eq!(account.line_count(), 2);

    // And the stretch this line covers is the pass's silence: it froze when
    // the pass first spoke, a minute after the handover a second in.
    account.record(&Activity::Thinking, at(base, 61));
    assert_eq!(
        said(&account, at(base, 900))[1],
        "1:01 waiting · 11 files, 1.6 MB",
    );
}

#[test]
fn a_handed_over_request_lands_below_whatever_spoke_before_it() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Anything filed before the handover means the placeholder never
    // appears at all, and the request line lands under it like any other.
    account.open_section("crates/engine", base);
    account.record(&tool("Read", "Cargo.lock"), at(base, 10));
    account.record(&tool("Read", "src/lib.rs"), at(base, 70));
    account.record_waiting(11, 34 * 1024, at(base, 130));
    account.record(&Activity::Thinking, at(base, 190));

    assert_eq!(
        said(&account, at(base, 200)),
        vec![
            "crates/engine".to_owned(),
            "1:10 Read Cargo.lock".to_owned(),
            "2:10 Read src/lib.rs".to_owned(),
            // Frozen where thinking began: the wait for this pass's first
            // word was a minute.
            "3:10 waiting · 11 files, 34 KB".to_owned(),
            "3:20 thinking".to_owned(),
        ],
    );
}

#[test]
fn a_handed_over_request_is_collapsed_into_nothing_and_swallows_nothing() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Its text differs from the bare `waiting` of the placeholder and from
    // whatever follows it, so `extend_or_open` has nothing to fold: the
    // repeated report after it opens its own line, and a second request
    // line would too.
    account.open_section("crates/engine", base);
    account.record_waiting(1, 934, at(base, 1));
    account.record(&Activity::Thinking, at(base, 2));
    account.record(&Activity::Thinking, at(base, 3));

    assert_eq!(
        said(&account, at(base, 10)),
        vec![
            "crates/engine".to_owned(),
            // One file, said as one file.
            "0:02 waiting · 1 file, 934 bytes".to_owned(),
            "0:10 thinking".to_owned(),
        ],
    );
}

#[test]
fn no_request_line_is_filed_where_there_is_no_live_section() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Before the first directory, and after the current one has been worded
    // and frozen: the same silence `record` keeps.
    account.record_waiting(11, 34 * 1024, at(base, 1));
    assert_eq!(account.line_count(), 0);

    account.open_section("crates/engine", at(base, 2));
    account.close_section(&Outcome::Cancelled, at(base, 3));
    account.record_waiting(11, 34 * 1024, at(base, 4));

    assert_eq!(
        said(&account, at(base, 9)),
        vec![
            "crates/engine".to_owned(),
            "0:01 cancelled — nothing reported spent".to_owned(),
        ],
    );
}

#[test]
fn a_mend_is_one_line_under_the_directory_it_was_done_to() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // The rejections that ran out, then what warlock did about it: one line
    // per mend, in the engine's own words, clocked and frozen like any
    // other. Nothing here counts the mends or folds two of them together —
    // two slots were mended and the panel says so twice.
    account.open_section("crates/warlock-tui/src", base);
    account.record_rejected(&["purpose is empty".to_owned()], 3, 3, at(base, 10));
    account.record_repaired(
        r#"files["writing.rs"] was 320 characters and was cut to 280"#,
        at(base, 20),
    );
    account.record_repaired(
        "purpose was not answered and was filled in from what warlock measured",
        at(base, 30),
    );

    // Each line is clocked where the next one froze it, by the ordinary
    // rule, and the newest goes on ticking.
    assert_eq!(
        said(&account, at(base, 40)),
        vec![
            "crates/warlock-tui/src".to_owned(),
            "0:20 rejected · attempt 3/3: purpose is empty".to_owned(),
            r#"0:30 repaired · files["writing.rs"] was 320 characters and was cut to 280"#
                .to_owned(),
            "0:40 repaired · purpose was not answered and was filled in from what warlock measured"
                .to_owned(),
        ],
    );
}

#[test]
fn no_repair_line_is_filed_where_there_is_no_live_section() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // A repair arriving before the first directory, and one arriving after
    // the current directory has been worded and frozen: the same silence
    // `record` and `record_rejected` keep.
    account.record_repaired("purpose was dropped", at(base, 1));
    assert_eq!(account.line_count(), 0);

    account.open_section("crates/engine", at(base, 2));
    account.close_section(&Outcome::Cancelled, at(base, 3));
    account.record_repaired("purpose was dropped", at(base, 4));

    assert_eq!(
        said(&account, at(base, 9)),
        vec![
            "crates/engine".to_owned(),
            "0:01 cancelled — nothing reported spent".to_owned(),
        ],
    );
}

#[test]
fn a_cost_is_arithmetic_rather_than_a_line() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
    account.record(&Activity::Cost { usd: 0.04 }, at(base, 2));

    // No line of its own: the section still reads as waiting for the pass
    // to be seen doing something, and the money was kept.
    assert_eq!(account.line_count(), 2);
    assert_eq!(
        said(&account, at(base, 2)),
        vec!["crates/engine".to_owned(), "0:02 waiting".to_owned()]
    );
    assert_eq!(account.sections()[0].cost(), Some(0.25));
}

#[test]
fn nothing_is_filed_where_there_is_no_live_section() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Before the first directory.
    account.record(&Activity::Thinking, at(base, 1));
    assert_eq!(account.line_count(), 0);

    // And after the current one has been worded and frozen.
    account.open_section("crates/engine", at(base, 2));
    account.close_section(&Outcome::Cancelled, at(base, 3));
    account.record(&Activity::Thinking, at(base, 4));
    account.record(&Activity::Cost { usd: 9.99 }, at(base, 5));
    account.close_section(&Outcome::Cancelled, at(base, 6));

    assert_eq!(
        said(&account, at(base, 7)),
        vec![
            "crates/engine".to_owned(),
            "0:01 cancelled — nothing reported spent".to_owned(),
        ],
    );
    assert_eq!(account.sections()[0].cost(), None);
}

#[test]
fn a_written_document_says_what_it_wrote_how_big_and_what_it_cost() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 0.2149 }, at(base, 1));
    account.close_section(
        &Outcome::Wrote {
            document: "crates/engine/WARLOCK.md".into(),
            bytes: 2_341,
        },
        at(base, 42),
    );

    assert_eq!(
        said(&account, at(base, 42))[1],
        "0:42 wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21",
    );
}

#[test]
fn a_written_document_whose_pass_never_priced_itself_says_so() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.close_section(
        &Outcome::Wrote {
            document: "crates/engine/WARLOCK.md".into(),
            bytes: 12,
        },
        at(base, 5),
    );

    assert_eq!(
        said(&account, at(base, 5))[1],
        "0:05 wrote crates/engine/WARLOCK.md — 12 bytes, no cost reported",
    );
}

#[test]
fn a_refusal_says_why_it_was_refused() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.close_section(
        &Outcome::Refused {
            reason: "the model returned an empty document".to_owned(),
        },
        at(base, 7),
    );

    assert_eq!(
        said(&account, at(base, 7))[1],
        "0:07 refused — the model returned an empty document",
    );
}

#[test]
fn a_cancellation_says_what_it_had_spent_by_then() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 0.03 }, at(base, 1));
    account.close_section(&Outcome::Cancelled, at(base, 65));

    assert_eq!(
        said(&account, at(base, 65))[1],
        "1:05 cancelled — $0.03 spent"
    );
}

#[test]
fn the_summary_counts_directories_wall_clock_and_money() {
    let base = Instant::now();
    let mut account = Account::new(base);

    for (index, directory) in ["crates/engine", "crates/tui"].into_iter().enumerate() {
        let opened = at(base, index as u64 * 60);
        account.open_section(directory, opened);
        account.record(&Activity::Cost { usd: 0.935 }, opened);
        account.close_section(
            &Outcome::Wrote {
                document: "WARLOCK.md".into(),
                bytes: 10,
            },
            opened + Duration::from_secs(30),
        );
    }
    account.finish(at(base, 252));

    assert_eq!(
        said(&account, at(base, 252)).last().map(String::as_str),
        Some("pact finished — 2 directories, 4:12, $1.87"),
    );
}

#[test]
fn one_directory_is_said_in_the_singular() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 1.0 }, at(base, 1));
    account.finish(at(base, 9));

    assert_eq!(
        said(&account, at(base, 9)).last().map(String::as_str),
        Some("pact finished — 1 directory, 0:09, $1.00"),
    );
}

#[test]
fn a_pass_that_never_reported_a_cost_makes_the_total_incomplete() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 1.65 }, at(base, 1));
    account.open_section("crates/tui", at(base, 2));
    account.open_section("crates/cli", at(base, 3));
    account.finish(at(base, 60));

    // Two passes said nothing, so the number is a floor rather than a total,
    // and the line says which.
    assert_eq!(
        said(&account, at(base, 60)).last().map(String::as_str),
        Some("pact finished — 3 directories, 1:00, $1.65 (incomplete: 2 passes reported no cost)"),
    );
}

#[test]
fn one_pass_missing_a_cost_is_said_in_the_singular_too() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.finish(at(base, 1));

    assert_eq!(
        said(&account, at(base, 1)).last().map(String::as_str),
        Some("pact finished — 1 directory, 0:01, $0.00 (incomplete: 1 pass reported no cost)"),
    );
}

#[test]
fn every_section_is_closed_at_the_end_with_what_is_said_about_it() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Two passes, one after the other, with nothing closing either of them
    // at the time: how a directory went is not known until the run ends.
    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 1));
    account.open_section("crates/tui", at(base, 30));
    account.record(&Activity::Thinking, at(base, 31));

    account.close_open_sections(at(base, 90), |section| {
        if section.directory() == Path::new("crates/engine") {
            Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2_341,
            }
        } else {
            Outcome::Refused {
                reason: "the model returned an empty document".to_owned(),
            }
        }
    });

    // The first section's ending is dated where that pass stopped — thirty
    // seconds in, where the next directory opened — and the second's, being
    // the one still live, is dated now.
    assert_eq!(
        said(&account, at(base, 900)),
        vec![
            "crates/engine".to_owned(),
            "0:30 thinking".to_owned(),
            "0:30 wrote crates/engine/WARLOCK.md — 2341 bytes, no cost reported".to_owned(),
            "crates/tui".to_owned(),
            "1:00 thinking".to_owned(),
            "1:00 refused — the model returned an empty document".to_owned(),
        ],
    );
    assert!(account.sections().iter().all(Section::is_closed));
}

#[test]
fn a_section_that_has_an_ending_already_is_not_asked_for_a_second_one() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // How a cancelled run is worded: the section it was stopped in is
    // closed on the spot, and the ones above it are closed with what the
    // run went on to say about them.
    account.open_section("crates/engine", base);
    account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
    account.open_section("crates/tui", at(base, 10));
    account.record(&Activity::Cost { usd: 0.03 }, at(base, 11));
    account.close_section(&Outcome::Cancelled, at(base, 20));

    let mut asked = Vec::new();
    account.close_open_sections(at(base, 20), |section| {
        asked.push(section.directory().display().to_string());
        Outcome::Wrote {
            document: "WARLOCK.md".into(),
            bytes: 12,
        }
    });

    assert_eq!(asked, vec!["crates/engine".to_owned()]);
    assert_eq!(
        said(&account, at(base, 20)),
        vec![
            "crates/engine".to_owned(),
            "0:10 wrote WARLOCK.md — 12 bytes, $0.21".to_owned(),
            "crates/tui".to_owned(),
            "0:10 cancelled — $0.03 spent".to_owned(),
        ],
    );
}

#[test]
fn a_window_is_the_slice_of_rows_the_panel_asked_for() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", base);
    account.record(&Activity::Thinking, at(base, 1));
    account.record(&tool("Read", "src/lib.rs"), at(base, 2));
    account.open_section("crates/tui", at(base, 3));
    account.record(&Activity::Thinking, at(base, 4));
    account.finish(at(base, 5));

    let now = at(base, 5);
    assert_eq!(account.line_count(), 6);
    assert_eq!(account.window(0, 6, now), account.lines(now));
    assert_eq!(
        account.window(2, 2, now),
        vec![
            Line::Clocked {
                clock: "0:03".to_owned(),
                text: "Read src/lib.rs".to_owned(),
            },
            Line::Directory {
                path: "crates/tui".into(),
            },
        ],
    );

    // Asking for more than there is, or starting past the end, is a request
    // rather than a claim about the account's length.
    assert_eq!(account.window(5, 99, now).len(), 1);
    assert!(account.window(6, 99, now).is_empty());
}

#[test]
fn the_clock_pads_seconds_and_lets_minutes_grow() {
    assert_eq!(clock(Duration::ZERO), "0:00");
    assert_eq!(clock(Duration::from_millis(1_999)), "0:01");
    assert_eq!(clock(Duration::from_secs(59)), "0:59");
    assert_eq!(clock(Duration::from_mins(1)), "1:00");
    assert_eq!(clock(Duration::from_secs(252)), "4:12");
    assert_eq!(clock(Duration::from_secs(4_384)), "73:04");
}

#[test]
fn a_now_from_before_a_section_started_reads_as_zero_rather_than_panicking() {
    let base = Instant::now();
    let mut account = Account::new(at(base, 10));

    account.open_section("crates/engine", at(base, 10));
    account.record(&Activity::Thinking, at(base, 11));

    assert_eq!(said(&account, base)[1], "0:00 thinking");

    account.finish(base);
    assert_eq!(
        said(&account, base).last().map(String::as_str),
        Some("pact finished — 1 directory, 0:00, $0.00 (incomplete: 1 pass reported no cost)"),
    );
}

#[test]
fn the_open_section_hands_back_the_instant_it_started() {
    let base = Instant::now();
    let mut account = Account::new(base);

    // Nothing has opened yet, so there is nothing being worked.
    assert_eq!(account.open_section_started(), None);

    account.open_section("crates/engine", at(base, 10));
    assert_eq!(account.open_section_started(), Some(at(base, 10)));

    // Lines under it do not move the start: the section began when it began.
    account.record(&Activity::Thinking, at(base, 12));
    assert_eq!(account.open_section_started(), Some(at(base, 10)));
}

#[test]
fn the_start_moves_to_the_second_section_the_moment_it_opens() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", at(base, 10));
    account.open_section("crates/tui", at(base, 70));

    // The first section froze when the second opened, so the answer is the
    // second's start and the handover is one step rather than a fade.
    assert_eq!(account.open_section_started(), Some(at(base, 70)));
    assert!(account.sections()[0].is_closed());
    assert!(!account.sections()[1].is_closed());
}

#[test]
fn a_closed_section_is_not_an_open_one() {
    let base = Instant::now();
    let mut account = Account::new(base);

    account.open_section("crates/engine", at(base, 10));
    account.close_section(
        &Outcome::Refused {
            reason: "no".into(),
        },
        at(base, 30),
    );

    // Closed with an outcome: the directory is done, so nothing is being
    // worked even though the section is still the last one there is.
    assert_eq!(account.open_section_started(), None);

    // And a run that ended freezes whatever was still live, so a finished
    // run has no open section either.
    account.open_section("crates/tui", at(base, 40));
    assert_eq!(account.open_section_started(), Some(at(base, 40)));
    account.finish(at(base, 90));
    assert_eq!(account.open_section_started(), None);
}
