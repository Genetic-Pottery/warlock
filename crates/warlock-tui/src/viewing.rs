//! The view key. [`view_press`] is the two steps between
//! [`App::view_target`], which decides what the press means over the selected
//! row and words every refusal itself, and
//! [`App::show_document`](warlock_tui::App::show_document), which takes lines
//! and never a path.
//!
//! Two things here are choices rather than consequences. The press consults no
//! run and no manifest, so `v` means the same thing during a pact as outside
//! one: `p`, `r` and `s` refuse mid-run because a second run would race the
//! first for the same documents and the same manifest, and a read races
//! nothing. And nothing is cleared on the way in, so a read that fails costs
//! one line on the footer rather than the document a reader was looking at.

use std::path::PathBuf;

use warlock_engine::{Viewed, view_file};
use warlock_tui::App;

use crate::error::one_line;

// The file that is now on the document card, and `None` for a press that put
// nothing there — a row `App::view_target` refused, or a read that failed and
// left the card holding what it held before. It is handed back rather than kept
// on `App`: a path there would be a path something later had to open, which is
// why `App::show_document` takes lines. The loop keeps it so the edit key can
// ask whether the file `$EDITOR` rewrote is the one on the card.
pub(crate) fn view_press(app: &mut App) -> Option<PathBuf> {
    // Every row-level refusal leaves through here, having already said its
    // piece: there is one place that decides what this press means over a row,
    // and it is not this file.
    let path = app.view_target()?;

    match view_file(&path) {
        Ok(Viewed { text, cut }) => {
            app.show_document(text.lines(), cut);
            Some(path)
        }
        // The engine's own wording — it names the file and says which of the two
        // ways it went — flattened onto the footer's single line the way every
        // other non-fatal failure in this binary is. The panel is not touched:
        // a reader who could not read this file is still looking at whatever
        // they were looking at before they asked.
        Err(error) => {
            app.set_message(one_line(&error.to_string()));
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/viewing.rs"]
mod tests;
