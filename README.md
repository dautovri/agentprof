# agentprof ⚡🤖

> **The AI Agent Workspace Profiler**  
> *A Rust CLI & TUI that measures what slows AI coding agents down: subshell startup latency, MCP tool-schema token load, instruction bloat (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`), and skill trigger collisions. Every figure it reports is measured — never guessed.*

[![Website](https://img.shields.io/badge/Website-dautovri.github.io%2Fagentprof-blue?logo=safari)](https://dautovri.github.io/agentprof/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built in Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/Version-0.2.0-green.svg)](Cargo.toml)

🌐 **Website & Interactive Demo:** [https://dautovri.github.io/agentprof/](https://dautovri.github.io/agentprof/)

---

## 🚀 Why agentprof?

When using AI coding agents (**Claude Code**, **Cursor**, **OpenCode**, **Grok**, **Codex**, **Aider**), hidden bottlenecks degrade performance and inflate costs:

1. **Subshell Latency Penalty:** Agents execute shell commands dozens or hundreds of times. If your `~/.zshrc`, Oh My Zsh, or version managers (`nvm`, `pyenv`, `conda`) take 500ms to initialize on every tool call, 50 tool executions waste **over 25–40 seconds** waiting on shell startup.
2. **Context Window & Token Bloat:** Oversized `AGENTS.md`, `CLAUDE.md`, and skill files quietly consume 10%–30% of your context window on *every single prompt turn*, degrading reasoning quality and burning through API credits.
3. **MCP Tool Schema Inflation:** Connected MCP servers inject their entire JSON tool definitions into every prompt, adding 15,000–35,000 tokens of overhead before you even type a message.
4. **Skill Keyword Collisions:** Multiple installed skills competing for the same intent (e.g. `review`, `qa`, `design`) cause agent confusion and incorrect tool invocation.
5. **Instruction Contradictions:** Conflicting rules across `.cursorrules`, `AGENTS.md`, and `CLAUDE.md` trigger hallucination loops.

`agentprof` diagnoses, lints, and fixes all of these with microsecond precision.

---

## ✨ Features

- 🖥️ **Interactive Terminal UI (`agentprof tui`)** — Full-screen `ratatui` dashboard with tabbed views for Context, MCP, Skills, Subshell, and History.
- 🔌 **MCP Tool Schema Profiler (`agentprof mcp --probe`)** — Performs a real MCP handshake (`initialize` → `tools/list`) against each configured server and counts the exact tokens its tool definitions add to every prompt. Results are cached; servers that have never been probed report `—` rather than a guess.
- 🎯 **Agent Skills Auditor (`agentprof skills`)** — Audits installed skills, calculates token weights, and detects trigger keyword collisions.
- 🔍 **Instruction Linter & Contradiction Detector (`agentprof lint`)** — Finds conflicting instructions (e.g. `ObservableObject` vs `@Observable`, OS target mismatches).
- 🗜️ **Rule Compressor (`agentprof compress <file>`)** — Strips conversational filler and compresses rule files for maximum token density.
- 🏃 **Agent Execution Wrapper (`agentprof wrap <cmd>`)** — Runs an agent with `AGENTPROF_FAST_PATH=1` exported, propagates its exit code, and prints a post-flight summary.
- 📝 **Workspace Health Score & PR Report (`agentprof report`)** — A 0–100 audit across five equally weighted categories (subshell latency, context budget, hygiene, secrets, MCP load). `--markdown` emits a PR comment; `--fail-under <score>` exits non-zero so CI can gate on it.
- 🐚 **Oh My Zsh & Shell Profiler (`agentprof omz`)** — Measures startup overhead of every loaded plugin and slow `eval` hook.
- ⚡ **Subshell Latency Benchmark (`agentprof bench`)** — Measures interactive vs non-interactive latency tax.
- 📁 **Workspace Ignore & Security Guard (`agentprof scan`)** — Flags unignored build caches and exposed secret files (`.env`, `.pem`, `credentials.json`).
- 🛠️ **Auto Optimizer (`agentprof fix`)** — Merges ignore patterns into `.claudeignore` / `.cursorignore` without discarding your existing rules (backups kept), and can install an **opt-in** shell fast-path guard. `--dry-run` previews every change.
- 📦 **JIT Rule Compiler (`agentprof compile`)** — Breaks monolithic rule files into modular, context-routed instructions.
- 🤖 **CI/CD Context Budget Gate (`agentprof ci`)** — Generates a GitHub Actions workflow that actually fails the build when the health score drops below `--min-score` or a secret becomes reachable by agent tools. An existing workflow is never overwritten without `--force`.

---

## 📦 Installation

### Via Homebrew (Recommended)
```bash
brew tap dautovri/tap
brew install agentprof
```

### Via Cargo (Direct from GitHub)
```bash
cargo install --git https://github.com/dautovri/agentprof.git
```

### Build from Source
```bash
git clone https://github.com/dautovri/agentprof.git
cd agentprof
cargo build --release
```

---

## 🛠️ Command Cheat Sheet

| Command | Description |
| :--- | :--- |
| `agentprof` / `agentprof scan` | Full unified audit across workspace, subshell, MCP, and skills. |
| `agentprof tui` | Interactive full-screen terminal dashboard. |
| `agentprof lint` | Checks workspace instruction files for contradictions & anti-patterns. |
| `agentprof compress <file>` | Compresses verbose instruction files into dense rule sheets. |
| `agentprof wrap "<cmd>"` | Wraps and accelerates an AI agent session (`agentprof wrap claude`). |
| `agentprof report` | 0–100 health score. `--markdown` for PRs, `--fail-under N` to gate CI. |
| `agentprof agent` | Profiles **OpenCode**, **Claude Code**, and **Grok** environments. |
| `agentprof mcp` | Shows configured MCP servers. Add `--probe` to measure real tool schemas. |
| `agentprof skills` | Audits installed agent skills and detects trigger collisions. |
| `agentprof history` | Real token usage and cost from Claude Code transcripts. `--sessions N` to widen. |
| `agentprof omz` | Profiles Oh My Zsh plugins and slow shell startup hooks. |
| `agentprof bench` | Benchmarks interactive vs non-interactive subshell latency. |
| `agentprof fix` | Merges agent ignore rules. `--shell` for the opt-in guard, `--dry-run` to preview. |
| `agentprof compile` | Compiles monolithic rule files into modular JIT instructions. |
| `agentprof ci` | Generates a GitHub Actions workflow for PR context budget checks. |

---

## 🔬 How numbers are produced

`agentprof` distinguishes **measured** values from **estimates**, and reports nothing it has not actually observed:

| Figure | Source |
| :--- | :--- |
| MCP schema tokens | A live `tools/list` handshake with the server (`--probe`), tokenized with `cl100k_base`. Unprobed servers show `—`. |
| Instruction/skill tokens | `cl100k_base` tokenization of the real file contents. |
| Shell latency | Median of N real shell spawns, after warm-up runs are discarded. |
| Session cost | The `usage` blocks in Claude Code transcripts, priced with cache-aware input/output rates. |
| Plugin & hook latency | Timed by sourcing the script in an isolated (`zsh -f`) shell. Untimed hooks report "not measured". |

Cost estimates use Claude Sonnet list pricing ($3/Mtok input, $15/Mtok output); cache writes bill at 1.25× and cache reads at 0.1× the input rate.

## 🧪 Development

```bash
cargo build --release
cargo test          # 90 unit + integration tests
cargo clippy --all-targets
```

## 📄 License

MIT License — see [LICENSE](LICENSE) for details.
