use std::time::{Duration, Instant};

use super::{Line, Log, Voice, clock, plural, size, tool_line};

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

fn said(log: &Log, now: Instant) -> Vec<String> {
    log.rows(now)
        .map(|line| match line {
            Line::Clocked { clock, text } => format!("{clock} {text}"),
            Line::Text { text }
            | Line::Said { text }
            | Line::Note { text }
            | Line::Wrapped { text, .. } => text,
        })
        .collect()
}

#[test]
fn a_log_that_has_heard_nothing_ticks_waiting_until_something_arrives() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);

    // Drawn and counted from the moment the log opens: the clock is already
    // moving with nothing recorded, which is what says the silence is work
    // and not a hang.
    assert!(log.is_empty());
    assert_eq!(log.row_count(), 1);
    assert_eq!(said(&log, at(base, 12)), ["0:12 waiting"]);

    log.push("thinking", at(base, 58));
    assert_eq!(log.row_count(), 1);
    assert_eq!(said(&log, at(base, 60)), ["1:00 thinking"]);
}

#[test]
fn a_repeated_report_is_one_line_however_often_it_arrives() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);

    for second in [2, 5, 9, 40] {
        log.extend_or_open("thinking", at(base, second));
    }
    assert_eq!(
        said(&log, at(base, 55)),
        ["0:55 thinking"],
        "one line, counting from when the stretch started",
    );

    log.extend_or_open("Read src/lib.rs", at(base, 60));
    log.extend_or_open("thinking", at(base, 61));
    log.extend_or_open("thinking", at(base, 65));

    assert_eq!(
        said(&log, at(base, 70)),
        ["1:00 thinking", "1:01 Read src/lib.rs", "1:10 thinking"],
    );
}

#[test]
fn the_newest_line_counts_up_with_the_now_it_is_asked_about() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);
    log.push("thinking", at(base, 1));

    let ticking: Vec<_> = [1, 9, 65, 3_600]
        .into_iter()
        .map(|seconds| said(&log, at(base, seconds)).join("|"))
        .collect();

    assert_eq!(
        ticking,
        [
            "0:01 thinking",
            "0:09 thinking",
            "1:05 thinking",
            "60:00 thinking"
        ],
    );
}

#[test]
fn a_line_freezes_at_the_instant_the_line_beneath_it_arrived() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);
    log.push("thinking", at(base, 2));
    log.push("Read src/lib.rs", at(base, 20));

    for now in [at(base, 20), at(base, 40), at(base, 900)] {
        assert_eq!(said(&log, now)[0], "0:20 thinking");
    }
    assert_eq!(said(&log, at(base, 40))[1], "0:40 Read src/lib.rs");
    assert_eq!(said(&log, at(base, 41))[1], "0:41 Read src/lib.rs");
}

#[test]
fn a_frozen_log_stops_ticking_at_the_first_instant_it_was_frozen() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);
    log.push("thinking", at(base, 2));

    log.freeze(at(base, 30));
    log.freeze(at(base, 600));

    assert!(log.is_closed());
    for now in [at(base, 30), at(base, 4_000)] {
        assert_eq!(said(&log, now), ["0:30 thinking"]);
    }
}

#[test]
fn a_waiting_line_freezes_with_the_log_it_waits_in() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);

    log.freeze(at(base, 30));

    for now in [at(base, 30), at(base, 900)] {
        assert_eq!(said(&log, now), ["0:30 waiting"]);
    }
}

#[test]
fn a_now_from_before_the_log_started_reads_as_zero_rather_than_panicking() {
    let base = Instant::now();
    let mut log = Log::opened_at(at(base, 10));
    log.push("thinking", at(base, 11));

    assert_eq!(log.started(), at(base, 10));
    assert_eq!(said(&log, base), ["0:00 thinking"]);
}

#[test]
fn a_tool_is_its_name_and_the_one_detail_it_carries() {
    assert_eq!(
        tool_line("Read", Some(&"src/lib.rs".to_owned())),
        "Read src/lib.rs"
    );
    assert_eq!(tool_line("WebFetch", None), "WebFetch");
}

#[test]
fn a_count_of_one_is_said_in_the_singular() {
    assert_eq!(plural(1, "file", "files"), "1 file");
    assert_eq!(plural(0, "file", "files"), "0 files");
    assert_eq!(plural(9, "file", "files"), "9 files");
}

#[test]
fn every_row_of_a_log_is_warlock_s_own_voice() {
    let base = Instant::now();
    let mut log = Log::opened_at(base);
    assert!(log.rows(base).all(|line| line.voice() == Voice::Warlock));

    log.push("thinking", at(base, 1));
    assert!(log.rows(base).all(|line| line.voice() == Voice::Warlock));
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
fn the_clock_pads_seconds_and_lets_minutes_grow() {
    assert_eq!(clock(Duration::ZERO), "0:00");
    assert_eq!(clock(Duration::from_millis(1_999)), "0:01");
    assert_eq!(clock(Duration::from_secs(59)), "0:59");
    assert_eq!(clock(Duration::from_mins(1)), "1:00");
    assert_eq!(clock(Duration::from_secs(252)), "4:12");
    assert_eq!(clock(Duration::from_secs(4_384)), "73:04");
}
