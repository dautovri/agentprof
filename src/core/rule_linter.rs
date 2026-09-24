use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::scanner::{InstructionScanner, RuleFileCategory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LintSeverity {
    Info,
    Warning,
    Error,
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

impl LintReport {
    pub fn max_severity(&self) -> Option<LintSeverity> {
        self.issues.iter().map(|i| i.severity).max()
    }
}

/// A convention that can be prescribed by an instruction file.
struct Term {
    /// How the convention is named in messages.
    label: &'static str,
    /// Lowercase spellings that name it.
    spellings: &'static [&'static str],
    /// Text that, directly after a spelling, means something else
    /// (`unittest.mock` is routinely used with pytest).
    not_followed_by: &'static [&'static str],
}

const fn term(label: &'static str, spellings: &'static [&'static str]) -> Term {
    Term {
        label,
        spellings,
        not_followed_by: &[],
    }
}

/// Mutually exclusive conventions that should not be prescribed by two
/// different instruction files at once.
const CONTRADICTION_PAIRS: &[(Term, Term, &str)] = &[
    (
        term("ObservableObject", &["observableobject"]),
        term("@Observable", &["@observable"]),
        "SwiftUI observation conflict",
    ),
    (
        term("SwiftData", &["swiftdata"]),
        term("Core Data", &["core data", "coredata"]),
        "Persistence framework conflict",
    ),
    (
        term("NavigationView", &["navigationview"]),
        term("NavigationStack", &["navigationstack"]),
        "SwiftUI navigation API conflict",
    ),
    (
        term("npm", &["npm install", "npm i", "npm ci"]),
        term("pnpm", &["pnpm install", "pnpm add", "pnpm i"]),
        "Package manager conflict",
    ),
    (
        term("npm", &["npm install", "npm i", "npm ci"]),
        term("yarn", &["yarn add", "yarn install"]),
        "Package manager conflict",
    ),
    (
        term("pnpm", &["pnpm install", "pnpm add", "pnpm i"]),
        term("yarn", &["yarn add", "yarn install"]),
        "Package manager conflict",
    ),
    (
        term("styled-components", &["styled-components"]),
        term("Tailwind", &["tailwind", "tailwindcss"]),
        "Styling approach conflict",
    ),
    (
        term("Redux", &["redux", "redux toolkit"]),
        term("Zustand", &["zustand"]),
        "State library conflict",
    ),
    (
        Term {
            label: "unittest",
            spellings: &["unittest"],
            not_followed_by: &[".mock"],
        },
        term("pytest", &["pytest"]),
        "Python test framework conflict",
    ),
    (
        term("tabs for indentation", &["tabs for indentation"]),
        term("spaces for indentation", &["spaces for indentation"]),
        "Indentation conflict",
    ),
];

/// Words that, earlier in the same clause, turn a mention into a rejection:
/// "use @Observable instead of ObservableObject", "never run npm install".
const NEGATION_CUES: &[&str] = &[
    "not ",
    "never",
    "don't",
    "dont ",
    "do not",
    "avoid",
    "instead of",
    "rather than",
    "no longer",
    "without",
    "deprecated",
    "legacy",
    "ban ",
    "forbid",
    "stop using",
];

/// Cues that reject what follows them unless a target marker comes first:
/// in "replace X with Y" and "migrate from X to Y", Y is the prescription.
const SOURCE_TARGET_CUES: &[(&str, &[&str])] = &[
    ("replace", &[" with ", " by "]),
    ("migrate from", &[" to "]),
    ("migrating from", &[" to "]),
    ("migrated from", &[" to "]),
    ("switch from", &[" to "]),
    ("switched from", &[" to "]),
    ("moving from", &[" to "]),
];

/// Words right after a mention that reject it: "ObservableObject is deprecated".
const TRAILING_NEGATION_CUES: &[&str] = &[
    "deprecated",
    "is banned",
    "is forbidden",
    "not allowed",
    "is discouraged",
];

/// Headings that make every mention in their section a rejection.
const NEGATIVE_HEADING_CUES: &[&str] = &[
    "don't",
    "dont",
    "do not",
    "avoid",
    "never",
    "forbidden",
    "banned",
    "anti-pattern",
    "antipattern",
    "deprecated",
    "legacy",
];

pub struct RuleLinter;

impl RuleLinter {
    pub fn lint_workspace(workspace_root: &Path) -> Result<LintReport> {
        let mut issues = Vec::new();
        let mut files_scanned = 0;
        let mut file_contents = Vec::new();

        // The same walk as `scan`/`context`: gitignore-aware, so the linter and
        // the context budget agree on which instruction files exist. Skills are
        // excluded because their bodies load only when invoked.
        for (path, rel, category) in InstructionScanner::instruction_files(workspace_root) {
            if matches!(category, RuleFileCategory::SkillDoc) {
                continue;
            }
            files_scanned += 1;
            if let Ok(content) = fs::read_to_string(&path) {
                Self::lint_single_file(&rel, &content, &mut issues);
                file_contents.push((rel, content));
            }
        }

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

    /// Lines outside fenced code blocks, as `(line number, text)`. Rules quoted
    /// inside examples are not the file's own instructions.
    fn prose_lines(content: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let mut fence: Option<&str> = None;
        for (i, line) in content.lines().enumerate() {
            let trim = line.trim_start();
            let marker = if trim.starts_with("```") {
                Some("```")
            } else if trim.starts_with("~~~") {
                Some("~~~")
            } else {
                None
            };
            match (fence, marker) {
                (None, Some(m)) => fence = Some(m),
                (Some(open), Some(m)) if open == m => fence = None,
                (None, None) => out.push((i + 1, line)),
                _ => {}
            }
        }
        out
    }

    /// Strips list markers ("- ", "* ", "1. ", "2) ") and trailing punctuation.
    fn normalize_directive(trim: &str) -> String {
        trim.trim_start_matches(|c: char| {
            c.is_ascii_digit() || matches!(c, '-' | '*' | '+' | '#' | '>' | '.' | ')' | ' ')
        })
        .trim_end_matches(['.', '!'])
        .to_lowercase()
    }

    fn lint_single_file(file: &str, content: &str, issues: &mut Vec<LintIssue>) {
        let mut seen_directives: HashMap<String, usize> = HashMap::new();

        for (line_num, line) in Self::prose_lines(content) {
            let trim = line.trim();
            let normalized = Self::normalize_directive(trim);
            let is_bullet = trim.starts_with('-') || trim.starts_with('*') || trim.starts_with('+');

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

            if is_bullet && trim.split_whitespace().count() > 100 {
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
            if normalized.len() > 25 && is_bullet {
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
            let lower = normalized.as_str();
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

    /// Byte offsets where `spelling` occurs in `line` as a whole word.
    fn find_mentions(line: &str, spelling: &str, not_followed_by: &[&str]) -> Vec<usize> {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(found) = line[from..].find(spelling) {
            let start = from + found;
            let end = start + spelling.len();
            let before_ok = line[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word(c) && c != '@');
            let after = &line[end..];
            let after_ok = after.chars().next().is_none_or(|c| !is_word(c))
                && !not_followed_by.iter().any(|s| after.starts_with(s));
            if before_ok && after_ok {
                out.push(start);
            }
            from = end;
        }
        out
    }

    /// True when the words around a mention reject it rather than prescribe it.
    fn is_negated(line: &str, start: usize, end: usize) -> bool {
        // Only the current clause counts: in "never use npm; use pnpm" the
        // "never" does not reach pnpm.
        let clause_start = line[..start]
            .rfind([';', '.', '!', '?', ',', ':', '('])
            .map(|i| i + 1)
            .unwrap_or(0);
        let clause = &line[clause_start..start];
        let clause = match clause.rfind(" but ") {
            Some(i) => &clause[i + 5..],
            None => clause,
        };
        for (cue, target_markers) in SOURCE_TARGET_CUES {
            if let Some(i) = clause.rfind(cue) {
                let after = &clause[i + cue.len()..];
                return !target_markers.iter().any(|m| after.contains(m));
            }
        }
        if NEGATION_CUES.iter().any(|cue| clause.contains(cue)) {
            return true;
        }
        let rest = &line[end..];
        let window = &rest[..rest
            .char_indices()
            .nth(30)
            .map(|(i, _)| i)
            .unwrap_or(rest.len())];
        TRAILING_NEGATION_CUES
            .iter()
            .any(|cue| window.contains(cue))
    }

    /// First line on which a file prescribes (rather than rejects) the term.
    fn prescribes(content: &str, term: &Term) -> Option<usize> {
        let mut negative_section = false;
        for (line_num, line) in Self::prose_lines(content) {
            let lower = line.to_lowercase();
            let trimmed = lower.trim_start();
            if trimmed.starts_with('#') {
                negative_section = NEGATIVE_HEADING_CUES.iter().any(|c| trimmed.contains(c));
                continue;
            }
            if negative_section {
                continue;
            }
            for spelling in term.spellings {
                for start in Self::find_mentions(&lower, spelling, term.not_followed_by) {
                    if !Self::is_negated(&lower, start, start + spelling.len()) {
                        return Some(line_num);
                    }
                }
            }
        }
        None
    }

    fn check_cross_file_contradictions(
        files: &[(String, String)],
        issues: &mut Vec<LintIssue>,
    ) -> usize {
        let mut count = 0;
        if files.len() < 2 {
            return 0;
        }

        for (i, (f1, c1)) in files.iter().enumerate() {
            for (f2, c2) in files.iter().skip(i + 1) {
                for (a, b, desc) in CONTRADICTION_PAIRS {
                    let hit = match (Self::prescribes(c1, a), Self::prescribes(c2, b)) {
                        (Some(l1), Some(l2)) => Some((l1, a, l2, b)),
                        _ => match (Self::prescribes(c1, b), Self::prescribes(c2, a)) {
                            (Some(l1), Some(l2)) => Some((l1, b, l2, a)),
                            _ => None,
                        },
                    };
                    let Some((l1, t1, l2, t2)) = hit else {
                        continue;
                    };
                    count += 1;
                    issues.push(LintIssue {
                        file: format!("{} vs {}", f1, f2),
                        line_number: l1,
                        severity: LintSeverity::Error,
                        code: "CROSS_FILE_CONFLICT".to_string(),
                        message: format!(
                            "{}: {}:{} prescribes '{}' while {}:{} prescribes '{}'.",
                            desc, f1, l1, t1.label, f2, l2, t2.label
                        ),
                        suggested_fix:
                            "Align instructions to use a single unified standard across all agent files."
                                .to_string(),
                    });
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

    fn conflicts(a: &str, b: &str) -> Vec<LintIssue> {
        let files = vec![
            ("A.md".to_string(), a.to_string()),
            ("B.md".to_string(), b.to_string()),
        ];
        let mut issues = Vec::new();
        RuleLinter::check_cross_file_contradictions(&files, &mut issues);
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
        let numbered = lint("4. Follow best practices\n");
        assert!(
            numbered.iter().any(|i| i.code == "VAGUE_RULE"),
            "{:?}",
            numbered
        );
    }

    #[test]
    fn test_rules_inside_code_blocks_are_ignored() {
        // A vague phrase shown as an example must not be linted as a real rule.
        let issues = lint("Example of what not to write:\n\n```\nwrite clean code\n```\n");
        assert!(issues.is_empty(), "{:?}", issues);
        let tilde = lint("~~~\nwrite clean code\n~~~\n");
        assert!(tilde.is_empty(), "{:?}", tilde);
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
    fn test_cross_file_contradiction_detected_with_line_numbers() {
        let issues = conflicts(
            "# Rules\nUse ObservableObject for view models",
            "Use @Observable for view models",
        );
        assert_eq!(issues.len(), 1, "{:?}", issues);
        assert_eq!(issues[0].code, "CROSS_FILE_CONFLICT");
        assert_eq!(issues[0].line_number, 2);
        assert!(
            issues[0].message.contains("A.md:2"),
            "{}",
            issues[0].message
        );
    }

    /// Regression: two identical files were reported as conflicting because
    /// "pnpm install" contains "npm install" and "@StateObject" contains "@State".
    #[test]
    fn test_agreeing_files_are_not_conflicts() {
        let rules = "- Always use pnpm install for dependencies\n- Use @StateObject for owned view models\n";
        assert!(
            conflicts(rules, rules).is_empty(),
            "{:?}",
            conflicts(rules, rules)
        );
    }

    #[test]
    fn test_rejected_mentions_are_not_prescriptions() {
        for rejection in [
            "Use @Observable instead of ObservableObject.",
            "Never use ObservableObject.",
            "Avoid ObservableObject in new code",
            "ObservableObject is deprecated here.",
            "Replace ObservableObject with @Observable",
        ] {
            let issues = conflicts(rejection, "Use @Observable everywhere");
            assert!(issues.is_empty(), "{:?} for {}", issues, rejection);
        }
        // In "migrate from X to Y" the target is prescribed, the source is not.
        let issues = conflicts("Migrate from Redux to Zustand", "Use Redux Toolkit");
        assert_eq!(issues.len(), 1, "{:?}", issues);
        assert!(
            issues[0].message.contains("'Zustand'"),
            "{}",
            issues[0].message
        );
        // A negation in an earlier clause does not reach the next one.
        let issues = conflicts(
            "Never run npm install; always use pnpm install",
            "Use npm install",
        );
        assert_eq!(issues.len(), 1, "{:?}", issues);
    }

    #[test]
    fn test_negative_sections_do_not_prescribe() {
        let issues = conflicts(
            "## Don'ts\n- ObservableObject\n- NavigationView\n",
            "Use @Observable and NavigationStack",
        );
        assert!(issues.is_empty(), "{:?}", issues);
    }

    #[test]
    fn test_word_boundaries_and_exclusions() {
        assert!(conflicts("Use tailwindcss", "Use styled-components").len() == 1);
        // unittest.mock is routinely used with pytest.
        assert!(conflicts("from unittest.mock import patch", "Run tests with pytest").is_empty());
        // A mention inside another word is not a mention.
        assert!(conflicts("Configure reduxjs-style stores", "Use zustand").is_empty());
    }
}
