use anyhow::Result;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::frontmatter;
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
    ClaudeRules,
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
            RuleFileCategory::ClaudeRules => "Claude Rules",
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionFileReport {
    pub path: PathBuf,
    pub relative_path: String,
    pub category: RuleFileCategory,
    pub lines: usize,
    pub bytes: usize,
    pub tokens_cl100k: usize,
    pub tokens_o200k: usize,
    /// Loaded into every session. When false, `load_condition` says when the
    /// file is loaded instead.
    pub always_loaded: bool,
    pub load_condition: Option<String>,
    pub status: HealthStatus,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceContextSummary {
    pub files: Vec<InstructionFileReport>,
    pub total_files: usize,
    pub total_lines: usize,
    /// Tokens of the files loaded into every session. Budgets, costs and
    /// scores are based on this figure.
    pub total_tokens_cl100k: usize,
    pub total_tokens_o200k: usize,
    /// Tokens of files loaded only under a condition (matching paths, skill
    /// invocation, nested directories).
    pub conditional_tokens_cl100k: usize,
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
        if path_str.contains("/.claude/rules/") && file_name.ends_with(".md") {
            return Some(RuleFileCategory::ClaudeRules);
        }
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
    /// When a file is loaded, or `None` if it is loaded into every session.
    ///
    /// Counting conditional files as always-loaded overstated the budget and
    /// penalised exactly the path-scoped layouts agents recommend.
    pub(crate) fn load_condition(
        category: &RuleFileCategory,
        rel: &str,
        content: &str,
    ) -> Option<String> {
        let fm = frontmatter::parse(content).unwrap_or_default();
        let matching = |globs: Vec<String>| Some(format!("files matching {}", globs.join(", ")));
        match category {
            RuleFileCategory::SkillDoc => Some("when the skill is invoked".to_string()),
            RuleFileCategory::CursorRules if rel.ends_with(".mdc") => {
                let globs = fm.list("globs");
                if fm.is_true("alwaysApply") {
                    None
                } else if !globs.is_empty() {
                    matching(globs)
                } else if fm.get("description").is_some_and(|d| !d.is_empty()) {
                    Some("when the agent picks it by description".to_string())
                } else {
                    Some("only when @-mentioned".to_string())
                }
            }
            RuleFileCategory::ClaudeRules => {
                let paths = fm.list("paths");
                if paths.is_empty() {
                    None
                } else {
                    matching(paths)
                }
            }
            RuleFileCategory::CopilotInstructions if rel.ends_with(".instructions.md") => {
                let apply_to = fm.list("applyTo");
                if apply_to.iter().any(|g| g == "**" || g == "**/*") {
                    None
                } else if apply_to.is_empty() {
                    Some("only when attached manually (no applyTo)".to_string())
                } else {
                    matching(apply_to)
                }
            }
            RuleFileCategory::WindsurfRules if rel.contains(".windsurf/rules/") => {
                match fm.get("trigger") {
                    None | Some("always_on") => None,
                    Some("glob") => matching(fm.list("globs")),
                    Some(other) => Some(format!("trigger: {}", other)),
                }
            }
            // Root memory files load at startup; nested ones only when the
            // agent works inside their directory.
            RuleFileCategory::ClaudeMd | RuleFileCategory::AgentsMd => {
                let parent = Path::new(rel)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string());
                match parent.as_deref() {
                    None | Some("") | Some(".claude") => None,
                    Some(dir) => Some(format!("when the agent works in {}/", dir)),
                }
            }
            _ => None,
        }
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
                recs.push("File exceeds 3,000 tokens. Move language- or area-specific sections into path-scoped rules (`agentprof compile`).".to_string());
            } else if tokens_cl100k > 1_500 {
                recs.push("Context size is moderate (>1,500 tokens). Prune conversational filler or older changelog notes.".to_string());
            }

            let load_condition = Self::load_condition(&cat, &rel, &content);
            files.push(InstructionFileReport {
                always_loaded: load_condition.is_none(),
                load_condition,
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
        let total_tokens_cl100k: usize = files
            .iter()
            .filter(|f| f.always_loaded)
            .map(|f| f.tokens_cl100k)
            .sum();
        let total_tokens_o200k: usize = files
            .iter()
            .filter(|f| f.always_loaded)
            .map(|f| f.tokens_o200k)
            .sum();
        let conditional_tokens_cl100k: usize = files
            .iter()
            .filter(|f| !f.always_loaded)
            .map(|f| f.tokens_cl100k)
            .sum();
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
            conditional_tokens_cl100k,
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
    fn test_load_conditions() {
        use RuleFileCategory::*;
        let cond =
            |cat, rel: &str, content: &str| InstructionScanner::load_condition(&cat, rel, content);

        assert_eq!(
            cond(
                CursorRules,
                ".cursor/rules/a.mdc",
                "---\nalwaysApply: true\n---\nx"
            ),
            None
        );
        assert_eq!(
            cond(
                CursorRules,
                ".cursor/rules/a.mdc",
                "---\nglobs: \"*.ts, *.tsx\"\nalwaysApply: false\n---\nx"
            ),
            Some("files matching *.ts, *.tsx".to_string())
        );
        assert_eq!(
            cond(ClaudeRules, ".claude/rules/style.md", "# always\n"),
            None
        );
        assert_eq!(
            cond(
                ClaudeRules,
                ".claude/rules/api.md",
                "---\npaths:\n  - \"src/api/**\"\n---\n"
            ),
            Some("files matching src/api/**".to_string())
        );
        assert_eq!(cond(ClaudeMd, "CLAUDE.md", ""), None);
        assert_eq!(cond(ClaudeMd, ".claude/CLAUDE.md", ""), None);
        assert_eq!(
            cond(ClaudeMd, "packages/api/CLAUDE.md", ""),
            Some("when the agent works in packages/api/".to_string())
        );
        assert!(cond(SkillDoc, ".claude/skills/x/SKILL.md", "").is_some());
        assert_eq!(
            cond(
                CopilotInstructions,
                ".github/instructions/all.instructions.md",
                "---\napplyTo: \"**\"\n---\n"
            ),
            None
        );
    }

    #[test]
    fn test_conditional_files_do_not_count_toward_the_always_loaded_total() {
        let dir = std::env::temp_dir().join(format!("agentprof_scan_cond_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude/rules")).unwrap();
        fs::write(dir.join("CLAUDE.md"), "# Rules\n- Use tabs.\n").unwrap();
        fs::write(
            dir.join(".claude/rules/api.md"),
            format!(
                "---\npaths:\n  - \"src/api/**\"\n---\n{}",
                "API rule.\n".repeat(50)
            ),
        )
        .unwrap();

        let summary = InstructionScanner::scan_workspace(&dir).unwrap();
        assert_eq!(summary.total_files, 2);
        let always: usize = summary
            .files
            .iter()
            .filter(|f| f.always_loaded)
            .map(|f| f.tokens_cl100k)
            .sum();
        assert_eq!(summary.total_tokens_cl100k, always);
        assert!(summary.conditional_tokens_cl100k > summary.total_tokens_cl100k);
        let _ = fs::remove_dir_all(&dir);
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
