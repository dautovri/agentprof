use anyhow::Result;
use owo_colors::OwoColorize;
use std::path::Path;

use crate::core::rule_compressor::RuleCompressor;
use crate::ui::formatters::Formatters;

pub struct CompressCommand;

impl CompressCommand {
    pub fn execute(file_path: &Path, overwrite: bool) -> Result<()> {
        println!(
            "{}",
            format!(
                "🗜️ Compressing instruction file '{}'...",
                file_path.display()
            )
            .bold()
        );

        let report = RuleCompressor::compress_file(file_path)?;
        let saved_path = RuleCompressor::save_report(&report, overwrite)?;

        println!(
            "  • Original Tokens:   {}",
            Formatters::format_tokens(report.original_tokens).yellow()
        );
        println!(
            "  • Compressed Tokens: {}",
            Formatters::format_tokens(report.compressed_tokens)
                .green()
                .bold()
        );
        println!(
            "  • Tokens Saved:      {} ({:.1}% context reduction)",
            Formatters::format_tokens(report.tokens_saved)
                .bold()
                .green(),
            report.savings_percentage
        );
        println!(
            "  • Output File:       {}",
            saved_path.display().bold().cyan()
        );

        if overwrite {
            println!("  ℹ️  Backup of original saved with .bak extension");
        }

        println!();
        Ok(())
    }
}
