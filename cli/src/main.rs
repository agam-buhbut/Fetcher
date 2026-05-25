use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod app;
mod ui;

use app::{App, Tab};

const TICK: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    // Always restore the terminal state, even on error.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> Result<()> {
    let mut app = App::new();
    app.tick(); // initial sample so the first frame has data

    let mut last_tick = Instant::now();

    loop {
        // Pick up any dirty Startup/Services flag the previous key set, so
        // the next draw sees fresh data instead of a one-tick-stale list.
        app.refresh_lazy_lists();
        terminal.draw(|f| ui::draw(f, &mut app))?;

        let timeout = TICK
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && handle_key(&mut app, key) {
                    return Ok(());
                }
            }
        }

        if last_tick.elapsed() >= TICK {
            app.tick();
            last_tick = Instant::now();
        }
    }
}

/// Returns `true` if the app should exit.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if app.filter_active {
        handle_filter_key(app, key.code);
        return false;
    }
    handle_normal_key(app, key.code, key.modifiers)
}

fn handle_filter_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.filter_active = false;
            app.filter.clear();
        }
        KeyCode::Enter => app.filter_active = false,
        KeyCode::Backspace => {
            app.filter.pop();
        }
        KeyCode::Char(c) => app.filter.push(c),
        _ => {}
    }
}

/// Returns `true` if the app should exit.
fn handle_normal_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('1') => app.tab = Tab::Processes,
        KeyCode::Char('2') => app.tab = Tab::Performance,
        KeyCode::Char('3') => app.tab = Tab::Startup,
        KeyCode::Char('4') => app.tab = Tab::Services,
        KeyCode::Tab => app.tab = app.tab.next(),
        KeyCode::BackTab => app.tab = app.tab.prev(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
        KeyCode::Home | KeyCode::Char('g') => app.jump_to(0),
        KeyCode::End | KeyCode::Char('G') => app.jump_to(i32::MAX),
        _ => match app.tab {
            Tab::Processes => app.handle_processes_key(code),
            Tab::Performance => {}
            Tab::Startup => app.handle_startup_key(code),
            Tab::Services => app.handle_services_key(code),
        },
    }
    false
}
