use std::path::Path;
use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::jit_compiler::JitRuleCompiler;
use crate::ui::formatters::Formatters;

pub struct CompileCommand;

impl CompileCommand {
    pub fn execute(workspace_root: &Path, json: bool) -> Result<()> {
        println!("{}", "⚡ Compiling monolithic rules into JIT modular instructions...".bold());

        let result = JitRuleCompiler::compile_monolithic_rules(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        println!("  • Source:        {}", result.source_file.display().bold());
        println!("  • Total Tokens:  {}", Formatters::format_tokens(result.original_tokens).yellow());
        println!("  • Output Dir:    {}", result.output_directory.display().bold().green());
        println!("  • Modules Built: {}", result.total_modules_created.bold());
        println!(
            "  • Token Savings: {}",
            format!("{:.1}% average context reduction per turn", result.token_savings_percentage).bold().green()
        );

        println!("\n{}", "Compiled Modules:".bold());
        for m in &result.modules {
            println!("  • {:<20} -> {} tokens (Pattern: `{}`)", m.name.bold(), Formatters::format_tokens(m.token_count).yellow(), m.target_file_pattern.cyan());
        }

        println!("\n{}", "✅ JIT instructions generated in .agentrules/".green().bold());
        Ok(())
    }
}
