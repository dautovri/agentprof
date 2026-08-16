#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenModel {
    /// Anthropic Claude / GPT-4 (cl100k_base tokenizer)
    Claude,
    /// OpenAI GPT-4o / o1 / o3 (o200k_base tokenizer)
    Gpt4o,
    /// Approximation for Google Gemini
    Gemini,
}

impl TokenModel {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            TokenModel::Claude => "Claude (cl100k)",
            TokenModel::Gpt4o => "GPT-4o (o200k)",
            TokenModel::Gemini => "Gemini (est.)",
        }
    }
}

use tiktoken_rs::{cl100k_base, o200k_base};

/// Fast token counter supporting multiple tokenizer backends
pub struct TokenCounter;

impl TokenCounter {
    /// Counts tokens for a string using standard cl100k_base (Claude 3.5/3.7, GPT-4)
    pub fn count_cl100k(text: &str) -> usize {
        match cl100k_base() {
            Ok(bpe) => bpe.encode_ordinary(text).len(),
            Err(_) => text.split_whitespace().count() * 4 / 3, // fallback heuristic
        }
    }

    /// Counts tokens using o200k_base (GPT-4o, o1, o3)
    pub fn count_o200k(text: &str) -> usize {
        match o200k_base() {
            Ok(bpe) => bpe.encode_ordinary(text).len(),
            Err(_) => Self::count_cl100k(text),
        }
    }

    /// Estimate cost per 100 turns in USD at Opus / Sonnet / GPT-4o rates
    /// Assumes ~$0.015 per 1k input tokens average
    pub fn estimate_cost_per_100_turns(tokens: usize) -> f64 {
        (tokens as f64 / 1_000.0) * 0.015 * 100.0
    }

    /// Calculates token share against a target context window (e.g. 128,000 or 200,000)
    pub fn context_percentage(tokens: usize, window: usize) -> f64 {
        if window == 0 {
            return 0.0;
        }
        (tokens as f64 / window as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_counting() {
        let text = "You are an expert software engineer adhering strictly to modern Swift guidelines.";
        let tokens = TokenCounter::count_cl100k(text);
        assert!(tokens > 5 && tokens < 30);
    }

    #[test]
    fn test_context_percentage() {
        let pct = TokenCounter::context_percentage(2000, 200_000);
        assert!((pct - 1.0).abs() < 0.01);
    }
}
