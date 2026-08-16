use std::fs;
use std::path::Path;
use anyhow::Result;
use serde::{Deserialize, Serialize};
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

pub struct RuleLinter;

impl RuleLinter {
    pub fn lint_workspace(workspace_root: &Path) -> Result<LintReport> {
        let mut issues = Vec::new();
        let mut files_scanned = 0;
        let mut file_contents = Vec::new();

        let canonical_root = workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());

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
            if path_str.contains("/.git/") || path_str.contains("/target/") || path_str.contains("/node_modules/") {
                continue;
            }

            let is_rule_file = file_name.eq_ignore_ascii_case("AGENTS.md")
                || file_name.eq_ignore_ascii_case("CLAUDE.md")
                || file_name.eq_ignore_ascii_case("GROK.md")
                || file_name.eq_ignore_ascii_case(".cursorrules")
                || file_name.eq_ignore_ascii_case("copilot-instructions.md");

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
        let contradictions_found = Self::check_cross_file_contradictions(&file_contents, &mut issues);

        let total_issues = issues.len();
        Ok(LintReport {
            total_files_linted: files_scanned,
            total_issues,
            issues,
            contradictions_found,
        })
    }

    fn lint_single_file(file: &str, content: &str, issues: &mut Vec<LintIssue>) {
        for (i, line) in content.lines().enumerate() {
            let line_num = i + 1;
            let trim = line.trim();

            // Check for vague non-actionable instructions
            if trim.to_lowercase() == "write clean code" || trim.to_lowercase() == "make it good" || trim.to_lowercase() == "be helpful" {
                issues.push(LintIssue {
                    file: file.to_string(),
                    line_number: line_num,
                    severity: LintSeverity::Warning,
                    code: "VAGUE_RULE".to_string(),
                    message: format!("Vague rule '{}' wastes tokens without providing actionable constraints.", trim),
                    suggested_fix: "Replace with specific architectural boundaries or conventions.".to_string(),
                });
            }

            // Check for oversized single bullet point lines (>120 words)
            if (trim.starts_with('-') || trim.starts_with('*')) && trim.split_whitespace().count() > 100 {
                issues.push(LintIssue {
                    file: file.to_string(),
                    line_number: line_num,
                    severity: LintSeverity::Info,
                    code: "DENSE_PARAGRAPH".to_string(),
                    message: "Overly long bullet point line (>100 words) reduces LLM directive adherence.".to_string(),
                    suggested_fix: "Break into 2-3 concise sub-bullet constraints.".to_string(),
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
                let pairs = [
                    ("iOS 16", "iOS 17", "Target OS version mismatch"),
                    ("iOS 17", "iOS 26", "Target OS version mismatch"),
                    ("ObservableObject", "@Observable", "State management pattern conflict"),
                    ("SwiftData", "CoreData", "Persistence framework conflict"),
                ];

                for (p1, p2, desc) in pairs {
                    if (c1.contains(p1) && c2.contains(p2)) || (c1.contains(p2) && c2.contains(p1)) {
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
