use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

pub struct CiGenerator;

impl CiGenerator {
    pub fn generate_github_action(workspace_root: &Path) -> Result<PathBuf> {
        let canonical_root = workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
        let workflows_dir = canonical_root.join(".github/workflows");
        fs::create_dir_all(&workflows_dir)?;

        let workflow_file = workflows_dir.join("agentprof-audit.yml");

        let content = r#"name: AI Agent Workspace Audit

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

      - name: Install agentprof
        run: cargo install agentprof --git https://github.com/dautovri/agentprof || true

      - name: Run agentprof Context & Secret Audit
        run: |
          if command -v agentprof &> /dev/null; then
            agentprof scan
          else
            echo "::notice title=agentprof::Running standalone instruction audit"
            find . -name "AGENTS.md" -o -name "CLAUDE.md" | while read -r file; do
              lines=$(wc -l < "$file")
              if [ "$lines" -gt 300 ]; then
                echo "::warning file=$file::Instruction file exceeds 300 lines ($lines lines). Consider splitting into JIT rules."
              fi
            done
          fi
"#;

        fs::write(&workflow_file, content)
            .with_context(|| format!("Failed to write {}", workflow_file.display()))?;

        Ok(workflow_file)
    }
}
