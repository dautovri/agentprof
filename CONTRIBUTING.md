# Contributing to agentprof

Thanks for helping. Bug reports, false-positive reports and pull requests are
all welcome.

## Ground rule: measured, never guessed

agentprof exists to report what agents actually load and run. Every number it
prints must come from a measurement (a tokenizer run, a timed spawn, a real
MCP handshake, a transcript) or be clearly labelled as an estimate. If a value
cannot be measured, report it as absent — never substitute a constant. Pull
requests that add placeholder numbers will be asked to change.

When agent behaviour is involved (which files Claude Code loads, how Cursor
applies rules, how an MCP config is shaped), link the vendor documentation in
a code comment next to the logic that depends on it.

## Development

```bash
cargo build
cargo test                               # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

CI runs the same checks on Linux and macOS, plus a build on the minimum
supported Rust version (see `rust-version` in `Cargo.toml`).

Tests must not depend on your machine. Integration tests run the binary with
`HOME` pointed at an empty directory; unit tests that need a home or config
directory take it as a parameter (for example `audit_with_home`,
`profile_in`, `analyze_home`) instead of reading the real one.

Every behaviour fix should come with a regression test that fails without it.

## Pull requests

1. Open an issue first for larger changes, so the approach can be agreed.
2. Keep each pull request to one concern and describe the user-visible effect.
3. Add an entry under `[Unreleased]` in `CHANGELOG.md`.
4. Make sure `cargo fmt`, `cargo clippy` and `cargo test` pass.

## Project layout

| Path | Purpose |
| :--- | :--- |
| `src/cli.rs` | Command-line interface (clap) |
| `src/commands/` | One module per subcommand: argument handling and output |
| `src/core/` | Measurement and fix logic, independent of presentation |
| `src/ui/` | Tables and the terminal UI |
| `tests/` | Integration tests that run the compiled binary |
| `action.yml` | The GitHub Action |
| `install.sh` | The installer used by `curl \| bash` and the Action |

## Releasing

See [RELEASING.md](RELEASING.md).
