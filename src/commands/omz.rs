use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::omz_profiler::OmzProfiler;
use crate::ui::tables::TableRenderer;

pub struct OmzCommand;

impl OmzCommand {
    pub fn execute(json: bool) -> Result<()> {
        if !json {
            println!("{}", "🐚 Profiling Oh My Zsh and Shell plugins...".bold());
        }
        let report = OmzProfiler::profile()?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        TableRenderer::render_omz_report(&report);

        if !report.recommendations.is_empty() {
            println!("\n{}", "💡 Recommendations:".bold().cyan());
            for (i, rec) in report.recommendations.iter().enumerate() {
                println!("  [{}] {}", i + 1, rec);
            }
        }
        println!();
        Ok(())
    }
}
