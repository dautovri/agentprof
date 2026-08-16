use std::fs;

#[test]
fn test_agentprof_cli_help() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agentprof"))
        .arg("--help")
        .output()
        .expect("Failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("agent"));
    assert!(stdout.contains("mcp"));
    assert!(stdout.contains("skills"));
    assert!(stdout.contains("history"));
    assert!(stdout.contains("compress"));
    assert!(stdout.contains("lint"));
    assert!(stdout.contains("report"));
    assert!(stdout.contains("wrap"));
    assert!(stdout.contains("tui"));
}

#[test]
fn test_agentprof_compress() {
    let temp_dir = std::env::temp_dir().join("agentprof_test_workspace_compress");
    let _ = fs::create_dir_all(&temp_dir);
    let sample = temp_dir.join("sample_rules.md");
    fs::write(&sample, "# Rules\n\nPlease make sure to always remember to strictly adhere to formatting.\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agentprof"))
        .args(["compress", sample.to_str().unwrap()])
        .output()
        .expect("Failed to execute compress");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Compressing instruction file"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_agentprof_lint() {
    let temp_dir = std::env::temp_dir().join("agentprof_test_workspace_lint");
    let _ = fs::create_dir_all(&temp_dir);
    let sample = temp_dir.join("AGENTS.md");
    fs::write(&sample, "# Rules\n\nwrite clean code\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agentprof"))
        .args(["lint", temp_dir.to_str().unwrap(), "--json"])
        .output()
        .expect("Failed to execute lint");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("VAGUE_RULE") || stdout.contains("total_issues"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_agentprof_report_markdown() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agentprof"))
        .args(["report", "--markdown"])
        .output()
        .expect("Failed to execute report");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AI Agent Workspace Health"));
}

#[test]
fn test_agentprof_wrap_command() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agentprof"))
        .args(["wrap", "echo", "agentprof wrap test"])
        .output()
        .expect("Failed to execute wrap");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Flight Recorder Summary"));
}
