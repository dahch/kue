use keyring::Entry;

const KEYRING_SERVICE: &str = "kue";

/// Save an API key to the system keychain for a given provider.
/// The key is identified by the provider name as the "user" in keyring terms.
pub fn save_api_key(provider: &str, key: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, provider).map_err(|e| e.to_string())?;
    entry.set_password(key).map_err(|e| e.to_string())
}

/// Retrieve an API key from the system keychain for a given provider.
/// Returns `Err` if no key is stored or if keychain access fails.
pub fn get_api_key(provider: &str) -> Result<String, String> {
    let entry = Entry::new(KEYRING_SERVICE, provider).map_err(|e| e.to_string())?;
    entry.get_password().map_err(|e| e.to_string())
}

/// Delete an API key from the system keychain for a given provider.
/// Succeeds even if the key doesn't exist (keyring returns an error which we
/// ignore for idempotency).
pub fn delete_api_key(provider: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, provider).map_err(|e| e.to_string())?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn save_key(provider: String, key: String) -> Result<(), String> {
    save_api_key(&provider, &key)
}

#[tauri::command]
pub fn has_key(provider: String) -> Result<bool, String> {
    Ok(get_api_key(&provider).is_ok())
}

#[tauri::command]
pub fn delete_key(provider: String) -> Result<(), String> {
    delete_api_key(&provider)
}

#[tauri::command]
pub fn list_saved_keys(providers: Vec<String>) -> Result<Vec<String>, String> {
    let saved = providers
        .into_iter()
        .filter(|p| get_api_key(p).is_ok())
        .collect();
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires real OS keychain — run manually or in a macOS CI environment"]
    fn save_and_get_roundtrip() {
        save_api_key("test-provider", "sk-test-key-12345").unwrap();
        let retrieved = get_api_key("test-provider").unwrap();
        assert_eq!(retrieved, "sk-test-key-12345");
        delete_api_key("test-provider").unwrap();
    }

    #[test]
    #[ignore = "requires real OS keychain"]
    fn get_nonexistent_key_returns_err() {
        let result = get_api_key("nonexistent-provider-xyz");
        assert!(result.is_err(), "should error for nonexistent key");
    }

    #[test]
    #[ignore = "requires real OS keychain"]
    fn delete_nonexistent_key_is_idempotent() {
        // Should not panic or error
        delete_api_key("ghost-provider").unwrap();
    }

    #[test]
    #[ignore = "requires real OS keychain"]
    fn delete_then_get_returns_err() {
        save_api_key("ephemeral", "temp-key").unwrap();
        delete_api_key("ephemeral").unwrap();
        let result = get_api_key("ephemeral");
        assert!(result.is_err(), "deleted key should not be retrievable");
    }

    #[test]
    #[ignore = "requires real OS keychain"]
    fn multiple_providers_dont_conflict() {
        save_api_key("provider-a", "key-a").unwrap();
        save_api_key("provider-b", "key-b").unwrap();
        assert_eq!(get_api_key("provider-a").unwrap(), "key-a");
        assert_eq!(get_api_key("provider-b").unwrap(), "key-b");
        delete_api_key("provider-a").unwrap();
        delete_api_key("provider-b").unwrap();
    }

    #[test]
    #[ignore = "requires real OS keychain"]
    fn key_never_in_settings_table() {
        save_api_key("openai", "sk-test").unwrap();
        // Verify: the key is NOT in the settings table of kue.db
        // This is an architectural test — the keyring crate stores data
        // in the macOS Keychain, not in any app file. We verify by
        // checking that no file in the temp dir contains the key.
        let tmp = std::env::temp_dir();
        let entries = std::fs::read_dir(&tmp).ok();
        if let Some(entries) = entries {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().contains("kue") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        assert!(
                            !content.contains("sk-test"),
                            "key should never appear in app files"
                        );
                    }
                }
            }
        }
        delete_api_key("openai").unwrap();
    }
}
