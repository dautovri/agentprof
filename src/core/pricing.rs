//! Anthropic API list prices, used to turn transcript token usage into an
//! API-equivalent cost.
//!
//! Source: <https://platform.claude.com/docs/en/about-claude/pricing>
//! (retrieved 2026-09-24). Update the table when prices change; unknown
//! models are reported as unpriced rather than guessed.

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    /// Cache hits bill at this fraction of the input price.
    pub cache_read_multiplier: f64,
}

/// Cache writes bill at 1.25x input (5-minute TTL) or 2x input (1-hour TTL).
pub const CACHE_WRITE_5M_MULTIPLIER: f64 = 1.25;
pub const CACHE_WRITE_1H_MULTIPLIER: f64 = 2.0;

const fn price(input: f64, output: f64, cache_read_multiplier: f64) -> ModelPrice {
    ModelPrice {
        input,
        output,
        cache_read_multiplier,
    }
}

/// Keyed by the model id with any date, platform or context suffix removed
/// (see [`normalize_model_id`]).
const PRICES: &[(&str, ModelPrice)] = &[
    ("claude-fable-5-1", price(10.0, 50.0, 0.025)),
    ("claude-mythos-5-1", price(10.0, 50.0, 0.025)),
    ("claude-fable-5", price(10.0, 50.0, 0.1)),
    ("claude-mythos-5", price(10.0, 50.0, 0.1)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.05)),
    ("claude-opus-5", price(5.0, 25.0, 0.1)),
    ("claude-opus-4-8", price(5.0, 25.0, 0.1)),
    ("claude-opus-4-7", price(5.0, 25.0, 0.1)),
    ("claude-opus-4-6", price(5.0, 25.0, 0.1)),
    ("claude-opus-4-5", price(5.0, 25.0, 0.1)),
    ("claude-opus-4-1", price(15.0, 75.0, 0.1)),
    ("claude-opus-4-0", price(15.0, 75.0, 0.1)),
    ("claude-opus-4", price(15.0, 75.0, 0.1)),
    ("claude-3-opus", price(15.0, 75.0, 0.1)),
    ("claude-sonnet-5", price(2.0, 10.0, 0.1)),
    ("claude-sonnet-4-6", price(3.0, 15.0, 0.1)),
    ("claude-sonnet-4-5", price(3.0, 15.0, 0.1)),
    ("claude-sonnet-4-0", price(3.0, 15.0, 0.1)),
    ("claude-sonnet-4", price(3.0, 15.0, 0.1)),
    ("claude-3-7-sonnet", price(3.0, 15.0, 0.1)),
    ("claude-3-5-sonnet", price(3.0, 15.0, 0.1)),
    ("claude-haiku-4-5", price(1.0, 5.0, 0.1)),
    ("claude-3-5-haiku", price(0.8, 4.0, 0.1)),
    ("claude-3-haiku", price(0.25, 1.25, 0.1)),
];

/// Fast-mode input/output prices for the models that support it.
const FAST_MODE_PRICES: &[(&str, f64, f64)] = &[
    ("claude-opus-5-5", 8.0, 40.0),
    ("claude-opus-5", 10.0, 50.0),
    ("claude-opus-4-8", 10.0, 50.0),
];

/// Reference model for "cost of fixed context" estimates, where no model is
/// known: Claude Sonnet 5.
pub const REFERENCE_MODEL: &str = "claude-sonnet-5";

/// Strips the parts of a model id that do not change its price: platform
/// prefixes (`us.anthropic.`), Vertex `@date` and Bedrock `-v1:0` suffixes,
/// Claude Code's `[1m]` context marker and `-YYYYMMDD` snapshot dates.
pub fn normalize_model_id(model: &str) -> String {
    let mut id = model.trim().to_ascii_lowercase();
    if let Some(pos) = id.rfind("anthropic.") {
        id = id[pos + "anthropic.".len()..].to_string();
    }
    for sep in ['@', '[', ':'] {
        if let Some(pos) = id.find(sep) {
            id.truncate(pos);
        }
    }
    if let Some(pos) = id.rfind("-v")
        && id[pos + 2..].chars().all(|c| c.is_ascii_digit())
        && pos + 2 < id.len()
    {
        id.truncate(pos);
    }
    if let Some(pos) = id.rfind('-') {
        let tail = &id[pos + 1..];
        if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) {
            id.truncate(pos);
        }
    }
    id
}

/// List price for a model, or `None` when the model is unknown.
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let id = normalize_model_id(model);
    PRICES.iter().find(|(key, _)| *key == id).map(|(_, p)| *p)
}

/// Price for a request, applying fast-mode pricing when `speed` is `"fast"`.
pub fn price_for_request(model: &str, fast: bool) -> Option<ModelPrice> {
    let base = price_for(model)?;
    if !fast {
        return Some(base);
    }
    let id = normalize_model_id(model);
    Some(
        FAST_MODE_PRICES
            .iter()
            .find(|(key, _, _)| *key == id)
            .map(|(_, input, output)| ModelPrice {
                input: *input,
                output: *output,
                ..base
            })
            .unwrap_or(base),
    )
}

pub fn reference_price() -> ModelPrice {
    price_for(REFERENCE_MODEL).expect("reference model is in the price table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalizes_platform_and_date_suffixes() {
        for (raw, expected) in [
            ("claude-sonnet-4-5-20250929", "claude-sonnet-4-5"),
            ("claude-opus-4-20250514", "claude-opus-4"),
            ("claude-haiku-4-5-20251001", "claude-haiku-4-5"),
            (
                "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                "claude-sonnet-4-5",
            ),
            ("anthropic.claude-opus-5", "claude-opus-5"),
            ("claude-opus-4-5@20251101", "claude-opus-4-5"),
            ("claude-sonnet-4-5[1m]", "claude-sonnet-4-5"),
            ("Claude-Opus-4-8", "claude-opus-4-8"),
            ("claude-opus-5-5", "claude-opus-5-5"),
        ] {
            assert_eq!(normalize_model_id(raw), expected, "{}", raw);
        }
    }

    #[test]
    fn test_prices_distinguish_similar_ids() {
        assert_eq!(price_for("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(price_for("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(price_for("claude-opus-4-1-20250805").unwrap().input, 15.0);
        assert_eq!(price_for("claude-opus-4-5-20251101").unwrap().input, 5.0);
        assert_eq!(
            price_for("claude-fable-5-1").unwrap().cache_read_multiplier,
            0.025
        );
        assert_eq!(price_for("claude-haiku-4-5-20251001").unwrap().output, 5.0);
    }

    #[test]
    fn test_unknown_models_are_unpriced() {
        assert!(price_for("<synthetic>").is_none());
        assert!(price_for("gpt-5").is_none());
    }

    #[test]
    fn test_fast_mode_only_changes_supported_models() {
        assert_eq!(
            price_for_request("claude-opus-5", true).unwrap().input,
            10.0
        );
        assert_eq!(
            price_for_request("claude-opus-5", false).unwrap().input,
            5.0
        );
        assert_eq!(
            price_for_request("claude-sonnet-5", true).unwrap().input,
            2.0
        );
    }
}
