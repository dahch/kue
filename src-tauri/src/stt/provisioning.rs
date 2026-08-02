use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tauri::Emitter;
use tauri::Manager;

use super::MOONSHINE_BASE;

// ---------------------------------------------------------------------------
// Download URLs
// ---------------------------------------------------------------------------

/// macOS arm64 wheel for moonshine-voice v0.0.73 (from PyPI).
/// Contains libmoonshine.dylib + libonnxruntime.1.23.2.dylib.
const WHEEL_URL: &str = "https://files.pythonhosted.org/packages/cc/a3/af1e1f0aaef7175727f9fedfec6863f551c0e00159a796b11a6579c6611d/moonshine_voice-0.0.73-py3-none-macosx_15_0_arm64.whl";

/// SHA-256 of the wheel published by PyPI (from pypi.org/pypi/moonshine-voice/json).
/// Trust anchor: PyPI publishes and signs these digests.
const WHEEL_SHA256: &str = "fb4c81f2fa96674d336530bc1cf2f86b6a4fc8b3c02e138a7a5c768c283f8bc7";

/// Base URL for Moonshine Medium Streaming English model (quantised).
const MODEL_BASE: &str = "https://download.moonshine.ai/model/medium-streaming-en/quantized";

/// Model file manifests: (filename, expected_size, sha256_hex).
///
/// ⚠ Trust level – pinned by us (not vendor-published):
///   These hashes were computed from files downloaded at build time.
///   They protect against *future* CDN compromise / substitution, but
///   they cannot detect a model that was already tampered with at source
///   on the day the pins were recorded. A vendor-published signature
///   would be stronger — none exists today from Moonshine.
const MODEL_FILES: &[(&str, u64, &str)] = &[
    (
        "adapter.ort",
        3_647_712,
        "16307442b7f4229f2f1511fc51b545cec9616e55872c588f3a297bbc6f4762ea",
    ),
    (
        "cross_kv.ort",
        11_544_952,
        "354b9a955caeb768b528f447f0a36ce4b850ca7b4531900165df304d97904fba",
    ),
    (
        "decoder_kv.ort",
        146_216_448,
        "fa67aa87521247f5bf44d3e44d4e4978e58c1f114249c3c6909c882624056715",
    ),
    (
        "decoder_kv_with_attention.ort",
        146_138_304,
        "40919de95d08690da3a8ff6df14cf55b3220046f3b767b4a4b769e7b32aaf2d2",
    ),
    (
        "encoder.ort",
        94_202_872,
        "a5f11167a62eef61787fe8410453257d6ddb8eba90af461a9604e5f2e93d5322",
    ),
    (
        "frontend.ort",
        47_467_256,
        "378fe8a5d7090a1b9ab88bbb1fc95bde010cdd64ec23419350d2d23c675636e9",
    ),
    (
        "streaming_config.json",
        513,
        "28e83b7a28e91472692a035e0dae3116422ae43aeb2bef5ed822c44ce89b88af",
    ),
    (
        "tokenizer.bin",
        249_974,
        "6884b35fd6377d4c4d32336a0bc152f36b64d1e45b6503683cdc238250a8472d",
    ),
];

/// Wheel-internal paths for the two dylib files.
const DYLIB_PATHS: &[&str] = &[
    "moonshine_voice/libmoonshine.dylib",
    "moonshine_voice/libonnxruntime.1.23.2.dylib",
];

// ---------------------------------------------------------------------------
// Progress reporting helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ProvisionStage {
    Dylibs,
    Model,
}

impl ProvisionStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dylibs => "dylibs",
            Self::Model => "model",
        }
    }
}

fn emit_progress(
    app_handle: &tauri::AppHandle,
    stage: ProvisionStage,
    file_index: u32,
    file_count: u32,
    downloaded_bytes: u64,
    total_bytes: u64,
) {
    let _ = app_handle.emit(
        "moonshine-download-progress",
        serde_json::json!({
            "stage": stage.as_str(),
            "file_index": file_index,
            "file_count": file_count,
            "downloaded_bytes": downloaded_bytes,
            "total_bytes": total_bytes.max(downloaded_bytes),
        }),
    );
}

/// Compute SHA-256 hex of `data` and compare against `expected`.
/// Returns an error (without details of the computed hash) on mismatch.
fn verify_sha256(data: &[u8], expected: &str) -> Result<(), String> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = hex::encode(hasher.finalize());
    if actual != expected {
        Err("SHA-256 mismatch — downloaded file is corrupt or has been tampered with. Retry the download.".into())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn a background thread that downloads Moonshine dylibs + model into
/// `{app_data_dir}/moonshine/`. Progress is reported through the
/// `moonshine-download-progress` Tauri event.
///
/// When provisioning completes, the global `MOONSHINE_BASE` is set, and a
/// `moonshine-provisioned` event is emitted. On failure, a
/// `moonshine-provision-error` event is emitted instead.
///
/// If everything is already installed the function is a no-op.
pub fn ensure_moonshine_installed(app_handle: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("kue-moonshine-provision".into())
        .spawn(move || {
            if let Err(e) = provision_sync(&app_handle) {
                eprintln!("[kue] Moonshine provision error: {e}");
                let _ = app_handle.emit("moonshine-provision-error", &e);
                return;
            }
            let _ = app_handle.emit("moonshine-provisioned", ());
        })
        .expect("failed to spawn moonshine provision thread");
}

/// Synchronous provisioning entry-point.
fn provision_sync(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let base = resolve_base(app_handle)?;

    if is_provisioned(&base) {
        if MOONSHINE_BASE.set(base.clone()).is_err() {
            eprintln!("[kue] MOONSHINE_BASE already set (concurrent provision)");
        }
        eprintln!("[kue] Moonshine already provisioned at {:?}", base);
        return Ok(());
    }

    eprintln!("[kue] Moonshine provisioning started → {:?}", base);

    // --- Dylibs ---
    let lib_dir = base.join("lib");
    let dylib_main = lib_dir.join("libmoonshine.dylib");
    let dylib_onnx = lib_dir.join("libonnxruntime.1.23.2.dylib");

    if !dylib_main.exists() || !dylib_onnx.exists() {
        provision_dylibs(app_handle, &lib_dir)?;
    }

    // --- Model ---
    let model_dir = base.join("models").join("en").join("medium-streaming");
    if !model_dir.join("encoder.ort").exists() {
        provision_model(app_handle, &model_dir)?;
    }

    if MOONSHINE_BASE.set(base.clone()).is_err() {
        eprintln!("[kue] MOONSHINE_BASE already set (concurrent provision)");
    }
    eprintln!("[kue] Moonshine provisioning complete");
    Ok(())
}

/// Construct the managed moonshine path given an app data directory.
/// This is the deterministic helper so tests can verify the path structure
/// without needing a real AppHandle.
pub fn moonshine_base_from_app_data(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("moonshine")
}

/// Compute the managed moonshine directory for this app instance.
pub fn resolve_base(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_data_dir()
        .map(|p| moonshine_base_from_app_data(&p))
        .map_err(|e| format!("app_data_dir: {e}"))
}

/// Returns true when dylibs + at least one model file exist.
pub fn is_provisioned(base: &Path) -> bool {
    let dylib_main = base.join("lib").join("libmoonshine.dylib");
    let dylib_onnx = base.join("lib").join("libonnxruntime.1.23.2.dylib");
    let model_file = base
        .join("models")
        .join("en")
        .join("medium-streaming")
        .join("encoder.ort");

    dylib_main.exists() && dylib_onnx.exists() && model_file.exists()
}

// ---------------------------------------------------------------------------
// Retry command (exposed to frontend)
// ---------------------------------------------------------------------------

/// Tauri command: check whether Moonshine dylibs + model are fully
/// provisioned on disk. Used by the frontend to decide whether to show
/// the download progress UI on first launch.
#[tauri::command]
pub fn is_moonshine_provisioned(app_handle: tauri::AppHandle) -> Result<bool, String> {
    let base = resolve_base(&app_handle)?;
    Ok(is_provisioned(&base))
}

/// Tauri command: retry Moonshine provisioning after a previous failure.
/// Returns immediately — the frontend should listen for
/// `moonshine-provisioned` / `moonshine-provision-error` events to
/// determine the actual outcome.
#[tauri::command]
pub fn retry_moonshine_download(app_handle: tauri::AppHandle) -> Result<String, String> {
    // Remove partial downloads that might be corrupt.
    if let Ok(base) = resolve_base(&app_handle) {
        let lib_dir = base.join("lib");
        let model_dir = base.join("models").join("en").join("medium-streaming");

        let _ = fs::remove_file(lib_dir.join("libmoonshine.dylib"));
        let _ = fs::remove_file(lib_dir.join("libonnxruntime.1.23.2.dylib"));

        for (name, _, _) in MODEL_FILES {
            let _ = fs::remove_file(model_dir.join(name));
        }
    }

    // Re-launch provisioning (same thread-spawning logic).
    // The global MOONSHINE_BASE path is unchanged — only the files are removed
    // and re-downloaded.
    ensure_moonshine_installed(app_handle);
    Ok("retry_initiated".to_string())
}

// ---------------------------------------------------------------------------
// Dylib provisioning (download wheel → extract → verify)
// ---------------------------------------------------------------------------

fn provision_dylibs(app_handle: &tauri::AppHandle, lib_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(lib_dir).map_err(|e| format!("create lib dir {:?}: {e}", lib_dir))?;

    eprintln!("[kue] Downloading moonshine-voice wheel (51.9 MB)...");

    let wheel_bytes = download_file(WHEEL_URL, app_handle, ProvisionStage::Dylibs, 0, 2)?;

    // Verify SHA-256 against the PyPI-published digest before extracting.
    // This is the strongest trust anchor we have for the wheel contents.
    eprintln!("[kue] Verifying wheel SHA-256...");
    verify_sha256(&wheel_bytes, WHEEL_SHA256)
        .map_err(|e| format!("Wheel integrity check failed: {e}"))?;

    eprintln!("[kue] Extracting dylibs from wheel...");
    extract_dylibs(&wheel_bytes, lib_dir)?;

    verify_dylib_sizes_plausible(lib_dir)?;
    eprintln!("[kue] Dylib sizes plausible");

    Ok(())
}

fn download_file(
    url: &str,
    app_handle: &tauri::AppHandle,
    stage: ProvisionStage,
    file_index: u32,
    file_count: u32,
) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let mut resp = client.get(url).send().map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            "No internet connection — check your network and try again.".into()
        } else {
            format!("download failed: {e}")
        }
    })?;

    let total = resp.content_length().unwrap_or(1);
    let mut buf = Vec::with_capacity(total as usize);
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    loop {
        let mut chunk = vec![0u8; 64 * 1024];
        let n = resp.read(&mut chunk).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        downloaded += n as u64;

        if last_emit.elapsed() >= Duration::from_millis(250) {
            emit_progress(app_handle, stage, file_index, file_count, downloaded, total);
            last_emit = std::time::Instant::now();
        }
    }

    if buf.is_empty() {
        return Err("Download yielded 0 bytes — file may have moved. Try again.".into());
    }

    Ok(buf)
}

fn extract_dylibs(wheel_bytes: &[u8], dest: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(wheel_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("corrupt wheel: {e}"))?;

    for target in DYLIB_PATHS {
        let mut file = archive
            .by_name(target)
            .map_err(|e| format!("dylib '{target}' not found in wheel: {e}"))?;

        let filename = Path::new(target)
            .file_name()
            .ok_or_else(|| format!("bad zip entry name: {target}"))?;

        let dest_path = dest.join(filename);
        let mut out =
            fs::File::create(&dest_path).map_err(|e| format!("create {:?}: {e}", dest_path))?;

        io::copy(&mut file, &mut out).map_err(|e| format!("extract {}: {e}", target))?;
    }

    Ok(())
}

/// Verify extracted dylibs have plausible sizes (non-zero, not truncated).
/// Actual loadability is checked later by `try_load_lib()` during engine creation.
fn verify_dylib_sizes_plausible(lib_dir: &Path) -> Result<(), String> {
    let main = lib_dir.join("libmoonshine.dylib");
    let onnx = lib_dir.join("libonnxruntime.1.23.2.dylib");

    let main_size = main
        .metadata()
        .map_err(|e| format!("libmoonshine.dylib missing: {e}"))?
        .len();

    let onnx_size = onnx
        .metadata()
        .map_err(|e| format!("libonnxruntime.*.dylib missing: {e}"))?
        .len();

    // libmoonshine.dylib should be ~27 MB, ONNX should be ~26 MB.
    // Allow for minor size variations.
    if main_size < 20_000_000 {
        return Err(format!(
            "libmoonshine.dylib appears truncated ({main_size} bytes)"
        ));
    }
    if onnx_size < 20_000_000 {
        return Err(format!(
            "libonnxruntime.*.dylib appears truncated ({onnx_size} bytes)"
        ));
    }

    eprintln!(
        "[kue] Dylib sizes verified: main={:.1} MB, onnx={:.1} MB",
        main_size as f64 / 1_048_576.0,
        onnx_size as f64 / 1_048_576.0,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Size validation helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `actual` is within ±10 % of `expected`.
/// Used both for skipping already-downloaded model files and for
/// post-download sanity checks.
fn is_plausible_size(actual: u64, expected: u64) -> bool {
    let min_ok = (expected as f64 * 0.90) as u64;
    let max_ok = (expected as f64 * 1.10) as u64;
    actual >= min_ok && actual <= max_ok
}

/// Returns `true` if a model file already exists at `path` with a plausible
/// size. If the file exists but has the wrong size, removes it so the
/// re-download is clean. Returns `false` if no file or wrong size.
fn check_existing_model_file(path: &Path, expected_size: u64) -> bool {
    if let Ok(meta) = path.metadata() {
        if is_plausible_size(meta.len(), expected_size) {
            return true;
        }
        let _ = fs::remove_file(path);
    }
    false
}

// ---------------------------------------------------------------------------
// Model provisioning
// ---------------------------------------------------------------------------

fn provision_model(app_handle: &tauri::AppHandle, model_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(model_dir).map_err(|e| format!("create model dir {:?}: {e}", model_dir))?;

    let file_count = MODEL_FILES.len() as u32;
    let total_mb: f64 = MODEL_FILES.iter().map(|(_, s, _)| *s as f64).sum::<f64>() / 1_048_576.0;
    eprintln!("[kue] Downloading Moonshine model ({file_count} files, ~{total_mb:.0} MB)...");

    for (idx, (name, expected_size, expected_hash)) in MODEL_FILES.iter().enumerate() {
        let url = format!("{MODEL_BASE}/{name}");
        let dest = model_dir.join(name);

        // Skip files that already exist with plausible size. Also verify
        // SHA-256 to catch corrupt files that happen to be the right size.
        if check_existing_model_file(&dest, *expected_size) {
            let size = dest.metadata().map(|m| m.len()).unwrap_or(0);
            match fs::read(&dest) {
                Ok(data) => match verify_sha256(&data, expected_hash) {
                    Ok(()) => {
                        eprintln!(
                            "[kue]   model file already present: {name} ({size} bytes, SHA-256 OK)"
                        );
                        continue;
                    }
                    Err(e) => {
                        eprintln!("[kue]   model file {name} failed SHA-256, re-downloading: {e}");
                        let _ = fs::remove_file(&dest);
                    }
                },
                Err(e) => {
                    eprintln!("[kue]   model file {name} unreadable ({e}), re-downloading");
                    let _ = fs::remove_file(&dest);
                }
            }
        }

        let data = download_file(
            &url,
            app_handle,
            ProvisionStage::Model,
            idx as u32,
            file_count,
        )?;

        // Size sanity check: ±10 % of expected.
        let actual = data.len() as u64;
        if !is_plausible_size(actual, *expected_size) {
            return Err(format!(
                "Model file {name} size mismatch: got {actual} bytes, expected ~{expected_size}"
            ));
        }

        // SHA-256 verification (pinned at build time — see MODEL_FILES doc).
        verify_sha256(&data, expected_hash)
            .map_err(|e| format!("Model file {name} integrity check failed: {e}"))?;

        fs::write(&dest, &data).map_err(|e| format!("write model file {:?}: {e}", dest))?;

        eprintln!(
            "[kue]   downloaded {name} ({:.1} MB, SHA-256 OK)",
            actual as f64 / 1_048_576.0
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_provisioned_returns_false_for_nonexistent_path() {
        let tmp = std::env::temp_dir().join("kue-test-provision-nonexistent");
        assert!(!is_provisioned(&tmp));
    }

    #[test]
    fn test_is_provisioned_detects_full_state() {
        let base = std::env::temp_dir().join("kue-test-provision-full");
        let lib = base.join("lib");
        let model = base.join("models").join("en").join("medium-streaming");
        let _ = fs::create_dir_all(&lib);
        let _ = fs::create_dir_all(&model);
        let _ = fs::write(lib.join("libmoonshine.dylib"), b"mock");
        let _ = fs::write(lib.join("libonnxruntime.1.23.2.dylib"), b"mock");
        let _ = fs::write(model.join("encoder.ort"), b"mock");

        assert!(is_provisioned(&base));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_is_provisioned_detects_partial_state() {
        let base = std::env::temp_dir().join("kue-test-provision-partial");

        // Empty dir.
        let _ = fs::create_dir_all(&base);
        assert!(!is_provisioned(&base));

        // Only one dylib.
        let lib = base.join("lib");
        let _ = fs::create_dir_all(&lib);
        let _ = fs::write(lib.join("libmoonshine.dylib"), b"mock");
        assert!(!is_provisioned(&base));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_extract_dylibs_rejects_corrupt_wheel() {
        let dest = std::env::temp_dir().join("kue-test-extract-bad");
        let _ = fs::create_dir_all(&dest);
        let result = extract_dylibs(b"not a zip file", &dest);
        assert!(result.is_err(), "corrupt input should produce an error");
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_model_files_list_has_eight_entries() {
        assert_eq!(MODEL_FILES.len(), 8);
    }

    #[test]
    fn test_model_total_size_is_plausible() {
        let total: u64 = MODEL_FILES.iter().map(|(_, s, _)| s).sum();
        // Should be roughly 400-500 MB for the quantized medium streaming model.
        let mb = total as f64 / 1_048_576.0;
        assert!(
            mb > 300.0,
            "model total ({mb:.0} MB) is too small — file sizes may be out of date"
        );
        assert!(
            mb < 600.0,
            "model total ({mb:.0} MB) is too large — verify file sizes"
        );
    }

    #[test]
    fn test_moonshine_base_from_app_data_appends_moonshine() {
        let p = moonshine_base_from_app_data(Path::new("/tmp/kue-data"));
        assert_eq!(p, PathBuf::from("/tmp/kue-data/moonshine"));
    }

    #[test]
    fn test_extract_dylibs_works_with_valid_zip() {
        use std::io::Write;

        let dest = std::env::temp_dir().join("kue-test-extract-valid");
        let _ = fs::create_dir_all(&dest);

        // Build a valid in-memory ZIP with the two expected dylib entries.
        let zip_bytes = {
            let buf = std::io::Cursor::new(Vec::new());
            let mut zip = zip::ZipWriter::new(buf);

            let opts = zip::write::FileOptions::<()>::default()
                .compression_method(zip::CompressionMethod::Stored);

            zip.start_file("moonshine_voice/libmoonshine.dylib", opts)
                .unwrap();
            zip.write_all(b"fake dylib content A").unwrap();

            zip.start_file("moonshine_voice/libonnxruntime.1.23.2.dylib", opts)
                .unwrap();
            zip.write_all(b"fake onnx content B").unwrap();

            zip.finish().unwrap().into_inner()
        };

        let result = extract_dylibs(&zip_bytes, &dest);
        assert!(result.is_ok(), "valid ZIP should extract: {:?}", result);

        let main_path = dest.join("libmoonshine.dylib");
        let onnx_path = dest.join("libonnxruntime.1.23.2.dylib");
        assert!(main_path.exists(), "libmoonshine.dylib should exist");
        assert!(
            onnx_path.exists(),
            "libonnxruntime.1.23.2.dylib should exist"
        );
        assert_eq!(
            fs::read_to_string(&main_path).unwrap(),
            "fake dylib content A"
        );
        assert_eq!(
            fs::read_to_string(&onnx_path).unwrap(),
            "fake onnx content B"
        );

        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_verify_dylib_sizes_plausible_truncation_detection() {
        // verify_dylib_sizes_plausible() checks for files < 20 MB → truncated.
        // Create a temp dir with small files to verify the error path.
        let lib_dir = std::env::temp_dir().join("kue-test-dylib-trunc");
        let _ = fs::create_dir_all(&lib_dir);
        let _ = fs::write(lib_dir.join("libmoonshine.dylib"), b"too small");
        let _ = fs::write(lib_dir.join("libonnxruntime.1.23.2.dylib"), b"also small");

        let result = verify_dylib_sizes_plausible(&lib_dir);
        assert!(
            result.is_err(),
            "tiny dylibs should be rejected as truncated"
        );
        assert!(
            result.unwrap_err().contains("truncated"),
            "error should mention truncation"
        );

        let _ = fs::remove_dir_all(&lib_dir);
    }

    #[test]
    fn test_verify_dylib_sizes_plausible_missing_file_detection() {
        let lib_dir = std::env::temp_dir().join("kue-test-dylib-missing");
        let _ = fs::create_dir_all(&lib_dir);
        // Only write one of the two dylibs.
        let _ = fs::write(lib_dir.join("libmoonshine.dylib"), b"x");

        let result = verify_dylib_sizes_plausible(&lib_dir);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("missing"),
            "should complain about missing file"
        );

        let _ = fs::remove_dir_all(&lib_dir);
    }

    #[test]
    fn test_verify_sha256_accepts_correct_hash() {
        let data = b"hello world";
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(data, expected).is_ok());
    }

    #[test]
    fn test_verify_sha256_rejects_wrong_hash() {
        let data = b"hello world";
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_sha256(data, wrong).is_err());
    }

    #[test]
    fn test_verify_sha256_rejects_tampered_content_same_size() {
        // Same-size content with different content → must reject.
        let original = b"AAAA-BBBB-CCCC-DDDD-EEEE";
        let tampered = b"ZZZZ-YYYY-XXXX-WWWW-VVVV";
        assert_eq!(
            original.len(),
            tampered.len(),
            "test strings must have identical length"
        );

        let hash_original = {
            let mut h = Sha256::new();
            h.update(original);
            hex::encode(h.finalize())
        };

        assert!(
            verify_sha256(tampered, &hash_original).is_err(),
            "same-size tampered content must produce SHA-256 mismatch"
        );
    }

    #[test]
    fn test_is_provisioned_requires_all_three_components() {
        let base = std::env::temp_dir().join("kue-test-all-three");
        let lib = base.join("lib");
        let model = base.join("models").join("en").join("medium-streaming");
        let _ = fs::create_dir_all(&lib);
        let _ = fs::create_dir_all(&model);

        // Missing everything.
        assert!(!is_provisioned(&base));

        // Only dylibs, no model.
        let _ = fs::write(lib.join("libmoonshine.dylib"), b"a");
        let _ = fs::write(lib.join("libonnxruntime.1.23.2.dylib"), b"a");
        assert!(!is_provisioned(&base));

        // All present.
        let _ = fs::write(model.join("encoder.ort"), b"a");
        assert!(is_provisioned(&base));

        let _ = fs::remove_dir_all(&base);
    }

    // -----------------------------------------------------------------------
    // ProvisionStage::as_str
    // -----------------------------------------------------------------------

    #[test]
    fn test_provision_stage_as_str_dylibs() {
        assert_eq!(ProvisionStage::Dylibs.as_str(), "dylibs");
    }

    #[test]
    fn test_provision_stage_as_str_model() {
        assert_eq!(ProvisionStage::Model.as_str(), "model");
    }

    // -----------------------------------------------------------------------
    // is_plausible_size — business logic boundary tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_plausible_size_exact_match() {
        assert!(is_plausible_size(1000, 1000));
    }

    #[test]
    fn test_is_plausible_size_ten_percent_below_accepted() {
        // 90 % of 1000 = 900 → lower boundary, inclusive
        assert!(is_plausible_size(900, 1000));
    }

    #[test]
    fn test_is_plausible_size_ten_percent_above_accepted() {
        // 110 % of 1000 = 1100 → upper boundary, inclusive
        assert!(is_plausible_size(1100, 1000));
    }

    #[test]
    fn test_is_plausible_size_just_below_lower_boundary_rejected() {
        // 89.9 % of 1000 = 899 → just outside lower boundary
        assert!(!is_plausible_size(899, 1000));
    }

    #[test]
    fn test_is_plausible_size_just_above_upper_boundary_rejected() {
        // 110.1 % of 1000 = 1101 → just outside upper boundary
        assert!(!is_plausible_size(1101, 1000));
    }

    #[test]
    fn test_is_plausible_size_zero_expected() {
        // Edge: expected size = 0 → min_ok = 0, max_ok = 0, only 0 is valid.
        assert!(is_plausible_size(0, 0));
        assert!(!is_plausible_size(1, 0));
    }

    #[test]
    fn test_is_plausible_size_actual_zero_when_expected_nonzero() {
        // 0 actual should be below the 90 % threshold for any positive expected.
        assert!(!is_plausible_size(0, 100));
    }

    #[test]
    fn test_is_plausible_size_large_values_no_overflow() {
        // Large file sizes: ~149 MB (typical model file).
        let expected: u64 = 146_275_943;
        assert!(is_plausible_size(expected, expected));
        assert!(is_plausible_size(146_275_943, expected));
        assert!(!is_plausible_size(0, expected));
    }

    #[test]
    fn test_is_plausible_size_uses_f64_casting_not_integer_arithmetic() {
        // Verify that 90 % of 1 rounds to 0 (integer truncation via f64 cast).
        assert!(is_plausible_size(0, 1)); // 0 ≥ 0 (min_ok = 0)
        assert!(!is_plausible_size(2, 1)); // 2 > 1 (max_ok = 1)
    }

    // -----------------------------------------------------------------------
    // check_existing_model_file — filesystem logic
    // -----------------------------------------------------------------------

    #[test]
    fn test_check_existing_model_file_file_missing_returns_false() {
        let tmp = std::env::temp_dir().join("kue-test-check-missing");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("nonexistent.ort");
        assert!(!check_existing_model_file(&path, 1000));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_check_existing_model_file_exact_size_returns_true() {
        let tmp = std::env::temp_dir().join("kue-test-check-exact");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("encoder.ort");
        fs::write(&path, vec![0u8; 1000]).unwrap();
        assert!(check_existing_model_file(&path, 1000));
        // File should still be there.
        assert!(path.exists());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_check_existing_model_file_wrong_size_removes_and_returns_false() {
        let tmp = std::env::temp_dir().join("kue-test-check-wrong-size");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("encoder.ort");
        fs::write(&path, vec![0u8; 10]).unwrap(); // 10 bytes, expected 1000 → way below 90 %
        assert!(!check_existing_model_file(&path, 1000));
        // File should have been removed.
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_check_existing_model_file_boundary_size_keeps_file() {
        let tmp = std::env::temp_dir().join("kue-test-check-boundary");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("encoder.ort");
        // 900 bytes = 90 % of 1000 → lower boundary, should be kept.
        fs::write(&path, vec![0u8; 900]).unwrap();
        assert!(check_existing_model_file(&path, 1000));
        assert!(path.exists());
        // 1100 bytes = 110 % of 1000 → upper boundary, should be kept.
        fs::write(&path, vec![0u8; 1100]).unwrap();
        assert!(check_existing_model_file(&path, 1000));
        assert!(path.exists());
        let _ = fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // extract_dylibs — error path for missing entries
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_dylibs_valid_zip_missing_expected_entry() {
        use std::io::Write;

        let dest = std::env::temp_dir().join("kue-test-extract-missing-entry");
        let _ = fs::create_dir_all(&dest);

        // Build a valid ZIP with only ONE of the two expected entries.
        let zip_bytes = {
            let buf = std::io::Cursor::new(Vec::new());
            let mut zip = zip::ZipWriter::new(buf);

            let opts = zip::write::FileOptions::<()>::default()
                .compression_method(zip::CompressionMethod::Stored);

            zip.start_file("moonshine_voice/libmoonshine.dylib", opts)
                .unwrap();
            zip.write_all(b"fake content").unwrap();
            // Intentionally skip libonnxruntime.1.23.2.dylib

            zip.finish().unwrap().into_inner()
        };

        let result = extract_dylibs(&zip_bytes, &dest);
        assert!(result.is_err(), "missing a DYLIB_PATHS entry should error");
        assert!(
            result.unwrap_err().contains("libonnxruntime"),
            "error should mention the missing dylib"
        );

        let _ = fs::remove_dir_all(&dest);
    }

    // -----------------------------------------------------------------------
    // verify_sha256 — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_verify_sha256_empty_input() {
        let data = b"";
        // SHA-256 of empty string:
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256(data, expected).is_ok());
    }

    #[test]
    fn test_verify_sha256_empty_input_wrong_hash_rejected() {
        let data = b"";
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_sha256(data, wrong).is_err());
    }

    // -----------------------------------------------------------------------
    // verify_dylib_sizes_plausible — success path uses file sizes ≥ 20 MB each.
    // This is intentionally NOT tested as a unit test because creating 40 MB
    // of temp files on every test run is wasteful. The error paths (missing
    // file, truncated file) are already covered above.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Integration-ready tests (commented / documented):
    //
    // The following functions require a `tauri::AppHandle` (concrete Wry
    // runtime) which cannot be constructed in unit tests — `tauri::test::mock_app`
    // returns `AppHandle<MockRuntime>`, and Rust's invariant generic prevents
    // substitution. See `stt::pipeline::tests::mock_app_runtime_type_mismatch_documented`.
    //
    // These functions can only be tested with a real Tauri app:
    //   - resolve_base
    //   - ensure_moonshine_installed
    //   - provision_sync (already-provisioned no-op path)
    //   - provision_dylibs
    //   - provision_model (file-skip optimization path)
    //   - retry_moonshine_download
    //   - download_file
    //   - emit_progress
    //
    // Until the test infrastructure supports injecting a mock runtime, these
    // remain uncovered. Mitigation: the helper functions they depend on
    // (is_plausible_size, check_existing_model_file, is_provisioned,
    //  verify_sha256, extract_dylibs, verify_dylib_sizes_plausible error paths,
    //  moonshine_base_from_app_data) are all fully tested above.
    // -----------------------------------------------------------------------
}
