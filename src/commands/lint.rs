use anyhow::Result;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use owo_colors::OwoColorize;
use std::path::Path;

use crate::core::rule_linter::RuleLinter;

pub struct LintCommand;

impl LintCommand {
    pub fn execute(workspace_root: &Path, json: bool) -> Result<()> {
        if !json {
            println!(
                "{}",
                "🔍 Linting workspace instruction files & checking for contradictions...".bold()
            );
        }

        let report = RuleLinter::lint_workspace(workspace_root)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        if report.issues.is_empty() {
            println!(
                "{}",
                "✅ No rule contradictions or instruction anti-patterns detected!"
                    .green()
                    .bold()
            );
            return Ok(());
        }

        println!(
            "{}",
            "\n📋 Instruction Quality & Conflict Report".bold().cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("File").fg(Color::Cyan),
                Cell::new("Line").fg(Color::Cyan),
                Cell::new("Severity").fg(Color::Cyan),
                Cell::new("Issue").fg(Color::Cyan),
                Cell::new("Suggested Fix").fg(Color::Cyan),
            ]);

        for issue in &report.issues {
            let sev_cell = match issue.severity {
                crate::core::rule_linter::LintSeverity::Error => {
                    Cell::new("🚨 Conflict").fg(Color::Red)
                }
                crate::core::rule_linter::LintSeverity::Warning => {
                    Cell::new("⚠️ Warning").fg(Color::Yellow)
                }
                crate::core::rule_linter::LintSeverity::Info => {
                    Cell::new("ℹ️ Info").fg(Color::Blue)
                }
            };

            table.add_row(vec![
                Cell::new(&issue.file),
                Cell::new(issue.line_number.to_string()),
                sev_cell,
                Cell::new(&issue.message),
                Cell::new(&issue.suggested_fix),
            ]);
        }

        println!("{table}");
        println!(
            "Total Issues: {} ({} contradictions) across {} files",
            report.total_issues.bold().yellow(),
            report.contradictions_found.bold().red(),
            report.total_files_linted.bold()
        );
        println!();
        Ok(())
    }
}
