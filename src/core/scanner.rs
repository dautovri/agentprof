use anyhow::Result;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleFileCategory {
    AgentsMd,
    ClaudeMd,
    CursorRules,
    WindsurfRules,
    CopilotInstructions,
    SkillDoc,
    GeminiMd,
    ClineRules,
    AiderConventions,
    CodexMd,
    GrokMd,
    CustomInstruction,
}

impl RuleFileCategory {
    pub fn label(&self) -> &'static str {
        match self {
            RuleFileCategory::AgentsMd => "AGENTS.md",
            RuleFileCategory::ClaudeMd => "CLAUDE.md",
            RuleFileCategory::CursorRules => "Cursor Rules",
            RuleFileCategory::WindsurfRules => "Windsurf Rules",
            RuleFileCategory::CopilotInstructions => "Copilot Rules",
            RuleFileCategory::SkillDoc => "Skill Definition",
            RuleFileCategory::GeminiMd => "GEMINI.md",
            RuleFileCategory::ClineRules => "Cline Rules",
            RuleFileCategory::AiderConventions => "Aider Conventions",
            RuleFileCategory::CodexMd => "Codex Rules",
            RuleFileCategory::GrokMd => "GROK.md",
            RuleFileCategory::CustomInstruction => "Instruction File",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Optimal,
    Warning,
    Bloated,
}

impl HealthStatus {
    #[allow(dead_code)]
    pub fn badge(&self) -> &'static str {
        match self {
            HealthStatus::Optimal => "✅ Optimal",
            HealthStatus::Warning => "⚠️ Warning",
            HealthStatus::Bloated => "🚨 Bloated",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionFileReport {
    pub path: PathBuf,
    pub relative_path: String,
    pub category: RuleFileCategory,
    pub lines: usize,
    pub bytes: usize,
    pub tokens_cl100k: usize,
    pub tokens_o200k: usize,
    pub status: HealthStatus,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceContextSummary {
    pub files: Vec<InstructionFileReport>,
    pub total_files: usize,
    pub total_lines: usize,
    pub total_tokens_cl100k: usize,
    pub total_tokens_o200k: usize,
    pub est_cost_per_100_turns: f64,
    pub pct_of_128k: f64,
    pub pct_of_200k: f64,
    pub warnings_count: usize,
    pub bloated_count: usize,
}

pub struct InstructionScanner;

impl InstructionScanner {
    /// Maps a filename to its agent-instruction category.
    ///
    /// Covers the formats the tool claims to support (Cursor, Windsurf, Copilot,
    /// Gemini, Cline, Aider, Codex) rather than only AGENTS.md/CLAUDE.md.
    pub(crate) fn classify(file_name: &str, path_str: &str) -> Option<RuleFileCategory> {
        if file_name.eq_ignore_ascii_case("AGENTS.md") || file_name.eq_ignore_ascii_case("AGENT.md")
        {
            return Some(RuleFileCategory::AgentsMd);
        }
        if file_name.eq_ignore_ascii_case("CLAUDE.md")
            || file_name.eq_ignore_ascii_case("CLAUDE.local.md")
        {
            return Some(RuleFileCategory::ClaudeMd);
        }
        if file_name.eq_ignore_ascii_case(".cursorrules")
            || (path_str.contains("/.cursor/rules/") && file_name.ends_with(".mdc"))
        {
            return Some(RuleFileCategory::CursorRules);
        }
        if file_name.eq_ignore_ascii_case(".windsurfrules")
            || (path_str.contains("/.windsurf/rules/") && file_name.ends_with(".md"))
        {
            return Some(RuleFileCategory::WindsurfRules);
        }
        if file_name.eq_ignore_ascii_case("copilot-instructions.md")
            || (path_str.contains("/.github/instructions/") && file_name.ends_with(".md"))
        {
            return Some(RuleFileCategory::CopilotInstructions);
        }
        if file_name.eq_ignore_ascii_case("SKILL.md") {
            return Some(RuleFileCategory::SkillDoc);
        }
        if file_name.eq_ignore_ascii_case("GEMINI.md") {
            return Some(RuleFileCategory::GeminiMd);
        }
        if file_name.eq_ignore_ascii_case(".clinerules")
            || (path_str.contains("/.clinerules/") && file_name.ends_with(".md"))
        {
            return Some(RuleFileCategory::ClineRules);
        }
        if file_name.eq_ignore_ascii_case("CONVENTIONS.md") {
            return Some(RuleFileCategory::AiderConventions);
        }
        if file_name.eq_ignore_ascii_case("CODEX.md") {
            return Some(RuleFileCategory::CodexMd);
        }
        if file_name.eq_ignore_ascii_case("GROK.md") {
            return Some(RuleFileCategory::GrokMd);
        }
        None
    }
}

impl InstructionScanner {
    /// Instruction files under `root` as `(path, relative path, category)`,
    /// sorted by relative path.
    ///
    /// Respects the repository's own ignore rules instead of a hardcoded skip
    /// list, so vendored or generated instruction files under ignored paths are
    /// not billed to the context budget. `scan`, `context` and `lint` all use
    /// this walk, so they agree on which files exist.
    pub(crate) fn instruction_files(root: &Path) -> Vec<(PathBuf, String, RuleFileCategory)> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let walker = WalkBuilder::new(&canonical_root)
            .follow_links(false)
            .max_depth(Some(6))
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .parents(false)
            // Honour .gitignore even when the directory is not a git checkout;
            // by default the crate applies git rules only inside a repository.
            .require_git(false)
            .build();

        let mut out = Vec::new();
        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let path_str = path.to_string_lossy();
            if path_str.contains("/.git/") || path_str.contains("/node_modules/") {
                continue;
            }
            let Some(cat) = Self::classify(file_name, &path_str) else {
                continue;
            };
            let rel = path
                .strip_prefix(&canonical_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            out.push((path.to_path_buf(), rel, cat));
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    pub fn scan_workspace(root: &Path) -> Result<WorkspaceContextSummary> {
        let mut files = Vec::new();

        for (path, rel, cat) in Self::instruction_files(root) {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };

            let lines = content.lines().count();
            let bytes = content.len();
            let tokens_cl100k = TokenCounter::count_cl100k(&content);
            let tokens_o200k = TokenCounter::count_o200k(&content);

            let status = if tokens_cl100k > 3_000 {
                HealthStatus::Bloated
            } else if tokens_cl100k > 1_500 {
                HealthStatus::Warning
            } else {
                HealthStatus::Optimal
            };

            let mut recs = Vec::new();
            if tokens_cl100k > 3_000 {
                recs.push("File exceeds 3,000 tokens. Consider splitting into JIT modular rules via `agentprof compile`.".to_string());
            } else if tokens_cl100k > 1_500 {
                recs.push("Context size is moderate (>1,500 tokens). Prune conversational filler or older changelog notes.".to_string());
            }

            files.push(InstructionFileReport {
                path,
                relative_path: rel,
                category: cat,
                lines,
                bytes,
                tokens_cl100k,
                tokens_o200k,
                status,
                recommendations: recs,
            });
        }

        // Sort largest token count first
        files.sort_by_key(|f| std::cmp::Reverse(f.tokens_cl100k));

        let total_files = files.len();
        let total_lines: usize = files.iter().map(|f| f.lines).sum();
        let total_tokens_cl100k: usize = files.iter().map(|f| f.tokens_cl100k).sum();
        let total_tokens_o200k: usize = files.iter().map(|f| f.tokens_o200k).sum();
        let warnings_count = files
            .iter()
            .filter(|f| matches!(f.status, HealthStatus::Warning))
            .count();
        let bloated_count = files
            .iter()
            .filter(|f| matches!(f.status, HealthStatus::Bloated))
            .count();

        let est_cost_per_100_turns = TokenCounter::estimate_cost_per_100_turns(total_tokens_cl100k);
        let pct_of_128k = TokenCounter::context_percentage(total_tokens_cl100k, 128_000);
        let pct_of_200k = TokenCounter::context_percentage(total_tokens_cl100k, 200_000);

        Ok(WorkspaceContextSummary {
            files,
            total_files,
            total_lines,
            total_tokens_cl100k,
            total_tokens_o200k,
            est_cost_per_100_turns,
            pct_of_128k,
            pct_of_200k,
            warnings_count,
            bloated_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classifies_known_instruction_files() {
        let cases = [
            ("AGENTS.md", ""),
            ("CLAUDE.md", ""),
            (".cursorrules", ""),
            (".windsurfrules", ""),
            ("copilot-instructions.md", ""),
            ("SKILL.md", ""),
            ("GEMINI.md", ""),
            (".clinerules", ""),
            ("CONVENTIONS.md", ""),
            ("CODEX.md", ""),
        ];
        for (name, path) in cases {
            assert!(
                InstructionScanner::classify(name, path).is_some(),
                "{} should be recognised",
                name
            );
        }
    }

    #[test]
    fn test_ignores_unrelated_markdown() {
        assert!(InstructionScanner::classify("README.md", "/repo/README.md").is_none());
        assert!(InstructionScanner::classify("main.rs", "/repo/src/main.rs").is_none());
    }

    #[test]
    fn test_nested_rule_directories_are_recognised() {
        assert!(
            InstructionScanner::classify("style.mdc", "/repo/.cursor/rules/style.mdc").is_some()
        );
        assert!(
            InstructionScanner::classify("style.md", "/repo/.windsurf/rules/style.md").is_some()
        );
        // The same extension outside the rules directory is not a rule file.
        assert!(InstructionScanner::classify("style.md", "/repo/docs/style.md").is_none());
    }

    #[test]
    fn test_gitignored_instruction_files_are_excluded() {
        let dir = std::env::temp_dir().join(format!("agentprof_scan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("generated")).unwrap();
        fs::write(dir.join(".gitignore"), "generated/\n").unwrap();
        fs::write(dir.join("AGENTS.md"), "# real rules\n").unwrap();
        fs::write(dir.join("generated/AGENTS.md"), "# generated noise\n").unwrap();

        let summary = InstructionScanner::scan_workspace(&dir).unwrap();
        assert_eq!(
            summary.total_files, 1,
            "gitignored rule files must not be counted"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
