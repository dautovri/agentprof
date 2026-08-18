use std::path::Path;

use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::fixer::{FixAction, FixOutcome, FixerEngine};

pub struct FixCommand;

impl FixCommand {
    pub fn execute(
        target_path: &Path,
        shell: bool,
        ignore: bool,
        zcompile: bool,
        all: bool,
        dry_run: bool,
        json: bool,
    ) -> Result<()> {
        // `--all` opts into the shell changes; a bare `agentprof fix` only
        // touches files inside the workspace.
        let run_shell = shell || all;
        let run_zcompile = zcompile || all;
        let run_ignore = ignore || all || (!shell && !zcompile);

        if !json {
            let header = if dry_run {
                "🔍 Previewing optimizations (no files will be written)..."
            } else {
                "🛠️ Applying optimizations..."
            };
            println!("{}", header.bold().cyan());
        }

        let mut actions: Vec<FixAction> = Vec::new();

        if run_ignore {
            actions.extend(FixerEngine::generate_ignore_files(target_path, dry_run)?);
        }
        if run_shell {
            match FixerEngine::inject_shell_fast_path(dry_run) {
                Ok(action) => actions.push(action),
                Err(e) if !json => println!("  ⚠️ Shell fast-path skipped: {}", e),
                Err(_) => {}
            }
        }
        if run_zcompile {
            match FixerEngine::compile_zshrc_bytecode(dry_run) {
                Ok(action) => actions.push(action),
                Err(e) if !json => println!("  ⚠️ zcompile skipped: {}", e),
                Err(_) => {}
            }
        }

        if json {
            println!("{}", serde_json::to_string_pretty(&actions)?);
            return Ok(());
        }

        for action in &actions {
            let (icon, label) = match action.outcome {
                FixOutcome::Created => ("✅", "created"),
                FixOutcome::Updated => ("✅", "updated"),
                FixOutcome::AlreadyApplied => ("ℹ️ ", "already applied"),
                FixOutcome::WouldChange => ("📝", "would change"),
                FixOutcome::Skipped => ("⏭️ ", "skipped"),
            };
            println!(
                "  {} {} [{}] — {}",
                icon,
                action.target.display().green(),
                label,
                action.detail.dimmed()
            );
            if let Some(backup) = &action.backup {
                println!("       backup: {}", backup.display().to_string().dimmed());
            }
        }

        if run_shell && !dry_run && actions.iter().any(|a| a.outcome == FixOutcome::Created) {
            println!(
                "\n{}",
                "Note: the shell guard is opt-in. It only activates when AGENTPROF_FAST_PATH=1 is set (as `agentprof wrap` does)."
                    .dimmed()
            );
        }

        println!(
            "\n{}",
            if dry_run {
                "Preview complete. Re-run without --dry-run to apply.".bold().to_string()
            } else {
                "🎉 Done. Re-run `agentprof scan` to verify improvements.".green().bold().to_string()
            }
        );
        Ok(())
    }
}
