use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::claude_permissions::ReadDenyRules;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavyDirectory {
    pub path: PathBuf,
    pub relative_path: String,
    pub estimated_files: usize,
    pub directory_type: String,
    /// Agent search tools (ripgrep-based Grep/Glob, Cursor indexing) skip
    /// gitignored paths, so this is what keeps them out of searches.
    pub is_ignored_by_git: bool,
    pub is_ignored_by_cursor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRiskFile {
    pub path: PathBuf,
    pub relative_path: String,
    pub risk_level: String,
    pub description: String,
    /// A `Read(...)` rule in a Claude Code settings file stops the agent from
    /// reading this file.
    pub blocked_for_claude: bool,
    pub is_ignored_by_cursor: bool,
    pub is_ignored_by_git: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceAuditReport {
    pub root: PathBuf,
    pub has_gitignore: bool,
    pub has_cursorignore: bool,
    /// Present only to warn about it: Claude Code never reads `.claudeignore`.
    pub has_claudeignore: bool,
    /// `Read` deny rules found across the Claude Code settings files in play.
    pub claude_read_deny_rules: usize,
    pub claude_deny_rule_sources: Vec<String>,
    pub detected_project_types: Vec<String>,
    pub heavy_directories: Vec<HeavyDirectory>,
    pub secret_risks: Vec<SecretRiskFile>,
    /// Heavy directories that are not gitignored.
    pub total_unignored_heavy_dirs: usize,
    /// Secret files Claude Code can read (no deny rule covers them).
    pub total_exposed_secrets: usize,
    pub recommendations: Vec<String>,
}

/// Directory names that hold build output or dependencies.
///
/// `estimated_files` is capped: counting every file under a large
/// `node_modules` costs more than the rest of the audit combined.
pub const FILE_COUNT_CAP: usize = 2_000;

pub struct WorkspaceGuard;

impl WorkspaceGuard {
    pub fn audit(root: &Path) -> Result<WorkspaceAuditReport> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::audit_with_home(root, home.as_deref())
    }

    /// Audits `root`, reading user-level Claude Code settings from `home`.
    pub fn audit_with_home(root: &Path, home: Option<&Path>) -> Result<WorkspaceAuditReport> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let cursorignore_path = canonical_root.join(".cursorignore");
        let gitignore_path = canonical_root.join(".gitignore");

        let has_cursorignore = cursorignore_path.exists();
        let has_gitignore = gitignore_path.exists();
        let has_claudeignore = canonical_root.join(".claudeignore").exists();

        // Real gitignore-syntax matchers, not substring checks on the file text.
        let cursor_matcher = Self::build_matcher(&canonical_root, &cursorignore_path);
        let git_matcher = Self::build_matcher(&canonical_root, &gitignore_path);
        let claude_rules = ReadDenyRules::load(&canonical_root, home);

        let detected_project_types = Self::detect_project_types(&canonical_root);

        let mut heavy_directories = Vec::new();
        let mut secret_risks = Vec::new();

        let mut walker = WalkDir::new(&canonical_root)
            .follow_links(false)
            .max_depth(6)
            .into_iter();

        while let Some(entry) = walker.next() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };

            let rel_path = path
                .strip_prefix(&canonical_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());

            if entry.file_type().is_dir() {
                if entry.depth() == 0 {
                    continue;
                }
                if file_name == ".git" {
                    walker.skip_current_dir();
                    continue;
                }

                let dtype = Self::heavy_dir_type(file_name, path, &canonical_root);

                // Report the directory, then refuse to walk through it. Descending
                // into node_modules/target to hunt for secrets cost orders of
                // magnitude more than the rest of the audit and surfaced vendored
                // test fixtures as if they were the user's own credentials.
                if Self::is_heavy_dir_name(file_name) {
                    walker.skip_current_dir();
                }

                let Some(dtype) = dtype else { continue };
                let file_count = WalkDir::new(path)
                    .max_depth(4)
                    .into_iter()
                    .take(FILE_COUNT_CAP)
                    .filter_map(|e| e.ok())
                    .count();

                heavy_directories.push(HeavyDirectory {
                    path: path.to_path_buf(),
                    relative_path: rel_path,
                    estimated_files: file_count,
                    directory_type: dtype.to_string(),
                    is_ignored_by_git: Self::is_ignored(&git_matcher, path, true),
                    is_ignored_by_cursor: Self::is_ignored(&cursor_matcher, path, true),
                });
            } else if let Some((risk, desc)) = Self::secret_risk(file_name) {
                secret_risks.push(SecretRiskFile {
                    path: path.to_path_buf(),
                    relative_path: rel_path,
                    risk_level: risk.to_string(),
                    description: desc.to_string(),
                    blocked_for_claude: claude_rules.blocks(path, false),
                    is_ignored_by_cursor: Self::is_ignored(&cursor_matcher, path, false),
                    is_ignored_by_git: Self::is_ignored(&git_matcher, path, false),
                });
            }
        }

        heavy_directories.sort_by_key(|d| std::cmp::Reverse(d.estimated_files));
        secret_risks.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        let total_unignored_heavy_dirs = heavy_directories
            .iter()
            .filter(|d| !d.is_ignored_by_git)
            .count();
        let total_exposed_secrets = secret_risks
            .iter()
            .filter(|s| !s.blocked_for_claude)
            .count();

        let mut recommendations = Vec::new();
        if total_exposed_secrets > 0 {
            recommendations.push(format!(
                "{} secret file(s) are readable by Claude Code: no `Read(...)` deny rule covers them. \
                 Run `agentprof fix` to add deny rules to .claude/settings.json.",
                total_exposed_secrets
            ));
        }
        if has_claudeignore {
            recommendations.push(
                "`.claudeignore` is not read by Claude Code and protects nothing. \
                 Use `permissions.deny` Read rules in .claude/settings.json instead (`agentprof fix`)."
                    .to_string(),
            );
        }
        // A secret that git also fails to ignore is a commit risk, not just an
        // agent-context risk, and is worth calling out separately.
        let committable = secret_risks.iter().filter(|s| !s.is_ignored_by_git).count();
        if committable > 0 {
            recommendations.push(format!(
                "{} secret file(s) are not gitignored either — they risk being committed.",
                committable
            ));
        }
        if total_unignored_heavy_dirs > 0 {
            recommendations.push(format!(
                "{} build/dependency folder(s) are not in .gitignore, so agent search tools crawl them. Add them to .gitignore.",
                total_unignored_heavy_dirs
            ));
        }
        if !has_gitignore && !heavy_directories.is_empty() {
            recommendations.push("No .gitignore found at the workspace root.".to_string());
        }

        Ok(WorkspaceAuditReport {
            root: canonical_root,
            has_gitignore,
            has_cursorignore,
            has_claudeignore,
            claude_read_deny_rules: claude_rules.rule_count,
            claude_deny_rule_sources: claude_rules.sources,
            detected_project_types,
            heavy_directories,
            secret_risks,
            total_unignored_heavy_dirs,
            total_exposed_secrets,
            recommendations,
        })
    }

    fn build_matcher(root: &Path, ignore_file: &Path) -> Option<Gitignore> {
        if !ignore_file.exists() {
            return None;
        }
        let mut builder = GitignoreBuilder::new(root);
        builder.add(ignore_file);
        builder.build().ok()
    }

    fn is_ignored(matcher: &Option<Gitignore>, path: &Path, is_dir: bool) -> bool {
        matcher
            .as_ref()
            .map(|m| m.matched_path_or_any_parents(path, is_dir).is_ignore())
            .unwrap_or(false)
    }

    pub(crate) fn is_heavy_dir_name(name: &str) -> bool {
        matches!(
            name,
            "node_modules"
                | "target"
                | ".build"
                | "DerivedData"
                | ".next"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".terraform"
                | ".turbo"
                | "vendor"
        )
    }

    /// Classifies a heavy directory, or returns None if the name is heavy only
    /// in some project layouts and this is not one of them.
    pub(crate) fn heavy_dir_type(name: &str, path: &Path, root: &Path) -> Option<&'static str> {
        match name {
            "node_modules" => Some("Node Dependencies"),
            // Only a Cargo `target/` is a build cache; plenty of projects have an
            // unrelated directory called `target`.
            "target" if root.join("Cargo.toml").exists() || path.join("CACHEDIR.TAG").exists() => {
                Some("Rust Build Cache")
            }
            ".build" => Some("Swift SPM Build Cache"),
            "DerivedData" => Some("Xcode DerivedData"),
            ".next" => Some("Next.js Build Output"),
            ".turbo" => Some("Turborepo Cache"),
            ".venv" | "venv" => Some("Python Virtualenv"),
            "__pycache__" => Some("Python Bytecode Cache"),
            ".terraform" => Some("Terraform Provider Cache"),
            "vendor" if root.join("go.mod").exists() || root.join("composer.json").exists() => {
                Some("Vendored Dependencies")
            }
            "build"
                if root.join("Package.swift").exists()
                    || root.join("project.yml").exists()
                    || root.join("CMakeLists.txt").exists() =>
            {
                Some("Build Artifacts")
            }
            _ => None,
        }
    }

    /// Flags files that commonly hold credentials. Kept in sync with
    /// `claude_permissions::SECRET_DENY_RULES`.
    pub(crate) fn secret_risk(file_name: &str) -> Option<(&'static str, &'static str)> {
        if file_name.starts_with(".env")
            && !file_name.ends_with(".example")
            && !file_name.ends_with(".sample")
            && !file_name.ends_with(".template")
        {
            return Some(("🚨 High", "Environment variable / API secret file"));
        }
        if file_name.ends_with(".pem")
            || file_name.ends_with(".key")
            || file_name.ends_with(".p12")
            || file_name.ends_with(".pfx")
            || matches!(file_name, "id_rsa" | "id_ecdsa" | "id_ed25519")
        {
            return Some(("🚨 Critical", "Private key / certificate"));
        }
        if matches!(
            file_name,
            "credentials.json" | "service-account.json" | ".netrc" | ".npmrc" | ".pypirc"
        ) {
            return Some(("🚨 High", "Credential / registry auth file"));
        }
        None
    }

    fn detect_project_types(root: &Path) -> Vec<String> {
        let mut types = Vec::new();
        let has_xcode_project = root
            .read_dir()
            .ok()
            .map(|mut d| {
                d.any(|e| {
                    e.ok()
                        .and_then(|p| {
                            p.path()
                                .extension()
                                .map(|ext| ext == "xcodeproj" || ext == "xcworkspace")
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        if root.join("Package.swift").exists()
            || root.join("project.yml").exists()
            || has_xcode_project
        {
            types.push("Swift / iOS / macOS (Xcode)".to_string());
        }
        if root.join("Cargo.toml").exists() {
            types.push("Rust (Cargo)".to_string());
        }
        if root.join("package.json").exists() {
            types.push("Node.js / TypeScript / React".to_string());
        }
        if root.join("requirements.txt").exists() || root.join("pyproject.toml").exists() {
            types.push("Python".to_string());
        }
        if root.join("go.mod").exists() {
            types.push("Go".to_string());
        }
        if types.is_empty() {
            types.push("Generic Software Workspace".to_string());
        }
        types
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::claude_permissions::{ReadDenyRules, SECRET_DENY_RULES, SettingsScope};
    use std::fs;

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentprof_guard_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_env_example_is_not_a_secret() {
        assert!(WorkspaceGuard::secret_risk(".env.example").is_none());
        assert!(WorkspaceGuard::secret_risk(".env.sample").is_none());
        assert!(WorkspaceGuard::secret_risk(".env").is_some());
        assert!(WorkspaceGuard::secret_risk(".env.production").is_some());
    }

    #[test]
    fn test_private_key_extensions_are_flagged() {
        for name in [
            "server.pem",
            "tls.key",
            "cert.p12",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
        ] {
            assert!(WorkspaceGuard::secret_risk(name).is_some(), "{}", name);
        }
        assert!(WorkspaceGuard::secret_risk("main.rs").is_none());
    }

    /// Every file name the audit flags must be covered by the rules `fix`
    /// writes, or the audit could never come back clean.
    #[test]
    fn test_deny_rules_cover_every_flagged_name() {
        let root = PathBuf::from("/w");
        let owned: Vec<String> = SECRET_DENY_RULES.iter().map(|s| s.to_string()).collect();
        let rules = ReadDenyRules::from_rules(&root, None, SettingsScope::Project, &owned);
        for name in [
            ".env",
            ".env.local",
            ".envrc",
            "a.pem",
            "a.key",
            "a.p12",
            "a.pfx",
            "id_rsa",
            "id_ecdsa",
            "id_ed25519",
            "credentials.json",
            "service-account.json",
            ".netrc",
            ".npmrc",
            ".pypirc",
        ] {
            assert!(
                WorkspaceGuard::secret_risk(name).is_some(),
                "{} not flagged",
                name
            );
            assert!(
                rules.blocks(&root.join("sub").join(name), false),
                "{} not denied",
                name
            );
        }
    }

    #[test]
    fn test_target_is_heavy_only_in_cargo_projects() {
        let dir = tempdir("target");
        fs::create_dir_all(dir.join("target")).unwrap();

        assert!(
            WorkspaceGuard::heavy_dir_type("target", &dir.join("target"), &dir).is_none(),
            "a plain `target` dir in a non-Cargo project is not a build cache"
        );

        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(
            WorkspaceGuard::heavy_dir_type("target", &dir.join("target"), &dir),
            Some("Rust Build Cache")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gitignore_comment_does_not_count_as_ignored() {
        let dir = tempdir("gi");
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        // Mentions node_modules only in a comment.
        fs::write(dir.join(".gitignore"), "# do not ignore node_modules\n").unwrap();

        let matcher = WorkspaceGuard::build_matcher(&dir, &dir.join(".gitignore"));
        assert!(
            !WorkspaceGuard::is_ignored(&matcher, &dir.join("node_modules"), true),
            "a commented mention must not count as an ignore rule"
        );

        fs::write(dir.join(".gitignore"), "node_modules/\n").unwrap();
        let matcher = WorkspaceGuard::build_matcher(&dir, &dir.join(".gitignore"));
        assert!(WorkspaceGuard::is_ignored(
            &matcher,
            &dir.join("node_modules"),
            true
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_audit_does_not_descend_into_heavy_dirs() {
        let dir = tempdir("walk");
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        // A vendored fixture key must not be reported as the user's secret.
        fs::write(dir.join("node_modules/pkg/test.pem"), "fixture").unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();

        let report = WorkspaceGuard::audit_with_home(&dir, None).unwrap();
        assert!(
            report.secret_risks.is_empty(),
            "secrets inside node_modules must not be reported: {:?}",
            report.secret_risks
        );
        assert_eq!(report.heavy_directories.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Regression: a `.claudeignore` entry used to count as protection even
    /// though Claude Code never reads that file.
    #[test]
    fn test_claudeignore_is_not_protection_but_deny_rules_are() {
        let dir = tempdir("deny");
        fs::write(dir.join(".env"), "TOKEN=x").unwrap();
        fs::write(dir.join(".claudeignore"), ".env\n").unwrap();

        let report = WorkspaceGuard::audit_with_home(&dir, None).unwrap();
        assert_eq!(report.total_exposed_secrets, 1);
        assert!(report.has_claudeignore);
        assert!(
            report
                .recommendations
                .iter()
                .any(|r| r.contains(".claudeignore"))
        );

        fs::create_dir_all(dir.join(".claude")).unwrap();
        fs::write(
            dir.join(".claude/settings.json"),
            r#"{"permissions": {"deny": ["Read(.env)"]}}"#,
        )
        .unwrap();
        let report = WorkspaceGuard::audit_with_home(&dir, None).unwrap();
        assert_eq!(report.total_exposed_secrets, 0);
        assert_eq!(report.claude_read_deny_rules, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_heavy_dirs_count_as_unignored_only_when_not_gitignored() {
        let dir = tempdir("heavy");
        fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        let report = WorkspaceGuard::audit_with_home(&dir, None).unwrap();
        assert_eq!(report.total_unignored_heavy_dirs, 1);

        fs::write(dir.join(".gitignore"), "node_modules/\n").unwrap();
        let report = WorkspaceGuard::audit_with_home(&dir, None).unwrap();
        assert_eq!(report.total_unignored_heavy_dirs, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
