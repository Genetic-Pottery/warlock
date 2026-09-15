use warlock_tui::App;

use crate::error::{Error, one_line};

// A seam, like `Screen`, `Wired` and `Converses`, and for their reason: what
// the footer says about a copy is pinned by tests that run on a build machine
// with no display, no compositor and no selection owner.
pub(crate) trait Clip {
    fn put(&mut self, text: &str) -> Result<(), arboard::Error>;
}

pub(crate) struct Clipboard {
    // The handle is held for the whole session rather than opened around each
    // copy, and that is not tidiness. X11 has no clipboard of its own: the
    // process that copied owns the selection and hands the bytes over when
    // something else asks for them, so the text dies with the handle that set
    // it. Wayland's data-control is the same arrangement under other names. A
    // handle opened per copy would pass a test that reads the clipboard back in
    // the same breath and then lose every paste in real use, which is why there
    // is one of these and `run` owns it.
    //
    // The failure to open is kept rather than returned, because a session with
    // no clipboard is still a session: warlock opens on a machine with no
    // display and the copy that cannot happen says so on the footer.
    opened: Result<arboard::Clipboard, arboard::Error>,
}

impl Clipboard {
    pub(crate) fn open() -> Self {
        Self {
            opened: arboard::Clipboard::new(),
        }
    }
}

impl Clip for Clipboard {
    fn put(&mut self, text: &str) -> Result<(), arboard::Error> {
        match &mut self.opened {
            Ok(clipboard) => clipboard.set_text(text),
            // Deliberately not retried. A second `arboard::Clipboard::new()`
            // here is exactly the per-copy handle the field above exists to
            // prevent, and what stopped the first one — no display, no
            // compositor — does not change while warlock is up. The reason is
            // re-worded rather than handed over because the error carrying it
            // is owned by that field until the session ends.
            Err(why) => Err(arboard::Error::Unknown {
                description: why.to_string(),
            }),
        }
    }
}

// The count is characters and says so: the bytes differ from it on anything
// non-ASCII, and a reader who has just copied a line wants to know the line
// went rather than how UTF-8 spells it.
pub(crate) fn copy(clipboard: &mut impl Clip, app: &mut App, text: &str) {
    match clipboard.put(text) {
        Ok(()) => {
            let characters = text.chars().count();
            let unit = if characters == 1 {
                "character"
            } else {
                "characters"
            };
            app.set_message(format!("copied {characters} {unit}"));
        }
        // Flattened onto the footer's one line like every other non-fatal
        // failure in this binary, and here for a reason of its own: what
        // arboard hands over is whatever the compositor's helper said, which is
        // some other program's output and is under nobody's control.
        Err(source) => app.set_message(one_line(&Error::Clipboard { source }.to_string())),
    }
}
