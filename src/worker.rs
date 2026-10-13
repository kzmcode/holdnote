//! Background pipeline: QUEUED → TRANSCRIBING → TRANSCRIBED → SAVING → RemoteConfirmed / RetryWait / ActionRequired.
//! OWNER: jobs/worker agent.

use std::sync::{Arc, RwLock};

use crate::events::EventSink;
use crate::jobs::JobStore;
use crate::settings::Settings;

/// Handle to the single background worker thread. Cheap to clone.
#[derive(Clone)]
pub struct WorkerHandle {
    // implementation detail (e.g. mpsc::Sender<Command>)
}

impl WorkerHandle {
    /// Process queue now (new job enqueued, network back, settings changed...).
    pub fn wake(&self) {
        todo!()
    }
    /// Reset a failed/waiting job to Queued (or to Transcribed when markdown exists) and process it.
    pub fn retry(&self, job_id: &str) {
        let _ = job_id;
        todo!()
    }
    pub fn retry_all(&self) {
        todo!()
    }
}

/// Spawn the worker. On start it performs crash recovery:
/// - jobs left in Transcribing → Queued, Saving → Transcribed (re-check Drive by job id before re-upload),
/// - jobs left in Recording whose WAV is recoverable → Recorded/Queued (use `recorder::repair_wav`),
/// - orphan WAVs in `paths::audio_dir()` without a job row are logged (and imported as Recorded jobs),
/// - deletes local audio of RemoteConfirmed jobs older than `audio_retention_hours`.
pub fn spawn(store: Arc<JobStore>, settings: Arc<RwLock<Settings>>, sink: EventSink) -> WorkerHandle {
    let _ = (store, settings, sink);
    todo!()
}
