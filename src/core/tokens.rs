use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

use crate::core::pricing::{self, CACHE_WRITE_5M_MULTIPLIER};

/// How fixed-context costs are estimated, for display next to the figures.
pub const COST_BASIS: &str =
    "Claude Sonnet 5 list price; fixed context cached as 1 write + 99 reads per 100 turns";

/// Fast token counter supporting multiple tokenizer backends.
///
/// These are OpenAI tokenizers. Claude's tokenizer is not public, and Claude
/// typically counts more tokens for the same text (Anthropic notes that Claude
/// 4.7 and later count roughly 30% more than earlier Claude models), so every
/// count here is an approximation for sizing, not an exact Claude figure.
///
/// Both tokenizers are process-wide singletons: building a `CoreBPE` parses a
/// ~1.7MB merge table, so re-building it per file made large audits ~100x
/// slower than the actual counting work.
pub struct TokenCounter;

impl TokenCounter {
    /// Counts tokens using cl100k_base.
    pub fn count_cl100k(text: &str) -> usize {
        cl100k_base_singleton().lock().encode_ordinary(text).len()
    }

    /// Counts tokens using o200k_base (GPT-4o and later OpenAI models).
    pub fn count_o200k(text: &str) -> usize {
        o200k_base_singleton().lock().encode_ordinary(text).len()
    }

    /// Cost in USD of carrying `tokens` of fixed context (instructions, tool
    /// schemas) through 100 agent turns at the reference model's list price.
    ///
    /// Agents cache their fixed prefix, so it bills as one 5-minute cache write
    /// followed by 99 cache reads. Pricing all 100 turns as full-price input
    /// overstated the cost roughly ninefold.
    pub fn estimate_cost_per_100_turns(tokens: usize) -> f64 {
        let p = pricing::reference_price();
        let mtok = tokens as f64 / 1_000_000.0;
        mtok * p.input * (CACHE_WRITE_5M_MULTIPLIER + 99.0 * p.cache_read_multiplier)
    }

    /// Cost in USD of one turn's worth of cached fixed context.
    pub fn cached_cost_per_turn(tokens: usize) -> f64 {
        let p = pricing::reference_price();
        tokens as f64 / 1_000_000.0 * p.input * p.cache_read_multiplier
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
        let text =
            "You are an expert software engineer adhering strictly to modern Swift guidelines.";
        let tokens = TokenCounter::count_cl100k(text);
        assert!(tokens > 5 && tokens < 30);
    }

    #[test]
    fn test_context_percentage() {
        let pct = TokenCounter::context_percentage(2000, 200_000);
        assert!((pct - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_empty_text_counts_zero() {
        assert_eq!(TokenCounter::count_cl100k(""), 0);
        assert_eq!(TokenCounter::count_o200k(""), 0);
    }

    #[test]
    fn test_zero_window_does_not_divide_by_zero() {
        assert_eq!(TokenCounter::context_percentage(1000, 0), 0.0);
    }

    #[test]
    fn test_fixed_context_cost_is_cache_aware() {
        // 1M tokens at $2/MTok: one 1.25x write plus 99 reads at 0.1x.
        let cost = TokenCounter::estimate_cost_per_100_turns(1_000_000);
        assert!((cost - 2.0 * (1.25 + 9.9)).abs() < 1e-9);
        assert!((TokenCounter::cached_cost_per_turn(1_000_000) - 0.2).abs() < 1e-9);
    }

    #[test]
    fn test_tokenizer_is_reused_and_stable() {
        let a = TokenCounter::count_cl100k("agentprof profiles agent workspaces");
        let b = TokenCounter::count_cl100k("agentprof profiles agent workspaces");
        assert_eq!(a, b);
    }
}
