use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::shell_bench::ShellBenchmarker;
use crate::ui::tables::TableRenderer;

pub struct BenchCommand;

impl BenchCommand {
    pub fn execute(iterations: usize, json: bool) -> Result<()> {
        if !json {
            println!("{}", format!("⚡ Benchmarking subshell spawn times ({} iterations)...", iterations).bold());
        }
        let result = ShellBenchmarker::run_benchmark(iterations)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        TableRenderer::render_shell_benchmark(&result);
        println!();
        Ok(())
    }
}
