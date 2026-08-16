use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentPlatformType {
    OpenCode,
    Claude,
    Grok,
}

impl AgentPlatformType {
    pub fn name(&self) -> &'static str {
        match self {
            AgentPlatformType::OpenCode => "OpenCode",
            AgentPlatformType::Claude => "Claude Code",
            AgentPlatformType::Grok => "Grok (xAI)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigFile {
    pub name: String,
    pub path: PathBuf,
    pub relative_path: String,
    pub lines: usize,
    pub tokens: usize,
    pub bytes: usize,
    pub is_global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillInfo {
    pub name: String,
    pub path: PathBuf,
    pub tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPlatformProfile {
    pub platform: AgentPlatformType,
    pub is_installed: bool,
    pub global_config_dir: Option<PathBuf>,
    pub config_files: Vec<AgentConfigFile>,
    pub skills: Vec<AgentSkillInfo>,
    pub total_skills_count: usize,
    pub total_skills_tokens: usize,
    pub total_fixed_instruction_tokens: usize,
    pub cache_history_size_bytes: usize,
    pub detected_mcp_servers: Vec<String>,
    pub context_window_size: usize,
    pub fixed_payload_percentage: f64,
    pub est_turn_cost_usd: f64,
    pub health_rating: &'static str,
    pub recommendations: Vec<String>,
}

pub struct AgentPlatformProfiler;

impl AgentPlatformProfiler {
    pub fn profile_all(workspace_root: &Path) -> Result<Vec<AgentPlatformProfile>> {
        let opencode = Self::profile_opencode(workspace_root)?;
        let claude = Self::profile_claude(workspace_root)?;
        let grok = Self::profile_grok(workspace_root)?;
        Ok(vec![opencode, claude, grok])
    }

    pub fn profile_opencode(workspace: &Path) -> Result<AgentPlatformProfile> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let global_dir = home.join(".config/opencode");
        let is_installed = global_dir.exists() || which::which("opencode").is_ok() || workspace.join(".opencode").exists();

        let mut config_files = Vec::new();
        let mut skills = Vec::new();
        let mut detected_mcp_servers = Vec::new();
        let cache_history_size = 0usize;

        // 1. Global config files
        if global_dir.exists() {
            let files = ["AGENTS.md", "opencode.json", "settings.json", "cli.json", "config.json"];
            for f in files {
                let p = global_dir.join(f);
                if p.exists() {
                    if let Ok(c) = fs::read_to_string(&p) {
                        let tokens = TokenCounter::count_cl100k(&c);
                        config_files.push(AgentConfigFile {
                            name: f.to_string(),
                            path: p.clone(),
                            relative_path: format!("~/.config/opencode/{}", f),
                            lines: c.lines().count(),
                            tokens,
                            bytes: c.len(),
                            is_global: true,
                        });

                        // Check for MCP servers in json
                        if (f == "opencode.json" || f == "config.json" || f == "settings.json") && (c.contains("mcpServers") || c.contains("mcp")) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&c) {
                                if let Some(mcp) = val.get("mcpServers").or_else(|| val.get("mcp")) {
                                    if let Some(obj) = mcp.as_object() {
                                        for key in obj.keys() {
                                            if !detected_mcp_servers.contains(key) {
                                                detected_mcp_servers.push(key.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Global skills in ~/.config/opencode/skills
            let skills_dir = global_dir.join("skills");
            if skills_dir.exists() {
                for entry in WalkDir::new(&skills_dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
                    if entry.file_name() == "SKILL.md" {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            let skill_name = entry.path().parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "skill".to_string());
                            skills.push(AgentSkillInfo {
                                name: skill_name,
                                path: entry.path().to_path_buf(),
                                tokens: TokenCounter::count_cl100k(&content),
                            });
                        }
                    }
                }
            }
        }

        // 2. Workspace level instructions
        let ws_agents = workspace.join("AGENTS.md");
        if ws_agents.exists() {
            if let Ok(c) = fs::read_to_string(&ws_agents) {
                config_files.push(AgentConfigFile {
                    name: "AGENTS.md (workspace)".to_string(),
                    path: ws_agents.clone(),
                    relative_path: "AGENTS.md".to_string(),
                    lines: c.lines().count(),
                    tokens: TokenCounter::count_cl100k(&c),
                    bytes: c.len(),
                    is_global: false,
                });
            }
        }

        let total_skills_tokens: usize = skills.iter().map(|s| s.tokens).sum();
        let total_skills_count = skills.len();
        let total_config_tokens: usize = config_files.iter().map(|c| c.tokens).sum();
        let total_fixed_instruction_tokens = total_config_tokens;

        let context_window = 128_000;
        let fixed_payload_percentage = (total_fixed_instruction_tokens as f64 / context_window as f64) * 100.0;
        let est_turn_cost_usd = (total_fixed_instruction_tokens as f64 / 1000.0) * 0.015;

        let mut recommendations = Vec::new();
        if total_fixed_instruction_tokens > 4_000 {
            recommendations.push("Global + local instruction payload exceeds 4k tokens. Prune ~/.config/opencode/AGENTS.md.".to_string());
        }
        if !detected_mcp_servers.is_empty() {
            recommendations.push(format!("{} MCP server(s) active. Ensure heavy schemas are not loaded unconditionally.", detected_mcp_servers.len()));
        }

        let health_rating = if fixed_payload_percentage < 3.0 {
            "✅ Lean & Fast"
        } else if fixed_payload_percentage < 8.0 {
            "⚠️ Moderate Overhead"
        } else {
            "🚨 Bloated"
        };

        Ok(AgentPlatformProfile {
            platform: AgentPlatformType::OpenCode,
            is_installed,
            global_config_dir: if global_dir.exists() { Some(global_dir) } else { None },
            config_files,
            skills,
            total_skills_count,
            total_skills_tokens,
            total_fixed_instruction_tokens,
            cache_history_size_bytes: cache_history_size,
            detected_mcp_servers,
            context_window_size: context_window,
            fixed_payload_percentage,
            est_turn_cost_usd,
            health_rating,
            recommendations,
        })
    }

    pub fn profile_claude(workspace: &Path) -> Result<AgentPlatformProfile> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let global_dir = home.join(".claude");
        let is_installed = global_dir.exists() || which::which("claude").is_ok() || workspace.join(".claude").exists();

        let mut config_files = Vec::new();
        let mut skills = Vec::new();
        let detected_mcp_servers = Vec::new();
        let mut cache_history_size = 0usize;

        if global_dir.exists() {
            let files = ["CLAUDE.md", "settings.json", "launch.json", "RTK.md"];
            for f in files {
                let p = global_dir.join(f);
                if p.exists() {
                    if let Ok(c) = fs::read_to_string(&p) {
                        config_files.push(AgentConfigFile {
                            name: f.to_string(),
                            path: p.clone(),
                            relative_path: format!("~/.claude/{}", f),
                            lines: c.lines().count(),
                            tokens: TokenCounter::count_cl100k(&c),
                            bytes: c.len(),
                            is_global: true,
                        });
                    }
                }
            }

            // Measure file history / memory cache
            let hist_dir = global_dir.join("file-history");
            if hist_dir.exists() {
                for e in WalkDir::new(&hist_dir).max_depth(2).into_iter().filter_map(|e| e.ok()).take(500) {
                    if let Ok(meta) = e.metadata() {
                        cache_history_size += meta.len() as usize;
                    }
                }
            }

            // Global skills in ~/.claude/skills
            let skills_dir = global_dir.join("skills");
            if skills_dir.exists() {
                for entry in WalkDir::new(&skills_dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
                    if entry.file_name() == "SKILL.md" {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            let skill_name = entry.path().parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "skill".to_string());
                            skills.push(AgentSkillInfo {
                                name: skill_name,
                                path: entry.path().to_path_buf(),
                                tokens: TokenCounter::count_cl100k(&content),
                            });
                        }
                    }
                }
            }
        }

        let ws_claude = workspace.join("CLAUDE.md");
        if ws_claude.exists() {
            if let Ok(c) = fs::read_to_string(&ws_claude) {
                config_files.push(AgentConfigFile {
                    name: "CLAUDE.md (workspace)".to_string(),
                    path: ws_claude.clone(),
                    relative_path: "CLAUDE.md".to_string(),
                    lines: c.lines().count(),
                    tokens: TokenCounter::count_cl100k(&c),
                    bytes: c.len(),
                    is_global: false,
                });
            }
        }

        let total_skills_tokens: usize = skills.iter().map(|s| s.tokens).sum();
        let total_skills_count = skills.len();
        let total_config_tokens: usize = config_files.iter().map(|c| c.tokens).sum();
        let total_fixed_instruction_tokens = total_config_tokens;

        let context_window = 200_000;
        let fixed_payload_percentage = (total_fixed_instruction_tokens as f64 / context_window as f64) * 100.0;
        let est_turn_cost_usd = (total_fixed_instruction_tokens as f64 / 1000.0) * 0.015;

        let mut recommendations = Vec::new();
        if cache_history_size > 50_000_000 {
            recommendations.push(format!("Claude file-history cache is large ({:.1} MB). Run cleanup if IDE feels sluggish.", cache_history_size as f64 / 1_000_000.0));
        }
        if total_skills_count > 10 {
            recommendations.push(format!("{} Claude skills installed ({} total tokens). Ensure on-demand loading is enabled.", total_skills_count, crate::ui::formatters::Formatters::format_tokens(total_skills_tokens)));
        }

        let health_rating = if fixed_payload_percentage < 2.0 {
            "✅ Ultra-Lean"
        } else if fixed_payload_percentage < 6.0 {
            "✅ Good"
        } else {
            "⚠️ Heavy"
        };

        Ok(AgentPlatformProfile {
            platform: AgentPlatformType::Claude,
            is_installed,
            global_config_dir: if global_dir.exists() { Some(global_dir) } else { None },
            config_files,
            skills,
            total_skills_count,
            total_skills_tokens,
            total_fixed_instruction_tokens,
            cache_history_size_bytes: cache_history_size,
            detected_mcp_servers,
            context_window_size: context_window,
            fixed_payload_percentage,
            est_turn_cost_usd,
            health_rating,
            recommendations,
        })
    }

    pub fn profile_grok(workspace: &Path) -> Result<AgentPlatformProfile> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let global_dir = home.join(".grok");
        let is_installed = global_dir.exists() || which::which("grok").is_ok() || workspace.join("GROK.md").exists() || workspace.join(".grokrules").exists();

        let mut config_files = Vec::new();
        let skills = Vec::new();
        let detected_mcp_servers = Vec::new();
        let cache_history_size = 0usize;

        if global_dir.exists() {
            let files = ["GROK.md", "config.json", "rules.md", "system.md"];
            for f in files {
                let p = global_dir.join(f);
                if p.exists() {
                    if let Ok(c) = fs::read_to_string(&p) {
                        config_files.push(AgentConfigFile {
                            name: f.to_string(),
                            path: p.clone(),
                            relative_path: format!("~/.grok/{}", f),
                            lines: c.lines().count(),
                            tokens: TokenCounter::count_cl100k(&c),
                            bytes: c.len(),
                            is_global: true,
                        });
                    }
                }
            }
        }

        // Workspace files
        let ws_grok = workspace.join("GROK.md");
        if ws_grok.exists() {
            if let Ok(c) = fs::read_to_string(&ws_grok) {
                config_files.push(AgentConfigFile {
                    name: "GROK.md (workspace)".to_string(),
                    path: ws_grok.clone(),
                    relative_path: "GROK.md".to_string(),
                    lines: c.lines().count(),
                    tokens: TokenCounter::count_cl100k(&c),
                    bytes: c.len(),
                    is_global: false,
                });
            }
        }

        let ws_grokrules = workspace.join(".grokrules");
        if ws_grokrules.exists() {
            if let Ok(c) = fs::read_to_string(&ws_grokrules) {
                config_files.push(AgentConfigFile {
                    name: ".grokrules (workspace)".to_string(),
                    path: ws_grokrules.clone(),
                    relative_path: ".grokrules".to_string(),
                    lines: c.lines().count(),
                    tokens: TokenCounter::count_cl100k(&c),
                    bytes: c.len(),
                    is_global: false,
                });
            }
        }

        let total_config_tokens: usize = config_files.iter().map(|c| c.tokens).sum();
        let total_fixed_instruction_tokens = total_config_tokens;

        let context_window = 128_000;
        let fixed_payload_percentage = (total_fixed_instruction_tokens as f64 / context_window as f64) * 100.0;
        let est_turn_cost_usd = (total_fixed_instruction_tokens as f64 / 1000.0) * 0.015;

        let mut recommendations = Vec::new();
        if !is_installed && config_files.is_empty() {
            recommendations.push("Grok rules not configured for this workspace. Add GROK.md if using Grok agent.".to_string());
        }

        let health_rating = if fixed_payload_percentage < 3.0 {
            "✅ Optimal"
        } else {
            "⚠️ Heavy"
        };

        Ok(AgentPlatformProfile {
            platform: AgentPlatformType::Grok,
            is_installed,
            global_config_dir: if global_dir.exists() { Some(global_dir) } else { None },
            config_files,
            skills,
            total_skills_count: 0,
            total_skills_tokens: 0,
            total_fixed_instruction_tokens,
            cache_history_size_bytes: cache_history_size,
            detected_mcp_servers,
            context_window_size: context_window,
            fixed_payload_percentage,
            est_turn_cost_usd,
            health_rating,
            recommendations,
        })
    }
}
