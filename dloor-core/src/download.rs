use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{
    config::{
        sanitized_ytdlp_error, BandwidthLimit, Config, CookieSource, Destination, MediaOptions,
    },
    detect_platform, DependencyReport, Error, MetadataPreview, Platform, Result,
    YtDlpUpdateOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Format {
    Audio,
    Video,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Video => "Video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Quality {
    Best,
    Compressed,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "2160p")]
    P2160,
}

impl Quality {
    pub const RESOLUTIONS: [Self; 4] = [Self::P720, Self::P1080, Self::P1440, Self::P2160];

    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best",
            Self::Compressed => "Compressed",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::P1440 => "1440p",
            Self::P2160 => "2160p",
        }
    }

    pub fn height(self) -> Option<u32> {
        match self {
            Self::Best | Self::Compressed => None,
            Self::P720 => Some(720),
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
            Self::P2160 => Some(2160),
        }
    }

    pub fn from_height(height: u64) -> Option<Self> {
        match height {
            720 => Some(Self::P720),
            1080 => Some(Self::P1080),
            1440 => Some(Self::P1440),
            2160 => Some(Self::P2160),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    pub format: Format,
    pub quality: Quality,
    pub playlist: PlaylistSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistSelection {
    Single,
    All,
    Item { index: usize },
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub item_percent: f64,
    pub overall_percent: f64,
    pub speed: String,
    pub eta: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    pub index: usize,
    pub total: usize,
    pub title: String,
    pub playlist_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSuccess {
    pub item: DownloadItem,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadFailure {
    pub item: DownloadItem,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadWarningKind {
    SubtitleEmbedding,
    ThumbnailEmbedding,
    ChapterEmbedding,
    SubtitleSidecar,
    OptionalPostProcessing,
    ResolutionFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadWarning {
    pub item: DownloadItem,
    pub kind: DownloadWarningKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadSummary {
    pub total: usize,
    pub succeeded: Vec<DownloadSuccess>,
    pub failed: Vec<DownloadFailure>,
    pub warnings: Vec<DownloadWarning>,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    DependenciesChecked {
        report: DependencyReport,
    },
    YtDlpUpdateFinished {
        outcome: YtDlpUpdateOutcome,
    },
    PreviewReady {
        preview: MetadataPreview,
    },
    PreviewFailed {
        error: String,
    },
    PreviewCancelled,
    Resolving,
    ItemStarted {
        item: DownloadItem,
        platform: Platform,
    },
    Progress {
        progress: DownloadProgress,
        item: DownloadItem,
        platform: Platform,
    },
    Converting {
        item: DownloadItem,
    },
    Uploading {
        item: DownloadItem,
    },
    ItemCompleted {
        result: DownloadSuccess,
    },
    ItemFailed {
        failure: DownloadFailure,
    },
    ItemWarning {
        warning: DownloadWarning,
    },
    Finished {
        summary: DownloadSummary,
    },
    Failed {
        error: String,
    },
    Cancelled {
        summary: DownloadSummary,
    },
}

#[derive(Debug, Clone)]
struct ResolvedItem {
    display: DownloadItem,
    target: DownloadTarget,
}

#[derive(Debug)]
struct ItemRunResult {
    path: String,
    warnings: Vec<DownloadWarning>,
}

#[derive(Debug)]
struct YtdlpOutput {
    path: PathBuf,
    warnings: Vec<DownloadWarning>,
}

#[derive(Debug, Clone, Copy)]
enum DownloadTarget {
    NoPlaylist,
    PlaylistItem(usize),
}

#[derive(Debug, serde::Deserialize)]
struct PlaylistEntry {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    playlist_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub request: DownloadRequest,
    pub config: Config,
    cancellation: CancellationToken,
}

impl DownloadJob {
    pub fn new(request: DownloadRequest, config: Config) -> Self {
        Self {
            request,
            config,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn spawn(self) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            if let Err(error) = self.run_with_sender(&tx).await {
                let event = terminal_event_for_error(error);
                let _ = tx.send(event).await;
            }
        });
        rx
    }

    pub async fn run_with_sender(self, tx: &mpsc::Sender<DownloadEvent>) -> Result<()> {
        let platform = detect_platform(&self.request.url)?;
        tx.send(DownloadEvent::Resolving).await.ok();
        let items = self.resolve_items().await?;
        let mut summary = DownloadSummary {
            total: items.len(),
            ..DownloadSummary::default()
        };

        for resolved in items {
            if self.cancellation.is_cancelled() {
                tx.send(DownloadEvent::Cancelled { summary }).await.ok();
                return Ok(());
            }
            let item = resolved.display.clone();
            tx.send(DownloadEvent::ItemStarted {
                item: item.clone(),
                platform,
            })
            .await
            .ok();

            match self
                .run_item(
                    &resolved,
                    platform,
                    summary.succeeded.len() + summary.failed.len(),
                    tx,
                )
                .await
            {
                Ok(outcome) => {
                    for warning in outcome.warnings {
                        summary.warnings.push(warning.clone());
                        tx.send(DownloadEvent::ItemWarning { warning }).await.ok();
                    }
                    let result = DownloadSuccess {
                        item,
                        path: outcome.path,
                    };
                    summary.succeeded.push(result.clone());
                    tx.send(DownloadEvent::ItemCompleted { result }).await.ok();
                }
                Err(Error::Cancelled) => {
                    tx.send(DownloadEvent::Cancelled { summary }).await.ok();
                    return Ok(());
                }
                Err(error) => {
                    let failure = DownloadFailure {
                        item,
                        error: error.to_string(),
                    };
                    summary.failed.push(failure.clone());
                    tx.send(DownloadEvent::ItemFailed { failure }).await.ok();
                }
            }
        }

        tx.send(DownloadEvent::Finished { summary }).await.ok();
        Ok(())
    }

    async fn run_item(
        &self,
        resolved: &ResolvedItem,
        platform: Platform,
        processed_items: usize,
        tx: &mpsc::Sender<DownloadEvent>,
    ) -> Result<ItemRunResult> {
        let (work_dir, _temp_guard) = self.prepare_work_dir().await?;
        let output_template = work_dir.join("%(title)s [%(id)s].%(ext)s");
        let output = self
            .run_ytdlp(
                &output_template,
                resolved.target,
                resolved.display.clone(),
                processed_items,
                platform,
                tx,
            )
            .await?;
        let downloaded = output.path;
        let mut warnings = output.warnings;
        self.ensure_not_cancelled()?;

        let final_local = if self.request.format == Format::Video
            && self.request.quality == Quality::Compressed
        {
            tx.send(DownloadEvent::Converting {
                item: resolved.display.clone(),
            })
            .await
            .ok();
            transcode_video(&downloaded, &self.cancellation).await?
        } else {
            downloaded
        };
        self.ensure_not_cancelled()?;

        let sidecars = if self.config.media.write_subtitles {
            collect_subtitle_sidecars(&work_dir, &final_local).await?
        } else {
            Vec::new()
        };
        let completed_path = match &self.config.destination {
            Destination::Local { path } => {
                fs::create_dir_all(path).await?;
                let destination = unique_destination(path, &final_local).await?;
                move_file(&final_local, &destination).await?;
                for sidecar in sidecars {
                    if let Err(error) = move_sidecar(&sidecar, path).await {
                        warnings.push(DownloadWarning {
                            item: resolved.display.clone(),
                            kind: DownloadWarningKind::SubtitleSidecar,
                            message: format!(
                                "A subtitle sidecar could not be saved: {}",
                                error_kind(&error)
                            ),
                        });
                    }
                }
                destination.to_string_lossy().to_string()
            }
            Destination::Cloud { remote, path } => {
                tx.send(DownloadEvent::Uploading {
                    item: resolved.display.clone(),
                })
                .await
                .ok();
                upload_to_cloud(&final_local, remote, path, &self.cancellation).await?;
                for sidecar in sidecars {
                    match upload_to_cloud(&sidecar, remote, path, &self.cancellation).await {
                        Ok(()) => {}
                        Err(Error::Cancelled) => return Err(Error::Cancelled),
                        Err(error) => warnings.push(DownloadWarning {
                            item: resolved.display.clone(),
                            kind: DownloadWarningKind::SubtitleSidecar,
                            message: format!(
                                "A subtitle sidecar could not be uploaded: {}",
                                error_kind(&error)
                            ),
                        }),
                    }
                }
                let file_name = final_local
                    .file_name()
                    .ok_or_else(|| Error::InvalidPath(final_local.display().to_string()))?
                    .to_string_lossy();
                format!("{}:{}/{}", remote, path.trim_matches('/'), file_name)
            }
        };
        self.ensure_not_cancelled()?;
        Ok(ItemRunResult {
            path: completed_path,
            warnings,
        })
    }

    fn ensure_not_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    async fn prepare_work_dir(&self) -> Result<(PathBuf, TempDir)> {
        // Always download into a temp dir so existing files at the destination
        // never collide with yt-dlp or ffmpeg output.
        let dir = tempfile::Builder::new().prefix("dloor-").tempdir()?;
        Ok((dir.path().to_path_buf(), dir))
    }

    async fn run_ytdlp(
        &self,
        output_template: &Path,
        target: DownloadTarget,
        item: DownloadItem,
        processed_items: usize,
        platform: Platform,
        tx: &mpsc::Sender<DownloadEvent>,
    ) -> Result<YtdlpOutput> {
        let mut args = base_ytdlp_args(output_template, &self.config.cookies);
        args.extend(format_args(self.request.format, self.request.quality));
        args.extend(bandwidth_args(self.config.bandwidth_limit.as_ref()));
        let (optional_args, mut warnings) = media_args(
            &self.config.media,
            self.request.format,
            self.request.quality,
            &item,
        );
        args.extend(optional_args);
        args.extend(download_target_args(target));
        args.push(OsString::from(&self.request.url));

        debug!(
            format = ?self.request.format,
            quality = ?self.request.quality,
            cookie_source = match self.config.cookies {
                CookieSource::None => "none",
                CookieSource::Browser { .. } => "browser",
                CookieSource::File { .. } => "file",
            },
            "spawning yt-dlp"
        );
        let mut child = Command::new("yt-dlp")
            .args(args)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stderr".to_string()))?;

        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();
        let mut output_path = None;
        let mut selected_height = None;
        let mut stderr_messages = Vec::new();
        let expected_parts = usize::from(self.request.format == Format::Video) + 1;
        let mut progress_tracker = ProgressTracker::new(expected_parts);
        let mut stdout_open = true;
        let mut stderr_open = true;
        while stdout_open || stderr_open {
            tokio::select! {
                _ = self.cancellation.cancelled() => {
                    child.kill().await.ok();
                    return Err(Error::Cancelled);
                }
                line = stdout_lines.next_line(), if stdout_open => {
                    match line? {
                        Some(line) => {
                            if let Some(path) = parse_output_line(&line) {
                                output_path = Some(path);
                            } else if let Some(height) = parse_height_line(&line) {
                                selected_height = Some(height);
                            } else {
                                // before_dl planning records share stdout with the
                                // after_move record, but are unambiguously prefixed.
                                // Actual progress records are read only from stderr.
                                progress_tracker.update(&line);
                            }
                        }
                        None => stdout_open = false,
                    }
                }
                line = stderr_lines.next_line(), if stderr_open => {
                    match line? {
                        Some(line) => {
                            if let Some(progress) = progress_tracker.update(&line) {
                                let progress = with_overall_progress(
                                    progress,
                                    processed_items,
                                    item.total,
                                );
                                tx.send(DownloadEvent::Progress {
                                    progress,
                                    item: item.clone(),
                                    platform,
                                })
                                    .await
                                    .ok();
                            } else {
                                stderr_messages.push(line);
                            }
                        }
                        None => stderr_open = false,
                    }
                }
            }
        }

        let status = tokio::select! {
            _ = self.cancellation.cancelled() => {
                child.kill().await.ok();
                return Err(Error::Cancelled);
            }
            status = child.wait() => status?,
        };
        let stderr = stderr_messages.join("\n");
        warnings.extend(postprocessing_warnings(&stderr, &self.config.media, &item));
        if let (Some(requested), Some(actual)) = (self.request.quality.height(), selected_height) {
            if actual != u64::from(requested) {
                warnings.push(DownloadWarning {
                    item: item.clone(),
                    kind: DownloadWarningKind::ResolutionFallback,
                    message: format!(
                        "Requested {requested}p was unavailable; yt-dlp selected {actual}p."
                    ),
                });
            }
        }
        deduplicate_warnings(&mut warnings);
        let output_path = output_path.ok_or(Error::MissingOutputFile)?;
        let output_is_valid = output_path.extension().is_none_or(|ext| ext != "part")
            && fs::metadata(&output_path)
                .await
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
        if !status.success() {
            if output_is_valid && !warnings.is_empty() {
                return Ok(YtdlpOutput {
                    path: output_path,
                    warnings,
                });
            }
            let message = if stderr.trim().is_empty() {
                format!("yt-dlp exited with {status}")
            } else {
                sanitized_ytdlp_error(&stderr, &self.config.cookies)
            };
            return Err(Error::ProcessFailed(message));
        }
        if !output_is_valid {
            return Err(Error::MissingOutputFile);
        }
        Ok(YtdlpOutput {
            path: output_path,
            warnings,
        })
    }

    async fn resolve_items(&self) -> Result<Vec<ResolvedItem>> {
        match self.request.playlist {
            PlaylistSelection::Single => {
                match self
                    .resolve_with_args(&["--no-playlist"], DownloadTarget::NoPlaylist)
                    .await
                {
                    Ok(items) if !items.is_empty() => Ok(limit_and_number_items(items, 1)),
                    Ok(_) | Err(Error::ProcessFailed(_)) | Err(Error::MissingOutputFile) => {
                        self.ensure_not_cancelled()?;
                        let items = self
                            .resolve_with_args(
                                &["--yes-playlist", "--playlist-end", "1"],
                                DownloadTarget::PlaylistItem(1),
                            )
                            .await?;
                        Ok(limit_and_number_items(items, 1))
                    }
                    Err(error) => Err(error),
                }
            }
            PlaylistSelection::All => {
                let items = self
                    .resolve_with_args(&["--yes-playlist"], DownloadTarget::PlaylistItem(1))
                    .await?;
                Ok(number_playlist_items(items))
            }
            PlaylistSelection::Item { index } => {
                let index_text = index.to_string();
                let items = self
                    .resolve_with_args(
                        &["--yes-playlist", "--playlist-items", &index_text],
                        DownloadTarget::PlaylistItem(index),
                    )
                    .await?;
                Ok(limit_and_number_items(items, 1))
            }
        }
    }

    async fn resolve_with_args(
        &self,
        selection_args: &[&str],
        default_target: DownloadTarget,
    ) -> Result<Vec<ResolvedItem>> {
        let mut command = Command::new("yt-dlp");
        command.args([
            "--flat-playlist",
            "--dump-json",
            "--no-download",
            "--ignore-errors",
        ]);
        command.args(selection_args);
        command.args(self.config.cookies.yt_dlp_args());
        command.arg(&self.request.url);
        command
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stderr".to_string()))?;
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut values = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                values.push(line);
            }
            values
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut values = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                values.push(line);
            }
            values.join("\n")
        });
        let status = tokio::select! {
            _ = self.cancellation.cancelled() => {
                child.kill().await.ok();
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                return Err(Error::Cancelled);
            }
            status = child.wait() => status?,
        };
        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();
        if !status.success() {
            let message = if stderr.trim().is_empty() {
                format!("yt-dlp metadata expansion exited with {status}")
            } else {
                sanitized_ytdlp_error(&stderr, &self.config.cookies)
            };
            return Err(Error::ProcessFailed(message));
        }
        let entries = parse_playlist_entries(stdout.iter().map(String::as_str), default_target)?;
        if entries.is_empty() {
            return Err(Error::MissingOutputFile);
        }
        Ok(entries)
    }
}

fn parse_playlist_entries<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    default_target: DownloadTarget,
) -> Result<Vec<ResolvedItem>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(position, line)| {
            let entry: PlaylistEntry = serde_json::from_str(line).map_err(|_| {
                Error::ProcessFailed("yt-dlp returned invalid metadata JSON".to_string())
            })?;
            let fallback_index = position + 1;
            let playlist_index = entry.playlist_index.or(match default_target {
                DownloadTarget::NoPlaylist => None,
                DownloadTarget::PlaylistItem(index) => Some(index + position),
            });
            let target = match default_target {
                DownloadTarget::NoPlaylist => DownloadTarget::NoPlaylist,
                DownloadTarget::PlaylistItem(_) => {
                    DownloadTarget::PlaylistItem(playlist_index.unwrap_or(fallback_index))
                }
            };
            Ok(ResolvedItem {
                display: DownloadItem {
                    index: fallback_index,
                    total: 0,
                    title: entry
                        .title
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| format!("Item {fallback_index}")),
                    playlist_index,
                },
                target,
            })
        })
        .collect()
}

fn number_playlist_items(mut items: Vec<ResolvedItem>) -> Vec<ResolvedItem> {
    let total = items.len();
    for (position, item) in items.iter_mut().enumerate() {
        item.display.index = position + 1;
        item.display.total = total;
    }
    items
}

fn limit_and_number_items(mut items: Vec<ResolvedItem>, limit: usize) -> Vec<ResolvedItem> {
    items.truncate(limit);
    number_playlist_items(items)
}

fn with_overall_progress(
    mut progress: DownloadProgress,
    processed_items: usize,
    total_items: usize,
) -> DownloadProgress {
    let total_items = total_items.max(1);
    progress.overall_percent =
        ((processed_items as f64 + progress.item_percent / 100.0) / total_items as f64 * 100.0)
            .clamp(0.0, 100.0);
    progress
}

fn terminal_event_for_error(error: Error) -> DownloadEvent {
    if matches!(error, Error::Cancelled) {
        debug!("download job cancelled");
        DownloadEvent::Cancelled {
            summary: DownloadSummary::default(),
        }
    } else {
        error!(kind = error_kind(&error), "download job failed");
        DownloadEvent::Failed {
            error: error.to_string(),
        }
    }
}

fn error_kind(error: &Error) -> &'static str {
    match error {
        Error::UnsupportedUrl(_) => "unsupported_url",
        Error::ConfigDirUnavailable => "config_dir_unavailable",
        Error::Io(_) => "io",
        Error::TomlDecode(_) => "toml_decode",
        Error::TomlEncode(_) => "toml_encode",
        Error::Json(_) => "json",
        Error::MissingTool(_) => "missing_tool",
        Error::ProcessFailed(_) => "process_failed",
        Error::Cancelled => "cancelled",
        Error::MissingOutputFile => "missing_output_file",
        Error::InvalidPath(_) => "invalid_path",
    }
}

const OUTPUT_PREFIX: &str = "DLOOR_OUTPUT|";
const HEIGHT_PREFIX: &str = "DLOOR_HEIGHT|";
const PLAN_PREFIX: &str = "DLOOR_PLAN|";
const PROGRESS_PREFIX: &str = "DLOOR_PROGRESS|";

fn base_ytdlp_args(output_template: &Path, cookies: &CookieSource) -> Vec<OsString> {
    let mut args = vec![
        "--newline".into(),
        "--progress".into(),
        "--progress-template".into(),
        concat!(
            "download:DLOOR_PROGRESS|%(progress._percent_str)s|",
            "%(progress.downloaded_bytes)s|",
            "%(progress.total_bytes,progress.total_bytes_estimate)s|",
            "%(progress.fragment_index)s|%(progress.fragment_count)s|",
            "%(info.playlist_index)s|%(info.n_entries)s|%(info.format_id)s|",
            "%(progress._speed_str)s|%(progress._eta_str)s"
        )
        .into(),
        "--print".into(),
        concat!(
            "before_dl:DLOOR_PLAN|%(playlist_index)s|%(n_entries)s|",
            "%(format_id)s|%(filesize,filesize_approx)s|",
            "%(requested_formats.0.format_id)s|",
            "%(requested_formats.0.filesize,requested_formats.0.filesize_approx)s|",
            "%(requested_formats.1.format_id)s|",
            "%(requested_formats.1.filesize,requested_formats.1.filesize_approx)s"
        )
        .into(),
        "--print".into(),
        "after_move:DLOOR_OUTPUT|%(filepath)s".into(),
        "--print".into(),
        "after_move:DLOOR_HEIGHT|%(height)s".into(),
        "-o".into(),
        output_template.as_os_str().to_os_string(),
    ];
    args.extend(cookies.yt_dlp_args());
    args
}

fn format_args(format: Format, quality: Quality) -> Vec<OsString> {
    match (format, quality) {
        (Format::Video, Quality::Best) => vec![
            "-f".into(),
            "bestvideo*+bestaudio/best".into(),
            "--merge-output-format".into(),
            "mp4".into(),
        ],
        (Format::Video, Quality::Compressed) => vec![
            "-f".into(),
            "bv*[height<=1080]+ba/b[height<=1080]/best".into(),
            "--merge-output-format".into(),
            "mp4".into(),
        ],
        (Format::Audio, Quality::Best) => vec![
            "-f".into(),
            "bestaudio".into(),
            "-x".into(),
            "--audio-format".into(),
            "m4a".into(),
            "--audio-quality".into(),
            "0".into(),
        ],
        (Format::Audio, Quality::Compressed) => vec![
            "-f".into(),
            "bestaudio".into(),
            "-x".into(),
            "--audio-format".into(),
            "mp3".into(),
            "--audio-quality".into(),
            "5".into(),
        ],
        (Format::Video, quality) => {
            let height = quality
                .height()
                .expect("remaining video quality variants have a height");
            vec![
                "-f".into(),
                format!("bestvideo*[height<={height}]+bestaudio/best[height<={height}]/best")
                    .into(),
                "--merge-output-format".into(),
                "mp4".into(),
            ]
        }
        (Format::Audio, Quality::P720 | Quality::P1080 | Quality::P1440 | Quality::P2160) => {
            vec![
                "-f".into(),
                "bestaudio".into(),
                "-x".into(),
                "--audio-format".into(),
                "m4a".into(),
                "--audio-quality".into(),
                "0".into(),
            ]
        }
    }
}

fn bandwidth_args(limit: Option<&BandwidthLimit>) -> Vec<OsString> {
    limit.map_or_else(Vec::new, |limit| {
        vec!["--limit-rate".into(), limit.as_str().into()]
    })
}

fn media_args(
    options: &MediaOptions,
    format: Format,
    quality: Quality,
    item: &DownloadItem,
) -> (Vec<OsString>, Vec<DownloadWarning>) {
    let mut args = Vec::new();
    let mut warnings = Vec::new();
    let wants_subtitles = options.write_subtitles || options.embed_subtitles;
    if wants_subtitles {
        args.push("--write-subs".into());
        if options.include_auto_subtitles {
            args.push("--write-auto-subs".into());
        }
        let languages = options
            .subtitle_languages
            .iter()
            .map(|language| language.trim())
            .filter(|language| !language.is_empty())
            .collect::<Vec<_>>();
        if !languages.is_empty() {
            args.push("--sub-langs".into());
            args.push(languages.join(",").into());
        }
    }

    let mut embeds_anything = false;
    if options.embed_subtitles {
        if format == Format::Video {
            args.push("--embed-subs".into());
            embeds_anything = true;
        } else {
            warnings.push(DownloadWarning {
                item: item.clone(),
                kind: DownloadWarningKind::SubtitleEmbedding,
                message:
                    "Subtitle embedding is unavailable for audio output; requested sidecars are still written."
                        .to_string(),
            });
        }
    }

    if options.embed_thumbnail {
        if format == Format::Video && quality == Quality::Compressed {
            warnings.push(DownloadWarning {
                item: item.clone(),
                kind: DownloadWarningKind::ThumbnailEmbedding,
                message:
                    "Thumbnail embedding is skipped because the compressed-video transcode cannot retain it reliably."
                        .to_string(),
            });
        } else {
            args.extend(["--write-thumbnail".into(), "--embed-thumbnail".into()]);
            embeds_anything = true;
        }
    }

    if options.embed_chapters {
        if format == Format::Audio && quality == Quality::Compressed {
            warnings.push(DownloadWarning {
                item: item.clone(),
                kind: DownloadWarningKind::ChapterEmbedding,
                message:
                    "Chapter embedding is skipped for compressed MP3 output because support varies by player."
                        .to_string(),
            });
        } else {
            args.push("--embed-chapters".into());
            embeds_anything = true;
        }
    }

    if embeds_anything {
        args.push("--ignore-errors".into());
    }
    (args, warnings)
}

fn postprocessing_warnings(
    stderr: &str,
    options: &MediaOptions,
    item: &DownloadItem,
) -> Vec<DownloadWarning> {
    let lower = stderr.to_ascii_lowercase();
    let indicates_failure = lower.contains("error")
        || lower.contains("failed")
        || lower.contains("unable")
        || lower.contains("unsupported");
    if !indicates_failure {
        return Vec::new();
    }

    let mut warnings = Vec::new();
    if options.embed_subtitles && (lower.contains("subtitle") || lower.contains("embedsubtitles")) {
        warnings.push(DownloadWarning {
            item: item.clone(),
            kind: DownloadWarningKind::SubtitleEmbedding,
            message: "Subtitle embedding was not fully applied; the downloaded media was retained."
                .to_string(),
        });
    }
    if options.embed_thumbnail
        && (lower.contains("thumbnail")
            || lower.contains("embedthumbnail")
            || lower.contains("atomicparsley"))
    {
        warnings.push(DownloadWarning {
            item: item.clone(),
            kind: DownloadWarningKind::ThumbnailEmbedding,
            message:
                "Thumbnail embedding was not fully applied; the downloaded media was retained."
                    .to_string(),
        });
    }
    if options.embed_chapters && (lower.contains("chapter") || lower.contains("metadata")) {
        warnings.push(DownloadWarning {
            item: item.clone(),
            kind: DownloadWarningKind::ChapterEmbedding,
            message: "Chapter embedding was not fully applied; the downloaded media was retained."
                .to_string(),
        });
    }
    if warnings.is_empty()
        && (options.embed_subtitles || options.embed_thumbnail || options.embed_chapters)
        && (lower.contains("postprocess")
            || lower.contains("post-process")
            || lower.contains("embed"))
    {
        warnings.push(DownloadWarning {
            item: item.clone(),
            kind: DownloadWarningKind::OptionalPostProcessing,
            message:
                "Optional media post-processing was not fully applied; the downloaded media was retained."
                    .to_string(),
        });
    }
    warnings
}

fn deduplicate_warnings(warnings: &mut Vec<DownloadWarning>) {
    let mut seen = HashSet::new();
    warnings.retain(|warning| seen.insert(warning.kind));
}

fn download_target_args(target: DownloadTarget) -> Vec<OsString> {
    match target {
        DownloadTarget::NoPlaylist => vec!["--no-playlist".into()],
        DownloadTarget::PlaylistItem(index) => vec![
            "--yes-playlist".into(),
            "--playlist-items".into(),
            index.max(1).to_string().into(),
        ],
    }
}

#[derive(Debug)]
struct ParsedProgress {
    percent: Option<f64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    fragment_index: Option<u64>,
    fragment_count: Option<u64>,
    playlist_index: Option<u64>,
    format_id: String,
    speed: String,
    eta: String,
}

#[derive(Debug, Default)]
struct ProgressPlan {
    format_ids: Vec<String>,
    sizes: HashMap<String, u64>,
    downloaded: HashMap<String, u64>,
}

#[derive(Debug)]
struct ProgressTracker {
    plans: HashMap<u64, ProgressPlan>,
    active_plan_index: Option<u64>,
    completed_formats: HashSet<(u64, String)>,
    last_playlist_index: Option<u64>,
    last_format_id: Option<String>,
    max_percent: f64,
    fallback_part_count: usize,
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new(1)
    }
}

impl ProgressTracker {
    fn new(fallback_part_count: usize) -> Self {
        Self {
            plans: HashMap::new(),
            active_plan_index: None,
            completed_formats: HashSet::new(),
            last_playlist_index: None,
            last_format_id: None,
            max_percent: 0.0,
            fallback_part_count: fallback_part_count.max(1),
        }
    }

    fn update(&mut self, line: &str) -> Option<DownloadProgress> {
        if let Some((playlist_index, plan)) = parse_plan_line(line) {
            self.active_plan_index = Some(playlist_index);
            self.plans.insert(playlist_index, plan);
            return None;
        }

        let parsed = parse_progress_fields(line)?;
        let playlist_index = parsed
            .playlist_index
            .or(self.active_plan_index)
            .unwrap_or(1)
            .max(1);

        if self.last_playlist_index == Some(playlist_index)
            && self.last_format_id.as_deref() != Some(parsed.format_id.as_str())
        {
            if let Some(previous) = self.last_format_id.take() {
                self.completed_formats.insert((playlist_index, previous));
            }
        }
        self.last_playlist_index = Some(playlist_index);
        self.last_format_id = Some(parsed.format_id.clone());

        let part_fraction = match (parsed.downloaded_bytes, parsed.total_bytes) {
            (Some(downloaded), Some(total)) if total > 0 => Some(downloaded as f64 / total as f64),
            _ => match (parsed.fragment_index, parsed.fragment_count) {
                (Some(index), Some(count)) if count > 0 => Some(index as f64 / count as f64),
                _ => parsed.percent.map(|percent| percent / 100.0),
            },
        }
        .map(|fraction| fraction.clamp(0.0, 1.0))?;

        let item_fraction = self.plans.get_mut(&playlist_index).map_or_else(
            || {
                let completed_parts = self
                    .completed_formats
                    .iter()
                    .filter(|(item, _)| *item == playlist_index)
                    .count()
                    .min(self.fallback_part_count - 1);
                (completed_parts as f64 + part_fraction) / self.fallback_part_count as f64
            },
            |plan| {
                if let Some(total) = parsed.total_bytes.filter(|total| *total > 0) {
                    plan.sizes.insert(parsed.format_id.clone(), total);
                }
                if let Some(downloaded) = parsed.downloaded_bytes {
                    plan.downloaded.insert(parsed.format_id.clone(), downloaded);
                }
                let all_sizes_known = !plan.format_ids.is_empty()
                    && plan
                        .format_ids
                        .iter()
                        .all(|format_id| plan.sizes.get(format_id).is_some_and(|size| *size > 0));
                let fraction = if all_sizes_known {
                    let total_bytes: u64 = plan
                        .format_ids
                        .iter()
                        .filter_map(|format_id| plan.sizes.get(format_id))
                        .sum();
                    let downloaded_bytes: u64 = plan
                        .format_ids
                        .iter()
                        .filter_map(|format_id| {
                            let total = plan.sizes.get(format_id)?;
                            Some(
                                plan.downloaded
                                    .get(format_id)
                                    .copied()
                                    .unwrap_or(0)
                                    .min(*total),
                            )
                        })
                        .sum();
                    downloaded_bytes as f64 / total_bytes as f64
                } else {
                    let part_count = plan.format_ids.len().max(self.fallback_part_count);
                    let completed_parts = self
                        .completed_formats
                        .iter()
                        .filter(|(item, format_id)| {
                            *item == playlist_index
                                && (plan.format_ids.is_empty()
                                    || plan.format_ids.contains(format_id))
                        })
                        .count()
                        .min(part_count - 1);
                    (completed_parts as f64 + part_fraction) / part_count as f64
                };
                fraction
            },
        );
        self.max_percent = self
            .max_percent
            .max((item_fraction * 100.0).clamp(0.0, 100.0));

        Some(DownloadProgress {
            item_percent: self.max_percent,
            overall_percent: self.max_percent,
            speed: parsed.speed,
            eta: parsed.eta,
        })
    }
}

fn parse_progress_fields(line: &str) -> Option<ParsedProgress> {
    let line = line.strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = line.splitn(10, '|').map(str::trim);
    let percent = parts
        .next()?
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok();
    let downloaded_bytes = parse_optional_u64(parts.next()?);
    let total_bytes = parse_optional_u64(parts.next()?);
    let fragment_index = parse_optional_u64(parts.next()?);
    let fragment_count = parse_optional_u64(parts.next()?);
    let playlist_index = parse_optional_u64(parts.next()?);
    let _n_entries = parse_optional_u64(parts.next()?);
    let format_id = parts.next()?.to_string();

    Some(ParsedProgress {
        percent,
        downloaded_bytes,
        total_bytes,
        fragment_index,
        fragment_count,
        playlist_index,
        format_id,
        speed: parts.next().unwrap_or("").to_string(),
        eta: parts.next().unwrap_or("").to_string(),
    })
}

fn parse_plan_line(line: &str) -> Option<(u64, ProgressPlan)> {
    let line = line.strip_prefix(PLAN_PREFIX)?;
    let mut parts = line.splitn(8, '|').map(str::trim);
    let playlist_index = parse_optional_u64(parts.next()?).unwrap_or(1).max(1);
    let _n_entries = parse_optional_u64(parts.next()?);
    let format_id = parts.next()?;
    let format_size = parse_optional_u64(parts.next()?);
    let requested = [
        (parts.next()?, parse_optional_u64(parts.next()?)),
        (parts.next()?, parse_optional_u64(parts.next()?)),
    ];
    let mut format_ids = Vec::new();
    let mut sizes = HashMap::new();
    for (id, size) in requested {
        if id != "NA" && !id.is_empty() {
            format_ids.push(id.to_string());
            if let Some(size) = size {
                sizes.insert(id.to_string(), size);
            }
        }
    }
    if format_ids.is_empty() {
        format_ids.push(format_id.to_string());
        if let Some(size) = format_size {
            sizes.insert(format_id.to_string(), size);
        }
    }

    Some((
        playlist_index,
        ProgressPlan {
            format_ids,
            sizes,
            downloaded: HashMap::new(),
        },
    ))
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn parse_output_line(line: &str) -> Option<PathBuf> {
    line.strip_prefix(OUTPUT_PREFIX).map(PathBuf::from)
}

fn parse_height_line(line: &str) -> Option<u64> {
    line.strip_prefix(HEIGHT_PREFIX)?.trim().parse().ok()
}

pub fn parse_progress_line(line: &str) -> Option<DownloadProgress> {
    ProgressTracker::default().update(line)
}

async fn unique_destination(dir: &Path, source: &Path) -> Result<PathBuf> {
    let file_name = source
        .file_name()
        .ok_or_else(|| Error::InvalidPath(source.display().to_string()))?;
    let candidate = dir.join(file_name);
    if !fs::try_exists(&candidate).await? {
        return Ok(candidate);
    }

    let stem = source
        .file_stem()
        .ok_or_else(|| Error::InvalidPath(source.display().to_string()))?
        .to_string_lossy();
    let extension = source.extension().map(|ext| ext.to_string_lossy());

    for counter in 1..=10_000u32 {
        let candidate = match &extension {
            Some(ext) => dir.join(format!("{stem} ({counter}).{ext}")),
            None => dir.join(format!("{stem} ({counter})")),
        };
        if !fs::try_exists(&candidate).await? {
            return Ok(candidate);
        }
    }

    Err(Error::InvalidPath(candidate.display().to_string()))
}

async fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    // rename fails across filesystems (temp dir vs destination), so copy + remove instead
    fs::copy(from, to).await?;
    fs::remove_file(from).await?;
    Ok(())
}

async fn collect_subtitle_sidecars(dir: &Path, media: &Path) -> Result<Vec<PathBuf>> {
    const SUBTITLE_EXTENSIONS: [&str; 8] =
        ["srt", "vtt", "ass", "ssa", "lrc", "ttml", "srv1", "srv3"];
    let mut entries = fs::read_dir(dir).await?;
    let mut sidecars = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path != media
            && path.extension().is_some_and(|extension| {
                SUBTITLE_EXTENSIONS
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
            && entry.file_type().await?.is_file()
        {
            sidecars.push(path);
        }
    }
    sidecars.sort();
    Ok(sidecars)
}

async fn move_sidecar(sidecar: &Path, destination_dir: &Path) -> Result<PathBuf> {
    let destination = unique_destination(destination_dir, sidecar).await?;
    move_file(sidecar, &destination).await?;
    Ok(destination)
}

async fn transcode_video(input: &Path, cancellation: &CancellationToken) -> Result<PathBuf> {
    let stem = input
        .file_stem()
        .ok_or_else(|| Error::InvalidPath(input.display().to_string()))?
        .to_string_lossy();
    let output = input.with_file_name(format!("{stem}.compressed.mp4"));
    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input.to_string_lossy().as_ref(),
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-map",
            "0:s?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
            "-c:v",
            "libx264",
            "-crf",
            "28",
            "-preset",
            "fast",
            "-vf",
            "scale='min(1920,iw)':-2",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-c:s",
            "mov_text",
            output.to_string_lossy().as_ref(),
        ])
        .kill_on_drop(true)
        .spawn()?;
    let status = wait_for_process(&mut child, cancellation).await?;

    if status.success() {
        finalize_transcode(input, &output).await
    } else {
        Err(Error::ProcessFailed(format!("ffmpeg exited with {status}")))
    }
}

async fn finalize_transcode(input: &Path, output: &Path) -> Result<PathBuf> {
    let metadata = fs::metadata(output)
        .await
        .map_err(|_| Error::MissingOutputFile)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(Error::MissingOutputFile);
    }
    fs::remove_file(input).await?;
    Ok(output.to_path_buf())
}

async fn upload_to_cloud(
    file: &Path,
    remote: &str,
    remote_path: &str,
    cancellation: &CancellationToken,
) -> Result<()> {
    let target = format!("{}:{}", remote, remote_path.trim_matches('/'));
    let mut child = Command::new("rclone")
        .args(["copy", file.to_string_lossy().as_ref(), target.as_str()])
        .kill_on_drop(true)
        .spawn()?;
    let status = wait_for_process(&mut child, cancellation).await?;
    if status.success() {
        fs::remove_file(file).await.ok();
        Ok(())
    } else {
        Err(Error::ProcessFailed(format!("rclone exited with {status}")))
    }
}

async fn wait_for_process(
    child: &mut tokio::process::Child,
    cancellation: &CancellationToken,
) -> Result<std::process::ExitStatus> {
    tokio::select! {
        _ = cancellation.cancelled() => {
            child.kill().await.ok();
            Err(Error::Cancelled)
        }
        status = child.wait() => Ok(status?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_template_output() {
        let progress =
            parse_progress_line("DLOOR_PROGRESS|42.7%|427|1000|NA|NA|NA|NA|18|1.24MiB/s|00:18")
                .unwrap();
        assert!((progress.item_percent - 42.7).abs() < 1e-9);
        assert!((progress.overall_percent - 42.7).abs() < 1e-9);
        assert_eq!(progress.speed, "1.24MiB/s");
        assert_eq!(progress.eta, "00:18");
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_progress_line("[download] Destination: file.mp4").is_none());
    }

    #[test]
    fn combines_video_and_audio_progress_without_resetting() {
        let mut tracker = ProgressTracker::new(2);
        tracker.update("DLOOR_PLAN|NA|NA|137+140|1100|137|1000|140|100");
        let video = tracker
            .update("DLOOR_PROGRESS|100%|1000|1000|NA|NA|NA|NA|137|1MiB/s|00:00")
            .unwrap();
        let audio_start = tracker
            .update("DLOOR_PROGRESS|0%|0|100|NA|NA|NA|NA|140|1MiB/s|00:01")
            .unwrap();
        let audio_half = tracker
            .update("DLOOR_PROGRESS|50%|50|100|NA|NA|NA|NA|140|1MiB/s|00:01")
            .unwrap();

        assert!((video.item_percent - 90.909).abs() < 0.001);
        assert_eq!(audio_start.item_percent, video.item_percent);
        assert!((audio_half.item_percent - 95.454).abs() < 0.001);
    }

    #[test]
    fn selected_playlist_item_progress_stays_item_local() {
        let mut tracker = ProgressTracker::new(2);
        tracker.update("DLOOR_PLAN|2|4|137+140|200|137|100|140|100");
        let progress = tracker
            .update("DLOOR_PROGRESS|50%|50|100|NA|NA|2|4|137|1MiB/s|00:01")
            .unwrap();

        assert_eq!(progress.item_percent, 25.0);
    }

    #[test]
    fn uses_equal_phases_until_every_format_size_is_known() {
        let mut tracker = ProgressTracker::new(2);
        tracker.update("DLOOR_PLAN|NA|NA|137+140|NA|137|1000|140|NA");
        let video = tracker
            .update("DLOOR_PROGRESS|100%|1000|1000|NA|NA|NA|NA|137|1MiB/s|00:00")
            .unwrap();
        let audio = tracker
            .update("DLOOR_PROGRESS|50%|NA|NA|NA|NA|NA|NA|140|NA|NA")
            .unwrap();

        assert_eq!(video.item_percent, 50.0);
        assert_eq!(audio.item_percent, 75.0);
    }

    #[test]
    fn uses_fragment_progress_when_stream_size_and_percent_are_unknown() {
        let mut tracker = ProgressTracker::new(1);
        let progress = tracker
            .update("DLOOR_PROGRESS|NA|NA|NA|3|12|NA|NA|hls|NA|NA")
            .unwrap();

        assert_eq!(progress.item_percent, 25.0);
    }

    #[test]
    fn ignores_progress_when_no_numeric_measure_is_available() {
        let mut tracker = ProgressTracker::new(1);

        assert!(tracker
            .update("DLOOR_PROGRESS|NA|NA|NA|NA|NA|NA|NA|hls|NA|NA")
            .is_none());
    }

    #[test]
    fn parses_and_numbers_playlist_entries() {
        let entries = parse_playlist_entries(
            [
                r#"{"title":"First","playlist_index":4}"#,
                r#"{"title":"Second","playlist_index":5}"#,
            ],
            DownloadTarget::PlaylistItem(1),
        )
        .unwrap();
        let entries = number_playlist_items(entries);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].display.index, 1);
        assert_eq!(entries[0].display.total, 2);
        assert_eq!(entries[0].display.title, "First");
        assert_eq!(entries[0].display.playlist_index, Some(4));
        assert!(matches!(entries[1].target, DownloadTarget::PlaylistItem(5)));
    }

    #[test]
    fn playlist_json_uses_a_safe_title_fallback() {
        let entries = parse_playlist_entries(
            [r#"{"title":"","playlist_index":1}"#],
            DownloadTarget::PlaylistItem(1),
        )
        .unwrap();

        assert_eq!(entries[0].display.title, "Item 1");
    }

    #[test]
    fn playlist_overall_progress_includes_completed_items() {
        let progress = with_overall_progress(
            DownloadProgress {
                item_percent: 50.0,
                overall_percent: 50.0,
                speed: String::new(),
                eta: String::new(),
            },
            2,
            4,
        );

        assert_eq!(progress.item_percent, 50.0);
        assert_eq!(progress.overall_percent, 62.5);
    }

    #[test]
    fn download_target_switches_playlist_flags() {
        assert_eq!(
            download_target_args(DownloadTarget::NoPlaylist),
            [OsString::from("--no-playlist")]
        );
        assert_eq!(
            download_target_args(DownloadTarget::PlaylistItem(7)),
            [
                OsString::from("--yes-playlist"),
                OsString::from("--playlist-items"),
                OsString::from("7"),
            ]
        );
    }

    #[test]
    fn parses_confirmed_output_path() {
        assert_eq!(
            parse_output_line("DLOOR_OUTPUT|/tmp/video name.mp4"),
            Some(PathBuf::from("/tmp/video name.mp4"))
        );
        assert_eq!(
            parse_output_line("[download] /tmp/unrelated.mp4"),
            None,
            "only the dedicated after_move record may select the output"
        );
    }

    #[test]
    fn browser_cookie_args_are_only_added_when_enabled() {
        let output = Path::new("video.%(ext)s");
        let without_auth = base_ytdlp_args(output, &CookieSource::None);
        assert!(!without_auth.contains(&OsString::from("--cookies-from-browser")));
        assert!(without_auth
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("before_dl:DLOOR_PLAN|")));
        assert!(without_auth
            .iter()
            .any(|arg| arg == "after_move:DLOOR_OUTPUT|%(filepath)s"));

        let with_auth = base_ytdlp_args(
            output,
            &CookieSource::Browser {
                browser: crate::Browser::Firefox,
            },
        );
        assert!(with_auth
            .windows(2)
            .any(|args| args == ["--cookies-from-browser", "firefox"]));
    }

    #[tokio::test]
    async fn unique_destination_appends_counter_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("video.mp4"), b"").await.unwrap();

        let destination = unique_destination(dir.path(), Path::new("video.mp4"))
            .await
            .unwrap();

        assert_eq!(destination, dir.path().join("video (1).mp4"));
    }

    #[tokio::test]
    async fn cancellation_stops_a_running_child_process() {
        let cancellation = CancellationToken::new();
        let mut child = Command::new("sh")
            .args(["-c", "sleep 10"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        cancellation.cancel();

        let result = wait_for_process(&mut child, &cancellation).await;

        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    fn cancellation_maps_to_cancelled_terminal_event() {
        assert!(matches!(
            terminal_event_for_error(Error::Cancelled),
            DownloadEvent::Cancelled { .. }
        ));
        assert!(matches!(
            terminal_event_for_error(Error::ProcessFailed("boom".to_string())),
            DownloadEvent::Failed { .. }
        ));
    }

    #[test]
    fn cancelled_job_does_not_advance_to_the_next_stage() {
        let job = DownloadJob::new(
            DownloadRequest {
                url: "https://example.com/video".to_string(),
                format: Format::Video,
                quality: Quality::Best,
                playlist: PlaylistSelection::Single,
            },
            Config::default(),
        );
        job.cancellation_token().cancel();

        assert!(matches!(job.ensure_not_cancelled(), Err(Error::Cancelled)));
    }

    #[tokio::test]
    async fn temporary_partial_files_are_removed_with_the_work_directory() {
        let job = DownloadJob::new(
            DownloadRequest {
                url: "https://example.com/video".to_string(),
                format: Format::Video,
                quality: Quality::Best,
                playlist: PlaylistSelection::Single,
            },
            Config::default(),
        );
        let partial_path = {
            let (work_dir, guard) = job.prepare_work_dir().await.unwrap();
            let partial_path = work_dir.join("video.mp4.part");
            fs::write(&partial_path, b"partial").await.unwrap();
            assert!(partial_path.exists());
            drop(guard);
            partial_path
        };

        assert!(!partial_path.exists());
    }

    #[tokio::test]
    async fn successful_transcode_keeps_only_the_confirmed_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("video.mp4");
        let output = dir.path().join("video.compressed.mp4");
        fs::write(&input, b"source").await.unwrap();
        fs::write(&output, b"compressed").await.unwrap();

        let completed_path = finalize_transcode(&input, &output).await.unwrap();

        assert_eq!(completed_path, output);
        assert!(!input.exists());
        assert!(output.exists());
    }

    #[tokio::test]
    async fn missing_transcode_output_does_not_delete_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("video.mp4");
        let output = dir.path().join("video.compressed.mp4");
        fs::write(&input, b"source").await.unwrap();

        let result = finalize_transcode(&input, &output).await;

        assert!(matches!(result, Err(Error::MissingOutputFile)));
        assert!(input.exists());
    }

    #[test]
    fn optional_media_arguments_and_unsupported_combinations_are_explicit() {
        let item = DownloadItem {
            index: 1,
            total: 1,
            title: "Example".to_string(),
            playlist_index: None,
        };
        let options = MediaOptions {
            write_subtitles: true,
            embed_subtitles: true,
            subtitle_languages: vec!["en".to_string(), "ja".to_string()],
            include_auto_subtitles: true,
            embed_thumbnail: true,
            embed_chapters: true,
        };

        let (video_args, video_warnings) =
            media_args(&options, Format::Video, Quality::Best, &item);
        for expected in [
            "--write-subs",
            "--write-auto-subs",
            "--embed-subs",
            "--embed-thumbnail",
            "--embed-chapters",
            "--ignore-errors",
        ] {
            assert!(video_args.contains(&OsString::from(expected)), "{expected}");
        }
        assert!(video_args
            .windows(2)
            .any(|args| args == ["--sub-langs", "en,ja"]));
        assert!(video_warnings.is_empty());

        let (audio_args, audio_warnings) =
            media_args(&options, Format::Audio, Quality::Compressed, &item);
        assert!(!audio_args.contains(&OsString::from("--embed-subs")));
        assert!(!audio_args.contains(&OsString::from("--embed-chapters")));
        assert!(audio_warnings
            .iter()
            .any(|warning| warning.kind == DownloadWarningKind::SubtitleEmbedding));
        assert!(audio_warnings
            .iter()
            .any(|warning| warning.kind == DownloadWarningKind::ChapterEmbedding));
    }

    #[test]
    fn embedding_errors_become_sanitized_warnings() {
        let item = DownloadItem {
            index: 1,
            total: 1,
            title: "Example".to_string(),
            playlist_index: None,
        };
        let options = MediaOptions {
            embed_thumbnail: true,
            ..MediaOptions::default()
        };

        let warnings = postprocessing_warnings(
            "ERROR: EmbedThumbnail failed for /private/video.mp4",
            &options,
            &item,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, DownloadWarningKind::ThumbnailEmbedding);
        assert!(!warnings[0].message.contains("/private/video.mp4"));
    }

    #[tokio::test]
    async fn explicit_subtitle_sidecars_are_collected_without_guessing_by_time() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("video.mp4");
        let subtitle = dir.path().join("video.en.vtt");
        fs::write(&media, b"media").await.unwrap();
        fs::write(&subtitle, b"subtitle").await.unwrap();
        fs::write(dir.path().join("thumbnail.webp"), b"image")
            .await
            .unwrap();

        assert_eq!(
            collect_subtitle_sidecars(dir.path(), &media).await.unwrap(),
            [subtitle]
        );
    }

    #[test]
    fn resolution_formats_use_a_bounded_selector_with_an_explicit_fallback() {
        for (quality, height) in [
            (Quality::P720, 720),
            (Quality::P1080, 1080),
            (Quality::P1440, 1440),
            (Quality::P2160, 2160),
        ] {
            let args = format_args(Format::Video, quality);
            let selector =
                format!("bestvideo*[height<={height}]+bestaudio/best[height<={height}]/best");
            assert!(args.contains(&OsString::from(selector)));
        }
        assert_eq!(parse_height_line("DLOOR_HEIGHT|1080"), Some(1080));
        assert_eq!(parse_height_line("DLOOR_HEIGHT|NA"), None);
    }

    #[test]
    fn bandwidth_limit_is_applied_only_to_download_arguments() {
        assert!(bandwidth_args(None).is_empty());
        let limit = "4.2M".parse::<BandwidthLimit>().unwrap();
        assert_eq!(
            bandwidth_args(Some(&limit)),
            [OsString::from("--limit-rate"), OsString::from("4.2M")]
        );
    }
}
