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
mod tests {
    use std::fs;
    use std::path::Path;

    use super::decide_state;
    use crate::{Manifest, NodeState, PactEntry, manifest_path};

    /// The shape [`subtree_hash`](crate::subtree_hash) returns, though nothing
    /// here parses it: the comparison is string equality and the value is
    /// opaque.
    const COMPUTED: &str = "9f2c0e1a4b6d8f0213456789abcdef0123456789abcdef0123456789abcdef01";

    /// The same length and alphabet, different content.
    const OTHER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn unjudged() -> PactEntry {
        PactEntry::new(
            ".",
            "crates/warlock-engine",
            "crates/warlock-engine/WARLOCK.md",
        )
        .expect("a relative path inside the root is storable")
    }

    #[test]
    fn the_four_cases() {
        let judged_matching = unjudged().with_grant(COMPUTED, "2026-08-19T07:32:00Z");
        let judged_differing = unjudged().with_grant(OTHER, "2026-08-19T07:32:00Z");
        let never_judged = unjudged();

        // A table so the rule is visible as a whole: the one fresh row is the
        // only one where a recorded hash equals a computed one.
        let cases = [
            (None, NodeState::Unpacted, "no entry at all"),
            (
                Some(&never_judged),
                NodeState::PactedStale,
                "pacted, never judged",
            ),
            (
                Some(&judged_differing),
                NodeState::PactedStale,
                "pacted, judged against other content",
            ),
            (
                Some(&judged_matching),
                NodeState::PactedFresh,
                "pacted, judged against this content",
            ),
        ];

        for (entry, expected, what) in cases {
            assert_eq!(decide_state(entry, COMPUTED), expected, "{what}");
        }
    }

    #[test]
    fn an_unpacted_node_is_unpacted_whatever_the_hash() {
        for hash in ["", COMPUTED, OTHER] {
            assert_eq!(decide_state(None, hash), NodeState::Unpacted);
        }
    }

    #[test]
    fn a_grant_is_compared_exactly() {
        // No prefix matching, no case folding, no trimming: a hash is opaque
        // and only equality means anything about it.
        let granted = unjudged().with_grant(COMPUTED, "2026-08-19T07:32:00Z");
        for near_miss in [
            COMPUTED[..COMPUTED.len() - 1].to_owned(),
            COMPUTED.to_uppercase(),
            format!(" {COMPUTED}"),
            format!("{COMPUTED}\n"),
        ] {
            assert_eq!(
                decide_state(Some(&granted), &near_miss),
                NodeState::PactedStale,
                "`{near_miss}` is not `{COMPUTED}`",
            );
        }
    }

    #[test]
    fn a_hand_written_grant_in_a_real_manifest_is_the_only_way_to_fresh() {
        // The point of going through the filesystem here: the fresh case has to
        // be reachable from a manifest a human wrote, because the only code
        // that writes a `granted_hash` is a subtree pact or refresh, and no
        // test in this module runs one. The text below is hand-written, not
        // produced by `Manifest::save`, and the hash in it is a literal.
        let root = tempfile::tempdir().expect("a temporary directory");
        hand_write(
            root.path(),
            &format!(
                concat!(
                    "version = 1\n\n",
                    "[[pact]]\n",
                    "module = \"crates/warlock-engine\"\n",
                    "document = \"crates/warlock-engine/WARLOCK.md\"\n",
                    "granted_hash = \"{granted}\"\n",
                    "granted_at = \"2026-08-19T07:32:00Z\"\n\n",
                    "[[pact]]\n",
                    "module = \"crates/warlock-tui\"\n",
                    "document = \"crates/warlock-tui/WARLOCK.md\"\n",
                ),
                granted = COMPUTED,
            ),
        );

        let manifest = Manifest::load(root.path()).expect("the fixture manifest loads");

        // The entry a human granted, against the content it was granted for.
        assert_eq!(
            decide_state(manifest.entry("crates/warlock-engine"), COMPUTED),
            NodeState::PactedFresh,
        );
        // The same entry once anything under it changes.
        assert_eq!(
            decide_state(manifest.entry("crates/warlock-engine"), OTHER),
            NodeState::PactedStale,
        );
        // Pacted in the same file, but with no grant written for it.
        assert_eq!(
            decide_state(manifest.entry("crates/warlock-tui"), COMPUTED),
            NodeState::PactedStale,
        );
        // Not in the file at all.
        assert_eq!(
            decide_state(manifest.entry("crates/warlock-cli"), COMPUTED),
            NodeState::Unpacted,
        );
    }

    /// Directly, the way a person with an editor would: not through
    /// `Manifest::save`.
    fn hand_write(root: &Path, text: &str) {
        let path = manifest_path(root);
        fs::create_dir_all(path.parent().expect("the manifest has a directory"))
            .expect("creates .warlock");
        fs::write(&path, text).expect("writes the manifest");
    }
}
