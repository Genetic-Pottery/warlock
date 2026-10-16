# warlock-tui

The `warlock` binary: the terminal interface and the headless subcommands.

- **The tree.** Every directory under the one you launched in. A directory a
  scope covers is green, and one no scope covers is grey. Press `s` to set the
  scope on the selected directory. For every key, see
  [CONTROLS.md](../../CONTROLS.md).
- **The panel.** The conversation with the model (`/brief`, `/push`, `/draft`,
  `/pull`) and a document card for reading a file.
- **The subcommands.** `check`, `scope`, `open`, `close`, `key`, `branch`, `brief`,
  `push`, `draft`, `pull` and `resume`. See [HEADLESS-CLI.md](../../HEADLESS-CLI.md).

The outside world is reached through seams with in-memory stand-ins in
`src/stubs.rs`: `claude` runs as a child process, `git` and `gh` run behind the
`Repository` and `Forge` traits, and Linear is behind `Opens`. Tests drive a
whole session with no terminal, no network and no `claude`.
