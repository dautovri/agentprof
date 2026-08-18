use std::path::Path;
use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::mcp_profiler::McpProfiler;
use crate::core::omz_profiler::OmzProfiler;
use crate::core::scanner::InstructionScanner;
use crate::core::shell_bench::ShellBenchmarker;
use crate::core::skills_auditor::SkillsAuditor;
use crate::core::workspace_guard::WorkspaceGuard;
use crate::ui::tables::TableRenderer;

pub struct ScanCommand;

impl ScanCommand {
    pub fn execute(target_path: &Path, json: bool) -> Result<()> {
        if !json {
            println!("{}", "⚡ Running agentprof full workspace & shell scan...".bold());
        }

        let context_summary = InstructionScanner::scan_workspace(target_path)?;
        let bench_result = ShellBenchmarker::run_benchmark(5)?;
        let omz_report = OmzProfiler::profile()?;
        let workspace_audit = WorkspaceGuard::audit(target_path)?;
        let mcp_report = McpProfiler::profile(target_path)?;
        let skills_report = SkillsAuditor::audit(target_path)?;

        if json {
            let output = serde_json::json!({
                "context": context_summary,
                "subshell": bench_result,
                "omz": omz_report,
                "workspace": workspace_audit,
                "mcp": mcp_report,
                "skills": skills_report,
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

        println!("\n{}", "💡 Actionable Recommendations:".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        let mut rec_count = 0;
        if !bench_result.has_agent_fast_path {
            rec_count += 1;
            println!("  [{}] Add subshell fast-path guard: `agentprof fix --shell`", rec_count);
        }
        if !workspace_audit.has_claudeignore || workspace_audit.total_exposed_secrets > 0 {
            rec_count += 1;
            println!("  [{}] Generate safe `.claudeignore`: `agentprof fix --ignore`", rec_count);
        }
        if context_summary.total_tokens_cl100k > 3000 {
            rec_count += 1;
            println!("  [{}] Compile monolithic rules into JIT modules: `agentprof compile`", rec_count);
        }
        if mcp_report.measured_schema_tokens > 5000 {
            rec_count += 1;
            println!("  [{}] Profile & prune heavy MCP tool schemas: `agentprof mcp`", rec_count);
        }
        if skills_report.bloated_skills_count > 0 {
            rec_count += 1;
            println!("  [{}] Audit bloated agent skills: `agentprof skills`", rec_count);
        }

        if rec_count == 0 {
            println!("  {}", "✅ Workspace and shell are fully optimized for AI agents!".green());
        }

        println!();
        Ok(())
    }
}
