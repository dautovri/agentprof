use std::io;
use std::path::Path;
use std::time::Duration;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::Terminal;

use crate::core::agent_platforms::AgentPlatformProfiler;
use crate::core::mcp_profiler::McpProfiler;
use crate::core::report_generator::ReportGenerator;
use crate::core::scanner::InstructionScanner;
use crate::core::session_history::SessionHistoryAnalyzer;
use crate::core::shell_bench::ShellBenchmarker;
use crate::core::skills_auditor::SkillsAuditor;
use crate::core::workspace_guard::WorkspaceGuard;
use crate::ui::formatters::Formatters;

pub struct TuiApp;

impl TuiApp {
    pub fn run(workspace_root: &Path) -> Result<()> {
        let health = ReportGenerator::calculate_health_score(workspace_root)?;
        let context = InstructionScanner::scan_workspace(workspace_root)?;
        let bench = ShellBenchmarker::run_benchmark(3)?;
        let mcp = McpProfiler::profile(workspace_root)?;
        let skills = SkillsAuditor::audit(workspace_root)?;
        let history = SessionHistoryAnalyzer::analyze()?;
        let guard = WorkspaceGuard::audit(workspace_root)?;
        let _agents = AgentPlatformProfiler::profile_all(workspace_root)?;

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut selected_tab = 0;
        let tab_titles = vec!["Overview", "Context", "MCP Schemas", "Skills", "Subshell", "History"];

        let res = (|| -> Result<()> {
            loop {
                terminal.draw(|f| {
                    let size = f.area();
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(3),
                            Constraint::Min(0),
                            Constraint::Length(3),
                        ])
                        .split(size);

                    // Header Tabs
                    let titles: Vec<Line> = tab_titles
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let style = if i == selected_tab {
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::Gray)
                            };
                            Line::from(vec![Span::raw(format!(" [{}] {} ", i + 1, t))]).style(style)
                        })
                        .collect();

                    let tabs = Tabs::new(titles)
                        .block(Block::default().borders(Borders::ALL).title(" ⚡ agentprof - AI Agent Workspace Optimizer "))
                        .select(selected_tab)
                        .highlight_style(Style::default().fg(Color::Yellow));
                    f.render_widget(tabs, chunks[0]);

                    // Body content based on selected tab
                    let body_text = match selected_tab {
                        0 => vec![
                            Line::from(vec![
                                Span::raw("  🎯 Health Score: "),
                                Span::styled(format!("{}/100 ({})", health.score, health.grade), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                            ]),
                            Line::from(""),
                            Line::from(vec![
                                Span::raw("  • Subshell Spawn Latency: "),
                                Span::styled(format!("{:.1}ms", bench.interactive_login_ms), Style::default().fg(Color::Yellow)),
                            ]),
                            Line::from(vec![
                                Span::raw("  • Context Instructions:   "),
                                Span::styled(format!("{} tokens ({} files)", Formatters::format_tokens(context.total_tokens_cl100k), context.total_files), Style::default().fg(Color::Cyan)),
                            ]),
                            Line::from(vec![
                                Span::raw("  • MCP Server Tool Load:   "),
                                Span::styled(format!("{} tokens ({} servers)", Formatters::format_tokens(mcp.total_estimated_tokens), mcp.total_servers), Style::default().fg(Color::Magenta)),
                            ]),
                            Line::from(vec![
                                Span::raw("  • Installed Agent Skills: "),
                                Span::styled(format!("{} skills ({} tokens)", skills.total_skills, Formatters::format_tokens(skills.total_tokens)), Style::default().fg(Color::Blue)),
                            ]),
                            Line::from(vec![
                                Span::raw("  • Unignored Build Caches: "),
                                Span::styled(format!("{} heavy folders", guard.total_unignored_heavy_dirs), Style::default().fg(Color::Red)),
                            ]),
                            Line::from(""),
                            Line::from(Span::styled("  💡 Press '1-6' to switch tabs, 'q' to quit", Style::default().fg(Color::DarkGray))),
                        ],
                        1 => {
                            let mut lines = vec![
                                Line::from(Span::styled(format!("Total Context: {} tokens across {} files", Formatters::format_tokens(context.total_tokens_cl100k), context.total_files), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                                Line::from(""),
                            ];
                            for file in context.files.iter().take(15) {
                                lines.push(Line::from(format!("  • {:<40} {:>6} tokens ({} lines)", file.relative_path, Formatters::format_tokens(file.tokens_cl100k), file.lines)));
                            }
                            lines
                        }
                        2 => {
                            let mut lines = vec![
                                Line::from(Span::styled(format!("Active MCP Tool Schemas ({} servers, {} tokens)", mcp.total_servers, Formatters::format_tokens(mcp.total_estimated_tokens)), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))),
                                Line::from(""),
                            ];
                            for s in &mcp.servers {
                                lines.push(Line::from(format!("  • {:<16} {:>6} tokens  {} (Source: {})", s.name, Formatters::format_tokens(s.estimated_schema_tokens), s.status, s.config_source)));
                            }
                            lines
                        }
                        3 => {
                            let mut lines = vec![
                                Line::from(Span::styled(format!("Skills Audit ({} skills, {} collisions detected)", skills.total_skills, skills.collisions.len()), Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))),
                                Line::from(""),
                            ];
                            for c in skills.collisions.iter().take(8) {
                                lines.push(Line::from(format!("  ⚠️ Collision on '{}': {}", c.keyword, c.colliding_skills.join(", "))));
                            }
                            lines
                        }
                        4 => vec![
                            Line::from(Span::styled("Subshell Spawn Benchmarks", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                            Line::from(""),
                            Line::from(format!("  • Interactive Login Shell:     {:.1}ms", bench.interactive_login_ms)),
                            Line::from(format!("  • Non-Interactive Subshell:    {:.1}ms", bench.non_interactive_ms)),
                            Line::from(format!("  • Tool Execution Latency Tax:  {:.1}ms per command", bench.latency_tax_ms)),
                            Line::from(format!("  • Projected 50 Tool Call Loss: +{:.1}s wasted waiting on .zshrc", bench.estimated_50_tool_calls_sec)),
                            Line::from(format!("  • Agent Fast-Path Guard:       {}", if bench.has_agent_fast_path { "Active" } else { "Missing" })),
                        ],
                        5 => vec![
                            Line::from(Span::styled("Agent Session History & Diagnostics", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
                            Line::from(""),
                            Line::from(format!("  • Recorded Sessions:      {}", history.total_sessions_found)),
                            Line::from(format!("  • Total Lifetime Turns:   {}", history.total_turns)),
                            Line::from(format!("  • Est. Historical Spend:  ${:.2}", history.total_estimated_cost_usd)),
                            Line::from(format!("  • Loop Thrash Incidents:  {}", history.loop_thrash_incidents)),
                        ],
                        _ => vec![Line::from("Unknown Tab")],
                    };

                    let body = Paragraph::new(body_text)
                        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", tab_titles[selected_tab])))
                        .wrap(Wrap { trim: true });
                    f.render_widget(body, chunks[1]);

                    // Footer status
                    let footer = Paragraph::new(" [Q/Esc] Quit  │  [Tab/←/→] Switch Tab  │  [1-6] Direct Select  │  agentprof v0.2.0 ")
                        .style(Style::default().fg(Color::DarkGray))
                        .block(Block::default().borders(Borders::ALL));
                    f.render_widget(footer, chunks[2]);
                })?;

                if event::poll(Duration::from_millis(150))? {
                    if let Event::Key(key) = event::read()? {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Tab | KeyCode::Right => {
                                selected_tab = (selected_tab + 1) % tab_titles.len();
                            }
                            KeyCode::Left => {
                                selected_tab = if selected_tab == 0 { tab_titles.len() - 1 } else { selected_tab - 1 };
                            }
                            KeyCode::Char('1') => selected_tab = 0,
                            KeyCode::Char('2') => selected_tab = 1,
                            KeyCode::Char('3') => selected_tab = 2,
                            KeyCode::Char('4') => selected_tab = 3,
                            KeyCode::Char('5') => selected_tab = 4,
                            KeyCode::Char('6') => selected_tab = 5,
                            _ => {}
                        }
                    }
                }
            }
            Ok(())
        })();

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        res
    }
}
