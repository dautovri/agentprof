use std::path::Path;
use anyhow::Result;

use crate::core::mcp_profiler::McpProfiler;
use crate::ui::tables::TableRenderer;

pub struct McpCommand;

impl McpCommand {
    pub fn execute(workspace_root: &Path, json: bool) -> Result<()> {
        let report = McpProfiler::profile(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_mcp_report(&report);
        println!();
        Ok(())
    }
}
