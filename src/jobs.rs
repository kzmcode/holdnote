//! Persistent job queue (SQLite, `paths::db_path()`).
//! OWNER: jobs/worker agent. Keep the public API below stable; add fields/methods as needed.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobStatus {
    Recording,
    Recorded,
    Queued,
    Transcribing,
    Transcribed,
    Saving,
    /// Drive API confirmed creation (`RemoteConfirmed`).
    RemoteConfirmed,
    /// Waiting for retry (offline / 5xx / 429). Amber.
    RetryWait,
    /// Needs user action (re-auth, folder missing, permission). Red + warning icon.
    ActionRequired,
    /// Transcription failed; audio kept for retry.
    Error,
    /// Too short / silent / cancelled.
    Discarded,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        todo!()
    }
    pub fn parse(s: &str) -> Option<Self> {
        let _ = s;
        todo!()
    }
    /// Japanese label for UI (e.g. "Google Driveに保存しました", "保存待ち").
    pub fn label_ja(&self) -> &'static str {
        todo!()
    }
    /// True when the job has not reached a terminal state (RemoteConfirmed/Discarded).
    pub fn is_pending(&self) -> bool {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub job_id: String,
    /// Recording start time with local offset.
    pub created_at: DateTime<FixedOffset>,
    pub duration_ms: i64,
    pub audio_path: PathBuf,
    pub markdown_path: Option<PathBuf>,
    /// Always "drive_api" in this build (kept for the SaveBackend abstraction).
    pub backend_type: String,
    /// Snapshot of the destination Drive folder ID at job creation.
    pub destination: Option<String>,
    pub destination_name: Option<String>,
    pub status: JobStatus,
    pub attempts: i64,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub remote_file_id: Option<String>,
    pub remote_web_link: Option<String>,
    /// SHA-256 (hex) of the generated markdown.
    pub content_sha256: Option<String>,
    /// First ~80 chars of the transcript for history rows.
    pub transcript_preview: Option<String>,
    pub file_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub struct JobStore {
    // implementation detail (e.g. Mutex<rusqlite::Connection>)
}

impl JobStore {
    /// Open/create the DB and run migrations.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let _ = path;
        todo!()
    }
    pub fn insert(&self, job: &Job) -> anyhow::Result<()> {
        let _ = job;
        todo!()
    }
    /// Replace all mutable columns of an existing job.
    pub fn update(&self, job: &Job) -> anyhow::Result<()> {
        let _ = job;
        todo!()
    }
    pub fn get(&self, job_id: &str) -> anyhow::Result<Option<Job>> {
        let _ = job_id;
        todo!()
    }
    /// Most recent first.
    pub fn list_recent(&self, limit: usize) -> anyhow::Result<Vec<Job>> {
        let _ = limit;
        todo!()
    }
    /// Jobs not in a terminal state, oldest first.
    pub fn list_pending(&self) -> anyhow::Result<Vec<Job>> {
        todo!()
    }
    /// Number of pending (unsaved) jobs, for the menu bar.
    pub fn pending_count(&self) -> anyhow::Result<usize> {
        todo!()
    }
    /// Most recent RemoteConfirmed job.
    pub fn last_saved(&self) -> anyhow::Result<Option<Job>> {
        todo!()
    }
    pub fn delete(&self, job_id: &str) -> anyhow::Result<()> {
        let _ = job_id;
        todo!()
    }
    pub fn delete_all(&self) -> anyhow::Result<()> {
        todo!()
    }
}
