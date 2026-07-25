mod app;
mod ui;

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use app::{App, AppAction};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_subscriber::EnvFilter;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

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
    let writer = SizeLimitedWriter::new(log_path, MAX_LOG_BYTES)?;
    let (writer, guard) = tracing_appender::non_blocking(writer);
    let filter = std::env::var(EnvFilter::DEFAULT_ENV)
        .map(EnvFilter::new)
        .unwrap_or_else(|_| EnvFilter::new("dloor=debug,dloor_core=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    tracing::debug!("file logging initialized");
    Ok(guard)
}

struct SizeLimitedWriter {
    path: PathBuf,
    backup_path: PathBuf,
    file: Option<File>,
    bytes_written: u64,
    max_bytes: u64,
}

impl SizeLimitedWriter {
    fn new(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        let backup_path = path.with_extension("log.1");
        let file = open_log_file(&path)?;
        let bytes_written = file.metadata()?.len();
        let mut writer = Self {
            path,
            backup_path,
            file: Some(file),
            bytes_written,
            max_bytes,
        };
        if writer.bytes_written >= writer.max_bytes {
            writer.rotate()?;
        }
        Ok(writer)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        if self.backup_path.exists() {
            std::fs::remove_file(&self.backup_path)?;
        }
        if self.path.exists() {
            std::fs::rename(&self.path, &self.backup_path)?;
        }
        self.file = Some(open_log_file(&self.path)?);
        self.bytes_written = 0;
        Ok(())
    }
}

impl Write for SizeLimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let incoming = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if self.bytes_written.saturating_add(incoming) > self.max_bytes {
            self.rotate()?;
        }
        let retained = if incoming > self.max_bytes {
            &buf[buf.len() - self.max_bytes as usize..]
        } else {
            buf
        };
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is not open"))?
            .write_all(retained)?;
        self.bytes_written += retained.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is not open"))?
            .flush()
    }
}

fn open_log_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if app.handle_key(key) == AppAction::Quit => return Ok(()),
                Event::Paste(text) => app.handle_paste(&text),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_writer_rotates_at_the_size_limit_and_keeps_one_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dloor.log");
        let mut writer = SizeLimitedWriter::new(path.clone(), 8).unwrap();

        writer.write_all(b"12345678").unwrap();
        writer.write_all(b"abcd").unwrap();
        writer.flush().unwrap();

        assert_eq!(std::fs::read(path).unwrap(), b"abcd");
        assert_eq!(
            std::fs::read(dir.path().join("dloor.log.1")).unwrap(),
            b"12345678"
        );
    }

    #[test]
    fn single_oversized_log_record_is_truncated_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dloor.log");
        let mut writer = SizeLimitedWriter::new(path.clone(), 4).unwrap();

        writer.write_all(b"private-tail").unwrap();
        writer.flush().unwrap();

        assert_eq!(std::fs::read(path).unwrap(), b"tail");
    }
}
