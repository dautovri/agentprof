use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
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

/// Intents that commonly collide across installed skills.
const COMMON_TRIGGERS: &[&str] = &[
    "review",
    "qa",
    "test",
    "security",
    "audit",
    "design",
    "diagram",
    "deploy",
    "ship",
    "screenshot",
    "ios",
    "swift",
    "mcp",
    "analytics",
    "scrape",
    "retro",
    "benchmark",
    "canary",
    "freeze",
    "brand",
    "landing",
    "pricing",
    "refactor",
    "migrate",
    "release",
    "debug",
    "docs",
    "documentation",
    "plan",
    "lint",
    "format",
];

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
            skill_paths.push((
                opencode_skills,
                "OpenCode (~/.config/opencode/skills)".to_string(),
            ));
        }

        // 3. Workspace skills
        let ws_agent_skills = workspace_root.join(".agent/skills");
        if ws_agent_skills.exists() {
            skill_paths.push((ws_agent_skills, "Workspace (.agent/skills)".to_string()));
        }
        let ws_opencode_skills = workspace_root.join(".opencode/skills");
        if ws_opencode_skills.exists() {
            skill_paths.push((
                ws_opencode_skills,
                "Workspace (.opencode/skills)".to_string(),
            ));
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
                    // Skill trees are reachable through several roots (plugin
                    // bundles, symlinked bundles), so the same SKILL.md appeared
                    // more than once and was counted twice in totals and in the
                    // "heaviest skills" list. Canonicalizing collapses those.
                    let identity = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                    if !seen_paths.insert(identity) {
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
                        let triggers = Self::extract_triggers(&skill_name, &desc);
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
        all_skills.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));

        let total_skills = all_skills.len();
        let total_tokens: usize = all_skills.iter().map(|s| s.tokens).sum();
        let average_tokens = total_tokens.checked_div(total_skills).unwrap_or(0);
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

    /// Reads `description:` from YAML frontmatter, falling back to an XML-style
    /// tag. Only the frontmatter block is searched so a `description:` line
    /// inside example code cannot be mistaken for the skill's own description.
    fn extract_description(content: &str) -> String {
        let mut lines = content.lines();
        if lines.next().map(str::trim) == Some("---") {
            for line in lines {
                let trimmed = line.trim();
                if trimmed == "---" {
                    break;
                }
                if let Some(rest) = trimmed.strip_prefix("description:") {
                    let value = rest.trim().trim_matches('"').trim_matches('\'');
                    if !value.is_empty() && value != ">" && value != "|" {
                        return value.to_string();
                    }
                }
            }
        }

        for line in content.lines().take(40) {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("<description>") {
                return rest.trim_end_matches("</description>").trim().to_string();
            }
        }
        String::new()
    }

    /// Extracts trigger keywords from the skill's *name and description* only.
    ///
    /// Two earlier problems are fixed here. First, scanning the first 25 lines of
    /// body text meant incidental prose drove collisions. Second, matching was
    /// bare substring containment, so "portfolios" matched `ios`, "latest"
    /// matched `test`, and "equal" matched `qa` — producing collision reports
    /// full of skills that share no actual intent.
    fn extract_triggers(name: &str, desc: &str) -> Vec<String> {
        let corpus = format!("{} {}", name.replace('-', " "), desc).to_lowercase();
        let words: HashSet<&str> = corpus
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();

        COMMON_TRIGGERS
            .iter()
            .filter(|kw| {
                // Multi-word triggers still need a substring check, but anchored
                // on the normalized corpus rather than raw file text.
                if kw.contains(' ') {
                    corpus.contains(*kw)
                } else {
                    words.contains(*kw)
                }
            })
            .map(|kw| kw.to_string())
            .collect()
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

                let severity = if sorted_skills.len() >= 6 {
                    "🚨 High Overlap".to_string()
                } else {
                    "⚠️ Moderate Overlap".to_string()
                };
                collisions.push(SkillCollision {
                    keyword: kw,
                    colliding_skills: sorted_skills,
                    severity,
                });
            }
        }

        collisions.sort_by_key(|c| std::cmp::Reverse(c.colliding_skills.len()));
        collisions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triggers_use_word_boundaries() {
        // "portfolios" contains "ios" and "latest" contains "test", but neither
        // is a real trigger for those intents.
        let triggers = SkillsAuditor::extract_triggers(
            "brutalist-skill",
            "For data-heavy dashboards and portfolios using the latest layouts.",
        );
        assert!(!triggers.contains(&"ios".to_string()), "got {:?}", triggers);
        assert!(
            !triggers.contains(&"test".to_string()),
            "got {:?}",
            triggers
        );
    }

    #[test]
    fn test_real_triggers_are_still_detected() {
        let triggers = SkillsAuditor::extract_triggers("ios-qa", "Run a QA pass on an iOS app.");
        assert!(triggers.contains(&"ios".to_string()), "got {:?}", triggers);
        assert!(triggers.contains(&"qa".to_string()), "got {:?}", triggers);
    }

    #[test]
    fn test_hyphenated_names_split_into_words() {
        let triggers = SkillsAuditor::extract_triggers("design-review", "");
        assert!(triggers.contains(&"design".to_string()));
        assert!(triggers.contains(&"review".to_string()));
    }

    #[test]
    fn test_description_read_from_frontmatter() {
        let content = "---\nname: demo\ndescription: Audits app store metadata\n---\n\n# Body\ndescription: not this one\n";
        assert_eq!(
            SkillsAuditor::extract_description(content),
            "Audits app store metadata"
        );
    }

    #[test]
    fn test_description_absent_yields_empty() {
        assert_eq!(SkillsAuditor::extract_description("# Just a heading\n"), "");
    }

    #[test]
    fn test_collision_needs_three_distinct_skills() {
        let mk = |name: &str, kw: &str| SkillItem {
            name: name.to_string(),
            path: PathBuf::from(name),
            relative_path: name.to_string(),
            source: "test".to_string(),
            description: String::new(),
            tokens: 10,
            lines: 1,
            trigger_keywords: vec![kw.to_string()],
            is_bloated: false,
        };
        let two = vec![mk("a", "design"), mk("b", "design")];
        assert!(SkillsAuditor::detect_collisions(&two).is_empty());

        let three = vec![mk("a", "design"), mk("b", "design"), mk("c", "design")];
        assert_eq!(SkillsAuditor::detect_collisions(&three).len(), 1);
    }
}
