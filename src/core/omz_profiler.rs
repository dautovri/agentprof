use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::core::shell_bench::ShellBenchmarker;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmzPluginReport {
    pub name: String,
    /// None when the plugin could not be located or timed. Absent measurements
    /// are reported as absent rather than filled in with a placeholder number.
    pub latency_ms: Option<f64>,
    pub percentage_of_total: f64,
    pub is_slow: bool,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowInitHook {
    pub command: String,
    pub tool_name: String,
    /// None when the hook was detected in the rc file but not independently timed.
    pub latency_ms: Option<f64>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmzProfileReport {
    pub is_omz_installed: bool,
    pub omz_path: Option<PathBuf>,
    pub theme_name: Option<String>,
    pub total_omz_overhead_ms: f64,
    pub total_shell_startup_ms: f64,
    pub plugins: Vec<OmzPluginReport>,
    pub slow_hooks: Vec<SlowInitHook>,
    pub bytecode_compiled: bool,
    pub recommendations: Vec<String>,
}

pub struct OmzProfiler;

impl OmzProfiler {
    pub fn profile() -> Result<OmzProfileReport> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .context("HOME environment variable not set")?;

        let zshrc_path = home.join(".zshrc");
        let omz_default_path = home.join(".oh-my-zsh");

        let is_omz_installed = omz_default_path.exists() || std::env::var("ZSH").is_ok();
        let omz_path = if omz_default_path.exists() {
            Some(omz_default_path)
        } else {
            std::env::var("ZSH").ok().map(PathBuf::from)
        };

        if !zshrc_path.exists() {
            return Ok(OmzProfileReport {
                is_omz_installed: false,
                omz_path: None,
                theme_name: None,
                total_omz_overhead_ms: 0.0,
                total_shell_startup_ms: 0.0,
                plugins: Vec::new(),
                slow_hooks: Vec::new(),
                bytecode_compiled: false,
                recommendations: vec!["No ~/.zshrc file found on this system.".to_string()],
            });
        }

        let zshrc_content = fs::read_to_string(&zshrc_path).unwrap_or_default();
        let plugins = Self::extract_plugins(&zshrc_content);
        let theme_name = Self::extract_theme(&zshrc_content);

        // Check if bytecode compiled (.zwc files exist)
        let zshrc_zwc = home.join(".zshrc.zwc");
        let bytecode_compiled = zshrc_zwc.exists();

        // 1. Total interactive startup: the median of real spawns, like
        //    `bench`. A single un-warmed run swung by 100ms+ between calls, and
        //    a missing zsh used to be reported as a ~0ms startup.
        let total_shell_startup_ms = which::which("zsh")
            .ok()
            .and_then(|zsh| ShellBenchmarker::measure(&zsh, &["-lic", "true"], 5).ok())
            .unwrap_or(0.0);

        // 2. Profile individual plugins
        let plugin_reports = Self::profile_plugins(&plugins, &omz_path, &home);

        // 3. Scan for slow evaluation hooks (nvm, starship, pyenv, brew)
        let slow_hooks = Self::detect_slow_hooks(&zshrc_content);

        let total_omz_overhead_ms: f64 = plugin_reports.iter().filter_map(|p| p.latency_ms).sum();

        let mut recommendations = Vec::new();

        if !bytecode_compiled && total_shell_startup_ms > 80.0 {
            recommendations.push(
                "Compile ~/.zshrc and plugins with `zcompile` to reduce disk I/O latency."
                    .to_string(),
            );
        }

        for plugin in &plugin_reports {
            if plugin.is_slow
                && let Some(rec) = &plugin.recommendation
            {
                recommendations.push(rec.clone());
            }
        }

        for hook in &slow_hooks {
            if hook.latency_ms.is_none_or(|ms| ms > 40.0) {
                recommendations.push(format!("{}: {}", hook.tool_name, hook.suggestion));
            }
        }

        if total_shell_startup_ms > 150.0 {
            recommendations.push("Add agent fast-path bypass (`agentprof fix --shell`) so AI subshells skip heavy hooks entirely.".to_string());
        }

        Ok(OmzProfileReport {
            is_omz_installed,
            omz_path,
            theme_name,
            total_omz_overhead_ms,
            total_shell_startup_ms,
            plugins: plugin_reports,
            slow_hooks,
            bytecode_compiled,
            recommendations,
        })
    }

    fn extract_plugins(zshrc: &str) -> Vec<String> {
        let mut list = Vec::new();
        // `plugins=(git nvm docker ...)`, possibly spanning lines. A
        // commented-out `# plugins=(...)` line is not the active list.
        let re = Regex::new(r"(?m)^[ \t]*plugins=\(([^)]*)\)").unwrap();
        if let Some(caps) = re.captures(zshrc)
            && let Some(matched) = caps.get(1)
        {
            for line in matched.as_str().lines() {
                let line = line.split('#').next().unwrap_or("");
                for token in line.split_whitespace() {
                    list.push(token.to_string());
                }
            }
        }
        list
    }

    fn extract_theme(zshrc: &str) -> Option<String> {
        let re = Regex::new(r#"ZSH_THEME=["']?([^"'\n]+)["']?"#).unwrap();
        re.captures(zshrc)
            .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
    }

    fn profile_plugins(
        plugins: &[String],
        omz_path: &Option<PathBuf>,
        home: &Path,
    ) -> Vec<OmzPluginReport> {
        let mut reports = Vec::new();
        if plugins.is_empty() {
            return reports;
        }

        // Previously fell back to a hardcoded "/Users/rd/.oh-my-zsh" — the
        // original author's own home directory, which exists on no other machine.
        let default_omz = home.join(".oh-my-zsh");
        let omz_dir = omz_path.as_deref().unwrap_or(&default_omz);

        let custom_dir = std::env::var("ZSH_CUSTOM")
            .map(PathBuf::from)
            .unwrap_or_else(|_| omz_dir.join("custom"));

        for plugin in plugins {
            // Find plugin path
            let custom_plugin = custom_dir
                .join("plugins")
                .join(plugin)
                .join(format!("{}.plugin.zsh", plugin));
            let standard_plugin = omz_dir
                .join("plugins")
                .join(plugin)
                .join(format!("{}.plugin.zsh", plugin));

            let target_path = if custom_plugin.exists() {
                Some(custom_plugin)
            } else if standard_plugin.exists() {
                Some(standard_plugin)
            } else {
                None
            };

            // A plugin that cannot be found is not timed at all. The previous
            // code substituted invented constants (2.0 / 1.5 / 1.0 ms) that were
            // then displayed and summed as if they were measurements.
            let latency_ms = target_path.and_then(|path| Self::time_source(&path, omz_dir));

            let is_slow = latency_ms.is_some_and(|ms| ms > 25.0);
            let rec = match latency_ms {
                Some(ms) if plugin == "nvm" => Some(format!(
                    "Plugin 'nvm' costs {:.1}ms per shell. Lazy-load it so the penalty is paid only when node/npm runs.",
                    ms
                )),
                Some(ms) if plugin == "git" && ms > 30.0 => Some(format!(
                    "The git plugin costs {:.1}ms per shell because it inspects repository status. Consider a cached git prompt.",
                    ms
                )),
                Some(ms) if is_slow => Some(format!(
                    "Plugin '{}' costs {:.1}ms per shell. Evaluate whether every session needs it.",
                    plugin, ms
                )),
                _ => None,
            };

            reports.push(OmzPluginReport {
                name: plugin.clone(),
                latency_ms,
                percentage_of_total: 0.0, // populated after sum
                is_slow,
                recommendation: rec,
            });
        }

        let total: f64 = reports.iter().filter_map(|r| r.latency_ms).sum();
        for r in &mut reports {
            if total > 0.0 {
                r.percentage_of_total = (r.latency_ms.unwrap_or(0.0) / total) * 100.0;
            }
        }

        // Sort slowest first; unmeasured plugins sink to the bottom.
        reports.sort_by(|a, b| {
            b.latency_ms
                .unwrap_or(-1.0)
                .partial_cmp(&a.latency_ms.unwrap_or(-1.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        reports
    }

    /// Times sourcing one plugin in an rc-free shell that has Oh My Zsh's own
    /// library loaded first. Plugins call into that library; sourcing one
    /// without it timed an early error exit instead of the plugin's real work.
    fn time_source(path: &Path, omz_dir: &Path) -> Option<f64> {
        let script = format!(
            "ZSH={omz}; for f in $ZSH/lib/*.zsh(N); do source $f; done >/dev/null 2>&1; \
             zmodload zsh/datetime; start=$EPOCHREALTIME; source {plugin} >/dev/null 2>&1; \
             end=$EPOCHREALTIME; print $(( (end - start) * 1000 ))",
            omz = shell_quote(&omz_dir.display().to_string()),
            plugin = shell_quote(&path.display().to_string())
        );
        // -f skips rc files so the measurement isolates this script.
        let out = Command::new("zsh").args(["-fc", &script]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f64>()
            .ok()
    }

    /// Detects known-slow initialization hooks in the rc file.
    ///
    /// Each detected hook is timed by running the tool's real init command in an
    /// isolated shell. Hooks that cannot be timed report `None` — previously they
    /// were assigned invented constants (nvm 280ms, conda 190ms, everything else
    /// 35-40ms) which were then rendered as if measured.
    fn detect_slow_hooks(zshrc: &str) -> Vec<SlowInitHook> {
        struct HookSpec {
            /// Substring that indicates the hook is present in the rc file.
            marker: &'static str,
            name: &'static str,
            /// A complete, runnable command used to time the hook, if one exists.
            timing_command: Option<&'static str>,
            suggestion: &'static str,
        }

        let specs = [
            HookSpec {
                marker: "nvm.sh",
                name: "NVM (Node Version Manager)",
                timing_command: Some("[ -r \"$HOME/.nvm/nvm.sh\" ] && . \"$HOME/.nvm/nvm.sh\""),
                suggestion: "Lazy-load NVM so its cost is paid only when node/npm is first invoked.",
            },
            HookSpec {
                marker: "starship init zsh",
                name: "Starship Prompt",
                timing_command: Some("starship init zsh"),
                suggestion: "Starship is fast, but ensure it is skipped in non-interactive agent shells.",
            },
            HookSpec {
                marker: "pyenv init",
                name: "Pyenv (Python Version Manager)",
                // The old pattern was the truncated string `eval "$(pyenv init`,
                // which is not a runnable command: it always errored and fell
                // back to a hardcoded 35ms.
                timing_command: Some("pyenv init -"),
                suggestion: "Lazy-load pyenv to avoid re-evaluating shims on every shell start.",
            },
            HookSpec {
                marker: "rbenv init",
                name: "Rbenv (Ruby Version Manager)",
                timing_command: Some("rbenv init -"),
                suggestion: "Lazy-load rbenv or defer it until ruby is executed.",
            },
            HookSpec {
                marker: "brew shellenv",
                name: "Homebrew Shellenv",
                timing_command: Some("brew shellenv"),
                suggestion: "Inline Homebrew's PATH exports instead of shelling out to `brew shellenv` each start.",
            },
            HookSpec {
                marker: "conda.sh",
                name: "Conda / Anaconda",
                timing_command: None,
                suggestion: "Conda initialization adds significant startup cost. Lazy-load it.",
            },
            HookSpec {
                marker: "sdkman-init.sh",
                name: "SDKMAN",
                timing_command: None,
                suggestion: "SDKMAN sources a large init script on every shell. Lazy-load it.",
            },
        ];

        let mut hooks = Vec::new();
        for spec in specs {
            if !zshrc.contains(spec.marker) {
                continue;
            }
            let latency_ms = spec.timing_command.and_then(Self::time_command);
            hooks.push(SlowInitHook {
                command: spec.marker.to_string(),
                tool_name: spec.name.to_string(),
                latency_ms,
                suggestion: spec.suggestion.to_string(),
            });
        }

        hooks.sort_by(|a, b| {
            b.latency_ms
                .unwrap_or(-1.0)
                .partial_cmp(&a.latency_ms.unwrap_or(-1.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hooks
    }

    /// Times a complete init command in an rc-free shell, returning None if the
    /// tool is absent or the command fails.
    fn time_command(command: &str) -> Option<f64> {
        let script = format!(
            "zmodload zsh/datetime; start=$EPOCHREALTIME; eval \"$({})\" >/dev/null 2>&1 || exit 1; end=$EPOCHREALTIME; print $(( (end - start) * 1000 ))",
            command
        );
        let out = Command::new("zsh").args(["-fc", &script]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f64>()
            .ok()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_plugins_parses_list() {
        let rc = "ZSH_THEME=\"robbyrussell\"\nplugins=(git nvm docker)\n";
        assert_eq!(
            OmzProfiler::extract_plugins(rc),
            vec!["git".to_string(), "nvm".to_string(), "docker".to_string()]
        );
    }

    #[test]
    fn test_extract_plugins_skips_comments_and_spans_lines() {
        let rc = "# plugins=(old stale)\nplugins=(\n  git # vcs\n  docker\n)\n";
        assert_eq!(
            OmzProfiler::extract_plugins(rc),
            vec!["git".to_string(), "docker".to_string()]
        );
    }

    #[test]
    fn test_extract_theme() {
        assert_eq!(
            OmzProfiler::extract_theme("ZSH_THEME=\"agnoster\"\n"),
            Some("agnoster".to_string())
        );
        assert_eq!(OmzProfiler::extract_theme("# nothing here\n"), None);
    }

    #[test]
    fn test_hooks_absent_from_rc_are_not_reported() {
        let hooks = OmzProfiler::detect_slow_hooks("# an empty rc file\n");
        assert!(hooks.is_empty());
    }

    #[test]
    fn test_detected_hook_without_tool_reports_no_invented_latency() {
        // conda has no timing command, so its latency must be absent rather
        // than a stand-in constant.
        let hooks = OmzProfiler::detect_slow_hooks("source /opt/conda/etc/profile.d/conda.sh\n");
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].latency_ms, None);
    }

    #[test]
    fn test_shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/a/b"), "'/a/b'");
        assert!(shell_quote("/it's").contains(r"'\''"));
    }
}
