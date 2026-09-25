use std::io::{self, IsTerminal, Stdout};
use std::panic;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
};

use crate::core::mcp_profiler::{McpProfileReport, McpProfiler};
use crate::core::report_generator::{ReportGenerator, WorkspaceHealthScore};
use crate::core::scanner::{InstructionScanner, WorkspaceContextSummary};
use crate::core::session_history::{SessionHistoryAnalyzer, SessionHistoryReport};
use crate::core::shell_bench::{ShellBenchmarkResult, ShellBenchmarker};
use crate::core::skills_auditor::{SkillsAuditReport, SkillsAuditor};
use crate::core::workspace_guard::{WorkspaceAuditReport, WorkspaceGuard};
use crate::ui::formatters::Formatters;

const TAB_TITLES: [&str; 6] = [
    "Overview",
    "Context",
    "MCP Schemas",
    "Skills",
    "Subshell",
    "History",
];

/// Restores the terminal on drop, including during an unwinding panic.
///
/// Without this, a panic anywhere inside the draw loop left the user's terminal
/// in raw mode on the alternate screen with no cursor — requiring a blind
/// `reset` to recover.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(TerminalGuard { terminal })
    }

    fn restore() {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

struct AppData {
    health: WorkspaceHealthScore,
    context: WorkspaceContextSummary,
    bench: ShellBenchmarkResult,
    mcp: McpProfileReport,
    skills: SkillsAuditReport,
    history: SessionHistoryReport,
    guard: WorkspaceAuditReport,
}

pub struct TuiApp;

impl TuiApp {
    pub fn run(workspace_root: &Path) -> Result<()> {
        // Fail with an explanation rather than a bare "Device not configured"
        // when the dashboard is piped or redirected.
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            anyhow::bail!(
                "`agentprof tui` needs an interactive terminal. \
                 For piped or scripted use, run `agentprof scan --json` or `agentprof report --json` instead."
            );
        }

        // Collection takes a few seconds; say so before the screen goes blank.
        println!("⚡ Collecting workspace, shell, MCP and skills data...");

        // Each report is gathered exactly once and the health score is derived
        // from these same values. Previously the TUI computed the health score
        // (which internally re-ran the scan, benchmark, guard and MCP profile)
        // and then re-ran all four again, doubling an already slow startup.
        let context = InstructionScanner::scan_workspace(workspace_root)?;
        let bench = ShellBenchmarker::run_benchmark(3)?;
        let guard = WorkspaceGuard::audit(workspace_root)?;
        let mcp = McpProfiler::profile(workspace_root)?;
        let health = ReportGenerator::score_parts(&context, &bench, &guard, &mcp);
        let skills = SkillsAuditor::audit(workspace_root)?;
        let history = SessionHistoryAnalyzer::analyze()?;

        let data = AppData {
            health,
            context,
            bench,
            mcp,
            skills,
            history,
            guard,
        };

        // A panic inside ratatui must not leave the terminal wrecked.
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            TerminalGuard::restore();
            default_hook(info);
        }));

        let mut guard_term = TerminalGuard::new()?;
        let result = Self::event_loop(&mut guard_term.terminal, &data);
        drop(guard_term);
        let _ = panic::take_hook();

        result
    }

    fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, data: &AppData) -> Result<()> {
        let mut selected_tab = 0usize;
        let mut scroll: u16 = 0;

        loop {
            let body_lines = Self::body(data, selected_tab);
            let total_lines = body_lines.len();

            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
                    .split(f.area());

                let titles: Vec<Line> = TAB_TITLES
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let style = if i == selected_tab {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        };
                        Line::from(Span::raw(format!(" [{}] {} ", i + 1, t))).style(style)
                    })
                    .collect();

                let tabs = Tabs::new(titles)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" ⚡ agentprof — AI Agent Workspace Optimizer "),
                    )
                    .select(selected_tab)
                    .highlight_style(Style::default().fg(Color::Yellow));
                f.render_widget(tabs, chunks[0]);

                let viewport = chunks[1].height.saturating_sub(2) as usize;
                let body = Paragraph::new(body_lines.clone())
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" {} ", TAB_TITLES[selected_tab])),
                    )
                    .wrap(Wrap { trim: false })
                    .scroll((scroll, 0));
                f.render_widget(body, chunks[1]);

                if total_lines > viewport {
                    let mut state = ScrollbarState::new(total_lines.saturating_sub(viewport))
                        .position(scroll as usize);
                    f.render_stateful_widget(
                        Scrollbar::new(ScrollbarOrientation::VerticalRight),
                        chunks[1],
                        &mut state,
                    );
                }

                let footer = Paragraph::new(format!(
                    " [q] Quit  │  [←/→/Tab] Tab  │  [1-6] Jump  │  [↑/↓/PgUp/PgDn] Scroll  │  agentprof v{} ",
                    env!("CARGO_PKG_VERSION")
                ))
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL));
                f.render_widget(footer, chunks[2]);
            })?;

            let viewport = terminal.size()?.height.saturating_sub(8) as usize;
            let max_scroll = total_lines.saturating_sub(viewport) as u16;

            if !event::poll(Duration::from_millis(150))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            // Terminals that report key releases would otherwise action every
            // keystroke twice.
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Tab | KeyCode::Right => {
                    selected_tab = (selected_tab + 1) % TAB_TITLES.len();
                    scroll = 0;
                }
                KeyCode::BackTab | KeyCode::Left => {
                    selected_tab = selected_tab.checked_sub(1).unwrap_or(TAB_TITLES.len() - 1);
                    scroll = 0;
                }
                KeyCode::Char(c @ '1'..='6') => {
                    selected_tab = c as usize - '1' as usize;
                    scroll = 0;
                }
                KeyCode::Down => scroll = scroll.saturating_add(1).min(max_scroll),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => scroll = scroll.saturating_add(10).min(max_scroll),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                KeyCode::Home => scroll = 0,
                KeyCode::End => scroll = max_scroll,
                _ => {}
            }
        }
        Ok(())
    }

    fn body(data: &AppData, tab: usize) -> Vec<Line<'static>> {
        match tab {
            0 => Self::overview(data),
            1 => Self::context_tab(data),
            2 => Self::mcp_tab(data),
            3 => Self::skills_tab(data),
            4 => Self::subshell_tab(data),
            5 => Self::history_tab(data),
            _ => vec![Line::from("Unknown tab")],
        }
    }

    fn overview(data: &AppData) -> Vec<Line<'static>> {
        let h = &data.health;
        let color = match (h.score * 100) / h.max_score.max(1) {
            80..=100 => Color::Green,
            60..=79 => Color::Yellow,
            _ => Color::Red,
        };

        let mut lines = vec![
            Line::from(vec![
                Span::raw("  🎯 Health Score: "),
                Span::styled(
                    format!("{}/{} ({})", h.score, h.max_score, h.grade),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for c in &h.categories {
            lines.push(Line::from(format!(
                "  • {:<26} {:>2}/{:<3} {}",
                c.name,
                c.score,
                c.max,
                c.detail.replace('`', "")
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "  • Installed Agent Skills:  {} skills ({} tokens always loaded)",
            data.skills.total_skills,
            Formatters::format_tokens(data.skills.always_loaded_tokens)
        )));
        lines.push(Line::from(format!(
            "  • Unignored Build Caches:  {} heavy folder(s)",
            data.guard.total_unignored_heavy_dirs
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press 1-6 to switch tabs, ↑/↓ to scroll, q to quit",
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    fn context_tab(data: &AppData) -> Vec<Line<'static>> {
        let c = &data.context;
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "Always loaded: {} tokens · on demand: {} tokens · {} file(s)",
                    Formatters::format_tokens(c.total_tokens_cl100k),
                    Formatters::format_tokens(c.conditional_tokens_cl100k),
                    c.total_files
                ),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        if c.files.is_empty() {
            lines.push(Line::from(
                "  No AI instruction files found in this workspace.",
            ));
        }
        for file in &c.files {
            lines.push(Line::from(format!(
                "  • {:<48} {:>8} tokens ({} lines) — {}",
                file.relative_path,
                Formatters::format_tokens(file.tokens_cl100k),
                file.lines,
                file.load_condition.as_deref().unwrap_or("every session")
            )));
        }
        lines
    }

    fn mcp_tab(data: &AppData) -> Vec<Line<'static>> {
        let m = &data.mcp;
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "{} server(s) enabled — {} measured, heaviest agent loads {} tokens/turn",
                    m.enabled_servers,
                    m.measured_server_count,
                    Formatters::format_tokens(m.max_upfront_tokens)
                ),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        for c in &m.clients {
            lines.push(Line::from(format!(
                "  {:<16} {:>8} tokens/turn  {}",
                c.client,
                Formatters::format_tokens(c.upfront_tokens),
                c.loading
            )));
        }
        lines.push(Line::from(""));
        for s in &m.servers {
            lines.push(Line::from(format!(
                "  • {:<20} {:<14} {:>8} tokens  {:<22} {}",
                s.name,
                s.client,
                s.schema_tokens
                    .map(Formatters::format_tokens)
                    .unwrap_or_else(|| "—".to_string()),
                s.status,
                s.scope
            )));
        }
        if m.measured_server_count < m.enabled_servers {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Run `agentprof mcp --probe` to measure the unmeasured servers.",
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines
    }

    fn skills_tab(data: &AppData) -> Vec<Line<'static>> {
        let s = &data.skills;
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "{} skills · {} tokens always loaded · {} on invocation · {} collision(s)",
                    s.total_skills,
                    Formatters::format_tokens(s.always_loaded_tokens),
                    Formatters::format_tokens(s.total_tokens),
                    s.collisions.len()
                ),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Heaviest skills:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ];
        for skill in s.top_heavy_skills.iter().take(15) {
            lines.push(Line::from(format!(
                "  • {:<32} {:>8} tokens",
                skill.name,
                Formatters::format_tokens(skill.tokens)
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Trigger collisions:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for c in &s.collisions {
            lines.push(Line::from(format!(
                "  ⚠️ '{}' → {}",
                c.keyword,
                c.colliding_skills.join(", ")
            )));
        }
        lines
    }

    fn subshell_tab(data: &AppData) -> Vec<Line<'static>> {
        let b = &data.bench;
        let mut lines = vec![
            Line::from(Span::styled(
                format!("Subshell spawn benchmark ({})", b.shell_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        if let Some(err) = &b.error {
            lines.push(Line::from(format!("  ⚠️  {}", err)));
            return lines;
        }

        let fmt = |v: Option<f64>| {
            v.map(|x| format!("{:.1}ms", x))
                .unwrap_or_else(|| "—".to_string())
        };
        lines.push(Line::from(format!(
            "  • Bare spawn (-c):              {}",
            fmt(b.non_interactive_ms)
        )));
        lines.push(Line::from(format!(
            "  • Login shell (-lc):            {}",
            fmt(b.login_ms)
        )));
        lines.push(Line::from(format!(
            "  • Interactive login (-lic):     {}",
            fmt(b.interactive_login_ms)
        )));
        lines.push(Line::from(format!(
            "  • Claude Code snapshot replay:  {}",
            fmt(b.claude_snapshot.as_ref().map(|s| s.replay_ms))
        )));
        if let Some(snapshot) = &b.codex_snapshot {
            lines.push(Line::from(format!(
                "  • Codex snapshot replay:        {}",
                fmt(Some(snapshot.replay_ms))
            )));
        }
        lines.push(Line::from(format!(
            "  • Per-command agent overhead:   {} {}",
            fmt(b.per_command_tax_ms),
            b.per_command_tax_source
                .as_deref()
                .map(|s| format!("({})", s))
                .unwrap_or_default()
        )));
        lines.push(Line::from(format!(
            "  • Across 50 commands:           {}",
            b.estimated_50_tool_calls_sec
                .map(|s| format!("+{:.1}s", s))
                .unwrap_or_else(|| "—".to_string())
        )));
        lines.push(Line::from(format!(
            "  • Rating:                       {}",
            b.rating.map(|r| r.badge()).unwrap_or("—")
        )));
        lines.push(Line::from(format!(
            "  • Agent fast-path guard:        {}",
            if b.has_agent_fast_path {
                "installed"
            } else {
                "missing"
            }
        )));
        lines.push(Line::from(format!(
            "  • Method:                       median of {} runs after warm-up",
            b.iterations
        )));
        lines
    }

    fn history_tab(data: &AppData) -> Vec<Line<'static>> {
        let h = &data.history;
        if h.total_sessions_found == 0 {
            return vec![Line::from("  No agent session transcripts found.")];
        }
        let mut lines = vec![
            Line::from(Span::styled(
                "Agent session history",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!(
                "  • Transcripts:        {} analyzed of {} found",
                h.sessions_analyzed, h.total_sessions_found
            )),
            Line::from(format!("  • Human turns:        {}", h.total_turns)),
            Line::from(format!(
                "  • Input / output:     {} / {}",
                Formatters::format_tokens(h.usage.input_tokens),
                Formatters::format_tokens(h.usage.output_tokens)
            )),
            Line::from(format!(
                "  • Cache write / read: {} / {}",
                Formatters::format_tokens(h.usage.cache_creation_tokens),
                Formatters::format_tokens(h.usage.cache_read_tokens)
            )),
            Line::from(format!(
                "  • Est. spend:         {} ({})",
                Formatters::format_currency(h.total_estimated_cost_usd),
                h.pricing_label
            )),
            Line::from(format!(
                "  • Loop thrash:        {}",
                h.loop_thrash_incidents
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Most used tools:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        ];
        for (tool, count) in h.tool_usage_distribution.iter().take(12) {
            lines.push(Line::from(format!("  • {:<32} {} calls", tool, count)));
        }
        lines
    }
}
