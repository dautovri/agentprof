use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillItem {
    pub name: String,
    pub path: PathBuf,
    pub relative_path: String,
    pub source: String,
    pub description: String,
    pub tokens: usize,
    pub lines: usize,
    pub trigger_keywords: Vec<String>,
    pub is_bloated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCollision {
    pub keyword: String,
    pub colliding_skills: Vec<String>,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsAuditReport {
    pub total_skills: usize,
    pub total_tokens: usize,
    pub average_tokens: usize,
    pub bloated_skills_count: usize,
    pub collisions: Vec<SkillCollision>,
    pub top_heavy_skills: Vec<SkillItem>,
    pub all_skills: Vec<SkillItem>,
    pub recommendations: Vec<String>,
}

pub struct SkillsAuditor;

impl SkillsAuditor {
    pub fn audit(workspace_root: &Path) -> Result<SkillsAuditReport> {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let mut skill_paths = Vec::new();

        // 1. Claude skills
        let claude_skills = home.join(".claude/skills");
        if claude_skills.exists() {
            skill_paths.push((claude_skills, "Claude (~/.claude/skills)".to_string()));
        }

        // 2. OpenCode skills
        let opencode_skills = home.join(".config/opencode/skills");
        if opencode_skills.exists() {
            skill_paths.push((opencode_skills, "OpenCode (~/.config/opencode/skills)".to_string()));
        }

        // 3. Workspace skills
        let ws_agent_skills = workspace_root.join(".agent/skills");
        if ws_agent_skills.exists() {
            skill_paths.push((ws_agent_skills, "Workspace (.agent/skills)".to_string()));
        }
        let ws_opencode_skills = workspace_root.join(".opencode/skills");
        if ws_opencode_skills.exists() {
            skill_paths.push((ws_opencode_skills, "Workspace (.opencode/skills)".to_string()));
        }

        let mut all_skills = Vec::new();
        let mut seen_paths = HashSet::new();

        for (base_dir, source_label) in skill_paths {
            for entry in WalkDir::new(&base_dir)
                .follow_links(false)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let path_str = path.to_string_lossy();
                if path_str.contains("/.git/") || path_str.contains("/node_modules/") {
                    continue;
                }
                if path.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
                    if !seen_paths.insert(path.to_path_buf()) {
                        continue;
                    }

                    if let Ok(content) = fs::read_to_string(path) {
                        let skill_name = path
                            .parent()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown-skill".to_string());

                        let tokens = TokenCounter::count_cl100k(&content);
                        let lines = content.lines().count();
                        let desc = Self::extract_description(&content);
                        let triggers = Self::extract_triggers(&skill_name, &desc, &content);
                        let is_bloated = tokens > 2_500;

                        all_skills.push(SkillItem {
                            name: skill_name,
                            path: path.to_path_buf(),
                            relative_path: path.to_string_lossy().to_string(),
                            source: source_label.clone(),
                            description: desc,
                            tokens,
                            lines,
                            trigger_keywords: triggers,
                            is_bloated,
                        });
                    }
                }
            }
        }

        // Detect collisions
        let collisions = Self::detect_collisions(&all_skills);

        // Sort by tokens descending
        all_skills.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        let total_skills = all_skills.len();
        let total_tokens: usize = all_skills.iter().map(|s| s.tokens).sum();
        let average_tokens = if total_skills > 0 { total_tokens / total_skills } else { 0 };
        let bloated_skills_count = all_skills.iter().filter(|s| s.is_bloated).count();

        let top_heavy_skills = all_skills.iter().take(10).cloned().collect();

        let mut recommendations = Vec::new();
        if bloated_skills_count > 0 {
            recommendations.push(format!("Found {} bloated skill(s) exceeding 2,500 tokens. Ensure they are loaded dynamically via skill tools, not preloaded.", bloated_skills_count));
        }
        if !collisions.is_empty() {
            recommendations.push(format!("Detected {} trigger collision(s) where multiple skills compete for identical intents.", collisions.len()));
        }

        Ok(SkillsAuditReport {
            total_skills,
            total_tokens,
            average_tokens,
            bloated_skills_count,
            collisions,
            top_heavy_skills,
            all_skills,
            recommendations,
        })
    }

    fn extract_description(content: &str) -> String {
        for line in content.lines() {
            let trim = line.trim();
            if trim.starts_with("description:") {
                return trim.trim_start_matches("description:").trim().trim_matches('"').to_string();
            }
            if trim.starts_with("<description>") {
                return trim.trim_start_matches("<description>").trim_end_matches("</description>").trim().to_string();
            }
        }
        "No description found".to_string()
    }

    fn extract_triggers(name: &str, desc: &str, content: &str) -> Vec<String> {
        let mut triggers = Vec::new();
        let corpus = format!("{} {} {}", name, desc, content.lines().take(25).collect::<Vec<_>>().join(" ")).to_lowercase();

        let common_keywords = [
            "review", "qa", "test", "security", "audit", "design", "diagram", "deploy", "ship",
            "screenshot", "ios", "swift", "mcp", "analytics", "scrape", "retro", "office-hours",
            "benchmark", "canary", "freeze", "careful", "brand", "landing", "pricing"
        ];

        for kw in common_keywords {
            if corpus.contains(kw) {
                triggers.push(kw.to_string());
            }
        }
        triggers
    }

    fn detect_collisions(skills: &[SkillItem]) -> Vec<SkillCollision> {
        let mut map: HashMap<String, HashSet<String>> = HashMap::new();

        for s in skills {
            for kw in &s.trigger_keywords {
                map.entry(kw.clone()).or_default().insert(s.name.clone());
            }
        }

        let mut collisions = Vec::new();
        for (kw, skill_set) in map {
            if skill_set.len() >= 3 {
                let mut sorted_skills: Vec<String> = skill_set.into_iter().collect();
                sorted_skills.sort();

                let severity = if sorted_skills.len() >= 6 { "🚨 High Overlap".to_string() } else { "⚠️ Moderate Overlap".to_string() };
                collisions.push(SkillCollision {
                    keyword: kw,
                    colliding_skills: sorted_skills,
                    severity,
                });
            }
        }

        collisions.sort_by(|a, b| b.colliding_skills.len().cmp(&a.colliding_skills.len()));
        collisions
    }
}
