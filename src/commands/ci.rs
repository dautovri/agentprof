use std::path::Path;

use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::ci_generator::{CiGenerator, CiWriteOutcome};

pub struct CiCommand;

impl CiCommand {
    pub fn execute(workspace_root: &Path, min_score: usize, force: bool) -> Result<()> {
        println!("{}", "🤖 Generating CI/CD context budget gate...".bold());
        let result = CiGenerator::generate_github_action(workspace_root, min_score, force)?;

        match result.outcome {
            CiWriteOutcome::Created => {
                println!("  ✅ Created: {}", result.path.display().green().bold());
            }
            CiWriteOutcome::Overwritten => {
                println!("  ✅ Replaced: {}", result.path.display().green().bold());
                if let Some(b) = &result.backup {
                    println!("     Backup: {}", b.display().to_string().dimmed());
                }
            }
            CiWriteOutcome::Preserved => {
                println!(
                    "  ℹ️  {} already exists — left untouched.",
                    result.path.display().yellow()
                );
                println!(
                    "     Re-run with {} to replace it (a backup is kept).",
                    "--force".bold()
                );
                return Ok(());
            }
        }

        println!(
            "     The workflow runs the agentprof GitHub Action and fails when the repository health score drops below {},",
            min_score.to_string().bold()
        );
        println!(
            "     a secret file is readable by Claude Code, or instruction files contradict each other."
        );
        println!();
        Ok(())
    }
}
