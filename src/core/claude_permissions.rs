//! Claude Code file-access rules.
//!
//! Claude Code does not read `.claudeignore`. The documented way to keep a file
//! away from the agent is a `Read(...)` rule in the `permissions.deny` list of a
//! settings file. Rules use gitignore syntax with four anchor forms — `//abs`,
//! `~/home`, `/settings-anchor` and `path` / `./path` — and a `!` rule carves
//! paths out of the rules listed before it in the same file.
//! See <https://code.claude.com/docs/en/permissions#read-and-edit>.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Deny rules `agentprof fix` writes for secret files.
///
/// Kept in sync with `WorkspaceGuard::secret_risk`, so every file the audit
/// flags is covered once the fix has run. Bare file names match at any depth.
/// The negations keep example/template env files readable, since agents need
/// them to learn which variables exist.
pub const SECRET_DENY_RULES: &[&str] = &[
    "Read(.env*)",
    "Read(!.env*.example)",
    "Read(!.env*.sample)",
    "Read(!.env*.template)",
    "Read(*.pem)",
    "Read(*.key)",
    "Read(*.p12)",
    "Read(*.pfx)",
    "Read(id_rsa)",
    "Read(id_ecdsa)",
    "Read(id_ed25519)",
    "Read(credentials.json)",
    "Read(service-account.json)",
    "Read(.netrc)",
    "Read(.npmrc)",
    "Read(.pypirc)",
];

/// Which settings file a rule came from. It decides what a `/path` rule is
/// anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsScope {
    Managed,
    User,
    Project,
    Local,
}

#[derive(Debug, Clone)]
pub struct SettingsFile {
    pub label: String,
    pub path: PathBuf,
    pub scope: SettingsScope,
}

/// Every settings file whose deny rules apply to a session started in
/// `workspace_root`, highest precedence first.
pub fn settings_files(workspace_root: &Path, home: Option<&Path>) -> Vec<SettingsFile> {
    let mut files = vec![
        SettingsFile {
            label: "managed-settings.json".to_string(),
            path: PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json"),
            scope: SettingsScope::Managed,
        },
        SettingsFile {
            label: "managed-settings.json".to_string(),
            path: PathBuf::from("/etc/claude-code/managed-settings.json"),
            scope: SettingsScope::Managed,
        },
        SettingsFile {
            label: ".claude/settings.local.json".to_string(),
            path: workspace_root.join(".claude/settings.local.json"),
            scope: SettingsScope::Local,
        },
        SettingsFile {
            label: ".claude/settings.json".to_string(),
            path: workspace_root.join(".claude/settings.json"),
            scope: SettingsScope::Project,
        },
    ];
    if let Some(home) = home {
        files.push(SettingsFile {
            label: "~/.claude/settings.json".to_string(),
            path: home.join(".claude/settings.json"),
            scope: SettingsScope::User,
        });
    }
    files
}

/// Returns the specifier of a `Read` rule: `Some(None)` for a bare `Read`
/// (every read is denied), `Some(Some(spec))` for `Read(spec)`, and `None` for
/// rules about other tools.
fn read_rule_specifier(rule: &str) -> Option<Option<&str>> {
    let rule = rule.trim();
    if rule == "Read" {
        return Some(None);
    }
    let spec = rule.strip_prefix("Read(")?.strip_suffix(')')?.trim();
    Some(Some(spec))
}

/// The `Read` deny rules in effect for one workspace, compiled to gitignore
/// matchers.
#[derive(Default)]
pub struct ReadDenyRules {
    /// One matcher per (settings file, anchor directory): a `!` rule only
    /// carves exceptions out of rules from its own file.
    matchers: Vec<(PathBuf, Gitignore)>,
    denies_everything: bool,
    /// Number of `Read` deny rules found across all settings files.
    pub rule_count: usize,
    /// Settings files that contributed at least one `Read` deny rule.
    pub sources: Vec<String>,
}

impl ReadDenyRules {
    /// Loads the rules from every settings file that applies to `workspace_root`.
    pub fn load(workspace_root: &Path, home: Option<&Path>) -> Self {
        let mut per_file = Vec::new();
        for file in settings_files(workspace_root, home) {
            let Ok(text) = fs::read_to_string(&file.path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let rules: Vec<String> = json
                .get("permissions")
                .and_then(|p| p.get("deny"))
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            per_file.push((file, rules));
        }
        Self::compile(workspace_root, home, &per_file)
    }

    /// Compiles the rules of one settings file of the given scope.
    #[cfg(test)]
    pub fn from_rules(
        workspace_root: &Path,
        home: Option<&Path>,
        scope: SettingsScope,
        rules: &[String],
    ) -> Self {
        let file = SettingsFile {
            label: "rules".to_string(),
            path: PathBuf::new(),
            scope,
        };
        Self::compile(workspace_root, home, &[(file, rules.to_vec())])
    }

    fn compile(
        workspace_root: &Path,
        home: Option<&Path>,
        per_file: &[(SettingsFile, Vec<String>)],
    ) -> Self {
        let cwd = canonical(workspace_root);
        let home = home.map(canonical);
        let mut out = ReadDenyRules::default();

        for (file, rules) in per_file {
            // `/path` resolves against the working directory for project and
            // local settings, and against ~/.claude for user settings. Managed
            // settings have no documented anchor, so such rules are skipped
            // rather than guessed at.
            let settings_anchor = match file.scope {
                SettingsScope::Project | SettingsScope::Local => Some(cwd.clone()),
                SettingsScope::User => home.as_ref().map(|h| h.join(".claude")),
                SettingsScope::Managed => None,
            };

            let mut builders: BTreeMap<PathBuf, GitignoreBuilder> = BTreeMap::new();
            let mut file_rule_count = 0;

            for rule in rules {
                let Some(spec) = read_rule_specifier(rule) else {
                    continue;
                };
                file_rule_count += 1;
                let Some(spec) = spec else {
                    out.denies_everything = true;
                    continue;
                };
                if matches!(spec, "*" | "**" | "**/*") {
                    out.denies_everything = true;
                    continue;
                }
                let Some((anchor, glob)) =
                    Self::to_glob(spec, &cwd, home.as_deref(), settings_anchor.as_deref())
                else {
                    continue;
                };
                let builder = builders
                    .entry(anchor.clone())
                    .or_insert_with(|| GitignoreBuilder::new(&anchor));
                let _ = builder.add_line(None, &glob);
            }

            if file_rule_count > 0 {
                out.rule_count += file_rule_count;
                if !out.sources.contains(&file.label) {
                    out.sources.push(file.label.clone());
                }
            }
            for (anchor, builder) in builders {
                if let Ok(matcher) = builder.build() {
                    out.matchers.push((anchor, matcher));
                }
            }
        }
        out
    }

    /// Translates a rule specifier into an anchor directory and a gitignore
    /// line relative to it.
    fn to_glob(
        spec: &str,
        cwd: &Path,
        home: Option<&Path>,
        settings_anchor: Option<&Path>,
    ) -> Option<(PathBuf, String)> {
        let (negation, body) = match spec.strip_prefix('!') {
            Some(rest) => ("!", rest),
            None => ("", spec),
        };
        if body.is_empty() {
            return None;
        }

        let (anchor, glob) = if let Some(rest) = body.strip_prefix("//") {
            (PathBuf::from("/"), anchored(rest)?)
        } else if let Some(rest) = body.strip_prefix("~/") {
            (home?.to_path_buf(), anchored(rest)?)
        } else if body.starts_with('/') {
            // A leading slash already anchors a gitignore pattern.
            (settings_anchor?.to_path_buf(), body.to_string())
        } else if let Some(rest) = body.strip_prefix("./") {
            // The docs leave open whether `./name` also matches nested copies.
            // Treating it as anchored can only under-report protection, never
            // claim a file is safe when it is not.
            (cwd.to_path_buf(), anchored(rest)?)
        } else if let Some(dir) = body.strip_suffix("/**").filter(|d| !d.contains('/')) {
            // In deny rules a single-segment directory pattern matches that
            // directory at any depth.
            (cwd.to_path_buf(), format!("**/{}/**", dir))
        } else {
            (cwd.to_path_buf(), body.to_string())
        };
        Some((anchor, format!("{}{}", negation, glob)))
    }

    /// True when a `Read` deny rule stops Claude Code from reading `path`.
    ///
    /// `path` must be absolute and canonical, like the workspace root the
    /// rules were compiled against.
    pub fn blocks(&self, path: &Path, is_dir: bool) -> bool {
        if self.denies_everything {
            return true;
        }
        self.matchers.iter().any(|(anchor, matcher)| {
            path.starts_with(anchor)
                && matcher
                    .matched_path_or_any_parents(path, is_dir)
                    .is_ignore()
        })
    }
}

fn anchored(rest: &str) -> Option<String> {
    if rest.is_empty() {
        None
    } else if rest.starts_with("**") {
        Some(rest.to_string())
    } else {
        Some(format!("/{}", rest))
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(root: &Path, list: &[&str]) -> ReadDenyRules {
        let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        ReadDenyRules::from_rules(root, None, SettingsScope::Project, &owned)
    }

    fn root() -> PathBuf {
        PathBuf::from("/work/repo")
    }

    #[test]
    fn test_bare_names_match_at_any_depth() {
        let r = rules(&root(), &["Read(.env)"]);
        assert!(r.blocks(&root().join(".env"), false));
        assert!(r.blocks(&root().join("apps/web/.env"), false));
        assert!(!r.blocks(&root().join(".env.local"), false));
    }

    #[test]
    fn test_secret_rules_block_what_the_audit_flags_and_spare_examples() {
        let r = rules(&root(), SECRET_DENY_RULES);
        for secret in [
            ".env",
            ".env.production",
            "svc/.env.local",
            ".envrc",
            "certs/server.pem",
            "tls.key",
            "id_ed25519",
            "gcp/service-account.json",
            ".npmrc",
        ] {
            assert!(
                r.blocks(&root().join(secret), false),
                "{} not blocked",
                secret
            );
        }
        for readable in [
            ".env.example",
            "api/.env.local.example",
            ".env.sample",
            "src/main.rs",
        ] {
            assert!(
                !r.blocks(&root().join(readable), false),
                "{} blocked",
                readable
            );
        }
    }

    #[test]
    fn test_negation_listed_first_carves_nothing() {
        let r = rules(&root(), &["Read(!.env.example)", "Read(.env*)"]);
        assert!(r.blocks(&root().join(".env.example"), false));
    }

    #[test]
    fn test_directory_rules() {
        let r = rules(&root(), &["Read(secrets/**)"]);
        assert!(r.blocks(&root().join("secrets/a.txt"), false));
        assert!(r.blocks(&root().join("deploy/secrets/b.txt"), false));
        assert!(!r.blocks(&root().join("src/secrets.rs"), false));

        let anchored = rules(&root(), &["Read(/config/**)"]);
        assert!(anchored.blocks(&root().join("config/prod.yml"), false));
        assert!(!anchored.blocks(&root().join("pkg/config/prod.yml"), false));
    }

    #[test]
    fn test_dot_slash_rules_are_treated_as_anchored() {
        let r = rules(&root(), &["Read(./.env)"]);
        assert!(r.blocks(&root().join(".env"), false));
        // Conservative: a nested copy is reported as unprotected.
        assert!(!r.blocks(&root().join("sub/.env"), false));
    }

    #[test]
    fn test_absolute_and_home_rules() {
        let owned = vec!["Read(//**/.env)".to_string(), "Read(~/work/**)".to_string()];
        let r = ReadDenyRules::from_rules(
            &root(),
            Some(Path::new("/home/me")),
            SettingsScope::User,
            &owned,
        );
        assert!(r.blocks(&root().join("x/.env"), false));
        assert!(r.blocks(Path::new("/home/me/work/notes.txt"), false));
        assert!(!r.blocks(Path::new("/home/me/other/notes.txt"), false));
    }

    #[test]
    fn test_user_scope_slash_rules_anchor_at_dot_claude() {
        let owned = vec!["Read(/secrets/**)".to_string()];
        let r = ReadDenyRules::from_rules(
            &root(),
            Some(Path::new("/home/me")),
            SettingsScope::User,
            &owned,
        );
        assert!(r.blocks(Path::new("/home/me/.claude/secrets/x"), false));
        assert!(!r.blocks(&root().join("secrets/x"), false));
    }

    #[test]
    fn test_bare_read_denies_everything_and_other_tools_are_ignored() {
        assert!(rules(&root(), &["Read"]).blocks(&root().join("a.txt"), false));
        let r = rules(&root(), &["Edit(.env)", "Bash(cat .env)"]);
        assert_eq!(r.rule_count, 0);
        assert!(!r.blocks(&root().join(".env"), false));
    }

    #[test]
    fn test_load_reads_project_and_local_settings() {
        let dir = std::env::temp_dir().join(format!("agentprof_perm_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        fs::write(
            dir.join(".claude/settings.json"),
            r#"{"permissions": {"deny": ["Read(.env)"]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/settings.local.json"),
            r#"{"permissions": {"deny": ["Read(*.pem)"], "allow": ["Read(.env)"]}}"#,
        )
        .unwrap();

        let r = ReadDenyRules::load(&dir, None);
        let root = dir.canonicalize().unwrap();
        assert_eq!(r.rule_count, 2);
        // Deny beats allow regardless of which file the allow is in.
        assert!(r.blocks(&root.join(".env"), false));
        assert!(r.blocks(&root.join("k/tls.pem"), false));
        let _ = fs::remove_dir_all(&dir);
    }
}
