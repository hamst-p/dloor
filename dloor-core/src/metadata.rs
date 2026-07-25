use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

use crate::{config::sanitized_ytdlp_error, CookieSource, DownloadEvent, Error, Result};

pub const PREVIEW_ITEM_LIMIT: usize = 5;

#[derive(Debug, Clone)]
pub struct MetadataRequest {
    pub url: String,
    pub cookies: CookieSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataPreview {
    pub title: String,
    pub uploader: Option<String>,
    pub duration_seconds: Option<u64>,
    pub resolutions: Vec<String>,
    pub playlist: Option<PlaylistPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistPreview {
    pub total_items: Option<usize>,
    pub sample_titles: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct MetadataJob {
    request: MetadataRequest,
    cancellation: CancellationToken,
}

impl MetadataJob {
    pub fn new(request: MetadataRequest) -> Self {
        Self {
            request,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn spawn(self) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let event = match self.run().await {
                Ok(preview) => DownloadEvent::PreviewReady { preview },
                Err(Error::Cancelled) => DownloadEvent::PreviewCancelled,
                Err(error) => DownloadEvent::PreviewFailed {
                    error: error.to_string(),
                },
            };
            tx.send(event).await.ok();
        });
        rx
    }

    async fn run(self) -> Result<MetadataPreview> {
        let mut child = Command::new("yt-dlp")
            .args(metadata_args(&self.request.cookies))
            .arg(&self.request.url)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::ProcessFailed("failed to capture yt-dlp stderr".to_string()))?;
        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut collected = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push(line);
            }
            collected.join("\n")
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
        let stdout = stdout_task
            .await
            .map_err(|error| Error::ProcessFailed(error.to_string()))??;
        let stderr = stderr_task.await.unwrap_or_default();
        if !status.success() {
            let message = if stderr.trim().is_empty() {
                format!("yt-dlp metadata preview exited with {status}")
            } else {
                sanitized_ytdlp_error(&stderr, &self.request.cookies)
            };
            return Err(Error::ProcessFailed(message));
        }
        parse_metadata_json(&stdout)
    }
}

fn metadata_args(cookies: &CookieSource) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        "--dump-single-json".into(),
        "--flat-playlist".into(),
        "--playlist-end".into(),
        PREVIEW_ITEM_LIMIT.to_string().into(),
        "--no-download".into(),
        "--no-warnings".into(),
    ];
    args.extend(cookies.yt_dlp_args());
    args
}

#[derive(Debug, serde::Deserialize)]
struct RawMetadata {
    title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    duration: Option<f64>,
    #[serde(default)]
    formats: Vec<RawFormat>,
    entries: Option<Vec<Option<RawEntry>>>,
    playlist_count: Option<usize>,
    n_entries: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct RawFormat {
    height: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct RawEntry {
    title: Option<String>,
}

pub fn parse_metadata_json(raw: &[u8]) -> Result<MetadataPreview> {
    let raw: RawMetadata = serde_json::from_slice(raw)?;
    let title = raw
        .title
        .filter(|title| !title.trim().is_empty())
        .ok_or_else(|| Error::ProcessFailed("metadata did not contain a title".to_string()))?;
    let mut heights: Vec<_> = raw
        .formats
        .into_iter()
        .filter_map(|format| format.height)
        .collect();
    heights.sort_unstable_by(|left, right| right.cmp(left));
    heights.dedup();
    let resolutions = heights
        .into_iter()
        .map(|height| format!("{height}p"))
        .collect();
    let playlist = raw.entries.map(|entries| {
        let available_entries = entries.len();
        let sample_titles = entries
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.title)
            .filter(|title| !title.trim().is_empty())
            .take(PREVIEW_ITEM_LIMIT)
            .collect::<Vec<_>>();
        let total_items = raw.playlist_count.or(raw.n_entries);
        let truncated = total_items.is_some_and(|total| total > sample_titles.len())
            || (total_items.is_none() && available_entries >= PREVIEW_ITEM_LIMIT);
        PlaylistPreview {
            total_items,
            sample_titles,
            truncated,
        }
    });

    Ok(MetadataPreview {
        title,
        uploader: raw.uploader.or(raw.channel),
        duration_seconds: raw
            .duration
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .map(|duration| duration.round() as u64),
        resolutions,
        playlist,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_item_metadata() {
        let preview = parse_metadata_json(
            br#"{
                "title":"Example",
                "uploader":"Creator",
                "duration":125.4,
                "formats":[{"height":720},{"height":1080},{"height":720},{"height":null}]
            }"#,
        )
        .unwrap();

        assert_eq!(preview.title, "Example");
        assert_eq!(preview.uploader.as_deref(), Some("Creator"));
        assert_eq!(preview.duration_seconds, Some(125));
        assert_eq!(preview.resolutions, ["1080p", "720p"]);
        assert!(preview.playlist.is_none());
    }

    #[test]
    fn playlist_preview_is_limited_and_reports_total_count() {
        let preview = parse_metadata_json(
            br#"{
                "title":"Playlist",
                "playlist_count":20,
                "entries":[
                    {"title":"One"},{"title":"Two"},{"title":"Three"},
                    {"title":"Four"},{"title":"Five"},{"title":"Six"}
                ]
            }"#,
        )
        .unwrap();
        let playlist = preview.playlist.unwrap();

        assert_eq!(playlist.total_items, Some(20));
        assert_eq!(playlist.sample_titles.len(), PREVIEW_ITEM_LIMIT);
        assert!(playlist.truncated);
    }

    #[test]
    fn malformed_or_titleless_metadata_is_rejected() {
        assert!(parse_metadata_json(b"not json").is_err());
        assert!(parse_metadata_json(br#"{"duration":10}"#).is_err());
    }

    #[test]
    fn metadata_command_limits_playlist_expansion_and_applies_browser_authentication() {
        let args = metadata_args(&CookieSource::Browser {
            browser: crate::Browser::Firefox,
        });

        assert!(args.windows(2).any(|args| args == ["--playlist-end", "5"]));
        assert!(args
            .windows(2)
            .any(|args| args == ["--cookies-from-browser", "firefox"]));
    }
}
