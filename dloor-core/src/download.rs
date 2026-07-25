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
    config::{Browser, Config, Destination},
    detect_platform, Error, Platform, Result,
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
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best",
            Self::Compressed => "Compressed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    pub format: Format,
    pub quality: Quality,
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub percent: f64,
    pub speed: String,
    pub eta: String,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress {
        progress: DownloadProgress,
        platform: Platform,
    },
    Converting,
    Uploading,
    Completed {
        path: String,
    },
    Failed {
        error: String,
    },
    Cancelled,
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
                let event = if matches!(error, Error::Cancelled) {
                    debug!("download job cancelled");
                    DownloadEvent::Cancelled
                } else {
                    error!(%error, "download job failed");
                    DownloadEvent::Failed {
                        error: error.to_string(),
                    }
                };
                let _ = tx.send(event).await;
            }
        });
        rx
    }

    pub async fn run_with_sender(self, tx: &mpsc::Sender<DownloadEvent>) -> Result<()> {
        let platform = detect_platform(&self.request.url)?;
        let (work_dir, _temp_guard) = self.prepare_work_dir().await?;
        let output_template = work_dir.join("%(title)s [%(id)s].%(ext)s");

        let downloaded = self.run_ytdlp(&output_template, platform, tx).await?;

        let final_local = if self.request.format == Format::Video
            && self.request.quality == Quality::Compressed
        {
            tx.send(DownloadEvent::Converting).await.ok();
            transcode_video(&downloaded, &self.cancellation).await?
        } else {
            downloaded
        };

        let completed_path = match &self.config.destination {
            Destination::Local { path } => {
                fs::create_dir_all(path).await?;
                let destination = unique_destination(path, &final_local).await?;
                move_file(&final_local, &destination).await?;
                destination.to_string_lossy().to_string()
            }
            Destination::Cloud { remote, path } => {
                tx.send(DownloadEvent::Uploading).await.ok();
                upload_to_cloud(&final_local, remote, path, &self.cancellation).await?;
                let file_name = final_local
                    .file_name()
                    .ok_or_else(|| Error::InvalidPath(final_local.display().to_string()))?
                    .to_string_lossy();
                format!("{}:{}/{}", remote, path.trim_matches('/'), file_name)
            }
        };

        tx.send(DownloadEvent::Completed {
            path: completed_path,
        })
        .await
        .ok();

        Ok(())
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
        platform: Platform,
        tx: &mpsc::Sender<DownloadEvent>,
    ) -> Result<PathBuf> {
        let mut args = base_ytdlp_args(output_template, self.config.browser);
        args.extend(format_args(self.request.format, self.request.quality));
        args.push(OsString::from(&self.request.url));

        debug!(?args, "spawning yt-dlp");
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
                            }
                        }
                        None => stdout_open = false,
                    }
                }
                line = stderr_lines.next_line(), if stderr_open => {
                    match line? {
                        Some(line) => {
                            if let Some(progress) = progress_tracker.update(&line) {
                                tx.send(DownloadEvent::Progress { progress, platform })
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
        if !status.success() {
            return Err(Error::ProcessFailed(if stderr.trim().is_empty() {
                format!("yt-dlp exited with {status}")
            } else {
                stderr
            }));
        }
        let output_path = output_path.ok_or(Error::MissingOutputFile)?;
        if output_path.extension().is_some_and(|ext| ext == "part")
            || !fs::try_exists(&output_path).await?
        {
            return Err(Error::MissingOutputFile);
        }
        Ok(output_path)
    }
}

const OUTPUT_PREFIX: &str = "DLOOR_OUTPUT|";
const PLAN_PREFIX: &str = "DLOOR_PLAN|";
const PROGRESS_PREFIX: &str = "DLOOR_PROGRESS|";

fn base_ytdlp_args(output_template: &Path, browser: Option<Browser>) -> Vec<OsString> {
    let mut args = vec![
        "--newline".into(),
        "--progress".into(),
        "--progress-template".into(),
        concat!(
            "download:DLOOR_PROGRESS|%(progress._percent_str)s|",
            "%(progress.downloaded_bytes)s|",
            "%(progress.total_bytes,progress.total_bytes_estimate)s|",
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
        "-o".into(),
        output_template.as_os_str().to_os_string(),
    ];
    if let Some(browser) = browser {
        args.push("--cookies-from-browser".into());
        args.push(browser.yt_dlp_name().into());
    }
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
    }
}

#[derive(Debug)]
struct ParsedProgress {
    percent: f64,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    playlist_index: Option<u64>,
    n_entries: Option<u64>,
    format_id: String,
    speed: String,
    eta: String,
}

#[derive(Debug, Default)]
struct ProgressPlan {
    n_entries: Option<u64>,
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

        let part_percent = match (parsed.downloaded_bytes, parsed.total_bytes) {
            (Some(downloaded), Some(total)) if total > 0 => {
                downloaded as f64 / total as f64 * 100.0
            }
            _ => parsed.percent,
        }
        .clamp(0.0, 100.0);

        let (item_fraction, planned_entries) = self.plans.get_mut(&playlist_index).map_or_else(
            || {
                let completed_parts = self
                    .completed_formats
                    .iter()
                    .filter(|(item, _)| *item == playlist_index)
                    .count()
                    .min(self.fallback_part_count - 1);
                (
                    (completed_parts as f64 + part_percent / 100.0)
                        / self.fallback_part_count as f64,
                    None,
                )
            },
            |plan| {
                if let Some(total) = parsed.total_bytes.filter(|total| *total > 0) {
                    plan.sizes.insert(parsed.format_id.clone(), total);
                }
                if let Some(downloaded) = parsed.downloaded_bytes {
                    plan.downloaded.insert(parsed.format_id.clone(), downloaded);
                }
                let total_bytes: u64 = plan.sizes.values().sum();
                let downloaded_bytes: u64 = plan
                    .sizes
                    .iter()
                    .map(|(format_id, total)| {
                        plan.downloaded
                            .get(format_id)
                            .copied()
                            .unwrap_or(0)
                            .min(*total)
                    })
                    .sum();
                let fraction = if total_bytes > 0 {
                    downloaded_bytes as f64 / total_bytes as f64
                } else {
                    part_percent / 100.0
                };
                (fraction, plan.n_entries)
            },
        );
        let n_entries = parsed.n_entries.or(planned_entries);
        let overall = n_entries
            .filter(|total| *total > 0)
            .map_or(item_fraction * 100.0, |total| {
                ((playlist_index.saturating_sub(1) as f64 + item_fraction) / total as f64) * 100.0
            });
        self.max_percent = self.max_percent.max(overall.clamp(0.0, 100.0));

        Some(DownloadProgress {
            percent: self.max_percent,
            speed: parsed.speed,
            eta: parsed.eta,
        })
    }
}

fn parse_progress_fields(line: &str) -> Option<ParsedProgress> {
    let line = line.strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = line.splitn(8, '|').map(str::trim);
    let percent = parts
        .next()?
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?;
    let downloaded_bytes = parse_optional_u64(parts.next()?);
    let total_bytes = parse_optional_u64(parts.next()?);
    let playlist_index = parse_optional_u64(parts.next()?);
    let n_entries = parse_optional_u64(parts.next()?);
    let format_id = parts.next()?.to_string();

    Some(ParsedProgress {
        percent,
        downloaded_bytes,
        total_bytes,
        playlist_index,
        n_entries,
        format_id,
        speed: parts.next().unwrap_or("").to_string(),
        eta: parts.next().unwrap_or("").to_string(),
    })
}

fn parse_plan_line(line: &str) -> Option<(u64, ProgressPlan)> {
    let line = line.strip_prefix(PLAN_PREFIX)?;
    let mut parts = line.splitn(8, '|').map(str::trim);
    let playlist_index = parse_optional_u64(parts.next()?).unwrap_or(1).max(1);
    let n_entries = parse_optional_u64(parts.next()?);
    let format_id = parts.next()?;
    let format_size = parse_optional_u64(parts.next()?);
    let requested = [
        (parts.next()?, parse_optional_u64(parts.next()?)),
        (parts.next()?, parse_optional_u64(parts.next()?)),
    ];
    let mut sizes = HashMap::new();
    for (id, size) in requested {
        if id != "NA" && !id.is_empty() {
            if let Some(size) = size {
                sizes.insert(id.to_string(), size);
            }
        }
    }
    if sizes.is_empty() {
        if let Some(size) = format_size {
            sizes.insert(format_id.to_string(), size);
        }
    }

    Some((
        playlist_index,
        ProgressPlan {
            n_entries,
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
            output.to_string_lossy().as_ref(),
        ])
        .kill_on_drop(true)
        .spawn()?;
    let status = wait_for_process(&mut child, cancellation).await?;

    if status.success() {
        fs::remove_file(input).await?;
        Ok(output)
    } else {
        Err(Error::ProcessFailed(format!("ffmpeg exited with {status}")))
    }
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
            parse_progress_line("DLOOR_PROGRESS|42.7%|427|1000|NA|NA|18|1.24MiB/s|00:18").unwrap();
        assert!((progress.percent - 42.7).abs() < 1e-9);
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
            .update("DLOOR_PROGRESS|100%|1000|1000|NA|NA|137|1MiB/s|00:00")
            .unwrap();
        let audio_start = tracker
            .update("DLOOR_PROGRESS|0%|0|100|NA|NA|140|1MiB/s|00:01")
            .unwrap();
        let audio_half = tracker
            .update("DLOOR_PROGRESS|50%|50|100|NA|NA|140|1MiB/s|00:01")
            .unwrap();

        assert!((video.percent - 90.909).abs() < 0.001);
        assert_eq!(audio_start.percent, video.percent);
        assert!((audio_half.percent - 95.454).abs() < 0.001);
    }

    #[test]
    fn combines_playlist_and_format_progress() {
        let mut tracker = ProgressTracker::new(2);
        tracker.update("DLOOR_PLAN|2|4|137+140|200|137|100|140|100");
        let progress = tracker
            .update("DLOOR_PROGRESS|50%|50|100|2|4|137|1MiB/s|00:01")
            .unwrap();

        assert_eq!(progress.percent, 31.25);
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
        let without_auth = base_ytdlp_args(output, None);
        assert!(!without_auth.contains(&OsString::from("--cookies-from-browser")));
        assert!(without_auth
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("before_dl:DLOOR_PLAN|")));
        assert!(without_auth
            .iter()
            .any(|arg| arg == "after_move:DLOOR_OUTPUT|%(filepath)s"));

        let with_auth = base_ytdlp_args(output, Some(Browser::Firefox));
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
}
