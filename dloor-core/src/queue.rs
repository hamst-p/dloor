use std::collections::VecDeque;

use crate::{DownloadEvent, DownloadProgress, DownloadRequest, DownloadSummary, Platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    Pending,
    Running,
    Succeeded,
    PartiallySucceeded,
    Failed,
    Cancelled,
}

impl QueueStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::PartiallySucceeded | Self::Failed | Self::Cancelled
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Waiting",
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::PartiallySucceeded => "Partial",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub id: JobId,
    pub request: DownloadRequest,
    pub title: String,
    pub platform: Platform,
    pub status: QueueStatus,
    pub progress: Option<DownloadProgress>,
    pub summary: Option<DownloadSummary>,
}

#[derive(Debug, Default)]
pub struct DownloadQueue {
    entries: VecDeque<QueuedJob>,
    active: Option<JobId>,
    next_id: u64,
}

impl DownloadQueue {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    pub fn enqueue(
        &mut self,
        request: DownloadRequest,
        title: String,
        platform: Platform,
    ) -> JobId {
        let id = JobId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.entries.push_back(QueuedJob {
            id,
            request,
            title,
            platform,
            status: QueueStatus::Pending,
            progress: None,
            summary: None,
        });
        id
    }

    pub fn start_next(&mut self) -> Option<QueuedJob> {
        if self.active.is_some() {
            return None;
        }
        let job = self
            .entries
            .iter_mut()
            .find(|job| job.status == QueueStatus::Pending)?;
        job.status = QueueStatus::Running;
        self.active = Some(job.id);
        Some(job.clone())
    }

    pub fn apply_event(&mut self, id: JobId, event: &DownloadEvent) -> bool {
        let Some(job) = self.entries.iter_mut().find(|job| job.id == id) else {
            return false;
        };
        if job.status != QueueStatus::Running {
            return false;
        }

        match event {
            DownloadEvent::ItemStarted { item, .. } => job.title = item.title.clone(),
            DownloadEvent::Progress { progress, item, .. } => {
                job.title = item.title.clone();
                job.progress = Some(progress.clone());
            }
            DownloadEvent::Finished { summary } => {
                job.status = status_for_summary(summary);
                job.summary = Some(summary.clone());
                self.active = None;
                return true;
            }
            DownloadEvent::Failed { .. } => {
                job.status = QueueStatus::Failed;
                self.active = None;
                return true;
            }
            DownloadEvent::Cancelled { summary } => {
                job.status = QueueStatus::Cancelled;
                job.summary = Some(summary.clone());
                self.active = None;
                return true;
            }
            DownloadEvent::DependenciesChecked { .. }
            | DownloadEvent::YtDlpUpdateFinished { .. }
            | DownloadEvent::PreviewReady { .. }
            | DownloadEvent::PreviewFailed { .. }
            | DownloadEvent::PreviewCancelled
            | DownloadEvent::Resolving
            | DownloadEvent::Converting { .. }
            | DownloadEvent::Uploading { .. }
            | DownloadEvent::ItemCompleted { .. }
            | DownloadEvent::ItemFailed { .. }
            | DownloadEvent::ItemWarning { .. } => {}
        }
        false
    }

    pub fn cancel_pending(&mut self, id: JobId) -> Option<QueuedJob> {
        let job = self
            .entries
            .iter_mut()
            .find(|job| job.id == id && job.status == QueueStatus::Pending)?;
        job.status = QueueStatus::Cancelled;
        Some(job.clone())
    }

    pub fn remove_pending(&mut self, id: JobId) -> Option<QueuedJob> {
        let position = self
            .entries
            .iter()
            .position(|job| job.id == id && job.status == QueueStatus::Pending)?;
        self.entries.remove(position)
    }

    pub fn remove_terminal(&mut self, id: JobId) -> Option<QueuedJob> {
        let position = self
            .entries
            .iter()
            .position(|job| job.id == id && job.status.is_terminal())?;
        self.entries.remove(position)
    }

    pub fn move_pending(&mut self, id: JobId, forward: bool) -> bool {
        let Some(position) = self
            .entries
            .iter()
            .position(|job| job.id == id && job.status == QueueStatus::Pending)
        else {
            return false;
        };
        let target = if forward {
            position.checked_add(1)
        } else {
            position.checked_sub(1)
        };
        let Some(target) = target.filter(|target| *target < self.entries.len()) else {
            return false;
        };
        if self.entries[target].status != QueueStatus::Pending {
            return false;
        }
        self.entries.swap(position, target);
        true
    }

    pub fn entry(&self, id: JobId) -> Option<&QueuedJob> {
        self.entries.iter().find(|job| job.id == id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &QueuedJob> {
        self.entries.iter()
    }

    pub fn active_id(&self) -> Option<JobId> {
        self.active
    }

    pub fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|job| job.status == QueueStatus::Pending)
            .count()
    }

    pub fn has_unfinished(&self) -> bool {
        self.entries
            .iter()
            .any(|job| matches!(job.status, QueueStatus::Pending | QueueStatus::Running))
    }
}

fn status_for_summary(summary: &DownloadSummary) -> QueueStatus {
    match (summary.succeeded.is_empty(), summary.failed.is_empty()) {
        (false, true) => QueueStatus::Succeeded,
        (false, false) => QueueStatus::PartiallySucceeded,
        (true, false) | (true, true) => QueueStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, PlaylistSelection, Quality};

    fn request(url: &str) -> DownloadRequest {
        DownloadRequest {
            url: url.to_string(),
            format: Format::Video,
            quality: Quality::Best,
            playlist: PlaylistSelection::Single,
        }
    }

    #[test]
    fn queue_runs_one_job_at_a_time() {
        let mut queue = DownloadQueue::new();
        let first = queue.enqueue(
            request("https://youtube.com/watch?v=one"),
            "one".to_string(),
            Platform::YouTube,
        );
        let second = queue.enqueue(
            request("https://youtube.com/watch?v=two"),
            "two".to_string(),
            Platform::YouTube,
        );

        assert_eq!(queue.start_next().unwrap().id, first);
        assert!(queue.start_next().is_none());
        assert!(queue.apply_event(
            first,
            &DownloadEvent::Finished {
                summary: DownloadSummary {
                    total: 1,
                    succeeded: Vec::new(),
                    failed: vec![crate::DownloadFailure {
                        item: crate::DownloadItem {
                            index: 1,
                            total: 1,
                            title: "one".to_string(),
                            playlist_index: None,
                        },
                        error: "failed".to_string(),
                    }],
                    warnings: Vec::new(),
                },
            }
        ));
        assert_eq!(queue.start_next().unwrap().id, second);
    }

    #[test]
    fn pending_jobs_can_be_reordered_removed_and_cancelled() {
        let mut queue = DownloadQueue::new();
        let first = queue.enqueue(
            request("https://youtube.com/watch?v=one"),
            "one".to_string(),
            Platform::YouTube,
        );
        let second = queue.enqueue(
            request("https://youtube.com/watch?v=two"),
            "two".to_string(),
            Platform::YouTube,
        );

        assert!(queue.move_pending(second, false));
        assert_eq!(queue.entries().next().unwrap().id, second);
        assert_eq!(queue.remove_pending(first).unwrap().id, first);
        assert_eq!(
            queue.cancel_pending(second).unwrap().status,
            QueueStatus::Cancelled
        );
        assert!(!queue.has_unfinished());
    }

    #[test]
    fn terminal_summary_maps_to_partial_status() {
        let summary = DownloadSummary {
            total: 2,
            succeeded: vec![crate::DownloadSuccess {
                item: crate::DownloadItem {
                    index: 1,
                    total: 2,
                    title: "ok".to_string(),
                    playlist_index: Some(1),
                },
                path: "/tmp/ok.mp4".to_string(),
            }],
            failed: vec![crate::DownloadFailure {
                item: crate::DownloadItem {
                    index: 2,
                    total: 2,
                    title: "bad".to_string(),
                    playlist_index: Some(2),
                },
                error: "failed".to_string(),
            }],
            warnings: Vec::new(),
        };

        assert_eq!(
            status_for_summary(&summary),
            QueueStatus::PartiallySucceeded
        );
    }
}
