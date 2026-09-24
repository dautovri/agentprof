use anyhow::Result;
use owo_colors::OwoColorize;
use std::path::Path;

use crate::core::mcp_profiler::McpProfiler;
use crate::core::omz_profiler::OmzProfiler;
use crate::core::report_generator::ReportGenerator;
use crate::core::rule_linter::RuleLinter;
use crate::core::scanner::InstructionScanner;
use crate::core::shell_bench::ShellBenchmarker;
use crate::core::skills_auditor::SkillsAuditor;
use crate::core::workspace_guard::WorkspaceGuard;
use crate::ui::tables::TableRenderer;

pub struct ScanCommand;

impl ScanCommand {
    pub fn execute(target_path: &Path, json: bool) -> Result<()> {
        if !json {
            println!(
                "{}",
                "⚡ Running agentprof full workspace & shell scan...".bold()
            );
        }

        let context_summary = InstructionScanner::scan_workspace(target_path)?;
        let bench_result = ShellBenchmarker::run_benchmark(5)?;
        let omz_report = OmzProfiler::profile()?;
        let workspace_audit = WorkspaceGuard::audit(target_path)?;
        let mcp_report = McpProfiler::profile(target_path)?;
        let skills_report = SkillsAuditor::audit(target_path)?;
        let lint_report = RuleLinter::lint_workspace(target_path)?;
        let health = ReportGenerator::score_parts(
            &context_summary,
            &bench_result,
            &workspace_audit,
            &mcp_report,
        );

        if json {
            let output = serde_json::json!({
                "health": health,
                "context": context_summary,
                "subshell": bench_result,
                "omz": omz_report,
                "workspace": workspace_audit,
                "mcp": mcp_report,
                "skills": skills_report,
                "lint": lint_report,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
            return Ok(());
        }

        TableRenderer::render_context_summary(&context_summary);
        TableRenderer::render_shell_benchmark(&bench_result);
        TableRenderer::render_omz_report(&omz_report);
        TableRenderer::render_mcp_report(&mcp_report);
        TableRenderer::render_skills_report(&skills_report);
        TableRenderer::render_workspace_audit(&workspace_audit);

        println!("\n{}", "🔍 Instruction Conflicts".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());
        if lint_report.contradictions_found == 0 {
            println!(
                "  {}",
                "✅ No contradictions between instruction files".green()
            );
        }
        for issue in lint_report
            .issues
            .iter()
            .filter(|i| i.code == "CROSS_FILE_CONFLICT")
        {
            println!("  🚨 {}", issue.message);
        }

        println!("\n{}", "💡 Actionable Recommendations:".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        let mut rec_count = 0;
        if let Some(tax) = bench_result.per_command_tax_ms
            && tax > 80.0
            && !bench_result.has_agent_fast_path
        {
            rec_count += 1;
            println!(
                "  [{}] Agents pay {:.0}ms of shell setup per command ({}). Trim your rc/profile, or try the opt-in guard: `agentprof fix --shell`",
                rec_count,
                tax,
                bench_result
                    .per_command_tax_source
                    .as_deref()
                    .unwrap_or("measured")
            );
        }
        if workspace_audit.total_exposed_secrets > 0 {
            rec_count += 1;
            println!(
                "  [{}] Block agent access to {} secret file(s) with Claude Code deny rules: `agentprof fix`",
                rec_count, workspace_audit.total_exposed_secrets
            );
        }
        if workspace_audit.has_claudeignore {
            rec_count += 1;
            println!(
                "  [{}] `.claudeignore` is not read by Claude Code; rely on deny rules instead: `agentprof fix`",
                rec_count
            );
        }
        if context_summary.total_tokens_cl100k > 3000 {
            rec_count += 1;
            println!(
                "  [{}] Move language-specific sections into path-scoped rules: `agentprof compile --dry-run`",
                rec_count
            );
        }
        if mcp_report.max_upfront_tokens > 5000 {
            rec_count += 1;
            println!(
                "  [{}] Profile & prune heavy MCP tool schemas: `agentprof mcp`",
                rec_count
            );
        }
        if skills_report.bloated_skills_count > 0 {
            rec_count += 1;
            println!(
                "  [{}] Audit bloated agent skills: `agentprof skills`",
                rec_count
            );
        }
        if lint_report.contradictions_found > 0 {
            rec_count += 1;
            println!(
                "  [{}] Resolve {} contradiction(s) between instruction files: `agentprof lint`",
                rec_count, lint_report.contradictions_found
            );
        }

        if rec_count == 0 {
            println!(
                "  {}",
                "✅ Workspace and shell are fully optimized for AI agents!".green()
            );
        }

        println!(
            "\n{} {}/100 — {} {}",
            "🤖 Workspace health:".bold(),
            health.score.bold(),
            health.grade,
            "(details: `agentprof report`)".dimmed()
        );
        println!();
        Ok(())
    }
}
