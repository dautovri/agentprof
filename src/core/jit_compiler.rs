//! Splits a monolithic instruction file into path-scoped rules in the formats
//! agents actually load on demand:
//!
//! * Claude Code: `.claude/rules/*.md` with `paths:` frontmatter
//! * Cursor: `.cursor/rules/*.mdc` with `globs:` and `alwaysApply: false`
//! * GitHub Copilot: `.github/instructions/*.instructions.md` with `applyTo:`
//!
//! Only sections whose heading names a language, framework or area with a
//! clear file pattern are moved. Everything else stays in the source file,
//! which is loaded into every session anyway.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RuleTarget {
    /// Claude Code: .claude/rules/*.md
    Claude,
    /// Cursor: .cursor/rules/*.mdc
    Cursor,
    /// GitHub Copilot: .github/instructions/*.instructions.md
    Copilot,
}

impl RuleTarget {
    fn path(&self, root: &Path, slug: &str) -> PathBuf {
        match self {
            RuleTarget::Claude => root.join(".claude/rules").join(format!("{}.md", slug)),
            RuleTarget::Cursor => root.join(".cursor/rules").join(format!("{}.mdc", slug)),
            RuleTarget::Copilot => root
                .join(".github/instructions")
                .join(format!("{}.instructions.md", slug)),
        }
    }

    fn render(&self, heading: &str, globs: &[String], body: &str) -> String {
        let quoted: Vec<String> = globs.iter().map(|g| format!("\"{}\"", g)).collect();
        let front = match self {
            RuleTarget::Claude => {
                let list: String = quoted.iter().map(|g| format!("  - {}\n", g)).collect();
                format!("---\npaths:\n{}---\n", list)
            }
            RuleTarget::Cursor => format!(
                "---\ndescription: \"{}\"\nglobs: {}\nalwaysApply: false\n---\n",
                heading.replace('"', "'"),
                globs.join(",")
            ),
            RuleTarget::Copilot => format!("---\napplyTo: \"{}\"\n---\n", globs.join(",")),
        };
        format!("{}\n{}\n", front, body.trim_end())
    }

    /// Targets for agents the workspace already uses; Claude Code otherwise.
    pub fn detect(root: &Path, source: &Path) -> Vec<RuleTarget> {
        let mut targets = Vec::new();
        let source_is_claude = source
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("CLAUDE.md"));
        if source_is_claude || root.join(".claude").is_dir() {
            targets.push(RuleTarget::Claude);
        }
        if root.join(".cursor").is_dir() || root.join(".cursorrules").exists() {
            targets.push(RuleTarget::Cursor);
        }
        if root.join(".github/instructions").is_dir()
            || root.join(".github/copilot-instructions.md").exists()
        {
            targets.push(RuleTarget::Copilot);
        }
        if targets.is_empty() {
            targets.push(RuleTarget::Claude);
        }
        targets
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledRuleModule {
    pub name: String,
    pub heading: String,
    pub token_count: usize,
    pub line_count: usize,
    pub globs: Vec<String>,
    pub outputs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationResult {
    pub source_file: PathBuf,
    pub original_tokens: usize,
    pub targets: Vec<RuleTarget>,
    /// Sections moved into path-scoped rules.
    pub modules: Vec<CompiledRuleModule>,
    pub total_modules_created: usize,
    /// Tokens that remain loaded in every session once the moved sections
    /// are removed from the source file.
    pub always_loaded_tokens: usize,
    /// Tokens that load only when matching files are in play.
    pub conditional_tokens: usize,
    pub deferrable_percentage: f64,
    pub written: Vec<PathBuf>,
    /// Rule files that already existed and were left alone (see `--force`).
    pub skipped_existing: Vec<PathBuf>,
    pub dry_run: bool,
    /// True when `--apply` removed the moved sections from the source file.
    pub source_rewritten: bool,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Formats to write; detected from the workspace when empty.
    pub targets: Vec<RuleTarget>,
    pub apply: bool,
    pub dry_run: bool,
    pub force: bool,
}

/// Heading keywords and the files they cover.
const SCOPES: &[(&[&str], &[&str])] = &[
    (
        &["swift", "swiftui", "ios", "xcode", "uikit"],
        &["**/*.swift"],
    ),
    (&["rust", "cargo"], &["**/*.rs"]),
    (
        &["python", "django", "flask", "pytest", "fastapi"],
        &["**/*.py"],
    ),
    (&["typescript"], &["**/*.ts", "**/*.tsx"]),
    (
        &["javascript", "node", "nodejs"],
        &["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"],
    ),
    (
        &["react", "frontend", "ui", "components"],
        &["**/*.tsx", "**/*.jsx"],
    ),
    (
        &["css", "styling", "styles", "tailwind"],
        &["**/*.css", "**/*.scss"],
    ),
    (&["golang"], &["**/*.go"]),
    (&["java"], &["**/*.java"]),
    (&["kotlin", "android"], &["**/*.kt", "**/*.kts"]),
    (&["ruby", "rails"], &["**/*.rb"]),
    (
        &["sql", "database", "migrations"],
        &["**/*.sql", "**/migrations/**"],
    ),
    (
        &["test", "tests", "testing"],
        &["**/*test*", "**/*spec*", "**/tests/**"],
    ),
    (
        &["docker", "dockerfile"],
        &["**/Dockerfile*", "**/docker-compose*.yml"],
    ),
    (&["terraform", "infrastructure"], &["**/*.tf"]),
    (&["workflows", "github actions"], &[".github/workflows/**"]),
];

struct Section {
    heading: String,
    text: String,
}

pub struct JitRuleCompiler;

impl JitRuleCompiler {
    pub fn compile(workspace_root: &Path, options: &CompileOptions) -> Result<CompilationResult> {
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        let source = ["AGENTS.md", "CLAUDE.md"]
            .iter()
            .map(|f| canonical_root.join(f))
            .find(|p| p.exists())
            .with_context(|| {
                format!(
                    "No AGENTS.md or CLAUDE.md found in {}",
                    canonical_root.display()
                )
            })?;

        let content = fs::read_to_string(&source)?;
        let original_tokens = TokenCounter::count_cl100k(&content);
        let targets = if options.targets.is_empty() {
            RuleTarget::detect(&canonical_root, &source)
        } else {
            options.targets.clone()
        };

        let sections = Self::split_sections(&content);
        let mut used_names: HashMap<String, usize> = HashMap::new();
        let mut modules = Vec::new();
        let mut kept = String::new();
        let mut written = Vec::new();
        let mut skipped_existing = Vec::new();

        for section in &sections {
            let globs = Self::scope_for(&section.heading);
            if globs.is_empty() {
                kept.push_str(&section.text);
                continue;
            }
            // Headings repeat across a long rule file ("## Setup" under two
            // parents); without disambiguation later sections overwrote
            // earlier ones.
            let name = Self::unique_name(&Self::slugify(&section.heading), &mut used_names);
            let mut outputs = Vec::new();
            for target in &targets {
                let path = target.path(&canonical_root, &name);
                if path.exists() && !options.force {
                    skipped_existing.push(path);
                    continue;
                }
                if !options.dry_run {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let heading = section.heading.trim_start_matches('#').trim();
                    fs::write(&path, target.render(heading, &globs, &section.text))
                        .with_context(|| format!("Failed to write {}", path.display()))?;
                    written.push(path.clone());
                }
                outputs.push(path);
            }
            modules.push(CompiledRuleModule {
                name,
                heading: section.heading.clone(),
                token_count: TokenCounter::count_cl100k(&section.text),
                line_count: section.text.lines().count(),
                globs,
                outputs,
            });
        }

        let conditional_tokens: usize = modules.iter().map(|m| m.token_count).sum();
        let always_loaded_tokens = TokenCounter::count_cl100k(&kept);
        let deferrable_percentage = if original_tokens > 0 {
            (conditional_tokens as f64 / original_tokens as f64) * 100.0
        } else {
            0.0
        };

        // Only remove sections from the source once every one of them exists
        // as a rule file in every requested format.
        let all_written = modules.iter().all(|m| m.outputs.len() == targets.len());
        let mut backup = None;
        let mut source_rewritten = false;
        if options.apply && !options.dry_run && !modules.is_empty() && all_written {
            let bak = source.with_file_name(format!(
                "{}.agentprof.bak",
                source.file_name().unwrap_or_default().to_string_lossy()
            ));
            fs::copy(&source, &bak)
                .with_context(|| format!("Failed to back up {}", source.display()))?;
            fs::write(&source, kept.trim_end().to_string() + "\n")?;
            backup = Some(bak);
            source_rewritten = true;
        }

        Ok(CompilationResult {
            source_file: source,
            original_tokens,
            targets,
            total_modules_created: modules.len(),
            modules,
            always_loaded_tokens,
            conditional_tokens,
            deferrable_percentage,
            written,
            skipped_existing,
            dry_run: options.dry_run,
            source_rewritten,
            backup,
        })
    }

    /// Splits on `#` and `##` headings, outside fenced code blocks. Text before
    /// the first heading is a section with an empty heading.
    fn split_sections(content: &str) -> Vec<Section> {
        let mut sections = vec![Section {
            heading: String::new(),
            text: String::new(),
        }];
        let mut in_fence = false;
        for line in content.split_inclusive('\n') {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
            }
            if !in_fence && (line.starts_with("# ") || line.starts_with("## ")) {
                sections.push(Section {
                    heading: line.trim_end().to_string(),
                    text: String::new(),
                });
            }
            if let Some(last) = sections.last_mut() {
                last.text.push_str(line);
            }
        }
        sections.retain(|s| !s.text.is_empty());
        sections
    }

    /// File patterns a heading scopes its section to, or none.
    pub(crate) fn scope_for(heading: &str) -> Vec<String> {
        let title = heading.trim_start_matches('#').trim().to_lowercase();
        if title.is_empty() {
            return Vec::new();
        }
        let words: Vec<&str> = title
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let mut globs: Vec<String> = Vec::new();
        // "go" alone is too common a word to match inside longer headings.
        if title == "go" {
            globs.push("**/*.go".to_string());
        }
        for (keywords, patterns) in SCOPES {
            let hit = keywords.iter().any(|k| {
                if k.contains(' ') {
                    title.contains(k)
                } else {
                    words.contains(k)
                }
            });
            if hit {
                for p in *patterns {
                    if !globs.iter().any(|g| g == p) {
                        globs.push(p.to_string());
                    }
                }
            }
        }
        globs
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::frontmatter;

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentprof_jit_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const RULES: &str = "# Project rules\n- Keep PRs small.\n\n## Swift Style\n- Use @Observable.\n\n## Testing\n- Run the test suite.\n\n## Release\n- Tag from main.\n";

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
    fn test_scopes_come_from_heading_words() {
        assert_eq!(
            JitRuleCompiler::scope_for("## Swift Style"),
            vec!["**/*.swift"]
        );
        assert!(JitRuleCompiler::scope_for("## Release").is_empty());
        // "ui" is a word here, not a substring of "build" or "guide".
        assert!(JitRuleCompiler::scope_for("## Build guide").is_empty());
        assert!(JitRuleCompiler::scope_for("## Before you go").is_empty());
        assert_eq!(JitRuleCompiler::scope_for("## Go"), vec!["**/*.go"]);
    }

    #[test]
    fn test_writes_native_rule_formats_and_keeps_general_sections() {
        let dir = tempdir("native");
        fs::write(dir.join("CLAUDE.md"), RULES).unwrap();

        let options = CompileOptions {
            targets: vec![RuleTarget::Claude, RuleTarget::Cursor, RuleTarget::Copilot],
            ..Default::default()
        };
        let result = JitRuleCompiler::compile(&dir, &options).unwrap();
        assert_eq!(result.total_modules_created, 2, "{:?}", result.modules);

        let claude = fs::read_to_string(dir.join(".claude/rules/swift-style.md")).unwrap();
        let fm = frontmatter::parse(&claude).unwrap();
        assert_eq!(fm.list("paths"), vec!["**/*.swift"]);
        assert!(claude.contains("Use @Observable"));

        let cursor = fs::read_to_string(dir.join(".cursor/rules/testing.mdc")).unwrap();
        let fm = frontmatter::parse(&cursor).unwrap();
        assert!(!fm.is_true("alwaysApply"));
        assert!(fm.list("globs").contains(&"**/tests/**".to_string()));

        let copilot =
            fs::read_to_string(dir.join(".github/instructions/swift-style.instructions.md"))
                .unwrap();
        assert!(copilot.starts_with("---\napplyTo: \"**/*.swift\"\n---"));

        // The source is untouched without --apply.
        assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), RULES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_apply_moves_sections_out_of_the_source_with_a_backup() {
        let dir = tempdir("apply");
        fs::write(dir.join("CLAUDE.md"), RULES).unwrap();

        let options = CompileOptions {
            targets: vec![RuleTarget::Claude],
            apply: true,
            ..Default::default()
        };
        let result = JitRuleCompiler::compile(&dir, &options).unwrap();
        assert!(result.source_rewritten);
        let source = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(source.contains("Keep PRs small") && source.contains("Tag from main"));
        assert!(!source.contains("Use @Observable") && !source.contains("Run the test suite"));
        assert_eq!(fs::read_to_string(result.backup.unwrap()).unwrap(), RULES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_existing_rule_files_are_not_overwritten_and_block_apply() {
        let dir = tempdir("existing");
        fs::write(dir.join("CLAUDE.md"), RULES).unwrap();
        fs::create_dir_all(dir.join(".claude/rules")).unwrap();
        fs::write(dir.join(".claude/rules/testing.md"), "mine").unwrap();

        let options = CompileOptions {
            targets: vec![RuleTarget::Claude],
            apply: true,
            ..Default::default()
        };
        let result = JitRuleCompiler::compile(&dir, &options).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(".claude/rules/testing.md")).unwrap(),
            "mine"
        );
        assert_eq!(result.skipped_existing.len(), 1);
        assert!(
            !result.source_rewritten,
            "must not drop sections that were not written"
        );
        assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), RULES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_repeated_headings_do_not_lose_content() {
        let dir = tempdir("repeat");
        fs::write(
            dir.join("AGENTS.md"),
            "## Python\nfirst rule\n\n## Python\nsecond rule\n",
        )
        .unwrap();

        let options = CompileOptions {
            targets: vec![RuleTarget::Claude],
            ..Default::default()
        };
        let result = JitRuleCompiler::compile(&dir, &options).unwrap();
        assert_eq!(result.total_modules_created, 2);
        let bodies: Vec<String> = result
            .modules
            .iter()
            .map(|m| fs::read_to_string(&m.outputs[0]).unwrap())
            .collect();
        assert!(bodies.iter().any(|b| b.contains("first rule")));
        assert!(bodies.iter().any(|b| b.contains("second rule")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dry_run_writes_nothing() {
        let dir = tempdir("dry");
        fs::write(dir.join("CLAUDE.md"), RULES).unwrap();
        let options = CompileOptions {
            targets: vec![RuleTarget::Claude],
            apply: true,
            dry_run: true,
            ..Default::default()
        };
        let result = JitRuleCompiler::compile(&dir, &options).unwrap();
        assert_eq!(result.total_modules_created, 2);
        assert!(result.written.is_empty());
        assert!(!dir.join(".claude").exists());
        assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), RULES);
        let _ = fs::remove_dir_all(&dir);
    }
}
