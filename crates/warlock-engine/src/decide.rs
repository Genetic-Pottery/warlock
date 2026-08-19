//! The staleness decision: manifest entry plus computed hash, in; colour, out.
//!
//! Section 6 of the design doc splits the job in two — the hash is the trigger,
//! an AI is the judge — and this module is the whole of the trigger's verdict.
//! It has no filesystem in it, no clock and no I/O: it takes what the manifest
//! said about a node ([`PactEntry`], or its absence) and what the content at
//! that node hashes to *now* ([`subtree_hash`](crate::subtree_hash)), and
//! answers with one of the three colours. That is why it is a function of two
//! values rather than a method on anything: the two inputs come from different
//! places, on different schedules, and the rule that combines them is small
//! enough to read in one sitting and to test exhaustively.
//!
//! The rule, in full:
//!
//! | manifest entry | granted hash | verdict |
//! | -------------- | ------------ | ------- |
//! | absent | — | [`Unpacted`] |
//! | present | none recorded | [`PactedStale`] |
//! | present | differs from the computed hash | [`PactedStale`] |
//! | present | equals the computed hash | [`PactedFresh`] |
//!
//! Two of those rows collapse into one idea, and it is the one worth stating
//! out loud: **never judged and judged-against-something-else are the same
//! answer.** Section 5 leaves no room for a fourth "unknown" colour, because
//! unjudged *is* stale — staleness is mechanical and needs nobody's opinion,
//! whereas freshness has to be granted. Hence the asymmetry here: every path
//! through this function returns stale except the single one where a hash
//! somebody recorded matches a hash just computed.
//!
//! And nothing in this project grants freshness today. There is no refresh
//! pass, no `claude` invocation, no code anywhere in this workspace that writes
//! a `granted_hash`. [`PactedFresh`] is reachable only when a human hand-writes
//! a hash into `.warlock/pacts.toml` — which is exactly how the test below
//! reaches it.
//!
//! [`Unpacted`]: NodeState::Unpacted
//! [`PactedStale`]: NodeState::PactedStale
//! [`PactedFresh`]: NodeState::PactedFresh

use crate::{NodeState, PactEntry};

/// The colour of a node, from its manifest entry and the hash of its content.
///
/// `entry` is the node's `[[pact]]` table, or `None` if the manifest has no
/// entry for it. `computed_hash` is what everything at and below the node
/// hashes to right now, in the same spelling
/// [`subtree_hash`](crate::subtree_hash) returns and
/// [`PactEntry::granted_hash`] stores — the comparison is a plain string
/// equality, and this function neither computes nor validates either side.
///
/// Total and infallible on purpose. Hashing can fail; deciding cannot. A caller
/// whose hash failed already knows what to do — the content it would have
/// compared against is unknown, so the node is [`PactedStale`] — and pushing
/// that case in here as a `Result` would only let it be forgotten somewhere
/// else. See the [module docs](self) for the four-case table this implements.
///
/// ```
/// use warlock_engine::{NodeState, PactEntry, decide_state};
///
/// let entry = PactEntry::new(".", "crates/engine", "crates/engine/README.md")?;
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
/// # Ok::<(), warlock_engine::ManifestError>(())
/// ```
///
/// [`PactedStale`]: NodeState::PactedStale
#[must_use]
pub fn decide_state(entry: Option<&PactEntry>, computed_hash: &str) -> NodeState {
    let Some(entry) = entry else {
        return NodeState::Unpacted;
    };
    // `None` and `Some(other)` land on the same arm deliberately: an entry with
    // no grant is not a special case to be handled elsewhere, it is just an
    // entry whose grant does not match, and there is no colour for it beyond
    // stale.
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

    /// A plausible computed hash: the shape [`subtree_hash`] returns, though
    /// nothing here parses it — the comparison is string equality and the value
    /// is opaque.
    ///
    /// [`subtree_hash`]: crate::subtree_hash
    const COMPUTED: &str = "9f2c0e1a4b6d8f0213456789abcdef0123456789abcdef0123456789abcdef01";

    /// The same length and alphabet, different content: what a node hashes to
    /// after somebody edited a file under it.
    const OTHER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// An entry for a module that has never been judged.
    fn unjudged() -> PactEntry {
        PactEntry::new(
            ".",
            "crates/warlock-engine",
            "crates/warlock-engine/README.md",
        )
        .expect("a relative path inside the root is storable")
    }

    #[test]
    fn the_four_cases() {
        let judged_matching = unjudged().with_grant(COMPUTED, "2026-08-19T07:32:00Z");
        let judged_differing = unjudged().with_grant(OTHER, "2026-08-19T07:32:00Z");
        let never_judged = unjudged();

        // Written as a table so the rule is visible as a whole: the one fresh
        // row is the only one where a recorded hash equals a computed one.
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
        // be reachable from a manifest a human wrote, because nothing in this
        // workspace writes a `granted_hash`. The text below is hand-written,
        // not produced by `Manifest::save`, and the hash in it is a literal.
        let root = tempfile::tempdir().expect("a temporary directory");
        hand_write(
            root.path(),
            &format!(
                concat!(
                    "version = 1\n\n",
                    "[[pact]]\n",
                    "module = \"crates/warlock-engine\"\n",
                    "readme = \"crates/warlock-engine/README.md\"\n",
                    "granted_hash = \"{granted}\"\n",
                    "granted_at = \"2026-08-19T07:32:00Z\"\n\n",
                    "[[pact]]\n",
                    "module = \"crates/warlock-tui\"\n",
                    "readme = \"crates/warlock-tui/README.md\"\n",
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

    /// Write `text` to `<root>/.warlock/pacts.toml` directly, the way a person
    /// with an editor would.
    fn hand_write(root: &Path, text: &str) {
        let path = manifest_path(root);
        fs::create_dir_all(path.parent().expect("the manifest has a directory"))
            .expect("creates .warlock");
        fs::write(&path, text).expect("writes the manifest");
    }
}
