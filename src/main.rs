//! HoldNote — hold-to-record voice memo for macOS, saved to Google Drive via the Drive API.

mod app;
mod events;
mod hotkey;
mod install;
mod jobs;
mod markdown;
mod notifications;
mod paths;
mod recorder;
mod security;
mod settings;
mod storage;
mod transcriber;
mod ui;
mod worker;

fn main() {
    init_logging();
    if let Err(e) = app::run() {
        log::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}

/// Log to stderr when run from a terminal, otherwise to
/// `~/Library/Application Support/HoldNote/logs/holdnote.log` (rotated at 1 MB).
/// Never log transcript text or OAuth tokens.
fn init_logging() {
    use std::io::IsTerminal;
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if !std::io::stderr().is_terminal() {
        let path = paths::logs_dir().join("holdnote.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
        }
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            b.target(env_logger::Target::Pipe(Box::new(f)));
        }
    }
    b.init();
}
