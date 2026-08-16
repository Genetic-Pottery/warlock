![Warlock](assets/warlock-logo.png)

# warlock
See your codebase the way your AI does. A TUI where documentation is the interface

## Contributing

Run these three checks before pushing. CI runs exactly the same three commands on
every push and pull request, so if they pass locally they pass there too:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
