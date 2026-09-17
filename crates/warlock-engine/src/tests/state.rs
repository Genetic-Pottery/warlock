use serde_test::{Token, assert_tokens};

use super::NodeState;

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
    // No format is involved, so nothing here fixes an on-disk
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
