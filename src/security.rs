//! macOS Keychain storage for Google OAuth tokens (generic password, service = BUNDLE_ID).
//! OWNER: drive agent. Never log secret values.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredTokens {
    pub refresh_token: String,
    pub access_token: Option<String>,
    /// Unix seconds.
    pub access_token_expires_at: Option<i64>,
}

pub fn load_tokens() -> anyhow::Result<Option<StoredTokens>> {
    todo!()
}
pub fn save_tokens(tokens: &StoredTokens) -> anyhow::Result<()> {
    let _ = tokens;
    todo!()
}
pub fn delete_tokens() -> anyhow::Result<()> {
    todo!()
}
