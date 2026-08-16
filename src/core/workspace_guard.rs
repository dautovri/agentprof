use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
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

        let claudeignore_content = fs::read_to_string(&claudeignore_path).unwrap_or_default();
        let cursorignore_content = fs::read_to_string(&cursorignore_path).unwrap_or_default();
        let gitignore_content = fs::read_to_string(&gitignore_path).unwrap_or_default();

        let detected_project_types = Self::detect_project_types(&canonical_root);

        let mut heavy_directories = Vec::new();
        let mut secret_risks = Vec::new();

        // Scan repository
        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let rel_path = path
                .strip_prefix(&canonical_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());

            if path.is_dir() {
                let heavy_type = match file_name {
                    "node_modules" => Some("Node Dependencies"),
                    "target" if path.join("debug").exists() || path.join("release").exists() => Some("Rust Build Cache"),
                    ".build" => Some("Swift SPM Build Cache"),
                    "DerivedData" => Some("Xcode DerivedData"),
                    ".xcarchive" | "build" if file_name == "build" && (canonical_root.join("Package.swift").exists() || canonical_root.join("project.yml").exists()) => Some("Build Artifacts"),
                    ".next" => Some("Next.js Build Output"),
                    ".venv" | "venv" | "__pycache__" => Some("Python Virtualenv / Cache"),
                    ".terraform" => Some("Terraform State Cache"),
                    _ => None,
                };

                if let Some(dtype) = heavy_type {
                    let file_count = WalkDir::new(path).max_depth(3).into_iter().filter_map(|e| e.ok()).count();
                    let is_in_claude = claudeignore_content.contains(file_name) || claudeignore_content.contains(&rel_path);
                    let is_in_cursor = cursorignore_content.contains(file_name) || cursorignore_content.contains(&rel_path);
                    let is_in_git = gitignore_content.contains(file_name) || gitignore_content.contains(&rel_path);

                    heavy_directories.push(HeavyDirectory {
                        path: path.to_path_buf(),
                        relative_path: rel_path.clone(),
                        estimated_files: file_count,
                        directory_type: dtype.to_string(),
                        is_ignored_by_claude: is_in_claude,
                        is_ignored_by_cursor: is_in_cursor,
                        is_ignored_by_git: is_in_git,
                    });
                }
            } else {
                // Secret Risk Check
                let secret_info = if file_name.starts_with(".env") && !file_name.ends_with(".example") {
                    Some(("🚨 High", "Environment variable / API secret file"))
                } else if file_name.ends_with(".pem") || file_name.ends_with(".key") || file_name == "id_rsa" {
                    Some(("🚨 Critical", "Private key / SSL certificate"))
                } else if file_name == "credentials.json" || file_name == "service-account.json" {
                    Some(("🚨 High", "Cloud provider credentials"))
                } else {
                    None
                };

                if let Some((risk, desc)) = secret_info {
                    let is_in_claude = claudeignore_content.contains(file_name) || claudeignore_content.contains(&rel_path);
                    secret_risks.push(SecretRiskFile {
                        path: path.to_path_buf(),
                        relative_path: rel_path,
                        risk_level: risk.to_string(),
                        description: desc.to_string(),
                        is_ignored_by_claude: is_in_claude,
                    });
                }
            }
        }

        let total_unignored_heavy_dirs = heavy_directories.iter().filter(|d| !d.is_ignored_by_claude).count();
        let total_exposed_secrets = secret_risks.iter().filter(|s| !s.is_ignored_by_claude).count();

        let mut recommendations = Vec::new();
        if !has_claudeignore {
            recommendations.push("Missing `.claudeignore`. Generate one with `agentprof fix --ignore`.".to_string());
        }
        if total_exposed_secrets > 0 {
            recommendations.push(format!("Found {} potential secret file(s) accessible to agent tools! Add them to `.claudeignore`.", total_exposed_secrets));
        }
        if total_unignored_heavy_dirs > 0 {
            recommendations.push(format!("Found {} unignored heavy directories slowing down agent search/grep tools.", total_unignored_heavy_dirs));
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

    fn detect_project_types(root: &Path) -> Vec<String> {
        let mut types = Vec::new();
        if root.join("Package.swift").exists() || root.join("project.yml").exists() || root.read_dir().ok().map(|mut d| d.any(|e| e.ok().map(|p| p.path().extension().map(|ext| ext == "xcodeproj" || ext == "xcworkspace").unwrap_or(false)).unwrap_or(false))).unwrap_or(false) {
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
