use std::path::Path;
use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::ci_generator::CiGenerator;

pub struct CiCommand;

impl CiCommand {
    pub fn execute(workspace_root: &Path) -> Result<()> {
        println!("{}", "🤖 Generating CI/CD Context Budget GitHub Action...".bold());
        let action_path = CiGenerator::generate_github_action(workspace_root)?;
        println!("  ✅ Created: {}", action_path.display().green().bold());
        println!("     (Runs context & token size checks on pull requests automatically)");
        println!();
        Ok(())
    }
}
