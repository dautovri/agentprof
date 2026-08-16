mod cli;
mod commands;
mod core;
mod ui;

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

    match cli.command {
        Some(Commands::Scan { path }) => {
            ScanCommand::execute(&path, json)?;
        }
        Some(Commands::Tui { path }) => {
            TuiCommand::execute(&path)?;
        }
        Some(Commands::Completions { shell }) => {
            CompletionsCommand::execute(shell);
        }
        Some(Commands::Compress { file, overwrite }) => {
            CompressCommand::execute(&file, overwrite)?;
        }
        Some(Commands::Lint { path }) => {
            LintCommand::execute(&path, json)?;
        }
        Some(Commands::Wrap { command }) => {
            let code = WrapCommand::execute(&command)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Report { path, markdown }) => {
            ReportCommand::execute(&path, markdown, json)?;
        }
        Some(Commands::Agent { path }) => {
            AgentCommand::execute(&path, json)?;
        }
        Some(Commands::Mcp { path }) => {
            McpCommand::execute(&path, json)?;
        }
        Some(Commands::Skills { path }) => {
            SkillsCommand::execute(&path, json)?;
        }
        Some(Commands::History) => {
            HistoryCommand::execute(json)?;
        }
        Some(Commands::Ci { path }) => {
            CiCommand::execute(&path)?;
        }
        Some(Commands::Omz) => {
            OmzCommand::execute(json)?;
        }
        Some(Commands::Bench { iterations }) => {
            BenchCommand::execute(iterations, json)?;
        }
        Some(Commands::Context { path }) => {
            ContextCommand::execute(&path, json)?;
        }
        Some(Commands::Fix {
            path,
            shell,
            ignore,
            zcompile,
            all,
        }) => {
            FixCommand::execute(&path, shell, ignore, zcompile, all)?;
        }
        Some(Commands::Compile { path }) => {
            CompileCommand::execute(&path, json)?;
        }
        None => {
            // Default action: run full scan
            ScanCommand::execute(&cli.path, json)?;
        }
    }

    Ok(())
}
