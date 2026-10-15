use ratatui::style::Color;

use super::{CONVERSATION_COLOUR, FOCUS_COLOUR, GUIDE_COLOUR, SYSTEM_COLOUR, colour_for};

#[test]
fn unscoped_is_gray() {
    assert_eq!(colour_for(false), Color::Indexed(245));
}

#[test]
fn scoped_is_green() {
    assert_eq!(colour_for(true), Color::Indexed(71));
}

#[test]
fn the_focus_colour_is_no_tree_colour() {
    for covered in [true, false] {
        assert_ne!(
            FOCUS_COLOUR,
            colour_for(covered),
            "the focused border shares the colour of covered={covered}"
        );
    }
}

#[test]
fn the_focus_colour_is_a_flat_indexed_green() {
    assert_eq!(FOCUS_COLOUR, Color::Indexed(42));
}

#[test]
fn the_guide_colour_is_a_flat_indexed_gray() {
    assert_eq!(GUIDE_COLOUR, Color::Indexed(240));
}

#[test]
fn the_guide_colour_is_no_tree_colour() {
    for covered in [true, false] {
        assert_ne!(
            GUIDE_COLOUR,
            colour_for(covered),
            "the indent guides share the colour of covered={covered}"
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
fn the_conversation_colour_is_no_tree_colour() {
    for covered in [true, false] {
        assert_ne!(
            CONVERSATION_COLOUR,
            colour_for(covered),
            "the conversation card shares the colour of covered={covered}"
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
fn the_system_colour_is_no_tree_colour() {
    for covered in [true, false] {
        assert_ne!(
            SYSTEM_COLOUR,
            colour_for(covered),
            "warlock's own voice shares the colour of covered={covered}"
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
fn scoped_and_unscoped_are_drawn_apart() {
    assert_ne!(colour_for(true), colour_for(false));
}
