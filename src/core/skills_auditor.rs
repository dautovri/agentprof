use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::core::frontmatter;
use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillItem {
    pub name: String,
    pub path: PathBuf,
    pub relative_path: String,
    pub source: String,
    pub description: String,
    /// Tokens of the name and description, which agents load into every
    /// session so they know the skill exists.
    pub always_loaded_tokens: usize,
    /// Tokens of the whole SKILL.md, loaded only when the skill is invoked.
    pub tokens: usize,
    pub lines: usize,
    pub trigger_keywords: Vec<String>,
    /// SKILL.md is longer than the 500 lines Anthropic's skill-authoring
    /// guidance recommends; reference material belongs in separate files.
    pub is_bloated: bool,
    pub issues: Vec<String>,
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
    /// Tokens loaded into every session: the sum of skill names and
    /// descriptions. This is the context cost of having skills installed.
    pub always_loaded_tokens: usize,
    /// Tokens of all SKILL.md files, each loaded only when its skill runs.
    pub total_tokens: usize,
    pub average_tokens: usize,
    pub bloated_skills_count: usize,
    pub collisions: Vec<SkillCollision>,
    pub top_heavy_skills: Vec<SkillItem>,
    pub all_skills: Vec<SkillItem>,
    pub recommendations: Vec<String>,
}

/// Anthropic's guidance keeps SKILL.md under 500 lines.
const MAX_SKILL_LINES: usize = 500;
/// The Agent Skills format caps descriptions at 1,024 characters.
const MAX_DESCRIPTION_CHARS: usize = 1024;

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
        Self::audit_with_home(workspace_root, &home)
    }

    pub(crate) fn audit_with_home(workspace_root: &Path, home: &Path) -> Result<SkillsAuditReport> {
        let mut skill_paths = Vec::new();

        // 1. Claude Code skills: personal and project.
        let claude_skills = home.join(".claude/skills");
        if claude_skills.exists() {
            skill_paths.push((claude_skills, "Claude (~/.claude/skills)".to_string()));
        }
        let ws_claude_skills = workspace_root.join(".claude/skills");
        if ws_claude_skills.exists() {
            skill_paths.push((
                ws_claude_skills,
                "Claude project (.claude/skills)".to_string(),
            ));
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
                        all_skills.push(Self::inspect_skill(path, &content, &source_label));
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

        let always_loaded_tokens: usize = all_skills.iter().map(|s| s.always_loaded_tokens).sum();
        let without_description = all_skills
            .iter()
            .filter(|s| s.description.is_empty())
            .count();

        let mut recommendations = Vec::new();
        if bloated_skills_count > 0 {
            recommendations.push(format!(
                "{} skill(s) have a SKILL.md over {} lines. Move reference material into separate files that load only when needed.",
                bloated_skills_count, MAX_SKILL_LINES
            ));
        }
        if without_description > 0 {
            recommendations.push(format!(
                "{} skill(s) have no description, so the agent has nothing to match requests against.",
                without_description
            ));
        }
        if !collisions.is_empty() {
            recommendations.push(format!(
                "Detected {} trigger collision(s) where multiple skills compete for identical intents.",
                collisions.len()
            ));
        }

        Ok(SkillsAuditReport {
            total_skills,
            always_loaded_tokens,
            total_tokens,
            average_tokens,
            bloated_skills_count,
            collisions,
            top_heavy_skills,
            all_skills,
            recommendations,
        })
    }

    fn inspect_skill(path: &Path, content: &str, source: &str) -> SkillItem {
        let frontmatter = frontmatter::parse(content).unwrap_or_default();
        let dir_name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown-skill".to_string());
        let name = frontmatter
            .get("name")
            .filter(|n| !n.is_empty())
            .map(String::from)
            .unwrap_or(dir_name);
        let description = frontmatter
            .get("description")
            .map(String::from)
            .or_else(|| Self::xml_description(content))
            .unwrap_or_default();

        let tokens = TokenCounter::count_cl100k(content);
        let lines = content.lines().count();
        // What an agent keeps in context for every installed skill.
        let always_loaded_tokens =
            TokenCounter::count_cl100k(&format!("{}: {}", name, description));

        let mut issues = Vec::new();
        if description.is_empty() {
            issues.push("missing description".to_string());
        } else if description.chars().count() > MAX_DESCRIPTION_CHARS {
            issues.push(format!(
                "description exceeds {} characters",
                MAX_DESCRIPTION_CHARS
            ));
        }
        let is_bloated = lines > MAX_SKILL_LINES;
        if is_bloated {
            issues.push(format!("SKILL.md exceeds {} lines", MAX_SKILL_LINES));
        }

        SkillItem {
            trigger_keywords: Self::extract_triggers(&name, &description),
            name,
            path: path.to_path_buf(),
            relative_path: path.to_string_lossy().to_string(),
            source: source.to_string(),
            description,
            always_loaded_tokens,
            tokens,
            lines,
            is_bloated,
            issues,
        }
    }

    /// Fallback for skills that declare `<description>` instead of frontmatter.
    fn xml_description(content: &str) -> Option<String> {
        content.lines().take(40).find_map(|line| {
            line.trim()
                .strip_prefix("<description>")
                .map(|rest| rest.trim_end_matches("</description>").trim().to_string())
        })
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
        let skill = SkillsAuditor::inspect_skill(Path::new("/s/demo/SKILL.md"), content, "test");
        assert_eq!(skill.description, "Audits app store metadata");
        assert!(skill.issues.is_empty(), "{:?}", skill.issues);
    }

    /// Regression: folded `description: >` values were read as empty, so the
    /// skill looked undescribed and never showed up in collision checks.
    #[test]
    fn test_folded_description_is_read() {
        let content = "---\nname: pr-review\ndescription: >\n  Review pull requests\n  for security issues.\n---\nBody\n";
        let skill = SkillsAuditor::inspect_skill(Path::new("/s/x/SKILL.md"), content, "test");
        assert_eq!(
            skill.description,
            "Review pull requests for security issues."
        );
        assert!(skill.trigger_keywords.contains(&"review".to_string()));
        assert!(skill.trigger_keywords.contains(&"security".to_string()));
    }

    #[test]
    fn test_description_absent_is_an_issue() {
        let skill =
            SkillsAuditor::inspect_skill(Path::new("/s/x/SKILL.md"), "# Just a heading\n", "test");
        assert_eq!(skill.description, "");
        assert_eq!(skill.name, "x");
        assert!(
            skill
                .issues
                .iter()
                .any(|i| i.contains("missing description"))
        );
    }

    /// Only the name and description stay in context; the body loads when the
    /// skill runs, so a long body must not count as always-loaded cost.
    #[test]
    fn test_always_loaded_tokens_cover_only_metadata() {
        let body = "Detailed step.\n".repeat(600);
        let content = format!("---\nname: big\ndescription: Deploy the app\n---\n{}", body);
        let skill = SkillsAuditor::inspect_skill(Path::new("/s/big/SKILL.md"), &content, "test");
        assert!(
            skill.always_loaded_tokens < 20,
            "{}",
            skill.always_loaded_tokens
        );
        assert!(skill.tokens > 1_000);
        assert!(skill.is_bloated, "600+ lines exceeds the 500-line guidance");
    }

    #[test]
    fn test_project_skills_are_discovered() {
        let dir = std::env::temp_dir().join(format!("agentprof_skills_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("ws/.claude/skills/deploy")).unwrap();
        fs::create_dir_all(dir.join("home")).unwrap();
        fs::write(
            dir.join("ws/.claude/skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: Deploy and release\n---\n",
        )
        .unwrap();

        let report = SkillsAuditor::audit_with_home(&dir.join("ws"), &dir.join("home")).unwrap();
        assert_eq!(report.total_skills, 1);
        assert_eq!(
            report.all_skills[0].source,
            "Claude project (.claude/skills)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_collision_needs_three_distinct_skills() {
        let mk = |name: &str, kw: &str| SkillItem {
            name: name.to_string(),
            path: PathBuf::from(name),
            relative_path: name.to_string(),
            source: "test".to_string(),
            description: String::new(),
            always_loaded_tokens: 1,
            tokens: 10,
            lines: 1,
            trigger_keywords: vec![kw.to_string()],
            is_bloated: false,
            issues: Vec::new(),
        };
        let two = vec![mk("a", "design"), mk("b", "design")];
        assert!(SkillsAuditor::detect_collisions(&two).is_empty());

        let three = vec![mk("a", "design"), mk("b", "design"), mk("c", "design")];
        assert_eq!(SkillsAuditor::detect_collisions(&three).len(), 1);
    }
}
