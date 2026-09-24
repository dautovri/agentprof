use std::path::Path;

use anyhow::Result;

use crate::core::report_generator::{ReportGenerator, ScoreScope};

pub struct ReportCommand;

impl ReportCommand {
    pub fn execute(
        workspace_root: &Path,
        markdown: bool,
        fail_under: Option<usize>,
        repo_only: bool,
        json: bool,
    ) -> Result<i32> {
        let scope = if repo_only {
            ScoreScope::RepoOnly
        } else {
            ScoreScope::Full
        };
        let health = ReportGenerator::calculate_health_score(workspace_root, scope)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&health)?);
        } else if markdown {
            println!("{}", health.summary_markdown);
        } else {
            crate::ui::tables::TableRenderer::render_health_report(&health);
        }

        // Exit non-zero so CI can gate on the score. Without this the generated
        // workflow could never fail, which made the advertised "context budget
        // gate" purely decorative.
        if let Some(threshold) = fail_under
            && health.score < threshold
        {
            eprintln!(
                "agentprof: health score {}/{} is below the required minimum of {}",
                health.score, health.max_score, threshold
            );
            return Ok(1);
        }
        Ok(0)
    }
}
