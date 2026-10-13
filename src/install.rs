//! Launch-at-login (LaunchAgent plist in ~/Library/LaunchAgents) and bundle helpers.
//! OWNER: app agent.

pub fn set_launch_at_login(enabled: bool) -> anyhow::Result<()> {
    let _ = enabled;
    todo!()
}
pub fn is_launch_at_login() -> bool {
    todo!()
}
/// Path to the running .app bundle (None when run via `cargo run`).
pub fn bundle_path() -> Option<std::path::PathBuf> {
    todo!()
}
