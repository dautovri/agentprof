use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::tokens::TokenCounter;

/// MCP protocol revision announced during the handshake. Servers negotiate
/// down to a revision they support.
const PROTOCOL_VERSION: &str = "2025-06-18";
/// Probe results older than this are not reused.
const CACHE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const CACHE_VERSION: u32 = 2;
/// Servers probed at the same time.
const PROBE_CONCURRENCY: usize = 8;
/// Context window used for Claude Code's `auto` tool-search threshold.
const CLAUDE_CODE_CONTEXT_WINDOW: usize = 200_000;

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
    /// Replayed from a previous successful probe of the same command.
    Cached,
    /// Not measured yet.
    Estimated,
    /// Probe was attempted and failed.
    ProbeFailed,
    /// Deliberately not started (untrusted workspace config, remote server,
    /// disabled server).
    NotProbed,
}

impl MeasurementSource {
    pub fn label(&self) -> &'static str {
        match self {
            MeasurementSource::Probed => "measured",
            MeasurementSource::Cached => "cached",
            MeasurementSource::Estimated => "estimate",
            MeasurementSource::ProbeFailed => "probe failed",
            MeasurementSource::NotProbed => "not probed",
        }
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, MeasurementSource::Probed | MeasurementSource::Cached)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    /// The agent whose configuration defines this server.
    pub client: String,
    pub config_source: String,
    /// Executable for stdio servers, URL for remote ones.
    pub command_or_type: String,
    /// `stdio` or `remote`.
    pub transport: String,
    /// `global` (user-level), `project` (per-project entry in the user's own
    /// config) or `workspace` (a file inside the repository).
    pub scope: String,
    pub enabled: bool,
    pub tool_count: Option<usize>,
    /// Tokens of the full tool definitions.
    pub schema_tokens: Option<usize>,
    /// Tokens of the tool names alone: what stays in context when an agent
    /// defers tool definitions until they are needed.
    pub tool_name_tokens: Option<usize>,
    pub measurement: MeasurementSource,
    pub tool_names: Vec<String>,
    pub environment_keys: Vec<String>,
    pub probe_error: Option<String>,
    /// Argv and environment from the config entry, carried through to probe
    /// time. Never serialized: environment values are often credentials.
    #[serde(default, skip)]
    pub raw_args: Vec<String>,
    #[serde(default, skip)]
    pub raw_env: Vec<(String, String)>,
    pub status: String,
    pub recommendation: Option<String>,
}

/// The per-turn MCP load of one agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientLoad {
    pub client: String,
    pub enabled_servers: usize,
    pub measured_servers: usize,
    /// Tokens of the measured servers' full tool definitions.
    pub measured_schema_tokens: usize,
    /// Tokens the agent actually puts into every turn for those servers.
    pub upfront_tokens: usize,
    /// `upfront`, or why definitions are deferred.
    pub loading: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpProfileReport {
    pub servers: Vec<McpServerInfo>,
    pub total_servers: usize,
    pub enabled_servers: usize,
    /// Sum over distinct measured servers of their full definitions.
    pub total_schema_tokens: usize,
    /// Same as `total_schema_tokens`; kept for JSON consumers.
    pub measured_schema_tokens: usize,
    pub measured_server_count: usize,
    pub clients: Vec<ClientLoad>,
    /// The largest per-turn load any single agent carries. This is what
    /// budgets and scores use.
    pub max_upfront_tokens: usize,
    pub pct_of_128k_context: f64,
    pub est_cost_per_100_turns: f64,
    pub config_files_scanned: Vec<String>,
    pub probed: bool,
    /// Workspace-defined servers that were not started because the workspace
    /// was not trusted.
    pub untrusted_workspace_servers: usize,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeOptions {
    pub probe: bool,
    /// Also start servers defined by files inside the workspace.
    pub trust_workspace: bool,
    pub timeout: Duration,
}

/// Persisted probe results, so the default (non-probing) run can still show
/// real numbers once a probe has been done.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProbeCache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, CachedProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProbe {
    tool_count: usize,
    schema_tokens: usize,
    #[serde(default)]
    tool_name_tokens: usize,
    tool_names: Vec<String>,
    #[serde(default)]
    probed_at: u64,
}

#[derive(Debug, Clone)]
struct ProbeResult {
    tool_names: Vec<String>,
    schema_tokens: usize,
    tool_name_tokens: usize,
}

/// A configuration file that may define MCP servers.
struct ConfigSource {
    client: &'static str,
    path: PathBuf,
    label: String,
    scope: &'static str,
    toml: bool,
}

pub struct McpProfiler;

impl McpProfiler {
    pub fn profile(workspace_root: &Path) -> Result<McpProfileReport> {
        Self::profile_with_options(
            workspace_root,
            ProbeOptions {
                timeout: Duration::from_secs(10),
                ..Default::default()
            },
        )
    }

    pub fn profile_with_options(
        workspace_root: &Path,
        options: ProbeOptions,
    ) -> Result<McpProfileReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        Self::profile_in(workspace_root, &home, options)
    }

    pub(crate) fn profile_in(
        workspace_root: &Path,
        home: &Path,
        options: ProbeOptions,
    ) -> Result<McpProfileReport> {
        let workspace_key = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        let mut servers: Vec<McpServerInfo> = Vec::new();
        let mut scanned_files = Vec::new();

        for source in Self::config_sources(workspace_root, home) {
            let Ok(content) = fs::read_to_string(&source.path) else {
                continue;
            };
            scanned_files.push(source.label.clone());
            let parsed = if source.toml {
                toml::from_str::<Value>(&content).ok()
            } else {
                serde_json::from_str::<Value>(&content).ok()
            };
            let Some(json) = parsed else {
                continue;
            };

            Self::extract_mcp_servers(&json, &source, source.scope, &mut servers);

            // ~/.claude.json additionally carries per-project server blocks; only
            // the entry for this workspace is in play for this run.
            if let Some(projects) = json.get("projects").and_then(|p| p.as_object()) {
                for (project_path, project_cfg) in projects {
                    if Path::new(project_path) == workspace_key {
                        let project_source = ConfigSource {
                            label: format!("{} (project scope)", source.label),
                            path: source.path.clone(),
                            ..source
                        };
                        Self::extract_mcp_servers(
                            project_cfg,
                            &project_source,
                            "project",
                            &mut servers,
                        );
                        break;
                    }
                }
            }
        }

        // Servers from .mcp.json that the user disabled in Claude Code settings.
        let disabled_mcpjson = Self::claude_disabled_mcpjson_servers(workspace_root, home);
        for s in servers
            .iter_mut()
            .filter(|s| s.client == "Claude Code" && s.scope == "workspace")
        {
            if disabled_mcpjson.contains(&s.name) {
                s.enabled = false;
            }
        }

        let cache_path = Self::cache_path(home);
        let mut cache = Self::load_cache(&cache_path);
        let now = unix_now();

        // Decide what to start, probing each distinct command once.
        let mut to_probe: Vec<(String, McpServerInfo)> = Vec::new();
        let mut queued: HashSet<String> = HashSet::new();
        let mut untrusted = 0;
        for server in servers.iter_mut() {
            let key = Self::probe_key(server);
            if !server.enabled {
                server.measurement = MeasurementSource::NotProbed;
                continue;
            }
            if options.probe {
                if server.transport != "stdio" {
                    server.measurement = MeasurementSource::NotProbed;
                    server.probe_error =
                        Some("remote servers are not probed (they usually need auth)".to_string());
                } else if server.scope == "workspace" && !options.trust_workspace {
                    server.measurement = MeasurementSource::NotProbed;
                    server.probe_error = Some(
                        "defined inside the workspace; re-run with --trust-workspace to start it"
                            .to_string(),
                    );
                    untrusted += 1;
                } else if queued.insert(key.clone()) {
                    to_probe.push((key, server.clone()));
                }
            }
        }

        let results = Self::probe_all(&to_probe, workspace_root, options.timeout);
        for (key, result) in &results {
            if let Ok(r) = result {
                cache.entries.insert(
                    key.clone(),
                    CachedProbe {
                        tool_count: r.tool_names.len(),
                        schema_tokens: r.schema_tokens,
                        tool_name_tokens: r.tool_name_tokens,
                        tool_names: r.tool_names.clone(),
                        probed_at: now,
                    },
                );
            }
        }

        for server in servers.iter_mut() {
            if !server.enabled || server.measurement == MeasurementSource::NotProbed {
                Self::finalize_status(server);
                continue;
            }
            let key = Self::probe_key(server);
            match results.get(&key) {
                Some(Ok(r)) => {
                    server.tool_count = Some(r.tool_names.len());
                    server.schema_tokens = Some(r.schema_tokens);
                    server.tool_name_tokens = Some(r.tool_name_tokens);
                    server.tool_names = r.tool_names.clone();
                    server.measurement = MeasurementSource::Probed;
                }
                Some(Err(e)) => {
                    server.measurement = MeasurementSource::ProbeFailed;
                    server.probe_error = Some(e.clone());
                }
                None => {
                    if let Some(hit) = cache
                        .entries
                        .get(&key)
                        .filter(|c| now.saturating_sub(c.probed_at) < CACHE_TTL_SECS)
                    {
                        server.tool_count = Some(hit.tool_count);
                        server.schema_tokens = Some(hit.schema_tokens);
                        server.tool_name_tokens = Some(hit.tool_name_tokens);
                        server.tool_names = hit.tool_names.clone();
                        server.measurement = MeasurementSource::Cached;
                    }
                }
            }
            Self::finalize_status(server);
        }

        if !results.is_empty() {
            cache.version = CACHE_VERSION;
            let _ = Self::save_cache(&cache_path, &cache);
        }

        servers.sort_by(|a, b| {
            b.schema_tokens
                .unwrap_or(0)
                .cmp(&a.schema_tokens.unwrap_or(0))
                .then_with(|| a.client.cmp(&b.client))
                .then_with(|| a.name.cmp(&b.name))
        });

        let tool_search = ToolSearchMode::detect(workspace_root, home);
        let clients = Self::client_loads(&servers, tool_search);
        let max_upfront_tokens = clients.iter().map(|c| c.upfront_tokens).max().unwrap_or(0);

        // Distinct measured servers (the same command can be configured in
        // several agents).
        let mut seen = HashSet::new();
        let measured: Vec<&McpServerInfo> = servers
            .iter()
            .filter(|s| s.enabled && s.measurement.is_measured())
            .filter(|s| seen.insert(Self::probe_key(s)))
            .collect();
        let measured_schema_tokens: usize = measured.iter().filter_map(|s| s.schema_tokens).sum();
        let measured_server_count = measured.len();
        let total_servers = servers.len();
        let enabled_servers = servers.iter().filter(|s| s.enabled).count();

        let mut recommendations = Vec::new();
        let unmeasured = servers
            .iter()
            .filter(|s| s.enabled && !s.measurement.is_measured())
            .count();
        if unmeasured > 0 && !options.probe {
            recommendations.push(format!(
                "{} enabled server(s) have not been measured. Run `agentprof mcp --probe` to read their real tool schemas.",
                unmeasured
            ));
        }
        if untrusted > 0 {
            recommendations.push(format!(
                "{} server(s) are defined by files inside this workspace and were not started. Only if you trust the repository, re-run with `--trust-workspace`.",
                untrusted
            ));
        }
        for c in &clients {
            if c.upfront_tokens > 5_000 {
                recommendations.push(format!(
                    "{} puts {} tokens of tool definitions into every turn. Disable servers you are not actively using.",
                    c.client, c.upfront_tokens
                ));
            }
        }
        if tool_search == ToolSearchMode::Upfront
            && clients
                .iter()
                .any(|c| c.client == "Claude Code" && c.measured_schema_tokens > 10_000)
        {
            recommendations.push(
                "ENABLE_TOOL_SEARCH=false makes Claude Code load every MCP tool definition up front. Unset it to defer them."
                    .to_string(),
            );
        }
        for s in &servers {
            if let Some(rec) = &s.recommendation {
                recommendations.push(rec.clone());
            }
        }

        Ok(McpProfileReport {
            total_servers,
            enabled_servers,
            total_schema_tokens: measured_schema_tokens,
            measured_schema_tokens,
            measured_server_count,
            pct_of_128k_context: TokenCounter::context_percentage(max_upfront_tokens, 128_000),
            est_cost_per_100_turns: TokenCounter::estimate_cost_per_100_turns(max_upfront_tokens),
            clients,
            max_upfront_tokens,
            servers,
            config_files_scanned: scanned_files,
            probed: options.probe,
            untrusted_workspace_servers: untrusted,
            recommendations,
        })
    }

    /// Every configuration file an agent reads MCP servers from.
    fn config_sources(workspace_root: &Path, home: &Path) -> Vec<ConfigSource> {
        let src = |client, path: PathBuf, label: &str, scope, toml| ConfigSource {
            client,
            path,
            label: label.to_string(),
            scope,
            toml,
        };
        vec![
            src(
                "Claude Code",
                home.join(".claude.json"),
                "~/.claude.json",
                "global",
                false,
            ),
            src(
                "Claude Code",
                workspace_root.join(".mcp.json"),
                ".mcp.json",
                "workspace",
                false,
            ),
            src(
                "Claude Desktop",
                home.join("Library/Application Support/Claude/claude_desktop_config.json"),
                "~/Library/Application Support/Claude/claude_desktop_config.json",
                "global",
                false,
            ),
            src(
                "Claude Desktop",
                home.join(".config/Claude/claude_desktop_config.json"),
                "~/.config/Claude/claude_desktop_config.json",
                "global",
                false,
            ),
            src(
                "Cursor",
                home.join(".cursor/mcp.json"),
                "~/.cursor/mcp.json",
                "global",
                false,
            ),
            src(
                "Cursor",
                workspace_root.join(".cursor/mcp.json"),
                ".cursor/mcp.json",
                "workspace",
                false,
            ),
            src(
                "VS Code",
                workspace_root.join(".vscode/mcp.json"),
                ".vscode/mcp.json",
                "workspace",
                false,
            ),
            src(
                "VS Code",
                home.join("Library/Application Support/Code/User/mcp.json"),
                "~/Library/Application Support/Code/User/mcp.json",
                "global",
                false,
            ),
            src(
                "VS Code",
                home.join(".config/Code/User/mcp.json"),
                "~/.config/Code/User/mcp.json",
                "global",
                false,
            ),
            src(
                "OpenCode",
                home.join(".config/opencode/opencode.json"),
                "~/.config/opencode/opencode.json",
                "global",
                false,
            ),
            src(
                "OpenCode",
                home.join(".config/opencode/config.json"),
                "~/.config/opencode/config.json",
                "global",
                false,
            ),
            src(
                "OpenCode",
                workspace_root.join("opencode.json"),
                "opencode.json",
                "workspace",
                false,
            ),
            src(
                "Codex",
                home.join(".codex/config.toml"),
                "~/.codex/config.toml",
                "global",
                true,
            ),
            src(
                "Gemini CLI",
                home.join(".gemini/settings.json"),
                "~/.gemini/settings.json",
                "global",
                false,
            ),
            src(
                "Gemini CLI",
                workspace_root.join(".gemini/settings.json"),
                ".gemini/settings.json",
                "workspace",
                false,
            ),
            src(
                "Windsurf",
                home.join(".codeium/windsurf/mcp_config.json"),
                "~/.codeium/windsurf/mcp_config.json",
                "global",
                false,
            ),
        ]
    }

    /// Reads the server map from any of the shapes agents use: `mcpServers`
    /// (Claude, Cursor, Gemini, Windsurf), `servers` (VS Code), `mcp`
    /// (OpenCode) and `mcp_servers` (Codex).
    fn extract_mcp_servers(
        json: &Value,
        source: &ConfigSource,
        scope: &str,
        servers: &mut Vec<McpServerInfo>,
    ) {
        let Some(obj) = json
            .get("mcpServers")
            .or_else(|| json.get("mcp_servers"))
            .or_else(|| json.get("servers"))
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
            // Later entries for the same agent and name override earlier ones,
            // matching how agents resolve scopes.
            if servers
                .iter()
                .any(|s| s.client == source.client && &s.name == name)
            {
                continue;
            }
            servers.push(Self::parse_entry(name, val, source, scope));
        }
    }

    fn parse_entry(name: &str, val: &Value, source: &ConfigSource, scope: &str) -> McpServerInfo {
        let str_list = |v: Option<&Value>| -> Vec<String> {
            v.and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };

        // `command` is a string (most agents) or a full argv array (OpenCode).
        let (command, mut args) = match val.get("command") {
            Some(Value::String(c)) => (Some(c.clone()), Vec::new()),
            Some(Value::Array(_)) => {
                let argv = str_list(val.get("command"));
                match argv.split_first() {
                    Some((first, rest)) => (Some(first.clone()), rest.to_vec()),
                    None => (None, Vec::new()),
                }
            }
            _ => (None, Vec::new()),
        };
        args.extend(str_list(val.get("args")));

        let url = ["url", "serverUrl", "httpUrl"]
            .iter()
            .find_map(|k| val.get(*k).and_then(|u| u.as_str()));
        let declared_type = val.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        let remote = url.is_some()
            || matches!(
                declared_type,
                "http" | "sse" | "remote" | "streamable-http" | "streamableHttp"
            );

        let raw_env: Vec<(String, String)> = val
            .get("env")
            .or_else(|| val.get("environment"))
            .and_then(|e| e.as_object())
            .map(|e| {
                e.iter()
                    .map(|(k, v)| {
                        let value = v
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| v.to_string());
                        (k.clone(), value)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let enabled = val.get("disabled").and_then(|d| d.as_bool()) != Some(true)
            && val.get("enabled").and_then(|e| e.as_bool()) != Some(false);

        McpServerInfo {
            name: name.to_string(),
            client: source.client.to_string(),
            config_source: source.label.clone(),
            command_or_type: url
                .map(String::from)
                .or(command)
                .unwrap_or_else(|| "stdio".to_string()),
            transport: if remote { "remote" } else { "stdio" }.to_string(),
            scope: scope.to_string(),
            enabled,
            tool_count: None,
            schema_tokens: None,
            tool_name_tokens: None,
            measurement: MeasurementSource::Estimated,
            tool_names: Vec::new(),
            environment_keys: raw_env.iter().map(|(k, _)| k.clone()).collect(),
            probe_error: None,
            raw_args: args,
            raw_env,
            status: String::new(),
            recommendation: None,
        }
    }

    /// `.mcp.json` servers the user switched off in Claude Code settings.
    fn claude_disabled_mcpjson_servers(workspace_root: &Path, home: &Path) -> HashSet<String> {
        let mut out = HashSet::new();
        for path in [
            home.join(".claude/settings.json"),
            workspace_root.join(".claude/settings.json"),
            workspace_root.join(".claude/settings.local.json"),
        ] {
            let Some(json) = fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            if let Some(list) = json
                .get("disabledMcpjsonServers")
                .and_then(|v| v.as_array())
            {
                out.extend(list.iter().filter_map(|v| v.as_str().map(String::from)));
            }
        }
        out
    }

    /// Identifies a server by what would actually be run, so a cached result
    /// is never reused for a different command that happens to share a name.
    fn probe_key(server: &McpServerInfo) -> String {
        let mut key = server.command_or_type.clone();
        for arg in &server.raw_args {
            key.push('\u{1f}');
            key.push_str(arg);
        }
        key
    }

    fn client_loads(servers: &[McpServerInfo], tool_search: ToolSearchMode) -> Vec<ClientLoad> {
        let mut order: Vec<&str> = Vec::new();
        for s in servers {
            if !order.contains(&s.client.as_str()) {
                order.push(&s.client);
            }
        }
        order
            .into_iter()
            .map(|client| {
                let enabled: Vec<&McpServerInfo> = servers
                    .iter()
                    .filter(|s| s.client == client && s.enabled)
                    .collect();
                let measured: Vec<&&McpServerInfo> = enabled
                    .iter()
                    .filter(|s| s.measurement.is_measured())
                    .collect();
                let schema: usize = measured.iter().filter_map(|s| s.schema_tokens).sum();
                let names: usize = measured.iter().filter_map(|s| s.tool_name_tokens).sum();
                let (upfront_tokens, loading) = if client == "Claude Code" {
                    tool_search.load(schema, names)
                } else {
                    (schema, "upfront".to_string())
                };
                ClientLoad {
                    client: client.to_string(),
                    enabled_servers: enabled.len(),
                    measured_servers: measured.len(),
                    measured_schema_tokens: schema,
                    upfront_tokens,
                    loading,
                }
            })
            .collect()
    }

    fn finalize_status(server: &mut McpServerInfo) {
        let tokens = server.schema_tokens;
        server.status = match (tokens, server.measurement) {
            _ if !server.enabled => "⏸ Disabled".to_string(),
            (_, MeasurementSource::ProbeFailed) => "❔ Unreachable".to_string(),
            (None, MeasurementSource::NotProbed) => "⏭ Not probed".to_string(),
            (None, _) => "❔ Not measured".to_string(),
            (Some(t), m) if t > 3_000 => format!("🚨 Heavy ({})", m.label()),
            (Some(t), m) if t > 1_500 => format!("⚠️ Moderate ({})", m.label()),
            (Some(_), m) => format!("✅ Lean ({})", m.label()),
        };

        server.recommendation = match tokens {
            Some(t) if t > 3_000 && server.enabled && server.measurement.is_measured() => {
                Some(format!(
                    "Server '{}' ({}) defines {} tokens across {} tools. Scope it to the sessions that need it.",
                    server.name,
                    server.client,
                    t,
                    server.tool_count.unwrap_or(0)
                ))
            }
            _ => None,
        };
    }

    fn cache_path(home: &Path) -> PathBuf {
        home.join(".cache/agentprof/mcp-probe-cache.json")
    }

    fn load_cache(path: &Path) -> ProbeCache {
        fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str::<ProbeCache>(&c).ok())
            // Earlier caches were keyed by server name only; drop them.
            .filter(|c| c.version == CACHE_VERSION)
            .unwrap_or_default()
    }

    fn save_cache(path: &Path, cache: &ProbeCache) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(cache)?)?;
        Ok(())
    }

    /// Probes servers concurrently, a bounded number at a time.
    fn probe_all(
        targets: &[(String, McpServerInfo)],
        cwd: &Path,
        timeout: Duration,
    ) -> HashMap<String, Result<ProbeResult, String>> {
        let mut results = HashMap::new();
        for chunk in targets.chunks(PROBE_CONCURRENCY) {
            thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(key, server)| {
                        scope.spawn(move || {
                            let outcome =
                                Self::probe_server(server, cwd, timeout).map_err(|e| e.to_string());
                            (key.clone(), outcome)
                        })
                    })
                    .collect();
                for handle in handles {
                    if let Ok((key, outcome)) = handle.join() {
                        results.insert(key, outcome);
                    }
                }
            });
        }
        results
    }

    /// Performs a real MCP stdio handshake (`initialize` -> `tools/list`,
    /// following pagination) and counts the tokens the tool definitions occupy.
    fn probe_server(server: &McpServerInfo, cwd: &Path, timeout: Duration) -> Result<ProbeResult> {
        let command = &server.command_or_type;
        if server.transport != "stdio"
            || command.starts_with("http://")
            || command.starts_with("https://")
        {
            anyhow::bail!("remote (HTTP/SSE) servers are not probed over stdio");
        }
        if command == "stdio" {
            anyhow::bail!("config entry has no executable command");
        }

        let mut cmd = Command::new(command);
        cmd.args(&server.raw_args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &server.raw_env {
            cmd.env(key, expand_env(value));
        }
        // Launchers such as `npx` start the real server as a child. A process
        // group lets the whole tree be stopped, not just the launcher.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not start '{}': {}", command, e))?;

        // Drain stderr from the start: a chatty server would otherwise fill the
        // pipe and block before it ever answers.
        let stderr_buf = Self::drain_stderr(&mut child);
        let result = Self::handshake(&mut child, timeout);
        Self::stop(&mut child);
        result.map_err(|e| match Self::last_line(&stderr_buf) {
            Some(tail) => anyhow::anyhow!("{} (stderr: {})", e, tail),
            None => e,
        })
    }

    fn handshake(child: &mut Child, timeout: Duration) -> Result<ProbeResult> {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout pipe"))?;

        // Pipe reads cannot be given a deadline directly, so responses are read
        // on a worker thread and the deadline is enforced here.
        let (tx, rx) = mpsc::channel::<Value>();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value.get("id").is_some()
                    && tx.send(value).is_err()
                {
                    return;
                }
            }
        });

        let init = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "agentprof", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let mut request_id = 2u64;
        let list =
            json!({"jsonrpc": "2.0", "id": request_id, "method": "tools/list", "params": {}});
        writeln!(stdin, "{}", init)?;
        writeln!(stdin, "{}", initialized)?;
        writeln!(stdin, "{}", list)?;
        stdin.flush()?;

        let deadline = Instant::now() + timeout;
        let mut tools: Vec<Value> = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = rx.recv_timeout(remaining).map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => {
                    anyhow::anyhow!("no tools/list response within {}s", timeout.as_secs())
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    anyhow::anyhow!("server exited before answering tools/list")
                }
            })?;
            let id = message.get("id").and_then(|i| i.as_u64());
            if let Some(error) = message.get("error") {
                let what = if id == Some(1) {
                    "initialize"
                } else {
                    "tools/list"
                };
                anyhow::bail!("{} failed: {}", what, error);
            }
            if id != Some(request_id) {
                continue;
            }
            let result = message
                .get("result")
                .ok_or_else(|| anyhow::anyhow!("tools/list response has no result"))?;
            let page = result
                .get("tools")
                .and_then(|t| t.as_array())
                .ok_or_else(|| anyhow::anyhow!("server returned no tool list"))?;
            tools.extend(page.iter().cloned());

            match result.get("nextCursor").and_then(|c| c.as_str()) {
                Some(cursor) if !cursor.is_empty() => {
                    request_id += 1;
                    let next = json!({
                        "jsonrpc": "2.0", "id": request_id, "method": "tools/list",
                        "params": {"cursor": cursor}
                    });
                    writeln!(stdin, "{}", next)?;
                    stdin.flush()?;
                }
                _ => break,
            }
        }

        let tool_names: Vec<String> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        // Agents serialize the whole tool array into the prompt, so the token
        // cost is the cost of that serialization.
        let schema_tokens = TokenCounter::count_cl100k(&serde_json::to_string(&tools)?);
        let tool_name_tokens = TokenCounter::count_cl100k(&tool_names.join("\n"));

        Ok(ProbeResult {
            tool_names,
            schema_tokens,
            tool_name_tokens,
        })
    }

    /// Collects the most recent stderr output (bounded) on a worker thread.
    fn drain_stderr(child: &mut Child) -> Arc<Mutex<Vec<u8>>> {
        let buf = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let sink = Arc::clone(&buf);
            thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut b = sink.lock().unwrap_or_else(|p| p.into_inner());
                    b.extend_from_slice(&chunk[..n]);
                    if b.len() > 16_384 {
                        let excess = b.len() - 16_384;
                        b.drain(..excess);
                    }
                }
            });
        }
        buf
    }

    /// The last non-empty stderr line, which usually explains a failure (a
    /// missing token, say).
    fn last_line(buf: &Arc<Mutex<Vec<u8>>>) -> Option<String> {
        // Give the reader a moment to drain what the process wrote.
        thread::sleep(Duration::from_millis(50));
        let bytes = buf.lock().ok()?.clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(200).collect())
    }

    /// Stops the server and every process it started.
    fn stop(child: &mut Child) {
        #[cfg(unix)]
        {
            // SAFETY: kill(2) with a negative pid signals the process group
            // created for this child; it has no memory-safety preconditions.
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Claude Code's tool-search setting, which decides whether MCP tool
/// definitions are loaded up front or on demand.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ToolSearchMode {
    /// Default: definitions are deferred; only tool names load up front.
    Deferred,
    /// `ENABLE_TOOL_SEARCH=false`.
    Upfront,
    /// `auto` / `auto:N`: up front until definitions reach N% of the window.
    Auto(f64),
}

impl ToolSearchMode {
    /// Reads `ENABLE_TOOL_SEARCH` from the environment, then from the `env`
    /// blocks of Claude Code settings (local wins over project over user).
    fn detect(workspace_root: &Path, home: &Path) -> Self {
        let from_settings = || {
            [
                workspace_root.join(".claude/settings.local.json"),
                workspace_root.join(".claude/settings.json"),
                home.join(".claude/settings.json"),
            ]
            .iter()
            .find_map(|p| {
                let json: Value = serde_json::from_str(&fs::read_to_string(p).ok()?).ok()?;
                json.get("env")?
                    .get("ENABLE_TOOL_SEARCH")?
                    .as_str()
                    .map(String::from)
            })
        };
        let value = std::env::var("ENABLE_TOOL_SEARCH")
            .ok()
            .or_else(from_settings);
        Self::parse(value.as_deref())
    }

    fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()) {
            None => ToolSearchMode::Deferred,
            Some(v) if v == "false" || v == "0" => ToolSearchMode::Upfront,
            Some(v) if v == "auto" => ToolSearchMode::Auto(10.0),
            Some(v) if v.starts_with("auto:") => {
                ToolSearchMode::Auto(v[5..].parse().unwrap_or(10.0))
            }
            Some(_) => ToolSearchMode::Deferred,
        }
    }

    /// Tokens loaded every turn, and a description of why.
    fn load(self, schema_tokens: usize, name_tokens: usize) -> (usize, String) {
        match self {
            ToolSearchMode::Deferred => (
                name_tokens,
                "deferred (tool search): only tool names load up front".to_string(),
            ),
            ToolSearchMode::Upfront => (
                schema_tokens,
                "upfront (ENABLE_TOOL_SEARCH=false)".to_string(),
            ),
            ToolSearchMode::Auto(pct) => {
                let threshold = (CLAUDE_CODE_CONTEXT_WINDOW as f64 * pct / 100.0) as usize;
                if schema_tokens >= threshold {
                    (
                        name_tokens,
                        format!(
                            "deferred (tool search auto: definitions ≥ {}% of context)",
                            pct
                        ),
                    )
                } else {
                    (
                        schema_tokens,
                        format!(
                            "upfront (tool search auto: definitions < {}% of context)",
                            pct
                        ),
                    )
                }
            }
        }
    }
}

/// Expands `${VAR}`, `${VAR:-default}` and `${env:VAR}` from the environment,
/// as agents do for MCP server configs.
fn expand_env(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let expr = &after[..end];
        let expr = expr.strip_prefix("env:").unwrap_or(expr);
        let (name, default) = match expr.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (expr, None),
        };
        match std::env::var(name) {
            Ok(v) if !v.is_empty() => out.push_str(&v),
            _ => out.push_str(default.unwrap_or("")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(client: &'static str, scope: &'static str) -> ConfigSource {
        ConfigSource {
            client,
            path: PathBuf::from("test.json"),
            label: "test.json".to_string(),
            scope,
            toml: false,
        }
    }

    fn extract(json: Value, client: &'static str) -> Vec<McpServerInfo> {
        let mut out = Vec::new();
        McpProfiler::extract_mcp_servers(&json, &source(client, "global"), "global", &mut out);
        out
    }

    fn by_name<'a>(servers: &'a [McpServerInfo], name: &str) -> &'a McpServerInfo {
        servers.iter().find(|s| s.name == name).unwrap()
    }

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentprof_mcp_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_extracts_servers_from_mcp_servers_key() {
        let servers = extract(
            json!({
                "mcpServers": {
                    "alpha": {"command": "/bin/alpha", "args": ["--stdio"], "env": {"TOKEN": "x"}},
                    "beta": {"url": "https://example.test/mcp"}
                }
            }),
            "Claude Code",
        );
        assert_eq!(servers.len(), 2);
        let alpha = by_name(&servers, "alpha");
        assert_eq!(alpha.command_or_type, "/bin/alpha");
        assert_eq!(alpha.raw_args, vec!["--stdio".to_string()]);
        assert_eq!(alpha.raw_env, vec![("TOKEN".to_string(), "x".to_string())]);
        assert_eq!(alpha.transport, "stdio");
        let beta = by_name(&servers, "beta");
        assert_eq!(beta.command_or_type, "https://example.test/mcp");
        assert_eq!(beta.transport, "remote");
    }

    /// Regression: VS Code keeps servers under `servers`, so .vscode/mcp.json
    /// was scanned but never yielded a server.
    #[test]
    fn test_vscode_servers_key() {
        let servers = extract(
            json!({"servers": {"github": {"type": "stdio", "command": "npx", "args": ["-y", "gh-mcp"]}}, "inputs": []}),
            "VS Code",
        );
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].command_or_type, "npx");
    }

    /// Regression: OpenCode's `command` is a full argv array, so servers were
    /// reported as running "local" and disabled ones were counted.
    #[test]
    fn test_opencode_command_arrays_environment_and_enabled() {
        let servers = extract(
            json!({"mcp": {
                "fs": {"type": "local", "command": ["npx", "-y", "server-fs", "."], "environment": {"A": "1"}, "enabled": true},
                "off": {"type": "local", "command": ["npx", "x"], "enabled": false},
                "web": {"type": "remote", "url": "https://mcp.example.test"}
            }}),
            "OpenCode",
        );
        let fs_server = by_name(&servers, "fs");
        assert_eq!(fs_server.command_or_type, "npx");
        assert_eq!(fs_server.raw_args, vec!["-y", "server-fs", "."]);
        assert_eq!(fs_server.environment_keys, vec!["A".to_string()]);
        assert!(!by_name(&servers, "off").enabled);
        assert_eq!(by_name(&servers, "web").transport, "remote");
    }

    #[test]
    fn test_codex_toml_config() {
        let toml_text = "[mcp_servers.docs]\ncommand = \"docs-mcp\"\nargs = [\"--stdio\"]\n\n[mcp_servers.docs.env]\nKEY = \"v\"\n";
        let json: Value = toml::from_str(toml_text).unwrap();
        let servers = extract(json, "Codex");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].command_or_type, "docs-mcp");
        assert_eq!(servers[0].raw_args, vec!["--stdio".to_string()]);
        assert_eq!(servers[0].environment_keys, vec!["KEY".to_string()]);
    }

    #[test]
    fn test_windsurf_disabled_and_server_url() {
        let servers = extract(
            json!({"mcpServers": {
                "a": {"command": "a", "disabled": true},
                "b": {"serverUrl": "https://b.example.test/mcp"}
            }}),
            "Windsurf",
        );
        assert!(!by_name(&servers, "a").enabled);
        assert_eq!(by_name(&servers, "b").transport, "remote");
    }

    #[test]
    fn test_no_servers_yields_nothing() {
        assert!(extract(json!({"unrelated": true}), "Cursor").is_empty());
    }

    #[test]
    fn test_unmeasured_server_reports_no_token_figure() {
        let mut server =
            extract(json!({"mcpServers": {"x": {"command": "cmd"}}}), "Cursor").remove(0);
        McpProfiler::finalize_status(&mut server);
        assert_eq!(server.status, "❔ Not measured");
        assert!(server.schema_tokens.is_none());
        assert!(server.recommendation.is_none());
    }

    #[test]
    fn test_remote_server_is_not_probed_over_stdio() {
        let server = extract(
            json!({"mcpServers": {"r": {"url": "https://example.test/mcp"}}}),
            "Cursor",
        )
        .remove(0);
        assert!(
            McpProfiler::probe_server(&server, Path::new("."), Duration::from_millis(200)).is_err()
        );
    }

    /// Workspace configs come from the repository; probing them executes its
    /// commands, so it needs an explicit opt-in.
    #[test]
    fn test_workspace_servers_are_not_started_without_trust() {
        let dir = tempdir("trust");
        let marker = dir.join("started");
        fs::write(
            dir.join(".mcp.json"),
            json!({"mcpServers": {"evil": {"command": "touch", "args": [marker.to_str().unwrap()]}}}).to_string(),
        )
        .unwrap();
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();

        let options = ProbeOptions {
            probe: true,
            trust_workspace: false,
            timeout: Duration::from_secs(2),
        };
        let report = McpProfiler::profile_in(&dir, &home, options).unwrap();
        assert!(
            !marker.exists(),
            "an untrusted workspace command was executed"
        );
        assert_eq!(report.untrusted_workspace_servers, 1);
        assert_eq!(report.servers[0].measurement, MeasurementSource::NotProbed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_probe_speaks_mcp_follows_pagination_and_passes_env() {
        let dir = tempdir("probe");
        // A tiny MCP server: answers initialize, then two pages of tools. The
        // second page's tool name comes from an env var set in the config.
        let script = dir.join("server.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"t","version":"1"}}}' ;;
    *'"cursor"'*) echo '{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"'"$TOOL_NAME"'","inputSchema":{"type":"object"}}]}}' ;;
    *'tools/list'*) echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"first","description":"d","inputSchema":{"type":"object"}}],"nextCursor":"p2"}}' ;;
  esac
done
"#,
        )
        .unwrap();
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".claude.json"),
            json!({"mcpServers": {"paged": {"command": "sh", "args": [script.to_str().unwrap()], "env": {"TOOL_NAME": "second"}}}}).to_string(),
        )
        .unwrap();

        let options = ProbeOptions {
            probe: true,
            trust_workspace: false,
            timeout: Duration::from_secs(5),
        };
        let report = McpProfiler::profile_in(&dir, &home, options).unwrap();
        let server = &report.servers[0];
        assert_eq!(
            server.measurement,
            MeasurementSource::Probed,
            "{:?}",
            server.probe_error
        );
        assert_eq!(
            server.tool_names,
            vec!["first".to_string(), "second".to_string()]
        );
        assert!(server.schema_tokens.unwrap() > server.tool_name_tokens.unwrap());

        // A later non-probing run reuses the cached measurement.
        let cached = McpProfiler::profile_in(&dir, &home, ProbeOptions::default()).unwrap();
        assert_eq!(cached.servers[0].measurement, MeasurementSource::Cached);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tool_search_modes() {
        assert_eq!(ToolSearchMode::parse(None), ToolSearchMode::Deferred);
        assert_eq!(
            ToolSearchMode::parse(Some("false")),
            ToolSearchMode::Upfront
        );
        assert_eq!(
            ToolSearchMode::parse(Some("auto:5")),
            ToolSearchMode::Auto(5.0)
        );

        assert_eq!(ToolSearchMode::Deferred.load(30_000, 200).0, 200);
        assert_eq!(ToolSearchMode::Upfront.load(30_000, 200).0, 30_000);
        // auto (10% of 200k = 20k): small sets load up front, large ones defer.
        assert_eq!(ToolSearchMode::Auto(10.0).load(5_000, 50).0, 5_000);
        assert_eq!(ToolSearchMode::Auto(10.0).load(25_000, 250).0, 250);
    }

    #[test]
    fn test_expand_env() {
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("AGENTPROF_TEST_TOKEN", "abc") };
        assert_eq!(expand_env("Bearer ${AGENTPROF_TEST_TOKEN}"), "Bearer abc");
        assert_eq!(expand_env("${env:AGENTPROF_TEST_TOKEN}"), "abc");
        assert_eq!(expand_env("${AGENTPROF_UNSET_VAR:-fallback}"), "fallback");
        assert_eq!(expand_env("plain"), "plain");
    }
}
