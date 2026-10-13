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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = app::run() {
        log::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}
