use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::core::report_generator::{ReportGenerator, ScoreScope};
use crate::core::rule_linter::RuleLinter;
use crate::core::sarif;
use crate::core::scanner::InstructionScanner;
use crate::core::workspace_guard::WorkspaceGuard;

pub struct ReportOptions {
    pub markdown: bool,
    pub fail_under: Option<usize>,
    pub repo_only: bool,
    pub fail_on_secrets: bool,
    pub sarif: Option<PathBuf>,
    pub json: bool,
}

pub struct ReportCommand;

impl ReportCommand {
    /// Prints the report and returns the process exit code.
    pub fn execute(workspace_root: &Path, options: &ReportOptions) -> Result<i32> {
        let scope = if options.repo_only {
            ScoreScope::RepoOnly
        } else {
            ScoreScope::Full
        };
        let health = ReportGenerator::calculate_health_score(workspace_root, scope)?;

        if options.json {
            println!("{}", serde_json::to_string_pretty(&health)?);
        } else if options.markdown {
            println!("{}", health.summary_markdown);
        } else {
            crate::ui::tables::TableRenderer::render_health_report(&health);
        }

        let guard = WorkspaceGuard::audit(workspace_root)?;
        if let Some(path) = &options.sarif {
            let context = InstructionScanner::scan_workspace(workspace_root)?;
            let lint = RuleLinter::lint_workspace(workspace_root)?;
            let log = sarif::build(&context, &guard, &lint);
            fs::write(path, serde_json::to_string_pretty(&log)?)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }

        // Exit non-zero so CI can gate on the score. Without this the generated
        // workflow could never fail, which made the advertised "context budget
        // gate" purely decorative.
        let mut code = 0;
        if let Some(threshold) = options.fail_under
            && health.score < threshold
        {
            eprintln!(
                "agentprof: health score {}/{} is below the required minimum of {}",
                health.score, health.max_score, threshold
            );
            code = 1;
        }
        if options.fail_on_secrets && guard.total_exposed_secrets > 0 {
            eprintln!(
                "agentprof: {} secret file(s) are readable by Claude Code; add Read deny rules with `agentprof fix`",
                guard.total_exposed_secrets
            );
            code = 1;
        }
        Ok(code)
    }
}
