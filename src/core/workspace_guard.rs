use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavyDirectory {
    pub path: PathBuf,
    pub relative_path: String,
    pub estimated_files: usize,
    pub directory_type: String,
    pub is_ignored_by_claude: bool,
    pub is_ignored_by_cursor: bool,
    pub is_ignored_by_git: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRiskFile {
    pub path: PathBuf,
    pub relative_path: String,
    pub risk_level: String,
    pub description: String,
    pub is_ignored_by_claude: bool,
    pub is_ignored_by_git: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceAuditReport {
    pub root: PathBuf,
    pub has_claudeignore: bool,
    pub has_cursorignore: bool,
    pub has_gitignore: bool,
    pub detected_project_types: Vec<String>,
    pub heavy_directories: Vec<HeavyDirectory>,
    pub secret_risks: Vec<SecretRiskFile>,
    pub total_unignored_heavy_dirs: usize,
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
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let claudeignore_path = canonical_root.join(".claudeignore");
        let cursorignore_path = canonical_root.join(".cursorignore");
        let gitignore_path = canonical_root.join(".gitignore");

        let has_claudeignore = claudeignore_path.exists();
        let has_cursorignore = cursorignore_path.exists();
        let has_gitignore = gitignore_path.exists();

        // Real gitignore-syntax matchers. The previous implementation asked
        // whether the ignore file's *text contained* the directory name, so
        // `# not node_modules` counted as ignoring node_modules, and a rule like
        // `build/` matched any path containing "build".
        let claude_matcher = Self::build_matcher(&canonical_root, &claudeignore_path);
        let cursor_matcher = Self::build_matcher(&canonical_root, &cursorignore_path);
        let git_matcher = Self::build_matcher(&canonical_root, &gitignore_path);

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
                    is_ignored_by_claude: Self::is_ignored(&claude_matcher, path, true),
                    is_ignored_by_cursor: Self::is_ignored(&cursor_matcher, path, true),
                    is_ignored_by_git: Self::is_ignored(&git_matcher, path, true),
                });
            } else if let Some((risk, desc)) = Self::secret_risk(file_name) {
                secret_risks.push(SecretRiskFile {
                    path: path.to_path_buf(),
                    relative_path: rel_path,
                    risk_level: risk.to_string(),
                    description: desc.to_string(),
                    is_ignored_by_claude: Self::is_ignored(&claude_matcher, path, false),
                    is_ignored_by_git: Self::is_ignored(&git_matcher, path, false),
                });
            }
        }

        heavy_directories.sort_by_key(|d| std::cmp::Reverse(d.estimated_files));
        secret_risks.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        let total_unignored_heavy_dirs = heavy_directories
            .iter()
            .filter(|d| !d.is_ignored_by_claude)
            .count();
        let total_exposed_secrets = secret_risks
            .iter()
            .filter(|s| !s.is_ignored_by_claude)
            .count();

        let mut recommendations = Vec::new();
        if !has_claudeignore {
            recommendations
                .push("Missing `.claudeignore`. Generate one with `agentprof fix --ignore`.".to_string());
        }
        if total_exposed_secrets > 0 {
            recommendations.push(format!(
                "Found {} potential secret file(s) reachable by agent tools. Add them to `.claudeignore`.",
                total_exposed_secrets
            ));
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
                "Found {} unignored heavy directories slowing down agent search/grep tools.",
                total_unignored_heavy_dirs
            ));
        }

        Ok(WorkspaceAuditReport {
            root: canonical_root,
            has_claudeignore,
            has_cursorignore,
            has_gitignore,
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
            // The previous match arm was `".xcarchive" | "build" if file_name == "build" && ...`,
            // where the guard applied to both patterns — so `.xcarchive` could
            // never match and was silently dead.
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
            || file_name == "id_rsa"
            || file_name == "id_ed25519"
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

        if root.join("Package.swift").exists() || root.join("project.yml").exists() || has_xcode_project {
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
    use std::fs;

    #[test]
    fn test_env_example_is_not_a_secret() {
        assert!(WorkspaceGuard::secret_risk(".env.example").is_none());
        assert!(WorkspaceGuard::secret_risk(".env.sample").is_none());
        assert!(WorkspaceGuard::secret_risk(".env").is_some());
        assert!(WorkspaceGuard::secret_risk(".env.production").is_some());
    }

    #[test]
    fn test_private_key_extensions_are_flagged() {
        for name in ["server.pem", "tls.key", "cert.p12", "id_rsa", "id_ed25519"] {
            assert!(WorkspaceGuard::secret_risk(name).is_some(), "{}", name);
        }
        assert!(WorkspaceGuard::secret_risk("main.rs").is_none());
    }

    #[test]
    fn test_target_is_heavy_only_in_cargo_projects() {
        let dir = std::env::temp_dir().join(format!("agentprof_guard_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
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
        let dir = std::env::temp_dir().join(format!("agentprof_guard_gi_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        // Mentions node_modules only in a comment.
        fs::write(dir.join(".claudeignore"), "# do not ignore node_modules\n").unwrap();

        let matcher = WorkspaceGuard::build_matcher(&dir, &dir.join(".claudeignore"));
        assert!(
            !WorkspaceGuard::is_ignored(&matcher, &dir.join("node_modules"), true),
            "a commented mention must not count as an ignore rule"
        );

        fs::write(dir.join(".claudeignore"), "node_modules/\n").unwrap();
        let matcher = WorkspaceGuard::build_matcher(&dir, &dir.join(".claudeignore"));
        assert!(WorkspaceGuard::is_ignored(&matcher, &dir.join("node_modules"), true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_audit_does_not_descend_into_heavy_dirs() {
        let dir = std::env::temp_dir().join(format!("agentprof_guard_walk_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        // A vendored fixture key must not be reported as the user's secret.
        fs::write(dir.join("node_modules/pkg/test.pem"), "fixture").unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();

        let report = WorkspaceGuard::audit(&dir).unwrap();
        assert!(
            report.secret_risks.is_empty(),
            "secrets inside node_modules must not be reported: {:?}",
            report.secret_risks
        );
        assert_eq!(report.heavy_directories.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
