use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub struct CiGenerator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiWriteOutcome {
    Created,
    Overwritten,
    /// Left alone because it already exists and `force` was not set.
    Preserved,
}

pub struct CiResult {
    pub path: PathBuf,
    pub outcome: CiWriteOutcome,
    pub backup: Option<PathBuf>,
}

impl CiGenerator {
    /// Writes the audit workflow.
    ///
    /// An existing workflow is preserved unless `force` is set, and is backed up
    /// before being replaced. The previous version overwrote any file at that
    /// path without warning, discarding local customisations.
    pub fn generate_github_action(
        workspace_root: &Path,
        min_score: usize,
        force: bool,
    ) -> Result<CiResult> {
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let workflows_dir = canonical_root.join(".github/workflows");
        fs::create_dir_all(&workflows_dir)?;

        let workflow_file = workflows_dir.join("agentprof-audit.yml");
        let exists = workflow_file.exists();

        if exists && !force {
            return Ok(CiResult {
                path: workflow_file,
                outcome: CiWriteOutcome::Preserved,
                backup: None,
            });
        }

        let backup = if exists {
            let bak = workflow_file.with_extension("yml.agentprof.bak");
            fs::copy(&workflow_file, &bak)?;
            Some(bak)
        } else {
            None
        };

        let content = Self::workflow_yaml(min_score);
        fs::write(&workflow_file, content)
            .with_context(|| format!("Failed to write {}", workflow_file.display()))?;

        Ok(CiResult {
            path: workflow_file,
            outcome: if exists {
                CiWriteOutcome::Overwritten
            } else {
                CiWriteOutcome::Created
            },
            backup,
        })
    }

    /// The workflow runs the agentprof GitHub Action pinned to this binary's
    /// own release, which installs a checksum-verified prebuilt binary.
    ///
    /// It used to `cargo install` from git on every run (minutes per job, no
    /// pinned version), scored machine-specific categories such as the
    /// runner's shell startup, and ran with the default token permissions.
    pub(crate) fn workflow_yaml(min_score: usize) -> String {
        format!(
            r#"name: AI Agent Workspace Audit

on:
  pull_request:
  push:
    branches: [main, master]

permissions:
  contents: read

jobs:
  agentprof:
    name: Agent workspace audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      # Fails the job when the repository health score drops below
      # `fail-under`, when a secret file is readable by Claude Code, or when
      # instruction files contradict each other. The report is written to the
      # job summary. Only repository contents are scored (`--repo-only`), so
      # the result does not depend on the runner.
      - uses: dautovri/agentprof@v{version}
        with:
          fail-under: {min_score}
          # To annotate pull requests, also grant `security-events: write`,
          # set `sarif: agentprof.sarif` and add a
          # github/codeql-action/upload-sarif step. To post a sticky PR
          # comment, grant `pull-requests: write` and set `comment: true`.
"#,
            version = env!("CARGO_PKG_VERSION"),
            min_score = min_score
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_pins_the_action_and_threshold() {
        let yaml = CiGenerator::workflow_yaml(75);
        assert!(yaml.contains("fail-under: 75"));
        assert!(yaml.contains(&format!(
            "dautovri/agentprof@v{}",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(yaml.contains("permissions:\n  contents: read"));
        assert!(
            !yaml.contains("cargo install"),
            "must not compile from source in CI"
        );
    }

    #[test]
    fn test_existing_workflow_is_preserved_without_force() {
        let dir = std::env::temp_dir().join(format!("agentprof_ci_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        let wf = dir.join(".github/workflows/agentprof-audit.yml");
        fs::write(&wf, "name: my custom workflow\n").unwrap();

        let result = CiGenerator::generate_github_action(&dir, 70, false).unwrap();
        assert_eq!(result.outcome, CiWriteOutcome::Preserved);
        assert_eq!(
            fs::read_to_string(&wf).unwrap(),
            "name: my custom workflow\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_force_overwrites_but_backs_up() {
        let dir = std::env::temp_dir().join(format!("agentprof_ci_f_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        let wf = dir.join(".github/workflows/agentprof-audit.yml");
        fs::write(&wf, "name: my custom workflow\n").unwrap();

        let result = CiGenerator::generate_github_action(&dir, 70, true).unwrap();
        assert_eq!(result.outcome, CiWriteOutcome::Overwritten);
        let backup = result.backup.expect("expected a backup");
        assert_eq!(
            fs::read_to_string(backup).unwrap(),
            "name: my custom workflow\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
