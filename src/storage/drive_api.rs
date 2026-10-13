//! Google Drive REST API v3 backend (method B). Uses `drive.file` scope only.
//! OWNER: drive agent.

use anyhow::anyhow;
use reqwest::blocking::{RequestBuilder, Response};
use sha2::{Digest, Sha256};

use crate::jobs::Job;
use crate::storage::oauth::{self, form_encode, OAuthClient};
use crate::storage::{PickedFolder, SaveBackend, SaveError, SaveReceipt};

const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

pub struct DriveApiBackend {
    pub client: OAuthClient,
}

/// Where an HTTP error happened; decides how 403/404 are mapped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// The request concerns the destination folder (get folder / list in folder / create in folder).
    Folder,
    /// Anything else (e.g. deleting the test file).
    Other,
}

struct Reply {
    status: u16,
    body: String,
}

// ---------- pure helpers (unit-tested) ----------

/// Escape a value for use inside single quotes in a Drive `q` expression.
pub fn escape_q(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

pub fn build_find_query(folder_id: &str, job_id: &str) -> String {
    format!(
        "'{}' in parents and appProperties has {{ key='holdnoteJobId' and value='{}' }} and trashed=false",
        escape_q(folder_id),
        escape_q(job_id)
    )
}

pub fn build_list_url(folder_id: &str, job_id: &str) -> String {
    let q = build_find_query(folder_id, job_id);
    let qs = form_encode(&[
        ("q", &q),
        ("fields", "files(id,webViewLink)"),
        ("pageSize", "1"),
        ("supportsAllDrives", "true"),
        ("includeItemsFromAllDrives", "true"),
        ("spaces", "drive"),
    ]);
    format!("{FILES_URL}?{qs}")
}

/// Build a `multipart/related` body: JSON metadata part + media part. Returns (content_type, body).
pub fn build_multipart(boundary: &str, metadata: &serde_json::Value, media_type: &str, media: &str) -> (String, Vec<u8>) {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {media_type}; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(media.as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

fn google_reason(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["errors"][0]["reason"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Map a non-success HTTP status (after the 401 refresh retry) to a SaveError.
fn map_status(status: u16, body: &str, ctx: Ctx) -> SaveError {
    let reason = google_reason(body);
    match status {
        401 => SaveError::AuthRequired,
        429 => SaveError::Retryable("Google Driveのリクエスト制限に達しました。後で再試行します".into()),
        500..=599 => SaveError::Retryable(format!("Google Driveが一時的に利用できません (HTTP {status})")),
        403 if reason == "storageQuotaExceeded" => {
            SaveError::Fatal("Google Driveの容量が不足しています。空き容量を確保してください".into())
        }
        403 if matches!(
            reason.as_str(),
            "rateLimitExceeded" | "userRateLimitExceeded" | "backendError" | "sharingRateLimitExceeded"
        ) =>
        {
            SaveError::Retryable("Google Driveのリクエスト制限に達しました。後で再試行します".into())
        }
        403 if matches!(reason.as_str(), "accessNotConfigured" | "SERVICE_DISABLED") => {
            SaveError::Fatal("Google Cloudプロジェクトで Drive API が有効になっていません".into())
        }
        404 if ctx == Ctx::Folder => SaveError::DestinationUnavailable("フォルダが見つかりません(削除・移動された可能性があります)".into()),
        403 if ctx == Ctx::Folder => SaveError::DestinationUnavailable("フォルダへの書き込み権限がありません".into()),
        _ => SaveError::Fatal(format!(
            "Google Drive APIエラー (HTTP {status}{})",
            if reason.is_empty() { String::new() } else { format!(", {reason}") }
        )),
    }
}

fn is_ok(status: u16) -> bool {
    (200..300).contains(&status)
}

fn net_err(e: reqwest::Error) -> SaveError {
    SaveError::Retryable(format!("Google Driveに接続できません: {}", e.without_url()))
}

fn parse_json(r: &Reply) -> Result<serde_json::Value, SaveError> {
    serde_json::from_str(&r.body).map_err(|_| SaveError::Retryable("Google Driveの応答を解釈できません".into()))
}

// ---------- backend ----------

impl DriveApiBackend {
    pub fn new(client: OAuthClient) -> Self {
        Self { client }
    }

    /// Send a request with bearer auth; on 401 refresh the token once and retry.
    fn send(&self, build: impl Fn(&reqwest::blocking::Client) -> RequestBuilder) -> Result<Reply, SaveError> {
        let http = oauth::http_client()?;
        for attempt in 0..2 {
            let token = oauth::access_token(&self.client)?;
            let resp: Response = build(&http).bearer_auth(token).send().map_err(net_err)?;
            let status = resp.status().as_u16();
            let body = resp.text().map_err(net_err)?;
            if status == 401 && attempt == 0 {
                oauth::invalidate_cached_token();
                continue;
            }
            return Ok(Reply { status, body });
        }
        Err(SaveError::AuthRequired)
    }

    /// Interactive: sign in (if needed) + desktop Picker → returns the chosen *folder* (rejects non-folders).
    /// Blocking.
    pub fn pick_folder_interactive(&self) -> anyhow::Result<PickedFolder> {
        let outcome = oauth::authorize_interactive(&self.client, true)?;
        let id = outcome
            .picked_file_ids
            .first()
            .ok_or_else(|| anyhow!("フォルダが選択されませんでした"))?;
        self.verify_folder(id).map_err(|e| anyhow!("{e}"))
    }

    /// files.get(folderId, fields=id,name,mimeType,trashed,capabilities/canAddChildren).
    pub fn verify_folder(&self, folder_id: &str) -> Result<PickedFolder, SaveError> {
        let qs = form_encode(&[
            ("fields", "id,name,mimeType,trashed,capabilities/canAddChildren"),
            ("supportsAllDrives", "true"),
        ]);
        let url = format!("{FILES_URL}/{}?{qs}", url::form_urlencoded::byte_serialize(folder_id.as_bytes()).collect::<String>());
        let r = self.send(|c| c.get(&url))?;
        if !is_ok(r.status) {
            return Err(map_status(r.status, &r.body, Ctx::Folder));
        }
        let v = parse_json(&r)?;
        if v["mimeType"].as_str() != Some(FOLDER_MIME) {
            return Err(SaveError::Fatal("フォルダを選択してください".into()));
        }
        if v["trashed"].as_bool() == Some(true) {
            return Err(SaveError::DestinationUnavailable("フォルダがゴミ箱にあります".into()));
        }
        if v["capabilities"]["canAddChildren"].as_bool() == Some(false) {
            return Err(SaveError::DestinationUnavailable("フォルダへの書き込み権限がありません".into()));
        }
        Ok(PickedFolder {
            id: v["id"].as_str().unwrap_or(folder_id).to_string(),
            name: v["name"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Create `HoldNote-test-<uuid>.md` in the folder and delete it (§8.1 保存テスト).
    pub fn test_save(&self, folder_id: &str) -> Result<(), SaveError> {
        let name = format!("HoldNote-test-{}.md", uuid::Uuid::new_v4());
        let (id, _) = self.create_file(folder_id, &name, "HoldNote save test\n", serde_json::json!({}))?;
        let url = format!("{FILES_URL}/{id}?supportsAllDrives=true");
        // Best effort cleanup; the write itself already proved access.
        let _ = self.send(|c| c.delete(&url));
        Ok(())
    }

    /// files.list q="'<folder>' in parents and appProperties has { key='holdnoteJobId' and value='<id>' } and trashed=false".
    pub fn find_by_job_id(&self, folder_id: &str, job_id: &str) -> Result<Option<SaveReceipt>, SaveError> {
        let url = build_list_url(folder_id, job_id);
        let r = self.send(|c| c.get(&url))?;
        if !is_ok(r.status) {
            return Err(map_status(r.status, &r.body, Ctx::Folder));
        }
        let v = parse_json(&r)?;
        Ok(v["files"].as_array().and_then(|a| a.first()).and_then(|f| {
            Some(SaveReceipt::RemoteConfirmed {
                file_id: f["id"].as_str()?.to_string(),
                web_view_link: f["webViewLink"].as_str().map(|s| s.to_string()),
            })
        }))
    }

    /// Multipart create. Returns (file id, webViewLink).
    fn create_file(
        &self,
        folder_id: &str,
        name: &str,
        content: &str,
        app_properties: serde_json::Value,
    ) -> Result<(String, Option<String>), SaveError> {
        let mut meta = serde_json::json!({
            "name": name,
            "parents": [folder_id],
            "mimeType": "text/markdown",
        });
        if app_properties.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            meta["appProperties"] = app_properties;
        }
        let boundary = format!("holdnote-{}", uuid::Uuid::new_v4().simple());
        let (ctype, body) = build_multipart(&boundary, &meta, "text/markdown", content);
        let url = format!("{UPLOAD_URL}?uploadType=multipart&supportsAllDrives=true&fields=id,webViewLink");
        let r = self.send(|c| c.post(&url).header("Content-Type", ctype.clone()).body(body.clone()))?;
        if !is_ok(r.status) {
            return Err(map_status(r.status, &r.body, Ctx::Folder));
        }
        let v = parse_json(&r)?;
        let id = v["id"]
            .as_str()
            .ok_or_else(|| SaveError::Retryable("Google Driveの応答にファイルIDがありません".into()))?
            .to_string();
        Ok((id, v["webViewLink"].as_str().map(|s| s.to_string())))
    }
}

impl SaveBackend for DriveApiBackend {
    /// Multipart upload (uploadType=multipart) with parents=[folder], mimeType text/markdown,
    /// appProperties {holdnoteJobId, holdnoteSha256}; fields=id,webViewLink.
    /// When job.attempts > 0, first `find_by_job_id` to avoid duplicates (API-09).
    /// Maps: network/timeout/5xx/429 → Retryable, 401 after refresh → AuthRequired, 404/403 on parent → DestinationUnavailable.
    fn save(&self, job: &Job, markdown: &str) -> Result<SaveReceipt, SaveError> {
        let folder = job
            .destination
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| SaveError::Fatal("保存先フォルダが設定されていません".into()))?;
        if job.attempts > 0 {
            if let Some(found) = self.find_by_job_id(folder, &job.job_id)? {
                return Ok(found);
            }
        }
        let name = job
            .file_name
            .clone()
            .unwrap_or_else(|| format!("HoldNote-{}.md", job.created_at.format("%Y%m%d-%H%M%S")));
        let sha = job
            .content_sha256
            .clone()
            .unwrap_or_else(|| Sha256::digest(markdown.as_bytes()).iter().map(|b| format!("{b:02x}")).collect());
        let props = serde_json::json!({ "holdnoteJobId": job.job_id, "holdnoteSha256": sha });
        let (file_id, web_view_link) = self.create_file(folder, &name, markdown, props)?;
        Ok(SaveReceipt::RemoteConfirmed { file_id, web_view_link })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_escaping() {
        assert_eq!(escape_q("a'b\\c"), "a\\'b\\\\c");
        let q = build_find_query("F1", "j'1");
        assert_eq!(
            q,
            "'F1' in parents and appProperties has { key='holdnoteJobId' and value='j\\'1' } and trashed=false"
        );
    }

    #[test]
    fn list_url_encoded() {
        let u = build_list_url("F1", "J1");
        assert!(u.starts_with("https://www.googleapis.com/drive/v3/files?q="));
        assert!(u.contains("supportsAllDrives=true"));
        assert!(!u.contains(' '));
        assert!(!u.contains('{'));
        assert!(u.contains("%27F1%27"));
    }

    #[test]
    fn multipart_body() {
        let meta = serde_json::json!({"name":"a.md","parents":["F"]});
        let (ct, body) = build_multipart("BND", &meta, "text/markdown", "# hi\n日本語");
        assert_eq!(ct, "multipart/related; boundary=BND");
        let s = String::from_utf8(body).unwrap();
        assert!(s.starts_with("--BND\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{"));
        assert!(s.contains("\r\n--BND\r\nContent-Type: text/markdown; charset=UTF-8\r\n\r\n# hi\n日本語\r\n--BND--\r\n"));
        assert!(s.ends_with("--BND--\r\n"));
    }

    #[test]
    fn status_mapping() {
        let quota = r#"{"error":{"errors":[{"reason":"storageQuotaExceeded"}]}}"#;
        assert!(matches!(map_status(403, quota, Ctx::Folder), SaveError::Fatal(m) if m.contains("容量")));
        let rate = r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        assert!(matches!(map_status(403, rate, Ctx::Folder), SaveError::Retryable(_)));
        assert!(matches!(map_status(403, "{}", Ctx::Folder), SaveError::DestinationUnavailable(_)));
        assert!(matches!(map_status(404, "", Ctx::Folder), SaveError::DestinationUnavailable(_)));
        assert!(matches!(map_status(404, "", Ctx::Other), SaveError::Fatal(_)));
        assert!(matches!(map_status(429, "", Ctx::Folder), SaveError::Retryable(_)));
        assert!(matches!(map_status(503, "", Ctx::Folder), SaveError::Retryable(_)));
        assert!(matches!(map_status(401, "", Ctx::Folder), SaveError::AuthRequired));
        assert!(matches!(map_status(400, "", Ctx::Folder), SaveError::Fatal(_)));
    }
}
