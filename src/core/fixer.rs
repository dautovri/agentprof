use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

/// Patterns agentprof manages inside ignore files.
const IGNORE_ENTRIES: &[(&str, &[&str])] = &[
    (
        "Secrets & sensitive files",
        &[
            ".env",
            ".env.*",
            "!.env.example",
            "*.pem",
            "*.key",
            "id_rsa",
            "id_ed25519",
            "credentials.json",
            "service-account.json",
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
    ("Logs & dumps", &["*.log", "*.tmp", "core.*"]),
];

impl FixerEngine {
    /// Adds agentprof's ignore patterns to `.claudeignore` / `.cursorignore`
    /// **without discarding existing content**.
    ///
    /// The previous implementation wrote the files unconditionally, destroying
    /// any hand-curated rules the user had. Existing files are now backed up and
    /// only the missing patterns are appended inside a managed block.
    pub fn generate_ignore_files(root: &Path, dry_run: bool) -> Result<Vec<FixAction>> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut actions = Vec::new();

        for name in [".claudeignore", ".cursorignore"] {
            let path = canonical_root.join(name);
            actions.push(Self::apply_ignore_file(&path, dry_run)?);
        }

        Ok(actions)
    }

    fn apply_ignore_file(path: &Path, dry_run: bool) -> Result<FixAction> {
        let existing = fs::read_to_string(path).unwrap_or_default();
        let exists = path.exists();

        // Anything already present — inside or outside our block — is left alone.
        let present: HashSet<&str> = existing
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();

        let mut block = String::new();
        for (heading, patterns) in IGNORE_ENTRIES {
            let missing: Vec<&str> = patterns
                .iter()
                .copied()
                .filter(|p| !present.contains(p))
                .collect();
            if missing.is_empty() {
                continue;
            }
            block.push_str(&format!("\n# {}\n", heading));
            for p in missing {
                block.push_str(p);
                block.push('\n');
            }
        }

        if block.is_empty() {
            return Ok(FixAction {
                target: path.to_path_buf(),
                outcome: FixOutcome::AlreadyApplied,
                detail: "all agentprof patterns already present".to_string(),
                backup: None,
            });
        }

        let added = block
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();

        if dry_run {
            return Ok(FixAction {
                target: path.to_path_buf(),
                outcome: FixOutcome::WouldChange,
                detail: format!("would add {} pattern(s)", added),
                backup: None,
            });
        }

        // Strip any previous managed block so repeated runs stay idempotent.
        let preserved = Self::strip_managed_block(&existing, IGNORE_BLOCK_START, IGNORE_BLOCK_END);

        let backup = if exists && !preserved.trim().is_empty() {
            let bak = Self::backup_file(path)?;
            Some(bak)
        } else {
            None
        };

        let mut out = preserved.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(IGNORE_BLOCK_START);
        out.push_str(
            "\n# Generated by agentprof. Edit above this line; this block is rewritten.\n",
        );
        out.push_str(block.trim_start_matches('\n'));
        out.push_str(IGNORE_BLOCK_END);
        out.push('\n');

        fs::write(path, out).with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(FixAction {
            target: path.to_path_buf(),
            outcome: if exists {
                FixOutcome::Updated
            } else {
                FixOutcome::Created
            },
            detail: format!("added {} pattern(s)", added),
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

    fn backup_file(path: &Path) -> Result<PathBuf> {
        // Never clobber an earlier backup.
        let mut candidate = path.with_extension("agentprof.bak");
        let mut n = 1;
        while candidate.exists() {
            candidate = path.with_extension(format!("agentprof.bak.{}", n));
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
            return Ok(FixAction {
                target: rc_path,
                outcome: FixOutcome::AlreadyApplied,
                detail: "fast-path guard already installed".to_string(),
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
        let path = dir.join(".claudeignore");
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
        let path = dir.join(".claudeignore");
        fs::write(&path, "keepme/\n").unwrap();

        let action = FixerEngine::apply_ignore_file(&path, false).unwrap();
        let backup = action.backup.expect("expected a backup");
        assert_eq!(fs::read_to_string(backup).unwrap(), "keepme/\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_second_run_is_idempotent() {
        let dir = tempdir("idempotent");
        let path = dir.join(".claudeignore");

        FixerEngine::apply_ignore_file(&path, false).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        let second_action = FixerEngine::apply_ignore_file(&path, false).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(second_action.outcome, FixOutcome::AlreadyApplied);
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dry_run_writes_nothing() {
        let dir = tempdir("dryrun");
        let path = dir.join(".claudeignore");

        let action = FixerEngine::apply_ignore_file(&path, true).unwrap();
        assert_eq!(action.outcome, FixOutcome::WouldChange);
        assert!(!path.exists(), "dry run created a file");
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
    fn test_shell_escape_handles_quotes() {
        assert_eq!(shell_escape("/a/b"), "'/a/b'");
        assert!(shell_escape("/it's/path").contains(r"'\''"));
    }
}
