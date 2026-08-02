use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::Mutex;

use libloading::{Library, Symbol};

use super::STTEngine;

type LoadTranscriber = unsafe extern "C" fn(
    *const std::os::raw::c_char,
    u32,
    *const std::os::raw::c_void,
    u64,
    i32,
) -> i32;

type FreeTranscriber = unsafe extern "C" fn(i32);

type CreateStream = unsafe extern "C" fn(i32, u32) -> i32;

type FreeStream = unsafe extern "C" fn(i32, i32) -> i32;

type StartStream = unsafe extern "C" fn(i32, i32) -> i32;

type StopStream = unsafe extern "C" fn(i32, i32) -> i32;

type AddAudioToStream = unsafe extern "C" fn(i32, i32, *const f32, u64, i32, u32) -> i32;

type TranscribeStream = unsafe extern "C" fn(i32, i32, u32, *mut *mut std::ffi::c_void) -> i32;

const MOONSHINE_HEADER_VERSION: i32 = 20000;
const MOONSHINE_MODEL_ARCH_MEDIUM_STREAMING: u32 = 5;

// ---------------------------------------------------------------------------
// C API struct representations — exact layouts matching moonshine-c-api.h
// ---------------------------------------------------------------------------

#[repr(C)]
struct CWord {
    text: *const std::os::raw::c_char,
    start: f32,
    end: f32,
    confidence: f32,
}

#[repr(C)]
struct CSpeakerSpan {
    start_time: f32,
    duration: f32,
    speaker_id: u64,
    speaker_index: u32,
    start_char: u64,
    end_char: u64,
}

#[repr(C)]
struct CTranscriptLine {
    text: *const std::os::raw::c_char,
    audio_data: *const f32,
    audio_data_count: usize,
    start_time: f32,
    duration: f32,
    id: u64,
    is_complete: i8,
    is_updated: i8,
    is_new: i8,
    has_text_changed: i8,
    have_speakers_changed: i8,
    speaker_spans: *const CSpeakerSpan,
    speaker_span_count: u64,
    last_transcription_latency_ms: u32,
    words: *const CWord,
    word_count: u64,
}

#[repr(C)]
struct CTranscript {
    lines: *mut CTranscriptLine,
    line_count: u64,
}

// ---------------------------------------------------------------------------
// MoonshineFFIEngine
// ---------------------------------------------------------------------------

pub struct MoonshineFFIEngine {
    lib: Option<Library>,
    transcriber_handle: Mutex<Option<i32>>,
    stream_handle: Mutex<Option<i32>>,
    model_path: Option<PathBuf>,
    language: String,
}

impl MoonshineFFIEngine {
    pub fn new() -> Self {
        Self {
            lib: None,
            transcriber_handle: Mutex::new(None),
            stream_handle: Mutex::new(None),
            model_path: None,
            language: "en".to_string(),
        }
    }

    fn try_load_lib() -> Option<Library> {
        let lib_names = ["libmoonshine.dylib", "libmoonshine.so", "moonshine.dll"];

        // Prefer the managed lib dir set by the provisioning module
        // (app_data_dir/moonshine/lib), then fall back to dev paths.
        // DYLD_LIBRARY_PATH is already set in lib.rs setup to include
        // the managed lib dir, so @rpath/libonnxruntime.*.dylib
        // resolution is handled there.
        let mut candidates = Vec::new();
        let managed_dir = super::managed_lib_dir();
        if let Some(ref managed) = managed_dir {
            for name in &lib_names {
                candidates.push(managed.join(name));
            }
        }

        for name in &lib_names {
            candidates.push(PathBuf::from(name));
            if let Ok(dir) = std::env::current_dir() {
                candidates.push(dir.join("lib").join(name));
                candidates.push(dir.join("moonshine-voice").join("lib").join(name));
            }
            if let Some(home) = std::env::var_os("HOME") {
                let home = PathBuf::from(home);
                candidates.push(home.join(".local").join("lib").join(name));
            }
            if let Some(prefix) = std::env::var_os("MOONSHINE_LIB_DIR") {
                let dir = PathBuf::from(prefix);
                candidates.push(dir.join(name));
            }
        }

        for candidate in &candidates {
            if candidate.exists() {
                match unsafe { Library::new(candidate) } {
                    Ok(lib) => {
                        log::info!("Loaded Moonshine library from {:?}", candidate);
                        return Some(lib);
                    }
                    Err(e) => {
                        log::warn!("Failed to load Moonshine lib from {:?}: {e}", candidate);
                    }
                }
            }
        }
        None
    }

    pub fn is_available() -> bool {
        Self::try_load_lib().is_some()
    }
}

// SAFETY: MoonshineFFIEngine is only ever used from one thread at a time.
// The Library is never moved to another thread while in use. All C API calls
// are serialized through the STTEngine trait's single-threaded usage.
unsafe impl Send for MoonshineFFIEngine {}

impl STTEngine for MoonshineFFIEngine {
    fn load(&mut self, model_path: &PathBuf, language: &str) -> Result<(), String> {
        let lib =
            Self::try_load_lib().ok_or_else(|| "Moonshine shared library not found".to_string())?;

        let load_fn: Symbol<LoadTranscriber> = unsafe {
            lib.get(b"moonshine_load_transcriber_from_files")
                .map_err(|e| format!("Failed to find moonshine_load_transcriber_from_files: {e}"))?
        };

        let model_path_c = std::ffi::CString::new(model_path.to_string_lossy().as_ref())
            .map_err(|e| format!("Invalid model path: {e}"))?;

        let handle = unsafe {
            load_fn(
                model_path_c.as_ptr(),
                MOONSHINE_MODEL_ARCH_MEDIUM_STREAMING,
                std::ptr::null(),
                0,
                MOONSHINE_HEADER_VERSION,
            )
        };

        if handle < 0 {
            return Err(format!("Moonshine load model failed (code: {handle})"));
        }

        let create_fn: Symbol<CreateStream> = unsafe {
            lib.get(b"moonshine_create_stream")
                .map_err(|e| format!("Failed to find moonshine_create_stream: {e}"))?
        };

        let stream_handle = unsafe { create_fn(handle, 0) };
        if stream_handle < 0 {
            unsafe {
                let free_fn: Symbol<FreeTranscriber> =
                    lib.get(b"moonshine_free_transcriber").unwrap();
                free_fn(handle);
            }
            return Err(format!(
                "Moonshine create stream failed (code: {stream_handle})"
            ));
        }

        let start_fn: Symbol<StartStream> = unsafe {
            lib.get(b"moonshine_start_stream")
                .map_err(|e| format!("Failed to find moonshine_start_stream: {e}"))?
        };
        let rc = unsafe { start_fn(handle, stream_handle) };
        if rc != 0 {
            unsafe {
                let free_s: Symbol<FreeStream> = lib.get(b"moonshine_free_stream").unwrap();
                free_s(handle, stream_handle);
                let free_t: Symbol<FreeTranscriber> =
                    lib.get(b"moonshine_free_transcriber").unwrap();
                free_t(handle);
            }
            return Err(format!("Moonshine start stream failed (code: {rc})"));
        }

        self.lib = Some(lib);
        *self.transcriber_handle.lock().unwrap() = Some(handle);
        *self.stream_handle.lock().unwrap() = Some(stream_handle);
        self.model_path = Some(model_path.clone());
        self.language = language.to_string();
        Ok(())
    }

    fn transcribe_audio_chunk(&self, chunk: &[i16]) -> Option<String> {
        let (handle, stream) = match (
            *self.transcriber_handle.lock().unwrap(),
            *self.stream_handle.lock().unwrap(),
        ) {
            (Some(h), Some(s)) => (h, s),
            _ => return None,
        };

        let lib = self.lib.as_ref()?;

        let add_audio_fn: Symbol<AddAudioToStream> =
            unsafe { lib.get(b"moonshine_transcribe_add_audio_to_stream").ok()? };

        let transcribe_fn: Symbol<TranscribeStream> =
            unsafe { lib.get(b"moonshine_transcribe_stream").ok()? };

        if chunk.is_empty() {
            return None;
        }

        let f32_samples: Vec<f32> = chunk
            .iter()
            .map(|&s| (s as f32) / (i16::MAX as f32))
            .collect();

        unsafe {
            let rc = add_audio_fn(
                handle,
                stream,
                f32_samples.as_ptr(),
                f32_samples.len() as u64,
                16000,
                0,
            );
            if rc != 0 {
                return None;
            }
        }

        let mut out_transcript: *mut std::ffi::c_void = std::ptr::null_mut();
        let rc = unsafe { transcribe_fn(handle, stream, 0, &mut out_transcript) };

        if rc != 0 || out_transcript.is_null() {
            return None;
        }

        let text = unsafe { parse_transcript(out_transcript as *const CTranscript) };
        text.filter(|t| !t.is_empty())
    }
}

impl Drop for MoonshineFFIEngine {
    fn drop(&mut self) {
        let lib = self.lib.as_ref();
        let handle = *self.transcriber_handle.lock().unwrap();
        let stream = *self.stream_handle.lock().unwrap();
        if let (Some(h), Some(s), Some(lib)) = (handle, stream, lib) {
            unsafe {
                let stop_fn = lib.get::<StopStream>(b"moonshine_stop_stream").ok();
                if let Some(stop_fn) = stop_fn {
                    stop_fn(h, s);
                }
                let free_s: Symbol<FreeStream> = lib.get(b"moonshine_free_stream").unwrap();
                free_s(h, s);
                let free_t: Symbol<FreeTranscriber> =
                    lib.get(b"moonshine_free_transcriber").unwrap();
                free_t(h);
            }
        }
    }
}

/// Parse a C `transcript_t` and return the text of the last completed line.
///
/// Uses proper `#[repr(C)]` struct definitions matching the Moonshine C API
/// layout, so no hardcoded byte offsets are needed. Returns `None` if there
/// are no completed non-empty lines.
///
/// # Safety
///
/// `ptr` must point to a valid `transcript_t` struct whose lifetime is
/// managed by the Moonshine library (valid until the next call to the
/// same transcriber).
unsafe fn parse_transcript(ptr: *const CTranscript) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    let tc = &*ptr;
    if tc.line_count == 0 || tc.lines.is_null() {
        return None;
    }

    // Iterate from the end to find the last completed line with text
    let lines = std::slice::from_raw_parts(tc.lines, tc.line_count as usize);
    for line in lines.iter().rev() {
        if line.is_complete == 0 || line.text.is_null() {
            continue;
        }
        let c_str = CStr::from_ptr(line.text);
        if let Ok(s) = c_str.to_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // MoonshineFFIEngine construction
    // -----------------------------------------------------------------------

    #[test]
    fn moonshine_ffi_engine_new_sets_defaults() {
        let engine = MoonshineFFIEngine::new();
        assert!(engine.lib.is_none());
        assert!(engine.transcriber_handle.lock().unwrap().is_none());
        assert!(engine.stream_handle.lock().unwrap().is_none());
        assert!(engine.model_path.is_none());
        assert_eq!(engine.language, "en");
    }

    #[test]
    fn moonshine_ffi_engine_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MoonshineFFIEngine>();
    }

    // -----------------------------------------------------------------------
    // parse_transcript — edge cases (additional)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_transcript_null_ptr() {
        let result = unsafe { parse_transcript(std::ptr::null()) };
        assert!(result.is_none());
    }

    #[test]
    fn parse_transcript_zero_lines() {
        let ct = CTranscript {
            lines: std::ptr::null_mut(),
            line_count: 0,
        };
        assert!(unsafe { parse_transcript(&ct).is_none() });
    }

    #[test]
    fn parse_transcript_null_lines_ptr() {
        let ct = CTranscript {
            lines: std::ptr::null_mut(),
            line_count: 5,
        };
        assert!(unsafe { parse_transcript(&ct).is_none() });
    }

    #[test]
    fn parse_transcript_single_completed_line() {
        let text = std::ffi::CString::new("hello world").unwrap();
        let line = CTranscriptLine {
            text: text.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 0.0,
            duration: 1.0,
            id: 1,
            is_complete: 1,
            is_updated: 0,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let ct = CTranscript {
            lines: &line as *const _ as *mut _,
            line_count: 1,
        };
        let result = unsafe { parse_transcript(&ct) };
        assert_eq!(result.as_deref(), Some("hello world"));
    }

    #[test]
    fn parse_transcript_skips_incomplete_lines() {
        let text1 = std::ffi::CString::new("incomplete").unwrap();
        let line1 = CTranscriptLine {
            text: text1.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 0.0,
            duration: 1.0,
            id: 1,
            is_complete: 0,
            is_updated: 0,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let text2 = std::ffi::CString::new("completed").unwrap();
        let line2 = CTranscriptLine {
            text: text2.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 1.0,
            duration: 1.0,
            id: 2,
            is_complete: 1,
            is_updated: 1,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let lines = [line1, line2];
        let ct = CTranscript {
            lines: &lines as *const _ as *mut _,
            line_count: 2,
        };
        let result = unsafe { parse_transcript(&ct) };
        assert_eq!(result.as_deref(), Some("completed"));
    }

    #[test]
    fn parse_transcript_skips_empty_text() {
        let text = std::ffi::CString::new("").unwrap();
        let line = CTranscriptLine {
            text: text.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 0.0,
            duration: 1.0,
            id: 1,
            is_complete: 1,
            is_updated: 0,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let ct = CTranscript {
            lines: &line as *const _ as *mut _,
            line_count: 1,
        };
        assert!(unsafe { parse_transcript(&ct).is_none() });
    }

    #[test]
    fn parse_transcript_prefers_last_completed() {
        let text1 = std::ffi::CString::new("first").unwrap();
        let text2 = std::ffi::CString::new("second").unwrap();
        let line1 = CTranscriptLine {
            text: text1.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 0.0,
            duration: 1.0,
            id: 1,
            is_complete: 1,
            is_updated: 1,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let line2 = CTranscriptLine {
            text: text2.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 1.0,
            duration: 1.0,
            id: 2,
            is_complete: 1,
            is_updated: 1,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let lines = [line1, line2];
        let ct = CTranscript {
            lines: &lines as *const _ as *mut _,
            line_count: 2,
        };
        let result = unsafe { parse_transcript(&ct) };
        assert_eq!(result.as_deref(), Some("second"));
    }

    #[test]
    fn parse_transcript_all_incomplete() {
        let text = std::ffi::CString::new("ongoing").unwrap();
        let line = CTranscriptLine {
            text: text.as_ptr(),
            audio_data: std::ptr::null(),
            audio_data_count: 0,
            start_time: 0.0,
            duration: 1.0,
            id: 1,
            is_complete: 0,
            is_updated: 0,
            is_new: 0,
            has_text_changed: 0,
            have_speakers_changed: 0,
            speaker_spans: std::ptr::null(),
            speaker_span_count: 0,
            last_transcription_latency_ms: 0,
            words: std::ptr::null(),
            word_count: 0,
        };
        let ct = CTranscript {
            lines: &line as *const _ as *mut _,
            line_count: 1,
        };
        assert!(unsafe { parse_transcript(&ct).is_none() });
    }

    #[test]
    fn c_struct_size_is_plausible() {
        // Verify our Rust struct sizes are reasonable for 64-bit macOS.
        // transcript_t should be 16 bytes: pointer + u64
        assert_eq!(std::mem::size_of::<CTranscript>(), 16);
        // transcript_line_t should be at least 40 bytes: 5*8 + 4+4 + 1*5 + pad
        assert!(std::mem::size_of::<CTranscriptLine>() >= 80);
    }
}
