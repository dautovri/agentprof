mod cli;
mod commands;
mod core;
mod ui;

use std::path::PathBuf;

use clap::Parser;
use cli::{Cli, Commands};
use commands::agent::AgentCommand;
use commands::bench::BenchCommand;
use commands::ci::CiCommand;
use commands::compile::CompileCommand;
use commands::completions::CompletionsCommand;
use commands::compress::CompressCommand;
use commands::context::ContextCommand;
use commands::fix::FixCommand;
use commands::history::HistoryCommand;
use commands::lint::LintCommand;
use commands::mcp::McpCommand;
use commands::omz::OmzCommand;
use commands::report::ReportCommand;
use commands::scan::ScanCommand;
use commands::skills::SkillsCommand;
use commands::tui::TuiCommand;
use commands::wrap::WrapCommand;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let json = cli.json;
    let global_path = cli.path.clone();

    // A subcommand's positional wins; otherwise fall back to the global
    // `--path`, then to the current directory.
    let resolve = |sub: Option<PathBuf>| -> PathBuf {
        sub.or_else(|| global_path.clone())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    match cli.command {
        Some(Commands::Scan { path }) => {
            let path = resolve(path);
            ScanCommand::execute(&path, json)?;
        }
        Some(Commands::Tui { path }) => {
            let path = resolve(path);
            TuiCommand::execute(&path)?;
        }
        Some(Commands::Completions { shell }) => {
            CompletionsCommand::execute(shell);
        }
        Some(Commands::Compress { file, overwrite }) => {
            CompressCommand::execute(&file, overwrite)?;
        }
        Some(Commands::Lint { path, fail_on }) => {
            let path = resolve(path);
            let code = LintCommand::execute(&path, fail_on, json)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Wrap { command }) => {
            let code = WrapCommand::execute(&command)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Report {
            path,
            markdown,
            fail_under,
            repo_only,
            fail_on_secrets,
            sarif,
        }) => {
            let path = resolve(path);
            let options = commands::report::ReportOptions {
                markdown,
                fail_under,
                repo_only,
                fail_on_secrets,
                sarif,
                json,
            };
            let code = ReportCommand::execute(&path, &options)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Agent { path }) => {
            let path = resolve(path);
            AgentCommand::execute(&path, json)?;
        }
        Some(Commands::Mcp {
            path,
            probe,
            trust_workspace,
            probe_timeout,
        }) => {
            let path = resolve(path);
            McpCommand::execute(&path, probe, trust_workspace, probe_timeout, json)?;
        }
        Some(Commands::Skills { path }) => {
            let path = resolve(path);
            SkillsCommand::execute(&path, json)?;
        }
        Some(Commands::History { sessions }) => {
            HistoryCommand::execute(sessions, json)?;
        }
        Some(Commands::Ci {
            path,
            min_score,
            force,
        }) => {
            let path = resolve(path);
            CiCommand::execute(&path, min_score, force)?;
        }
        Some(Commands::Omz) => {
            OmzCommand::execute(json)?;
        }
        Some(Commands::Bench { iterations }) => {
            BenchCommand::execute(iterations, json)?;
        }
        Some(Commands::Context { path }) => {
            let path = resolve(path);
            ContextCommand::execute(&path, json)?;
        }
        Some(Commands::Fix {
            path,
            shell,
            ignore,
            zcompile,
            all,
            dry_run,
        }) => {
            let path = resolve(path);
            FixCommand::execute(&path, shell, ignore, zcompile, all, dry_run, json)?;
        }
        Some(Commands::Compile {
            path,
            target,
            apply,
            dry_run,
            force,
        }) => {
            let path = resolve(path);
            let options = core::jit_compiler::CompileOptions {
                targets: target,
                apply,
                dry_run,
                force,
            };
            CompileCommand::execute(&path, &options, json)?;
        }
        None => {
            // Default action: run full scan
            ScanCommand::execute(&resolve(None), json)?;
        }
    }

    Ok(())
}
