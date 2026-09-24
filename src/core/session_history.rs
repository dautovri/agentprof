use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::tokens::Pricing;

/// How many transcripts to read by default.
///
/// Transcript directories routinely reach multiple GB; reading every one on a
/// bare `agentprof history` would take minutes. The scan is bounded to the most
/// recently modified sessions and the report states its own coverage.
pub const DEFAULT_SESSION_LIMIT: usize = 50;

/// Cache-aware multipliers on the base input price, per Anthropic's published
/// prompt-caching rates.
const CACHE_WRITE_MULTIPLIER: f64 = 1.25;
const CACHE_READ_MULTIPLIER: f64 = 0.1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_creation_tokens: usize,
    pub cache_read_tokens: usize,
}

impl TokenUsage {
    fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }

    pub fn total(&self) -> usize {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    /// Billable cost using cache-aware input rates.
    pub fn cost_usd(&self, pricing: &Pricing) -> f64 {
        pricing.input_cost(self.input_tokens)
            + pricing.input_cost(self.cache_creation_tokens) * CACHE_WRITE_MULTIPLIER
            + pricing.input_cost(self.cache_read_tokens) * CACHE_READ_MULTIPLIER
            + pricing.output_cost(self.output_tokens)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub project: String,
    pub last_modified: String,
    pub turn_count: usize,
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
    pub usage: TokenUsage,
    pub total_tokens_used: usize,
    pub total_estimated_cost_usd: f64,
    pub pricing_label: String,
    pub tool_usage_distribution: Vec<(String, usize)>,
    pub recent_sessions: Vec<SessionSummary>,
    pub loop_thrash_incidents: usize,
    pub prompt_history_entries: usize,
    pub sources_scanned: Vec<String>,
    pub recommendations: Vec<String>,
}

pub struct SessionHistoryAnalyzer;

impl SessionHistoryAnalyzer {
    pub fn analyze() -> Result<SessionHistoryReport> {
        Self::analyze_with_limit(DEFAULT_SESSION_LIMIT)
    }

    pub fn analyze_with_limit(limit: usize) -> Result<SessionHistoryReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
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
        let mut usage = TokenUsage::default();
        let mut loop_thrash_incidents = 0usize;
        let pricing = Pricing::DEFAULT;

        for (path, modified) in &transcripts {
            if let Some(summary) =
                Self::parse_transcript(path, *modified, &pricing, &mut tool_counts)
            {
                total_turns += summary.turn_count;
                usage.add(&summary.usage);
                if summary.repeated_tool_calls > 0 {
                    loop_thrash_incidents += 1;
                }
                sessions.push(summary);
            }
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

        let total_estimated_cost_usd = usage.cost_usd(&pricing);
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
            usage,
            total_tokens_used,
            total_estimated_cost_usd,
            pricing_label: pricing.label.to_string(),
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
        pricing: &Pricing,
        global_tools: &mut HashMap<String, usize>,
    ) -> Option<SessionSummary> {
        let file = File::open(path).ok()?;
        let reader = BufReader::with_capacity(256 * 1024, file);

        let mut usage = TokenUsage::default();
        let mut turn_count = 0usize;
        let mut session_tools: HashMap<String, usize> = HashMap::new();
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

            if let Some(model) = message.get("model").and_then(|m| m.as_str())
                && !models.iter().any(|m| m == model)
            {
                models.push(model.to_string());
            }

            if let Some(u) = message.get("usage") {
                usage.add(&TokenUsage {
                    input_tokens: Self::field(u, "input_tokens"),
                    output_tokens: Self::field(u, "output_tokens"),
                    cache_creation_tokens: Self::field(u, "cache_creation_input_tokens"),
                    cache_read_tokens: Self::field(u, "cache_read_input_tokens"),
                });
            }

            if let Some(blocks) = message.get("content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    let Some(name) = block.get("name").and_then(|n| n.as_str()) else {
                        continue;
                    };
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
        if usage.total() == 0 && session_tools.is_empty() {
            return None;
        }

        let dominant_tool = session_tools
            .iter()
            .max_by_key(|(name, count)| (**count, std::cmp::Reverse(name.as_str())))
            .map(|(name, _)| name.clone());

        Some(SessionSummary {
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
            usage,
            estimated_cost_usd: usage.cost_usd(pricing),
            models,
            dominant_tool,
            repeated_tool_calls,
        })
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

    #[test]
    fn test_usage_cost_is_cache_aware() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        };
        let pricing = Pricing::DEFAULT;
        assert!((usage.cost_usd(&pricing) - pricing.input_per_mtok).abs() < 1e-9);

        // Cache reads bill at 10% of the input rate.
        let cached = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 1_000_000,
        };
        assert!((cached.cost_usd(&pricing) - pricing.input_per_mtok * 0.1).abs() < 1e-9);
    }

    #[test]
    fn test_usage_total_sums_all_buckets() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_tokens: 4,
            cache_read_tokens: 8,
        };
        assert_eq!(usage.total(), 15);
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
