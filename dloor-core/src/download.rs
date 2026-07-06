use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::SystemTime,
};

use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tracing::debug;

use crate::{
    config::{Config, Destination},
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
}

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub request: DownloadRequest,
    pub config: Config,
}

impl DownloadJob {
    pub fn new(request: DownloadRequest, config: Config) -> Self {
        Self { request, config }
    }

    pub fn spawn(self) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            if let Err(error) = self.run_with_sender(&tx).await {
                let _ = tx
                    .send(DownloadEvent::Failed {
                        error: error.to_string(),
                    })
                    .await;
            }
        });
        rx
    }

    pub async fn run_with_sender(self, tx: &mpsc::Sender<DownloadEvent>) -> Result<()> {
        let platform = detect_platform(&self.request.url)?;
        let started_at = SystemTime::now();
        let (work_dir, _temp_guard) = self.prepare_work_dir().await?;
        let output_template = work_dir.join("%(title)s [%(id)s].%(ext)s");

        self.run_ytdlp(&output_template, platform, tx).await?;
        let downloaded = newest_file(&work_dir, started_at).await?;

        let final_local = if self.request.format == Format::Video
            && self.request.quality == Quality::Compressed
        {
            tx.send(DownloadEvent::Converting).await.ok();
            transcode_video(&downloaded).await?
        } else {
            downloaded
        };

        let completed_path = match &self.config.destination {
            Destination::Local { .. } => final_local.to_string_lossy().to_string(),
            Destination::Cloud { remote, path } => {
                tx.send(DownloadEvent::Uploading).await.ok();
                upload_to_cloud(&final_local, remote, path).await?;
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

    async fn prepare_work_dir(&self) -> Result<(PathBuf, Option<TempDir>)> {
        match &self.config.destination {
            Destination::Local { path } if self.request.quality != Quality::Compressed => {
                fs::create_dir_all(path).await?;
                Ok((path.clone(), None))
            }
            Destination::Local { path } => {
                fs::create_dir_all(path).await?;
                Ok((path.clone(), None))
            }
            Destination::Cloud { .. } => {
                let dir = tempfile::Builder::new().prefix("dloor-").tempdir()?;
                Ok((dir.path().to_path_buf(), Some(dir)))
            }
        }
    }

    async fn run_ytdlp(
        &self,
        output_template: &Path,
        platform: Platform,
        tx: &mpsc::Sender<DownloadEvent>,
    ) -> Result<()> {
        let mut args = base_ytdlp_args(output_template);
        args.extend(format_args(self.request.format, self.request.quality));
        args.push(OsString::from(&self.request.url));

        debug!(?args, "spawning yt-dlp");
        let mut child = Command::new("yt-dlp")
            .args(args)
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
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut collected = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push(line);
            }
            collected.join("\n")
        });

        while let Some(line) = stdout_lines.next_line().await? {
            if let Some(progress) = parse_progress_line(&line) {
                tx.send(DownloadEvent::Progress { progress, platform })
                    .await
                    .ok();
            }
        }

        let status = child.wait().await?;
        let stderr = stderr_task.await.unwrap_or_default();
        if !status.success() {
            return Err(Error::ProcessFailed(if stderr.trim().is_empty() {
                format!("yt-dlp exited with {status}")
            } else {
                stderr
            }));
        }
        Ok(())
    }
}

fn base_ytdlp_args(output_template: &Path) -> Vec<OsString> {
    vec![
        "--newline".into(),
        "--progress-template".into(),
        "%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s".into(),
        "-o".into(),
        output_template.as_os_str().to_os_string(),
    ]
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

pub fn parse_progress_line(line: &str) -> Option<DownloadProgress> {
    let mut parts = line.split('|').map(str::trim);
    let percent = parts
        .next()?
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?;
    let speed = parts.next().unwrap_or("").to_string();
    let eta = parts.next().unwrap_or("").to_string();

    Some(DownloadProgress {
        percent: percent.clamp(0.0, 100.0),
        speed,
        eta,
    })
}

async fn newest_file(dir: &Path, after: SystemTime) -> Result<PathBuf> {
    let mut entries = fs::read_dir(dir).await?;
    let mut newest: Option<(PathBuf, SystemTime)> = None;

    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(after);
        if modified < after {
            continue;
        }
        if newest.as_ref().is_none_or(|(_, old)| modified > *old) {
            newest = Some((entry.path(), modified));
        }
    }

    newest.map(|(path, _)| path).ok_or(Error::MissingOutputFile)
}

async fn transcode_video(input: &Path) -> Result<PathBuf> {
    let stem = input
        .file_stem()
        .ok_or_else(|| Error::InvalidPath(input.display().to_string()))?
        .to_string_lossy();
    let output = input.with_file_name(format!("{stem}.compressed.mp4"));
    let status = Command::new("ffmpeg")
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
        .status()
        .await?;

    if status.success() {
        Ok(output)
    } else {
        Err(Error::ProcessFailed(format!("ffmpeg exited with {status}")))
    }
}

async fn upload_to_cloud(file: &Path, remote: &str, remote_path: &str) -> Result<()> {
    let target = format!("{}:{}", remote, remote_path.trim_matches('/'));
    let status = Command::new("rclone")
        .args(["copy", file.to_string_lossy().as_ref(), target.as_str()])
        .status()
        .await?;
    if status.success() {
        fs::remove_file(file).await.ok();
        Ok(())
    } else {
        Err(Error::ProcessFailed(format!("rclone exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_template_output() {
        let progress = parse_progress_line(" 42.7%|1.24MiB/s|00:18").unwrap();
        assert_eq!(progress.percent, 42.7);
        assert_eq!(progress.speed, "1.24MiB/s");
        assert_eq!(progress.eta, "00:18");
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_progress_line("[download] Destination: file.mp4").is_none());
    }
}
