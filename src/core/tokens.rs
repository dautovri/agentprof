use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

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

/// Published API list prices, in USD per 1M tokens.
///
/// Fixed instruction/schema payloads are re-sent as *input* on every turn, so
/// input pricing is what any "cost of context" figure must be based on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    pub label: &'static str,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl Pricing {
    /// Default reference model for cost estimates (Claude Sonnet class).
    pub const DEFAULT: Pricing = Pricing {
        label: "Claude Sonnet (input $3/Mtok)",
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
    };

    pub fn input_cost(&self, tokens: usize) -> f64 {
        (tokens as f64 / 1_000_000.0) * self.input_per_mtok
    }

    pub fn output_cost(&self, tokens: usize) -> f64 {
        (tokens as f64 / 1_000_000.0) * self.output_per_mtok
    }
}

/// Fast token counter supporting multiple tokenizer backends.
///
/// Both tokenizers are process-wide singletons: building a `CoreBPE` parses a
/// ~1.7MB merge table, so re-building it per file made large audits ~100x
/// slower than the actual counting work.
pub struct TokenCounter;

impl TokenCounter {
    /// Counts tokens using cl100k_base (Claude 3.x/4.x approximation, GPT-4).
    pub fn count_cl100k(text: &str) -> usize {
        cl100k_base_singleton().lock().encode_ordinary(text).len()
    }

    /// Counts tokens using o200k_base (GPT-4o, o1, o3).
    pub fn count_o200k(text: &str) -> usize {
        o200k_base_singleton().lock().encode_ordinary(text).len()
    }

    /// Cost in USD of re-sending `tokens` of fixed context as input for 100 turns.
    pub fn estimate_cost_per_100_turns(tokens: usize) -> f64 {
        Pricing::DEFAULT.input_cost(tokens) * 100.0
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
    fn test_cost_uses_input_pricing() {
        // 10k fixed tokens re-sent 100 times = 1M input tokens = one input-Mtok charge.
        let cost = TokenCounter::estimate_cost_per_100_turns(10_000);
        assert!((cost - Pricing::DEFAULT.input_per_mtok).abs() < 1e-9);
    }

    #[test]
    fn test_tokenizer_is_reused_and_stable() {
        let a = TokenCounter::count_cl100k("agentprof profiles agent workspaces");
        let b = TokenCounter::count_cl100k("agentprof profiles agent workspaces");
        assert_eq!(a, b);
    }
}
