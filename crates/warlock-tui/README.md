# warlock-tui

The terminal front end of warlock. It is the crate that ships the `warlock`
executable: the binary target is named `warlock`, so `cargo run` from the repo
root builds and runs `warlock`, not `warlock-tui`.

Its job is presentation and input — drawing the current state of the work tree
and turning keystrokes into requests. Nothing is implemented yet: this crate
currently exists as shape only, and the rendering and event loop arrive in a
later slice together with the terminal libraries they require.

## The dependency edge runs one way

`warlock-tui` is the side of the boundary that is allowed to depend on
[`warlock-engine`](../warlock-engine/README.md). The domain logic lives in the
engine; this crate consumes it and never the other way around. The edge runs
**TUI → engine, and never back**.

That dependency is not declared yet, because there is nothing in the engine to
call. It will be added, as a `path` dependency, in the slice that gives this
crate something to render.
