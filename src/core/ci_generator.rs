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

    pub(crate) fn workflow_yaml(min_score: usize) -> String {
        format!(
            r#"name: AI Agent Workspace Audit

on:
  pull_request:
    branches: [main, master, develop]
  push:
    branches: [main, master]

jobs:
  agentprof-audit:
    name: Audit Context & Instruction Budget
    runs-on: ubuntu-latest
    steps:
      - name: Checkout Code
        uses: actions/checkout@v4

      - name: Install Rust Toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo bin
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin
          key: agentprof-${{{{ runner.os }}}}

      - name: Install agentprof
        run: cargo install --git https://github.com/dautovri/agentprof agentprof --locked

      - name: Write health report to the job summary
        run: agentprof report --markdown >> "$GITHUB_STEP_SUMMARY"

      # This step is the gate: `--fail-under` exits non-zero when the workspace
      # health score drops below the threshold, failing the check.
      - name: Enforce agent workspace health budget
        run: agentprof report --fail-under {min_score}

      - name: Fail on secrets reachable by agent tools
        run: |
          exposed=$(agentprof scan --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspace"]["total_exposed_secrets"])')
          echo "Exposed secret files: $exposed"
          if [ "$exposed" -gt 0 ]; then
            echo "::error title=agentprof::$exposed secret file(s) are reachable by agent tools"
            exit 1
          fi
"#,
            min_score = min_score
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_embeds_the_threshold_and_gates() {
        let yaml = CiGenerator::workflow_yaml(75);
        assert!(yaml.contains("--fail-under 75"));
        assert!(yaml.contains("exit 1"));
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
