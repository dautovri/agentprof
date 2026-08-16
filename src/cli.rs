use std::path::PathBuf;
use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(
    name = "agentprof",
    version = "0.2.0",
    about = "AI Agent Workspace Optimizer & Shell Latency Profiler",
    long_about = "Full-suite optimizer for AI coding agents (Claude, OpenCode, Cursor, Grok): profiles Oh My Zsh plugins, subshell latency, MCP schemas, skills collisions, and rule bloat."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Target directory to scan (defaults to current directory)
    #[arg(short, long, global = true, default_value = ".")]
    pub path: PathBuf,

    /// Output results as JSON
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Full workspace, shell, MCP, and skills audit (default)
    Scan {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Launch full-screen interactive Terminal UI dashboard
    Tui {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
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
        #[arg(default_value = ".")]
        path: PathBuf,
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
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output GitHub-flavored Markdown
        #[arg(short, long)]
        markdown: bool,
    },

    /// Profile specific AI agent platforms (OpenCode, Claude Code, Grok)
    Agent {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Profile MCP (Model Context Protocol) server tool schemas & token load
    Mcp {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Audit installed AI agent skills, token weights, and trigger keyword collisions
    Skills {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Inspect agent session history, lifetime costs, and loop thrashing
    History,

    /// Generate a GitHub Action workflow to enforce context budgets in CI/CD
    Ci {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
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
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Apply automated optimizations (.claudeignore, fast-path bypass guard)
    Fix {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Apply shell fast-path bypass in ~/.zshrc
        #[arg(long)]
        shell: bool,

        /// Generate safe .claudeignore and .cursorignore files
        #[arg(long)]
        ignore: bool,

        /// Compile ~/.zshrc to bytecode using zcompile
        #[arg(long)]
        zcompile: bool,

        /// Apply all optimizations
        #[arg(short, long)]
        all: bool,
    },

    /// Compile monolithic instruction files into modular JIT rules
    Compile {
        /// Target directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}
