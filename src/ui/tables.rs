use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use owo_colors::OwoColorize;

use crate::core::agent_platforms::AgentPlatformProfile;
use crate::core::mcp_profiler::McpProfileReport;
use crate::core::omz_profiler::OmzProfileReport;
use crate::core::scanner::WorkspaceContextSummary;
use crate::core::session_history::SessionHistoryReport;
use crate::core::shell_bench::ShellBenchmarkResult;
use crate::core::skills_auditor::SkillsAuditReport;
use crate::core::workspace_guard::WorkspaceAuditReport;
use crate::ui::formatters::Formatters;

pub struct TableRenderer;

impl TableRenderer {
    pub fn render_mcp_report(report: &McpProfileReport) {
        println!("{}", "\n🔌 Model Context Protocol (MCP) Tool Schema Profiler".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        if report.servers.is_empty() {
            println!("{}", "No active MCP servers found in inspected configurations.".dimmed());
            return;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                Cell::new("MCP Server").fg(Color::Cyan),
                Cell::new("Config Source").fg(Color::Cyan),
                Cell::new("Tools (est)").fg(Color::Cyan),
                Cell::new("Schema Tokens").fg(Color::Cyan),
                Cell::new("Status").fg(Color::Cyan),
            ]);

        for s in &report.servers {
            let status_cell = if s.estimated_schema_tokens > 3000 {
                Cell::new(&s.status).fg(Color::Red)
            } else if s.estimated_schema_tokens > 1500 {
                Cell::new(&s.status).fg(Color::Yellow)
            } else {
                Cell::new(&s.status).fg(Color::Green)
            };

            table.add_row(vec![
                Cell::new(&s.name),
                Cell::new(&s.config_source),
                Cell::new(s.estimated_tool_count.to_string()),
                Cell::new(Formatters::format_tokens(s.estimated_schema_tokens)),
                status_cell,
            ]);
        }

        println!("{table}");

        println!(
            "Total MCP Schema Load: {} tokens across {} servers ({:.1}% of 128k context)",
            Formatters::format_tokens(report.total_estimated_tokens).bold().yellow(),
            report.total_servers.bold(),
            report.pct_of_128k_context.bold().magenta()
        );
        println!(
            "Estimated Turn Schema Cost: {} per 100 turns",
            Formatters::format_currency(report.est_cost_per_100_turns).bold().green()
        );

        for rec in &report.recommendations {
            println!("💡 Recommendation: {}", rec.dimmed());
        }
    }

    pub fn render_skills_report(report: &SkillsAuditReport) {
        println!("{}", "\n🎯 Agent Skills & Keyword Collision Auditor".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        println!(
            "Total Installed Skills:  {} skills",
            report.total_skills.bold().cyan()
        );
        println!(
            "Total Skills Corpus:     {} tokens (Avg: {} tokens/skill)",
            Formatters::format_tokens(report.total_tokens).bold().yellow(),
            report.average_tokens.to_string().dimmed()
        );
        println!(
            "Bloated Skills (>2.5k):  {}",
            if report.bloated_skills_count > 0 { format!("🚨 {} skills", report.bloated_skills_count).red().to_string() } else { "✅ None".green().to_string() }
        );

        if !report.collisions.is_empty() {
            println!("\n{}", "⚠️ Trigger Keyword Collisions (Skills competing for identical intents):".bold().yellow());
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
                    format!("{}, +{} more", c.colliding_skills[..4].join(", "), c.colliding_skills.len() - 4)
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
                println!("  • {:<28} -> {} tokens ({} lines) [{}]", s.name.bold(), Formatters::format_tokens(s.tokens).yellow(), s.lines, badge);
            }
        }
    }

    pub fn render_session_history(report: &SessionHistoryReport) {
        println!("{}", "\n📉 Agent Session History & Loop Thrash Diagnostics".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        println!("  • Recorded Sessions Found: {}", report.total_sessions_found.bold());
        println!("  • Total Conversation Turns: {}", report.total_turns.bold().cyan());
        println!(
            "  • Lifetime Token Usage:    {}",
            Formatters::format_tokens(report.total_tokens_used).bold().yellow()
        );
        println!(
            "  • Est. Lifetime AI Spend:  {}",
            Formatters::format_currency(report.total_estimated_cost_usd).bold().green()
        );
        println!(
            "  • Loop Thrash Incidents:   {}",
            if report.loop_thrash_incidents > 0 { format!("🚨 {} detected", report.loop_thrash_incidents).red().to_string() } else { "✅ 0 detected".green().to_string() }
        );

        if !report.tool_usage_distribution.is_empty() {
            println!("\n{}", "Most Frequently Invoked Agent Tools:".bold());
            for (tool, count) in report.tool_usage_distribution.iter().take(6) {
                println!("  • {:<16} -> {} calls", tool.bold().blue(), count);
            }
        }

        for rec in &report.recommendations {
            println!("💡 Recommendation: {}", rec.dimmed());
        }
    }

    pub fn render_agent_profiles(profiles: &[AgentPlatformProfile]) {
        println!("{}", "\n🤖 Agent Platform Profiler (OpenCode / Claude / Grok)".bold().cyan());
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

            let config_count_str = format!("{} files ({} skills)", p.config_files.len(), p.total_skills_count);
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
                println!("\n{} [{} Context Details]:", "▶".bold().blue(), p.platform.name().bold());
                for f in &p.config_files {
                    let scope = if f.is_global { "Global" } else { "Workspace" };
                    println!("  • {:<24} ({}) -> {} tokens ({} lines)", f.name.bold(), scope.dimmed(), Formatters::format_tokens(f.tokens).yellow(), f.lines);
                }
                if !p.detected_mcp_servers.is_empty() {
                    println!("  • MCP Servers: {}", p.detected_mcp_servers.join(", ").cyan());
                }
                if p.total_skills_count > 0 {
                    println!("  • Discovered Skills: {} skills ({} total tokens)", p.total_skills_count, Formatters::format_tokens(p.total_skills_tokens));
                }
                for rec in &p.recommendations {
                    println!("  💡 Recommendation: {}", rec.dimmed());
                }
            }
        }
    }

    pub fn render_context_summary(summary: &WorkspaceContextSummary) {
        println!("{}", "\n🤖 AI Agent Context & Instruction Budget".bold().cyan());
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
                crate::core::scanner::HealthStatus::Optimal => Cell::new("Optimal").fg(Color::Green),
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
            Formatters::format_tokens(summary.total_tokens_cl100k).bold().yellow(),
            summary.total_files.bold(),
            summary.pct_of_128k.bold().magenta()
        );
        println!(
            "Estimated Turn Cost: {} per 100 prompt turns",
            Formatters::format_currency(summary.est_cost_per_100_turns).bold().green()
        );
    }

    pub fn render_shell_benchmark(bench: &ShellBenchmarkResult) {
        println!("{}", "\n⚡ Subshell Spawn & Tool Execution Latency".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        println!("  • Interactive Login (`zsh -lic`):    {}", Formatters::format_ms(bench.interactive_login_ms).bold().yellow());
        println!("  • Non-Interactive (`zsh -c`):        {}", Formatters::format_ms(bench.non_interactive_ms).bold().green());
        println!("  • Single-Command Latency Tax:        {}", Formatters::format_ms(bench.latency_tax_ms).bold().red());
        println!(
            "  • Agent Tool Latency Tax (50 calls): {}",
            format!("+{:.1}s wasted", bench.estimated_50_tool_calls_sec).bold().red()
        );
        println!("  • Status Rating:                     {}", bench.rating.badge());
        let fast_path_label = if bench.has_agent_fast_path {
            "✅ Enabled".green().to_string()
        } else {
            "❌ Missing (run `agentprof fix --shell`)".red().to_string()
        };
        println!("  • Agent Fast-Path Guard:             {}", fast_path_label);
    }

    pub fn render_omz_report(report: &OmzProfileReport) {
        println!("{}", "\n🐚 Oh My Zsh & Shell Plugin Latency Audit".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        if !report.is_omz_installed {
            println!("{}", "Oh My Zsh is not detected on this machine.".dimmed());
            return;
        }

        println!(
            "Total Shell Startup Time:   {}",
            Formatters::format_ms(report.total_shell_startup_ms).bold().yellow()
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
                let impact_cell = if plugin.latency_ms > 100.0 {
                    Cell::new("🚨 Critical").fg(Color::Red)
                } else if plugin.latency_ms > 30.0 {
                    Cell::new("⚠️ Heavy").fg(Color::Yellow)
                } else {
                    Cell::new("✅ Fast").fg(Color::Green)
                };

                table.add_row(vec![
                    Cell::new(&plugin.name),
                    Cell::new(Formatters::format_ms(plugin.latency_ms)),
                    Cell::new(format!("{:.1}%", plugin.percentage_of_total)),
                    impact_cell,
                ]);
            }
            println!("{table}");
        }

        if !report.slow_hooks.is_empty() {
            println!("\n{}", "Detected Slow External Evals / Initializers:".bold());
            for hook in &report.slow_hooks {
                println!("  • {} (~{}) -> {}", hook.tool_name.bold().yellow(), Formatters::format_ms(hook.latency_ms), hook.suggestion.dimmed());
            }
        }
    }

    pub fn render_workspace_audit(audit: &WorkspaceAuditReport) {
        println!("{}", "\n📁 Workspace Ignore & Security Guard".bold().cyan());
        println!("{}", "═".repeat(78).dimmed());

        let claude_lbl = if audit.has_claudeignore { "✅ Present".green().to_string() } else { "❌ Missing".red().to_string() };
        let cursor_lbl = if audit.has_cursorignore { "✅ Present".green().to_string() } else { "❌ Missing".red().to_string() };
        let git_lbl = if audit.has_gitignore { "✅ Present".green().to_string() } else { "❌ Missing".yellow().to_string() };

        println!("  • .claudeignore:  {}", claude_lbl);
        println!("  • .cursorignore:  {}", cursor_lbl);
        println!("  • .gitignore:     {}", git_lbl);

        if !audit.secret_risks.is_empty() {
            println!("\n{}", "🚨 Exposed Secrets Accessible to Agent Search Tools:".bold().red());
            for s in &audit.secret_risks {
                println!("  • [{}] {} -> {}", s.risk_level, s.relative_path.bold(), s.description.dimmed());
            }
        }

        if !audit.heavy_directories.is_empty() {
            println!("\n{}", "Unignored Heavy Build / Cache Directories:".bold().yellow());
            for d in &audit.heavy_directories {
                let status = if d.is_ignored_by_claude { "✅ Ignored" } else { "❌ Unignored" };
                println!("  • {} ({}) -> {} (~{} files)", d.relative_path.bold(), d.directory_type.dimmed(), status, d.estimated_files);
            }
        }
    }
}
