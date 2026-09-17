use serde::{Deserialize, Serialize};

/// Three states, and a fourth is unrepresentable: no catch-all variant, and
/// freshness is never an `Option` that could be null. Unjudged is not a state —
/// it is [`PactedStale`], because staleness is mechanical and needs nobody's
/// judgement, while freshness has to be granted.
///
/// [`PactedStale`]: NodeState::PactedStale
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeState {
    /// Rendered gray.
    Unpacted,
    /// Rendered yellow.
    PactedStale,
    /// Rendered green.
    PactedFresh,
}

impl NodeState {
    /// Lets counting and legends iterate exhaustively without spreading
    /// knowledge of the variant list across the crate.
    pub const ALL: [Self; 3] = [Self::Unpacted, Self::PactedStale, Self::PactedFresh];

    #[must_use]
    pub const fn is_pacted(self) -> bool {
        match self {
            Self::Unpacted => false,
            Self::PactedStale | Self::PactedFresh => true,
        }
    }
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
