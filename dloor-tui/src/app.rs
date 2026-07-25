use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dloor_core::{
    check_dependencies, config::default_download_dir, detect_platform, Browser, Config,
    Destination, DownloadEvent, DownloadItem, DownloadJob, DownloadProgress, DownloadQueue,
    DownloadRequest, DownloadSummary, Format, HistoryEntry, HistoryStatus, HistoryStore, JobId,
    Platform, PlaylistSelection, Quality, QueueStatus, DEFAULT_HISTORY_LIMIT,
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
    Queue(QueueState),
    History(HistoryState),
    Complete(CompleteState),
    ExitConfirm,
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

#[derive(Debug)]
pub struct DownloadViewState {
    pub job_id: JobId,
}

#[derive(Debug, Default)]
pub struct QueueState {
    pub selected: usize,
}

#[derive(Debug, Default)]
pub struct HistoryState {
    pub selected: usize,
}

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
    pub queue: DownloadQueue,
    pub history: HistoryStore,
    pub active_download: Option<ActiveDownload>,
    pub notification: Option<String>,
    startup_error: Option<String>,
}

#[derive(Debug)]
pub struct ActiveDownload {
    pub job_id: JobId,
    pub request: DownloadRequest,
    pub platform: Platform,
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
    Back,
    ReturnToMain { clear_input: bool },
    ShowError(String),
    Quit,
    ForceQuit,
}

#[derive(Debug)]
enum DownloadTerminal {
    Finished(JobId, DownloadSummary),
    Failed(JobId, String),
    Cancelled(JobId, DownloadSummary),
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
        let history = HistoryStore::open(Config::history_path()?, DEFAULT_HISTORY_LIMIT)?;

        Ok(Self {
            shared: SharedState {
                config,
                first_run,
                rclone_available,
                spinner_index: 0,
                queue: DownloadQueue::new(),
                history,
                active_download: None,
                notification: None,
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
        let monitored_job = match &self.navigation.current {
            Screen::Download(state) => Some(state.job_id),
            _ => None,
        };
        let terminal = self.shared.poll_queue();
        match terminal {
            Some(DownloadTerminal::Finished(job_id, summary)) => {
                if monitored_job == Some(job_id) {
                    self.navigation
                        .replace(Screen::Complete(CompleteState { summary }));
                } else {
                    self.shared.notification = Some(format!(
                        "Job {} finished: {} succeeded, {} failed",
                        job_id.0,
                        summary.succeeded.len(),
                        summary.failed.len()
                    ));
                }
            }
            Some(DownloadTerminal::Failed(job_id, error)) => {
                if monitored_job == Some(job_id) {
                    self.navigation.show_error(error);
                } else {
                    self.shared.notification = Some(format!("Job {} failed", job_id.0));
                }
            }
            Some(DownloadTerminal::Cancelled(job_id, summary)) => {
                if monitored_job == Some(job_id) {
                    if summary.succeeded.is_empty() && summary.failed.is_empty() {
                        self.navigation.return_to_main(false);
                    } else {
                        self.navigation
                            .replace(Screen::Complete(CompleteState { summary }));
                    }
                } else {
                    self.shared.notification = Some(format!("Job {} cancelled", job_id.0));
                }
            }
            None => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.shared.cancel_all();
            return AppAction::Quit;
        }

        let transition = match &mut self.navigation.current {
            Screen::Setup(state) => handle_setup_key(state, &mut self.shared, key),
            Screen::Main(state) => handle_main_key(state, &mut self.shared, key),
            Screen::HowToUse => handle_how_to_use_key(key),
            Screen::Playlist(state) => handle_playlist_key(state, key),
            Screen::Format(state) => handle_format_key(state, &self.shared, key),
            Screen::Quality(state) => handle_quality_key(state, &mut self.shared, key),
            Screen::Download(state) => handle_download_key(state, &mut self.shared, key),
            Screen::Queue(state) => handle_queue_key(state, &mut self.shared, key),
            Screen::History(state) => handle_history_key(state, &mut self.shared, key),
            Screen::Complete(_) => handle_complete_key(key),
            Screen::ExitConfirm => handle_exit_confirm_key(&mut self.shared, key),
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
            Transition::Back => {
                if !self.navigation.back() {
                    return AppAction::Quit;
                }
            }
            Transition::ReturnToMain { clear_input } => {
                self.navigation.return_to_main(clear_input);
            }
            Transition::ShowError(message) => self.navigation.show_error(message),
            Transition::Quit => {
                if self.shared.queue.has_unfinished() {
                    self.navigation.push(Screen::ExitConfirm);
                } else {
                    return AppAction::Quit;
                }
            }
            Transition::ForceQuit => return AppAction::Quit,
        }
        AppAction::Continue
    }
}

impl SharedState {
    fn enqueue(&mut self, request: DownloadRequest, title: String) -> JobId {
        let platform = detect_platform(&request.url).expect("validated URL is queued");
        let id = self.queue.enqueue(request, title, platform);
        self.notification = Some(format!("Job {} added to the queue", id.0));
        id
    }

    fn poll_queue(&mut self) -> Option<DownloadTerminal> {
        let mut terminal = None;
        if let Some(mut active) = self.active_download.take() {
            while let Ok(event) = active.receiver.try_recv() {
                self.queue.apply_event(active.job_id, &event);
                self.record_event(&active, &event);
                match event {
                    DownloadEvent::Resolving => {
                        active.status_text = "Resolving items...".to_string();
                    }
                    DownloadEvent::ItemStarted { item, platform } => {
                        active.platform = platform;
                        active.item = Some(item);
                        active.progress = None;
                        active.status_text = "Starting item...".to_string();
                    }
                    DownloadEvent::Progress {
                        progress,
                        item,
                        platform,
                    } => {
                        active.platform = platform;
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
                        terminal = Some(DownloadTerminal::Finished(active.job_id, summary));
                    }
                    DownloadEvent::Failed { error } => {
                        terminal = Some(DownloadTerminal::Failed(active.job_id, error));
                    }
                    DownloadEvent::Cancelled { summary } => {
                        terminal = Some(DownloadTerminal::Cancelled(active.job_id, summary));
                    }
                }
            }
            if terminal.is_none() {
                self.active_download = Some(active);
            } else {
                self.queue.remove_terminal(active.job_id);
            }
        }

        if self.active_download.is_none() {
            self.start_next();
        }
        terminal
    }

    fn start_next(&mut self) {
        let Some(queued) = self.queue.start_next() else {
            return;
        };
        let job = DownloadJob::new(queued.request.clone(), self.config.clone());
        let cancellation = job.cancellation_token();
        self.active_download = Some(ActiveDownload {
            job_id: queued.id,
            request: queued.request,
            platform: queued.platform,
            item: None,
            progress: None,
            status_text: "Starting download...".to_string(),
            receiver: job.spawn(),
            cancellation,
        });
    }

    fn record_event(&mut self, active: &ActiveDownload, event: &DownloadEvent) {
        let entry = match event {
            DownloadEvent::ItemCompleted { result } => Some(HistoryEntry::succeeded(
                &active.request,
                active.platform,
                result,
            )),
            DownloadEvent::ItemFailed { failure } => Some(HistoryEntry::failed(
                &active.request,
                active.platform,
                failure,
            )),
            DownloadEvent::Failed { .. } => Some(HistoryEntry::unfinished(
                &active.request,
                active.platform,
                active
                    .item
                    .as_ref()
                    .map_or_else(|| "Download job".to_string(), |item| item.title.clone()),
                HistoryStatus::Failed,
            )),
            DownloadEvent::Cancelled { summary } => match active.item.as_ref() {
                Some(item) => {
                    let already_recorded = summary
                        .succeeded
                        .iter()
                        .any(|result| result.item.playlist_index == item.playlist_index)
                        || summary
                            .failed
                            .iter()
                            .any(|failure| failure.item.playlist_index == item.playlist_index);
                    (!already_recorded).then(|| {
                        HistoryEntry::unfinished(
                            &active.request,
                            active.platform,
                            item.title.clone(),
                            HistoryStatus::Cancelled,
                        )
                    })
                }
                None => Some(HistoryEntry::unfinished(
                    &active.request,
                    active.platform,
                    self.queue
                        .entry(active.job_id)
                        .map_or_else(|| "Download job".to_string(), |job| job.title.clone()),
                    HistoryStatus::Cancelled,
                )),
            },
            DownloadEvent::Resolving
            | DownloadEvent::ItemStarted { .. }
            | DownloadEvent::Progress { .. }
            | DownloadEvent::Converting { .. }
            | DownloadEvent::Uploading { .. }
            | DownloadEvent::Finished { .. } => None,
        };
        if let Some(entry) = entry {
            if let Err(error) = self.history.append(entry) {
                self.notification = Some(format!("Could not update history: {error}"));
            }
        }
    }

    fn cancel_job(&mut self, id: JobId) {
        if self.active_download.as_ref().map(|active| active.job_id) == Some(id) {
            if let Some(active) = &mut self.active_download {
                active.cancellation.cancel();
                active.status_text = "Cancelling...".to_string();
            }
        } else if let Some(job) = self.queue.cancel_pending(id) {
            let entry = HistoryEntry::unfinished(
                &job.request,
                job.platform,
                job.title,
                HistoryStatus::Cancelled,
            );
            if let Err(error) = self.history.append(entry) {
                self.notification = Some(format!("Could not update history: {error}"));
            }
            self.queue.remove_terminal(id);
        }
    }

    fn cancel_all(&mut self) {
        if let Some(active) = &mut self.active_download {
            active.cancellation.cancel();
            let title = active
                .item
                .as_ref()
                .map_or_else(|| "Download job".to_string(), |item| item.title.clone());
            let entry = HistoryEntry::unfinished(
                &active.request,
                active.platform,
                title,
                HistoryStatus::Cancelled,
            );
            if let Err(error) = self.history.append(entry) {
                self.notification = Some(format!("Could not update history: {error}"));
            }
        }
        let pending: Vec<_> = self
            .queue
            .entries()
            .filter(|job| job.status == QueueStatus::Pending)
            .map(|job| job.id)
            .collect();
        for id in pending {
            self.cancel_job(id);
        }
    }

    fn retry_history(&mut self, index: usize) {
        let Some(entry) = self.history.entries().get(index).cloned() else {
            return;
        };
        let Some(request) = entry.retry_request() else {
            self.notification = Some("Successful history entries cannot be retried".to_string());
            return;
        };
        self.enqueue(request, format!("Retry: {}", entry.title));
    }

    pub fn active_for(&self, id: JobId) -> Option<&ActiveDownload> {
        self.active_download
            .as_ref()
            .filter(|active| active.job_id == id)
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
            if input == "/queue" {
                state.url_input.clear();
                return Transition::Push(Screen::Queue(QueueState::default()));
            }
            if input == "/history" {
                state.url_input.clear();
                return Transition::Push(Screen::History(HistoryState::default()));
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
            let request = DownloadRequest {
                url: state.url.clone(),
                format: state.format,
                quality: if state.selected == 0 {
                    Quality::Best
                } else {
                    Quality::Compressed
                },
                playlist: state.playlist,
            };
            shared.enqueue(request, state.url.clone());
            return Transition::ReturnToMain { clear_input: true };
        }
        _ => {}
    }
    Transition::Stay
}

fn handle_download_key(
    state: &DownloadViewState,
    shared: &mut SharedState,
    key: KeyEvent,
) -> Transition {
    if key.code == KeyCode::Esc {
        shared.cancel_job(state.job_id);
    }
    Transition::Stay
}

fn handle_queue_key(state: &mut QueueState, shared: &mut SharedState, key: KeyEvent) -> Transition {
    let ids: Vec<_> = shared
        .queue
        .entries()
        .filter(|job| matches!(job.status, QueueStatus::Pending | QueueStatus::Running))
        .map(|job| job.id)
        .collect();
    if ids.is_empty() {
        state.selected = 0;
    } else {
        state.selected = state.selected.min(ids.len() - 1);
    }
    let selected_id = ids.get(state.selected).copied();

    match key.code {
        KeyCode::Esc => Transition::Back,
        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(id) = selected_id {
                if shared.queue.move_pending(id, false) {
                    state.selected = state.selected.saturating_sub(1);
                }
            }
            Transition::Stay
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(id) = selected_id {
                if shared.queue.move_pending(id, true) {
                    state.selected = (state.selected + 1).min(ids.len().saturating_sub(1));
                }
            }
            Transition::Stay
        }
        KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            Transition::Stay
        }
        KeyCode::Down => {
            if !ids.is_empty() {
                state.selected = (state.selected + 1).min(ids.len() - 1);
            }
            Transition::Stay
        }
        KeyCode::Char('d') => {
            if let Some(id) = selected_id {
                shared.queue.remove_pending(id);
            }
            Transition::Stay
        }
        KeyCode::Char('c') => {
            if let Some(id) = selected_id {
                shared.cancel_job(id);
            }
            Transition::Stay
        }
        KeyCode::Enter => selected_id.map_or(Transition::Stay, |job_id| {
            Transition::Push(Screen::Download(DownloadViewState { job_id }))
        }),
        _ => Transition::Stay,
    }
}

fn handle_history_key(
    state: &mut HistoryState,
    shared: &mut SharedState,
    key: KeyEvent,
) -> Transition {
    let count = shared.history.entries().len();
    if count == 0 {
        state.selected = 0;
    } else {
        state.selected = state.selected.min(count - 1);
    }
    match key.code {
        KeyCode::Esc => Transition::Back,
        KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            Transition::Stay
        }
        KeyCode::Down => {
            if count > 0 {
                state.selected = (state.selected + 1).min(count - 1);
            }
            Transition::Stay
        }
        KeyCode::Char('r') => {
            if count > 0 {
                shared.retry_history(count - 1 - state.selected);
            }
            Transition::Stay
        }
        _ => Transition::Stay,
    }
}

fn handle_exit_confirm_key(shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            shared.cancel_all();
            Transition::ForceQuit
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Transition::Back,
        _ => Transition::Stay,
    }
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
        let history_dir = tempfile::tempdir().unwrap();
        let history = HistoryStore::open(history_dir.path().join("history.jsonl"), 10).unwrap();
        SharedState {
            config: Config::default(),
            first_run,
            rclone_available: false,
            spinner_index: 0,
            queue: DownloadQueue::new(),
            history,
            active_download: None,
            notification: None,
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
