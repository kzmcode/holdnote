//! Google Drive REST API v3 backend (method B). Uses `drive.file` scope only.
//! OWNER: drive agent.

use crate::jobs::Job;
use crate::storage::oauth::OAuthClient;
use crate::storage::{PickedFolder, SaveBackend, SaveError, SaveReceipt};

pub struct DriveApiBackend {
    pub client: OAuthClient,
}

impl DriveApiBackend {
    pub fn new(client: OAuthClient) -> Self {
        Self { client }
    }

    /// Interactive: sign in (if needed) + desktop Picker → returns the chosen *folder* (rejects non-folders).
    /// Blocking.
    pub fn pick_folder_interactive(&self) -> anyhow::Result<PickedFolder> {
        todo!()
    }

    /// files.get(folderId, fields=id,name,mimeType,trashed,capabilities/canAddChildren).
    pub fn verify_folder(&self, folder_id: &str) -> Result<PickedFolder, SaveError> {
        let _ = folder_id;
        todo!()
    }

    /// Create `HoldNote-test-<uuid>.md` in the folder and delete it (§8.1 保存テスト).
    pub fn test_save(&self, folder_id: &str) -> Result<(), SaveError> {
        let _ = folder_id;
        todo!()
    }

    /// files.list q="'<folder>' in parents and appProperties has { key='holdnoteJobId' and value='<id>' } and trashed=false".
    pub fn find_by_job_id(&self, folder_id: &str, job_id: &str) -> Result<Option<SaveReceipt>, SaveError> {
        let _ = (folder_id, job_id);
        todo!()
    }
}

impl SaveBackend for DriveApiBackend {
    /// Multipart upload (uploadType=multipart) with parents=[folder], mimeType text/markdown,
    /// appProperties {holdnoteJobId, holdnoteSha256}; fields=id,webViewLink.
    /// When job.attempts > 0, first `find_by_job_id` to avoid duplicates (API-09).
    /// Maps: network/timeout/5xx/429 → Retryable, 401 after refresh → AuthRequired, 404/403 on parent → DestinationUnavailable.
    fn save(&self, job: &Job, markdown: &str) -> Result<SaveReceipt, SaveError> {
        let _ = (job, markdown);
        todo!()
    }
}
