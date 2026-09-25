use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs agentprof against an empty home directory, so results never depend
/// on the developer's own agent configs, transcripts or shell rc files.
fn bin() -> Command {
    let home = std::env::temp_dir().join(format!("agentprof_it_home_{}", std::process::id()));
    fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agentprof"));
    // Keep the runner's own agent state (Codex snapshots, config) out of reach.
    cmd.env("HOME", &home).env_remove("CODEX_HOME");
    cmd
}

fn run(args: &[&str]) -> Output {
    bin()
        .args(args)
        .output()
        .expect("failed to execute agentprof")
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agentprof_it_{}_{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Every `--json` invocation must emit a single parseable document on stdout.
fn assert_valid_json(out: &Output, label: &str) -> serde_json::Value {
    assert!(out.status.success(), "{} exited with {}", label, out.status);
    serde_json::from_str(&stdout(out))
        .unwrap_or_else(|e| panic!("{} did not emit valid JSON: {}\n{}", label, e, stdout(out)))
}

#[test]
fn test_cli_help_lists_all_commands() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let text = stdout(&out);
    for cmd in [
        "scan",
        "agent",
        "mcp",
        "skills",
        "history",
        "compress",
        "lint",
        "report",
        "wrap",
        "tui",
        "omz",
        "bench",
        "context",
        "fix",
        "compile",
        "ci",
        "completions",
    ] {
        assert!(text.contains(cmd), "`{}` missing from --help", cmd);
    }
}

#[test]
fn test_version_matches_cargo_manifest() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    assert!(
        stdout(&out).contains(env!("CARGO_PKG_VERSION")),
        "reported version does not match Cargo.toml"
    );
}

/// Regression: human-readable banners used to be printed to stdout even under
/// `--json`, so no consumer could parse the output of these commands.
#[test]
fn test_json_output_is_never_polluted_by_banners() {
    let dir = workspace("jsonpure");
    fs::write(dir.join("AGENTS.md"), "# Rules\n\n- Use tabs.\n").unwrap();
    let path = dir.to_str().unwrap();

    for cmd in [
        "scan", "lint", "context", "report", "skills", "mcp", "agent",
    ] {
        let out = run(&[cmd, path, "--json"]);
        let value = assert_valid_json(&out, cmd);
        assert!(
            value.is_object() || value.is_array(),
            "{} JSON has odd shape",
            cmd
        );
    }

    let out = run(&["bench", "--iterations", "3", "--json"]);
    assert_valid_json(&out, "bench");
    let out = run(&["history", "--sessions", "1", "--json"]);
    assert_valid_json(&out, "history");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_lint_detects_vague_rule() {
    let dir = workspace("lint");
    fs::write(
        dir.join("AGENTS.md"),
        "# Rules\n\n- Follow best practices\n",
    )
    .unwrap();

    let out = run(&["lint", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "lint");
    assert_eq!(value["total_issues"].as_u64().unwrap(), 1);
    assert_eq!(value["issues"][0]["code"], "VAGUE_RULE");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_lint_reports_cross_file_contradiction() {
    let dir = workspace("lintconflict");
    fs::write(
        dir.join("AGENTS.md"),
        "Use ObservableObject for view models.\n",
    )
    .unwrap();
    fs::write(dir.join("CLAUDE.md"), "Use @Observable for view models.\n").unwrap();

    // A contradiction is an error, so lint fails the way linters do in CI...
    let out = run(&["lint", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1), "lint must exit 1 on a conflict");
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        value["contradictions_found"].as_u64().unwrap() >= 1,
        "expected a cross-file contradiction, got {}",
        value
    );

    // ...unless the caller opts out.
    let out = run(&["lint", dir.to_str().unwrap(), "--fail-on", "never"]);
    assert!(out.status.success());

    let _ = fs::remove_dir_all(&dir);
}

/// Regression: identical files were reported as conflicting because
/// "pnpm install" contains "npm install".
#[test]
fn test_lint_does_not_flag_agreeing_files() {
    let dir = workspace("lintagree");
    let rules =
        "- Always use pnpm install for dependencies\n- Use @StateObject for owned view models\n";
    fs::write(dir.join("AGENTS.md"), rules).unwrap();
    fs::write(dir.join("CLAUDE.md"), rules).unwrap();

    let out = run(&["lint", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "lint");
    assert_eq!(value["contradictions_found"], 0, "{}", value);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_compress_reduces_tokens_and_keeps_original() {
    let dir = workspace("compress");
    let sample = dir.join("rules.md");
    fs::write(
        &sample,
        "# Rules\n\nPlease make sure to always remember to strictly adhere to formatting.\n",
    )
    .unwrap();
    let before = fs::read_to_string(&sample).unwrap();

    let out = run(&["compress", sample.to_str().unwrap()]);
    assert!(out.status.success());

    // Without --overwrite the source file must be untouched.
    assert_eq!(fs::read_to_string(&sample).unwrap(), before);
    let compressed = dir.join("rules.compressed.md");
    assert!(compressed.exists(), "compressed output was not written");
    assert!(fs::read_to_string(&compressed).unwrap().len() < before.len());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_compress_overwrite_creates_a_backup() {
    let dir = workspace("compressbak");
    let sample = dir.join("rules.md");
    let original = "# Rules\n\nPlease make sure to always remember to test.\n";
    fs::write(&sample, original).unwrap();

    let out = run(&["compress", sample.to_str().unwrap(), "--overwrite"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(dir.join("rules.bak")).unwrap(), original);

    let _ = fs::remove_dir_all(&dir);
}

/// Regression: `fix --ignore` used to overwrite ignore files wholesale,
/// destroying hand-curated rules.
#[test]
fn test_fix_preserves_existing_ignore_rules() {
    let dir = workspace("fixignore");
    fs::write(dir.join(".cursorignore"), "# curated\nprivate-notes/\n").unwrap();

    let out = run(&["fix", "--ignore", dir.to_str().unwrap()]);
    assert!(out.status.success());

    let after = fs::read_to_string(dir.join(".cursorignore")).unwrap();
    assert!(after.contains("# curated"), "user comment lost");
    assert!(after.contains("private-notes/"), "user rule lost");
    assert!(after.contains("target/"), "agentprof rules not added");

    let _ = fs::remove_dir_all(&dir);
}

/// Claude Code only honours `permissions.deny`, so that is what `fix` must
/// write — and the audit must then report the secret as protected.
#[test]
fn test_fix_protects_secrets_with_claude_deny_rules() {
    let dir = workspace("fixdeny");
    fs::write(dir.join(".env"), "TOKEN=abc\n").unwrap();
    fs::write(dir.join(".env.example"), "TOKEN=\n").unwrap();
    let path = dir.to_str().unwrap();

    let before = assert_valid_json(&run(&["scan", path, "--json"]), "scan");
    assert_eq!(before["workspace"]["total_exposed_secrets"], 1);

    let out = run(&["fix", path]);
    assert!(out.status.success());
    assert!(
        !dir.join(".claudeignore").exists(),
        ".claudeignore must not be generated"
    );
    let settings = fs::read_to_string(dir.join(".claude/settings.json")).unwrap();
    assert!(settings.contains("Read(.env*)"), "{}", settings);

    let after = assert_valid_json(&run(&["scan", path, "--json"]), "scan");
    assert_eq!(after["workspace"]["total_exposed_secrets"], 0);
    assert_eq!(
        after["workspace"]["secret_risks"].as_array().unwrap().len(),
        1
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fix_dry_run_writes_nothing() {
    let dir = workspace("fixdry");
    let out = run(&["fix", "--ignore", "--dry-run", dir.to_str().unwrap()]);
    assert!(out.status.success());
    for created in [".claudeignore", ".cursorignore", ".claude"] {
        assert!(!dir.join(created).exists(), "dry run created {}", created);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_report_markdown_and_fail_under_gate() {
    let dir = workspace("report");

    let out = run(&["report", dir.to_str().unwrap(), "--markdown"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("AI Agent Workspace Health"));

    // A threshold above the maximum must fail; zero must always pass.
    let strict = run(&["report", dir.to_str().unwrap(), "--fail-under", "101"]);
    assert!(
        !strict.status.success(),
        "--fail-under 101 should fail the gate"
    );

    let lenient = run(&["report", dir.to_str().unwrap(), "--fail-under", "0"]);
    assert!(
        lenient.status.success(),
        "--fail-under 0 should always pass"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_report_headline_equals_category_sum() {
    let dir = workspace("reportsum");
    let out = run(&["report", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "report");

    let measured: Vec<&serde_json::Value> = value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["measured"] == true)
        .collect();
    let points: u64 = measured.iter().map(|c| c["score"].as_u64().unwrap()).sum();
    let max: u64 = measured.iter().map(|c| c["max"].as_u64().unwrap()).sum();
    assert_eq!(value["points"].as_u64().unwrap(), points);
    assert_eq!(value["max_points"].as_u64().unwrap(), max);
    let expected = ((points as f64 / max as f64) * 100.0).round() as u64;
    assert_eq!(value["score"].as_u64().unwrap(), expected);

    let _ = fs::remove_dir_all(&dir);
}

/// `--repo-only` must not depend on the machine: no shell benchmark, no MCP
/// configs, the same score everywhere.
#[test]
fn test_report_repo_only_is_machine_independent() {
    let dir = workspace("reportrepo");
    fs::write(dir.join(".gitignore"), "target/\n").unwrap();
    fs::write(dir.join("AGENTS.md"), "# Rules\n- Run cargo test.\n").unwrap();

    let out = run(&["report", dir.to_str().unwrap(), "--repo-only", "--json"]);
    let value = assert_valid_json(&out, "report");
    assert_eq!(value["scope"], "repo_only");
    assert_eq!(value["max_points"], 60);
    assert_eq!(value["score"], 100, "{}", value);
    let unmeasured: Vec<&str> = value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["measured"] == false)
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        unmeasured,
        vec!["Agent Shell Overhead", "MCP Tool Schema Load"]
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Regression: repeated headings silently overwrote each other's module file.
#[test]
fn test_compile_keeps_content_of_repeated_headings() {
    let dir = workspace("compile");
    fs::write(
        dir.join("AGENTS.md"),
        "## Python\nalpha rule\n\n## Python\nbeta rule\n",
    )
    .unwrap();

    let out = run(&["compile", dir.to_str().unwrap(), "--target", "claude"]);
    assert!(out.status.success());

    let combined: String = fs::read_dir(dir.join(".claude/rules"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    assert!(combined.contains("alpha rule"), "first section lost");
    assert!(combined.contains("beta rule"), "second section lost");
    assert!(combined.contains("paths:"), "rules must be path-scoped");
    assert!(
        !dir.join(".agentrules").exists(),
        "no agent reads .agentrules/"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_compile_without_rule_file_fails_cleanly() {
    let dir = workspace("compilenone");
    let out = run(&["compile", dir.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "compile should fail without a rule file"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("No AGENTS.md"));
    let _ = fs::remove_dir_all(&dir);
}

/// Regression: an existing workflow was replaced without warning.
#[test]
fn test_ci_does_not_clobber_existing_workflow() {
    let dir = workspace("ci");
    fs::create_dir_all(dir.join(".github/workflows")).unwrap();
    let wf = dir.join(".github/workflows/agentprof-audit.yml");
    fs::write(&wf, "name: mine\n").unwrap();

    let out = run(&["ci", dir.to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(&wf).unwrap(), "name: mine\n");

    // With --force it is replaced, but the original is kept.
    let out = run(&["ci", dir.to_str().unwrap(), "--force"]);
    assert!(out.status.success());
    assert!(fs::read_to_string(&wf).unwrap().contains("fail-under:"));
    assert_eq!(
        fs::read_to_string(dir.join(".github/workflows/agentprof-audit.yml.agentprof.bak"))
            .unwrap(),
        "name: mine\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Regression: secrets vendored inside dependency directories were reported as
/// the user's own exposed credentials.
#[test]
fn test_scan_ignores_secrets_inside_dependency_directories() {
    let dir = workspace("secrets");
    fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    fs::write(dir.join("node_modules/pkg/fixture.pem"), "test key").unwrap();
    fs::write(dir.join(".env"), "TOKEN=abc").unwrap();

    let out = run(&["scan", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "scan");
    let risks = value["workspace"]["secret_risks"].as_array().unwrap();

    assert_eq!(
        risks.len(),
        1,
        "expected only the top-level .env: {:?}",
        risks
    );
    assert_eq!(risks[0]["relative_path"], ".env");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_env_example_is_not_reported_as_a_secret() {
    let dir = workspace("envexample");
    fs::write(dir.join(".env.example"), "TOKEN=").unwrap();

    let out = run(&["scan", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "scan");
    assert!(
        value["workspace"]["secret_risks"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Regression: the global `--path` flag was shadowed by each subcommand's
/// positional and silently rejected.
#[test]
fn test_global_path_flag_and_positional_both_work() {
    let dir = workspace("pathflag");
    fs::write(dir.join("AGENTS.md"), "# Rules\n").unwrap();
    let path = dir.to_str().unwrap();

    for args in [
        vec!["context", "--path", path, "--json"],
        vec!["context", path, "--json"],
    ] {
        let out = run(&args);
        let value = assert_valid_json(&out, &args.join(" "));
        assert_eq!(value["total_files"].as_u64().unwrap(), 1, "for {:?}", args);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_wrap_propagates_exit_code_and_env() {
    let ok = run(&["wrap", "sh", "-c", "printf child-output"]);
    assert!(ok.status.success());
    // The summary goes to stderr; stdout carries only the child's output.
    assert!(String::from_utf8_lossy(&ok.stderr).contains("Flight Recorder Summary"));
    assert_eq!(stdout(&ok), "child-output");

    let failing = run(&["wrap", "sh", "-c", "exit 42"]);
    assert_eq!(
        failing.status.code(),
        Some(42),
        "exit code was not propagated"
    );

    let env = run(&["wrap", "sh", "-c", "printf %s \"$AGENTPROF_FAST_PATH\""]);
    assert!(
        stdout(&env).contains('1'),
        "fast-path env var was not injected"
    );
}

#[test]
fn test_wrap_without_command_fails() {
    let out = run(&["wrap"]);
    assert!(!out.status.success(), "wrap with no command should fail");
}

#[test]
fn test_mcp_reports_no_invented_token_counts() {
    let dir = workspace("mcp");
    fs::write(
        dir.join(".mcp.json"),
        r#"{"mcpServers":{"never-probed":{"command":"/nonexistent/agentprof-test-server"}}}"#,
    )
    .unwrap();

    let out = run(&["mcp", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "mcp");
    let server = value["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "never-probed")
        .expect("workspace server not discovered");

    // An unmeasured server must report null, not a guessed number.
    assert!(
        server["schema_tokens"].is_null(),
        "invented a token count: {}",
        server
    );
    assert!(server["tool_count"].is_null());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_completions_generate_for_each_shell() {
    for shell in ["zsh", "bash", "fish", "powershell"] {
        let out = run(&["completions", shell]);
        assert!(out.status.success(), "completions failed for {}", shell);
        assert!(!stdout(&out).is_empty(), "empty completions for {}", shell);
    }
}

#[test]
fn test_scan_on_empty_directory_succeeds() {
    let dir = workspace("empty");
    let out = run(&["scan", dir.to_str().unwrap(), "--json"]);
    let value = assert_valid_json(&out, "scan");
    assert_eq!(value["context"]["total_files"].as_u64().unwrap(), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_nonexistent_path_does_not_panic() {
    let out = run(&["context", "/definitely/not/a/real/path/agentprof", "--json"]);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        !text.contains("panicked"),
        "a missing path should not panic: {}",
        text
    );
}

#[test]
fn test_skills_report_has_no_duplicate_entries() {
    let out = run(&["skills", "--json"]);
    let value = assert_valid_json(&out, "skills");
    let skills = value["all_skills"].as_array().unwrap();

    let mut paths: Vec<&str> = skills.iter().filter_map(|s| s["path"].as_str()).collect();
    let total = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        total,
        paths.len(),
        "the same skill was counted more than once"
    );
}

#[test]
fn test_bench_reports_the_shell_it_measured() {
    let out = run(&["bench", "--iterations", "3", "--json"]);
    let value = assert_valid_json(&out, "bench");
    assert!(
        !value["shell_name"].as_str().unwrap_or_default().is_empty(),
        "benchmark did not name the shell it measured"
    );
}

#[test]
fn test_generated_ci_workflow_is_valid_yaml_shape() {
    let dir = workspace("ciyaml");
    let out = run(&["ci", dir.to_str().unwrap(), "--min-score", "85"]);
    assert!(out.status.success());

    let yaml = fs::read_to_string(dir.join(".github/workflows/agentprof-audit.yml")).unwrap();
    assert!(yaml.starts_with("name:"));
    assert!(yaml.contains("fail-under: 85"), "threshold not embedded");
    assert!(yaml.contains("runs-on: ubuntu-latest"));
    assert!(
        yaml.contains("permissions:"),
        "no least-privilege permissions block"
    );
    assert!(
        yaml.contains(&format!(
            "uses: dautovri/agentprof@v{}",
            env!("CARGO_PKG_VERSION")
        )),
        "action not pinned to this release"
    );
    // No unexpanded Rust format placeholders may leak into the YAML.
    assert!(!yaml.contains("{version}") && !yaml.contains("{min_score}"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_repeated_runs_are_stable() {
    let dir = workspace("stable");
    fs::write(dir.join("AGENTS.md"), "# Rules\n\n- Use tabs.\n").unwrap();
    let path: &Path = &dir;

    let first = run(&["context", path.to_str().unwrap(), "--json"]);
    let second = run(&["context", path.to_str().unwrap(), "--json"]);
    assert_eq!(
        stdout(&first),
        stdout(&second),
        "context output is not deterministic"
    );

    let _ = fs::remove_dir_all(&dir);
}
