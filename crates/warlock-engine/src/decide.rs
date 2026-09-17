//! The trigger half of the freshness decision: no filesystem, no clock, no I/O,
//! so the rule that combines a manifest entry with a computed hash can be read
//! and tested on its own.

use crate::{NodeState, PactEntry};

/// Total and infallible on purpose. Hashing can fail; deciding cannot. A caller
/// whose hash failed already knows the answer — the content it would compare
/// against is unknown, which is the stale side of this rule — and taking a
/// `Result` here would only let that case be forgotten somewhere else.
///
/// ```
/// use warlock_engine::{NodeState, PactEntry, decide_state};
///
/// let entry = PactEntry::new(".", "crates/engine", "crates/engine/WARLOCK.md")?;
///
/// // No entry: outside Warlock's management, whatever the content hashes to.
/// assert_eq!(decide_state(None, "abc123"), NodeState::Unpacted);
/// // Pacted but never judged, and pacted but judged against other content,
/// // are the same answer: stale.
/// assert_eq!(decide_state(Some(&entry), "abc123"), NodeState::PactedStale);
///
/// let granted = entry.with_grant("abc123", "2026-08-19T07:32:00Z");
/// assert_eq!(decide_state(Some(&granted), "def456"), NodeState::PactedStale);
/// assert_eq!(decide_state(Some(&granted), "abc123"), NodeState::PactedFresh);
/// # Ok::<(), warlock_engine::manifest::Error>(())
/// ```
#[must_use]
pub fn decide_state(entry: Option<&PactEntry>, computed_hash: &str) -> NodeState {
    let Some(entry) = entry else {
        return NodeState::Unpacted;
    };
    // `None` and `Some(other)` share an arm deliberately: never judged and
    // judged against other content are the same answer, and there is no fourth
    // colour for the first of them.
    match entry.granted_hash() {
        Some(granted) if granted == computed_hash => NodeState::PactedFresh,
        Some(_) | None => NodeState::PactedStale,
    }
}

#[cfg(test)]
#[path = "tests/decide.rs"]
mod tests;
