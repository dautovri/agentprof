//! SARIF 2.1.0 output, so findings show up as GitHub code-scanning alerts and
//! pull-request annotations.

use serde_json::{Value, json};

use crate::core::rule_linter::{LintReport, LintSeverity};
use crate::core::scanner::WorkspaceContextSummary;
use crate::core::workspace_guard::WorkspaceAuditReport;

/// Always-loaded instruction files above this size are reported.
const LARGE_INSTRUCTION_FILE_TOKENS: usize = 3_000;

struct Rule {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    level: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        id: "secret-readable-by-agent",
        name: "SecretReadableByAgent",
        description: "A credential file is readable by Claude Code: no Read(...) deny rule in .claude/settings.json covers it.",
        level: "error",
    },
    Rule {
        id: "build-dir-not-ignored",
        name: "BuildDirectoryNotIgnored",
        description: "A build or dependency directory is not gitignored, so agent search tools crawl it.",
        level: "warning",
    },
    Rule {
        id: "large-instruction-file",
        name: "LargeInstructionFile",
        description: "An instruction file loaded into every agent session is larger than 3,000 tokens.",
        level: "warning",
    },
    Rule {
        id: "instruction-conflict",
        name: "InstructionConflict",
        description: "Two instruction files prescribe mutually exclusive conventions.",
        level: "error",
    },
    Rule {
        id: "vague-rule",
        name: "VagueRule",
        description: "An instruction spends tokens without constraining behaviour.",
        level: "warning",
    },
    Rule {
        id: "hedged-absolute",
        name: "HedgedAbsolute",
        description: "An always/never directive is immediately qualified, leaving the rule ambiguous.",
        level: "warning",
    },
    Rule {
        id: "duplicate-rule",
        name: "DuplicateRule",
        description: "The same directive appears twice in one file.",
        level: "note",
    },
    Rule {
        id: "dense-paragraph",
        name: "DenseParagraph",
        description: "A single bullet runs past 100 words.",
        level: "note",
    },
];

fn lint_rule_id(code: &str) -> &'static str {
    match code {
        "CROSS_FILE_CONFLICT" => "instruction-conflict",
        "VAGUE_RULE" => "vague-rule",
        "HEDGED_ABSOLUTE" => "hedged-absolute",
        "DUPLICATE_RULE" => "duplicate-rule",
        _ => "dense-paragraph",
    }
}

fn result(rule_id: &str, level: &str, message: String, uri: &str, line: usize) -> Value {
    json!({
        "ruleId": rule_id,
        "level": level,
        "message": {"text": message},
        "locations": [{
            "physicalLocation": {
                "artifactLocation": {"uri": uri, "uriBaseId": "%SRCROOT%"},
                "region": {"startLine": line.max(1)}
            }
        }]
    })
}

/// Builds a SARIF log from the repository-level findings.
pub fn build(
    context: &WorkspaceContextSummary,
    guard: &WorkspaceAuditReport,
    lint: &LintReport,
) -> Value {
    let mut results = Vec::new();

    for secret in guard.secret_risks.iter().filter(|s| !s.blocked_for_claude) {
        results.push(result(
            "secret-readable-by-agent",
            "error",
            format!(
                "{} ({}) is readable by Claude Code. Add a Read(...) deny rule to .claude/settings.json (`agentprof fix`).",
                secret.relative_path, secret.description
            ),
            &secret.relative_path,
            1,
        ));
    }

    for dir in guard
        .heavy_directories
        .iter()
        .filter(|d| !d.is_ignored_by_git)
    {
        let (uri, message) = if guard.has_gitignore {
            (
                ".gitignore".to_string(),
                format!(
                    "{} ({}) is not in .gitignore; agent search tools will crawl it.",
                    dir.relative_path, dir.directory_type
                ),
            )
        } else {
            (
                dir.relative_path.clone(),
                format!(
                    "{} ({}) is not gitignored and the repository has no .gitignore.",
                    dir.relative_path, dir.directory_type
                ),
            )
        };
        results.push(result("build-dir-not-ignored", "warning", message, &uri, 1));
    }

    for file in context
        .files
        .iter()
        .filter(|f| f.always_loaded && f.tokens_cl100k > LARGE_INSTRUCTION_FILE_TOKENS)
    {
        results.push(result(
            "large-instruction-file",
            "warning",
            format!(
                "{} is loaded into every session and is ≈{} tokens. Move topic-specific sections into path-scoped rules (`agentprof compile`).",
                file.relative_path, file.tokens_cl100k
            ),
            &file.relative_path,
            1,
        ));
    }

    for issue in &lint.issues {
        let level = match issue.severity {
            LintSeverity::Error => "error",
            LintSeverity::Warning => "warning",
            LintSeverity::Info => "note",
        };
        // Cross-file conflicts name both files; anchor the alert on the first.
        let uri = issue.file.split(" vs ").next().unwrap_or(&issue.file);
        results.push(result(
            lint_rule_id(&issue.code),
            level,
            format!("{} {}", issue.message, issue.suggested_fix),
            uri,
            issue.line_number,
        ));
    }

    let rules: Vec<Value> = RULES
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "shortDescription": {"text": r.description},
                "defaultConfiguration": {"level": r.level},
                "helpUri": "https://github.com/dautovri/agentprof#readme"
            })
        })
        .collect();

    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "agentprof",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/dautovri/agentprof",
                    "rules": rules
                }
            },
            "results": results
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rule_linter::LintIssue;
    use crate::core::workspace_guard::SecretRiskFile;
    use std::path::PathBuf;

    #[test]
    fn test_sarif_contains_rules_and_located_results() {
        let guard = WorkspaceAuditReport {
            secret_risks: vec![
                SecretRiskFile {
                    path: PathBuf::from("/w/.env"),
                    relative_path: ".env".to_string(),
                    risk_level: "High".to_string(),
                    description: "Environment file".to_string(),
                    blocked_for_claude: false,
                    is_ignored_by_cursor: false,
                    is_ignored_by_git: false,
                },
                SecretRiskFile {
                    path: PathBuf::from("/w/k.pem"),
                    relative_path: "k.pem".to_string(),
                    risk_level: "High".to_string(),
                    description: "Key".to_string(),
                    blocked_for_claude: true,
                    is_ignored_by_cursor: false,
                    is_ignored_by_git: false,
                },
            ],
            ..Default::default()
        };
        let lint = LintReport {
            issues: vec![LintIssue {
                file: "AGENTS.md vs CLAUDE.md".to_string(),
                line_number: 7,
                severity: LintSeverity::Error,
                code: "CROSS_FILE_CONFLICT".to_string(),
                message: "conflict".to_string(),
                suggested_fix: "align".to_string(),
            }],
            ..Default::default()
        };

        let sarif = build(&WorkspaceContextSummary::default(), &guard, &lint);
        assert_eq!(sarif["version"], "2.1.0");
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        // The blocked key is not reported.
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["ruleId"], "secret-readable-by-agent");
        let conflict = &results[1];
        assert_eq!(conflict["ruleId"], "instruction-conflict");
        let location = &conflict["locations"][0]["physicalLocation"];
        assert_eq!(location["artifactLocation"]["uri"], "AGENTS.md");
        assert_eq!(location["region"]["startLine"], 7);

        let rule_ids: Vec<&str> = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        for r in results {
            assert!(rule_ids.contains(&r["ruleId"].as_str().unwrap()));
        }
    }
}
