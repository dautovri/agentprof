use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleFileCategory {
    AgentsMd,
    ClaudeMd,
    CursorRules,
    WindsurfRules,
    CopilotInstructions,
    SkillDoc,
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
    pub fn scan_workspace(root: &Path) -> Result<WorkspaceContextSummary> {
        let mut files = Vec::new();
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            let file_name = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let path_str = path.to_string_lossy();
            // Skip common build or cache directories from scanning themselves as rule files
            if path_str.contains("/.git/") 
                || path_str.contains("/target/") 
                || path_str.contains("/node_modules/") 
                || path_str.contains("/.build/") 
                || path_str.contains("/DerivedData/") 
            {
                continue;
            }

            let category = if file_name.eq_ignore_ascii_case("AGENTS.md") {
                Some(RuleFileCategory::AgentsMd)
            } else if file_name.eq_ignore_ascii_case("CLAUDE.md") {
                Some(RuleFileCategory::ClaudeMd)
            } else if file_name.eq_ignore_ascii_case(".cursorrules") 
                || (path_str.contains("/.cursor/rules/") && file_name.ends_with(".mdc")) {
                Some(RuleFileCategory::CursorRules)
            } else if file_name.eq_ignore_ascii_case(".windsurfrules") {
                Some(RuleFileCategory::WindsurfRules)
            } else if file_name.eq_ignore_ascii_case("copilot-instructions.md") {
                Some(RuleFileCategory::CopilotInstructions)
            } else if file_name.eq_ignore_ascii_case("SKILL.md") {
                Some(RuleFileCategory::SkillDoc)
            } else {
                None
            };

            if let Some(cat) = category {
                if let Ok(content) = fs::read_to_string(path) {
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

                    let rel = path
                        .strip_prefix(&canonical_root)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());

                    files.push(InstructionFileReport {
                        path: path.to_path_buf(),
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
            }
        }

        // Sort largest token count first
        files.sort_by(|a, b| b.tokens_cl100k.cmp(&a.tokens_cl100k));

        let total_files = files.len();
        let total_lines: usize = files.iter().map(|f| f.lines).sum();
        let total_tokens_cl100k: usize = files.iter().map(|f| f.tokens_cl100k).sum();
        let total_tokens_o200k: usize = files.iter().map(|f| f.tokens_o200k).sum();
        let warnings_count = files.iter().filter(|f| matches!(f.status, HealthStatus::Warning)).count();
        let bloated_count = files.iter().filter(|f| matches!(f.status, HealthStatus::Bloated)).count();

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
