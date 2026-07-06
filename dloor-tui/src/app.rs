use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dloor_core::{
    check_dependencies, config::default_download_dir, detect_platform, Config, Destination,
    DownloadEvent, DownloadJob, DownloadProgress, DownloadRequest, Format, Platform, Quality,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Main,
    HowToUse,
    Format,
    Quality,
    Download,
    Complete,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupField {
    Destination,
    LocalPath,
    Remote,
    RemotePath,
}

#[derive(Debug, Clone)]
pub struct SetupState {
    pub cloud: bool,
    pub field: SetupField,
    pub local_path: String,
    pub remote: String,
    pub remote_path: String,
    pub rclone_available: bool,
}

#[derive(Debug)]
pub struct App {
    pub screen: Screen,
    pub config: Config,
    pub first_run: bool,
    pub setup: SetupState,
    pub url_input: String,
    pub selected_format: usize,
    pub selected_quality: usize,
    pub selected_platform: Option<Platform>,
    pub progress: Option<DownloadProgress>,
    pub status_text: String,
    pub completed_path: String,
    pub error_message: String,
    pub download_rx: Option<mpsc::Receiver<DownloadEvent>>,
    pub spinner_index: usize,
    pub should_quit: bool,
    startup_error: Option<String>,
}

impl App {
    pub fn new() -> Result<Self> {
        let first_run = !Config::exists();
        let config = Config::load_or_default()?;
        let report = check_dependencies(Some(&config));
        let startup_error = (!report.is_ready()).then(|| report.message());
        let rclone_available = !report
            .missing_optional
            .iter()
            .chain(report.missing_required.iter())
            .any(|tool| tool.command() == "rclone");

        let setup = SetupState::from_config(&config, rclone_available);
        Ok(Self {
            screen: if first_run {
                Screen::Setup
            } else {
                Screen::Main
            },
            config,
            first_run,
            setup,
            url_input: String::new(),
            selected_format: 0,
            selected_quality: 0,
            selected_platform: None,
            progress: None,
            status_text: "Ready".to_string(),
            completed_path: String::new(),
            error_message: String::new(),
            download_rx: None,
            spinner_index: 0,
            should_quit: false,
            startup_error,
        })
    }

    pub fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    pub fn tick(&mut self) {
        self.spinner_index = self.spinner_index.wrapping_add(1);
        let mut events = Vec::new();
        if let Some(rx) = &mut self.download_rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.apply_download_event(event);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return AppAction::Quit;
        }

        match self.screen {
            Screen::Setup => self.handle_setup_key(key),
            Screen::Main => self.handle_main_key(key),
            Screen::HowToUse => self.handle_how_to_use_key(key),
            Screen::Format => self.handle_select_key(key, Screen::Quality, 2),
            Screen::Quality => self.handle_quality_key(key),
            Screen::Download => AppAction::Continue,
            Screen::Complete => self.handle_complete_key(key),
            Screen::Error => self.handle_error_key(key),
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        match self.screen {
            Screen::Setup => {
                for ch in text.chars().filter(|ch| !ch.is_control()) {
                    self.push_setup_char(ch);
                }
            }
            Screen::Main => self.url_input.push_str(text.trim()),
            _ => {}
        }
    }

    fn handle_setup_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc if !self.first_run => self.screen = Screen::Main,
            KeyCode::Tab | KeyCode::Down => self.next_setup_field(),
            KeyCode::BackTab | KeyCode::Up => self.prev_setup_field(),
            KeyCode::Left | KeyCode::Right if self.setup.field == SetupField::Destination => {
                self.setup.cloud = !self.setup.cloud && self.setup.rclone_available;
            }
            KeyCode::Enter => {
                if let Err(error) = self.save_setup() {
                    self.show_error(error.to_string());
                }
            }
            KeyCode::Backspace => self.backspace_setup_field(),
            KeyCode::Char(ch) => self.push_setup_char(ch),
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_main_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => AppAction::Quit,
            KeyCode::Char('q') if self.url_input.is_empty() => AppAction::Quit,
            KeyCode::Enter => {
                let input = self.url_input.trim().to_string();
                if input == "/quit" {
                    return AppAction::Quit;
                }
                if input == "/settings" {
                    self.setup = SetupState::from_config(&self.config, self.setup.rclone_available);
                    self.screen = Screen::Setup;
                    self.url_input.clear();
                    return AppAction::Continue;
                }
                if input == "/howtouse" {
                    self.screen = Screen::HowToUse;
                    self.url_input.clear();
                    return AppAction::Continue;
                }
                match detect_platform(&input) {
                    Ok(platform) => {
                        self.selected_platform = Some(platform);
                        self.screen = Screen::Format;
                    }
                    Err(error) => self.show_error(error.to_string()),
                }
                AppAction::Continue
            }
            KeyCode::Backspace => {
                self.url_input.pop();
                AppAction::Continue
            }
            KeyCode::Char(ch) => {
                self.url_input.push(ch);
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    fn handle_how_to_use_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Char('q') => return AppAction::Quit,
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_select_key(&mut self, key: KeyEvent, next: Screen, count: usize) -> AppAction {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Up | KeyCode::Left => self.move_selection(count, false),
            KeyCode::Down | KeyCode::Right => self.move_selection(count, true),
            KeyCode::Enter => self.screen = next,
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_quality_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Format,
            KeyCode::Up | KeyCode::Left => self.move_selection(2, false),
            KeyCode::Down | KeyCode::Right => self.move_selection(2, true),
            KeyCode::Enter => self.start_download(),
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_complete_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Enter => {
                self.url_input.clear();
                self.progress = None;
                self.status_text = "Ready".to_string();
                self.screen = Screen::Main;
            }
            KeyCode::Char('q') | KeyCode::Esc => return AppAction::Quit,
            _ => {}
        }
        AppAction::Continue
    }

    fn handle_error_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Char('q') => return AppAction::Quit,
            _ => {}
        }
        AppAction::Continue
    }

    fn move_selection(&mut self, count: usize, forward: bool) {
        let selection = match self.screen {
            Screen::Format => &mut self.selected_format,
            Screen::Quality => &mut self.selected_quality,
            _ => return,
        };
        if forward {
            *selection = (*selection + 1) % count;
        } else {
            *selection = (*selection + count - 1) % count;
        }
    }

    fn start_download(&mut self) {
        let request = DownloadRequest {
            url: self.url_input.trim().to_string(),
            format: if self.selected_format == 0 {
                Format::Video
            } else {
                Format::Audio
            },
            quality: if self.selected_quality == 0 {
                Quality::Best
            } else {
                Quality::Compressed
            },
        };
        let job = DownloadJob::new(request, self.config.clone());
        self.download_rx = Some(job.spawn());
        self.progress = None;
        self.status_text = "Starting download...".to_string();
        self.screen = Screen::Download;
    }

    fn apply_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Progress { progress, platform } => {
                self.selected_platform = Some(platform);
                self.status_text = "Downloading".to_string();
                self.progress = Some(progress);
            }
            DownloadEvent::Converting => self.status_text = "Converting...".to_string(),
            DownloadEvent::Uploading => self.status_text = "Uploading...".to_string(),
            DownloadEvent::Completed { path } => {
                self.completed_path = path;
                self.download_rx = None;
                self.screen = Screen::Complete;
            }
            DownloadEvent::Failed { error } => {
                self.download_rx = None;
                self.show_error(error);
            }
        }
    }

    fn save_setup(&mut self) -> Result<()> {
        let destination = if self.setup.cloud {
            Destination::Cloud {
                remote: self.setup.remote.trim().to_string(),
                path: self.setup.remote_path.trim().trim_matches('/').to_string(),
            }
        } else {
            Destination::Local {
                path: self.setup.local_path.trim().into(),
            }
        };
        self.config.destination = destination;
        self.config.save()?;
        self.first_run = false;
        self.screen = Screen::Main;
        Ok(())
    }

    fn show_error(&mut self, message: String) {
        self.error_message = message;
        self.screen = Screen::Error;
    }

    pub fn destination_label(&self) -> String {
        match &self.config.destination {
            Destination::Local { path } => format!("local: {}", path.to_string_lossy()),
            Destination::Cloud { remote, .. } => format!("cloud: Google Drive ({remote})"),
        }
    }

    fn next_setup_field(&mut self) {
        self.setup.field = match (self.setup.field, self.setup.cloud) {
            (SetupField::Destination, false) => SetupField::LocalPath,
            (SetupField::Destination, true) => SetupField::Remote,
            (SetupField::LocalPath, _) => SetupField::Destination,
            (SetupField::Remote, _) => SetupField::RemotePath,
            (SetupField::RemotePath, _) => SetupField::Destination,
        };
    }

    fn prev_setup_field(&mut self) {
        self.setup.field = match (self.setup.field, self.setup.cloud) {
            (SetupField::Destination, false) => SetupField::LocalPath,
            (SetupField::Destination, true) => SetupField::RemotePath,
            (SetupField::LocalPath, _) => SetupField::Destination,
            (SetupField::Remote, _) => SetupField::Destination,
            (SetupField::RemotePath, _) => SetupField::Remote,
        };
    }

    fn push_setup_char(&mut self, ch: char) {
        match self.setup.field {
            SetupField::LocalPath => self.setup.local_path.push(ch),
            SetupField::Remote => self.setup.remote.push(ch),
            SetupField::RemotePath => self.setup.remote_path.push(ch),
            SetupField::Destination => {}
        }
    }

    fn backspace_setup_field(&mut self) {
        match self.setup.field {
            SetupField::LocalPath => {
                self.setup.local_path.pop();
            }
            SetupField::Remote => {
                self.setup.remote.pop();
            }
            SetupField::RemotePath => {
                self.setup.remote_path.pop();
            }
            SetupField::Destination => {}
        }
    }
}

impl SetupState {
    fn from_config(config: &Config, rclone_available: bool) -> Self {
        match &config.destination {
            Destination::Local { path } => Self {
                cloud: false,
                field: SetupField::Destination,
                local_path: path.to_string_lossy().to_string(),
                remote: "gdrive".to_string(),
                remote_path: "dloor".to_string(),
                rclone_available,
            },
            Destination::Cloud { remote, path } => Self {
                cloud: rclone_available,
                field: SetupField::Destination,
                local_path: default_download_dir().to_string_lossy().to_string(),
                remote: remote.clone(),
                remote_path: path.clone(),
                rclone_available,
            },
        }
    }
}
