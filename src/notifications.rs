//! User notifications (macOS Notification Center). Must not block the caller.
//! OWNER: app agent.

/// Post a notification (title, body). Fire-and-forget.
pub fn notify(title: &str, body: &str) {
    let _ = (title, body);
    todo!()
}
