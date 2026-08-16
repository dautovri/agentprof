# agentprof ⚡🤖

> **The AI Agent Workspace Optimizer & Shell Latency Profiler**  
> *A high-performance Rust CLI & TUI that eliminates subshell startup latency, profiles Oh My Zsh plugins, audits AI context bloat (`AGENTS.md`, `CLAUDE.md`, `.cursorrules`), inspects MCP tool schemas, resolves skill collisions, and accelerates AI coding agents.*

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
- 🔌 **MCP Tool Schema Profiler (`agentprof mcp`)** — Measures exact token footprint of all connected Model Context Protocol tool definitions.
- 🎯 **Agent Skills Auditor (`agentprof skills`)** — Audits installed skills, calculates token weights, and detects trigger keyword collisions.
- 🔍 **Instruction Linter & Contradiction Detector (`agentprof lint`)** — Finds conflicting instructions (e.g. `ObservableObject` vs `@Observable`, OS target mismatches).
- 🗜️ **Rule Compressor (`agentprof compress <file>`)** — Strips conversational filler and compresses rule files for maximum token density.
- 🏃 **Agent Execution Wrapper & Flight Recorder (`agentprof wrap <cmd>`)** — Injects subshell fast-path into live agent sessions and prints a post-flight summary card.
- 📝 **Workspace Health Score & PR Report (`agentprof report --markdown`)** — Generates a 0–100 health audit table for PR comments and badges.
- 🐚 **Oh My Zsh & Shell Profiler (`agentprof omz`)** — Measures startup overhead of every loaded plugin and slow `eval` hook.
- ⚡ **Subshell Latency Benchmark (`agentprof bench`)** — Measures interactive vs non-interactive latency tax.
- 📁 **Workspace Ignore & Security Guard (`agentprof scan`)** — Flags unignored build caches and exposed secret files (`.env`, `.pem`, `credentials.json`).
- 🛠️ **1-Click Auto Optimizer (`agentprof fix --all`)** — Injects non-interactive fast-path bypass guards into `~/.zshrc` and generates `.claudeignore`.
- 📦 **JIT Rule Compiler (`agentprof compile`)** — Breaks monolithic rule files into modular, context-routed instructions.
- 🤖 **CI/CD Context Budget Gate (`agentprof ci`)** — Generates a GitHub Actions workflow (`.github/workflows/agentprof-audit.yml`).

---

## 📦 Installation

### Via Homebrew (Recommended)
```bash
brew tap dautovri/tap
brew install agentprof
```

### Via Cargo
```bash
cargo install agentprof
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
| `agentprof report --markdown` | Generates a 0–100 Health Score Markdown summary for PRs. |
| `agentprof agent` | Profiles **OpenCode**, **Claude Code**, and **Grok** environments. |
| `agentprof mcp` | Profiles active MCP server tool schemas and token load. |
| `agentprof skills` | Audits installed agent skills and detects trigger collisions. |
| `agentprof history` | Lifetime AI token usage, session costs, and loop thrashing diagnostics. |
| `agentprof omz` | Profiles Oh My Zsh plugins and slow shell startup hooks. |
| `agentprof bench` | Benchmarks interactive vs non-interactive subshell latency. |
| `agentprof fix --all` | Injects fast-path bypass into `~/.zshrc` and generates `.claudeignore`. |
| `agentprof compile` | Compiles monolithic rule files into modular JIT instructions. |
| `agentprof ci` | Generates a GitHub Actions workflow for PR context budget checks. |

---

## 📄 License

MIT License — see [LICENSE](LICENSE) for details.
