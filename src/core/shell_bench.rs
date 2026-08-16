use std::process::Command;
use std::time::{Duration, Instant};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellBenchmarkResult {
    pub shell_name: String,
    pub interactive_login_ms: f64,
    pub non_interactive_ms: f64,
    pub latency_tax_ms: f64,
    pub estimated_50_tool_calls_sec: f64,
    pub has_agent_fast_path: bool,
    pub rating: ShellPerformanceRating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellPerformanceRating {
    BlazingFast, // < 25ms
    Good,        // 25ms - 80ms
    Sluggish,    // 80ms - 250ms
    Critical,    // > 250ms
}

impl ShellPerformanceRating {
    pub fn badge(&self) -> &'static str {
        match self {
            ShellPerformanceRating::BlazingFast => "⚡ Blazing Fast (<25ms)",
            ShellPerformanceRating::Good => "✅ Good (<80ms)",
            ShellPerformanceRating::Sluggish => "⚠️ Sluggish (80-250ms)",
            ShellPerformanceRating::Critical => "🚨 Critical (>250ms - slowing agent)",
        }
    }
}

pub struct ShellBenchmarker;

impl ShellBenchmarker {
    /// Benchmarks interactive login vs non-interactive shell spawn times
    pub fn run_benchmark(iterations: usize) -> Result<ShellBenchmarkResult> {
        let iters = iterations.max(3);

        // 1. Measure non-interactive (zsh -c "true")
        let mut non_interactive_durations = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = Instant::now();
            let _ = Command::new("zsh")
                .args(["-c", "true"])
                .output();
            non_interactive_durations.push(start.elapsed());
        }
        let non_interactive_ms = Self::average_ms(&non_interactive_durations);

        // 2. Measure interactive login shell (zsh -lic "true")
        let mut interactive_durations = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = Instant::now();
            let _ = Command::new("zsh")
                .args(["-lic", "true"])
                .output();
            interactive_durations.push(start.elapsed());
        }
        let interactive_login_ms = Self::average_ms(&interactive_durations);

        let latency_tax_ms = (interactive_login_ms - non_interactive_ms).max(0.0);
        let estimated_50_tool_calls_sec = (latency_tax_ms * 50.0) / 1000.0;

        // Check if agent fast-path guard exists in ~/.zshrc
        let has_agent_fast_path = Self::check_for_agent_guard();

        let rating = if interactive_login_ms < 35.0 {
            ShellPerformanceRating::BlazingFast
        } else if interactive_login_ms < 100.0 {
            ShellPerformanceRating::Good
        } else if interactive_login_ms < 250.0 {
            ShellPerformanceRating::Sluggish
        } else {
            ShellPerformanceRating::Critical
        };

        Ok(ShellBenchmarkResult {
            shell_name: "zsh".to_string(),
            interactive_login_ms,
            non_interactive_ms,
            latency_tax_ms,
            estimated_50_tool_calls_sec,
            has_agent_fast_path,
            rating,
        })
    }

    fn average_ms(durations: &[Duration]) -> f64 {
        if durations.is_empty() {
            return 0.0;
        }
        let total: f64 = durations.iter().map(|d| d.as_secs_f64() * 1000.0).sum();
        total / durations.len() as f64
    }

    pub fn check_for_agent_guard() -> bool {
        let home = match std::env::var("HOME") {
            Ok(h) => std::path::PathBuf::from(h),
            Err(_) => return false,
        };
        let zshrc = home.join(".zshrc");
        if let Ok(content) = std::fs::read_to_string(zshrc) {
            content.contains("CLAUDE_CODE") 
                || content.contains("OPENCODE") 
                || content.contains("AI_AGENT")
                || content.contains("[agentprof]")
                || content.contains("[[ $- != *i* ]]")
        } else {
            false
        }
    }
}
