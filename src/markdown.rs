//! Markdown file generation (requirements §5.2).
//! OWNER: transcriber/markdown agent.

use chrono::{DateTime, FixedOffset};

#[derive(Debug, Clone)]
pub struct MarkdownMeta {
    pub created_at: DateTime<FixedOffset>,
    pub duration_seconds: f64,
    pub language: String,
    /// "drive_api" in this build.
    pub save_mode: String,
}

/// `YYYY-MM-DD_HH-mm-ss_<first 6 hex chars of job id>.md`
pub fn file_name(created_at: &DateTime<FixedOffset>, job_id: &str) -> String {
    let _ = (created_at, job_id);
    todo!()
}

/// Front matter + "# 音声メモ（YYYY-MM-DD HH:MM）" + "## 文字起こし" + transcript. Ends with "\n".
pub fn render(meta: &MarkdownMeta, transcript: &str) -> String {
    let _ = (meta, transcript);
    todo!()
}
