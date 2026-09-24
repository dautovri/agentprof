# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version
is 0.x, minor releases may change command output and JSON fields.

## [Unreleased]

## [0.3.0] - Unreleased

The first release since v0.2.0. Every release asset and the Homebrew formula
of v0.2.0 were built from the initial commit, so they include the
destructive `.claudeignore` overwrite and invented numbers fixed in
[#1](https://github.com/dautovri/agentprof/pull/1).

### Security

- `fix` no longer writes `.claudeignore`, which Claude Code never reads.
  Secrets are protected with `Read(...)` deny rules merged into
  `.claude/settings.json`, and the audit checks secrets against the deny
  rules of every settings file in play.
- `mcp --probe` no longer starts servers defined by files inside the scanned
  repository (`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`,
  `opencode.json`) unless `--trust-workspace` is given. Probing an untrusted
  clone used to run its commands.

### Added

- GitHub Action (`uses: dautovri/agentprof@v0.3.0`) that installs a
  checksum-verified binary, writes the report to the job summary, can post a
  PR comment and SARIF, and gates on score, readable secrets and conflicts.
- `report --repo-only` for machine-independent CI scores, `--sarif FILE`
  for GitHub code scanning, and `--fail-on-secrets`.
- `compile` writes native path-scoped rules: `.claude/rules` (`paths`),
  `.cursor/rules` (`.mdc` globs) and `.github/instructions` (`applyTo`),
  with `--target`, `--apply`, `--dry-run` and `--force`.
- `lint --fail-on <error|warning|never>`; lint exits 1 on conflicts.
- MCP: Codex `config.toml`, VS Code `servers`, OpenCode argv arrays, global
  Cursor, Gemini CLI, Windsurf and Linux Claude Desktop configs; per-agent
  loads; Claude Code tool-search deferral; tools/list pagination.
- Shell benchmark measures login shells and Claude Code's per-command
  snapshot replay.
- Linux arm64 and static (musl) Linux release binaries, SHA256SUMS, and a
  generated Homebrew formula for the prebuilt binaries.
- CI on every push and pull request (fmt, clippy, tests on Linux and macOS,
  MSRV 1.88).

### Fixed

- `history` counted every API request once per content block; requests are
  now de-duplicated by message and request id and priced per model.
- `lint` reported identical files as conflicting (`pnpm install` contains
  `npm install`); matching now uses word boundaries and ignores rejections.
- The `.cursorignore` block hid files like `src/core.ts` (`core.*`), and
  rewriting the block dropped previously written patterns.
- Skills are costed by what agents keep loaded (name and description), and
  project skills in `.claude/skills` are found.
- Context totals count only always-loaded files; path-scoped rules and
  nested memory files are reported as on-demand.
- `agent` no longer counts `settings.json` and other config as prompt tokens.
- Categories that were not measured no longer earn full marks; the score is
  normalised over measured categories.
- Fixed-context cost estimates account for prompt caching.
- `wrap` prints to stderr so the wrapped program's stdout stays clean.

### Changed

- `install.sh` downloads prebuilt binaries and verifies their checksum; it
  no longer installs a Rust toolchain unasked.
- JSON: `WorkspaceAuditReport` replaces `is_ignored_by_claude` with
  `blocked_for_claude` (secrets) and `is_ignored_by_git` (directories);
  scores gain `points`, `max_points`, `scope` and per-category `measured`.

## [0.2.0] - 2026-08-16

- Initial public release.

[Unreleased]: https://github.com/dautovri/agentprof/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/dautovri/agentprof/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/dautovri/agentprof/releases/tag/v0.2.0
