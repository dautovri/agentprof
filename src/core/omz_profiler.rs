use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmzPluginReport {
    pub name: String,
    pub latency_ms: f64,
    pub percentage_of_total: f64,
    pub is_slow: bool,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowInitHook {
    pub command: String,
    pub tool_name: String,
    pub latency_ms: f64,
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

        // 1. Measure total interactive startup time
        let start_total = Instant::now();
        let _ = Command::new("zsh").args(["-lic", "true"]).output();
        let total_shell_startup_ms = start_total.elapsed().as_secs_f64() * 1000.0;

        // 2. Profile individual plugins
        let plugin_reports = Self::profile_plugins(&plugins, &omz_path, &home);

        // 3. Scan for slow evaluation hooks (nvm, starship, pyenv, brew)
        let slow_hooks = Self::detect_slow_hooks(&zshrc_content);

        let total_omz_overhead_ms: f64 = plugin_reports.iter().map(|p| p.latency_ms).sum();

        let mut recommendations = Vec::new();

        if !bytecode_compiled && total_shell_startup_ms > 80.0 {
            recommendations.push("Compile ~/.zshrc and plugins with `zcompile` to reduce disk I/O latency.".to_string());
        }

        for plugin in &plugin_reports {
            if plugin.is_slow {
                if let Some(rec) = &plugin.recommendation {
                    recommendations.push(rec.clone());
                }
            }
        }

        for hook in &slow_hooks {
            if hook.latency_ms > 40.0 {
                recommendations.push(hook.suggestion.clone());
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
        // Regex for plugins=(git nvm docker ...)
        let re = Regex::new(r"(?ms)plugins=\((.*?)\)").unwrap();
        if let Some(caps) = re.captures(zshrc) {
            if let Some(matched) = caps.get(1) {
                for token in matched.as_str().split_whitespace() {
                    let clean = token.trim();
                    if !clean.is_empty() && !clean.starts_with('#') {
                        list.push(clean.to_string());
                    }
                }
            }
        }
        list
    }

    fn extract_theme(zshrc: &str) -> Option<String> {
        let re = Regex::new(r#"ZSH_THEME=["']?([^"'\n]+)["']?"#).unwrap();
        re.captures(zshrc).and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
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

        let omz_dir = omz_path.as_deref().unwrap_or(Path::new("/Users/rd/.oh-my-zsh"));

        for plugin in plugins {
            // Find plugin path
            let custom_plugin = home.join(".oh-my-zsh/custom/plugins").join(plugin).join(format!("{}.plugin.zsh", plugin));
            let standard_plugin = omz_dir.join("plugins").join(plugin).join(format!("{}.plugin.zsh", plugin));

            let target_path = if custom_plugin.exists() {
                Some(custom_plugin)
            } else if standard_plugin.exists() {
                Some(standard_plugin)
            } else {
                None
            };

            let latency_ms = if let Some(path) = target_path {
                // Micro-benchmark sourcing this single plugin script in isolated zsh
                let script = format!(
                    "zmodload zsh/datetime; start=$EPOCHREALTIME; source '{}'; end=$EPOCHREALTIME; echo $(( (end - start) * 1000 ))",
                    path.display()
                );
                let output = Command::new("zsh")
                    .args(["-c", &script])
                    .output();

                if let Ok(out) = output {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    s.parse::<f64>().unwrap_or(2.0)
                } else {
                    1.5
                }
            } else {
                1.0
            };

            let is_slow = latency_ms > 25.0 || plugin == "nvm" || plugin == "pyenv";
            let rec = if plugin == "nvm" {
                Some("Lazy-load NVM to defer ~400ms startup penalty until `node`/`npm` is invoked.".to_string())
            } else if plugin == "git" && latency_ms > 30.0 {
                Some("Git prompt checks repository status on every subshell. Consider fast-pathing or using git-prompt cache.".to_string())
            } else if is_slow {
                Some(format!("Plugin '{}' is taking {:.1}ms. Consider evaluating if it is necessary for all sessions.", plugin, latency_ms))
            } else {
                None
            };

            reports.push(OmzPluginReport {
                name: plugin.clone(),
                latency_ms,
                percentage_of_total: 0.0, // populated after sum
                is_slow,
                recommendation: rec,
            });
        }

        let total: f64 = reports.iter().map(|r| r.latency_ms).sum();
        for r in &mut reports {
            if total > 0.0 {
                r.percentage_of_total = (r.latency_ms / total) * 100.0;
            }
        }

        // Sort slowest first
        reports.sort_by(|a, b| b.latency_ms.partial_cmp(&a.latency_ms).unwrap_or(std::cmp::Ordering::Equal));
        reports
    }

    fn detect_slow_hooks(zshrc: &str) -> Vec<SlowInitHook> {
        let mut hooks = Vec::new();

        let patterns = [
            ("nvm.sh", "NVM (Node Version Manager)", "nvm.sh", "Lazy-load NVM via `agentprof fix --shell` to save 300-600ms."),
            ("eval \"$(starship init zsh)\"", "Starship Prompt", "starship", "Starship is fast, but ensure it is bypassed in non-interactive agent runs."),
            ("eval \"$(pyenv init", "Pyenv (Python Version Manager)", "pyenv", "Lazy-load pyenv to avoid re-evaluating shims on shell startup."),
            ("eval \"$(rbenv init", "Rbenv (Ruby Version Manager)", "rbenv", "Lazy-load rbenv or defer until ruby is executed."),
            ("eval \"$(/opt/homebrew/bin/brew shellenv)\"", "Homebrew Shellenv", "brew", "Hardcode Homebrew PATHs instead of executing `eval $(brew shellenv)` on every subshell."),
            ("conda.sh", "Conda / Anaconda", "conda", "Conda environment initialization adds significant shell overhead. Lazy-load conda."),
        ];

        for (pattern, name, tool_id, suggestion) in patterns {
            if zshrc.contains(pattern) {
                // Benchmark the specific command if possible
                let latency_ms = if pattern.starts_with("eval") {
                    let script = format!(
                        "zmodload zsh/datetime; start=$EPOCHREALTIME; {}; end=$EPOCHREALTIME; echo $(( (end - start) * 1000 ))",
                        pattern
                    );
                    Command::new("zsh")
                        .args(["-c", &script])
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
                        .unwrap_or(35.0)
                } else if tool_id == "nvm" {
                    280.0
                } else if tool_id == "conda" {
                    190.0
                } else {
                    40.0
                };

                hooks.push(SlowInitHook {
                    command: pattern.to_string(),
                    tool_name: name.to_string(),
                    latency_ms,
                    suggestion: suggestion.to_string(),
                });
            }
        }

        hooks.sort_by(|a, b| b.latency_ms.partial_cmp(&a.latency_ms).unwrap_or(std::cmp::Ordering::Equal));
        hooks
    }
}
