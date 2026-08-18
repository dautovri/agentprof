use anyhow::Result;

use crate::core::session_history::SessionHistoryAnalyzer;
use crate::ui::tables::TableRenderer;

pub struct HistoryCommand;

impl HistoryCommand {
    pub fn execute(sessions: usize, json: bool) -> Result<()> {
        let limit = if sessions == 0 { usize::MAX } else { sessions };
        let report = SessionHistoryAnalyzer::analyze_with_limit(limit)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_session_history(&report);
        println!();
        Ok(())
    }
}
