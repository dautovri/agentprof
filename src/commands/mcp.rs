use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::mcp_profiler::{McpProfiler, ProbeOptions};
use crate::ui::tables::TableRenderer;

pub struct McpCommand;

impl McpCommand {
    pub fn execute(
        workspace_root: &Path,
        probe: bool,
        trust_workspace: bool,
        probe_timeout: u64,
        json: bool,
    ) -> Result<()> {
        if probe && !json {
            println!(
                "{}",
                "🔌 Probing MCP servers (starting each configured server to read its tool list)..."
                    .bold()
            );
        }

        let report = McpProfiler::profile_with_options(
            workspace_root,
            ProbeOptions {
                probe,
                trust_workspace,
                timeout: Duration::from_secs(probe_timeout.max(1)),
            },
        )?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_mcp_report(&report);
        println!();
        Ok(())
    }
}
