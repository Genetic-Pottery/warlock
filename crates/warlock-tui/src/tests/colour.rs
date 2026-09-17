use ratatui::style::Color;
use warlock_engine::NodeState;

use super::{CONVERSATION_COLOUR, FOCUS_COLOUR, GUIDE_COLOUR, SYSTEM_COLOUR, colour_for};

#[test]
fn unpacted_is_gray() {
    assert_eq!(colour_for(NodeState::Unpacted), Color::Indexed(245));
}

#[test]
fn pacted_stale_is_yellow() {
    assert_eq!(colour_for(NodeState::PactedStale), Color::Indexed(214));
}

#[test]
fn pacted_fresh_is_green() {
    assert_eq!(colour_for(NodeState::PactedFresh), Color::Indexed(71));
}

#[test]
fn the_focus_colour_is_no_states_colour() {
    for state in NodeState::ALL {
        assert_ne!(
            FOCUS_COLOUR,
            colour_for(state),
            "the focused border shares {state:?}'s colour"
        );
    }
}

#[test]
fn the_guide_colour_is_a_flat_indexed_gray() {
    assert_eq!(GUIDE_COLOUR, Color::Indexed(240));
}

#[test]
fn the_guide_colour_is_no_states_colour() {
    for state in NodeState::ALL {
        assert_ne!(
            GUIDE_COLOUR,
            colour_for(state),
            "the indent guides share {state:?}'s colour"
        );
    }
}

#[test]
fn the_guide_colour_is_not_the_focus_colour() {
    assert_ne!(
        GUIDE_COLOUR, FOCUS_COLOUR,
        "the indent guides share the focused border's colour"
    );
}

#[test]
fn the_conversation_colour_is_a_flat_indexed_mauve() {
    assert_eq!(CONVERSATION_COLOUR, Color::Indexed(139));
}

#[test]
fn the_conversation_colour_is_no_states_colour() {
    for state in NodeState::ALL {
        assert_ne!(
            CONVERSATION_COLOUR,
            colour_for(state),
            "the conversation card shares {state:?}'s colour"
        );
    }
}

#[test]
fn the_conversation_colour_is_not_the_focus_colour() {
    assert_ne!(
        CONVERSATION_COLOUR, FOCUS_COLOUR,
        "the conversation card shares the focused border's colour"
    );
}

#[test]
fn the_conversation_colour_is_not_the_guide_colour() {
    assert_ne!(
        CONVERSATION_COLOUR, GUIDE_COLOUR,
        "the conversation card shares the indent guides' colour"
    );
}

#[test]
fn the_system_colour_is_the_focus_colour() {
    assert_eq!(
        SYSTEM_COLOUR, FOCUS_COLOUR,
        "warlock's own voice and the focused border are one colour on purpose",
    );
}

#[test]
fn the_system_colour_is_no_states_colour() {
    for state in NodeState::ALL {
        assert_ne!(
            SYSTEM_COLOUR,
            colour_for(state),
            "warlock's own voice shares {state:?}'s colour"
        );
    }
}

#[test]
fn the_system_colour_is_not_the_model_s_own_words() {
    assert_ne!(
        SYSTEM_COLOUR, CONVERSATION_COLOUR,
        "warlock talking about its work is drawn as the model talking",
    );
}

#[test]
fn no_two_states_share_a_colour() {
    for state in NodeState::ALL {
        for other in NodeState::ALL {
            assert_eq!(
                state == other,
                colour_for(state) == colour_for(other),
                "{state:?} and {other:?} are drawn the same"
            );
        }
    }
}
