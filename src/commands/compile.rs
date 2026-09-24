use anyhow::Result;
use owo_colors::OwoColorize;
use std::path::Path;

use crate::core::jit_compiler::{CompileOptions, JitRuleCompiler};
use crate::ui::formatters::Formatters;

pub struct CompileCommand;

impl CompileCommand {
    pub fn execute(workspace_root: &Path, options: &CompileOptions, json: bool) -> Result<()> {
        let result = JitRuleCompiler::compile(workspace_root, options)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        println!(
            "{}",
            "📦 Moving scoped sections into path-scoped agent rules...".bold()
        );
        println!(
            "  • Source:          {}",
            result.source_file.display().bold()
        );
        let targets: Vec<String> = result.targets.iter().map(|t| format!("{:?}", t)).collect();
        println!("  • Formats:         {}", targets.join(", ").bold());

        if result.modules.is_empty() {
            println!(
                "\n{}",
                "No section heading names a language or area with a clear file pattern; nothing to move."
                    .dimmed()
            );
            return Ok(());
        }

        println!(
            "  • Always loaded:   {} tokens → {} tokens",
            Formatters::format_tokens(result.original_tokens).yellow(),
            Formatters::format_tokens(result.always_loaded_tokens).green()
        );
        println!(
            "  • On demand:       {} tokens ({:.1}% of the file)",
            Formatters::format_tokens(result.conditional_tokens).green(),
            result.deferrable_percentage
        );

        println!("\n{}", "Scoped sections:".bold());
        for m in &result.modules {
            println!(
                "  • {:<28} {:>7} tokens  {}",
                m.heading.trim_start_matches('#').trim().bold(),
                Formatters::format_tokens(m.token_count).yellow(),
                m.globs.join(", ").cyan()
            );
        }
        for path in &result.skipped_existing {
            println!(
                "  {} {} already exists (use --force to replace it)",
                "⏭".yellow(),
                path.display()
            );
        }

        if result.dry_run {
            println!(
                "\n{}",
                "Preview only. Re-run without --dry-run to write the rule files.".bold()
            );
        } else if result.source_rewritten {
            println!(
                "\n{} Moved sections removed from {} (backup: {}).",
                "✅".green(),
                result.source_file.display(),
                result
                    .backup
                    .as_ref()
                    .map(|b| b.display().to_string())
                    .unwrap_or_default()
            );
        } else {
            println!(
                "\n{} Wrote {} rule file(s). The sections are still in {}; remove them there \
                 (or re-run with --apply) so they stop loading in every session.",
                "✅".green(),
                result.written.len(),
                result.source_file.display()
            );
        }
        Ok(())
    }
}
