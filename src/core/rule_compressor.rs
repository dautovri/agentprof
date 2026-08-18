use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::core::tokens::TokenCounter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionReport {
    pub source_path: PathBuf,
    pub original_tokens: usize,
    pub original_lines: usize,
    pub compressed_tokens: usize,
    pub compressed_lines: usize,
    pub tokens_saved: usize,
    pub savings_percentage: f64,
    pub compressed_content: String,
}

pub struct RuleCompressor;

impl RuleCompressor {
    pub fn compress_file(file_path: &Path) -> Result<CompressionReport> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;

        let original_tokens = TokenCounter::count_cl100k(&content);
        let original_lines = content.lines().count();

        let compressed_content = Self::compress_text(&content);
        let compressed_tokens = TokenCounter::count_cl100k(&compressed_content);
        let compressed_lines = compressed_content.lines().count();

        let tokens_saved = original_tokens.saturating_sub(compressed_tokens);
        let savings_percentage = if original_tokens > 0 {
            (tokens_saved as f64 / original_tokens as f64) * 100.0
        } else {
            0.0
        };

        Ok(CompressionReport {
            source_path: file_path.to_path_buf(),
            original_tokens,
            original_lines,
            compressed_tokens,
            compressed_lines,
            tokens_saved,
            savings_percentage,
            compressed_content,
        })
    }

    pub fn compress_text(text: &str) -> String {
        let mut lines = Vec::new();

        // Regex patterns for conversational and boilerplate filler
        let filler_patterns = [
            (Regex::new(r"(?i)^(please\s+|make\s+sure\s+to\s+|always\s+remember\s+to\s+|it\s+is\s+critical\s+that\s+you\s+|you\s+must\s+always\s+)").unwrap(), ""),
            (Regex::new(r"(?i)\b(in\s+order\s+to)\b").unwrap(), "to"),
            (Regex::new(r"(?i)\b(as\s+a\s+senior\s+software\s+engineer[^,\.\n]*,?\s*)").unwrap(), ""),
            (Regex::new(r"(?i)\b(strictly\s+adhere\s+to)\b").unwrap(), "follow"),
            (Regex::new(r"(?i)\b(at\s+all\s+times)\b").unwrap(), ""),
            (Regex::new(r"(?i)\b(for\s+the\s+purpose\s+of)\b").unwrap(), "for"),
        ];

        let mut in_code_block = false;
        let mut prev_was_blank = false;

        for raw_line in text.lines() {
            let trimmed = raw_line.trim();

            if trimmed.starts_with("```") {
                in_code_block = !in_code_block;
                lines.push(raw_line.to_string());
                prev_was_blank = false;
                continue;
            }

            if in_code_block {
                lines.push(raw_line.to_string());
                continue;
            }

            if trimmed.is_empty() {
                if !prev_was_blank {
                    lines.push(String::new());
                    prev_was_blank = true;
                }
                continue;
            }
            prev_was_blank = false;

            let mut processed = raw_line.to_string();
            for (re, replacement) in &filler_patterns {
                processed = re.replace_all(&processed, *replacement).to_string();
            }

            lines.push(processed);
        }

        lines.join("\n")
    }

    /// Writes the already-computed report, so the file is compressed once.
    ///
    /// The command previously called `compress_file` and then
    /// `save_compressed_file`, which compressed and re-tokenized the whole file
    /// a second time purely to write it out.
    pub fn save_report(report: &CompressionReport, overwrite: bool) -> Result<PathBuf> {
        let file_path = report.source_path.as_path();

        let target_path = if overwrite {
            // Back up first, never clobbering an earlier backup.
            let mut bak = file_path.with_extension("bak");
            let mut n = 1;
            while bak.exists() {
                bak = file_path.with_extension(format!("bak.{}", n));
                n += 1;
            }
            fs::copy(file_path, &bak)
                .with_context(|| format!("Failed to back up {}", file_path.display()))?;
            file_path.to_path_buf()
        } else {
            let file_stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("rules");
            let ext = file_path.extension().and_then(|s| s.to_str()).unwrap_or("md");
            file_path.with_file_name(format!("{}.compressed.{}", file_stem, ext))
        };

        fs::write(&target_path, &report.compressed_content)?;
        Ok(target_path)
    }
}
