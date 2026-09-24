use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::mcp_profiler::{McpProfileReport, McpProfiler};
use crate::core::scanner::{InstructionScanner, WorkspaceContextSummary};
use crate::core::shell_bench::{ShellBenchmarkResult, ShellBenchmarker};
use crate::core::workspace_guard::{WorkspaceAuditReport, WorkspaceGuard};

/// A single scored dimension. Each category is computed independently and the
/// total is their sum, so a category score and the headline can never disagree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub name: String,
    pub score: usize,
    pub max: usize,
    pub detail: String,
}

impl CategoryScore {
    fn new(name: &str, max: usize, deductions: usize, detail: String) -> Self {
        CategoryScore {
            name: name.to_string(),
            score: max.saturating_sub(deductions),
            max,
            detail,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHealthScore {
    pub score: usize,
    pub max_score: usize,
    pub grade: &'static str,
    pub categories: Vec<CategoryScore>,
    pub subshell_latency_score: usize,
    pub context_budget_score: usize,
    pub workspace_hygiene_score: usize,
    pub security_score: usize,
    pub mcp_schema_score: usize,
    pub summary_markdown: String,
}

pub struct ReportGenerator;

impl ReportGenerator {
    pub fn calculate_health_score(workspace_root: &Path) -> Result<WorkspaceHealthScore> {
        let context = InstructionScanner::scan_workspace(workspace_root)?;
        let bench = ShellBenchmarker::run_benchmark(3)?;
        let guard = WorkspaceGuard::audit(workspace_root)?;
        let mcp = McpProfiler::profile(workspace_root)?;
        Ok(Self::score_parts(&context, &bench, &guard, &mcp))
    }

    /// Scores already-collected reports.
    ///
    /// Callers that need the individual reports too (the TUI, `scan`) use this
    /// so the expensive collection runs once instead of twice.
    pub fn score_parts(
        context: &WorkspaceContextSummary,
        bench: &ShellBenchmarkResult,
        guard: &WorkspaceAuditReport,
        mcp: &McpProfileReport,
    ) -> WorkspaceHealthScore {
        // 1. Subshell latency (20). Scored on the avoidable tax, not on total
        //    startup time, which includes work every shell must do.
        let subshell = match bench.latency_tax_ms {
            Some(tax) => {
                let mut deduction = match tax {
                    t if t > 250.0 => 14,
                    t if t > 80.0 => 8,
                    t if t > 25.0 => 3,
                    _ => 0,
                };
                if !bench.has_agent_fast_path && tax > 80.0 {
                    deduction += 6;
                }
                CategoryScore::new(
                    "Subshell Spawn Latency",
                    20,
                    deduction,
                    format!(
                        "`{:.1}ms` avoidable tax per command ({})",
                        tax, bench.shell_name
                    ),
                )
            }
            None => CategoryScore {
                name: "Subshell Spawn Latency".to_string(),
                score: 20,
                max: 20,
                detail: bench
                    .error
                    .clone()
                    .unwrap_or_else(|| "not measured".to_string()),
            },
        };

        // 2. Context budget (20), scaled across the full range so a very large
        //    instruction payload can actually reach zero.
        let ctx_tokens = context.total_tokens_cl100k;
        let context_score = CategoryScore::new(
            "Context & Token Budget",
            20,
            match ctx_tokens {
                t if t > 20_000 => 20,
                t if t > 10_000 => 14,
                t if t > 5_000 => 9,
                t if t > 2_000 => 4,
                _ => 0,
            },
            format!(
                "`≈{}` always-loaded tokens across {} instruction file(s)",
                ctx_tokens, context.total_files
            ),
        );

        // 3. Workspace hygiene (20). Agent search tools skip gitignored paths,
        //    so a build folder only costs anything when git does not ignore it.
        let mut hygiene_deduction = (guard.total_unignored_heavy_dirs * 6).min(15);
        if !guard.has_gitignore {
            hygiene_deduction += 5;
        }
        let hygiene = CategoryScore::new(
            "Workspace Hygiene",
            20,
            hygiene_deduction,
            format!(
                "`{}` build/dependency folder(s) not in .gitignore",
                guard.total_unignored_heavy_dirs
            ),
        );

        // 4. Secrets (20). Only a Claude Code `Read` deny rule counts as
        //    protection; `.claudeignore` is not read by any agent.
        let security = CategoryScore::new(
            "Secret & Security Guard",
            20,
            (guard.total_exposed_secrets * 10).min(20),
            format!(
                "`{}` secret file(s) readable by Claude Code (no Read deny rule)",
                guard.total_exposed_secrets
            ),
        );

        // 5. MCP schema load (20). Previously displayed but never scored, so a
        //    workspace paying 30k tokens per turn in tool schemas scored the same
        //    as one with no MCP servers at all.
        let mcp_detail = if mcp.measured_server_count == 0 {
            format!(
                "{} server(s) configured, none measured — run `agentprof mcp --probe`",
                mcp.total_servers
            )
        } else {
            format!(
                "`{}` measured tokens across {} of {} server(s)",
                mcp.measured_schema_tokens, mcp.measured_server_count, mcp.total_servers
            )
        };
        let mcp_score = CategoryScore::new(
            "MCP Tool Schema Load",
            20,
            if mcp.measured_server_count == 0 {
                0
            } else {
                match mcp.measured_schema_tokens {
                    t if t > 30_000 => 20,
                    t if t > 20_000 => 14,
                    t if t > 10_000 => 9,
                    t if t > 5_000 => 4,
                    _ => 0,
                }
            },
            mcp_detail,
        );

        let categories = vec![subshell, context_score, hygiene, security, mcp_score];
        let max_score: usize = categories.iter().map(|c| c.max).sum();
        let final_score: usize = categories.iter().map(|c| c.score).sum();

        let pct = (final_score as f64 / max_score as f64) * 100.0;
        let grade = if pct >= 90.0 {
            "A (Optimal)"
        } else if pct >= 80.0 {
            "B (Good)"
        } else if pct >= 70.0 {
            "C (Needs Optimization)"
        } else if pct >= 60.0 {
            "D (Significant Overhead)"
        } else {
            "F (Critical Bottlenecks)"
        };

        let mut rows = String::new();
        for c in &categories {
            rows.push_str(&format!(
                "| **{}** | {}/{} | {} |\n",
                c.name, c.score, c.max, c.detail
            ));
        }

        let summary_markdown = format!(
            "### 🤖 AI Agent Workspace Health: **{}/{}** ({})\n\n\
             | Category | Score | Status |\n\
             | :--- | :---: | :--- |\n\
             {}\n\
             *Generated by [agentprof](https://github.com/dautovri/agentprof)*\n",
            final_score, max_score, grade, rows
        );

        WorkspaceHealthScore {
            score: final_score,
            max_score,
            grade,
            subshell_latency_score: categories[0].score,
            context_budget_score: categories[1].score,
            workspace_hygiene_score: categories[2].score,
            security_score: categories[3].score,
            mcp_schema_score: categories[4].score,
            categories,
            summary_markdown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_score_saturates_at_zero() {
        let c = CategoryScore::new("x", 20, 99, "detail".into());
        assert_eq!(c.score, 0);
    }

    #[test]
    fn test_category_score_subtracts_deductions() {
        let c = CategoryScore::new("x", 20, 6, "detail".into());
        assert_eq!(c.score, 14);
    }

    #[test]
    fn test_total_equals_sum_of_categories() {
        let health = ReportGenerator::calculate_health_score(Path::new(".")).unwrap();
        let sum: usize = health.categories.iter().map(|c| c.score).sum();
        assert_eq!(health.score, sum, "headline score must equal category sum");
        assert_eq!(health.max_score, 100);
    }
}
