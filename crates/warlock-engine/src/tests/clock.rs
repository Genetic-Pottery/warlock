use super::{MAX_REPRESENTABLE, MIN_REPRESENTABLE, now_rfc3339, rfc3339_from_unix_seconds};

/// The table is the specification: computed independently of this code, and
/// what a rewrite of the arithmetic has to keep agreeing with.
#[test]
fn fixed_instants() {
    let cases = [
        (0, "1970-01-01T00:00:00Z", "the epoch itself"),
        (1, "1970-01-01T00:00:01Z", "one second after it"),
        (
            1_787_320_991,
            "2026-08-21T14:03:11Z",
            "an ordinary afternoon",
        ),
        (1_709_208_000, "2024-02-29T12:00:00Z", "a leap day, midday"),
        (
            1_709_251_200,
            "2024-03-01T00:00:00Z",
            "the day after a leap day",
        ),
        (
            951_782_400,
            "2000-02-29T00:00:00Z",
            "the 400-year rule: 2000 is a leap year",
        ),
        (
            -2_203_977_600,
            "1900-02-28T00:00:00Z",
            "the 100-year rule: 1900 is not",
        ),
        (
            -2_203_891_200,
            "1900-03-01T00:00:00Z",
            "and so 28 February is followed by 1 March",
        ),
        (
            4_107_542_399,
            "2100-02-28T23:59:59Z",
            "the 100-year rule again, forwards",
        ),
    ];

    for (seconds, expected, what) in cases {
        assert_eq!(rfc3339_from_unix_seconds(seconds), expected, "{what}");
    }
}

#[test]
fn a_year_rolls_over_at_the_right_second() {
    let rollovers = [
        (946_684_799, "1999-12-31T23:59:59Z", "2000-01-01T00:00:00Z"),
        (
            1_798_761_599,
            "2026-12-31T23:59:59Z",
            "2027-01-01T00:00:00Z",
        ),
    ];

    for (last, before, after) in rollovers {
        assert_eq!(rfc3339_from_unix_seconds(last), before);
        assert_eq!(rfc3339_from_unix_seconds(last + 1), after);
    }
}

#[test]
fn a_clock_set_before_the_epoch_reports_a_date_before_the_epoch() {
    // A dead battery, a virtual machine restored from a snapshot, a
    // deliberately skewed test box: none of them may panic, and none of
    // them may silently become 1970.
    let cases = [
        (-1, "1969-12-31T23:59:59Z", "one second before the epoch"),
        (-86_400, "1969-12-31T00:00:00Z", "one day before it"),
        (-14_182_940, "1969-07-20T20:17:40Z", "several months before"),
        (-62_135_596_800, "0001-01-01T00:00:00Z", "the first year"),
    ];

    for (seconds, expected, what) in cases {
        assert_eq!(rfc3339_from_unix_seconds(seconds), expected, "{what}");
    }
}

#[test]
fn absurd_clocks_are_clamped_rather_than_mis_spelled() {
    // Past the ends of a four-digit year the answer stops being a date and
    // starts being a promise about the shape of the string.
    assert_eq!(
        rfc3339_from_unix_seconds(MIN_REPRESENTABLE),
        "0000-01-01T00:00:00Z",
    );
    assert_eq!(
        rfc3339_from_unix_seconds(MAX_REPRESENTABLE),
        "9999-12-31T23:59:59Z",
    );
    for extreme in [i64::MIN, MIN_REPRESENTABLE - 1] {
        assert_eq!(rfc3339_from_unix_seconds(extreme), "0000-01-01T00:00:00Z");
    }
    for extreme in [i64::MAX, MAX_REPRESENTABLE + 1] {
        assert_eq!(rfc3339_from_unix_seconds(extreme), "9999-12-31T23:59:59Z");
    }
}

#[test]
fn every_second_of_a_day_is_spelled_in_range() {
    // Cheap, and it catches an off-by-one in the hour/minute/second split
    // that a handful of fixed instants could miss.
    let start = 1_709_164_800; // 2024-02-29T00:00:00Z
    for offset in 0..2 * 86_400 {
        let stamp = rfc3339_from_unix_seconds(start + offset);
        let day = if offset < 86_400 {
            "2024-02-29"
        } else {
            "2024-03-01"
        };
        assert!(stamp.starts_with(day), "{stamp}");
        assert_eq!(
            stamp,
            format!(
                "{day}T{:02}:{:02}:{:02}Z",
                (offset % 86_400) / 3_600,
                (offset % 3_600) / 60,
                offset % 60,
            ),
        );
    }
}

#[test]
fn the_shape_is_fixed_whatever_the_instant() {
    // What every caller actually depends on, asserted positionally rather
    // than by parsing: nothing in this workspace parses these back, so the
    // shape is the whole contract.
    let stamps = [
        now_rfc3339(),
        rfc3339_from_unix_seconds(0),
        rfc3339_from_unix_seconds(-1),
        rfc3339_from_unix_seconds(i64::MAX),
    ];

    for stamp in stamps {
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.is_ascii(), "{stamp}");
        for (index, byte) in stamp.bytes().enumerate() {
            let expected = match index {
                4 | 7 => b'-',
                10 => b'T',
                13 | 16 => b':',
                19 => b'Z',
                _ => {
                    assert!(byte.is_ascii_digit(), "{stamp} at {index}");
                    continue;
                }
            };
            assert_eq!(byte, expected, "{stamp} at {index}");
        }
    }
}

#[test]
fn now_is_somewhere_around_now() {
    // Not a clock test — it cannot be — but enough to catch a `now` wired
    // to the wrong epoch or scaled by a thousand: this build was written in
    // 2026, and the timestamp it produces should start with a plausible
    // year rather than 1970 or 55 000.
    let stamp = now_rfc3339();
    let year: i64 = stamp[..4].parse().expect("the year is four digits");
    assert!((2020..2200).contains(&year), "{stamp}");
}
