//! Thirty lines of arithmetic rather than a date/time dependency. What is
//! needed is a total function from [`SystemTime`] to twenty ASCII bytes, always
//! UTC, always to the second; nothing in this workspace parses the string back,
//! so parsing, time zones, leap seconds and locales would all be bought and
//! unused. The proleptic Gregorian calendar is closed-form, extended backwards
//! past 1582 rather than modelling a switch whose date depends on the country.

use std::time::{SystemTime, UNIX_EPOCH};

/// No leap second ever makes a Unix day 86 401 seconds long, by construction of
/// Unix time, so leap seconds need no modelling here.
const SECONDS_PER_DAY: i64 = 86_400;

/// From `0000-03-01`, the shifted-year epoch [`civil_from_days`] counts from.
const DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH: i64 = 719_468;

/// Exact, and the reason the century rules need no code of their own.
const DAYS_PER_ERA: i64 = 146_097;

/// `0000-01-01T00:00:00Z`, the earliest instant a four-digit year can spell.
const MIN_REPRESENTABLE: i64 = -62_167_219_200;

/// `9999-12-31T23:59:59Z`, the latest one.
const MAX_REPRESENTABLE: i64 = 253_402_300_799;

/// Infallible by decision. The one thing that can go wrong — a system clock set
/// before 1970 — is not a failure a caller can act on: refusing to grant a pact
/// because a laptop came back from a dead battery at 1969 is a worse answer
/// than writing `1969-12-31T23:59:59Z` into a field nothing parses.
///
/// Not monotonic, and never an ordering key: two calls can go backwards across
/// an NTP step.
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
    // how far behind it is, so the error branch is a negation rather than a
    // fallback to some made-up instant.
    let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(MAX_REPRESENTABLE),
        Err(behind) => {
            i64::try_from(behind.duration().as_secs()).map_or(MIN_REPRESENTABLE, |seconds| -seconds)
        }
    };
    rfc3339_from_unix_seconds(seconds)
}

/// Split from [`now_rfc3339`] so the calendar arithmetic is testable against
/// fixed instants.
///
/// Seconds outside what a four-digit year can spell are clamped rather than
/// rendered with a fifth digit or a leading `-`: always twenty valid characters
/// is worth more than fidelity to a clock claiming to be in the year 12 000,
/// and reaching either clamp needs an error of nearly two thousand years.
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

/// Howard Hinnant's `civil_from_days`, taking a day number counted from
/// `1970-01-01` to a proleptic Gregorian year, month (1-12) and day (1-31).
/// Its trick is to start the year on 1 March, which puts the leap day at the
/// end where it perturbs nothing: month lengths become the arithmetic
/// progression `(153 * mp + 2) / 5` below, and the 400-year era absorbs the
/// century rules. Nothing here is derivable by reading the constants, so change
/// it against the reference rather than by reasoning about the lines.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + DAYS_FROM_SHIFTED_EPOCH_TO_UNIX_EPOCH;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA); // [0, 146_096]

    // Remove the era's leap days before dividing: the +1/4, -1/100 and +1/400
    // of the Gregorian rule, in day-of-era terms.
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
#[path = "tests/clock.rs"]
mod tests;
