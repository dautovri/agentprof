use anyhow::Result;
use std::path::Path;

use crate::core::agent_platforms::AgentPlatformProfiler;
use crate::ui::tables::TableRenderer;

pub struct AgentCommand;

impl AgentCommand {
    pub fn execute(workspace_root: &Path, json: bool) -> Result<()> {
        let profiles = AgentPlatformProfiler::profile_all(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&profiles)?);
            return Ok(());
        }

        TableRenderer::render_agent_profiles(&profiles);
        println!();
        Ok(())
    }
}
