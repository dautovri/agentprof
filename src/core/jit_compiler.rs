use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// Tokens always loaded regardless of which file is being edited (the
    /// modules whose pattern is `*`).
    pub always_loaded_tokens: usize,
    /// Tokens loaded only when a matching file is in play.
    pub conditional_tokens: usize,
    /// Share of the original file that becomes conditional. This is the real
    /// saving: the previous figure compared the original against the *average*
    /// module size, which reported a large reduction even when every module
    /// still loaded unconditionally.
    pub deferrable_percentage: f64,
}

pub struct JitRuleCompiler;

impl JitRuleCompiler {
    pub fn compile_monolithic_rules(workspace_root: &Path) -> Result<CompilationResult> {
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        // Find primary rule file
        let candidate = if canonical_root.join("AGENTS.md").exists() {
            canonical_root.join("AGENTS.md")
        } else if canonical_root.join("CLAUDE.md").exists() {
            canonical_root.join("CLAUDE.md")
        } else {
            anyhow::bail!(
                "No AGENTS.md or CLAUDE.md found in {}",
                canonical_root.display()
            );
        };

        let content = fs::read_to_string(&candidate)?;
        let original_tokens = crate::core::tokens::TokenCounter::count_cl100k(&content);

        let out_dir = canonical_root.join(".agentrules");
        fs::create_dir_all(&out_dir)?;

        let mut modules = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        let mut current_title = "core".to_string();
        let mut current_lines = Vec::new();
        // Headings repeat across a long rule file ("## Setup" under two parents,
        // for example). Without disambiguation each later section silently
        // overwrote the earlier module file and its rules were lost.
        let mut used_names: HashMap<String, usize> = HashMap::new();

        for line in lines {
            if line.starts_with("# ") || line.starts_with("## ") {
                if !current_lines.is_empty() {
                    let text = current_lines.join("\n");
                    let name = Self::unique_name(&current_title, &mut used_names);
                    modules.push(Self::save_module(&out_dir, &name, &text)?);
                    current_lines.clear();
                }
                current_title = Self::slugify(line);
            }
            current_lines.push(line);
        }

        if !current_lines.is_empty() {
            let text = current_lines.join("\n");
            let name = Self::unique_name(&current_title, &mut used_names);
            modules.push(Self::save_module(&out_dir, &name, &text)?);
        }

        // Generate JIT Router index
        let router_md = out_dir.join("index.md");
        let mut router_content = String::from(
            "# JIT Rule Router\n\nLoad modular rules on-demand by matched file extension:\n\n",
        );
        for m in &modules {
            router_content.push_str(&format!(
                "- **{}** (`{}`): {} tokens -> `{}`\n",
                m.name,
                m.target_file_pattern,
                m.token_count,
                m.path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        fs::write(&router_md, router_content)?;

        let always_loaded_tokens: usize = modules
            .iter()
            .filter(|m| m.target_file_pattern == "*")
            .map(|m| m.token_count)
            .sum();
        let conditional_tokens: usize = modules
            .iter()
            .filter(|m| m.target_file_pattern != "*")
            .map(|m| m.token_count)
            .sum();
        let deferrable_percentage = if original_tokens > 0 {
            (conditional_tokens as f64 / original_tokens as f64) * 100.0
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
            always_loaded_tokens,
            conditional_tokens,
            deferrable_percentage,
        })
    }

    /// Turns a markdown heading into a safe, lowercase file stem.
    ///
    /// Only a handful of characters were replaced before, so headings
    /// containing `:`, `(`, `?` or a path separator produced awkward or
    /// unwritable filenames.
    pub(crate) fn slugify(heading: &str) -> String {
        let cleaned: String = heading
            .trim_start_matches('#')
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();

        let slug = cleaned
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");

        if slug.is_empty() {
            "general".to_string()
        } else {
            slug.chars().take(60).collect()
        }
    }

    fn unique_name(base: &str, used: &mut HashMap<String, usize>) -> String {
        let count = used.entry(base.to_string()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base.to_string()
        } else {
            format!("{}-{}", base, count)
        }
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
        }
        .to_string();

        Ok(CompiledRuleModule {
            name: title.to_string(),
            token_count,
            line_count,
            target_file_pattern: pattern,
            path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_strips_unsafe_characters() {
        assert_eq!(JitRuleCompiler::slugify("## API: v2 (beta)"), "api-v2-beta");
        assert_eq!(
            JitRuleCompiler::slugify("# Swift / iOS Rules"),
            "swift-ios-rules"
        );
        assert_eq!(JitRuleCompiler::slugify("##   "), "general");
    }

    #[test]
    fn test_slugify_cannot_escape_the_output_directory() {
        let slug = JitRuleCompiler::slugify("## ../../etc/passwd");
        assert!(
            !slug.contains('/'),
            "slug must not contain a path separator"
        );
        assert!(
            !slug.contains(".."),
            "slug must not contain a parent reference"
        );
    }

    #[test]
    fn test_duplicate_headings_get_distinct_module_names() {
        let mut used = HashMap::new();
        assert_eq!(JitRuleCompiler::unique_name("setup", &mut used), "setup");
        assert_eq!(JitRuleCompiler::unique_name("setup", &mut used), "setup-2");
        assert_eq!(JitRuleCompiler::unique_name("setup", &mut used), "setup-3");
    }

    #[test]
    fn test_repeated_headings_do_not_lose_content() {
        let dir = std::env::temp_dir().join(format!("agentprof_jit_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("AGENTS.md"),
            "## Setup\nfirst rule\n\n## Setup\nsecond rule\n",
        )
        .unwrap();

        let result = JitRuleCompiler::compile_monolithic_rules(&dir).unwrap();
        assert_eq!(result.total_modules_created, 2);

        let bodies: Vec<String> = result
            .modules
            .iter()
            .map(|m| fs::read_to_string(&m.path).unwrap())
            .collect();
        assert!(bodies.iter().any(|b| b.contains("first rule")));
        assert!(bodies.iter().any(|b| b.contains("second rule")));
        let _ = fs::remove_dir_all(&dir);
    }
}
