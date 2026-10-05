# Contributing to warlock

## Run the checks

CI runs these three commands on Linux and macOS for every push and pull request.
A pull request merges only when all of them pass on both. Run them before you
push:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

To fix formatting, run `cargo fmt`.

Read each command's exit status. Don't pipe them into `tail`, `head`, or `grep`:
the shell reports the status of the last command in a pipe, so a failing check
looks like a passing one.

## Keep it portable

Warlock supports macOS and Linux. Windows isn't supported.

- Don't hard-code absolute paths. Find the home directory from `HOME`, and
  resolve repository paths from the repository root.
- Don't write anything that depends on your workstation: your distribution,
  package manager, toolchain installer, shell setup, or local directories. This
  applies to code, comments, docs, and tests.
- Put platform-specific code behind `#[cfg(unix)]` or `#[cfg(target_os = "…")]`.
  Code gated to one platform must still build cleanly on the other. For example,
  an import used only by a Linux-only test fails clippy on macOS as unused.

## Leave out personal details

Don't commit names, email addresses, usernames, machine names, or paths under
your home directory. In tests and examples, use placeholders such as `Ada`,
`/home/ada`, or `/repo`.

## Own the whole comment

If you change a comment, read the whole block against the code as it is now,
and then rewrite it whole or leave it alone. Don't append a sentence to an
existing block: the part you didn't read goes stale and still looks
authoritative. Deleting stale text and writing nothing back is a good outcome.

The same applies to Markdown. If you change a section, check the whole section
against the current code.

`CLAUDE.md` has the full policy on when a comment is worth writing.

## Release

To release, bump the version in `Cargo.toml` and push a tag that matches it,
with a leading `v`:

```sh
git tag v<version>
git push origin v<version>
```

The tag runs `.github/workflows/release.yml`, which builds the binaries,
publishes a GitHub Release, and updates the Homebrew formula. That workflow is
generated: to change it, edit `dist-workspace.toml` and run `dist generate`.

Don't add a pre-release suffix such as `-alpha.1`. GitHub never marks a
pre-release as latest, so the README's `curl` installer, which downloads from
`releases/latest`, would keep installing the previous release.
