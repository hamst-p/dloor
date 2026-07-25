mod app;
mod ui;

use std::{io, time::Duration};

use anyhow::Result;
use app::{App, AppAction};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = init_logging()?;

    let mut app = App::new()?;
    if let Some(message) = app.startup_error() {
        eprintln!("{message}");
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}

fn init_logging() -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let log_path = dloor_core::Config::log_path()?;
    let log_dir = log_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid log path: {}", log_path.display()))?;
    std::fs::create_dir_all(log_dir)?;
    let file_name = log_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid log path: {}", log_path.display()))?;
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::NEVER)
        .filename_prefix(file_name.to_string_lossy())
        .build(log_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = std::env::var(EnvFilter::DEFAULT_ENV)
        .map(EnvFilter::new)
        .unwrap_or_else(|_| EnvFilter::new("dloor=debug,dloor_core=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    tracing::debug!(path = %log_path.display(), "file logging initialized");
    Ok(guard)
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            return Ok(());
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if app.handle_key(key) == AppAction::Quit => return Ok(()),
                Event::Paste(text) => app.handle_paste(&text),
                _ => {}
            }
        }
    }
}
