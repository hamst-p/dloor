use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dloor_core::{
    check_dependency_presence, check_media_capabilities, config::default_download_dir,
    detect_platform, diagnose_ytdlp_error, Browser, Config, CookieSource, DependencyJob,
    DependencyReport, Destination, DownloadEvent, DownloadItem, DownloadJob, DownloadProgress,
    DownloadQueue, DownloadRequest, DownloadSummary, ErrorDiagnosis, Format, HistoryEntry,
    HistoryStatus, HistoryStore, JobId, MetadataJob, MetadataPreview, MetadataRequest, Platform,
    PlaylistSelection, Quality, QueueStatus, Tool, YtDlpUpdateJob, YtDlpUpdateOutcome,
    DEFAULT_HISTORY_LIMIT,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::clipboard::ClipboardService;

#[derive(Debug)]
pub enum Screen {
    Setup(SetupState),
    Main(MainState),
    HowToUse,
    PreviewLoading(PreviewLoadingState),
    Preview(PreviewState),
    GenericConfirm(GenericConfirmState),
    Playlist(PlaylistState),
    Format(FormatState),
    Quality(QualityState),
    Download(DownloadViewState),
    Queue(QueueState),
    History(HistoryState),
    Complete(CompleteState),
    UpdateConfirm,
    UpdateRunning,
    UpdateResult(UpdateResultState),
    ExitConfirm,
    Error(ErrorState),
}

#[derive(Debug, Default)]
pub struct MainState {
    pub url_input: String,
}

#[derive(Debug, Default)]
pub struct PreviewLoadingState {
    pub cancelling: bool,
}

#[derive(Debug)]
pub struct PreviewState {
    pub url: String,
    pub preview: MetadataPreview,
}

#[derive(Debug)]
pub struct GenericConfirmState {
    pub url: String,
}

#[derive(Debug)]
pub struct FormatState {
    pub url: String,
    pub title: String,
    pub playlist: PlaylistSelection,
    pub selected: usize,
    pub available_resolutions: Vec<Quality>,
}

#[derive(Debug)]
pub struct PlaylistState {
    pub url: String,
    pub title: String,
    pub selected: usize,
}

#[derive(Debug)]
pub struct QualityState {
    pub url: String,
    pub title: String,
    pub playlist: PlaylistSelection,
    pub format: Format,
    pub options: Vec<Quality>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub note: Option<String>,
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
pub struct UpdateResultState {
    pub outcome: YtDlpUpdateOutcome,
}

#[derive(Debug)]
pub struct ErrorState {
    pub raw: String,
    pub diagnosis: Option<ErrorDiagnosis>,
    pub scroll_offset: u16,
    pub copy_status: Option<String>,
}

impl ErrorState {
    fn new(raw: String, report: &DependencyReport) -> Self {
        let version = report
            .version(Tool::YtDlp)
            .map(|version| version.raw.as_str());
        let diagnosis = diagnose_ytdlp_error(&raw, version, report.yt_dlp_freshness());
        Self {
            raw,
            diagnosis,
            scroll_offset: 0,
            copy_status: None,
        }
    }

    pub fn diagnostic_text(&self) -> String {
        let mut sections = Vec::new();
        if let Some(diagnosis) = &self.diagnosis {
            sections.push(format!(
                "{}\n\nSuggested action:\n{}",
                diagnosis.summary, diagnosis.advice
            ));
        }
        sections.push(format!("Raw error:\n{}", self.raw));
        sections.join("\n\n")
    }

    fn max_scroll(&self) -> u16 {
        let estimated_lines: usize = self
            .diagnostic_text()
            .lines()
            .map(|line| line.chars().count().div_ceil(80).max(1))
            .sum();
        u16::try_from(estimated_lines.saturating_sub(6)).unwrap_or(u16::MAX)
    }

    fn scroll_by(&mut self, amount: i16) {
        if amount.is_negative() {
            self.scroll_offset = self.scroll_offset.saturating_sub(amount.unsigned_abs());
        } else {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add(amount as u16)
                .min(self.max_scroll());
        }
    }
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
    CookieSource,
    Browser,
    CookieFile,
    GenericConfirmation,
    ClipboardAutofill,
    BandwidthLimit,
    WriteSubtitles,
    EmbedSubtitles,
    SubtitleLanguages,
    AutoSubtitles,
    EmbedThumbnail,
    EmbedChapters,
}

#[derive(Debug)]
pub struct SetupState {
    pub cloud: bool,
    pub field: SetupField,
    pub local_path: String,
    pub remote: String,
    pub remote_path: String,
    pub cookie_source_index: usize,
    pub browser_index: usize,
    pub cookie_file_path: String,
    pub confirm_generic_urls: bool,
    pub clipboard_autofill: bool,
    pub bandwidth_limit: String,
    pub write_subtitles: bool,
    pub embed_subtitles: bool,
    pub subtitle_languages: String,
    pub include_auto_subtitles: bool,
    pub embed_thumbnail: bool,
    pub embed_chapters: bool,
    pub scroll_offset: usize,
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
    pub active_preview: Option<ActivePreview>,
    pub dependency_report: DependencyReport,
    dependency_receiver: Option<mpsc::Receiver<DownloadEvent>>,
    update_receiver: Option<mpsc::Receiver<DownloadEvent>>,
    clipboard: ClipboardService,
    clipboard_receiver: Option<mpsc::Receiver<String>>,
    clipboard_checked_for_main: bool,
    last_clipboard_candidate: Option<String>,
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
pub struct ActivePreview {
    url: String,
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
enum PreviewTerminal {
    Ready(String, MetadataPreview),
    Failed(String),
    Cancelled,
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
        let presence = check_dependency_presence(Some(&config));
        let startup_error = (!presence.is_ready()).then(|| presence.message());
        let rclone_available = !presence
            .missing_optional
            .iter()
            .chain(presence.missing_required.iter())
            .any(|tool| tool.command() == "rclone");
        let initial_screen = if first_run {
            Screen::Setup(SetupState::from_config(&config, rclone_available))
        } else {
            Screen::Main(MainState::default())
        };
        let history = HistoryStore::open(Config::history_path()?, DEFAULT_HISTORY_LIMIT)?;
        let media_warnings = check_media_capabilities(&config.media);
        let dependency_receiver = Some(DependencyJob::new(config.clone()).spawn());

        Ok(Self {
            shared: SharedState {
                config,
                first_run,
                rclone_available,
                spinner_index: 0,
                queue: DownloadQueue::new(),
                history,
                active_download: None,
                active_preview: None,
                dependency_report: presence,
                dependency_receiver,
                update_receiver: None,
                clipboard: ClipboardService::new(),
                clipboard_receiver: None,
                clipboard_checked_for_main: false,
                last_clipboard_candidate: None,
                notification: (!media_warnings.is_empty()).then(|| media_warnings.join(" ")),
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
        self.refresh_clipboard_autofill();
        if let Some(report) = self.shared.poll_dependency_check() {
            if let Some(warning) = report.warnings.first() {
                self.shared.notification = Some(warning.clone());
            }
            self.shared.rclone_available = !report
                .missing_optional
                .iter()
                .chain(report.missing_required.iter())
                .any(|tool| tool.command() == "rclone");
            self.shared.dependency_report = report;
        }
        if let Some(outcome) = self.shared.poll_update() {
            if matches!(self.navigation.current, Screen::UpdateRunning) {
                self.navigation
                    .replace(Screen::UpdateResult(UpdateResultState { outcome }));
            }
            self.shared.refresh_dependencies();
        }
        let preview_terminal = self.shared.poll_preview();
        match preview_terminal {
            Some(PreviewTerminal::Ready(url, preview)) => {
                if matches!(self.navigation.current, Screen::PreviewLoading(_)) {
                    self.navigation
                        .replace(Screen::Preview(PreviewState { url, preview }));
                }
            }
            Some(PreviewTerminal::Failed(error)) => {
                let state = ErrorState::new(error, &self.shared.dependency_report);
                self.navigation.show_error(state);
            }
            Some(PreviewTerminal::Cancelled) => {
                if matches!(self.navigation.current, Screen::PreviewLoading(_)) {
                    self.navigation.back();
                }
            }
            None => {}
        }
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
                    let state = ErrorState::new(error, &self.shared.dependency_report);
                    self.navigation.show_error(state);
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

    fn refresh_clipboard_autofill(&mut self) {
        let Screen::Main(state) = &mut self.navigation.current else {
            self.shared.leave_main_clipboard_context();
            return;
        };

        if !self.shared.clipboard_checked_for_main {
            self.shared.clipboard_checked_for_main = true;
            if self.shared.config.clipboard_autofill && state.url_input.is_empty() {
                self.shared.start_clipboard_read();
            }
        }

        let Some(raw) = self.shared.poll_clipboard_read() else {
            return;
        };
        let Some(candidate) = clipboard_url_candidate(
            &raw,
            &state.url_input,
            self.shared.last_clipboard_candidate.as_deref(),
        ) else {
            return;
        };
        state.url_input.clone_from(&candidate);
        self.shared.last_clipboard_candidate = Some(candidate);
        self.shared.notification =
            Some("URL prefilled from the clipboard; review it before pressing Enter".to_string());
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
            Screen::PreviewLoading(state) => {
                handle_preview_loading_key(state, &mut self.shared, key)
            }
            Screen::Preview(state) => handle_preview_key(state, key),
            Screen::GenericConfirm(state) => {
                handle_generic_confirm_key(state, &mut self.shared, key)
            }
            Screen::Playlist(state) => handle_playlist_key(state, key),
            Screen::Format(state) => handle_format_key(state, &self.shared, key),
            Screen::Quality(state) => handle_quality_key(state, &mut self.shared, key),
            Screen::Download(state) => handle_download_key(state, &mut self.shared, key),
            Screen::Queue(state) => handle_queue_key(state, &mut self.shared, key),
            Screen::History(state) => handle_history_key(state, &mut self.shared, key),
            Screen::Complete(_) => handle_complete_key(key),
            Screen::UpdateConfirm => handle_update_confirm_key(&mut self.shared, key),
            Screen::UpdateRunning => Transition::Stay,
            Screen::UpdateResult(_) => handle_update_result_key(key),
            Screen::ExitConfirm => handle_exit_confirm_key(&mut self.shared, key),
            Screen::Error(state) => handle_error_key(state, &mut self.shared, key),
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
            Transition::ShowError(message) => {
                let state = ErrorState::new(message, &self.shared.dependency_report);
                self.navigation.show_error(state);
            }
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
    fn start_clipboard_read(&mut self) {
        if self.clipboard_receiver.is_some() {
            return;
        }
        let clipboard = self.clipboard.clone();
        let (sender, receiver) = mpsc::channel(1);
        self.clipboard_receiver = Some(receiver);
        tokio::task::spawn_blocking(move || {
            if let Ok(text) = clipboard.read_text() {
                let _ = sender.blocking_send(text);
            }
        });
    }

    fn poll_clipboard_read(&mut self) -> Option<String> {
        let receiver = self.clipboard_receiver.as_mut()?;
        match receiver.try_recv() {
            Ok(text) => {
                self.clipboard_receiver = None;
                Some(text)
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.clipboard_receiver = None;
                None
            }
            Err(mpsc::error::TryRecvError::Empty) => None,
        }
    }

    fn leave_main_clipboard_context(&mut self) {
        self.clipboard_checked_for_main = false;
        self.clipboard_receiver = None;
    }

    fn poll_dependency_check(&mut self) -> Option<DependencyReport> {
        let receiver = self.dependency_receiver.as_mut()?;
        let event = receiver.try_recv().ok()?;
        match event {
            DownloadEvent::DependenciesChecked { report } => {
                self.dependency_receiver = None;
                Some(report)
            }
            _ => None,
        }
    }

    fn refresh_dependencies(&mut self) {
        self.dependency_receiver = Some(DependencyJob::new(self.config.clone()).spawn());
    }

    fn start_update(&mut self) {
        if self.update_receiver.is_none() {
            self.update_receiver = Some(YtDlpUpdateJob.spawn());
        }
    }

    fn poll_update(&mut self) -> Option<YtDlpUpdateOutcome> {
        let receiver = self.update_receiver.as_mut()?;
        let event = receiver.try_recv().ok()?;
        match event {
            DownloadEvent::YtDlpUpdateFinished { outcome } => {
                self.update_receiver = None;
                Some(outcome)
            }
            _ => None,
        }
    }

    pub fn dependency_warning(&self) -> Option<&str> {
        self.dependency_report.warnings.first().map(String::as_str)
    }

    fn start_preview(&mut self, url: String) {
        let job = MetadataJob::new(MetadataRequest {
            url: url.clone(),
            cookies: self.config.cookies.clone(),
        });
        let cancellation = job.cancellation_token();
        self.active_preview = Some(ActivePreview {
            url,
            receiver: job.spawn(),
            cancellation,
        });
    }

    fn poll_preview(&mut self) -> Option<PreviewTerminal> {
        let active = self.active_preview.as_mut()?;
        let event = active.receiver.try_recv().ok()?;
        let terminal = match event {
            DownloadEvent::DependenciesChecked { .. }
            | DownloadEvent::YtDlpUpdateFinished { .. } => return None,
            DownloadEvent::PreviewReady { preview } => {
                PreviewTerminal::Ready(active.url.clone(), preview)
            }
            DownloadEvent::PreviewFailed { error } => PreviewTerminal::Failed(error),
            DownloadEvent::PreviewCancelled => PreviewTerminal::Cancelled,
            _ => return None,
        };
        self.active_preview = None;
        Some(terminal)
    }

    fn cancel_preview(&mut self) {
        if let Some(active) = &self.active_preview {
            active.cancellation.cancel();
        }
    }

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
                    DownloadEvent::DependenciesChecked { .. }
                    | DownloadEvent::YtDlpUpdateFinished { .. }
                    | DownloadEvent::PreviewReady { .. }
                    | DownloadEvent::PreviewFailed { .. }
                    | DownloadEvent::PreviewCancelled => {}
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
                    DownloadEvent::ItemWarning { warning } => {
                        active.item = Some(warning.item);
                        active.status_text = format!("Warning: {}", warning.message);
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
            DownloadEvent::DependenciesChecked { .. }
            | DownloadEvent::YtDlpUpdateFinished { .. }
            | DownloadEvent::PreviewReady { .. }
            | DownloadEvent::PreviewFailed { .. }
            | DownloadEvent::PreviewCancelled
            | DownloadEvent::Resolving
            | DownloadEvent::ItemStarted { .. }
            | DownloadEvent::Progress { .. }
            | DownloadEvent::Converting { .. }
            | DownloadEvent::Uploading { .. }
            | DownloadEvent::ItemWarning { .. }
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
        self.cancel_preview();
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

    fn show_error(&mut self, state: ErrorState) {
        self.return_to_main(false);
        self.push(Screen::Error(state));
    }
}

fn clipboard_url_candidate(
    raw: &str,
    current_input: &str,
    last_candidate: Option<&str>,
) -> Option<String> {
    let candidate = raw.trim();
    if !current_input.is_empty() || candidate.is_empty() || last_candidate == Some(candidate) {
        return None;
    }
    detect_platform(candidate).ok()?;
    Some(candidate.to_string())
}

fn handle_setup_key(state: &mut SetupState, shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc if !shared.first_run => return Transition::Back,
        KeyCode::Tab | KeyCode::Down => state.next_field(),
        KeyCode::BackTab | KeyCode::Up => state.prev_field(),
        KeyCode::Left | KeyCode::Right if state.field == SetupField::Destination => {
            state.cloud = !state.cloud && shared.rclone_available;
        }
        KeyCode::Left if state.field == SetupField::CookieSource => {
            state.cookie_source_index = (state.cookie_source_index + 2) % 3;
        }
        KeyCode::Right if state.field == SetupField::CookieSource => {
            state.cookie_source_index = (state.cookie_source_index + 1) % 3;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::GenericConfirmation => {
            state.confirm_generic_urls = !state.confirm_generic_urls;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::ClipboardAutofill => {
            state.clipboard_autofill = !state.clipboard_autofill;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::WriteSubtitles => {
            state.write_subtitles = !state.write_subtitles;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::EmbedSubtitles => {
            state.embed_subtitles = !state.embed_subtitles;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::AutoSubtitles => {
            state.include_auto_subtitles = !state.include_auto_subtitles;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::EmbedThumbnail => {
            state.embed_thumbnail = !state.embed_thumbnail;
        }
        KeyCode::Left | KeyCode::Right if state.field == SetupField::EmbedChapters => {
            state.embed_chapters = !state.embed_chapters;
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
            if input == "/update" {
                state.url_input.clear();
                return Transition::Push(Screen::UpdateConfirm);
            }
            match detect_platform(&input) {
                Ok(Platform::Generic) if shared.config.confirm_generic_urls => {
                    Transition::Push(Screen::GenericConfirm(GenericConfirmState { url: input }))
                }
                Ok(_) => {
                    shared.start_preview(input);
                    Transition::Push(Screen::PreviewLoading(PreviewLoadingState::default()))
                }
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

fn handle_update_confirm_key(shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
            shared.start_update();
            Transition::Push(Screen::UpdateRunning)
        }
        KeyCode::Esc | KeyCode::Char('n' | 'N') => Transition::Back,
        _ => Transition::Stay,
    }
}

fn handle_update_result_key(key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => Transition::ReturnToMain { clear_input: true },
        KeyCode::Char('q') => Transition::Quit,
        _ => Transition::Stay,
    }
}

fn handle_generic_confirm_key(
    state: &GenericConfirmState,
    shared: &mut SharedState,
    key: KeyEvent,
) -> Transition {
    match key.code {
        KeyCode::Esc => Transition::Back,
        KeyCode::Enter => {
            shared.start_preview(state.url.clone());
            Transition::Push(Screen::PreviewLoading(PreviewLoadingState::default()))
        }
        KeyCode::Char('a') => {
            shared.config.confirm_generic_urls = false;
            if let Err(error) = shared.config.save() {
                return Transition::ShowError(error.to_string());
            }
            shared.start_preview(state.url.clone());
            Transition::Push(Screen::PreviewLoading(PreviewLoadingState::default()))
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

fn handle_preview_loading_key(
    state: &mut PreviewLoadingState,
    shared: &mut SharedState,
    key: KeyEvent,
) -> Transition {
    if key.code == KeyCode::Esc {
        state.cancelling = true;
        shared.cancel_preview();
    }
    Transition::Stay
}

fn handle_preview_key(state: &PreviewState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc => Transition::Back,
        KeyCode::Enter if state.preview.playlist.is_some() => {
            Transition::Push(Screen::Playlist(PlaylistState {
                url: state.url.clone(),
                title: state.preview.title.clone(),
                selected: 0,
            }))
        }
        KeyCode::Enter => Transition::Push(Screen::Format(FormatState {
            url: state.url.clone(),
            title: state.preview.title.clone(),
            playlist: PlaylistSelection::Single,
            selected: 0,
            available_resolutions: metadata_qualities(&state.preview),
        })),
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
                title: state.title.clone(),
                playlist: if state.selected == 0 {
                    PlaylistSelection::Single
                } else {
                    PlaylistSelection::All
                },
                selected: 0,
                available_resolutions: Quality::RESOLUTIONS.to_vec(),
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
            let format = if state.selected == 0 {
                Format::Video
            } else {
                Format::Audio
            };
            let mut options = vec![Quality::Best, Quality::Compressed];
            if format == Format::Video {
                options.extend(state.available_resolutions.iter().copied());
            }
            let selected = options
                .iter()
                .position(|quality| *quality == shared.config.default_quality)
                .unwrap_or(0);
            let note = (!options.contains(&shared.config.default_quality)).then(|| {
                format!(
                    "Configured default {} is unavailable here; using Best.",
                    shared.config.default_quality.label()
                )
            });
            return Transition::Push(Screen::Quality(QualityState {
                url: state.url.clone(),
                title: state.title.clone(),
                playlist: state.playlist,
                format,
                options,
                selected,
                scroll_offset: selected.saturating_sub(5),
                note,
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
        KeyCode::Up | KeyCode::Left => {
            move_selection(&mut state.selected, state.options.len(), false);
            state.sync_scroll();
        }
        KeyCode::Down | KeyCode::Right => {
            move_selection(&mut state.selected, state.options.len(), true);
            state.sync_scroll();
        }
        KeyCode::Enter => {
            let request = DownloadRequest {
                url: state.url.clone(),
                format: state.format,
                quality: state.options[state.selected],
                playlist: state.playlist,
            };
            shared.enqueue(request, state.title.clone());
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

fn handle_error_key(state: &mut ErrorState, shared: &mut SharedState, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => Transition::Back,
        KeyCode::Char('q') => Transition::Quit,
        KeyCode::Up => {
            state.scroll_by(-1);
            Transition::Stay
        }
        KeyCode::Down => {
            state.scroll_by(1);
            Transition::Stay
        }
        KeyCode::PageUp => {
            state.scroll_by(-8);
            Transition::Stay
        }
        KeyCode::PageDown => {
            state.scroll_by(8);
            Transition::Stay
        }
        KeyCode::Home => {
            state.scroll_offset = 0;
            Transition::Stay
        }
        KeyCode::End => {
            state.scroll_offset = state.max_scroll();
            Transition::Stay
        }
        KeyCode::Char('c') => {
            state.copy_status = Some(match shared.clipboard.copy_text(state.diagnostic_text()) {
                Ok(()) => "Copied full diagnostics to the clipboard".to_string(),
                Err(error) => format!("Copy failed: {error}"),
            });
            Transition::Stay
        }
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

fn metadata_qualities(preview: &MetadataPreview) -> Vec<Quality> {
    Quality::RESOLUTIONS
        .into_iter()
        .filter(|quality| {
            preview
                .resolutions
                .iter()
                .any(|resolution| resolution == quality.label())
        })
        .collect()
}

impl QualityState {
    fn sync_scroll(&mut self) {
        const VISIBLE_OPTIONS: usize = 6;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + VISIBLE_OPTIONS {
            self.scroll_offset = self.selected + 1 - VISIBLE_OPTIONS;
        }
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
    shared.config.cookies = match state.cookie_source_index {
        0 => CookieSource::None,
        1 => CookieSource::Browser {
            browser: Browser::ALL[state.browser_index],
        },
        2 => {
            let path = state.cookie_file_path.trim();
            if path.is_empty() {
                anyhow::bail!("Cookie file path cannot be empty");
            }
            CookieSource::File { path: path.into() }
        }
        _ => unreachable!("cookie source selection is always normalized"),
    };
    shared.config.confirm_generic_urls = state.confirm_generic_urls;
    shared.config.clipboard_autofill = state.clipboard_autofill;
    shared.config.bandwidth_limit = if state.bandwidth_limit.trim().is_empty() {
        None
    } else {
        Some(state.bandwidth_limit.parse().map_err(anyhow::Error::msg)?)
    };
    shared.config.media.write_subtitles = state.write_subtitles;
    shared.config.media.embed_subtitles = state.embed_subtitles;
    shared.config.media.subtitle_languages = state
        .subtitle_languages
        .split(',')
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_string)
        .collect();
    shared.config.media.include_auto_subtitles = state.include_auto_subtitles;
    shared.config.media.embed_thumbnail = state.embed_thumbnail;
    shared.config.media.embed_chapters = state.embed_chapters;
    shared.config.save()?;
    let media_warnings = check_media_capabilities(&shared.config.media);
    if !media_warnings.is_empty() {
        shared.notification = Some(media_warnings.join(" "));
    }
    shared.first_run = false;
    Ok(())
}

impl SetupState {
    fn from_config(config: &Config, rclone_available: bool) -> Self {
        let browser_index = match &config.cookies {
            CookieSource::Browser { browser } => Browser::ALL
                .iter()
                .position(|candidate| candidate == browser),
            CookieSource::None | CookieSource::File { .. } => None,
        }
        .unwrap_or(0);
        let cookie_source_index = match &config.cookies {
            CookieSource::None => 0,
            CookieSource::Browser { .. } => 1,
            CookieSource::File { .. } => 2,
        };
        let cookie_file_path = match &config.cookies {
            CookieSource::File { path } => path.to_string_lossy().to_string(),
            CookieSource::None | CookieSource::Browser { .. } => String::new(),
        };
        match &config.destination {
            Destination::Local { path } => Self {
                cloud: false,
                field: SetupField::Destination,
                local_path: path.to_string_lossy().to_string(),
                remote: "gdrive".to_string(),
                remote_path: "dloor".to_string(),
                cookie_source_index,
                browser_index,
                cookie_file_path,
                confirm_generic_urls: config.confirm_generic_urls,
                clipboard_autofill: config.clipboard_autofill,
                bandwidth_limit: config
                    .bandwidth_limit
                    .as_ref()
                    .map_or_else(String::new, |limit| limit.as_str().to_string()),
                write_subtitles: config.media.write_subtitles,
                embed_subtitles: config.media.embed_subtitles,
                subtitle_languages: config.media.subtitle_languages.join(","),
                include_auto_subtitles: config.media.include_auto_subtitles,
                embed_thumbnail: config.media.embed_thumbnail,
                embed_chapters: config.media.embed_chapters,
                scroll_offset: 0,
            },
            Destination::Cloud { remote, path } => Self {
                cloud: rclone_available,
                field: SetupField::Destination,
                local_path: default_download_dir().to_string_lossy().to_string(),
                remote: remote.clone(),
                remote_path: path.clone(),
                cookie_source_index,
                browser_index,
                cookie_file_path,
                confirm_generic_urls: config.confirm_generic_urls,
                clipboard_autofill: config.clipboard_autofill,
                bandwidth_limit: config
                    .bandwidth_limit
                    .as_ref()
                    .map_or_else(String::new, |limit| limit.as_str().to_string()),
                write_subtitles: config.media.write_subtitles,
                embed_subtitles: config.media.embed_subtitles,
                subtitle_languages: config.media.subtitle_languages.join(","),
                include_auto_subtitles: config.media.include_auto_subtitles,
                embed_thumbnail: config.media.embed_thumbnail,
                embed_chapters: config.media.embed_chapters,
                scroll_offset: 0,
            },
        }
    }

    fn next_field(&mut self) {
        self.move_field(true);
    }

    fn prev_field(&mut self) {
        self.move_field(false);
    }

    fn move_field(&mut self, forward: bool) {
        let fields = self.fields();
        let current = fields
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % fields.len()
        } else {
            (current + fields.len() - 1) % fields.len()
        };
        self.field = fields[next];
        self.scroll_offset = next.saturating_sub(5);
    }

    fn fields(&self) -> Vec<SetupField> {
        let mut fields = vec![SetupField::Destination];
        if self.cloud {
            fields.extend([SetupField::Remote, SetupField::RemotePath]);
        } else {
            fields.push(SetupField::LocalPath);
        }
        fields.push(SetupField::CookieSource);
        match self.cookie_source_index {
            1 => fields.push(SetupField::Browser),
            2 => fields.push(SetupField::CookieFile),
            _ => {}
        }
        fields.extend([
            SetupField::GenericConfirmation,
            SetupField::ClipboardAutofill,
            SetupField::BandwidthLimit,
            SetupField::WriteSubtitles,
            SetupField::EmbedSubtitles,
            SetupField::SubtitleLanguages,
            SetupField::AutoSubtitles,
            SetupField::EmbedThumbnail,
            SetupField::EmbedChapters,
        ]);
        fields
    }

    fn push_char(&mut self, ch: char) {
        match self.field {
            SetupField::LocalPath => self.local_path.push(ch),
            SetupField::Remote => self.remote.push(ch),
            SetupField::RemotePath => self.remote_path.push(ch),
            SetupField::CookieFile => self.cookie_file_path.push(ch),
            SetupField::BandwidthLimit => self.bandwidth_limit.push(ch),
            SetupField::SubtitleLanguages => self.subtitle_languages.push(ch),
            SetupField::Destination
            | SetupField::CookieSource
            | SetupField::Browser
            | SetupField::GenericConfirmation
            | SetupField::ClipboardAutofill
            | SetupField::WriteSubtitles
            | SetupField::EmbedSubtitles
            | SetupField::AutoSubtitles
            | SetupField::EmbedThumbnail
            | SetupField::EmbedChapters => {}
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
            SetupField::CookieFile => {
                self.cookie_file_path.pop();
            }
            SetupField::BandwidthLimit => {
                self.bandwidth_limit.pop();
            }
            SetupField::SubtitleLanguages => {
                self.subtitle_languages.pop();
            }
            SetupField::Destination
            | SetupField::CookieSource
            | SetupField::Browser
            | SetupField::GenericConfirmation
            | SetupField::ClipboardAutofill
            | SetupField::WriteSubtitles
            | SetupField::EmbedSubtitles
            | SetupField::AutoSubtitles
            | SetupField::EmbedThumbnail
            | SetupField::EmbedChapters => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_screen() -> Screen {
        Screen::Format(FormatState {
            url: "https://youtube.com/watch?v=example".to_string(),
            title: "Example".to_string(),
            playlist: PlaylistSelection::Single,
            selected: 0,
            available_resolutions: vec![Quality::P720],
        })
    }

    fn quality_screen() -> Screen {
        Screen::Quality(QualityState {
            url: "https://youtube.com/watch?v=example".to_string(),
            title: "Example".to_string(),
            playlist: PlaylistSelection::Single,
            format: Format::Video,
            options: vec![Quality::Best, Quality::Compressed, Quality::P720],
            selected: 0,
            scroll_offset: 0,
            note: None,
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
            active_preview: None,
            dependency_report: DependencyReport::default(),
            dependency_receiver: None,
            update_receiver: None,
            clipboard: ClipboardService::unavailable(),
            clipboard_receiver: None,
            clipboard_checked_for_main: false,
            last_clipboard_candidate: None,
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
        navigation.show_error(ErrorState::new(
            "failed".to_string(),
            &DependencyReport::default(),
        ));

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
        let options = [Quality::Best, Quality::Compressed, Quality::P1080];
        assert_eq!(
            options
                .iter()
                .position(|quality| *quality == Quality::P1080),
            Some(2)
        );
        let preview = MetadataPreview {
            title: "Example".to_string(),
            uploader: None,
            duration_seconds: None,
            resolutions: vec!["1080p".to_string(), "480p".to_string()],
            playlist: None,
        };
        assert_eq!(metadata_qualities(&preview), [Quality::P1080]);
    }

    #[test]
    fn clipboard_candidates_must_be_valid_new_urls() {
        assert_eq!(
            clipboard_url_candidate(" https://youtube.com/watch?v=example \n", "", None),
            Some("https://youtube.com/watch?v=example".to_string())
        );
        assert_eq!(
            clipboard_url_candidate("https://media.example/video/1", "", None),
            Some("https://media.example/video/1".to_string())
        );
        assert_eq!(
            clipboard_url_candidate(
                "https://media.example/video/1",
                "",
                Some("https://media.example/video/1")
            ),
            None
        );
        assert_eq!(
            clipboard_url_candidate(
                "https://youtube.com/watch?v=clipboard",
                "https://youtube.com/watch?v=typed",
                None
            ),
            None
        );
        for invalid in ["", "not a URL", "youtube.com/watch?v=example", "--exec=bad"] {
            assert_eq!(
                clipboard_url_candidate(invalid, "", None),
                None,
                "{invalid}"
            );
        }
    }
}
