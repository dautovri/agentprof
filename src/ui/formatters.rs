pub struct Formatters;

impl Formatters {
    pub fn format_tokens(tokens: usize) -> String {
        let s = tokens.to_string();
        let mut result = String::new();
        let len = s.len();
        for (i, c) in s.chars().enumerate() {
            result.push(c);
            if (len - 1 - i).is_multiple_of(3) && i != len - 1 {
                result.push(',');
            }
        }
        result
    }

    pub fn format_ms(ms: f64) -> String {
        format!("{:.1}ms", ms)
    }

    pub fn format_currency(usd: f64) -> String {
        format!("${:.2}", usd)
    }
}
