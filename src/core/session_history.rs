use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::pricing::{self, CACHE_WRITE_1H_MULTIPLIER, CACHE_WRITE_5M_MULTIPLIER};

/// How many transcripts to read by default.
///
/// Transcript directories routinely reach multiple GB; reading every one on a
/// bare `agentprof history` would take minutes. The scan is bounded to the most
/// recently modified sessions and the report states its own coverage.
pub const DEFAULT_SESSION_LIMIT: usize = 50;

pub const PRICING_LABEL: &str =
    "API-equivalent cost at Anthropic list prices per model; subscription plans bill differently";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    /// All cache writes, whatever their TTL.
    pub cache_creation_tokens: usize,
    /// The part of `cache_creation_tokens` written with the 1-hour TTL, which
    /// bills at 2x input instead of 1.25x.
    #[serde(default)]
    pub cache_creation_1h_tokens: usize,
    pub cache_read_tokens: usize,
}

impl TokenUsage {
    fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_creation_1h_tokens += other.cache_creation_1h_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }

    /// Field-wise maximum. Streaming writes several entries for one API
    /// request with accumulating counts, so the largest value is the final one.
    fn max_merge(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.cache_creation_tokens = self.cache_creation_tokens.max(other.cache_creation_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .max(other.cache_creation_1h_tokens);
        self.cache_read_tokens = self.cache_read_tokens.max(other.cache_read_tokens);
    }

    pub fn total(&self) -> usize {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    /// Billable cost at the given list price, cache-aware.
    pub fn cost_usd(&self, price: &pricing::ModelPrice) -> f64 {
        let per = |tokens: usize| tokens as f64 / 1_000_000.0;
        let cache_1h = self
            .cache_creation_1h_tokens
            .min(self.cache_creation_tokens);
        let cache_5m = self.cache_creation_tokens - cache_1h;
        per(self.input_tokens) * price.input
            + per(cache_5m) * price.input * CACHE_WRITE_5M_MULTIPLIER
            + per(cache_1h) * price.input * CACHE_WRITE_1H_MULTIPLIER
            + per(self.cache_read_tokens) * price.input * price.cache_read_multiplier
            + per(self.output_tokens) * price.output
    }
}

/// One API request, as reconstructed from possibly several transcript lines.
#[derive(Debug, Clone, Default)]
struct RequestUsage {
    model: String,
    fast: bool,
    usage: TokenUsage,
}

impl RequestUsage {
    fn merge(&mut self, other: &RequestUsage) {
        if self.model.is_empty() {
            self.model = other.model.clone();
        }
        self.fast |= other.fast;
        self.usage.max_merge(&other.usage);
    }

    /// `None` when the model is not in the price table.
    fn cost(&self) -> Option<f64> {
        pricing::price_for_request(&self.model, self.fast).map(|p| self.usage.cost_usd(&p))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    pub requests: usize,
    pub usage: TokenUsage,
    /// `None` when the model is not in the price table.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub project: String,
    pub last_modified: String,
    pub turn_count: usize,
    pub api_requests: usize,
    pub usage: TokenUsage,
    pub estimated_cost_usd: f64,
    pub models: Vec<String>,
    pub dominant_tool: Option<String>,
    pub repeated_tool_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionHistoryReport {
    /// Transcripts found on disk.
    pub total_sessions_found: usize,
    /// Transcripts actually parsed (bounded by the session limit).
    pub sessions_analyzed: usize,
    /// True when `sessions_analyzed < total_sessions_found`.
    pub truncated: bool,
    pub total_turns: usize,
    /// Distinct API requests, after de-duplication.
    pub api_requests: usize,
    /// Transcript entries skipped because they repeat a request already
    /// counted (streaming writes one entry per content block, and resumed
    /// sessions copy earlier entries).
    pub duplicate_entries_merged: usize,
    pub usage: TokenUsage,
    pub total_tokens_used: usize,
    pub total_estimated_cost_usd: f64,
    pub pricing_label: String,
    pub cost_by_model: Vec<ModelUsage>,
    /// Models whose tokens are counted but not priced.
    pub unpriced_models: Vec<String>,
    pub tool_usage_distribution: Vec<(String, usize)>,
    pub recent_sessions: Vec<SessionSummary>,
    pub loop_thrash_incidents: usize,
    pub prompt_history_entries: usize,
    pub sources_scanned: Vec<String>,
    pub recommendations: Vec<String>,
}

/// Requests keyed by (message id, request id).
type RequestKey = (String, String);

struct ParsedSession {
    summary: SessionSummary,
    keyed: HashMap<RequestKey, RequestUsage>,
    unkeyed: Vec<RequestUsage>,
    duplicate_entries: usize,
}

pub struct SessionHistoryAnalyzer;

impl SessionHistoryAnalyzer {
    pub fn analyze() -> Result<SessionHistoryReport> {
        Self::analyze_with_limit(DEFAULT_SESSION_LIMIT)
    }

    pub fn analyze_with_limit(limit: usize) -> Result<SessionHistoryReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        Self::analyze_home(&home, limit)
    }

    pub(crate) fn analyze_home(home: &Path, limit: usize) -> Result<SessionHistoryReport> {
        let mut sources_scanned = Vec::new();

        // Claude Code writes one JSONL transcript per session under
        // ~/.claude/projects/<slugified-cwd>/<session-uuid>.jsonl.
        let projects_dir = home.join(".claude/projects");
        let mut transcripts = Self::collect_transcripts(&projects_dir);
        if !transcripts.is_empty() {
            sources_scanned.push("~/.claude/projects/**/*.jsonl".to_string());
        }

        let total_sessions_found = transcripts.len();
        // Newest first, so a bounded scan covers the most relevant sessions.
        transcripts.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
        transcripts.truncate(limit);
        let sessions_analyzed = transcripts.len();

        let mut tool_counts: HashMap<String, usize> = HashMap::new();
        let mut sessions = Vec::new();
        let mut total_turns = 0usize;
        let mut loop_thrash_incidents = 0usize;
        let mut duplicate_entries_merged = 0usize;

        // A resumed session copies earlier requests into its own file, so the
        // same request can appear in several transcripts. Totals count it once.
        let mut all_keyed: HashMap<RequestKey, RequestUsage> = HashMap::new();
        let mut all_unkeyed: Vec<RequestUsage> = Vec::new();

        for (path, modified) in &transcripts {
            let Some(parsed) = Self::parse_transcript(path, *modified, &mut tool_counts) else {
                continue;
            };
            total_turns += parsed.summary.turn_count;
            if parsed.summary.repeated_tool_calls > 0 {
                loop_thrash_incidents += 1;
            }
            duplicate_entries_merged += parsed.duplicate_entries;
            for (key, request) in parsed.keyed {
                match all_keyed.get_mut(&key) {
                    Some(existing) => {
                        existing.merge(&request);
                        duplicate_entries_merged += 1;
                    }
                    None => {
                        all_keyed.insert(key, request);
                    }
                }
            }
            all_unkeyed.extend(parsed.unkeyed);
            sessions.push(parsed.summary);
        }

        // ~/.claude/history.jsonl is the typed-prompt history, not a turn log.
        // It is reported as its own metric rather than conflated with turns.
        let mut prompt_history_entries = 0usize;
        let history_file = home.join(".claude/history.jsonl");
        if let Ok(content) = fs::read_to_string(&history_file) {
            prompt_history_entries = content.lines().filter(|l| !l.trim().is_empty()).count();
            sources_scanned.push("~/.claude/history.jsonl".to_string());
        }

        sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

        let mut tool_usage_distribution: Vec<(String, usize)> = tool_counts.into_iter().collect();
        tool_usage_distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let requests: Vec<&RequestUsage> = all_keyed.values().chain(all_unkeyed.iter()).collect();
        let mut usage = TokenUsage::default();
        let mut by_model: HashMap<String, ModelUsage> = HashMap::new();
        for request in &requests {
            usage.add(&request.usage);
            let model = if request.model.is_empty() {
                "unknown".to_string()
            } else {
                request.model.clone()
            };
            let entry = by_model.entry(model.clone()).or_insert_with(|| ModelUsage {
                model,
                requests: 0,
                usage: TokenUsage::default(),
                cost_usd: Some(0.0),
            });
            entry.requests += 1;
            entry.usage.add(&request.usage);
            entry.cost_usd = match (entry.cost_usd, request.cost()) {
                (Some(total), Some(cost)) => Some(total + cost),
                _ => None,
            };
        }
        let mut cost_by_model: Vec<ModelUsage> = by_model.into_values().collect();
        cost_by_model.sort_by(|a, b| {
            b.cost_usd
                .unwrap_or(0.0)
                .total_cmp(&a.cost_usd.unwrap_or(0.0))
                .then_with(|| a.model.cmp(&b.model))
        });
        let unpriced_models: Vec<String> = cost_by_model
            .iter()
            .filter(|m| m.cost_usd.is_none() && m.usage.total() > 0)
            .map(|m| m.model.clone())
            .collect();
        let total_estimated_cost_usd: f64 = cost_by_model.iter().filter_map(|m| m.cost_usd).sum();
        let total_tokens_used = usage.total();

        let mut recommendations = Vec::new();
        if loop_thrash_incidents > 0 {
            recommendations.push(format!(
                "{} session(s) repeated an identical tool call back-to-back. Narrow file paths or cache results to break retry loops.",
                loop_thrash_incidents
            ));
        }
        if usage.cache_read_tokens > 0 {
            let cached_share = usage.cache_read_tokens as f64
                / (usage.cache_read_tokens + usage.cache_creation_tokens + usage.input_tokens)
                    .max(1) as f64
                * 100.0;
            if cached_share < 50.0 {
                recommendations.push(format!(
                    "Only {:.0}% of input tokens were served from prompt cache. Stable instruction files improve cache hit rate.",
                    cached_share
                ));
            }
        }
        if !unpriced_models.is_empty() {
            recommendations.push(format!(
                "No list price for {}; their tokens are counted but excluded from the cost.",
                unpriced_models.join(", ")
            ));
        }
        if total_sessions_found > sessions_analyzed {
            recommendations.push(format!(
                "Analyzed the {} most recent of {} transcripts. Use `--sessions <n>` (or `--sessions 0` for all) to widen the scan.",
                sessions_analyzed, total_sessions_found
            ));
        }

        Ok(SessionHistoryReport {
            total_sessions_found,
            sessions_analyzed,
            truncated: sessions_analyzed < total_sessions_found,
            total_turns,
            api_requests: requests.len(),
            duplicate_entries_merged,
            usage,
            total_tokens_used,
            total_estimated_cost_usd,
            pricing_label: PRICING_LABEL.to_string(),
            cost_by_model,
            unpriced_models,
            tool_usage_distribution: tool_usage_distribution.into_iter().take(12).collect(),
            recent_sessions: sessions.into_iter().take(5).collect(),
            loop_thrash_incidents,
            prompt_history_entries,
            sources_scanned,
            recommendations,
        })
    }

    fn collect_transcripts(projects_dir: &Path) -> Vec<(PathBuf, SystemTime)> {
        let mut out = Vec::new();
        if !projects_dir.is_dir() {
            return out;
        }
        // Transcripts nest deeper than one level for worktrees and sub-agents,
        // so this walks the tree rather than reading a single directory level.
        for entry in WalkDir::new(projects_dir)
            .follow_links(false)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((path.to_path_buf(), modified));
        }
        out
    }

    fn parse_transcript(
        path: &Path,
        modified: SystemTime,
        global_tools: &mut HashMap<String, usize>,
    ) -> Option<ParsedSession> {
        let file = File::open(path).ok()?;
        let reader = BufReader::with_capacity(256 * 1024, file);

        let mut keyed: HashMap<RequestKey, RequestUsage> = HashMap::new();
        let mut unkeyed: Vec<RequestUsage> = Vec::new();
        let mut duplicate_entries = 0usize;
        let mut turn_count = 0usize;
        let mut session_tools: HashMap<String, usize> = HashMap::new();
        let mut seen_tool_use_ids: HashSet<String> = HashSet::new();
        let mut models: Vec<String> = Vec::new();
        let mut repeated_tool_calls = 0usize;
        let mut last_call: Option<(String, String)> = None;

        for line in reader.lines().map_while(Result::ok) {
            // Transcript lines are dominated by large tool-result payloads. A
            // substring prefilter avoids paying full JSON parsing on those.
            let interesting = line.contains("\"usage\"")
                || line.contains("\"tool_use\"")
                || line.contains("\"type\":\"user\"");
            if !interesting {
                continue;
            }

            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };

            let Some(message) = value.get("message") else {
                continue;
            };

            // Tool results are also recorded with `type: "user"`, so counting
            // every user line inflates turns by the number of tool calls. A real
            // human turn carries string content or a content array with no
            // tool_result block.
            if value.get("type").and_then(|t| t.as_str()) == Some("user")
                && value.get("isSidechain").and_then(|s| s.as_bool()) != Some(true)
                && Self::is_human_turn(message)
            {
                turn_count += 1;
            }

            let model = message
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or_default();
            if !model.is_empty() && model != "<synthetic>" && !models.iter().any(|m| m == model) {
                models.push(model.to_string());
            }

            if let Some(u) = message.get("usage") {
                let request = RequestUsage {
                    model: model.to_string(),
                    fast: u.get("speed").and_then(|s| s.as_str()) == Some("fast"),
                    usage: Self::parse_usage(u),
                };
                if request.usage.total() > 0 {
                    // Claude Code writes one transcript line per content block
                    // (thinking, text, tool_use), each repeating the full usage
                    // of the same API request. Summing every line counted a
                    // single request two or three times.
                    let message_id = message.get("id").and_then(|i| i.as_str());
                    let request_id = value.get("requestId").and_then(|i| i.as_str());
                    match (message_id, request_id) {
                        (None, None) => unkeyed.push(request),
                        (m, r) => {
                            let key = (m.unwrap_or("").to_string(), r.unwrap_or("").to_string());
                            match keyed.get_mut(&key) {
                                Some(existing) => {
                                    existing.merge(&request);
                                    duplicate_entries += 1;
                                }
                                None => {
                                    keyed.insert(key, request);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(blocks) = message.get("content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    let Some(name) = block.get("name").and_then(|n| n.as_str()) else {
                        continue;
                    };
                    // A block copied into the transcript twice is one call.
                    if let Some(id) = block.get("id").and_then(|i| i.as_str())
                        && !seen_tool_use_ids.insert(id.to_string())
                    {
                        continue;
                    }
                    *session_tools.entry(name.to_string()).or_default() += 1;
                    *global_tools.entry(name.to_string()).or_default() += 1;

                    // Loop thrash: the same tool invoked with byte-identical
                    // input twice in a row is a retry that made no progress.
                    let input = block
                        .get("input")
                        .map(|i| i.to_string())
                        .unwrap_or_default();
                    let call = (name.to_string(), input);
                    if last_call.as_ref() == Some(&call) {
                        repeated_tool_calls += 1;
                    }
                    last_call = Some(call);
                }
            }
        }

        // A transcript with no assistant turns carries no signal worth reporting.
        if keyed.is_empty() && unkeyed.is_empty() && session_tools.is_empty() {
            return None;
        }

        let mut usage = TokenUsage::default();
        let mut cost = 0.0;
        for request in keyed.values().chain(unkeyed.iter()) {
            usage.add(&request.usage);
            cost += request.cost().unwrap_or(0.0);
        }

        let dominant_tool = session_tools
            .iter()
            .max_by_key(|(name, count)| (**count, std::cmp::Reverse(name.as_str())))
            .map(|(name, _)| name.clone());

        let summary = SessionSummary {
            session_id: path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            project: path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| Self::unslug_project(&n.to_string_lossy()))
                .unwrap_or_else(|| "unknown".to_string()),
            last_modified: Self::format_age(modified),
            turn_count,
            api_requests: keyed.len() + unkeyed.len(),
            usage,
            estimated_cost_usd: cost,
            models,
            dominant_tool,
            repeated_tool_calls,
        };

        Some(ParsedSession {
            summary,
            keyed,
            unkeyed,
            duplicate_entries,
        })
    }

    fn parse_usage(u: &serde_json::Value) -> TokenUsage {
        let cache_creation_1h = u
            .get("cache_creation")
            .map(|c| Self::field(c, "ephemeral_1h_input_tokens"))
            .unwrap_or(0);
        TokenUsage {
            input_tokens: Self::field(u, "input_tokens"),
            output_tokens: Self::field(u, "output_tokens"),
            cache_creation_tokens: Self::field(u, "cache_creation_input_tokens"),
            cache_creation_1h_tokens: cache_creation_1h,
            cache_read_tokens: Self::field(u, "cache_read_input_tokens"),
        }
    }

    fn is_human_turn(message: &serde_json::Value) -> bool {
        match message.get("content") {
            Some(serde_json::Value::String(_)) => true,
            Some(serde_json::Value::Array(blocks)) => !blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")),
            _ => false,
        }
    }

    fn field(value: &serde_json::Value, key: &str) -> usize {
        value.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as usize
    }

    /// `-Users-rd-Documents-GitHub-foo` -> `foo`
    fn unslug_project(slug: &str) -> String {
        slug.rsplit('-')
            .find(|segment| !segment.is_empty())
            .unwrap_or(slug)
            .to_string()
    }

    fn format_age(modified: SystemTime) -> String {
        let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
            return "just now".to_string();
        };
        let secs = elapsed.as_secs();
        match secs {
            0..=3599 => format!("{}m ago", (secs / 60).max(1)),
            3600..=86_399 => format!("{}h ago", secs / 3600),
            _ => format!("{}d ago", secs / 86_400),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentprof_hist_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude/projects/-tmp-demo")).unwrap();
        dir
    }

    fn assistant_line(msg_id: &str, req_id: &str, model: &str, block: &str, usage: &str) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{req_id}","message":{{"id":"{msg_id}","model":"{model}","content":[{block}],"usage":{usage}}}}}"#
        )
    }

    #[test]
    fn test_usage_cost_is_cache_aware() {
        let price = pricing::price_for("claude-sonnet-4-6").unwrap();
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            ..Default::default()
        };
        assert!((usage.cost_usd(&price) - 3.0).abs() < 1e-9);

        let cached = TokenUsage {
            cache_read_tokens: 1_000_000,
            ..Default::default()
        };
        assert!((cached.cost_usd(&price) - 0.3).abs() < 1e-9);

        // 1-hour cache writes bill at 2x input, 5-minute writes at 1.25x.
        let writes = TokenUsage {
            cache_creation_tokens: 2_000_000,
            cache_creation_1h_tokens: 1_000_000,
            ..Default::default()
        };
        assert!((writes.cost_usd(&price) - (3.75 + 6.0)).abs() < 1e-9);
    }

    #[test]
    fn test_usage_total_sums_all_buckets() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_tokens: 4,
            cache_creation_1h_tokens: 0,
            cache_read_tokens: 8,
        };
        assert_eq!(usage.total(), 15);
    }

    /// Regression: one API request streamed as a text block and a tool_use
    /// block was counted twice.
    #[test]
    fn test_request_split_across_lines_is_counted_once() {
        let home = tempdir("dedupe");
        let usage = r#"{"input_tokens":1000,"output_tokens":500,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}"#;
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.to_string(),
            assistant_line(
                "msg_1",
                "req_1",
                "claude-opus-4-5",
                r#"{"type":"text","text":"ok"}"#,
                usage,
            ),
            assistant_line(
                "msg_1",
                "req_1",
                "claude-opus-4-5",
                r#"{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}"#,
                usage,
            ),
        ];
        fs::write(
            home.join(".claude/projects/-tmp-demo/s1.jsonl"),
            lines.join("\n"),
        )
        .unwrap();

        let report = SessionHistoryAnalyzer::analyze_home(&home, 10).unwrap();
        assert_eq!(report.api_requests, 1);
        assert_eq!(report.duplicate_entries_merged, 1);
        assert_eq!(report.usage.input_tokens, 1000);
        assert_eq!(report.usage.output_tokens, 500);
        // Priced at Opus 4.5 rates ($5 / $25), not a fixed Sonnet rate.
        let expected = 1000.0 / 1e6 * 5.0 + 500.0 / 1e6 * 25.0;
        assert!((report.total_estimated_cost_usd - expected).abs() < 1e-9);
        assert_eq!(
            report.tool_usage_distribution,
            vec![("Bash".to_string(), 1)]
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn test_accumulating_stream_entries_keep_the_final_counts() {
        let home = tempdir("accumulate");
        let early = r#"{"input_tokens":10,"output_tokens":1}"#;
        let late = r#"{"input_tokens":10,"output_tokens":90}"#;
        let lines = [
            assistant_line(
                "msg_2",
                "req_2",
                "claude-sonnet-5",
                r#"{"type":"text","text":"a"}"#,
                early,
            ),
            assistant_line(
                "msg_2",
                "req_2",
                "claude-sonnet-5",
                r#"{"type":"text","text":"b"}"#,
                late,
            ),
        ];
        fs::write(
            home.join(".claude/projects/-tmp-demo/s2.jsonl"),
            lines.join("\n"),
        )
        .unwrap();

        let report = SessionHistoryAnalyzer::analyze_home(&home, 10).unwrap();
        assert_eq!(report.usage.output_tokens, 90);
        assert_eq!(report.usage.input_tokens, 10);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn test_copied_requests_in_resumed_sessions_are_counted_once() {
        let home = tempdir("resume");
        let usage = r#"{"input_tokens":100,"output_tokens":10}"#;
        let line = assistant_line(
            "msg_3",
            "req_3",
            "claude-haiku-4-5",
            r#"{"type":"text","text":"x"}"#,
            usage,
        );
        fs::write(home.join(".claude/projects/-tmp-demo/a.jsonl"), &line).unwrap();
        fs::write(home.join(".claude/projects/-tmp-demo/b.jsonl"), &line).unwrap();

        let report = SessionHistoryAnalyzer::analyze_home(&home, 10).unwrap();
        assert_eq!(report.sessions_analyzed, 2);
        assert_eq!(report.api_requests, 1);
        assert_eq!(report.usage.input_tokens, 100);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn test_unknown_models_are_counted_but_not_priced() {
        let home = tempdir("unpriced");
        let usage = r#"{"input_tokens":100,"output_tokens":10}"#;
        let line = assistant_line(
            "msg_4",
            "req_4",
            "mystery-model",
            r#"{"type":"text","text":"x"}"#,
            usage,
        );
        fs::write(home.join(".claude/projects/-tmp-demo/c.jsonl"), &line).unwrap();

        let report = SessionHistoryAnalyzer::analyze_home(&home, 10).unwrap();
        assert_eq!(report.usage.input_tokens, 100);
        assert_eq!(report.total_estimated_cost_usd, 0.0);
        assert_eq!(report.unpriced_models, vec!["mystery-model".to_string()]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn test_unslug_project_takes_last_segment() {
        assert_eq!(
            SessionHistoryAnalyzer::unslug_project("-Users-rd-Documents-GitHub-agentprof"),
            "agentprof"
        );
    }

    #[test]
    fn test_tool_results_are_not_counted_as_human_turns() {
        let tool_result = serde_json::json!({
            "content": [{"type": "tool_result", "content": "ok"}]
        });
        assert!(!SessionHistoryAnalyzer::is_human_turn(&tool_result));

        let typed = serde_json::json!({"content": "review this project"});
        assert!(SessionHistoryAnalyzer::is_human_turn(&typed));

        let blocks = serde_json::json!({
            "content": [{"type": "text", "text": "hi"}]
        });
        assert!(SessionHistoryAnalyzer::is_human_turn(&blocks));
    }

    #[test]
    fn test_missing_transcript_dir_yields_no_sessions() {
        let empty =
            SessionHistoryAnalyzer::collect_transcripts(Path::new("/nonexistent/agentprof"));
        assert!(empty.is_empty());
    }
}
