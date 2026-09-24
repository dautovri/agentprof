use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::tokens::TokenCounter;

/// How a server's schema token figure was obtained.
///
/// The distinction matters: a measured figure is the real cost of that server's
/// tool definitions, an estimate is a guess from config size and must never be
/// presented as if it were measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    /// Live `tools/list` handshake with the server.
    Probed,
    /// Replayed from a previous successful probe.
    Cached,
    /// Heuristic from the config entry; not a measurement.
    Estimated,
    /// Probe was attempted and failed.
    ProbeFailed,
}

impl MeasurementSource {
    pub fn label(&self) -> &'static str {
        match self {
            MeasurementSource::Probed => "measured",
            MeasurementSource::Cached => "cached",
            MeasurementSource::Estimated => "estimate",
            MeasurementSource::ProbeFailed => "probe failed",
        }
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, MeasurementSource::Probed | MeasurementSource::Cached)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub config_source: String,
    pub command_or_type: String,
    pub scope: String,
    pub tool_count: Option<usize>,
    pub schema_tokens: Option<usize>,
    pub measurement: MeasurementSource,
    pub tool_names: Vec<String>,
    pub environment_keys: Vec<String>,
    pub probe_error: Option<String>,
    /// Argv from the config entry, carried through to probe time.
    #[serde(default, skip_serializing)]
    pub raw_args: Option<Vec<String>>,
    pub status: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpProfileReport {
    pub servers: Vec<McpServerInfo>,
    pub total_servers: usize,
    /// Sum over servers with a measured or estimated figure.
    pub total_schema_tokens: usize,
    /// Sum over servers whose figure came from a real handshake.
    pub measured_schema_tokens: usize,
    pub measured_server_count: usize,
    pub pct_of_128k_context: f64,
    pub est_cost_per_100_turns: f64,
    pub config_files_scanned: Vec<String>,
    pub probed: bool,
    pub recommendations: Vec<String>,
}

/// Persisted probe results, so the default (non-probing) run can still show
/// real numbers once a probe has been done.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProbeCache {
    entries: HashMap<String, CachedProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProbe {
    tool_count: usize,
    schema_tokens: usize,
    tool_names: Vec<String>,
}

pub struct McpProfiler;

impl McpProfiler {
    pub fn profile(workspace_root: &Path) -> Result<McpProfileReport> {
        Self::profile_with_options(workspace_root, false, Duration::from_secs(10))
    }

    pub fn profile_with_options(
        workspace_root: &Path,
        probe: bool,
        timeout: Duration,
    ) -> Result<McpProfileReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let mut servers_map: HashMap<String, McpServerInfo> = HashMap::new();
        let mut scanned_files = Vec::new();

        let workspace_key = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        let candidate_paths = vec![
            (home.join(".claude.json"), "~/.claude.json".to_string()),
            (
                home.join(".claude/settings.json"),
                "~/.claude/settings.json".to_string(),
            ),
            (
                home.join("Library/Application Support/Claude/claude_desktop_config.json"),
                "~/Library/Application Support/Claude/claude_desktop_config.json".to_string(),
            ),
            (
                home.join(".config/opencode/opencode.json"),
                "~/.config/opencode/opencode.json".to_string(),
            ),
            (
                home.join(".config/opencode/config.json"),
                "~/.config/opencode/config.json".to_string(),
            ),
            (
                home.join(".config/opencode/settings.json"),
                "~/.config/opencode/settings.json".to_string(),
            ),
            (
                home.join(".codex/config.json"),
                "~/.codex/config.json".to_string(),
            ),
            (
                workspace_root.join(".mcp.json"),
                ".mcp.json (workspace)".to_string(),
            ),
            (
                workspace_root.join("opencode.json"),
                "opencode.json (workspace)".to_string(),
            ),
            (
                workspace_root.join(".cursor/mcp.json"),
                ".cursor/mcp.json".to_string(),
            ),
            (
                workspace_root.join(".vscode/mcp.json"),
                ".vscode/mcp.json".to_string(),
            ),
        ];

        for (path, label) in candidate_paths {
            if !path.exists() {
                continue;
            }
            scanned_files.push(label.clone());
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };

            let scope = if label.starts_with('~') {
                "global"
            } else {
                "workspace"
            };
            Self::extract_mcp_servers(&json, &label, scope, &mut servers_map);

            // ~/.claude.json additionally carries per-project server blocks; only
            // the entry for this workspace is in play for this run.
            if let Some(projects) = json.get("projects").and_then(|p| p.as_object()) {
                for (project_path, project_cfg) in projects {
                    if Path::new(project_path) == workspace_key {
                        Self::extract_mcp_servers(
                            project_cfg,
                            &format!("{} (project scope)", label),
                            "project",
                            &mut servers_map,
                        );
                    }
                }
            }
        }

        let cache_path = Self::cache_path(&home);
        let mut cache = Self::load_cache(&cache_path);

        let mut servers: Vec<McpServerInfo> = servers_map.into_values().collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));

        for server in &mut servers {
            if probe {
                match Self::probe_server(server, timeout) {
                    Ok((tool_names, schema_tokens)) => {
                        cache.entries.insert(
                            server.name.clone(),
                            CachedProbe {
                                tool_count: tool_names.len(),
                                schema_tokens,
                                tool_names: tool_names.clone(),
                            },
                        );
                        server.tool_count = Some(tool_names.len());
                        server.schema_tokens = Some(schema_tokens);
                        server.tool_names = tool_names;
                        server.measurement = MeasurementSource::Probed;
                    }
                    Err(e) => {
                        server.measurement = MeasurementSource::ProbeFailed;
                        server.probe_error = Some(e.to_string());
                    }
                }
            } else if let Some(hit) = cache.entries.get(&server.name) {
                server.tool_count = Some(hit.tool_count);
                server.schema_tokens = Some(hit.schema_tokens);
                server.tool_names = hit.tool_names.clone();
                server.measurement = MeasurementSource::Cached;
            }

            Self::finalize_status(server);
        }

        if probe {
            let _ = Self::save_cache(&cache_path, &cache);
        }

        servers.sort_by(|a, b| {
            b.schema_tokens
                .unwrap_or(0)
                .cmp(&a.schema_tokens.unwrap_or(0))
                .then_with(|| a.name.cmp(&b.name))
        });

        let total_servers = servers.len();
        let total_schema_tokens: usize = servers.iter().filter_map(|s| s.schema_tokens).sum();
        let measured: Vec<&McpServerInfo> = servers
            .iter()
            .filter(|s| s.measurement.is_measured())
            .collect();
        let measured_schema_tokens: usize = measured.iter().filter_map(|s| s.schema_tokens).sum();
        let measured_server_count = measured.len();

        let pct_of_128k_context = TokenCounter::context_percentage(total_schema_tokens, 128_000);
        let est_cost_per_100_turns = TokenCounter::estimate_cost_per_100_turns(total_schema_tokens);

        let mut recommendations = Vec::new();
        let unmeasured = total_servers - measured_server_count;
        if unmeasured > 0 && !probe {
            recommendations.push(format!(
                "{} of {} server(s) have never been measured. Run `agentprof mcp --probe` to read their real tool schemas.",
                unmeasured, total_servers
            ));
        }
        if total_servers > 3 {
            recommendations.push(format!(
                "{} MCP servers are configured. Each connected server injects its tool definitions into every prompt.",
                total_servers
            ));
        }
        if measured_schema_tokens > 5_000 {
            recommendations.push(format!(
                "Measured tool schemas total {} tokens on every turn. Disable servers you are not actively using.",
                measured_schema_tokens
            ));
        }
        for s in &servers {
            if let Some(rec) = &s.recommendation {
                recommendations.push(rec.clone());
            }
        }

        Ok(McpProfileReport {
            servers,
            total_servers,
            total_schema_tokens,
            measured_schema_tokens,
            measured_server_count,
            pct_of_128k_context,
            est_cost_per_100_turns,
            config_files_scanned: scanned_files,
            probed: probe,
            recommendations,
        })
    }

    fn finalize_status(server: &mut McpServerInfo) {
        let tokens = server.schema_tokens;
        server.status = match (tokens, server.measurement) {
            (_, MeasurementSource::ProbeFailed) => "❔ Unreachable".to_string(),
            (None, _) => "❔ Not measured".to_string(),
            (Some(t), m) if t > 3_000 => format!("🚨 Heavy ({})", m.label()),
            (Some(t), m) if t > 1_500 => format!("⚠️ Moderate ({})", m.label()),
            (Some(_), m) => format!("✅ Lean ({})", m.label()),
        };

        server.recommendation = match tokens {
            Some(t) if t > 3_000 && server.measurement.is_measured() => Some(format!(
                "Server '{}' injects {} tokens across {} tools on every turn. Scope it to the sessions that need it.",
                server.name,
                t,
                server.tool_count.unwrap_or(0)
            )),
            _ => None,
        };
    }

    fn extract_mcp_servers(
        json: &serde_json::Value,
        source_label: &str,
        scope: &str,
        servers_map: &mut HashMap<String, McpServerInfo>,
    ) {
        let Some(obj) = json
            .get("mcpServers")
            .or_else(|| json.get("mcp_servers"))
            .or_else(|| json.get("mcp"))
            .and_then(|v| v.as_object())
        else {
            return;
        };

        for (name, val) in obj {
            // Some configs carry non-object placeholders under `mcp`.
            if !val.is_object() {
                continue;
            }
            let cmd = val
                .get("command")
                .and_then(|c| c.as_str())
                .or_else(|| val.get("url").and_then(|u| u.as_str()))
                .or_else(|| val.get("type").and_then(|t| t.as_str()))
                .unwrap_or("stdio")
                .to_string();

            let env_keys = val
                .get("env")
                .and_then(|e| e.as_object())
                .map(|e| e.keys().cloned().collect())
                .unwrap_or_default();

            let raw_args = val.get("args").and_then(|a| a.as_array()).map(|args| {
                args.iter()
                    .filter_map(|a| a.as_str().map(String::from))
                    .collect::<Vec<String>>()
            });

            servers_map
                .entry(name.clone())
                .or_insert_with(|| McpServerInfo {
                    name: name.clone(),
                    config_source: source_label.to_string(),
                    command_or_type: cmd,
                    scope: scope.to_string(),
                    tool_count: None,
                    schema_tokens: None,
                    measurement: MeasurementSource::Estimated,
                    tool_names: Vec::new(),
                    environment_keys: env_keys,
                    probe_error: None,
                    raw_args,
                    status: String::new(),
                    recommendation: None,
                });
        }
    }

    fn cache_path(home: &Path) -> PathBuf {
        home.join(".cache/agentprof/mcp-probe-cache.json")
    }

    fn load_cache(path: &Path) -> ProbeCache {
        fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn save_cache(path: &Path, cache: &ProbeCache) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(cache)?)?;
        Ok(())
    }

    /// Performs a real MCP stdio handshake (`initialize` -> `tools/list`) and
    /// counts the tokens the returned tool definitions would occupy.
    fn probe_server(server: &McpServerInfo, timeout: Duration) -> Result<(Vec<String>, usize)> {
        let command = &server.command_or_type;
        if command.starts_with("http://") || command.starts_with("https://") {
            anyhow::bail!("remote (HTTP/SSE) servers are not probed over stdio");
        }
        if command == "stdio" {
            anyhow::bail!("config entry has no executable command");
        }

        let args: Vec<String> = server.raw_args.clone().unwrap_or_default();

        let mut child = Command::new(command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not start '{}': {}", command, e))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout pipe"))?;

        let init = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "agentprof", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        let initialized =
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let list =
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});

        let write_result = (|| -> std::io::Result<()> {
            writeln!(stdin, "{}", init)?;
            writeln!(stdin, "{}", initialized)?;
            writeln!(stdin, "{}", list)?;
            stdin.flush()
        })();
        if let Err(e) = write_result {
            let _ = child.kill();
            anyhow::bail!("handshake write failed: {}", e);
        }

        // Pipe reads cannot be given a deadline directly, so the read happens on
        // a worker thread and the parent enforces the timeout.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(|i| i.as_u64()) == Some(2) {
                    let _ = tx.send(value);
                    return;
                }
            }
            let _ = tx.send(serde_json::Value::Null);
        });

        let response = rx.recv_timeout(timeout);
        let _ = child.kill();
        let _ = child.wait();

        let response = response
            .map_err(|_| anyhow::anyhow!("no tools/list response within {}s", timeout.as_secs()))?;

        let tools = response
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
            .ok_or_else(|| anyhow::anyhow!("server returned no tool list"))?;

        let tool_names = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        // Agents serialize the whole tool array into the prompt, so the token
        // cost is the cost of that serialization.
        let schema_tokens = TokenCounter::count_cl100k(&serde_json::to_string(tools)?);

        Ok((tool_names, schema_tokens))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extracts_servers_from_mcp_servers_key() {
        let json = serde_json::json!({
            "mcpServers": {
                "alpha": {"command": "/bin/alpha", "args": ["--stdio"]},
                "beta": {"url": "https://example.test/mcp"}
            }
        });
        let mut map = HashMap::new();
        McpProfiler::extract_mcp_servers(&json, "test.json", "global", &mut map);
        assert_eq!(map.len(), 2);
        assert_eq!(map["alpha"].command_or_type, "/bin/alpha");
        assert_eq!(
            map["alpha"].raw_args.as_deref(),
            Some(&["--stdio".to_string()][..])
        );
        assert_eq!(map["beta"].command_or_type, "https://example.test/mcp");
    }

    #[test]
    fn test_no_servers_yields_empty_map() {
        let json = serde_json::json!({"unrelated": true});
        let mut map = HashMap::new();
        McpProfiler::extract_mcp_servers(&json, "test.json", "global", &mut map);
        assert!(map.is_empty());
    }

    #[test]
    fn test_unmeasured_server_reports_no_token_figure() {
        let mut server = McpServerInfo {
            name: "x".into(),
            config_source: "c".into(),
            command_or_type: "cmd".into(),
            scope: "global".into(),
            tool_count: None,
            schema_tokens: None,
            measurement: MeasurementSource::Estimated,
            tool_names: vec![],
            environment_keys: vec![],
            probe_error: None,
            raw_args: None,
            status: String::new(),
            recommendation: None,
        };
        McpProfiler::finalize_status(&mut server);
        assert_eq!(server.status, "❔ Not measured");
        assert!(server.schema_tokens.is_none());
        assert!(server.recommendation.is_none());
    }

    #[test]
    fn test_remote_server_is_not_probed_over_stdio() {
        let server = McpServerInfo {
            name: "remote".into(),
            config_source: "c".into(),
            command_or_type: "https://example.test/mcp".into(),
            scope: "global".into(),
            tool_count: None,
            schema_tokens: None,
            measurement: MeasurementSource::Estimated,
            tool_names: vec![],
            environment_keys: vec![],
            probe_error: None,
            raw_args: None,
            status: String::new(),
            recommendation: None,
        };
        assert!(McpProfiler::probe_server(&server, Duration::from_millis(200)).is_err());
    }
}
