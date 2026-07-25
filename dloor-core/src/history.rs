use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    DownloadFailure, DownloadRequest, DownloadSuccess, Format, Platform, PlaylistSelection,
    Quality, Result,
};

pub const DEFAULT_HISTORY_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Succeeded,
    Failed,
    Cancelled,
}

impl HistoryStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub source_url: String,
    pub playlist_index: Option<usize>,
    pub title: String,
    pub platform: Platform,
    pub format: Format,
    pub quality: Quality,
    pub playlist: PlaylistSelection,
    pub destination_path: Option<String>,
    pub status: HistoryStatus,
    pub recorded_at: String,
}

impl HistoryEntry {
    pub fn succeeded(
        request: &DownloadRequest,
        platform: Platform,
        result: &DownloadSuccess,
    ) -> Self {
        Self::new(
            request,
            platform,
            result.item.title.clone(),
            result.item.playlist_index,
            Some(result.path.clone()),
            HistoryStatus::Succeeded,
        )
    }

    pub fn failed(
        request: &DownloadRequest,
        platform: Platform,
        failure: &DownloadFailure,
    ) -> Self {
        Self::new(
            request,
            platform,
            failure.item.title.clone(),
            failure.item.playlist_index,
            None,
            HistoryStatus::Failed,
        )
    }

    pub fn unfinished(
        request: &DownloadRequest,
        platform: Platform,
        title: String,
        status: HistoryStatus,
    ) -> Self {
        let playlist_index = match request.playlist {
            PlaylistSelection::Item { index } => Some(index),
            PlaylistSelection::Single | PlaylistSelection::All => None,
        };
        Self::new(request, platform, title, playlist_index, None, status)
    }

    pub fn retry_request(&self) -> Option<DownloadRequest> {
        if self.status == HistoryStatus::Succeeded {
            return None;
        }
        Some(DownloadRequest {
            url: self.source_url.clone(),
            format: self.format,
            quality: self.quality,
            playlist: self.playlist,
        })
    }

    fn new(
        request: &DownloadRequest,
        platform: Platform,
        title: String,
        playlist_index: Option<usize>,
        destination_path: Option<String>,
        status: HistoryStatus,
    ) -> Self {
        Self {
            source_url: request.url.clone(),
            playlist_index,
            title,
            platform,
            format: request.format,
            quality: request.quality,
            playlist: playlist_index
                .map_or(request.playlist, |index| PlaylistSelection::Item { index }),
            destination_path,
            status,
            recorded_at: current_timestamp(),
        }
    }
}

#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    limit: usize,
    entries: Vec<HistoryEntry>,
}

impl HistoryStore {
    pub fn open(path: impl Into<PathBuf>, limit: usize) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            open_private_append(&path)?;
        }
        let entries = load_entries(&path, limit)?;
        Ok(Self {
            path,
            limit: limit.max(1),
            entries,
        })
    }

    pub fn append(&mut self, entry: HistoryEntry) -> Result<()> {
        let mut file = open_private_append(&self.path)?;
        serde_json::to_writer(&mut file, &entry)?;
        file.write_all(b"\n")?;
        file.flush()?;
        self.entries.push(entry);
        if self.entries.len() > self.limit {
            let remove_count = self.entries.len() - self.limit;
            self.entries.drain(..remove_count);
            self.compact()?;
        }
        Ok(())
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    fn compact(&self) -> Result<()> {
        let temporary = self.path.with_extension("jsonl.tmp");
        {
            let file = create_private_file(&temporary)?;
            let mut writer = BufWriter::new(file);
            for entry in &self.entries {
                serde_json::to_writer(&mut writer, entry)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

fn load_entries(path: &Path, limit: usize) -> Result<Vec<HistoryEntry>> {
    let reader = BufReader::new(File::open(path)?);
    let mut entries: Vec<_> = reader
        .lines()
        .map_while(std::result::Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();
    let limit = limit.max(1);
    if entries.len() > limit {
        let remove_count = entries.len() - limit;
        entries.drain(..remove_count);
    }
    Ok(entries)
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc().unix_timestamp().to_string())
}

fn open_private_append(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn create_private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize, status: HistoryStatus) -> HistoryEntry {
        HistoryEntry {
            source_url: format!("https://youtube.com/watch?v={index}"),
            playlist_index: Some(index),
            title: format!("Item {index}"),
            platform: Platform::YouTube,
            format: Format::Video,
            quality: Quality::Best,
            playlist: PlaylistSelection::Item { index },
            destination_path: None,
            status,
            recorded_at: "2026-07-26T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn history_round_trips_and_ignores_a_malformed_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut store = HistoryStore::open(&path, 10).unwrap();
        store.append(entry(1, HistoryStatus::Succeeded)).unwrap();
        let mut raw = OpenOptions::new().append(true).open(&path).unwrap();
        raw.write_all(b"{\"unfinished\":\n").unwrap();

        let restored = HistoryStore::open(path, 10).unwrap();

        assert_eq!(restored.entries(), [entry(1, HistoryStatus::Succeeded)]);
    }

    #[test]
    fn history_compacts_to_the_configured_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut store = HistoryStore::open(&path, 2).unwrap();
        store.append(entry(1, HistoryStatus::Failed)).unwrap();
        store.append(entry(2, HistoryStatus::Failed)).unwrap();
        store.append(entry(3, HistoryStatus::Failed)).unwrap();

        let restored = HistoryStore::open(path, 2).unwrap();

        assert_eq!(restored.entries().len(), 2);
        assert_eq!(restored.entries()[0].playlist_index, Some(2));
        assert_eq!(restored.entries()[1].playlist_index, Some(3));
    }

    #[test]
    fn failed_playlist_item_retries_only_that_item() {
        let history = entry(7, HistoryStatus::Failed);

        assert!(matches!(
            history.retry_request().unwrap().playlist,
            PlaylistSelection::Item { index: 7 }
        ));
        assert!(entry(1, HistoryStatus::Succeeded).retry_request().is_none());
    }
}
