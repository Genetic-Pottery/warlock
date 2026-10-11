use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

// The scroll lives inside `Open` for `QuitConfirm`'s reason: "closed, scrolled
// ten rows down" is not a state that can be written down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum HelpSheet {
    #[default]
    Closed,
    Open(u16),
}

const PAGE: u16 = 10;

impl HelpSheet {
    #[must_use]
    pub(crate) const fn open() -> Self {
        Self::Open(0)
    }

    #[must_use]
    pub(crate) const fn scroll(self) -> Option<u16> {
        match self {
            Self::Closed => None,
            Self::Open(scroll) => Some(scroll),
        }
    }

    // The ceiling is the frame's to know, not this module's: how far the sheet
    // can scroll depends on how many of its rows the terminal shows.
    #[must_use]
    pub(crate) fn clamped(self, most: u16) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(scroll) => Self::Open(scroll.min(most)),
        }
    }
}

// `?` closes as well as Esc, so the key that opened the sheet is the one a
// hand reaches for to put it away. `q` deliberately does not: it is quit
// everywhere else, and a `q` that only sometimes quit would be two keys.
#[must_use]
pub(crate) fn help_for(key: KeyEvent, scroll: u16) -> HelpSheet {
    if key.kind != KeyEventKind::Press {
        return HelpSheet::Open(scroll);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('?') => HelpSheet::Closed,
        KeyCode::Up | KeyCode::Char('k') => HelpSheet::Open(scroll.saturating_sub(1)),
        KeyCode::Down | KeyCode::Char('j') => HelpSheet::Open(scroll.saturating_add(1)),
        KeyCode::PageUp => HelpSheet::Open(scroll.saturating_sub(PAGE)),
        KeyCode::PageDown => HelpSheet::Open(scroll.saturating_add(PAGE)),
        KeyCode::Char('g') => HelpSheet::Open(0),
        KeyCode::Char('G') => HelpSheet::Open(u16::MAX),
        _ => HelpSheet::Open(scroll),
    }
}

#[cfg(test)]
#[path = "tests/help.rs"]
mod tests;
