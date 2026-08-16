use std::path::Path;
use anyhow::Result;

use crate::core::scanner::InstructionScanner;
use crate::ui::tables::TableRenderer;

pub struct ContextCommand;

impl ContextCommand {
    pub fn execute(target_path: &Path, json: bool) -> Result<()> {
        let summary = InstructionScanner::scan_workspace(target_path)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&summary)?);
            return Ok(());
        }

        TableRenderer::render_context_summary(&summary);
        println!();
        Ok(())
    }
}
