//! The state vocabulary.
//!
//! Section 5 of the design doc fixes the model: a node is either outside
//! Warlock's management, or it is pacted and stale, or it is pacted and fresh.
//! There is deliberately no "unknown" or "unjudged" state, because unjudged
//! *is* stale — staleness is mechanical and needs no judgement to assert.

use serde::{Deserialize, Serialize};

/// What Warlock knows about one node of the tree.
///
/// Exactly three states exist, and a fourth is unrepresentable: there is no
/// catch-all variant, and freshness is never modelled as an `Option` that
/// could be null. Anything not yet judged is [`PactedStale`], not unknown.
///
/// [`PactedStale`]: NodeState::PactedStale
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeState {
    /// Not pacted: outside Warlock's management. Rendered gray.
    Unpacted,
    /// Pacted, and stale: files at or below this node's document have changed
    /// since the last freshness grant, or freshness was never granted at all.
    /// Stale is mechanical and immediate. Rendered yellow.
    PactedStale,
    /// Pacted, and fresh: an AI pass read the diff and granted freshness.
    /// Fresh is earned and can only be granted, never assumed. Rendered green.
    PactedFresh,
}

impl NodeState {
    /// Every state, in the order a reader would list them: unmanaged, then
    /// managed-and-owing, then managed-and-settled.
    ///
    /// Useful for exhaustive iteration (counting, legends) without spreading
    /// knowledge of the variant list across the crate.
    pub const ALL: [Self; 3] = [Self::Unpacted, Self::PactedStale, Self::PactedFresh];

    /// Whether this node is under Warlock's management at all.
    #[must_use]
    pub const fn is_pacted(self) -> bool {
        match self {
            Self::Unpacted => false,
            Self::PactedStale | Self::PactedFresh => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_test::{Token, assert_tokens};

    use super::NodeState;

    /// The token a given variant serialises to and deserialises back from.
    /// Written out by hand rather than derived from the value, so a rename
    /// shows up here as a failing test instead of passing silently.
    fn token(state: NodeState) -> Token {
        Token::UnitVariant {
            name: "NodeState",
            variant: match state {
                NodeState::Unpacted => "Unpacted",
                NodeState::PactedStale => "PactedStale",
                NodeState::PactedFresh => "PactedFresh",
            },
        }
    }

    #[test]
    fn every_state_survives_a_serde_round_trip() {
        // `assert_tokens` goes both ways through serde's own token stream: the
        // value must serialise to exactly these tokens and deserialise back
        // from them. No format is involved, so nothing here fixes an on-disk
        // representation.
        for state in NodeState::ALL {
            assert_tokens(&state, &[token(state)]);
        }
    }

    #[test]
    fn states_do_not_round_trip_into_each_other() {
        for state in NodeState::ALL {
            for other in NodeState::ALL {
                assert_eq!(
                    state == other,
                    token(state) == token(other),
                    "{state:?} and {other:?} share a serialised form"
                );
            }
        }
    }

    #[test]
    fn all_lists_every_state_once() {
        assert_eq!(NodeState::ALL.len(), 3);
        for state in NodeState::ALL {
            assert_eq!(
                NodeState::ALL.iter().filter(|&&s| s == state).count(),
                1,
                "{state:?} should appear exactly once"
            );
        }
    }

    #[test]
    fn only_pacted_states_are_pacted() {
        assert!(!NodeState::Unpacted.is_pacted());
        assert!(NodeState::PactedStale.is_pacted());
        assert!(NodeState::PactedFresh.is_pacted());
    }
}
