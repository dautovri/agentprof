use std::path::Path;
use anyhow::Result;

use crate::core::report_generator::ReportGenerator;

pub struct ReportCommand;

impl ReportCommand {
    pub fn execute(workspace_root: &Path, markdown: bool, json: bool) -> Result<()> {
        let health = ReportGenerator::calculate_health_score(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&health)?);
            return Ok(());
        }

        if markdown {
            println!("{}", health.summary_markdown);
            return Ok(());
        }

        println!("{}", health.summary_markdown);
        Ok(())
    }
}
