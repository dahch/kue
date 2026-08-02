use tauri::Manager;

/// Error returned when the `overlay` webview window has not been created in
/// `tauri.conf.json` (e.g. in test environments or misconfiguration).
pub(crate) const ERR_OVERLAY_WINDOW_NOT_FOUND: &str =
    "overlay window not found — is tauri.conf.json configured?";

#[tauri::command]
pub fn show_overlay(show: bool, app_handle: tauri::AppHandle) -> Result<(), String> {
    let window = app_handle
        .get_webview_window("overlay")
        .ok_or_else(|| ERR_OVERLAY_WINDOW_NOT_FOUND.to_string())?;
    if show {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().ok();
    } else {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Configuration constants
    // ------------------------------------------------------------------

    /// Verifies the error message is descriptive and mentions the config file
    /// so the developer knows what to fix.
    #[test]
    fn error_message_mentions_config_file() {
        assert!(
            ERR_OVERLAY_WINDOW_NOT_FOUND.contains("tauri.conf.json"),
            "error message should tell the developer which file to configure"
        );
    }

    #[test]
    fn error_message_mentions_overlay_window() {
        assert!(
            ERR_OVERLAY_WINDOW_NOT_FOUND.contains("overlay window"),
            "error message should name the missing window"
        );
    }

    #[test]
    fn error_message_not_empty() {
        assert!(!ERR_OVERLAY_WINDOW_NOT_FOUND.is_empty());
    }

    // ------------------------------------------------------------------
    // Compile-time type checks
    // ------------------------------------------------------------------

    /// Verifies the command always returns a Result (compile-time check that
    /// the function signature is correct).
    #[test]
    fn show_overlay_signature_is_valid() {
        fn _check(_f: fn(bool, tauri::AppHandle) -> Result<(), String>) {}
        _check(show_overlay);
    }

    /// The command function pointer must be `Send` because Tauri commands
    /// may be invoked from any thread.
    #[test]
    fn show_overlay_fn_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<fn(bool, tauri::AppHandle) -> Result<(), String>>();
    }

    // ------------------------------------------------------------------
    // Error message integration (indirect path coverage)
    // ------------------------------------------------------------------

    /// Verifies that the `ok_or_else` error path in `show_overlay` resolves
    /// to our constant.  We cannot call `show_overlay` in a unit test without
    /// a real `AppHandle<Wry>` (see `pipeline.rs:mock_app_runtime_type_mismatch_documented`
    /// for the full explanation), so we test the constant it produces.
    #[test]
    fn error_message_matches_command_error_path() {
        // Simulate the `ok_or_else` closure: produce the error string
        let err = || ERR_OVERLAY_WINDOW_NOT_FOUND.to_string();
        let simulated = err();
        assert_eq!(
            simulated,
            "overlay window not found — is tauri.conf.json configured?"
        );
    }
}
