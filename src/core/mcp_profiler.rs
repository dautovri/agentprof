use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub config_source: String,
    pub command_or_type: String,
    pub estimated_tool_count: usize,
    pub estimated_schema_tokens: usize,
    pub environment_keys: Vec<String>,
    pub status: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpProfileReport {
    pub servers: Vec<McpServerInfo>,
    pub total_servers: usize,
    pub total_estimated_tokens: usize,
    pub pct_of_128k_context: f64,
    pub est_cost_per_100_turns: f64,
    pub config_files_scanned: Vec<String>,
    pub recommendations: Vec<String>,
}

pub struct McpProfiler;

impl McpProfiler {
    pub fn profile(workspace_root: &Path) -> Result<McpProfileReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let mut servers_map: HashMap<String, McpServerInfo> = HashMap::new();
        let mut scanned_files = Vec::new();

        // Candidates for MCP configs
        let candidate_paths = vec![
            (home.join(".config/opencode/opencode.json"), "~/.config/opencode/opencode.json"),
            (home.join(".config/opencode/config.json"), "~/.config/opencode/config.json"),
            (home.join(".config/opencode/settings.json"), "~/.config/opencode/settings.json"),
            (home.join(".claude/settings.json"), "~/.claude/settings.json"),
            (home.join("Library/Application Support/Claude/claude_desktop_config.json"), "~/Library/Application Support/Claude/claude_desktop_config.json"),
            (workspace_root.join("opencode.json"), "opencode.json (workspace)"),
            (workspace_root.join(".cursor/mcp.json"), ".cursor/mcp.json"),
        ];

        for (path, label) in candidate_paths {
            if path.exists() {
                scanned_files.push(label.to_string());
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        Self::extract_mcp_servers(&json, label, &mut servers_map);
                    }
                }
            }
        }

        let mut servers: Vec<McpServerInfo> = servers_map.into_values().collect();
        servers.sort_by(|a, b| b.estimated_schema_tokens.cmp(&a.estimated_schema_tokens));

        let total_servers = servers.len();
        let total_estimated_tokens: usize = servers.iter().map(|s| s.estimated_schema_tokens).sum();
        let pct_of_128k_context = TokenCounter::context_percentage(total_estimated_tokens, 128_000);
        let est_cost_per_100_turns = TokenCounter::estimate_cost_per_100_turns(total_estimated_tokens);

        let mut recommendations = Vec::new();
        if total_servers > 3 {
            recommendations.push(format!("{} MCP servers are active. Each server injects tool definitions into every prompt.", total_servers));
        }
        if total_estimated_tokens > 5_000 {
            recommendations.push("MCP tool schemas consume >5,000 tokens on every turn. Consider disabling unused servers in inactive sessions.".to_string());
        }

        Ok(McpProfileReport {
            servers,
            total_servers,
            total_estimated_tokens,
            pct_of_128k_context,
            est_cost_per_100_turns,
            config_files_scanned: scanned_files,
            recommendations,
        })
    }

    fn extract_mcp_servers(
        json: &serde_json::Value,
        source_label: &str,
        servers_map: &mut HashMap<String, McpServerInfo>,
    ) {
        let mcp_obj = json.get("mcpServers")
            .or_else(|| json.get("mcp"))
            .and_then(|v| v.as_object());

        if let Some(obj) = mcp_obj {
            for (name, val) in obj {
                let cmd = val.get("command")
                    .and_then(|c| c.as_str())
                    .or_else(|| val.get("type").and_then(|t| t.as_str()))
                    .unwrap_or("stdio")
                    .to_string();

                let mut env_keys = Vec::new();
                if let Some(env) = val.get("env").and_then(|e| e.as_object()) {
                    for k in env.keys() {
                        env_keys.push(k.clone());
                    }
                }

                // Estimate tools and schema token overhead based on known MCP server types
                let (tools, tokens) = Self::estimate_server_overhead(name, &cmd, val);

                let status = if tokens > 3_000 {
                    "🚨 Heavy Schema".to_string()
                } else if tokens > 1_500 {
                    "⚠️ Moderate".to_string()
                } else {
                    "✅ Lean".to_string()
                };

                let rec = if tokens > 3_000 {
                    Some(format!("Server '{}' injects ~{} tokens of tool schemas. Defer or scope to specific tasks.", name, tokens))
                } else {
                    None
                };

                servers_map.insert(
                    name.clone(),
                    McpServerInfo {
                        name: name.clone(),
                        config_source: source_label.to_string(),
                        command_or_type: cmd,
                        estimated_tool_count: tools,
                        estimated_schema_tokens: tokens,
                        environment_keys: env_keys,
                        status,
                        recommendation: rec,
                    },
                );
            }
        }
    }

    fn estimate_server_overhead(name: &str, _cmd: &str, val: &serde_json::Value) -> (usize, usize) {
        let n = name.to_lowercase();
        // Base estimation by server domain
        if n.contains("frida") {
            (12, 4_800)
        } else if n.contains("lldb") {
            (18, 6_200)
        } else if n.contains("github") || n.contains("git") {
            (24, 8_500)
        } else if n.contains("postgres") || n.contains("mysql") || n.contains("sqlite") {
            (8, 2_400)
        } else if n.contains("context7") || n.contains("docs") {
            (3, 1_100)
        } else if n.contains("nvd") || n.contains("cve") {
            (4, 1_400)
        } else if n.contains("filesystem") || n.contains("fetch") {
            (6, 1_800)
        } else {
            // General heuristic: check args or json size
            let raw_len = val.to_string().len();
            let estimated_tools = (raw_len / 100).max(4);
            let estimated_tokens = (estimated_tools * 350).max(1_200);
            (estimated_tools, estimated_tokens)
        }
    }
}
