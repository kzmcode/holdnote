//! Persistent job queue (SQLite, `paths::db_path()`).
//! OWNER: jobs/worker agent. Keep the public API below stable; add fields/methods as needed.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, OptionalExtension, Row};

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
        match self {
            JobStatus::Recording => "recording",
            JobStatus::Recorded => "recorded",
            JobStatus::Queued => "queued",
            JobStatus::Transcribing => "transcribing",
            JobStatus::Transcribed => "transcribed",
            JobStatus::Saving => "saving",
            JobStatus::RemoteConfirmed => "remote_confirmed",
            JobStatus::RetryWait => "retry_wait",
            JobStatus::ActionRequired => "action_required",
            JobStatus::Error => "error",
            JobStatus::Discarded => "discarded",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "recording" => JobStatus::Recording,
            "recorded" => JobStatus::Recorded,
            "queued" => JobStatus::Queued,
            "transcribing" => JobStatus::Transcribing,
            "transcribed" => JobStatus::Transcribed,
            "saving" => JobStatus::Saving,
            "remote_confirmed" => JobStatus::RemoteConfirmed,
            "retry_wait" => JobStatus::RetryWait,
            "action_required" => JobStatus::ActionRequired,
            "error" => JobStatus::Error,
            "discarded" => JobStatus::Discarded,
            _ => return None,
        })
    }
    /// Japanese label for UI (e.g. "Google Driveに保存しました", "保存待ち").
    pub fn label_ja(&self) -> &'static str {
        match self {
            JobStatus::Recording => "録音中",
            JobStatus::Recorded | JobStatus::Queued => "待機中（処理待ち）",
            JobStatus::Transcribing => "文字起こし中…",
            JobStatus::Transcribed => "保存待ち（文字起こし済み）",
            JobStatus::Saving => "保存中…",
            JobStatus::RemoteConfirmed => "Google Driveに保存しました",
            JobStatus::RetryWait => "保存待ち",
            JobStatus::ActionRequired => "保存できませんでした",
            JobStatus::Error => "文字起こし失敗",
            JobStatus::Discarded => "破棄",
        }
    }
    /// True when the job has not reached a terminal state (RemoteConfirmed/Discarded).
    pub fn is_pending(&self) -> bool {
        !matches!(self, JobStatus::RemoteConfirmed | JobStatus::Discarded)
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
    conn: Mutex<Connection>,
}

const SCHEMA_VERSION: i64 = 1;

const COLS: &str = "job_id, created_at, duration_ms, audio_path, markdown_path, backend_type, destination, \
    destination_name, status, attempts, last_error_code, last_error_message, next_retry_at, remote_file_id, \
    remote_web_link, content_sha256, transcript_preview, file_name, updated_at";

fn conv_err(i: usize, msg: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, msg.into())
}

fn parse_utc(i: usize, s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| conv_err(i, e.to_string()))
}

fn row_to_job(r: &Row) -> rusqlite::Result<Job> {
    let created: String = r.get(1)?;
    let status: String = r.get(8)?;
    let next: Option<String> = r.get(12)?;
    let updated: String = r.get(18)?;
    Ok(Job {
        job_id: r.get(0)?,
        created_at: DateTime::parse_from_rfc3339(&created).map_err(|e| conv_err(1, e.to_string()))?,
        duration_ms: r.get(2)?,
        audio_path: PathBuf::from(r.get::<_, String>(3)?),
        markdown_path: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
        backend_type: r.get(5)?,
        destination: r.get(6)?,
        destination_name: r.get(7)?,
        status: JobStatus::parse(&status).ok_or_else(|| conv_err(8, format!("unknown status {status}")))?,
        attempts: r.get(9)?,
        last_error_code: r.get(10)?,
        last_error_message: r.get(11)?,
        next_retry_at: match next {
            Some(n) => Some(parse_utc(12, &n)?),
            None => None,
        },
        remote_file_id: r.get(13)?,
        remote_web_link: r.get(14)?,
        content_sha256: r.get(15)?,
        transcript_preview: r.get(16)?,
        file_name: r.get(17)?,
        updated_at: parse_utc(18, &updated)?,
    })
}

impl JobStore {
    /// Open/create the DB and run migrations.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS jobs (
                    job_id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    audio_path TEXT NOT NULL,
                    markdown_path TEXT,
                    backend_type TEXT NOT NULL DEFAULT 'drive_api',
                    destination TEXT,
                    destination_name TEXT,
                    status TEXT NOT NULL,
                    attempts INTEGER NOT NULL DEFAULT 0,
                    last_error_code TEXT,
                    last_error_message TEXT,
                    next_retry_at TEXT,
                    remote_file_id TEXT,
                    remote_web_link TEXT,
                    content_sha256 TEXT,
                    transcript_preview TEXT,
                    file_name TEXT,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
                CREATE INDEX IF NOT EXISTS idx_jobs_created ON jobs(created_at);",
            )?;
        }
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert(&self, job: &Job) -> anyhow::Result<()> {
        let sql = format!(
            "INSERT INTO jobs ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)"
        );
        self.conn().execute(&sql, Self::bind(job))?;
        Ok(())
    }

    fn bind(j: &Job) -> impl rusqlite::Params {
        use rusqlite::types::Value;
        let text = |s: String| Value::Text(s);
        let opt = |s: Option<String>| s.map_or(Value::Null, Value::Text);
        rusqlite::params_from_iter(vec![
            text(j.job_id.clone()),
            text(j.created_at.to_rfc3339()),
            Value::Integer(j.duration_ms),
            text(j.audio_path.to_string_lossy().into_owned()),
            opt(j.markdown_path.as_ref().map(|p| p.to_string_lossy().into_owned())),
            text(j.backend_type.clone()),
            opt(j.destination.clone()),
            opt(j.destination_name.clone()),
            text(j.status.as_str().to_string()),
            Value::Integer(j.attempts),
            opt(j.last_error_code.clone()),
            opt(j.last_error_message.clone()),
            opt(j.next_retry_at.map(|d| d.to_rfc3339())),
            opt(j.remote_file_id.clone()),
            opt(j.remote_web_link.clone()),
            opt(j.content_sha256.clone()),
            opt(j.transcript_preview.clone()),
            opt(j.file_name.clone()),
            text(j.updated_at.to_rfc3339()),
        ])
    }

    /// Replace all mutable columns of an existing job.
    pub fn update(&self, job: &Job) -> anyhow::Result<()> {
        let sql = "UPDATE jobs SET created_at=?2, duration_ms=?3, audio_path=?4, markdown_path=?5, backend_type=?6, \
            destination=?7, destination_name=?8, status=?9, attempts=?10, last_error_code=?11, last_error_message=?12, \
            next_retry_at=?13, remote_file_id=?14, remote_web_link=?15, content_sha256=?16, transcript_preview=?17, \
            file_name=?18, updated_at=?19 WHERE job_id=?1";
        let n = self.conn().execute(sql, Self::bind(job))?;
        if n == 0 {
            anyhow::bail!("job not found: {}", job.job_id);
        }
        Ok(())
    }

    pub fn get(&self, job_id: &str) -> anyhow::Result<Option<Job>> {
        let sql = format!("SELECT {COLS} FROM jobs WHERE job_id=?1");
        Ok(self.conn().query_row(&sql, [job_id], row_to_job).optional()?)
    }

    fn query(&self, tail: &str, p: impl rusqlite::Params) -> anyhow::Result<Vec<Job>> {
        let sql = format!("SELECT {COLS} FROM jobs {tail}");
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(p, row_to_job)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Most recent first.
    pub fn list_recent(&self, limit: usize) -> anyhow::Result<Vec<Job>> {
        self.query("ORDER BY created_at DESC, rowid DESC LIMIT ?1", [limit as i64])
    }

    /// Jobs not in a terminal state, oldest first.
    pub fn list_pending(&self) -> anyhow::Result<Vec<Job>> {
        self.query(
            "WHERE status NOT IN ('remote_confirmed','discarded') ORDER BY created_at ASC, rowid ASC",
            [],
        )
    }

    /// Number of pending (unsaved) jobs, for the menu bar.
    pub fn pending_count(&self) -> anyhow::Result<usize> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM jobs WHERE status NOT IN ('remote_confirmed','discarded')",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Most recent RemoteConfirmed job.
    pub fn last_saved(&self) -> anyhow::Result<Option<Job>> {
        Ok(self
            .query("WHERE status='remote_confirmed' ORDER BY updated_at DESC, rowid DESC LIMIT 1", [])?
            .into_iter()
            .next())
    }

    pub fn delete(&self, job_id: &str) -> anyhow::Result<()> {
        self.conn().execute("DELETE FROM jobs WHERE job_id=?1", [job_id])?;
        Ok(())
    }

    pub fn delete_all(&self) -> anyhow::Result<()> {
        self.conn().execute("DELETE FROM jobs", [])?;
        Ok(())
    }

    /// RemoteConfirmed jobs last updated before `cutoff` (for audio retention cleanup).
    pub fn list_confirmed_before(&self, cutoff: DateTime<Utc>) -> anyhow::Result<Vec<Job>> {
        let all = self.query("WHERE status='remote_confirmed'", [])?;
        Ok(all.into_iter().filter(|j| j.updated_at < cutoff).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("holdnote-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn job(id: &str, status: JobStatus, created: &str) -> Job {
        Job {
            job_id: id.into(),
            created_at: DateTime::parse_from_rfc3339(created).unwrap(),
            duration_ms: 1234,
            audio_path: PathBuf::from(format!("/tmp/{id}.wav")),
            markdown_path: None,
            backend_type: "drive_api".into(),
            destination: Some("folder".into()),
            destination_name: Some("メモ".into()),
            status,
            attempts: 0,
            last_error_code: None,
            last_error_message: None,
            next_retry_at: None,
            remote_file_id: None,
            remote_web_link: None,
            content_sha256: None,
            transcript_preview: None,
            file_name: None,
            updated_at: Utc::now(),
        }
    }

    fn store() -> (TmpDir, JobStore) {
        let d = TmpDir::new();
        let s = JobStore::open(&d.0.join("jobs.sqlite3")).unwrap();
        (d, s)
    }

    #[test]
    fn status_roundtrip_and_labels() {
        for s in [
            JobStatus::Recording,
            JobStatus::Recorded,
            JobStatus::Queued,
            JobStatus::Transcribing,
            JobStatus::Transcribed,
            JobStatus::Saving,
            JobStatus::RemoteConfirmed,
            JobStatus::RetryWait,
            JobStatus::ActionRequired,
            JobStatus::Error,
            JobStatus::Discarded,
        ] {
            assert_eq!(JobStatus::parse(s.as_str()), Some(s));
            assert!(!s.label_ja().is_empty());
        }
        assert_eq!(JobStatus::parse("x"), None);
        assert!(!JobStatus::Discarded.is_pending());
        assert!(JobStatus::Error.is_pending());
    }

    #[test]
    fn crud_and_queries() {
        let (_d, s) = store();
        let mut a = job("a", JobStatus::Queued, "2026-01-01T10:00:00+09:00");
        let b = job("b", JobStatus::RemoteConfirmed, "2026-01-02T10:00:00+09:00");
        let c = job("c", JobStatus::RetryWait, "2026-01-03T10:00:00+09:00");
        for j in [&a, &b, &c] {
            s.insert(j).unwrap();
        }
        assert!(s.insert(&a).is_err());
        assert_eq!(s.get("a").unwrap().unwrap().created_at, a.created_at);
        assert!(s.get("zz").unwrap().is_none());
        assert_eq!(s.pending_count().unwrap(), 2);
        let pend: Vec<_> = s.list_pending().unwrap().into_iter().map(|j| j.job_id).collect();
        assert_eq!(pend, ["a", "c"]);
        let rec: Vec<_> = s.list_recent(2).unwrap().into_iter().map(|j| j.job_id).collect();
        assert_eq!(rec, ["c", "b"]);
        assert_eq!(s.last_saved().unwrap().unwrap().job_id, "b");

        a.status = JobStatus::RetryWait;
        a.attempts = 3;
        a.next_retry_at = Some(Utc::now());
        a.last_error_message = Some("err".into());
        s.update(&a).unwrap();
        let g = s.get("a").unwrap().unwrap();
        assert_eq!(g.status, JobStatus::RetryWait);
        assert_eq!(g.attempts, 3);
        assert!(g.next_retry_at.is_some());
        assert!(s.update(&job("nope", JobStatus::Queued, "2026-01-01T10:00:00+09:00")).is_err());

        s.delete("a").unwrap();
        assert!(s.get("a").unwrap().is_none());
        s.delete_all().unwrap();
        assert_eq!(s.list_recent(10).unwrap().len(), 0);
    }

    #[test]
    fn reopen_keeps_data() {
        let d = TmpDir::new();
        let p = d.0.join("j.db");
        JobStore::open(&p)
            .unwrap()
            .insert(&job("a", JobStatus::Queued, "2026-01-01T10:00:00+09:00"))
            .unwrap();
        assert!(JobStore::open(&p).unwrap().get("a").unwrap().is_some());
    }

    #[test]
    fn confirmed_before() {
        let (_d, s) = store();
        let mut b = job("b", JobStatus::RemoteConfirmed, "2026-01-02T10:00:00+09:00");
        b.updated_at = Utc::now() - chrono::Duration::hours(48);
        s.insert(&b).unwrap();
        s.insert(&job("c", JobStatus::RemoteConfirmed, "2026-01-03T10:00:00+09:00")).unwrap();
        let old = s.list_confirmed_before(Utc::now() - chrono::Duration::hours(24)).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].job_id, "b");
    }
}
