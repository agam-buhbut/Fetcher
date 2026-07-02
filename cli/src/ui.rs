use std::collections::VecDeque;
use std::fmt::Write as _;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table, TableState, Tabs,
};
use ratatui::Frame;
use taskmgr_core::startup::AutostartScope;
use taskmgr_core::{human_bytes, opt_bytes, truncate, ServiceScope, SortColumn, SortOrder};

use crate::app::{App, Tab};

const ALL_TABS: [Tab; 4] = [
    Tab::Processes,
    Tab::Performance,
    Tab::Startup,
    Tab::Services,
];

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tabs
            Constraint::Min(0),    // body
            Constraint::Length(1), // status
        ])
        .split(f.area());

    draw_tabs(f, layout[0], app);
    match app.tab {
        Tab::Processes => draw_processes(f, layout[1], app),
        Tab::Performance => draw_performance(f, layout[1], app),
        Tab::Startup => draw_startup(f, layout[1], app),
        Tab::Services => draw_services(f, layout[1], app),
    }
    draw_status(f, layout[2], app);
}

fn tab_index(tab: Tab) -> usize {
    ALL_TABS.iter().position(|t| *t == tab).unwrap_or(0)
}

fn draw_tabs(f: &mut Frame<'_>, area: Rect, app: &App) {
    let titles: Vec<Line<'_>> = ALL_TABS
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" {}  {} ", i + 1, t.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Task Manager"))
        .select(tab_index(app.tab))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).bold());
    f.render_widget(tabs, area);
}

fn draw_status(f: &mut Frame<'_>, area: Rect, app: &App) {
    let text: String = if app.filter_active {
        format!("/{}", app.filter)
    } else if let Some((_, msg)) = &app.status {
        msg.clone()
    } else {
        match app.tab {
            Tab::Processes => {
                "[1-4] tab  [↑↓/jk] select  [s/m/p/n/d/w] sort  [x] kill  [X] force  [/] filter  [q] quit"
                    .into()
            }
            Tab::Performance => "[1-4] tab  [q] quit".into(),
            Tab::Startup => "[1-4] tab  [↑↓] select  [space] toggle  [r] refresh  [q] quit".into(),
            Tab::Services => {
                "[1-4] tab  [↑↓]  [s] start  [S] stop  [R] restart  [e] enable  [E] disable  [u] user/sys  [q] quit"
                    .into()
            }
        }
    };
    let p = Paragraph::new(text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

fn draw_performance(f: &mut Frame<'_>, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    let cpu_pct = app
        .snapshot
        .cpu
        .as_ref()
        .map_or(0, |c| c.global_usage as u16)
        .min(100);
    let mem_pct = app
        .snapshot
        .memory
        .as_ref()
        .map_or(0, |m| m.used_percent() as u16)
        .min(100);

    let cpu_label = match &app.snapshot.cpu {
        Some(c) => format!("CPU  {:.1}%   ({} cores)", c.global_usage, c.per_core.len()),
        None => "CPU".into(),
    };
    let mem_label = match &app.snapshot.memory {
        Some(m) => {
            let mut label = format!(
                "Memory  {} / {}",
                human_bytes(m.used_bytes),
                human_bytes(m.total_bytes)
            );
            if m.swap_total_bytes > 0 {
                // Infallible on String; let _ = keeps unused_must_use happy.
                let _ = write!(
                    label,
                    "   ·   Swap  {} / {}",
                    human_bytes(m.swap_used_bytes),
                    human_bytes(m.swap_total_bytes)
                );
            }
            label
        }
        None => "Memory".into(),
    };

    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(cpu_label))
            .gauge_style(Style::default().fg(Color::Cyan))
            .percent(cpu_pct),
        chunks[0],
    );
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(mem_label))
            .gauge_style(Style::default().fg(Color::Magenta))
            .percent(mem_pct),
        chunks[1],
    );

    // Percent graphs get a fixed 0–100 scale; auto-scaling them makes an
    // idle 4% CPU render as a full-height wall. Byte graphs auto-scale.
    spark(
        f,
        chunks[2],
        "CPU %",
        &app.cpu_history,
        Color::Cyan,
        Some(100),
        |last| format!("({last}%)"),
    );
    spark(
        f,
        chunks[3],
        "Memory %",
        &app.mem_history,
        Color::Magenta,
        Some(100),
        |last| format!("({last}%)"),
    );
    spark(
        f,
        chunks[4],
        "Disk B/s (R+W)",
        &app.disk_history,
        Color::Yellow,
        None,
        |last| format!("({}/s)", human_bytes(last)),
    );
    spark(
        f,
        chunks[5],
        "Net B/s (RX+TX)",
        &app.net_history,
        Color::Green,
        None,
        |last| format!("({}/s)", human_bytes(last)),
    );
}

#[allow(clippy::too_many_arguments)]
fn spark<F>(
    f: &mut Frame<'_>,
    area: Rect,
    label: &str,
    data: &VecDeque<u64>,
    color: Color,
    fixed_max: Option<u64>,
    format_last: F,
) where
    F: Fn(u64) -> String,
{
    let last = data.back().copied().unwrap_or(0);
    let title = format!("{label}  {}", format_last(last));
    let max = fixed_max
        .unwrap_or_else(|| data.iter().copied().max().unwrap_or(1))
        .max(1);
    let slice: Vec<u64> = data.iter().copied().collect();
    let s = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(&slice)
        .max(max)
        .style(Style::default().fg(color));
    f.render_widget(s, area);
}

fn draw_processes(f: &mut Frame<'_>, area: Rect, app: &mut App) {
    let arrow = match app.sort.order {
        SortOrder::Ascending => "↑",
        SortOrder::Descending => "↓",
    };
    let label = |col: SortColumn, base: &str| -> String {
        if app.sort.column == col {
            format!("{base}{arrow}")
        } else {
            base.to_string()
        }
    };

    let header = Row::new(vec![
        Cell::from(label(SortColumn::Pid, "PID")),
        Cell::from("USER"),
        Cell::from(label(SortColumn::Name, "NAME")),
        Cell::from("ST"),
        Cell::from(label(SortColumn::Cpu, "CPU%")),
        Cell::from(label(SortColumn::Memory, "MEM")),
        Cell::from(label(SortColumn::DiskRead, "DiskR/s")),
        Cell::from(label(SortColumn::DiskWrite, "DiskW/s")),
    ])
    .style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::White)
            .bg(Color::DarkGray),
    );

    // Cells own their strings, so the snapshot borrow ends with this block
    // and the persistent table state below can be borrowed mutably.
    let (rows, row_count) = {
        let rows_data = app.filtered_processes();
        let rows: Vec<Row<'_>> = rows_data
            .iter()
            .map(|r| {
                Row::new(vec![
                    Cell::from(r.pid.to_string()),
                    Cell::from(truncate(&r.user, 10)),
                    Cell::from(truncate(&r.name, 30)),
                    Cell::from(r.status.short()),
                    Cell::from(format!("{:>5.1}", r.cpu_percent)),
                    Cell::from(human_bytes(r.memory_bytes)),
                    Cell::from(rate_str(r.disk_read_per_sec)),
                    Cell::from(rate_str(r.disk_write_per_sec)),
                ])
            })
            .collect();
        (rows, rows_data.len())
    };

    let widths = [
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(30),
        Constraint::Length(2),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
    ];

    let title = format!("Processes ({row_count})");
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
        .highlight_symbol("▶ ");

    select_row(&mut app.proc_table, app.proc_selected, row_count);
    f.render_stateful_widget(table, area, &mut app.proc_table);
}

fn draw_startup(f: &mut Frame<'_>, area: Rect, app: &mut App) {
    let header = Row::new(vec!["ON", "NAME", "SCOPE", "EXEC"]).style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    let rows: Vec<Row<'_>> = app
        .autostart
        .iter()
        .map(|e| {
            let on = if e.enabled { "[x]" } else { "[ ]" };
            let scope = match e.scope {
                AutostartScope::User => "user",
                AutostartScope::System => "system",
            };
            Row::new(vec![
                Cell::from(on),
                Cell::from(truncate(&e.name, 28)),
                Cell::from(scope),
                Cell::from(truncate(&e.exec, 60)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(4),
        Constraint::Length(30),
        Constraint::Length(8),
        Constraint::Min(20),
    ];
    let title = format!("Startup ({})", app.autostart.len());
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
        .highlight_symbol("▶ ");
    select_row(
        &mut app.startup_table,
        app.startup_selected,
        app.autostart.len(),
    );
    f.render_stateful_widget(table, area, &mut app.startup_table);
}

fn draw_services(f: &mut Frame<'_>, area: Rect, app: &mut App) {
    let header = Row::new(vec!["NAME", "ACTIVE", "SUB", "DESCRIPTION"]).style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    let rows: Vec<Row<'_>> = app
        .services
        .iter()
        .map(|u| {
            let active_color = match u.active_state.as_str() {
                "active" => Color::Green,
                "failed" => Color::Red,
                "inactive" => Color::DarkGray,
                _ => Color::Yellow,
            };
            Row::new(vec![
                Cell::from(truncate(&u.name, 38)),
                Cell::from(Span::styled(
                    u.active_state.clone(),
                    Style::default().fg(active_color),
                )),
                Cell::from(truncate(&u.sub_state, 12)),
                Cell::from(truncate(&u.description, 60)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(40),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Min(20),
    ];
    let scope_label = match app.services_scope {
        ServiceScope::User => "user",
        ServiceScope::System => "system",
    };
    let title = format!("Services [{scope_label}] ({})", app.services.len());
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
        .highlight_symbol("▶ ");
    select_row(
        &mut app.services_table,
        app.services_selected,
        app.services.len(),
    );
    f.render_stateful_widget(table, area, &mut app.services_table);
}

/// Per-process disk rate cell. A true 0 B/s and a permission-denied
/// `/proc/<pid>/io` read are indistinguishable, so render both as "—"
/// rather than a wall of zeros.
fn rate_str(v: u64) -> String {
    opt_bytes((v > 0).then_some(v))
}

/// Update a persistent table state's selection, clamped to the row count.
/// The state itself lives in `App` so the scroll offset survives redraws.
fn select_row(state: &mut TableState, selected: usize, len: usize) {
    state.select(if len == 0 {
        None
    } else {
        Some(selected.min(len - 1))
    });
}
