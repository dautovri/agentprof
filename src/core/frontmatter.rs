//! A small reader for the YAML frontmatter used by agent instruction files
//! (`SKILL.md`, Cursor `.mdc`, Claude Code `.claude/rules`, Copilot
//! `.instructions.md`).
//!
//! It handles the shapes these files use in practice — plain, quoted and
//! multi-line scalars, `|` / `>` block scalars, and inline or block lists —
//! without pulling in a full YAML implementation.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Scalar(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frontmatter {
    fields: Vec<(String, Value)>,
}

impl Frontmatter {
    /// The value of a scalar key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.iter().find_map(|(k, v)| match v {
            Value::Scalar(s) if k == key => Some(s.as_str()),
            _ => None,
        })
    }

    /// A key's values: the items of a list, or a scalar split on commas.
    pub fn list(&self, key: &str) -> Vec<String> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| match v {
                Value::List(items) => items.clone(),
                Value::Scalar(s) => s
                    .split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect(),
            })
            .unwrap_or_default()
    }

    pub fn is_true(&self, key: &str) -> bool {
        self.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
    }
}

/// Parses the frontmatter block at the top of `content`, if there is one.
pub fn parse(content: &str) -> Option<Frontmatter> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let block: Vec<&str> = lines
        .by_ref()
        .take_while(|l| {
            let t = l.trim_end();
            t != "---" && t != "..."
        })
        .collect();

    let mut fields = Vec::new();
    let mut i = 0;
    while i < block.len() {
        let line = block[i];
        i += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') || indent(line) > 0 {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let rest = rest.trim();

        // Lines indented under this key belong to its value.
        let start = i;
        while i < block.len() && (block[i].trim().is_empty() || indent(block[i]) > 0) {
            i += 1;
        }
        let nested = &block[start..i];

        let value = if rest.is_empty() {
            let items: Vec<String> = nested
                .iter()
                .filter_map(|l| l.trim().strip_prefix('-'))
                .map(|item| unquote(item.trim()))
                .collect();
            if items.is_empty() {
                Value::Scalar(String::new())
            } else {
                Value::List(items)
            }
        } else if rest.starts_with('|') || rest.starts_with('>') {
            Value::Scalar(block_scalar(nested, rest.starts_with('>')))
        } else if rest.starts_with('[') {
            let inner = rest.trim_start_matches('[').trim_end_matches(']');
            Value::List(
                inner
                    .split(',')
                    .map(|p| unquote(p.trim()))
                    .filter(|p| !p.is_empty())
                    .collect(),
            )
        } else if rest.starts_with('"') || rest.starts_with('\'') {
            let joined = std::iter::once(rest)
                .chain(nested.iter().map(|l| l.trim()))
                .collect::<Vec<_>>()
                .join(" ");
            Value::Scalar(unquote(&joined))
        } else {
            let plain = strip_comment(rest);
            let continued = std::iter::once(plain)
                .chain(nested.iter().map(|l| l.trim()).filter(|l| !l.is_empty()))
                .collect::<Vec<_>>()
                .join(" ");
            Value::Scalar(continued)
        };
        fields.push((key, value));
    }
    Some(Frontmatter { fields })
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `|` keeps line breaks; `>` folds them into spaces (blank lines stay breaks).
fn block_scalar(lines: &[&str], folded: bool) -> String {
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    let body: Vec<&str> = lines
        .iter()
        .map(|l| {
            if l.len() >= min_indent {
                &l[min_indent..]
            } else {
                ""
            }
        })
        .collect();
    let text = if folded {
        let mut out = String::new();
        for line in &body {
            if line.trim().is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(line.trim_end());
            }
        }
        out
    } else {
        body.join("\n")
    };
    text.trim().to_string()
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return v[1..v.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\\\", "\\");
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    v.to_string()
}

/// Drops a trailing ` # comment` from a plain scalar.
fn strip_comment(value: &str) -> &str {
    match value.find(" #") {
        Some(i) => value[..i].trim_end(),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_and_quoted_scalars() {
        let fm = parse(
            "---\nname: demo\ndescription: \"Audits: app store\" \nversion: 1 # note\n---\nbody",
        )
        .unwrap();
        assert_eq!(fm.get("name"), Some("demo"));
        assert_eq!(fm.get("description"), Some("Audits: app store"));
        assert_eq!(fm.get("version"), Some("1"));
    }

    #[test]
    fn test_folded_and_literal_block_scalars() {
        let fm = parse(
            "---\ndescription: >\n  Reviews pull requests\n  for security issues.\nnotes: |-\n  line one\n  line two\nname: x\n---\n",
        )
        .unwrap();
        assert_eq!(
            fm.get("description"),
            Some("Reviews pull requests for security issues.")
        );
        assert_eq!(fm.get("notes"), Some("line one\nline two"));
        assert_eq!(fm.get("name"), Some("x"));
    }

    #[test]
    fn test_plain_scalar_continuation_lines() {
        let fm = parse("---\ndescription: Deploys the app\n  and tags a release\n---\n").unwrap();
        assert_eq!(
            fm.get("description"),
            Some("Deploys the app and tags a release")
        );
    }

    #[test]
    fn test_lists() {
        let fm = parse("---\npaths:\n  - \"src/**/*.ts\"\n  - lib/**\nglobs: [\"*.rs\", '*.toml']\napplyTo: \"**/*.py,**/*.pyi\"\n---\n").unwrap();
        assert_eq!(fm.list("paths"), vec!["src/**/*.ts", "lib/**"]);
        assert_eq!(fm.list("globs"), vec!["*.rs", "*.toml"]);
        assert_eq!(fm.list("applyTo"), vec!["**/*.py", "**/*.pyi"]);
    }

    #[test]
    fn test_booleans_and_missing_frontmatter() {
        let fm = parse("---\nalwaysApply: true\n---\n").unwrap();
        assert!(fm.is_true("alwaysApply"));
        assert!(!fm.is_true("missing"));
        assert!(parse("# No frontmatter\n").is_none());
    }
}
