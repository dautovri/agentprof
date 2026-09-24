use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LintSeverity {
    Warning,
    Error,
    Info,
}

impl LintSeverity {
    #[allow(dead_code)]
    pub fn badge(&self) -> &'static str {
        match self {
            LintSeverity::Warning => "⚠️ Warning",
            LintSeverity::Error => "🚨 Conflict",
            LintSeverity::Info => "ℹ️ Info",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintIssue {
    pub file: String,
    pub line_number: usize,
    pub severity: LintSeverity,
    pub code: String,
    pub message: String,
    pub suggested_fix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LintReport {
    pub total_files_linted: usize,
    pub total_issues: usize,
    pub issues: Vec<LintIssue>,
    pub contradictions_found: usize,
}

/// Mutually exclusive conventions that should not be mandated by two different
/// instruction files at once.
const CONTRADICTION_PAIRS: &[(&str, &str, &str)] = &[
    (
        "ObservableObject",
        "@Observable",
        "SwiftUI state management conflict",
    ),
    ("@StateObject", "@State", "SwiftUI state ownership conflict"),
    ("SwiftData", "CoreData", "Persistence framework conflict"),
    (
        "NavigationView",
        "NavigationStack",
        "SwiftUI navigation API conflict",
    ),
    ("npm install", "pnpm install", "Package manager conflict"),
    ("yarn add", "pnpm add", "Package manager conflict"),
    ("styled-components", "tailwind", "Styling approach conflict"),
    ("Redux", "Zustand", "State library conflict"),
    ("unittest", "pytest", "Python test framework conflict"),
    ("black", "ruff format", "Python formatter conflict"),
    (
        "tabs for indentation",
        "spaces for indentation",
        "Indentation conflict",
    ),
];

pub struct RuleLinter;

impl RuleLinter {
    pub fn lint_workspace(workspace_root: &Path) -> Result<LintReport> {
        let mut issues = Vec::new();
        let mut files_scanned = 0;
        let mut file_contents = Vec::new();

        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .max_depth(5)
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
            if path_str.contains("/.git/")
                || path_str.contains("/target/")
                || path_str.contains("/node_modules/")
            {
                continue;
            }

            // Reuse the scanner's classification so the linter and the context
            // budget agree on what counts as an instruction file.
            let is_rule_file = !matches!(
                crate::core::scanner::InstructionScanner::classify(file_name, &path_str),
                None | Some(crate::core::scanner::RuleFileCategory::SkillDoc)
            ) || file_name.eq_ignore_ascii_case("GROK.md");

            if is_rule_file {
                files_scanned += 1;
                if let Ok(content) = fs::read_to_string(path) {
                    let rel = path
                        .strip_prefix(&canonical_root)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());

                    Self::lint_single_file(&rel, &content, &mut issues);
                    file_contents.push((rel, content));
                }
            }
        }

        // Cross-file contradiction check
        let contradictions_found =
            Self::check_cross_file_contradictions(&file_contents, &mut issues);

        let total_issues = issues.len();
        Ok(LintReport {
            total_files_linted: files_scanned,
            total_issues,
            issues,
            contradictions_found,
        })
    }

    /// Vague directives that consume tokens without constraining behaviour.
    const VAGUE_PHRASES: &'static [&'static str] = &[
        "write clean code",
        "write good code",
        "make it good",
        "be helpful",
        "do your best",
        "follow best practices",
        "use best practices",
        "write high quality code",
        "be professional",
        "think carefully",
        "don't make mistakes",
        "write production ready code",
    ];

    fn lint_single_file(file: &str, content: &str, issues: &mut Vec<LintIssue>) {
        let mut in_code_block = false;
        let mut seen_directives: HashMap<String, usize> = HashMap::new();

        for (i, line) in content.lines().enumerate() {
            let line_num = i + 1;
            let trim = line.trim();

            if trim.starts_with("```") {
                in_code_block = !in_code_block;
                continue;
            }
            // Rules quoted inside examples are not the file's own instructions.
            if in_code_block {
                continue;
            }

            let normalized = trim
                .trim_start_matches(['-', '*', '#', '>', '1', '2', '3', '.', ' '])
                .trim_end_matches(['.', '!'])
                .to_lowercase();

            if Self::VAGUE_PHRASES.contains(&normalized.as_str()) {
                issues.push(LintIssue {
                    file: file.to_string(),
                    line_number: line_num,
                    severity: LintSeverity::Warning,
                    code: "VAGUE_RULE".to_string(),
                    message: format!(
                        "Vague rule '{}' spends tokens without providing an actionable constraint.",
                        trim
                    ),
                    suggested_fix: "Replace with a specific, checkable constraint (a named API, a file layout, a forbidden pattern).".to_string(),
                });
            }

            if (trim.starts_with('-') || trim.starts_with('*'))
                && trim.split_whitespace().count() > 100
            {
                issues.push(LintIssue {
                    file: file.to_string(),
                    line_number: line_num,
                    severity: LintSeverity::Info,
                    code: "DENSE_PARAGRAPH".to_string(),
                    message: "Overly long bullet point (>100 words) reduces directive adherence."
                        .to_string(),
                    suggested_fix: "Break into 2-3 concise sub-bullet constraints.".to_string(),
                });
            }

            // Duplicated directives waste context and can conflict as the file drifts.
            if normalized.len() > 25 && (trim.starts_with('-') || trim.starts_with('*')) {
                match seen_directives.get(&normalized) {
                    Some(&first_line) => {
                        issues.push(LintIssue {
                            file: file.to_string(),
                            line_number: line_num,
                            severity: LintSeverity::Info,
                            code: "DUPLICATE_RULE".to_string(),
                            message: format!("This rule already appears on line {}.", first_line),
                            suggested_fix: "Delete the duplicate to reclaim context.".to_string(),
                        });
                    }
                    None => {
                        seen_directives.insert(normalized.clone(), line_num);
                    }
                }
            }

            // "Never X ... except sometimes X" reads as an unresolved conflict.
            let lower = trim.to_lowercase();
            if (lower.starts_with("always ") || lower.starts_with("never "))
                && (lower.contains(" unless ") || lower.contains(" except when "))
            {
                issues.push(LintIssue {
                    file: file.to_string(),
                    line_number: line_num,
                    severity: LintSeverity::Warning,
                    code: "HEDGED_ABSOLUTE".to_string(),
                    message: "An absolute directive ('always'/'never') is immediately qualified, leaving the real rule ambiguous.".to_string(),
                    suggested_fix: "State the condition first, then the directive.".to_string(),
                });
            }
        }
    }

    fn check_cross_file_contradictions(
        files: &[(String, String)],
        issues: &mut Vec<LintIssue>,
    ) -> usize {
        let mut count = 0;
        if files.len() < 2 {
            return 0;
        }

        // Check pairs of files
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let (f1, c1) = &files[i];
                let (f2, c2) = &files[j];

                // Contradiction patterns
                for &(p1, p2, desc) in CONTRADICTION_PAIRS {
                    if (c1.contains(p1) && c2.contains(p2)) || (c1.contains(p2) && c2.contains(p1))
                    {
                        count += 1;
                        issues.push(LintIssue {
                            file: format!("{} vs {}", f1, f2),
                            line_number: 1,
                            severity: LintSeverity::Error,
                            code: "CROSS_FILE_CONFLICT".to_string(),
                            message: format!("{}: '{}' mentions {} while '{}' mentions {}.", desc, f1, p1, f2, p2),
                            suggested_fix: "Align instructions to use a single unified standard across all agent files.".to_string(),
                        });
                    }
                }
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(content: &str) -> Vec<LintIssue> {
        let mut issues = Vec::new();
        RuleLinter::lint_single_file("TEST.md", content, &mut issues);
        issues
    }

    #[test]
    fn test_vague_rule_is_flagged_with_list_markers() {
        let issues = lint("- Follow best practices.\n");
        assert!(
            issues.iter().any(|i| i.code == "VAGUE_RULE"),
            "{:?}",
            issues
        );
    }

    #[test]
    fn test_rules_inside_code_blocks_are_ignored() {
        // A vague phrase shown as an example must not be linted as a real rule.
        let issues = lint("Example of what not to write:\n\n```\nwrite clean code\n```\n");
        assert!(issues.is_empty(), "{:?}", issues);
    }

    #[test]
    fn test_duplicate_directive_is_reported_once() {
        let content = "- Always run cargo fmt before committing changes\n- Always run cargo fmt before committing changes\n";
        let dups: Vec<_> = lint(content)
            .into_iter()
            .filter(|i| i.code == "DUPLICATE_RULE")
            .collect();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].line_number, 2);
    }

    #[test]
    fn test_hedged_absolute_is_flagged() {
        let issues = lint("Never use force unwrap unless the value is a compile-time literal\n");
        assert!(
            issues.iter().any(|i| i.code == "HEDGED_ABSOLUTE"),
            "{:?}",
            issues
        );
    }

    #[test]
    fn test_clean_rules_produce_no_issues() {
        let issues = lint("- Use `@Observable` for view models.\n- Target iOS 26 and above.\n");
        assert!(issues.is_empty(), "{:?}", issues);
    }

    #[test]
    fn test_cross_file_contradiction_detected() {
        let files = vec![
            (
                "A.md".to_string(),
                "Use ObservableObject for view models".to_string(),
            ),
            (
                "B.md".to_string(),
                "Use @Observable for view models".to_string(),
            ),
        ];
        let mut issues = Vec::new();
        let count = RuleLinter::check_cross_file_contradictions(&files, &mut issues);
        assert_eq!(count, 1);
        assert_eq!(issues[0].code, "CROSS_FILE_CONFLICT");
    }
}
