use std::path::PathBuf;
use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(
    name = "agentprof",
    version,
    about = "AI Agent Workspace Optimizer & Shell Latency Profiler",
    long_about = "Full-suite optimizer for AI coding agents (Claude, OpenCode, Cursor, Grok): profiles Oh My Zsh plugins, subshell latency, MCP schemas, skills collisions, and rule bloat."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Target directory (defaults to the current directory)
    // Uses a distinct argument id: a subcommand positional also named `path`
    // shadowed the global one, which made `--path` silently unusable on every
    // subcommand.
    #[arg(short, long = "path", id = "global_path", global = true, value_name = "DIR")]
    pub path: Option<PathBuf>,

    /// Output results as JSON
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Full workspace, shell, MCP, and skills audit (default)
    Scan {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Launch full-screen interactive Terminal UI dashboard
    Tui {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Generate shell auto-completions (zsh, bash, fish, powershell)
    Completions {
        /// Target shell type
        shell: Shell,
    },

    /// Compress verbose instruction files (AGENTS.md, CLAUDE.md)
    Compress {
        /// Target rule file to compress
        file: PathBuf,

        /// Overwrite the file in place (creates .bak backup)
        #[arg(short, long)]
        overwrite: bool,
    },

    /// Lint instruction files for contradictions, vague rules, and anti-patterns
    Lint {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Wrap and accelerate an agent session with subshell fast-path injection
    Wrap {
        /// Command and arguments to run (e.g. `agentprof wrap claude`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },

    /// Generate an Agent Workspace Health Score (0-100) and Markdown report
    Report {
        /// Target directory
        path: Option<PathBuf>,

        /// Output GitHub-flavored Markdown
        #[arg(short, long)]
        markdown: bool,

        /// Exit with code 1 if the health score is below this value (for CI gating)
        #[arg(long, value_name = "SCORE")]
        fail_under: Option<usize>,
    },

    /// Profile specific AI agent platforms (OpenCode, Claude Code, Grok)
    Agent {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Profile MCP (Model Context Protocol) server tool schemas & token load
    Mcp {
        /// Target directory
        path: Option<PathBuf>,

        /// Start each server and read its real tool list over the MCP protocol.
        /// This executes the commands in your MCP config.
        #[arg(long)]
        probe: bool,

        /// Per-server probe timeout in seconds
        #[arg(long, default_value_t = 10)]
        probe_timeout: u64,
    },

    /// Audit installed AI agent skills, token weights, and trigger keyword collisions
    Skills {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Inspect agent session history, lifetime costs, and loop thrashing
    History {
        /// Number of most-recent session transcripts to analyze (0 = all)
        #[arg(short, long, default_value_t = crate::core::session_history::DEFAULT_SESSION_LIMIT)]
        sessions: usize,
    },

    /// Generate a GitHub Action workflow to enforce context budgets in CI/CD
    Ci {
        /// Target directory
        path: Option<PathBuf>,

        /// Minimum health score the generated workflow will require
        #[arg(long, default_value_t = 70)]
        min_score: usize,

        /// Replace an existing workflow file (a backup is kept)
        #[arg(long)]
        force: bool,
    },

    /// Profile Oh My Zsh plugins and shell initialization hooks
    Omz,

    /// Benchmark subshell spawn latency and calculate agent tool tax
    Bench {
        /// Number of benchmark iterations
        #[arg(short, long, default_value_t = 5)]
        iterations: usize,
    },

    /// Audit workspace instruction files and token expenditure
    Context {
        /// Target directory
        path: Option<PathBuf>,
    },

    /// Apply automated optimizations (.claudeignore, fast-path bypass guard)
    Fix {
        /// Target directory
        path: Option<PathBuf>,

        /// Apply shell fast-path bypass in ~/.zshrc
        #[arg(long)]
        shell: bool,

        /// Generate safe .claudeignore and .cursorignore files
        #[arg(long)]
        ignore: bool,

        /// Compile ~/.zshrc to bytecode using zcompile
        #[arg(long)]
        zcompile: bool,

        /// Apply all optimizations, including changes to your shell rc file
        #[arg(short, long)]
        all: bool,

        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Compile monolithic instruction files into modular JIT rules
    Compile {
        /// Target directory
        path: Option<PathBuf>,
    },
}
