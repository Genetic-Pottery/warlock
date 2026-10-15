# warlock-engine

The domain library behind warlock. It reads and writes the scope manifest,
answers boundary questions, and holds the brief, draft and pull vocabulary. It
touches the filesystem, but it opens no sockets and spawns no processes, so its
tests need no `claude`, no network and no terminal.

The main pieces:

- `manifest`: `.warlock/scopes.toml`. Each `[[directory]]` row gives a
  directory a scope, and each `[[scope]]` record routes a scope to a Linear
  team, review state and label.
- `scope`: the boundary rules. `scope_covering` finds the nearest scoped
  directory at or above a path. `scope_opens_to` decides whether this machine's
  sigils open that scope. A path no scope covers is closed to everyone.
- `load` and `tree`: the directory tree a front end draws. Each node carries its
  own scope and the scope that covers it.
- `route` and `filing`: where work under a scope is filed, and which key files
  it.
- `sigils` and `keys`: what this machine holds, stored under `~/.warlock/` and
  never committed.
- `drafting`, `splitting`, `working` and `pulls`: the schemas a model fills when
  it cuts a brief into tickets, splits a ticket into sub-tasks or works one, and
  the run records a pull keeps.
