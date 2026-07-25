use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dloor_core::{
    check_dependencies, config::default_download_dir, detect_platform, Browser, Config,
    Destination, DownloadEvent, DownloadItem, DownloadJob, DownloadProgress, DownloadRequest,
    DownloadSummary, Format, Platform, PlaylistSelection, Quality,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum Screen {
    Setup(SetupState),
    Main(MainState),
    HowToUse,
    Playlist(PlaylistState),
    Format(FormatState),
    Quality(QualityState),
    Download(DownloadViewState),
    Complete(CompleteState),
    Error(ErrorState),
}

#[derive(Debug, Default)]
pub struct MainState {
    pub url_input: String,
}

#[derive(Debug)]
pub struct FormatState {
    pub url: String,
    pub playlist: PlaylistSelection,
    pub selected: usize,
}

#[derive(Debug)]
pub struct PlaylistState {
    pub url: String,
    pub selected: usize,
}

#[derive(Debug)]
pub struct QualityState {
    pub url: String,
    pub playlist: PlaylistSelection,
    pub format: Format,
    pub selected: usize,
}

#[derive(Debug, Default)]
pub struct DownloadViewState;

#[derive(Debug)]
pub struct CompleteState {
    pub summary: DownloadSummary,
}

#[derive(Debug)]
pub struct ErrorState {
    pub message: String,
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
    BrowserAuthentication,
    Browser,
}

#[derive(Debug)]
pub struct SetupState {
    pub cloud: bool,
    pub field: SetupField,
    pub local_path: String,
    pub remote: String,
    pub remote_path: String,
    pub use_browser_cookies: bool,
    pub browser_index: usize,
}

#[derive(Debug)]
pub struct SharedState {
    pub config: Config,
    pub first_run: bool,
    pub rclone_available: bool,
    pub spinner_index: usize,
    pub active_download: Option<ActiveDownload>,
    startup_error: Option<String>,
}

#[derive(Debug)]
pub struct ActiveDownload {
    pub platform: Option<Platform>,
    pub item: Option<DownloadItem>,
    pub progress: Option<DownloadProgress>,
    pub status_text: String,
    receiver: mpsc::Receiver<DownloadEvent>,
    cancellation: CancellationToken,
}

#[derive(Debug)]
pub struct Navigation {
    pub current: Screen,
    back_stack: Vec<Screen>,
}

#[derive(Debug)]
enum Transition {
    Stay,
    Push(Screen),
    Replace(Screen),
    Back,
    ReturnToMain { clear_input: bool },
    ShowError(String),
    Quit,
}

#[derive(Debug)]
enum DownloadTerminal {
    Finished(DownloadSummary),
    Failed(String),
    Cancelled(DownloadSummary),
}

#[derive(Debug)]
pub struct App {
    pub shared: SharedState,
    pub navigation: Navigation,
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
        let initial_screen = if first_run {
            Screen::Setup(SetupState::from_config(&config, rclone_available))
        } else {
            Screen::Main(MainState::default())
        };

        Ok(Self {
            shared: SharedState {
                config,
                first_run,
                rclone_available,
                spinner_index: 0,
                active_download: None,
                startup_error,
            },
            navigation: Navigation::new(initial_screen),
        })
    }

    pub fn startup_error(&self) -> Option<&str> {
        self.shared.startup_error.as_deref()
    }

    pub fn tick(&mut self) {
        self.shared.spinner_index = self.shared.spinner_index.wrapping_add(1);
        let terminal = self.shared.poll_download();
        match terminal {
            Some(DownloadTerminal::Finished(summary)) => {
                self.navigation
                    .replace(Screen::Complete(CompleteState { summary }));
            }
            Some(DownloadTerminal::Failed(error)) => self.navigation.show_error(error),
            Some(DownloadTerminal::Cancelled(summary)) => {
                if summary.succeeded.is_empty() && summary.failed.is_empty() {
                    self.navigation.return_to_main(false);
                } else {
                    self.navigation
                        .replace(Screen::Complete(CompleteState { summary }));
                }
            }
            None => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.shared.cancel_active_download();
            return AppAction::Quit;
        }

        let transition = match &mut self.navigation.current {
            Screen::Setup(state) => handle_setup_key(state, &mut self.shared, key),
            Screen::Main(state) => handle_main_key(state, &mut self.shared, key),
            Screen::HowToUse => handle_how_to_use_key(key),
            Screen::Playlist(state) => handle_playlist_key(state, key),
            Screen::Format(state) => handle_format_key(state, &self.shared, key),
            Screen::Quality(state) => handle_quality_key(state, &mut self.shared, key),
            Screen::Download(_) => handle_download_key(&mut self.shared, key),
            Screen::Complete(_) => handle_complete_key(key),
            Screen::Error(_) => handle_error_key(key),
        };
        self.apply_transition(transition)
    }

    pub fn handle_paste(&mut self, text: &str) {
        match &mut self.navigation.current {
            Screen::Setup(state) => {
                for ch in text.chars().filter(|ch| !ch.is_control()) {
                    state.push_char(ch);
                }
            }
            Screen::Main(state) => state.url_input.push_str(text.trim()),
            _ => {}
        }
    }

    fn apply_transition(&mut self, transition: Transition) -> AppAction {
        match transition {
            Transition::Stay => {}
            Transition::Push(screen) => self.navigation.push(screen),
            Transition::Replace(screen) => self.navigation.replace(screen),
            Transition::Back => {
                if !self.navigation.back() {
                    return AppAction::Quit;
                }
            }
            Transition::ReturnToMain { clear_input } => {
                self.navigation.return_to_main(clear_input);
            }
            Transition::ShowError(message) => self.navigation.show_error(message),
            Transition::Quit => return AppAction::Quit,
        }
        AppAction::Continue
    }
}

impl SharedState {
    fn start_download(&mut self, request: DownloadRequest) {
        let platform = detect_platform(&request.url).ok();
        let job = DownloadJob::new(request, self.config.clone());
        let cancellation = job.cancellation_token();
        self.active_download = Some(ActiveDownload {
            platform,
            item: None,
            progress: None,
            status_text: "Starting download...".to_string(),
            receiver: job.spawn(),
            cancellation,
        });
    }

    fn poll_download(&mut self) -> Option<DownloadTerminal> {
        let active = self.active_download.as_mut()?;
        let mut terminal = None;
        while let Ok(event) = active.receiver.try_recv() {
            match event {
                DownloadEvent::Resolving => {
                    active.status_text = "Resolving items...".to_string();
                }
                DownloadEvent::ItemStarted { item, platform } => {
                    active.platform = Some(platform);
                    active.item = Some(item);
                    active.progress = None;
                    active.status_text = "Starting item...".to_string();
                }
                DownloadEvent::Progress {
                    progress,
                    item,
                    platform,
                } => {
                    active.platform = Some(platform);
                    active.item = Some(item);
                    active.status_text = "Downloading".to_string();
                    active.progress = Some(progress);
                }
                DownloadEvent::Converting { item } => {
                    active.item = Some(item);
                    active.status_text = "Converting...".to_string();
                }
                DownloadEvent::Uploading { item } => {
                    active.item = Some(item);
                    active.status_text = "Uploading...".to_string();
                }
                DownloadEvent::ItemCompleted { result } => {
                    active.item = Some(result.item);
                    active.status_text = "Item completed".to_string();
                }
                DownloadEvent::ItemFailed { failure } => {
                    active.item = Some(failure.item);
                    active.status_text = "Item failed; continuing...".to_string();
                }
                DownloadEvent::Finished { summary } => {
                    terminal = Some(DownloadTerminal::Finished(summary));
                }
                DownloadEvent::Failed { error } => {
                    terminal = Some(DownloadTerminal::Failed(error));
                }
                DownloadEvent::Cancelled { summary } => {
                    terminal = Some(DownloadTerminal::Cancelled(summary));
                }
            }
        }
        if terminal.is_some() {
            self.active_download = None;
        }
        terminal
    }

    fn cancel_active_download(&mut self) {
        if let Some(active) = &mut self.active_download {
            active.cancellation.cancel();
            active.status_text = "Cancelling...".to_string();
        }
    }
}

impl Navigation {
    fn new(current: Screen) -> Self {
        Self {
            current,
            back_stack: Vec::new(),
        }
    }

    fn push(&mut self, screen: Screen) {
        let previous = std::mem::replace(&mut self.current, screen);
        self.back_stack.push(previous);
    }

    fn replace(&mut self, screen: Screen) {
        self.current = screen;
    }

    fn back(&mut self) -> bool {
        let Some(previous) = self.back_stack.pop() else {
            return false;
        };
        self.current = previous;
        true
    }

    fn return_to_main(&mut self, clear_input: bool) {
        let mut main = self
            .back_stack
            .drain(..)
            .find_map(|screen| match screen {
                Screen::Main(state) => Some(state),
                _ => None,
            })
            .or_else(
                || match std::mem::replace(&mut self.current, Screen::HowToUse) {
                    Screen::Main(state) => Some(state),
                    _ => None,
                },
            )
            .unwrap_or_default();
        if clear_input {
            main.url_input.clear();
        }
        self.current = Screen::Main(main);
        self.back_stack.clear();
    }

    fn show_error(&mut self, message: String) {
        self.return_to_main(false);
        self.push(Screen::Error(ErrorState { message }));
    }
}

fn handle_setup_key(state: &mut SetupState, shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc if !shared.first_run => return Transition::Back,
        KeyCode::Tab | KeyCode::Down => state.next_field(),
        KeyCode::BackTab | KeyCode::Up => state.prev_field(),
        KeyCode::Left | KeyCode::Right if state.field == SetupField::Destination => {
            state.cloud = !state.cloud && shared.rclone_available;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::BrowserAuthentication => {
            state.use_browser_cookies = !state.use_browser_cookies;
        }
        KeyCode::Left if state.field == SetupField::Browser => {
            state.browser_index =
                (state.browser_index + Browser::ALL.len() - 1) % Browser::ALL.len();
        }
        KeyCode::Right if state.field == SetupField::Browser => {
            state.browser_index = (state.browser_index + 1) % Browser::ALL.len();
        }
        KeyCode::Enter => {
            if let Err(error) = save_setup(state, shared) {
                return Transition::ShowError(error.to_string());
            }
            return Transition::ReturnToMain { clear_input: false };
        }
        KeyCode::Backspace => state.backspace_field(),
        KeyCode::Char(ch) => state.push_char(ch),
        _ => {}
    }
    Transition::Stay
}

fn handle_main_key(state: &mut MainState, shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc => Transition::Quit,
        KeyCode::Char('q') if state.url_input.is_empty() => Transition::Quit,
        KeyCode::Enter => {
            let input = state.url_input.trim().to_string();
            if input == "/quit" {
                return Transition::Quit;
            }
            if input == "/settings" {
                state.url_input.clear();
                return Transition::Push(Screen::Setup(SetupState::from_config(
                    &shared.config,
                    shared.rclone_available,
                )));
            }
            if input == "/howtouse" {
                state.url_input.clear();
                return Transition::Push(Screen::HowToUse);
            }
            match detect_platform(&input) {
                Ok(_) => Transition::Push(Screen::Playlist(PlaylistState {
                    url: input,
                    selected: 0,
                })),
                Err(error) => Transition::ShowError(error.to_string()),
            }
        }
        KeyCode::Backspace => {
            state.url_input.pop();
            Transition::Stay
        }
        KeyCode::Char(ch) => {
            state.url_input.push(ch);
            Transition::Stay
        }
        _ => Transition::Stay,
    }
}

fn handle_how_to_use_key(key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => Transition::Back,
        KeyCode::Char('q') => Transition::Quit,
        _ => Transition::Stay,
    }
}

fn handle_playlist_key(state: &mut PlaylistState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc => return Transition::Back,
        KeyCode::Up | KeyCode::Left => move_selection(&mut state.selected, 2, false),
        KeyCode::Down | KeyCode::Right => move_selection(&mut state.selected, 2, true),
        KeyCode::Enter => {
            return Transition::Push(Screen::Format(FormatState {
                url: state.url.clone(),
                playlist: if state.selected == 0 {
                    PlaylistSelection::Single
                } else {
                    PlaylistSelection::All
                },
                selected: 0,
            }));
        }
        _ => {}
    }
    Transition::Stay
}

fn handle_format_key(state: &mut FormatState, shared: &SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc => return Transition::Back,
        KeyCode::Up | KeyCode::Left => move_selection(&mut state.selected, 2, false),
        KeyCode::Down | KeyCode::Right => move_selection(&mut state.selected, 2, true),
        KeyCode::Enter => {
            return Transition::Push(Screen::Quality(QualityState {
                url: state.url.clone(),
                playlist: state.playlist,
                format: if state.selected == 0 {
                    Format::Video
                } else {
                    Format::Audio
                },
                selected: quality_index(shared.config.default_quality),
            }));
        }
        _ => {}
    }
    Transition::Stay
}

fn handle_quality_key(
    state: &mut QualityState,
    shared: &mut SharedState,
    key: KeyEvent,
) -> Transition {
    match key.code {
        KeyCode::Esc => return Transition::Back,
        KeyCode::Up | KeyCode::Left => move_selection(&mut state.selected, 2, false),
        KeyCode::Down | KeyCode::Right => move_selection(&mut state.selected, 2, true),
        KeyCode::Enter => {
            shared.start_download(DownloadRequest {
                url: state.url.clone(),
                format: state.format,
                quality: if state.selected == 0 {
                    Quality::Best
                } else {
                    Quality::Compressed
                },
                playlist: state.playlist,
            });
            return Transition::Replace(Screen::Download(DownloadViewState));
        }
        _ => {}
    }
    Transition::Stay
}

fn handle_download_key(shared: &mut SharedState, key: KeyEvent) -> Transition {
    if key.code == KeyCode::Esc {
        shared.cancel_active_download();
    }
    Transition::Stay
}

fn handle_complete_key(key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter => Transition::ReturnToMain { clear_input: true },
        KeyCode::Char('q') | KeyCode::Esc => Transition::Quit,
        _ => Transition::Stay,
    }
}

fn handle_error_key(key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => Transition::Back,
        KeyCode::Char('q') => Transition::Quit,
        _ => Transition::Stay,
    }
}

fn move_selection(selection: &mut usize, count: usize, forward: bool) {
    if forward {
        *selection = (*selection + 1) % count;
    } else {
        *selection = (*selection + count - 1) % count;
    }
}

fn quality_index(quality: Quality) -> usize {
    match quality {
        Quality::Best => 0,
        Quality::Compressed => 1,
    }
}

fn save_setup(state: &SetupState, shared: &mut SharedState) -> Result<()> {
    shared.config.destination = if state.cloud {
        Destination::Cloud {
            remote: state.remote.trim().to_string(),
            path: state.remote_path.trim().trim_matches('/').to_string(),
        }
    } else {
        Destination::Local {
            path: state.local_path.trim().into(),
        }
    };
    shared.config.browser = state
        .use_browser_cookies
        .then(|| Browser::ALL[state.browser_index]);
    shared.config.save()?;
    shared.first_run = false;
    Ok(())
}

impl SetupState {
    fn from_config(config: &Config, rclone_available: bool) -> Self {
        let browser_index = config
            .browser
            .and_then(|selected| Browser::ALL.iter().position(|browser| *browser == selected))
            .unwrap_or(0);
        match &config.destination {
            Destination::Local { path } => Self {
                cloud: false,
                field: SetupField::Destination,
                local_path: path.to_string_lossy().to_string(),
                remote: "gdrive".to_string(),
                remote_path: "dloor".to_string(),
                use_browser_cookies: config.browser.is_some(),
                browser_index,
            },
            Destination::Cloud { remote, path } => Self {
                cloud: rclone_available,
                field: SetupField::Destination,
                local_path: default_download_dir().to_string_lossy().to_string(),
                remote: remote.clone(),
                remote_path: path.clone(),
                use_browser_cookies: config.browser.is_some(),
                browser_index,
            },
        }
    }

    fn next_field(&mut self) {
        self.field = match (self.field, self.cloud, self.use_browser_cookies) {
            (SetupField::Destination, false, _) => SetupField::LocalPath,
            (SetupField::Destination, true, _) => SetupField::Remote,
            (SetupField::LocalPath, _, _) => SetupField::BrowserAuthentication,
            (SetupField::Remote, _, _) => SetupField::RemotePath,
            (SetupField::RemotePath, _, _) => SetupField::BrowserAuthentication,
            (SetupField::BrowserAuthentication, _, true) => SetupField::Browser,
            (SetupField::BrowserAuthentication, _, false) => SetupField::Destination,
            (SetupField::Browser, _, _) => SetupField::Destination,
        };
    }

    fn prev_field(&mut self) {
        self.field = match (self.field, self.cloud, self.use_browser_cookies) {
            (SetupField::Destination, _, true) => SetupField::Browser,
            (SetupField::Destination, _, false) => SetupField::BrowserAuthentication,
            (SetupField::LocalPath, _, _) => SetupField::Destination,
            (SetupField::Remote, _, _) => SetupField::Destination,
            (SetupField::RemotePath, _, _) => SetupField::Remote,
            (SetupField::BrowserAuthentication, false, _) => SetupField::LocalPath,
            (SetupField::BrowserAuthentication, true, _) => SetupField::RemotePath,
            (SetupField::Browser, _, _) => SetupField::BrowserAuthentication,
        };
    }

    fn push_char(&mut self, ch: char) {
        match self.field {
            SetupField::LocalPath => self.local_path.push(ch),
            SetupField::Remote => self.remote.push(ch),
            SetupField::RemotePath => self.remote_path.push(ch),
            SetupField::Destination | SetupField::BrowserAuthentication | SetupField::Browser => {}
        }
    }

    fn backspace_field(&mut self) {
        match self.field {
            SetupField::LocalPath => {
                self.local_path.pop();
            }
            SetupField::Remote => {
                self.remote.pop();
            }
            SetupField::RemotePath => {
                self.remote_path.pop();
            }
            SetupField::Destination | SetupField::BrowserAuthentication | SetupField::Browser => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_screen() -> Screen {
        Screen::Format(FormatState {
            url: "https://youtube.com/watch?v=example".to_string(),
            playlist: PlaylistSelection::Single,
            selected: 0,
        })
    }

    fn quality_screen() -> Screen {
        Screen::Quality(QualityState {
            url: "https://youtube.com/watch?v=example".to_string(),
            playlist: PlaylistSelection::Single,
            format: Format::Video,
            selected: 0,
        })
    }

    fn shared_state(first_run: bool) -> SharedState {
        SharedState {
            config: Config::default(),
            first_run,
            rclone_available: false,
            spinner_index: 0,
            active_download: None,
            startup_error: None,
        }
    }

    #[test]
    fn navigation_stack_restores_format_then_main() {
        let mut navigation = Navigation::new(Screen::Main(MainState::default()));
        navigation.push(format_screen());
        navigation.push(quality_screen());

        assert!(navigation.back());
        assert!(matches!(navigation.current, Screen::Format(_)));
        assert!(navigation.back());
        assert!(matches!(navigation.current, Screen::Main(_)));
    }

    #[test]
    fn empty_navigation_stack_cannot_go_back() {
        let mut navigation = Navigation::new(Screen::Main(MainState::default()));

        assert!(!navigation.back());
        assert!(matches!(navigation.current, Screen::Main(_)));
    }

    #[test]
    fn return_to_main_preserves_or_clears_the_root_input() {
        let mut navigation = Navigation::new(Screen::Main(MainState {
            url_input: "https://example.com".to_string(),
        }));
        navigation.push(format_screen());
        navigation.return_to_main(false);
        assert!(matches!(
            &navigation.current,
            Screen::Main(MainState { url_input }) if url_input == "https://example.com"
        ));

        navigation.push(format_screen());
        navigation.return_to_main(true);
        assert!(matches!(
            &navigation.current,
            Screen::Main(MainState { url_input }) if url_input.is_empty()
        ));
    }

    #[test]
    fn errors_always_return_to_main() {
        let mut navigation = Navigation::new(Screen::Main(MainState::default()));
        navigation.push(format_screen());
        navigation.show_error("failed".to_string());

        assert!(matches!(navigation.current, Screen::Error(_)));
        assert!(navigation.back());
        assert!(matches!(navigation.current, Screen::Main(_)));
    }

    #[test]
    fn escape_stays_on_first_run_setup_but_goes_back_from_settings() {
        let mut first_run = shared_state(true);
        let mut first_run_setup = SetupState::from_config(&first_run.config, false);
        let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(
            handle_setup_key(&mut first_run_setup, &mut first_run, escape),
            Transition::Stay
        ));

        let mut configured = shared_state(false);
        let mut settings = SetupState::from_config(&configured.config, false);
        assert!(matches!(
            handle_setup_key(&mut settings, &mut configured, escape),
            Transition::Back
        ));
    }

    #[test]
    fn configured_quality_maps_to_the_initial_quality_selection() {
        assert_eq!(quality_index(Quality::Best), 0);
        assert_eq!(quality_index(Quality::Compressed), 1);
    }
}
