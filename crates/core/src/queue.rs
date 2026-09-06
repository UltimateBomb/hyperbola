//! The download queue: a pure state machine.
//!
//! Nothing here spawns a process. The shell asks [`Queue::start_next`] what to
//! run, runs it, and feeds results back in. That split is what lets the same
//! queue drive a desktop window and an Android foreground service, and what
//! makes every rule below testable without touching the network.

use serde::{Deserialize, Serialize};

use crate::domain::{Download, DownloadId, DownloadOptions, DownloadState, Progress, Stage};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What happened to a download after a failure was reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOutcome {
    /// Put back in the queue for another attempt.
    Requeued,
    /// Out of attempts, or the error is not worth retrying.
    GaveUp,
}

/// Aggregate numbers for the header bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QueueStats {
    pub queued: usize,
    pub running: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
    /// Combined speed of everything currently running, bytes per second.
    pub speed_bps: f64,
}

#[derive(Debug, Clone)]
pub struct Queue {
    items: Vec<Download>,
    next_id: u64,
    max_concurrent: usize,
    max_attempts: u32,
}

impl Queue {
    /// `max_concurrent` is how many downloads may run at once; it is clamped
    /// to at least one.
    pub fn new(max_concurrent: usize) -> Self {
        Queue {
            items: Vec::new(),
            next_id: 1,
            max_concurrent: max_concurrent.max(1),
            max_attempts: 3,
        }
    }

    pub fn set_max_concurrent(&mut self, max_concurrent: usize) {
        self.max_concurrent = max_concurrent.max(1);
    }

    /// How many times a retryable failure is retried automatically.
    pub fn set_max_attempts(&mut self, max_attempts: u32) {
        self.max_attempts = max_attempts;
    }

    /// Rebuilds a queue from items that were persisted between runs.
    ///
    /// A download that was running when the app closed comes back paused: the
    /// process behind it is gone, and showing it as running would be a lie
    /// the user cannot act on. Its partial file is still on disk, so resuming
    /// continues rather than restarts.
    pub fn restore(&mut self, items: Vec<Download>) {
        self.next_id = items.iter().map(|d| d.id.0).max().unwrap_or(0) + 1;
        self.items = items
            .into_iter()
            .map(|mut download| {
                if let DownloadState::Running(progress) = download.state {
                    download.state = DownloadState::Paused(progress);
                }
                download
            })
            .collect();
    }

    pub fn items(&self) -> &[Download] {
        &self.items
    }

    pub fn get(&self, id: DownloadId) -> Option<&Download> {
        self.items.iter().find(|d| d.id == id)
    }

    /// Mutable access for the shell to attach what only it knows — the
    /// system's own handle for a finished file, for instance.
    pub fn get_mut(&mut self, id: DownloadId) -> Option<&mut Download> {
        self.items.iter_mut().find(|d| d.id == id)
    }

    /// Appends a download and returns its id.
    pub fn add(&mut self, options: DownloadOptions, title: impl Into<String>) -> DownloadId {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        self.items.push(Download {
            id,
            options,
            title: title.into(),
            thumbnail: None,
            state: DownloadState::Queued,
            attempts: 0,
            metadata: BTreeMap::new(),
        });
        id
    }

    pub fn running_count(&self) -> usize {
        self.items.iter().filter(|d| d.state.is_active()).count()
    }

    /// The next download to run, or `None` when the queue is empty or the
    /// concurrency limit is reached. Marks the returned download as running.
    pub fn start_next(&mut self) -> Option<DownloadId> {
        if self.running_count() >= self.max_concurrent {
            return None;
        }
        let id = self
            .items
            .iter()
            .find(|d| matches!(d.state, DownloadState::Queued))
            .map(|d| d.id)?;
        if let Some(download) = self.get_mut(id) {
            download.attempts += 1;
            download.state = DownloadState::Running(Progress {
                stage: Stage::Probing,
                ..Progress::default()
            });
        }
        Some(id)
    }

    /// Records a progress sample. Ignored for downloads that are not running,
    /// so a late sample from a canceled process cannot revive it.
    pub fn on_progress(&mut self, id: DownloadId, progress: Progress) {
        if let Some(download) = self.get_mut(id) {
            if download.state.is_active() {
                download.state = DownloadState::Running(progress);
            }
        }
    }

    pub fn on_completed(&mut self, id: DownloadId, path: impl Into<PathBuf>) {
        if let Some(download) = self.get_mut(id) {
            download.state = DownloadState::Completed { path: path.into() };
        }
    }

    /// Reports a failure and applies the retry policy.
    pub fn on_failed(
        &mut self,
        id: DownloadId,
        message: impl Into<String>,
        retryable: bool,
    ) -> FailureOutcome {
        let max_attempts = self.max_attempts;
        let Some(download) = self.get_mut(id) else {
            return FailureOutcome::GaveUp;
        };
        // A canceled download that reports a failure on its way out stays canceled.
        if matches!(download.state, DownloadState::Canceled) {
            return FailureOutcome::GaveUp;
        }
        if retryable && download.attempts < max_attempts {
            download.state = DownloadState::Queued;
            FailureOutcome::Requeued
        } else {
            download.state = DownloadState::Failed {
                message: message.into(),
                retryable,
            };
            FailureOutcome::GaveUp
        }
    }

    /// Stops a running download but keeps its partial file and progress.
    pub fn pause(&mut self, id: DownloadId) {
        if let Some(download) = self.get_mut(id) {
            match &download.state {
                DownloadState::Running(progress) => {
                    download.state = DownloadState::Paused(*progress);
                }
                DownloadState::Queued => {
                    download.state = DownloadState::Paused(Progress::default());
                }
                _ => {}
            }
        }
    }

    /// Puts a paused download back in line. yt-dlp continues from the `.part`
    /// file, so the attempt counter is not spent on a manual resume.
    pub fn resume(&mut self, id: DownloadId) {
        if let Some(download) = self.get_mut(id) {
            if matches!(download.state, DownloadState::Paused(_)) {
                download.state = DownloadState::Queued;
            }
        }
    }

    pub fn cancel(&mut self, id: DownloadId) {
        if let Some(download) = self.get_mut(id) {
            if !matches!(download.state, DownloadState::Completed { .. }) {
                download.state = DownloadState::Canceled;
            }
        }
    }

    /// Manual retry of a finished-but-failed download; resets the attempts so
    /// the automatic policy applies again.
    pub fn retry(&mut self, id: DownloadId) {
        if let Some(download) = self.get_mut(id) {
            if download.state.is_terminal()
                && !matches!(download.state, DownloadState::Completed { .. })
            {
                download.attempts = 0;
                download.state = DownloadState::Queued;
            }
        }
    }

    pub fn remove(&mut self, id: DownloadId) {
        self.items.retain(|d| d.id != id);
    }

    /// Drops completed and canceled entries, keeping failures visible.
    pub fn clear_finished(&mut self) {
        self.items.retain(|d| {
            !matches!(
                d.state,
                DownloadState::Completed { .. } | DownloadState::Canceled
            )
        });
    }

    pub fn stats(&self) -> QueueStats {
        let mut stats = QueueStats {
            queued: 0,
            running: 0,
            paused: 0,
            completed: 0,
            failed: 0,
            speed_bps: 0.0,
        };
        for download in &self.items {
            match &download.state {
                DownloadState::Queued => stats.queued += 1,
                DownloadState::Running(progress) => {
                    stats.running += 1;
                    stats.speed_bps += progress.speed_bps.unwrap_or(0.0);
                }
                DownloadState::Paused(_) => stats.paused += 1,
                DownloadState::Completed { .. } => stats.completed += 1,
                DownloadState::Failed { .. } => stats.failed += 1,
                DownloadState::Canceled => {}
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_with(count: usize, max_concurrent: usize) -> Queue {
        let mut queue = Queue::new(max_concurrent);
        for i in 0..count {
            queue.add(
                DownloadOptions::video(format!("https://x/{i}"), "/out"),
                format!("clip {i}"),
            );
        }
        queue
    }

    fn progress(bytes: u64, speed: f64) -> Progress {
        Progress {
            stage: Stage::Downloading,
            downloaded_bytes: bytes,
            total_bytes: Some(1000),
            speed_bps: Some(speed),
            eta_secs: Some(10),
        }
    }

    #[test]
    fn starts_downloads_in_order_up_to_the_limit() {
        let mut queue = queue_with(3, 2);
        assert_eq!(queue.start_next(), Some(DownloadId(1)));
        assert_eq!(queue.start_next(), Some(DownloadId(2)));
        assert_eq!(queue.start_next(), None, "third start exceeds the limit");
        assert_eq!(queue.running_count(), 2);
    }

    #[test]
    fn raising_the_limit_frees_the_next_slot() {
        let mut queue = queue_with(3, 1);
        queue.start_next();
        assert_eq!(queue.start_next(), None);
        queue.set_max_concurrent(3);
        assert_eq!(queue.start_next(), Some(DownloadId(2)));
    }

    #[test]
    fn completing_one_lets_the_next_start() {
        let mut queue = queue_with(2, 1);
        let first = queue.start_next().unwrap();
        queue.on_completed(first, "/out/clip.mp4");
        assert_eq!(queue.start_next(), Some(DownloadId(2)));
        assert_eq!(queue.stats().completed, 1);
    }

    #[test]
    fn progress_only_lands_on_running_downloads() {
        let mut queue = queue_with(1, 1);
        let id = queue.start_next().unwrap();
        queue.on_progress(id, progress(500, 1000.0));
        match &queue.get(id).unwrap().state {
            DownloadState::Running(p) => assert_eq!(p.fraction(), Some(0.5)),
            other => panic!("expected running, got {other:?}"),
        }

        queue.cancel(id);
        queue.on_progress(id, progress(900, 1000.0));
        assert_eq!(queue.get(id).unwrap().state, DownloadState::Canceled);
    }

    #[test]
    fn retryable_failures_go_back_in_line_until_attempts_run_out() {
        let mut queue = queue_with(1, 1);
        queue.set_max_attempts(2);

        let id = queue.start_next().unwrap();
        assert_eq!(
            queue.on_failed(id, "connection reset", true),
            FailureOutcome::Requeued
        );
        assert_eq!(queue.get(id).unwrap().state, DownloadState::Queued);

        // Second attempt.
        assert_eq!(queue.start_next(), Some(id));
        assert_eq!(
            queue.on_failed(id, "connection reset", true),
            FailureOutcome::GaveUp
        );
        match &queue.get(id).unwrap().state {
            DownloadState::Failed { message, retryable } => {
                assert_eq!(message, "connection reset");
                assert!(retryable);
            }
            other => panic!("expected failure, got {other:?}"),
        }
        assert_eq!(queue.get(id).unwrap().attempts, 2);
    }

    #[test]
    fn permanent_failures_are_not_retried() {
        let mut queue = queue_with(1, 1);
        let id = queue.start_next().unwrap();
        assert_eq!(
            queue.on_failed(id, "Video unavailable", false),
            FailureOutcome::GaveUp
        );
        assert_eq!(queue.stats().failed, 1);
    }

    #[test]
    fn a_cancelled_download_stays_cancelled_when_its_process_reports_an_error() {
        let mut queue = queue_with(1, 1);
        let id = queue.start_next().unwrap();
        queue.cancel(id);
        assert_eq!(queue.on_failed(id, "killed", true), FailureOutcome::GaveUp);
        assert_eq!(queue.get(id).unwrap().state, DownloadState::Canceled);
    }

    #[test]
    fn pause_keeps_progress_and_resume_does_not_spend_an_attempt() {
        let mut queue = queue_with(1, 1);
        let id = queue.start_next().unwrap();
        queue.on_progress(id, progress(700, 500.0));
        queue.pause(id);
        match &queue.get(id).unwrap().state {
            DownloadState::Paused(p) => assert_eq!(p.downloaded_bytes, 700),
            other => panic!("expected paused, got {other:?}"),
        }
        assert_eq!(queue.stats().paused, 1);

        queue.resume(id);
        assert_eq!(queue.get(id).unwrap().state, DownloadState::Queued);
        assert_eq!(
            queue.get(id).unwrap().attempts,
            1,
            "resume reuses the attempt"
        );
    }

    #[test]
    fn manual_retry_resets_the_attempt_counter() {
        let mut queue = queue_with(1, 1);
        queue.set_max_attempts(1);
        let id = queue.start_next().unwrap();
        queue.on_failed(id, "boom", true);
        queue.retry(id);
        assert_eq!(queue.get(id).unwrap().attempts, 0);
        assert_eq!(queue.get(id).unwrap().state, DownloadState::Queued);
    }

    #[test]
    fn completed_downloads_cannot_be_cancelled_or_retried() {
        let mut queue = queue_with(1, 1);
        let id = queue.start_next().unwrap();
        queue.on_completed(id, "/out/clip.mp4");
        queue.cancel(id);
        queue.retry(id);
        assert!(matches!(
            queue.get(id).unwrap().state,
            DownloadState::Completed { .. }
        ));
    }

    #[test]
    fn clearing_finished_keeps_failures_visible() {
        let mut queue = queue_with(3, 3);
        let a = queue.start_next().unwrap();
        let b = queue.start_next().unwrap();
        let c = queue.start_next().unwrap();
        queue.on_completed(a, "/out/a.mp4");
        queue.cancel(b);
        queue.on_failed(c, "Video unavailable", false);

        queue.clear_finished();
        assert_eq!(queue.items().len(), 1);
        assert_eq!(queue.items()[0].id, c);
    }

    #[test]
    fn restoring_turns_running_downloads_back_into_paused_ones() {
        let mut original = queue_with(3, 3);
        let running = original.start_next().unwrap();
        original.on_progress(running, progress(400, 900.0));
        let completed = original.start_next().unwrap();
        original.on_completed(completed, "/out/b.mp4");

        let saved = original.items().to_vec();
        let mut restored = Queue::new(3);
        restored.restore(saved);

        match &restored.get(running).unwrap().state {
            DownloadState::Paused(p) => assert_eq!(p.downloaded_bytes, 400),
            other => panic!("expected paused, got {other:?}"),
        }
        assert!(matches!(
            restored.get(completed).unwrap().state,
            DownloadState::Completed { .. }
        ));
        // New downloads must not reuse an id that came back from disk.
        let fresh = restored.add(DownloadOptions::video("https://x/new", "/out"), "new");
        assert_eq!(fresh, DownloadId(4));
    }

    #[test]
    fn stats_sum_the_speed_of_running_downloads() {
        let mut queue = queue_with(3, 3);
        let a = queue.start_next().unwrap();
        let b = queue.start_next().unwrap();
        queue.on_progress(a, progress(100, 1_500_000.0));
        queue.on_progress(b, progress(200, 500_000.0));
        let stats = queue.stats();
        assert_eq!(stats.running, 2);
        assert_eq!(stats.queued, 1);
        assert_eq!(stats.speed_bps, 2_000_000.0);
    }
}
