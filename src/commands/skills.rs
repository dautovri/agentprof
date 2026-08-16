use std::path::Path;
use anyhow::Result;

use crate::core::skills_auditor::SkillsAuditor;
use crate::ui::tables::TableRenderer;

pub struct SkillsCommand;

impl SkillsCommand {
    pub fn execute(workspace_root: &Path, json: bool) -> Result<()> {
        let report = SkillsAuditor::audit(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_skills_report(&report);
        println!();
        Ok(())
    }
}
