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
    /// `<shell> -c true`: the unavoidable cost of any spawn. None when the
    /// shell could not be spawned at all.
    pub non_interactive_ms: Option<f64>,
    /// `<shell> -lc true`: a login shell, as agents that run commands through
    /// `bash -lc` start (it reads the login profile).
    #[serde(default)]
    pub login_ms: Option<f64>,
    /// `<shell> -lic true`: an interactive login shell (reads the rc file too).
    pub interactive_login_ms: Option<f64>,
    /// Claude Code captures your shell once per session and replays that
    /// snapshot before every command it runs.
    #[serde(default)]
    pub claude_snapshot: Option<SnapshotReplay>,
    /// Interactive-login time minus the baseline.
    pub latency_tax_ms: Option<f64>,
    /// What an agent pays per command on top of a bare spawn: the larger of
    /// the Claude Code snapshot replay and the login-shell tax.
    #[serde(default)]
    pub per_command_tax_ms: Option<f64>,
    #[serde(default)]
    pub per_command_tax_source: Option<String>,
    pub estimated_50_tool_calls_sec: Option<f64>,
    pub has_agent_fast_path: bool,
    pub rating: Option<ShellPerformanceRating>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReplay {
    pub path: PathBuf,
    pub size_bytes: u64,
    /// Median time to spawn the shell and source the snapshot.
    pub replay_ms: f64,
    /// `replay_ms` minus a bare spawn.
    pub tax_ms: f64,
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
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::run_benchmark_in(iterations, home.as_deref())
    }

    pub(crate) fn run_benchmark_in(
        iterations: usize,
        home: Option<&Path>,
    ) -> Result<ShellBenchmarkResult> {
        let iters = iterations.max(3);
        let (shell_name, shell_path) = Self::detect_shell();
        let has_agent_fast_path = Self::check_for_agent_guard();

        let base = ShellBenchmarkResult {
            shell_name: shell_name.clone(),
            shell_path: shell_path.display().to_string(),
            iterations: iters,
            non_interactive_ms: None,
            login_ms: None,
            interactive_login_ms: None,
            claude_snapshot: None,
            latency_tax_ms: None,
            per_command_tax_ms: None,
            per_command_tax_source: None,
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
        let login_ms = Self::measure(&shell_path, &["-lc", "true"], iters).ok();

        let interactive_login_ms = match Self::measure(&shell_path, &["-lic", "true"], iters) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ShellBenchmarkResult {
                    non_interactive_ms: Some(non_interactive_ms),
                    login_ms,
                    error: Some(format!("interactive login shell failed: {}", e)),
                    ..base
                });
            }
        };

        let claude_snapshot = home
            .and_then(|h| Self::find_claude_snapshot(h, &shell_name))
            .and_then(|snapshot| {
                let path_arg = snapshot.to_string_lossy().to_string();
                let replay_ms = Self::measure(
                    &shell_path,
                    &[
                        "-c",
                        "source \"$1\" >/dev/null 2>&1; true",
                        "agentprof",
                        &path_arg,
                    ],
                    iters,
                )
                .ok()?;
                Some(SnapshotReplay {
                    size_bytes: std::fs::metadata(&snapshot).map(|m| m.len()).unwrap_or(0),
                    path: snapshot,
                    replay_ms,
                    tax_ms: (replay_ms - non_interactive_ms).max(0.0),
                })
            });

        let latency_tax_ms = (interactive_login_ms - non_interactive_ms).max(0.0);

        // Score what agents actually pay per command, not a shell mode they
        // may never use.
        let mut candidates: Vec<(f64, String)> = Vec::new();
        if let Some(snapshot) = &claude_snapshot {
            candidates.push((snapshot.tax_ms, "Claude Code snapshot replay".to_string()));
        }
        if let Some(login) = login_ms {
            candidates.push((
                (login - non_interactive_ms).max(0.0),
                format!("login shell (`{} -lc`)", shell_name),
            ));
        }
        let (per_command_tax_ms, per_command_tax_source) = candidates
            .into_iter()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(tax, source)| (Some(tax), Some(source)))
            .unwrap_or((None, None));

        Ok(ShellBenchmarkResult {
            non_interactive_ms: Some(non_interactive_ms),
            login_ms,
            interactive_login_ms: Some(interactive_login_ms),
            claude_snapshot,
            latency_tax_ms: Some(latency_tax_ms),
            per_command_tax_ms,
            per_command_tax_source,
            estimated_50_tool_calls_sec: per_command_tax_ms.map(|t| t * 50.0 / 1000.0),
            rating: per_command_tax_ms.map(ShellPerformanceRating::from_tax_ms),
            ..base
        })
    }

    /// The newest Claude Code shell snapshot for `shell_name`, if any.
    pub(crate) fn find_claude_snapshot(home: &Path, shell_name: &str) -> Option<PathBuf> {
        let prefix = format!("snapshot-{}-", shell_name);
        std::fs::read_dir(home.join(".claude/shell-snapshots"))
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with(&prefix) && name.ends_with(".sh")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, path)| path)
    }

    /// Returns the **median** spawn time in ms, after discarding warm-up runs.
    ///
    /// The mean was previously used, which let a single scheduling hiccup swing
    /// the headline number by 100ms+ between consecutive runs.
    pub(crate) fn measure(shell: &Path, args: &[&str], iterations: usize) -> Result<f64> {
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
    fn test_finds_the_newest_snapshot_for_the_shell() {
        let home = std::env::temp_dir().join(format!("agentprof_snap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let dir = home.join(".claude/shell-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("snapshot-zsh-1-a.sh"), "true").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("snapshot-zsh-2-b.sh"), "true").unwrap();
        std::fs::write(dir.join("snapshot-bash-3-c.sh"), "true").unwrap();

        let found = ShellBenchmarker::find_claude_snapshot(&home, "zsh").unwrap();
        assert!(
            found.ends_with("snapshot-zsh-2-b.sh"),
            "{}",
            found.display()
        );
        assert!(ShellBenchmarker::find_claude_snapshot(&home, "fish").is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn test_snapshot_replay_is_measured_and_scored() {
        let home = std::env::temp_dir().join(format!("agentprof_snap_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let (shell_name, _) = ShellBenchmarker::detect_shell();
        let dir = home.join(".claude/shell-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("snapshot-{}-1-x.sh", shell_name)),
            "export AGENTPROF_SNAP=1\n",
        )
        .unwrap();

        let result = ShellBenchmarker::run_benchmark_in(3, Some(&home)).unwrap();
        if result.error.is_none() {
            let snapshot = result.claude_snapshot.expect("snapshot should be measured");
            assert!(snapshot.replay_ms > 0.0);
            assert!(result.per_command_tax_ms.is_some());
            assert!(result.per_command_tax_source.is_some());
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn test_detect_shell_returns_existing_path() {
        let (name, path) = ShellBenchmarker::detect_shell();
        assert!(!name.is_empty());
        assert!(!path.as_os_str().is_empty());
    }
}
