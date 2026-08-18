use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Discarded spawns that warm the page cache before timing starts.
const WARMUP_ITERATIONS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellBenchmarkResult {
    pub shell_name: String,
    pub shell_path: String,
    pub iterations: usize,
    /// None when the shell could not be spawned at all.
    pub interactive_login_ms: Option<f64>,
    pub non_interactive_ms: Option<f64>,
    pub latency_tax_ms: Option<f64>,
    pub estimated_50_tool_calls_sec: Option<f64>,
    pub has_agent_fast_path: bool,
    pub rating: Option<ShellPerformanceRating>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellPerformanceRating {
    BlazingFast,
    Good,
    Sluggish,
    Critical,
}

impl ShellPerformanceRating {
    /// Thresholds are on the *latency tax* — the avoidable per-command cost an
    /// agent pays — not on total startup, which includes work a plain
    /// non-interactive spawn would do anyway.
    pub fn from_tax_ms(tax_ms: f64) -> Self {
        if tax_ms < 25.0 {
            ShellPerformanceRating::BlazingFast
        } else if tax_ms < 80.0 {
            ShellPerformanceRating::Good
        } else if tax_ms < 250.0 {
            ShellPerformanceRating::Sluggish
        } else {
            ShellPerformanceRating::Critical
        }
    }

    pub fn badge(&self) -> &'static str {
        match self {
            ShellPerformanceRating::BlazingFast => "⚡ Blazing Fast (<25ms tax)",
            ShellPerformanceRating::Good => "✅ Good (25-80ms tax)",
            ShellPerformanceRating::Sluggish => "⚠️ Sluggish (80-250ms tax)",
            ShellPerformanceRating::Critical => "🚨 Critical (>250ms tax - slowing agent)",
        }
    }
}

pub struct ShellBenchmarker;

impl ShellBenchmarker {
    /// Resolves the shell to benchmark from `$SHELL`, falling back to zsh then
    /// bash. Benchmarking a hardcoded zsh on a bash or fish machine measures
    /// something the user never runs.
    pub fn detect_shell() -> (String, PathBuf) {
        if let Ok(shell) = std::env::var("SHELL")
            && !shell.is_empty()
        {
            let path = PathBuf::from(&shell);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| shell.clone());
            if path.exists() {
                return (name, path);
            }
        }
        for candidate in ["zsh", "bash", "sh"] {
            if let Ok(path) = which::which(candidate) {
                return (candidate.to_string(), path);
            }
        }
        ("sh".to_string(), PathBuf::from("/bin/sh"))
    }

    pub fn run_benchmark(iterations: usize) -> Result<ShellBenchmarkResult> {
        let iters = iterations.max(3);
        let (shell_name, shell_path) = Self::detect_shell();
        let has_agent_fast_path = Self::check_for_agent_guard();

        let base = ShellBenchmarkResult {
            shell_name: shell_name.clone(),
            shell_path: shell_path.display().to_string(),
            iterations: iters,
            interactive_login_ms: None,
            non_interactive_ms: None,
            latency_tax_ms: None,
            estimated_50_tool_calls_sec: None,
            has_agent_fast_path,
            rating: None,
            error: None,
        };

        // fish does not accept the POSIX `-lic` bundle; skip rather than time a
        // usage error.
        if shell_name == "fish" || shell_name == "nu" {
            return Ok(ShellBenchmarkResult {
                error: Some(format!(
                    "{} does not support POSIX `-lic` startup flags; interactive-vs-non-interactive comparison is not applicable.",
                    shell_name
                )),
                ..base
            });
        }

        let non_interactive_ms = match Self::measure(&shell_path, &["-c", "true"], iters) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ShellBenchmarkResult {
                    error: Some(format!("could not spawn {}: {}", shell_path.display(), e)),
                    ..base
                });
            }
        };

        let interactive_login_ms = match Self::measure(&shell_path, &["-lic", "true"], iters) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ShellBenchmarkResult {
                    non_interactive_ms: Some(non_interactive_ms),
                    error: Some(format!("interactive login shell failed: {}", e)),
                    ..base
                });
            }
        };

        let latency_tax_ms = (interactive_login_ms - non_interactive_ms).max(0.0);

        Ok(ShellBenchmarkResult {
            interactive_login_ms: Some(interactive_login_ms),
            non_interactive_ms: Some(non_interactive_ms),
            latency_tax_ms: Some(latency_tax_ms),
            estimated_50_tool_calls_sec: Some((latency_tax_ms * 50.0) / 1000.0),
            rating: Some(ShellPerformanceRating::from_tax_ms(latency_tax_ms)),
            ..base
        })
    }

    /// Returns the **median** spawn time in ms, after discarding warm-up runs.
    ///
    /// The mean was previously used, which let a single scheduling hiccup swing
    /// the headline number by 100ms+ between consecutive runs.
    fn measure(shell: &Path, args: &[&str], iterations: usize) -> Result<f64> {
        for _ in 0..WARMUP_ITERATIONS {
            Self::spawn_once(shell, args)?;
        }

        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            samples.push(Self::spawn_once(shell, args)?);
        }
        Ok(Self::median_ms(&mut samples))
    }

    fn spawn_once(shell: &Path, args: &[&str]) -> Result<Duration> {
        let start = Instant::now();
        let status = Command::new(shell)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        let elapsed = start.elapsed();
        if !status.success() {
            anyhow::bail!(
                "`{} {}` exited with {}",
                shell.display(),
                args.join(" "),
                status
            );
        }
        Ok(elapsed)
    }

    pub(crate) fn median_ms(samples: &mut [Duration]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        samples.sort();
        let mid = samples.len() / 2;
        if samples.len().is_multiple_of(2) {
            let a = samples[mid - 1].as_secs_f64() * 1000.0;
            let b = samples[mid].as_secs_f64() * 1000.0;
            (a + b) / 2.0
        } else {
            samples[mid].as_secs_f64() * 1000.0
        }
    }

    /// Detects an agentprof-installed fast-path guard.
    ///
    /// Only the agentprof sentinel counts. Matching a bare mention of
    /// `CLAUDE_CODE` reported "guard installed" for any rc file that merely
    /// referenced the variable.
    pub fn check_for_agent_guard() -> bool {
        let Ok(home) = std::env::var("HOME") else {
            return false;
        };
        let home = PathBuf::from(home);
        for rc in [".zshrc", ".bashrc", ".bash_profile", ".profile"] {
            if let Ok(content) = std::fs::read_to_string(home.join(rc))
                && content.contains(crate::core::fixer::FAST_PATH_SENTINEL)
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_of_odd_sample_count() {
        let mut s = vec![
            Duration::from_millis(30),
            Duration::from_millis(10),
            Duration::from_millis(20),
        ];
        assert!((ShellBenchmarker::median_ms(&mut s) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_median_of_even_sample_count() {
        let mut s = vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
            Duration::from_millis(40),
        ];
        assert!((ShellBenchmarker::median_ms(&mut s) - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_median_ignores_single_outlier() {
        // The mean here is 226ms; the median stays at the honest 20ms.
        let mut s = vec![
            Duration::from_millis(20),
            Duration::from_millis(20),
            Duration::from_millis(1040),
        ];
        assert!((ShellBenchmarker::median_ms(&mut s) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_empty_samples_do_not_panic() {
        assert_eq!(ShellBenchmarker::median_ms(&mut []), 0.0);
    }

    #[test]
    fn test_rating_thresholds_are_on_latency_tax() {
        assert_eq!(
            ShellPerformanceRating::from_tax_ms(10.0),
            ShellPerformanceRating::BlazingFast
        );
        assert_eq!(
            ShellPerformanceRating::from_tax_ms(50.0),
            ShellPerformanceRating::Good
        );
        assert_eq!(
            ShellPerformanceRating::from_tax_ms(150.0),
            ShellPerformanceRating::Sluggish
        );
        assert_eq!(
            ShellPerformanceRating::from_tax_ms(600.0),
            ShellPerformanceRating::Critical
        );
    }

    #[test]
    fn test_detect_shell_returns_existing_path() {
        let (name, path) = ShellBenchmarker::detect_shell();
        assert!(!name.is_empty());
        assert!(!path.as_os_str().is_empty());
    }
}
