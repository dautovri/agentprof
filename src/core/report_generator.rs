use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::mcp_profiler::{McpProfileReport, McpProfiler};
use crate::core::scanner::{InstructionScanner, WorkspaceContextSummary};
use crate::core::shell_bench::{ShellBenchmarkResult, ShellBenchmarker};
use crate::core::workspace_guard::{WorkspaceAuditReport, WorkspaceGuard};

/// A single scored dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub name: String,
    pub score: usize,
    pub max: usize,
    /// False when the category could not be measured (or was left out on
    /// purpose). It then does not count toward the total, instead of silently
    /// earning full marks.
    pub measured: bool,
    pub detail: String,
}

impl CategoryScore {
    fn new(name: &str, max: usize, deductions: usize, detail: String) -> Self {
        CategoryScore {
            name: name.to_string(),
            score: max.saturating_sub(deductions),
            max,
            measured: true,
            detail,
        }
    }

    fn not_measured(name: &str, max: usize, detail: String) -> Self {
        CategoryScore {
            name: name.to_string(),
            score: 0,
            max,
            measured: false,
            detail,
        }
    }
}

/// Which categories a health score covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreScope {
    /// Workspace plus this machine (shell, MCP servers).
    Full,
    /// Only what is committed to the repository, so the result is the same
    /// on every machine. Use this in CI.
    RepoOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHealthScore {
    /// 0–100: points earned as a share of the measured categories' maximum.
    pub score: usize,
    pub max_score: usize,
    pub points: usize,
    pub max_points: usize,
    pub scope: ScoreScope,
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
    pub fn calculate_health_score(
        workspace_root: &Path,
        scope: ScoreScope,
    ) -> Result<WorkspaceHealthScore> {
        let context = InstructionScanner::scan_workspace(workspace_root)?;
        let guard = WorkspaceGuard::audit(workspace_root)?;
        if scope == ScoreScope::RepoOnly {
            return Ok(Self::score(&context, None, &guard, None));
        }
        let bench = ShellBenchmarker::run_benchmark(3)?;
        let mcp = McpProfiler::profile(workspace_root)?;
        Ok(Self::score(&context, Some(&bench), &guard, Some(&mcp)))
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
        Self::score(context, Some(bench), guard, Some(mcp))
    }

    /// `bench` and `mcp` are `None` in repo-only mode.
    fn score(
        context: &WorkspaceContextSummary,
        bench: Option<&ShellBenchmarkResult>,
        guard: &WorkspaceAuditReport,
        mcp: Option<&McpProfileReport>,
    ) -> WorkspaceHealthScore {
        const MACHINE_ONLY: &str = "not scored with --repo-only (depends on the machine)";

        // 1. Per-command shell cost (20), on what agents actually run: Claude
        //    Code's snapshot replay or a login shell, whichever costs more.
        let subshell = match bench {
            None => {
                CategoryScore::not_measured("Agent Shell Overhead", 20, MACHINE_ONLY.to_string())
            }
            Some(bench) => match bench.per_command_tax_ms {
                Some(tax) => {
                    let deduction = match tax {
                        t if t > 250.0 => 14,
                        t if t > 80.0 => 8,
                        t if t > 25.0 => 3,
                        _ => 0,
                    };
                    CategoryScore::new(
                        "Agent Shell Overhead",
                        20,
                        deduction,
                        format!(
                            "`{:.1}ms` per command ({}, {})",
                            tax,
                            bench
                                .per_command_tax_source
                                .as_deref()
                                .unwrap_or("measured"),
                            bench.shell_name
                        ),
                    )
                }
                None => CategoryScore::not_measured(
                    "Agent Shell Overhead",
                    20,
                    bench
                        .error
                        .clone()
                        .unwrap_or_else(|| "not measured".to_string()),
                ),
            },
        };

        // 2. Context budget (20), on the files loaded into every session,
        //    scaled across the full range so a very large payload reaches zero.
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

        // 5. MCP load (20), on the largest per-turn load of any one agent after
        //    deferred loading. Servers never measured are not guessed at.
        let mcp_score = match mcp {
            None => {
                CategoryScore::not_measured("MCP Tool Schema Load", 20, MACHINE_ONLY.to_string())
            }
            Some(mcp) if mcp.enabled_servers == 0 => CategoryScore::new(
                "MCP Tool Schema Load",
                20,
                0,
                "no MCP servers enabled".to_string(),
            ),
            Some(mcp) if mcp.measured_server_count == 0 => CategoryScore::not_measured(
                "MCP Tool Schema Load",
                20,
                format!(
                    "{} server(s) enabled, none measured — run `agentprof mcp --probe`",
                    mcp.enabled_servers
                ),
            ),
            Some(mcp) => CategoryScore::new(
                "MCP Tool Schema Load",
                20,
                match mcp.max_upfront_tokens {
                    t if t > 30_000 => 20,
                    t if t > 20_000 => 14,
                    t if t > 10_000 => 9,
                    t if t > 5_000 => 4,
                    _ => 0,
                },
                format!(
                    "`{}` tokens per turn for the heaviest agent ({} of {} server(s) measured)",
                    mcp.max_upfront_tokens, mcp.measured_server_count, mcp.enabled_servers
                ),
            ),
        };

        let categories = vec![subshell, context_score, hygiene, security, mcp_score];
        let points: usize = categories
            .iter()
            .filter(|c| c.measured)
            .map(|c| c.score)
            .sum();
        let max_points: usize = categories
            .iter()
            .filter(|c| c.measured)
            .map(|c| c.max)
            .sum();
        let score = if max_points == 0 {
            100
        } else {
            ((points as f64 / max_points as f64) * 100.0).round() as usize
        };

        let grade = match score {
            90.. => "A (Optimal)",
            80.. => "B (Good)",
            70.. => "C (Needs Optimization)",
            60.. => "D (Significant Overhead)",
            _ => "F (Critical Bottlenecks)",
        };

        let mut rows = String::new();
        for c in &categories {
            let value = if c.measured {
                format!("{}/{}", c.score, c.max)
            } else {
                "n/a".to_string()
            };
            rows.push_str(&format!("| **{}** | {} | {} |\n", c.name, value, c.detail));
        }
        let scope = if bench.is_none() {
            ScoreScope::RepoOnly
        } else {
            ScoreScope::Full
        };
        let scope_note = match scope {
            ScoreScope::RepoOnly => " · repository checks only",
            ScoreScope::Full => "",
        };

        let summary_markdown = format!(
            "### 🤖 AI Agent Workspace Health: **{}/100** ({})\n\n\
             | Category | Score | Status |\n\
             | :--- | :---: | :--- |\n\
             {}\n\
             *{} of {} points across measured categories{} · generated by [agentprof](https://github.com/dautovri/agentprof)*\n",
            score, grade, rows, points, max_points, scope_note
        );

        WorkspaceHealthScore {
            score,
            max_score: 100,
            points,
            max_points,
            scope,
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
    fn test_score_is_the_share_of_measured_points() {
        let health =
            ReportGenerator::calculate_health_score(Path::new("."), ScoreScope::Full).unwrap();
        let points: usize = health
            .categories
            .iter()
            .filter(|c| c.measured)
            .map(|c| c.score)
            .sum();
        let max: usize = health
            .categories
            .iter()
            .filter(|c| c.measured)
            .map(|c| c.max)
            .sum();
        assert_eq!(health.points, points);
        assert_eq!(health.max_points, max);
        assert_eq!(
            health.score,
            ((points as f64 / max as f64) * 100.0).round() as usize
        );
        assert_eq!(health.max_score, 100);
    }

    /// Regression: unmeasured categories used to earn full marks, so a CI
    /// runner with no MCP config got a free 20/20.
    #[test]
    fn test_repo_only_leaves_machine_categories_out() {
        let context = WorkspaceContextSummary::default();
        let guard = WorkspaceAuditReport {
            has_gitignore: true,
            total_exposed_secrets: 1,
            ..Default::default()
        };
        let health = ReportGenerator::score(&context, None, &guard, None);
        assert_eq!(health.scope, ScoreScope::RepoOnly);
        assert_eq!(health.max_points, 60);
        assert_eq!(health.points, 50);
        assert_eq!(health.score, 83);
        assert!(health.summary_markdown.contains("n/a"));
    }
}
