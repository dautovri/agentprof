use std::path::Path;

pub struct Formatters;

impl Formatters {
    /// A path relative to the workspace root, or `~/`-relative to home, for
    /// display. Falls back to the full path.
    pub fn display_path(path: &Path, root: &Path, home: Option<&Path>) -> String {
        if let Ok(rel) = path.strip_prefix(root)
            && !rel.as_os_str().is_empty()
        {
            return rel.display().to_string();
        }
        if let Some(home) = home
            && let Ok(rel) = path.strip_prefix(home)
        {
            return format!("~/{}", rel.display());
        }
        path.display().to_string()
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_path_prefers_workspace_then_home() {
        let root = Path::new("/w/app");
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            Formatters::display_path(Path::new("/w/app/.claude/settings.json"), root, home),
            ".claude/settings.json"
        );
        assert_eq!(
            Formatters::display_path(Path::new("/home/me/.zshrc"), root, home),
            "~/.zshrc"
        );
        assert_eq!(
            Formatters::display_path(Path::new("/etc/x"), root, home),
            "/etc/x"
        );
    }

    #[test]
    fn test_format_tokens_groups_thousands() {
        assert_eq!(Formatters::format_tokens(1234567), "1,234,567");
        assert_eq!(Formatters::format_tokens(12), "12");
    }
}
