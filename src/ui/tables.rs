use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use owo_colors::OwoColorize;

use crate::core::agent_platforms::AgentPlatformProfile;
use crate::core::mcp_profiler::McpProfileReport;
use crate::core::omz_profiler::OmzProfileReport;
use crate::core::report_generator::WorkspaceHealthScore;
use crate::core::scanner::WorkspaceContextSummary;
use crate::core::session_history::SessionHistoryReport;
use crate::core::shell_bench::ShellBenchmarkResult;
use crate::core::skills_auditor::SkillsAuditReport;
use crate::core::tokens::TokenCounter;
use crate::core::workspace_guard::WorkspaceAuditReport;
use crate::ui::formatters::Formatters;

pub struct TableRenderer;

impl TableRenderer {
    pub fn render_health_report(health: &WorkspaceHealthScore) {
        println!("{}", "\n🤖 AI Agent Workspace Health".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        let pct = (health.score * 100) / health.max_score.max(1);
        let headline = format!("{}/{} — {}", health.score, health.max_score, health.grade);
        println!(
            "  Score: {}",
            match pct {
                80..=100 => headline.green().bold().to_string(),
                60..=79 => headline.yellow().bold().to_string(),
                _ => headline.red().bold().to_string(),
            }
        );
        println!();

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("Category").fg(Color::Cyan),
                Cell::new("Score").fg(Color::Cyan),
                Cell::new("Detail").fg(Color::Cyan),
            ]);

        for c in &health.categories {
            let ratio = (c.score * 100) / c.max.max(1);
            let score_cell = Cell::new(format!("{}/{}", c.score, c.max)).fg(match ratio {
                80..=100 => Color::Green,
                50..=79 => Color::Yellow,
                _ => Color::Red,
            });
            table.add_row(vec![
                Cell::new(&c.name),
                score_cell,
                Cell::new(c.detail.replace('`', "")),
            ]);
        }
        println!("{table}");
        println!(
            "{}",
            "Use `--markdown` for a PR comment, or `--fail-under <score>` to gate CI.".dimmed()
        );
    }

    pub fn render_mcp_report(report: &McpProfileReport) {
        println!(
            "{}",
            "\n🔌 Model Context Protocol (MCP) Tool Schema Profiler"
                .bold()
                .cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        if report.servers.is_empty() {
            println!(
                "{}",
                "No MCP servers found in any inspected configuration.".dimmed()
            );
            return;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("MCP Server").fg(Color::Cyan),
                Cell::new("Scope").fg(Color::Cyan),
                Cell::new("Tools").fg(Color::Cyan),
                Cell::new("Schema Tokens").fg(Color::Cyan),
                Cell::new("Status").fg(Color::Cyan),
            ]);

        for s in &report.servers {
            let color = match s.schema_tokens {
                Some(t) if t > 3_000 => Color::Red,
                Some(t) if t > 1_500 => Color::Yellow,
                Some(_) => Color::Green,
                None => Color::DarkGrey,
            };
            let dash = "—".to_string();
            table.add_row(vec![
                Cell::new(&s.name),
                Cell::new(&s.scope),
                Cell::new(
                    s.tool_count
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| dash.clone()),
                ),
                Cell::new(
                    s.schema_tokens
                        .map(Formatters::format_tokens)
                        .unwrap_or(dash),
                ),
                Cell::new(&s.status).fg(color),
            ]);
        }

        println!("{table}");

        if report.measured_server_count > 0 {
            println!(
                "Measured Schema Load: {} tokens across {} of {} servers ({:.1}% of a 128k context)",
                Formatters::format_tokens(report.measured_schema_tokens)
                    .bold()
                    .yellow(),
                report.measured_server_count.bold(),
                report.total_servers.bold(),
                TokenCounter::context_percentage(report.measured_schema_tokens, 128_000)
                    .bold()
                    .magenta()
            );
            println!(
                "Cost of Re-sending Measured Schemas: {} per 100 turns",
                Formatters::format_currency(TokenCounter::estimate_cost_per_100_turns(
                    report.measured_schema_tokens
                ))
                .bold()
                .green()
            );
        } else {
            println!(
                "{}",
                "No server has been measured yet — token columns stay blank rather than guess."
                    .dimmed()
            );
        }

        let failed: Vec<&str> = report
            .servers
            .iter()
            .filter(|s| s.probe_error.is_some())
            .map(|s| s.name.as_str())
            .collect();
        if !failed.is_empty() {
            println!(
                "{}",
                format!("⚠️  Probe failed for: {}", failed.join(", ")).yellow()
            );
        }

        for rec in &report.recommendations {
            println!("💡 Recommendation: {}", rec.dimmed());
        }
    }

    pub fn render_skills_report(report: &SkillsAuditReport) {
        println!(
            "{}",
            "\n🎯 Agent Skills & Keyword Collision Auditor"
                .bold()
                .cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        println!(
            "Total Installed Skills:  {} skills",
            report.total_skills.bold().cyan()
        );
        println!(
            "Total Skills Corpus:     {} tokens (Avg: {} tokens/skill)",
            Formatters::format_tokens(report.total_tokens)
                .bold()
                .yellow(),
            report.average_tokens.to_string().dimmed()
        );
        println!(
            "Bloated Skills (>2.5k):  {}",
            if report.bloated_skills_count > 0 {
                format!("🚨 {} skills", report.bloated_skills_count)
                    .red()
                    .to_string()
            } else {
                "✅ None".green().to_string()
            }
        );

        if !report.collisions.is_empty() {
            println!(
                "\n{}",
                "⚠️ Trigger Keyword Collisions (Skills competing for identical intents):"
                    .bold()
                    .yellow()
            );
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec![
                    Cell::new("Trigger Keyword").fg(Color::Cyan),
                    Cell::new("Compromised Skills").fg(Color::Cyan),
                    Cell::new("Severity").fg(Color::Cyan),
                ]);

            for c in report.collisions.iter().take(6) {
                let sev_cell = if c.colliding_skills.len() >= 6 {
                    Cell::new(&c.severity).fg(Color::Red)
                } else {
                    Cell::new(&c.severity).fg(Color::Yellow)
                };

                let skills_summary = if c.colliding_skills.len() > 4 {
                    format!(
                        "{}, +{} more",
                        c.colliding_skills[..4].join(", "),
                        c.colliding_skills.len() - 4
                    )
                } else {
                    c.colliding_skills.join(", ")
                };

                table.add_row(vec![
                    Cell::new(&c.keyword),
                    Cell::new(skills_summary),
                    sev_cell,
                ]);
            }
            println!("{table}");
        }

        if !report.top_heavy_skills.is_empty() {
            println!("\n{}", "Top 5 Heaviest Skills:".bold());
            for s in report.top_heavy_skills.iter().take(5) {
                let badge = if s.is_bloated { "🚨 Bloated" } else { "✅" };
                println!(
                    "  • {:<28} -> {} tokens ({} lines) [{}]",
                    s.name.bold(),
                    Formatters::format_tokens(s.tokens).yellow(),
                    s.lines,
                    badge
                );
            }
        }
    }

    pub fn render_session_history(report: &SessionHistoryReport) {
        println!(
            "{}",
            "\n📉 Agent Session History & Loop Thrash Diagnostics"
                .bold()
                .cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        if report.total_sessions_found == 0 {
            println!(
                "{}",
                "No agent session transcripts found under ~/.claude/projects/.".dimmed()
            );
            return;
        }

        let coverage = if report.truncated {
            format!(
                "{} analyzed of {} found",
                report.sessions_analyzed, report.total_sessions_found
            )
        } else {
            format!("{} analyzed", report.sessions_analyzed)
        };
        println!("  • Session Transcripts:     {}", coverage.bold());
        println!(
            "  • User Turns (analyzed):   {}",
            report.total_turns.bold().cyan()
        );
        if report.prompt_history_entries > 0 {
            println!(
                "  • Prompt History Entries:  {}",
                report.prompt_history_entries.bold().dimmed()
            );
        }

        let u = &report.usage;
        println!(
            "  • Tokens (in / out):       {} / {}",
            Formatters::format_tokens(u.input_tokens).bold().yellow(),
            Formatters::format_tokens(u.output_tokens).bold().yellow()
        );
        println!(
            "  • Tokens (cache w / r):    {} / {}",
            Formatters::format_tokens(u.cache_creation_tokens)
                .bold()
                .magenta(),
            Formatters::format_tokens(u.cache_read_tokens)
                .bold()
                .green()
        );
        println!(
            "  • Total Tokens:            {}",
            Formatters::format_tokens(report.total_tokens_used).bold()
        );
        println!(
            "  • Est. Spend:              {} {}",
            Formatters::format_currency(report.total_estimated_cost_usd)
                .bold()
                .green(),
            format!("({})", report.pricing_label).dimmed()
        );
        println!(
            "  • Loop Thrash Incidents:   {}",
            if report.loop_thrash_incidents > 0 {
                format!(
                    "🚨 {} session(s) with repeated identical calls",
                    report.loop_thrash_incidents
                )
                .red()
                .to_string()
            } else {
                "✅ 0 detected".green().to_string()
            }
        );

        if !report.tool_usage_distribution.is_empty() {
            println!("\n{}", "Most Frequently Invoked Agent Tools:".bold());
            for (tool, count) in report.tool_usage_distribution.iter().take(8) {
                println!("  • {:<20} -> {} calls", tool.bold().blue(), count);
            }
        }

        if !report.recent_sessions.is_empty() {
            println!("\n{}", "Recent Sessions:".bold());
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_content_arrangement(ContentArrangement::Dynamic)
                .set_header(vec![
                    Cell::new("Project").fg(Color::Cyan),
                    Cell::new("Age").fg(Color::Cyan),
                    Cell::new("Turns").fg(Color::Cyan),
                    Cell::new("Tokens").fg(Color::Cyan),
                    Cell::new("Cost").fg(Color::Cyan),
                    Cell::new("Top Tool").fg(Color::Cyan),
                ]);
            for s in &report.recent_sessions {
                table.add_row(vec![
                    Cell::new(&s.project),
                    Cell::new(&s.last_modified),
                    Cell::new(s.turn_count.to_string()),
                    Cell::new(Formatters::format_tokens(s.usage.total())),
                    Cell::new(Formatters::format_currency(s.estimated_cost_usd)),
                    Cell::new(s.dominant_tool.as_deref().unwrap_or("—")),
                ]);
            }
            println!("{table}");
        }

        for rec in &report.recommendations {
            println!("💡 Recommendation: {}", rec.dimmed());
        }
    }

    pub fn render_agent_profiles(profiles: &[AgentPlatformProfile]) {
        println!(
            "{}",
            "\n🤖 Agent Platform Profiler (OpenCode / Claude / Grok)"
                .bold()
                .cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                Cell::new("Agent Platform").fg(Color::Cyan),
                Cell::new("Installed").fg(Color::Cyan),
                Cell::new("Config / Rules").fg(Color::Cyan),
                Cell::new("Fixed Tokens").fg(Color::Cyan),
                Cell::new("Context %").fg(Color::Cyan),
                Cell::new("Rating").fg(Color::Cyan),
            ]);

        for p in profiles {
            let installed_cell = if p.is_installed {
                Cell::new("✅ Yes").fg(Color::Green)
            } else {
                Cell::new("❌ No").fg(Color::DarkGrey)
            };

            let config_count_str = format!(
                "{} files ({} skills)",
                p.config_files.len(),
                p.total_skills_count
            );
            let rating_cell = if p.fixed_payload_percentage < 3.0 {
                Cell::new(p.health_rating).fg(Color::Green)
            } else if p.fixed_payload_percentage < 7.0 {
                Cell::new(p.health_rating).fg(Color::Yellow)
            } else {
                Cell::new(p.health_rating).fg(Color::Red)
            };

            table.add_row(vec![
                Cell::new(p.platform.name()),
                installed_cell,
                Cell::new(&config_count_str),
                Cell::new(Formatters::format_tokens(p.total_fixed_instruction_tokens)),
                Cell::new(format!("{:.1}%", p.fixed_payload_percentage)),
                rating_cell,
            ]);
        }

        println!("{table}");

        for p in profiles {
            if !p.config_files.is_empty() || !p.detected_mcp_servers.is_empty() {
                println!(
                    "\n{} [{} Context Details]:",
                    "▶".bold().blue(),
                    p.platform.name().bold()
                );
                for f in &p.config_files {
                    let scope = if f.is_global { "Global" } else { "Workspace" };
                    println!(
                        "  • {:<24} ({}) -> {} tokens ({} lines)",
                        f.name.bold(),
                        scope.dimmed(),
                        Formatters::format_tokens(f.tokens).yellow(),
                        f.lines
                    );
                }
                if !p.detected_mcp_servers.is_empty() {
                    println!(
                        "  • MCP Servers: {}",
                        p.detected_mcp_servers.join(", ").cyan()
                    );
                }
                if p.total_skills_count > 0 {
                    println!(
                        "  • Discovered Skills: {} skills ({} total tokens)",
                        p.total_skills_count,
                        Formatters::format_tokens(p.total_skills_tokens)
                    );
                }
                for rec in &p.recommendations {
                    println!("  💡 Recommendation: {}", rec.dimmed());
                }
            }
        }
    }

    pub fn render_context_summary(summary: &WorkspaceContextSummary) {
        println!(
            "{}",
            "\n🤖 AI Agent Context & Instruction Budget".bold().cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        if summary.files.is_empty() {
            println!("{}", "No AI instruction files found in workspace.".dimmed());
            return;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("File").fg(Color::Cyan),
                Cell::new("Type").fg(Color::Cyan),
                Cell::new("Tokens").fg(Color::Cyan),
                Cell::new("Lines").fg(Color::Cyan),
                Cell::new("Status").fg(Color::Cyan),
            ]);

        for file in &summary.files {
            let status_cell = match file.status {
                crate::core::scanner::HealthStatus::Optimal => {
                    Cell::new("Optimal").fg(Color::Green)
                }
                crate::core::scanner::HealthStatus::Warning => Cell::new("Heavy").fg(Color::Yellow),
                crate::core::scanner::HealthStatus::Bloated => Cell::new("Bloated").fg(Color::Red),
            };

            table.add_row(vec![
                Cell::new(&file.relative_path),
                Cell::new(file.category.label()),
                Cell::new(Formatters::format_tokens(file.tokens_cl100k)),
                Cell::new(file.lines.to_string()),
                status_cell,
            ]);
        }

        println!("{table}");

        println!(
            "Total Overhead: {} tokens across {} files ({:.1}% of 128k context)",
            Formatters::format_tokens(summary.total_tokens_cl100k)
                .bold()
                .yellow(),
            summary.total_files.bold(),
            summary.pct_of_128k.bold().magenta()
        );
        println!(
            "Estimated Turn Cost: {} per 100 prompt turns",
            Formatters::format_currency(summary.est_cost_per_100_turns)
                .bold()
                .green()
        );
    }

    pub fn render_shell_benchmark(bench: &ShellBenchmarkResult) {
        println!(
            "{}",
            "\n⚡ Subshell Spawn & Tool Execution Latency".bold().cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        println!(
            "  • Shell:                             {}",
            bench.shell_name.bold()
        );

        if let Some(err) = &bench.error {
            println!("  {}", format!("⚠️  {}", err).yellow());
            return;
        }

        let (Some(interactive), Some(non_interactive), Some(tax), Some(fifty)) = (
            bench.interactive_login_ms,
            bench.non_interactive_ms,
            bench.latency_tax_ms,
            bench.estimated_50_tool_calls_sec,
        ) else {
            println!("  {}", "⚠️  Benchmark did not complete.".yellow());
            return;
        };

        println!(
            "  • Interactive Login (`{} -lic`):     {}",
            bench.shell_name,
            Formatters::format_ms(interactive).bold().yellow()
        );
        println!(
            "  • Non-Interactive (`{} -c`):         {}",
            bench.shell_name,
            Formatters::format_ms(non_interactive).bold().green()
        );
        println!(
            "  • Single-Command Latency Tax:        {}",
            Formatters::format_ms(tax).bold().red()
        );
        println!(
            "  • Agent Tool Latency Tax (50 calls): {}",
            format!("+{:.1}s wasted", fifty).bold().red()
        );
        println!(
            "  • Status Rating:                     {}",
            bench.rating.map(|r| r.badge()).unwrap_or("—")
        );
        println!(
            "  • Agent Fast-Path Guard:             {}",
            if bench.has_agent_fast_path {
                "✅ Installed".green().to_string()
            } else {
                "❌ Missing (run `agentprof fix --shell`)".red().to_string()
            }
        );
        println!(
            "  {}",
            format!("(median of {} runs, after warm-up)", bench.iterations).dimmed()
        );
    }

    pub fn render_omz_report(report: &OmzProfileReport) {
        println!(
            "{}",
            "\n🐚 Oh My Zsh & Shell Plugin Latency Audit".bold().cyan()
        );
        println!("{}", "═".repeat(78).dimmed());

        if !report.is_omz_installed {
            println!("{}", "Oh My Zsh is not detected on this machine.".dimmed());
            return;
        }

        println!(
            "Total Shell Startup Time:   {}",
            Formatters::format_ms(report.total_shell_startup_ms)
                .bold()
                .yellow()
        );
        if let Some(theme) = &report.theme_name {
            println!("Theme:                      {}", theme.bold().blue());
        }
        let bytecode_label = if report.bytecode_compiled {
            "✅ Yes".green().to_string()
        } else {
            "❌ No (run `zcompile ~/.zshrc`)".yellow().to_string()
        };
        println!("Bytecode Compiled (.zwc):  {}", bytecode_label);

        if !report.plugins.is_empty() {
            println!("\n{}", "Plugin Startup Breakdown:".bold());
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec![
                    Cell::new("Plugin").fg(Color::Cyan),
                    Cell::new("Time").fg(Color::Cyan),
                    Cell::new("Share").fg(Color::Cyan),
                    Cell::new("Impact").fg(Color::Cyan),
                ]);

            for plugin in &report.plugins {
                let impact_cell = match plugin.latency_ms {
                    Some(ms) if ms > 100.0 => Cell::new("🚨 Critical").fg(Color::Red),
                    Some(ms) if ms > 30.0 => Cell::new("⚠️ Heavy").fg(Color::Yellow),
                    Some(_) => Cell::new("✅ Fast").fg(Color::Green),
                    None => Cell::new("❔ Not measured").fg(Color::DarkGrey),
                };

                table.add_row(vec![
                    Cell::new(&plugin.name),
                    Cell::new(
                        plugin
                            .latency_ms
                            .map(Formatters::format_ms)
                            .unwrap_or_else(|| "—".to_string()),
                    ),
                    Cell::new(if plugin.latency_ms.is_some() {
                        format!("{:.1}%", plugin.percentage_of_total)
                    } else {
                        "—".to_string()
                    }),
                    impact_cell,
                ]);
            }
            println!("{table}");
        }

        if !report.slow_hooks.is_empty() {
            println!(
                "\n{}",
                "Detected Slow External Evals / Initializers:".bold()
            );
            for hook in &report.slow_hooks {
                let timing = hook
                    .latency_ms
                    .map(|ms| format!("~{}", Formatters::format_ms(ms)))
                    .unwrap_or_else(|| "not measured".to_string());
                println!(
                    "  • {} ({}) -> {}",
                    hook.tool_name.bold().yellow(),
                    timing,
                    hook.suggestion.dimmed()
                );
            }
        }
    }

    pub fn render_workspace_audit(audit: &WorkspaceAuditReport) {
        println!("{}", "\n📁 Workspace Ignore & Security Guard".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        let claude_lbl = if audit.has_claudeignore {
            "✅ Present".green().to_string()
        } else {
            "❌ Missing".red().to_string()
        };
        let cursor_lbl = if audit.has_cursorignore {
            "✅ Present".green().to_string()
        } else {
            "❌ Missing".red().to_string()
        };
        let git_lbl = if audit.has_gitignore {
            "✅ Present".green().to_string()
        } else {
            "❌ Missing".yellow().to_string()
        };

        println!("  • .claudeignore:  {}", claude_lbl);
        println!("  • .cursorignore:  {}", cursor_lbl);
        println!("  • .gitignore:     {}", git_lbl);

        if !audit.secret_risks.is_empty() {
            println!(
                "\n{}",
                "🚨 Exposed Secrets Accessible to Agent Search Tools:"
                    .bold()
                    .red()
            );
            for s in &audit.secret_risks {
                println!(
                    "  • [{}] {} -> {}",
                    s.risk_level,
                    s.relative_path.bold(),
                    s.description.dimmed()
                );
            }
        }

        if !audit.heavy_directories.is_empty() {
            println!(
                "\n{}",
                "Unignored Heavy Build / Cache Directories:".bold().yellow()
            );
            for d in &audit.heavy_directories {
                let status = if d.is_ignored_by_claude {
                    "✅ Ignored"
                } else {
                    "❌ Unignored"
                };
                // The counter stops at a cap, so show "N+" rather than implying
                // the directory holds exactly N files.
                let count = if d.estimated_files >= crate::core::workspace_guard::FILE_COUNT_CAP {
                    format!("{}+", Formatters::format_tokens(d.estimated_files))
                } else {
                    Formatters::format_tokens(d.estimated_files)
                };
                println!(
                    "  • {} ({}) -> {} ({} files)",
                    d.relative_path.bold(),
                    d.directory_type.dimmed(),
                    status,
                    count
                );
            }
        }
    }
}
