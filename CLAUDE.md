## About the repo

Everything in docs/ is from the past even if it was written today.
This means its not a document of current state or facts about the system.
Its a general direction about decisions made in the past and iterated on up to the present.
Ideas can be referenced from here but they should not be viewed as currently corrects or as a source of truth.

## Before you report work finished

Run all three, and read each exit status:

    cargo fmt
    cargo clippy --all-targets -- -D warnings
    cargo test

Never pipe one of them into `tail`, `head` or `grep` to shorten the output. The
shell reports the last command's status, so `cargo clippy … | tail -5` passes
whether clippy passed or not, and work checked that way is reported done on a red
tree. Read the whole output, or redirect it to a file and read the file.

`cargo fmt` is the one that gets skipped, and the first thing CI fails on.

## Comments

Every line here was written by a model and will be read by one. A comment that
describes what the code does is a second copy of the code that drifts out of
date, and there is no human reader it is buying anything for. It also costs
money: `fitting.rs` sends each file's bytes into a pass, so a stale paragraph
is evidence in front of the model writing a `.warlock.md`, and the falsehood
ends up in the document.

Roughly 3,700 such lines were deleted on purpose. Do not write them back.

**Do not write:** a module header summarising the file, a restatement of what a
function does, a walk through steps the code already shows, or a paragraph on
obvious control flow. Do not add one while doing unrelated work.

**Write a comment only when one of these holds:**

- Its absence would let a plausible, correct-looking edit break something.
- It names an alternative that was considered and rejected, and why.
- It records a constraint invisible from the code — a wire format, or a version
  string whose bump restales every repository in existence.

A comment that survives this test is a guardrail, not documentation. Long is
fine when it is one of these; the length was never the problem.

**Touching a comment means owning all of it.** Appending a sentence to an
existing block leaves everything above it unread and unchecked, which is how a
block ends up half true — and the half that is wrong looks exactly as
authoritative as the half that is right. So if you have something to add to a
comment, read the whole block against the code as it is now and rewrite it
whole, or leave it alone. Never append. Deleting the stale half and writing
nothing back is a good outcome.

Doctests are code and stay, verbatim, including where the prose around them
goes. Deleting a `///` block never deletes the ` ```` ` example inside it.
