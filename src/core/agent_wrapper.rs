use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;

use crate::core::shell_bench::ShellBenchmarker;

pub struct AgentWrapper;

impl AgentWrapper {
    pub fn wrap_command(cmd_and_args: &[String]) -> Result<i32> {
        let Some((program, args)) = cmd_and_args.split_first() else {
            anyhow::bail!("No command specified to wrap. Example: `agentprof wrap claude`");
        };

        let guard_installed = ShellBenchmarker::check_for_agent_guard();

        // agentprof's own output goes to stderr so the wrapped program's stdout
        // stays clean for pipes and redirects.

        eprintln!(
            "{}",
            "⚡ [agentprof] Wrapping agent session...".bold().cyan()
        );
        eprintln!("  • Program:  {}", program.bold());
        eprintln!("  • Injected: AGENTPROF_FAST_PATH=1, AGENTPROF_ACTIVE=1");
        if !guard_installed {
            eprintln!(
                "  {}",
                "• No fast-path guard found in your shell rc — the variables are exported but nothing consumes them yet. Run `agentprof fix --shell`."
                    .yellow()
            );
        }
        eprintln!();

        let start_time = Instant::now();

        let mut child = Command::new(program)
            .args(args)
            .env("AGENTPROF_FAST_PATH", "1")
            .env("AGENTPROF_ACTIVE", "1")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("Failed to spawn program '{}'", program))?;

        let status = child.wait()?;
        let duration = start_time.elapsed();

        // A signal-terminated child has no exit code. Returning 0 there reported
        // a crashed agent as a clean exit; the shell convention is 128 + signal.
        let exit_code = status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|s| 128 + s).unwrap_or(1)
            }
            #[cfg(not(unix))]
            {
                1
            }
        });

        eprintln!("\n{}", "═".repeat(78).dimmed());
        eprintln!("{}", "🛫 [agentprof] Flight Recorder Summary".bold().cyan());
        eprintln!("  • Total Session Duration: {:.1}s", duration.as_secs_f64());
        eprintln!(
            "  • Exit Status:            {}",
            if exit_code == 0 {
                "✅ Clean exit (0)".green().to_string()
            } else {
                format!("⚠️ Exit code {}", exit_code).yellow().to_string()
            }
        );

        // The previous build printed a fixed "Saved ~15-30s via agent fast-path"
        // on every run, regardless of whether a guard existed or how long the
        // session was. Report only what is actually known.
        eprintln!(
            "  • Shell Fast-Path:        {}",
            if guard_installed {
                "active: shells this session starts skip your rc file (PATH restored from snapshot)"
                    .green()
                    .to_string()
            } else {
                "not installed — no shell startup cost was avoided"
                    .dimmed()
                    .to_string()
            }
        );
        eprintln!("{}", "═".repeat(78).dimmed());

        Ok(exit_code)
    }
}
