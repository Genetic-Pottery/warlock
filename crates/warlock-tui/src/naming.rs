//! How a line names more than one thing.
//!
//! Both joins below reach the screen from the library and from the binary —
//! `Sigils::line` and `brief` on one side, `check`, `config`, `boundary`,
//! `cut`, `error` and `writing` on the other — which is why they live here
//! rather than in whichever module first needed one. Three separate copies of
//! [`and_listed`] each carried a comment saying it was written in the shape of
//! one of the others.

/// `` `a`, `b`, `c` ``.
///
/// One spelling for the list, so the line a person reads off a cut and the line
/// they read off its failure name the issues the same way.
#[must_use]
pub fn listed<S: AsRef<str>>(items: &[S]) -> String {
    items
        .iter()
        .map(|item| format!("`{}`", item.as_ref()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `a, b and c`, with the items already decorated by the caller.
///
/// No comma before the `and`: a refusal naming more than one thing is read as a
/// sentence, and a serial comma there would be read as one more item.
#[must_use]
pub fn and_listed(items: &[String]) -> String {
    let Some((last, rest)) = items.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        return last.clone();
    }
    format!("{} and {last}", rest.join(", "))
}

#[cfg(test)]
#[path = "tests/naming.rs"]
mod tests;
