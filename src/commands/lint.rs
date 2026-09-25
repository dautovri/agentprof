use anyhow::Result;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use owo_colors::OwoColorize;
use std::path::Path;

use crate::cli::FailOn;
use crate::core::rule_linter::{LintReport, LintSeverity, RuleLinter};

pub struct LintCommand;

impl LintCommand {
    /// Returns the process exit code: 1 when an issue reaches `fail_on`.
    pub fn execute(workspace_root: &Path, fail_on: FailOn, json: bool) -> Result<i32> {
        if !json {
            println!(
                "{}",
                "🔍 Linting workspace instruction files & checking for contradictions...".bold()
            );
        }

        let report = RuleLinter::lint_workspace(workspace_root)?;

        let code = Self::exit_code(&report, fail_on);

        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(code);
        }

        if report.issues.is_empty() {
            println!(
                "{}",
                "✅ No rule contradictions or instruction anti-patterns detected!"
                    .green()
                    .bold()
            );
            return Ok(code);
        }

        println!(
            "{}",
            "\n📋 Instruction Quality & Conflict Report".bold().cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        let mut table = Table::new();
        table
            .load_style(UTF8_FULL.with_rounded_corners())
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
                LintSeverity::Error => Cell::new("🚨 Conflict").fg(Color::Red),
                LintSeverity::Warning => Cell::new("⚠️ Warning").fg(Color::Yellow),
                LintSeverity::Info => Cell::new("ℹ️ Info").fg(Color::Blue),
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
        Ok(code)
    }

    fn exit_code(report: &LintReport, fail_on: FailOn) -> i32 {
        let threshold = match fail_on {
            FailOn::Error => LintSeverity::Error,
            FailOn::Warning => LintSeverity::Warning,
            FailOn::Never => return 0,
        };
        match report.max_severity() {
            Some(worst) if worst >= threshold => 1,
            _ => 0,
        }
    }
}
