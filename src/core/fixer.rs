use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::claude_permissions::SECRET_DENY_RULES;

/// Unique marker identifying an agentprof-managed block.
///
/// Detection keys off this string alone. Matching loose tokens like
/// `CLAUDE_CODE` made any rc file that merely mentioned the variable look like
/// it already had a guard installed.
pub const FAST_PATH_SENTINEL: &str = "# >>> agentprof fast-path >>>";
pub const FAST_PATH_END: &str = "# <<< agentprof fast-path <<<";
const IGNORE_BLOCK_START: &str = "# >>> agentprof managed >>>";
const IGNORE_BLOCK_END: &str = "# <<< agentprof managed <<<";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixOutcome {
    Created,
    Updated,
    AlreadyApplied,
    /// Nothing was written because this was a dry run.
    WouldChange,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixAction {
    pub target: PathBuf,
    pub outcome: FixOutcome,
    pub detail: String,
    pub backup: Option<PathBuf>,
}

pub struct FixerEngine;

/// Patterns agentprof manages inside `.cursorignore`.
///
/// The secret patterns mirror `claude_permissions::SECRET_DENY_RULES`. Core
/// dumps are matched as `core.<pid>` only: the earlier `core.*` also hid
/// ordinary source files such as `src/core.ts` from the agent.
const IGNORE_ENTRIES: &[(&str, &[&str])] = &[
    (
        "Secrets & sensitive files",
        &[
            ".env*",
            "!.env*.example",
            "!.env*.sample",
            "!.env*.template",
            "*.pem",
            "*.key",
            "*.p12",
            "*.pfx",
            "id_rsa",
            "id_ecdsa",
            "id_ed25519",
            "credentials.json",
            "service-account.json",
            ".netrc",
            ".npmrc",
            ".pypirc",
        ],
    ),
    (
        "Swift / Apple build output",
        &[".build/", "DerivedData/", "*.xcarchive/", "*.dSYM/"],
    ),
    ("Rust build output", &["target/"]),
    (
        "Node / web build output",
        &[
            "node_modules/",
            ".next/",
            "dist/",
            "out/",
            ".turbo/",
            ".cache/",
        ],
    ),
    (
        "Python caches",
        &[
            "__pycache__/",
            "*.pyc",
            ".venv/",
            "venv/",
            ".pytest_cache/",
            ".mypy_cache/",
            ".ruff_cache/",
        ],
    ),
    ("Logs & dumps", &["*.log", "*.tmp", "core.[0-9]*"]),
];

const CLAUDE_SETTINGS_SCHEMA: &str = "https://json.schemastore.org/claude-code-settings.json";

impl FixerEngine {
    /// Protects the workspace from agent file access:
    ///
    /// * adds `Read(...)` deny rules for secret files to `.claude/settings.json`
    ///   (the mechanism Claude Code actually enforces), and
    /// * maintains a managed block in `.cursorignore` for Cursor.
    ///
    /// `.claudeignore` is no longer written: Claude Code never reads it, so a
    /// generated one only created a false sense of protection.
    pub fn generate_ignore_files(root: &Path, dry_run: bool) -> Result<Vec<FixAction>> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut actions = vec![
            Self::apply_claude_deny_rules(&canonical_root, dry_run)?,
            Self::apply_ignore_file(&canonical_root.join(".cursorignore"), dry_run)?,
        ];

        let claudeignore = canonical_root.join(".claudeignore");
        if claudeignore.exists() {
            actions.push(FixAction {
                target: claudeignore,
                outcome: FixOutcome::Skipped,
                detail: "Claude Code does not read .claudeignore; left untouched. \
                         Protection now lives in .claude/settings.json."
                    .to_string(),
                backup: None,
            });
        }

        Ok(actions)
    }

    /// Merges agentprof's secret deny rules into `.claude/settings.json`.
    ///
    /// agentprof's rules are placed first and the user's own rules after them,
    /// so the user's `!` exceptions still carve paths out of ours, while our
    /// exceptions (for `.env.example` and friends) can never loosen a rule the
    /// user wrote. Everything else in the file is preserved as-is.
    pub(crate) fn apply_claude_deny_rules(root: &Path, dry_run: bool) -> Result<FixAction> {
        let path = root.join(".claude").join("settings.json");
        let existing_text = fs::read_to_string(&path).ok();

        let skipped = |detail: &str| FixAction {
            target: path.clone(),
            outcome: FixOutcome::Skipped,
            detail: detail.to_string(),
            backup: None,
        };

        let mut doc = match existing_text.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(map)) => Value::Object(map),
                Ok(_) => return Ok(skipped("not a JSON object; left unchanged")),
                Err(e) => {
                    return Ok(skipped(&format!("not valid JSON ({}); left unchanged", e)));
                }
            },
            _ => json!({ "$schema": CLAUDE_SETTINGS_SCHEMA }),
        };

        let Some(permissions) = doc
            .as_object_mut()
            .map(|o| o.entry("permissions").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut)
        else {
            return Ok(skipped("`permissions` is not an object; left unchanged"));
        };
        let Some(deny) = permissions
            .entry("deny")
            .or_insert_with(|| json!([]))
            .as_array_mut()
        else {
            return Ok(skipped(
                "`permissions.deny` is not an array; left unchanged",
            ));
        };

        let before: Vec<Value> = deny.clone();
        let ours: Vec<Value> = SECRET_DENY_RULES.iter().map(|r| json!(r)).collect();
        let added = ours.iter().filter(|r| !before.contains(r)).count();
        let theirs = before.iter().filter(|r| !ours.contains(r)).cloned();
        let merged: Vec<Value> = ours.iter().cloned().chain(theirs).collect();

        if merged == before {
            return Ok(FixAction {
                target: path,
                outcome: FixOutcome::AlreadyApplied,
                detail: "secret-file deny rules already present".to_string(),
                backup: None,
            });
        }
        let detail = if added > 0 {
            format!(
                "{} Read deny rule(s) for secret files — Claude Code will refuse to read or edit them",
                added
            )
        } else {
            "reordered deny rules so user exceptions keep applying".to_string()
        };
        if dry_run {
            return Ok(FixAction {
                target: path,
                outcome: FixOutcome::WouldChange,
                detail: format!("would add {}", detail),
                backup: None,
            });
        }

        *deny = merged;
        let exists = path.exists();
        let backup = if exists {
            Some(Self::backup_file(&path)?)
        } else {
            None
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = serde_json::to_string_pretty(&doc)?;
        out.push('\n');
        fs::write(&path, out).with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(FixAction {
            target: path,
            outcome: if exists {
                FixOutcome::Updated
            } else {
                FixOutcome::Created
            },
            detail: format!("added {}", detail),
            backup,
        })
    }

    /// Rewrites agentprof's managed block in an ignore file, leaving every line
    /// outside the block untouched.
    ///
    /// Only lines outside the block count as "already present". Counting the
    /// block's own lines made a re-run drop every pattern except newly added
    /// ones, since the old block is replaced wholesale.
    fn apply_ignore_file(path: &Path, dry_run: bool) -> Result<FixAction> {
        let existing = fs::read_to_string(path).unwrap_or_default();
        let exists = path.exists();

        let user_part = Self::strip_managed_block(&existing, IGNORE_BLOCK_START, IGNORE_BLOCK_END);
        let user_patterns: HashSet<&str> = user_part
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        let old_block_patterns: HashSet<String> = Self::managed_block_lines(&existing)
            .into_iter()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();

        let mut block = String::new();
        let mut block_patterns = Vec::new();
        for (heading, patterns) in IGNORE_ENTRIES {
            let missing: Vec<&str> = patterns
                .iter()
                .copied()
                .filter(|p| !user_patterns.contains(p))
                .collect();
            if missing.is_empty() {
                continue;
            }
            block.push_str(&format!("\n# {}\n", heading));
            for p in missing {
                block.push_str(p);
                block.push('\n');
                block_patterns.push(p);
            }
        }

        let mut out = user_part.trim_end().to_string();
        if !block.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(IGNORE_BLOCK_START);
            out.push_str("\n# Generated by agentprof. Edit outside this block; it is rewritten.\n");
            out.push_str(block.trim_start_matches('\n'));
            out.push_str(IGNORE_BLOCK_END);
        }
        if !out.is_empty() {
            out.push('\n');
        }

        if out == existing {
            return Ok(FixAction {
                target: path.to_path_buf(),
                outcome: FixOutcome::AlreadyApplied,
                detail: "all agentprof patterns already present".to_string(),
                backup: None,
            });
        }

        let added = block_patterns
            .iter()
            .filter(|p| !old_block_patterns.contains(**p))
            .count();
        let removed = old_block_patterns
            .iter()
            .filter(|p| !block_patterns.contains(&p.as_str()))
            .count();
        let mut detail = format!("{} pattern(s) added", added);
        if removed > 0 {
            detail.push_str(&format!(", {} outdated pattern(s) removed", removed));
        }

        if dry_run {
            return Ok(FixAction {
                target: path.to_path_buf(),
                outcome: FixOutcome::WouldChange,
                detail: format!("would change: {}", detail),
                backup: None,
            });
        }

        let backup = if exists && !user_part.trim().is_empty() {
            Some(Self::backup_file(path)?)
        } else {
            None
        };
        fs::write(path, out).with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(FixAction {
            target: path.to_path_buf(),
            outcome: if exists {
                FixOutcome::Updated
            } else {
                FixOutcome::Created
            },
            detail,
            backup,
        })
    }

    pub(crate) fn strip_managed_block(content: &str, start: &str, end: &str) -> String {
        let mut out = String::new();
        let mut skipping = false;
        for line in content.lines() {
            if line.trim() == start {
                skipping = true;
                continue;
            }
            if line.trim() == end {
                skipping = false;
                continue;
            }
            if !skipping {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    fn managed_block_lines(content: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut inside = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == IGNORE_BLOCK_START {
                inside = true;
            } else if trimmed == IGNORE_BLOCK_END {
                inside = false;
            } else if inside {
                out.push(trimmed.to_string());
            }
        }
        out
    }

    fn backup_file(path: &Path) -> Result<PathBuf> {
        // Never clobber an earlier backup.
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut candidate = path.with_file_name(format!("{}.agentprof.bak", file_name));
        let mut n = 1;
        while candidate.exists() {
            candidate = path.with_file_name(format!("{}.agentprof.bak.{}", file_name, n));
            n += 1;
        }
        fs::copy(path, &candidate)
            .with_context(|| format!("Failed to back up {}", path.display()))?;
        Ok(candidate)
    }

    /// Installs an opt-in fast-path guard in the user's shell rc file.
    ///
    /// The guard activates **only** when `AGENTPROF_FAST_PATH=1` is exported
    /// (which `agentprof wrap` does). It deliberately does not trigger on
    /// `$- != *i*` or on the mere presence of `CLAUDE_CODE`: the former breaks
    /// every `zsh script.sh` invocation, and the latter silently strips the
    /// user's real interactive environment whenever they work inside an agent.
    ///
    /// Before returning early it restores a PATH snapshot captured from a full
    /// interactive shell, so skipping the rc file cannot lose tool visibility.
    pub fn inject_shell_fast_path(dry_run: bool) -> Result<FixAction> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .context("HOME environment variable not set")?;

        let (shell_name, _) = crate::core::shell_bench::ShellBenchmarker::detect_shell();
        let rc_name = match shell_name.as_str() {
            "bash" => ".bashrc",
            "zsh" => ".zshrc",
            other => anyhow::bail!(
                "shell '{}' is not supported by `fix --shell` (zsh and bash only)",
                other
            ),
        };
        let rc_path = home.join(rc_name);

        if !rc_path.exists() {
            anyhow::bail!("{} not found", rc_path.display());
        }

        let existing = fs::read_to_string(&rc_path)?;
        if existing.contains(FAST_PATH_SENTINEL) {
            // The PATH snapshot goes stale as tools are installed or version
            // managers switch; re-running the fix refreshes it.
            if dry_run {
                return Ok(FixAction {
                    target: rc_path,
                    outcome: FixOutcome::WouldChange,
                    detail: "guard installed; would refresh its PATH snapshot".to_string(),
                    backup: None,
                });
            }
            let snapshot_path = Self::write_path_snapshot(&home, &shell_name)?;
            return Ok(FixAction {
                target: rc_path,
                outcome: FixOutcome::Updated,
                detail: format!(
                    "guard already installed; PATH snapshot refreshed at {}",
                    snapshot_path.display()
                ),
                backup: None,
            });
        }

        if dry_run {
            return Ok(FixAction {
                target: rc_path,
                outcome: FixOutcome::WouldChange,
                detail: "would prepend an opt-in fast-path guard".to_string(),
                backup: None,
            });
        }

        let snapshot_path = Self::write_path_snapshot(&home, &shell_name)?;
        let backup = Self::backup_file(&rc_path)?;

        let guard = format!(
            r#"{sentinel}
# Activates only when AGENTPROF_FAST_PATH=1 is exported (see `agentprof wrap`).
# Restores a PATH snapshot first so skipping the rest of this file cannot hide tools.
if [ "${{AGENTPROF_FAST_PATH:-0}}" = "1" ]; then
  [ -r "{snapshot}" ] && . "{snapshot}"
  return 0 2>/dev/null || exit 0
fi
{end}

"#,
            sentinel = FAST_PATH_SENTINEL,
            snapshot = snapshot_path.display(),
            end = FAST_PATH_END,
        );

        fs::write(&rc_path, format!("{}{}", guard, existing))?;

        Ok(FixAction {
            target: rc_path,
            outcome: FixOutcome::Created,
            detail: format!(
                "opt-in guard installed; PATH snapshot at {}",
                snapshot_path.display()
            ),
            backup: Some(backup),
        })
    }

    /// Captures the PATH a full interactive login shell produces, so the
    /// fast path can reproduce it without re-running the rc file.
    fn write_path_snapshot(home: &Path, shell_name: &str) -> Result<PathBuf> {
        let dir = home.join(".agentprof");
        fs::create_dir_all(&dir)?;
        let snapshot = dir.join("fastpath.sh");

        let captured = Command::new(shell_name)
            .args(["-lic", "printf %s \"$PATH\""])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());

        fs::write(
            &snapshot,
            format!(
                "# Generated by agentprof: PATH captured from a full interactive login shell.\n\
                 # Re-generate with `agentprof fix --shell`, or edit freely — it is only sourced\n\
                 # when AGENTPROF_FAST_PATH=1.\nexport PATH={}\n",
                shell_escape(&captured)
            ),
        )?;
        Ok(snapshot)
    }

    /// Compiles the shell rc file to zsh bytecode, verifying the `.zwc` appeared.
    pub fn compile_zshrc_bytecode(dry_run: bool) -> Result<FixAction> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .context("HOME environment variable not set")?;
        let zshrc = home.join(".zshrc");
        let zwc = home.join(".zshrc.zwc");

        if !zshrc.exists() {
            return Ok(FixAction {
                target: zshrc,
                outcome: FixOutcome::Skipped,
                detail: "~/.zshrc not found".to_string(),
                backup: None,
            });
        }
        if which::which("zsh").is_err() {
            return Ok(FixAction {
                target: zshrc,
                outcome: FixOutcome::Skipped,
                detail: "zsh is not installed".to_string(),
                backup: None,
            });
        }
        if dry_run {
            return Ok(FixAction {
                target: zwc,
                outcome: FixOutcome::WouldChange,
                detail: "would run `zcompile ~/.zshrc`".to_string(),
                backup: None,
            });
        }

        let before = fs::metadata(&zwc).and_then(|m| m.modified()).ok();
        let status = Command::new("zsh")
            .args([
                "-fc",
                "zcompile -R -- \"$HOME/.zshrc.zwc\" \"$HOME/.zshrc\"",
            ])
            .status()?;

        if !status.success() {
            return Ok(FixAction {
                target: zwc,
                outcome: FixOutcome::Skipped,
                detail: format!("zcompile exited with {}", status),
                backup: None,
            });
        }

        let after = fs::metadata(&zwc).and_then(|m| m.modified()).ok();
        if after.is_none() {
            return Ok(FixAction {
                target: zwc,
                outcome: FixOutcome::Skipped,
                detail: "zcompile reported success but no .zwc was produced".to_string(),
                backup: None,
            });
        }

        Ok(FixAction {
            target: zwc,
            outcome: if before == after {
                FixOutcome::AlreadyApplied
            } else {
                FixOutcome::Created
            },
            detail: "~/.zshrc compiled to bytecode".to_string(),
            backup: None,
        })
    }
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentprof_fixer_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_existing_ignore_file_content_is_preserved() {
        let dir = tempdir("preserve");
        let path = dir.join(".cursorignore");
        fs::write(&path, "# my curated rules\nsecret-project/\n").unwrap();

        FixerEngine::apply_ignore_file(&path, false).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# my curated rules"),
            "user comment was dropped"
        );
        assert!(
            after.contains("secret-project/"),
            "user pattern was dropped"
        );
        assert!(
            after.contains("target/"),
            "agentprof patterns were not added"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_existing_ignore_file_is_backed_up() {
        let dir = tempdir("backup");
        let path = dir.join(".cursorignore");
        fs::write(&path, "keepme/\n").unwrap();

        let action = FixerEngine::apply_ignore_file(&path, false).unwrap();
        let backup = action.backup.expect("expected a backup");
        assert_eq!(fs::read_to_string(backup).unwrap(), "keepme/\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_second_run_is_idempotent() {
        let dir = tempdir("idempotent");
        let path = dir.join(".cursorignore");

        FixerEngine::apply_ignore_file(&path, false).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        let second_action = FixerEngine::apply_ignore_file(&path, false).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(second_action.outcome, FixOutcome::AlreadyApplied);
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Regression: patterns inside the old managed block counted as "already
    /// present", so rewriting the block on upgrade dropped all of them.
    #[test]
    fn test_upgrading_the_block_keeps_every_pattern_and_drops_core_star() {
        let dir = tempdir("upgrade");
        let path = dir.join(".cursorignore");
        fs::write(
            &path,
            format!(
                "mine/\n\n{}\n# Logs & dumps\n*.log\ncore.*\ntarget/\n{}\n",
                IGNORE_BLOCK_START, IGNORE_BLOCK_END
            ),
        )
        .unwrap();

        let action = FixerEngine::apply_ignore_file(&path, false).unwrap();
        assert_eq!(action.outcome, FixOutcome::Updated);
        let after = fs::read_to_string(&path).unwrap();
        for pattern in [
            "mine/",
            "*.log",
            "target/",
            "node_modules/",
            "*.pem",
            "core.[0-9]*",
        ] {
            assert!(
                after.lines().any(|l| l == pattern),
                "{} missing:\n{}",
                pattern,
                after
            );
        }
        assert!(
            !after.lines().any(|l| l == "core.*"),
            "core.* hides src/core.ts"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dry_run_writes_nothing() {
        let dir = tempdir("dryrun");
        let path = dir.join(".cursorignore");

        let action = FixerEngine::apply_ignore_file(&path, true).unwrap();
        assert_eq!(action.outcome, FixOutcome::WouldChange);
        assert!(!path.exists(), "dry run created a file");

        let action = FixerEngine::apply_claude_deny_rules(&dir, true).unwrap();
        assert_eq!(action.outcome, FixOutcome::WouldChange);
        assert!(!dir.join(".claude").exists(), "dry run created .claude/");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_strip_managed_block_removes_only_the_block() {
        let content = format!(
            "user line\n{}\nmanaged\n{}\ntrailing line\n",
            IGNORE_BLOCK_START, IGNORE_BLOCK_END
        );
        let stripped =
            FixerEngine::strip_managed_block(&content, IGNORE_BLOCK_START, IGNORE_BLOCK_END);
        assert!(stripped.contains("user line"));
        assert!(stripped.contains("trailing line"));
        assert!(!stripped.contains("managed"));
    }

    #[test]
    fn test_deny_rules_create_a_settings_file() {
        let dir = tempdir("deny_new");
        let action = FixerEngine::apply_claude_deny_rules(&dir, false).unwrap();
        assert_eq!(action.outcome, FixOutcome::Created);

        let doc: Value =
            serde_json::from_str(&fs::read_to_string(dir.join(".claude/settings.json")).unwrap())
                .unwrap();
        assert_eq!(doc["$schema"], CLAUDE_SETTINGS_SCHEMA);
        let deny = doc["permissions"]["deny"].as_array().unwrap();
        assert_eq!(deny.len(), SECRET_DENY_RULES.len());
        assert_eq!(deny[0], "Read(.env*)");

        let again = FixerEngine::apply_claude_deny_rules(&dir, false).unwrap();
        assert_eq!(again.outcome, FixOutcome::AlreadyApplied);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_deny_rules_merge_keeps_user_settings_and_key_order() {
        let dir = tempdir("deny_merge");
        fs::create_dir_all(dir.join(".claude")).unwrap();
        let path = dir.join(".claude/settings.json");
        fs::write(
            &path,
            r#"{
  "model": "opus",
  "permissions": {
    "allow": ["Bash(npm test)"],
    "deny": ["Read(./secrets/**)", "Read(!.env.shared)", "Read(*.pem)"]
  },
  "hooks": {}
}"#,
        )
        .unwrap();

        let action = FixerEngine::apply_claude_deny_rules(&dir, false).unwrap();
        assert_eq!(action.outcome, FixOutcome::Updated);
        assert!(action.backup.is_some());

        let text = fs::read_to_string(&path).unwrap();
        let model = text.find("\"model\"").unwrap();
        let permissions = text.find("\"permissions\"").unwrap();
        let hooks = text.find("\"hooks\"").unwrap();
        assert!(
            model < permissions && permissions < hooks,
            "key order changed:\n{}",
            text
        );

        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["permissions"]["allow"][0], "Bash(npm test)");
        let deny: Vec<&str> = doc["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        // Ours first, then the user's remaining rules in their original order;
        // the user's `!` exception comes after ours, so it still applies.
        assert_eq!(&deny[..SECRET_DENY_RULES.len()], SECRET_DENY_RULES);
        assert_eq!(
            &deny[SECRET_DENY_RULES.len()..],
            ["Read(./secrets/**)", "Read(!.env.shared)"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_invalid_settings_json_is_never_overwritten() {
        let dir = tempdir("deny_bad");
        fs::create_dir_all(dir.join(".claude")).unwrap();
        let path = dir.join(".claude/settings.json");
        fs::write(&path, "{ \"permissions\": { // comment\n } }").unwrap();

        let action = FixerEngine::apply_claude_deny_rules(&dir, false).unwrap();
        assert_eq!(action.outcome, FixOutcome::Skipped);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{ \"permissions\": { // comment\n } }"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_claudeignore_is_no_longer_generated() {
        let dir = tempdir("no_claudeignore");
        let actions = FixerEngine::generate_ignore_files(&dir, false).unwrap();
        assert!(!dir.join(".claudeignore").exists());
        assert!(dir.join(".claude/settings.json").exists());
        assert!(dir.join(".cursorignore").exists());
        assert_eq!(actions.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_shell_escape_handles_quotes() {
        assert_eq!(shell_escape("/a/b"), "'/a/b'");
        assert!(shell_escape("/it's/path").contains(r"'\''"));
    }
}
