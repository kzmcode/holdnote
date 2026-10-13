//! Save backends. This build ships only the Google Drive API backend (method B).

pub mod drive_api;
pub mod oauth;

use crate::jobs::Job;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedFolder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum SaveReceipt {
    /// Drive API returned success for files.create (or a matching file was found by job id).
    RemoteConfirmed { file_id: String, web_view_link: Option<String> },
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SaveError {
    /// Network down / timeout / 5xx / 429 → RetryWait with backoff.
    #[error("{0}")]
    Retryable(String),
    /// Re-auth needed (refresh failed / revoked) → ActionRequired.
    #[error("Googleの再認証が必要です")]
    AuthRequired,
    /// Destination folder missing / moved / no permission → ActionRequired (reselect folder).
    #[error("保存先フォルダにアクセスできません: {0}")]
    DestinationUnavailable(String),
    /// Client not configured, other non-retryable errors → ActionRequired.
    #[error("{0}")]
    Fatal(String),
}

impl SaveError {
    /// Stable code stored in `jobs.last_error_code`.
    pub fn code(&self) -> &'static str {
        match self {
            SaveError::Retryable(_) => "retryable",
            SaveError::AuthRequired => "auth_required",
            SaveError::DestinationUnavailable(_) => "destination_unavailable",
            SaveError::Fatal(_) => "fatal",
        }
    }
}

pub trait SaveBackend: Send + Sync {
    /// Upload `markdown` for `job` (job.destination is the folder snapshot, job.file_name the name).
    /// Must be idempotent per job id: implementations check for an existing file with the job id first
    /// when `job.attempts > 0` (API-09, §7.4).
    fn save(&self, job: &Job, markdown: &str) -> Result<SaveReceipt, SaveError>;
}
