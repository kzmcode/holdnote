//! macOS Keychain storage for Google OAuth tokens (generic password, service = BUNDLE_ID).
//! OWNER: drive agent. Never log secret values.

use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

const ACCOUNT: &str = "google-oauth";
/// errSecItemNotFound
const ERR_NOT_FOUND: i32 = -25300;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredTokens {
    pub refresh_token: String,
    pub access_token: Option<String>,
    /// Unix seconds.
    pub access_token_expires_at: Option<i64>,
}

pub fn load_tokens() -> anyhow::Result<Option<StoredTokens>> {
    match get_generic_password(crate::paths::BUNDLE_ID, ACCOUNT) {
        Ok(bytes) => match serde_json::from_slice::<StoredTokens>(&bytes) {
            Ok(t) => Ok(Some(t)),
            // Corrupt entry: treat as not connected.
            Err(_) => Ok(None),
        },
        Err(e) if e.code() == ERR_NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Keychainの読み取りに失敗しました (code {})", e.code())),
    }
}

pub fn save_tokens(tokens: &StoredTokens) -> anyhow::Result<()> {
    let data = serde_json::to_vec(tokens)?;
    set_generic_password(crate::paths::BUNDLE_ID, ACCOUNT, &data)
        .map_err(|e| anyhow::anyhow!("Keychainへの保存に失敗しました (code {})", e.code()))
}

pub fn delete_tokens() -> anyhow::Result<()> {
    match delete_generic_password(crate::paths::BUNDLE_ID, ACCOUNT) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERR_NOT_FOUND => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Keychainからの削除に失敗しました (code {})", e.code())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_roundtrip_json() {
        let t = StoredTokens { refresh_token: "r".into(), access_token: Some("a".into()), access_token_expires_at: Some(1) };
        let s = serde_json::to_vec(&t).unwrap();
        let back: StoredTokens = serde_json::from_slice(&s).unwrap();
        assert_eq!(back.refresh_token, "r");
        assert_eq!(back.access_token_expires_at, Some(1));
    }
}
