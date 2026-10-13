//! Google OAuth 2.0 for desktop apps: system browser + loopback redirect (127.0.0.1:<random>) + PKCE (S256)
//! + state check. Also the desktop Google Picker flow (trigger_onepick / allow_folder_selection).
//! OWNER: drive agent. Never log tokens.

#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
    /// Google Cloud project number, used as Picker app id if needed.
    pub project_number: Option<String>,
}

pub const SCOPE_DRIVE_FILE: &str = "https://www.googleapis.com/auth/drive.file";

#[derive(Debug, Clone)]
pub struct AuthOutcome {
    /// Email of the account if obtainable without extra scopes (e.g. from id_token with "openid email"), else None.
    pub email: Option<String>,
    /// Present when the flow included the Picker and the user picked something.
    pub picked_file_ids: Vec<String>,
}

/// Full interactive sign-in. Opens the browser, waits (≤5 min) for the loopback redirect,
/// exchanges the code, stores tokens in Keychain via `security`. Blocking — call from a background thread.
/// `with_picker`: request the desktop Picker in the same consent (prompt=consent&trigger_onepick=true&allow_folder_selection=true).
pub fn authorize_interactive(client: &OAuthClient, with_picker: bool) -> anyhow::Result<AuthOutcome> {
    let _ = (client, with_picker);
    todo!()
}

/// Valid access token: cached one if not expiring within 60 s, else refresh with the stored refresh token.
/// Err(SaveError::AuthRequired) if no refresh token or refresh rejected (invalid_grant); Retryable on network errors.
pub fn access_token(client: &OAuthClient) -> Result<String, crate::storage::SaveError> {
    let _ = client;
    todo!()
}

/// Revoke the token at Google (best effort) and delete it from Keychain (API-11).
pub fn disconnect() -> anyhow::Result<()> {
    todo!()
}

/// True if a refresh token exists in Keychain.
pub fn is_connected() -> bool {
    todo!()
}
