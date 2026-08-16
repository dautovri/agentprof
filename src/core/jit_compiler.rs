use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledRuleModule {
    pub name: String,
    pub token_count: usize,
    pub line_count: usize,
    pub target_file_pattern: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationResult {
    pub source_file: PathBuf,
    pub original_tokens: usize,
    pub total_modules_created: usize,
    pub output_directory: PathBuf,
    pub modules: Vec<CompiledRuleModule>,
    pub token_savings_percentage: f64,
}

pub struct JitRuleCompiler;

impl JitRuleCompiler {
    pub fn compile_monolithic_rules(workspace_root: &Path) -> Result<CompilationResult> {
        let canonical_root = workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());

        // Find primary rule file
        let candidate = if canonical_root.join("AGENTS.md").exists() {
            canonical_root.join("AGENTS.md")
        } else if canonical_root.join("CLAUDE.md").exists() {
            canonical_root.join("CLAUDE.md")
        } else {
            anyhow::bail!("No AGENTS.md or CLAUDE.md found in {}", canonical_root.display());
        };

        let content = fs::read_to_string(&candidate)?;
        let original_tokens = crate::core::tokens::TokenCounter::count_cl100k(&content);

        let out_dir = canonical_root.join(".agentrules");
        fs::create_dir_all(&out_dir)?;

        let mut modules = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        let mut current_title = "core".to_string();
        let mut current_lines = Vec::new();

        for line in lines {
            if line.starts_with("# ") || line.starts_with("## ") {
                if !current_lines.is_empty() {
                    let text = current_lines.join("\n");
                    let mod_file = Self::save_module(&out_dir, &current_title, &text)?;
                    modules.push(mod_file);
                    current_lines.clear();
                }
                let clean_title = line.trim_start_matches('#').trim().to_lowercase()
                    .replace(' ', "-")
                    .replace('/', "-")
                    .replace('&', "and");
                current_title = if clean_title.is_empty() { "general".to_string() } else { clean_title };
            }
            current_lines.push(line);
        }

        if !current_lines.is_empty() {
            let text = current_lines.join("\n");
            let mod_file = Self::save_module(&out_dir, &current_title, &text)?;
            modules.push(mod_file);
        }

        // Generate JIT Router index
        let router_md = out_dir.join("index.md");
        let mut router_content = String::from("# JIT Rule Router\n\nLoad modular rules on-demand by matched file extension:\n\n");
        for m in &modules {
            router_content.push_str(&format!("- **{}** (`{}`): {} tokens -> `{}`\n", m.name, m.target_file_pattern, m.token_count, m.path.file_name().unwrap_or_default().to_string_lossy()));
        }
        fs::write(&router_md, router_content)?;

        let avg_module_tokens = if !modules.is_empty() {
            modules.iter().map(|m| m.token_count).sum::<usize>() as f64 / modules.len() as f64
        } else {
            0.0
        };

        let token_savings_percentage = if original_tokens > 0 {
            ((original_tokens as f64 - avg_module_tokens) / original_tokens as f64) * 100.0
        } else {
            0.0
        };

        let total_modules_created = modules.len();

        Ok(CompilationResult {
            source_file: candidate,
            original_tokens,
            total_modules_created,
            output_directory: out_dir,
            modules,
            token_savings_percentage,
        })
    }

    fn save_module(out_dir: &Path, title: &str, content: &str) -> Result<CompiledRuleModule> {
        let filename = format!("{}.md", title);
        let path = out_dir.join(&filename);
        fs::write(&path, content)?;

        let token_count = crate::core::tokens::TokenCounter::count_cl100k(content);
        let line_count = content.lines().count();

        let pattern = if title.contains("swift") {
            "**/*.swift"
        } else if title.contains("rust") {
            "**/*.rs"
        } else if title.contains("python") {
            "**/*.py"
        } else if title.contains("react") || title.contains("ui") {
            "**/*.{tsx,jsx,css}"
        } else {
            "*"
        }.to_string();

        Ok(CompiledRuleModule {
            name: title.to_string(),
            token_count,
            line_count,
            target_file_pattern: pattern,
            path,
        })
    }
}
