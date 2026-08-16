use anyhow::Result;

use crate::core::session_history::SessionHistoryAnalyzer;
use crate::ui::tables::TableRenderer;

pub struct HistoryCommand;

impl HistoryCommand {
    pub fn execute(json: bool) -> Result<()> {
        let report = SessionHistoryAnalyzer::analyze()?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_session_history(&report);
        println!();
        Ok(())
    }
}
