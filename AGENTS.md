# agentprof — guide for coding agents

Rust CLI that measures what AI coding agents load and run (instruction files,
MCP tool definitions, skills, readable secrets, shell overhead) and fixes what
it can.

## Commands

- Build: `cargo build`
- Test: `cargo test` (unit tests live next to the code; `tests/` runs the binary)
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --all`

Run all four before finishing a change. CI runs them on Linux and macOS.

## Layout

- `src/cli.rs`: clap definitions. `src/main.rs`: dispatch.
- `src/commands/`: one file per subcommand; argument handling and printing only.
- `src/core/`: measurement and fix logic. Keep it free of printing.
- `src/ui/`: tables (`tables.rs`) and the ratatui dashboard (`tui.rs`).

## Rules

- Never report a guessed number. Measure it, or return `None` and show "—".
- When logic depends on how an agent behaves (which files it loads, config
  shapes, permission syntax), cite the vendor documentation in a comment.
- Tests must not read the real home directory or depend on the machine:
  pass `home` explicitly (`audit_with_home`, `profile_in`, `analyze_home`)
  and use temporary directories.
- Every bug fix gets a regression test that fails without the fix.
- `--json` output must stay a single valid JSON document on stdout; human
  banners belong in the non-JSON branch.
- Never overwrite user files without a backup, and keep `--dry-run` honest.
