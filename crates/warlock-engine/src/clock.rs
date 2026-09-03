//! The wall clock, in the one spelling the manifest records: RFC 3339, UTC.
//!
//! A grant carries a timestamp — [`PactEntry::with_grant`] takes a
//! `granted_at` — and that string is written into `.warlock/pacts.toml`, a file
//! that is committed to git, read by people and diffed against other clones. So
//! it has exactly two jobs: be honest about the instant, and be spelled the
//! same way every time by every build on every machine. Nothing in this
//! workspace ever parses it back; the manifest stores it verbatim and the
//! staleness decision is made from hashes alone.
//!
//! That is why this is thirty lines of arithmetic rather than a dependency.
//! A date/time crate would buy parsing, time zones, leap seconds, formatting
//! languages and a calendar back to the Julian reform — none of which anybody
//! here asks for — in exchange for a supply chain the root manifest's rule
//! ("prefer std", every dependency comments why it earns its place) would have
//! to justify. What is actually needed is a total function from
//! [`SystemTime`] to twenty ASCII bytes, and the
//! proleptic Gregorian calendar is closed-form.
//!
//! ## What is computed, and what is deliberately not
//!
//! * **UTC only.** The `Z` is a constant, not a rendered offset. A local offset
//!   would make two people's manifests disagree about the same instant and
//!   would need the tz database to produce.
//! * **Seconds, no fraction.** `2026-08-21T14:03:11Z`, always twenty
//!   characters. RFC 3339 makes the fractional part optional, and a grant is
//!   not an event log.
//! * **Unix time, as the platform reports it.** Leap seconds are not modelled,
//!   because the input does not contain them: `SystemTime`'s epoch offset is
//!   already the leap-second-free count everything else on the machine agrees
//!   on. Inventing a correction here would put this crate's idea of "now" at
//!   odds with `ls -l`.
//! * **The proleptic Gregorian calendar**, extended backwards past 1582 without
//!   apology. The alternative is a Julian/Gregorian switch whose date depends
//!   on which country you ask, for the sake of instants no clock on this
//!   machine can be set to.
//!
//! ## The days-to-date algorithm
//!
//! [`civil_from_days`] is Howard Hinnant's `civil_from_days`, whose trick is to
//! move the start of the year to 1 March. Do that and the leap day falls at the
//! *end* of the year, where it perturbs nothing: every month's length becomes a
//! plain arithmetic progression in the month index (the `(153 * mp + 2) / 5`
//! below), and the whole conversion is four divisions with no table and no
//! branch per month. The 400-year "era" then absorbs the century rules — 400
//! Gregorian years are exactly 146 097 days, always — so the only leap-year
//! reasoning left in the code is that constant.
//!
//! [`PactEntry::with_grant`]: crate::PactEntry::with_grant

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds in a day, as this calendar counts them: no leap second ever makes a
/// Unix day 86 401 seconds long, by construction of Unix time.
const SECONDS_PER_DAY: i64 = 86_400;

/// Days from `0000-03-01` (the shifted-year epoch this arithmetic counts from)
/// to `1970-01-01`. Adding it turns a Unix day number into an era-relative one.
const DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH: i64 = 719_468;

/// Days in one 400-year Gregorian era. Exact, and the reason the century rules
/// need no code of their own.
const DAYS_PER_ERA: i64 = 146_097;

/// The earliest instant RFC 3339 can spell with a four-digit year:
/// `0000-01-01T00:00:00Z`.
const MIN_REPRESENTABLE: i64 = -62_167_219_200;

/// The latest instant RFC 3339 can spell with a four-digit year:
/// `9999-12-31T23:59:59Z`.
const MAX_REPRESENTABLE: i64 = 253_402_300_799;

/// Now, as an RFC 3339 UTC timestamp to the second: `2026-08-21T14:03:11Z`.
///
/// Total and infallible, which is a decision rather than an accident. The one
/// thing that can go wrong — a system clock set before 1970, so that
/// [`SystemTime::now`] is *behind* [`UNIX_EPOCH`] — is not a failure this
/// crate can do anything about and not one a caller can either: refusing to
/// grant a pact because a laptop came back from a dead battery with its clock
/// at 1969 would be a worse answer than writing `1969-12-31T23:59:59Z` into a
/// field nothing parses. So a pre-epoch clock produces a pre-epoch timestamp,
/// and the signature stays a plain `String`.
///
/// The value is whatever the platform's wall clock says. It is not monotonic,
/// two calls can go backwards across an NTP step, and none of that matters
/// here: this is a note for a human reading a manifest, never an ordering key.
///
/// ```
/// let stamp = warlock_engine::now_rfc3339();
///
/// assert_eq!(stamp.len(), 20);
/// assert!(stamp.ends_with('Z'));
/// assert_eq!(&stamp[4..5], "-");
/// assert_eq!(&stamp[10..11], "T");
/// ```
#[must_use]
pub fn now_rfc3339() -> String {
    // `duration_since` reports a clock behind the epoch as an error carrying
    // how far behind it is, so the negative branch is a negation, not a
    // fallback to some made-up instant.
    let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(MAX_REPRESENTABLE),
        Err(behind) => {
            i64::try_from(behind.duration().as_secs()).map_or(MIN_REPRESENTABLE, |seconds| -seconds)
        }
    };
    rfc3339_from_unix_seconds(seconds)
}

/// An instant given in seconds since the Unix epoch, as an RFC 3339 UTC
/// timestamp to the second.
///
/// Pure: no clock, no filesystem, no allocation beyond the string returned.
/// [`now_rfc3339`] is this function plus a call to [`SystemTime::now`], which
/// is what makes the calendar arithmetic testable against fixed instants.
///
/// Seconds outside the range RFC 3339's four-digit year can spell are clamped
/// to its ends rather than rendered with a fifth digit or a leading `-`. The
/// output of this function is always a valid RFC 3339 timestamp of exactly
/// twenty characters — that invariant is worth more than fidelity to a clock
/// claiming to be in the year 12 000, and reaching either clamp needs an error
/// of nearly two thousand years.
fn rfc3339_from_unix_seconds(seconds: i64) -> String {
    let seconds = seconds.clamp(MIN_REPRESENTABLE, MAX_REPRESENTABLE);

    // Euclidean, not truncating: `-1` is one second into the *last* day before
    // the epoch, so it has to floor to day `-1` with a positive 86 399 seconds
    // left over, not to day `0` with a negative remainder.
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let second_of_day = seconds.rem_euclid(SECONDS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The proleptic Gregorian year, month (1-12) and day (1-31) of a day number
/// counted from `1970-01-01`, which is day 0.
///
/// Howard Hinnant's `civil_from_days`, transcribed into integer arithmetic that
/// cannot overflow for any day number a clamped [`rfc3339_from_unix_seconds`]
/// can hand it. The shape of it, in the order the lines run:
///
/// 1. Shift the epoch to `0000-03-01`, so that a year runs March to February
///    and its leap day is the last day of it.
/// 2. Split into a 400-year *era* (exactly [`DAYS_PER_ERA`] days) and a day
///    within it, flooring so that negative day numbers land in earlier eras
///    instead of at a negative offset in this one.
/// 3. Recover the year of the era by removing the leap days the four-, hundred-
///    and four-hundred-year rules insert, then divide by 365.
/// 4. Turn the day of that shifted year into a month and a day with the
///    `(153 * mp + 2) / 5` progression, which encodes the 31/30/31/30/31 …
///    pattern the March-first ordering makes regular.
/// 5. Shift back: months 0-9 are March-December, months 10-11 are the January
///    and February that belong to the *next* real year.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA); // [0, 146_096]

    // Remove the era's leap days before dividing: the +1/4 (every fourth year),
    // -1/100 (except centuries) and +1/400 (except every fourth century) of the
    // Gregorian rule, applied in day-of-era terms.
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]

    let shifted_month = (5 * day_of_year + 2) / 153; // [0, 11], 0 == March
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1; // [1, 31]
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };

    // January and February were counted as the tail of the previous shifted
    // year, so they belong to the calendar year after it.
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::{MAX_REPRESENTABLE, MIN_REPRESENTABLE, now_rfc3339, rfc3339_from_unix_seconds};

    /// Fixed instants and the string each one must produce, computed
    /// independently of this code. The table is the specification: if the
    /// arithmetic is ever rewritten, these are what it has to keep agreeing
    /// with.
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
        // The last second of a year and the first of the next differ by one, and
        // every field moves at once.
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
        // Walk a whole day a second at a time across a leap-day boundary and
        // check the fields never leave their ranges. Cheap, and it catches an
        // off-by-one in the hour/minute/second split that a handful of fixed
        // instants could miss.
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
}
