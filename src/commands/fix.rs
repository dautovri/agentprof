use std::path::Path;
use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::fixer::FixerEngine;

pub struct FixCommand;

impl FixCommand {
    pub fn execute(target_path: &Path, shell: bool, ignore: bool, zcompile: bool, all: bool) -> Result<()> {
        let run_all = all || (!shell && !ignore && !zcompile);

        println!("{}", "🛠️ Applying optimizations...".bold().cyan());

        if run_all || ignore {
            let files = FixerEngine::generate_ignore_files(target_path)?;
            println!("  ✅ Generated agent ignore files:");
            for f in files {
                println!("     • {}", f.display().green());
            }
        }

        if run_all || shell {
            match FixerEngine::inject_shell_fast_path() {
                Ok((path, modified)) => {
                    if modified {
                        println!("  ✅ Injected agent fast-path bypass into {}", path.display().green());
                        println!("     (Backup created at ~/.zshrc.agentprof.bak)");
                    } else {
                        println!("  ℹ️  Fast-path bypass is already present in {}", path.display().yellow());
                    }
                }
                Err(e) => {
                    println!("  ⚠️ Could not update ~/.zshrc: {}", e);
                }
            }
        }

        if run_all || zcompile {
            match FixerEngine::compile_zshrc_bytecode() {
                Ok(true) => println!("  ✅ Compiled ~/.zshrc to bytecode via `zcompile`"),
                Ok(false) => println!("  ℹ️  `zcompile` step skipped (file not found or unchanged)"),
                Err(e) => println!("  ⚠️ `zcompile` failed: {}", e),
            }
        }

        println!("\n{}", "🎉 Optimization complete! Re-run `agentprof scan` to verify improvements.".green().bold());
        Ok(())
    }
}
