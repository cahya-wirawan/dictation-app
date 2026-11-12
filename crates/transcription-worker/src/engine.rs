//! Safe wrapper around the C shim. One context, one job at a time.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_void};
use std::fs::File;
use std::io::Read;
use std::ptr::NonNull;

#[repr(C)]
struct WhisperContext {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn dw_silence_logs();
    fn dw_init_from_loader(
        ctx: *mut c_void,
        read: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> usize,
        eof: unsafe extern "C" fn(*mut c_void) -> bool,
        use_gpu: c_int,
    ) -> *mut WhisperContext;
    fn dw_transcribe(
        ctx: *mut WhisperContext,
        pcm: *const c_float,
        n_samples: c_int,
        language: *const c_char,
        n_threads: c_int,
    ) -> c_int;
    fn whisper_free(ctx: *mut WhisperContext);
    fn whisper_full_n_segments(ctx: *mut WhisperContext) -> c_int;
    fn whisper_full_get_segment_text(ctx: *mut WhisperContext, i_segment: c_int) -> *const c_char;
    fn whisper_full_lang_id(ctx: *mut WhisperContext) -> c_int;
    fn whisper_lang_str(id: c_int) -> *const c_char;
    fn whisper_lang_id(lang: *const c_char) -> c_int;
}

pub fn silence_logs() {
    // SAFETY: installs a static no-op callback; no arguments.
    unsafe { dw_silence_logs() }
}

/// Owned whisper.cpp context (RAII). Not Sync: used by the single job loop.
pub struct Engine {
    ctx: NonNull<WhisperContext>,
}

pub struct RawTranscript {
    pub segments: Vec<String>,
    pub language: Option<String>,
}

impl Engine {
    /// Loads from `reader`, which hashes every byte whisper.cpp consumes.
    /// The caller must verify the digest before using the engine.
    pub fn load_from(reader: &mut HashingReader, use_gpu: bool) -> Option<Self> {
        let ctx_ptr = (reader as *mut HashingReader).cast::<c_void>();
        // SAFETY: `reader` outlives the call; the callbacks only access it
        // through this pointer, and whisper.cpp does not retain the loader.
        let ctx = unsafe { dw_init_from_loader(ctx_ptr, read_cb, eof_cb, c_int::from(use_gpu)) };
        NonNull::new(ctx).map(|ctx| Self { ctx })
    }

    pub fn language_supported(lang: &str) -> bool {
        if lang == "auto" {
            return true;
        }
        let Ok(c) = CString::new(lang) else {
            return false;
        };
        // SAFETY: valid C string; pure lookup.
        unsafe { whisper_lang_id(c.as_ptr()) >= 0 }
    }

    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        language: &str,
        threads: usize,
        max_segments: u32,
    ) -> Result<RawTranscript, String> {
        let n = c_int::try_from(pcm.len()).map_err(|_| "too many samples".to_string())?;
        let lang = CString::new(language).map_err(|_| "bad language".to_string())?;
        let threads = c_int::try_from(threads.max(1)).unwrap_or(1);
        // SAFETY: ctx is live and exclusively borrowed; pcm outlives the call.
        let rc =
            unsafe { dw_transcribe(self.ctx.as_ptr(), pcm.as_ptr(), n, lang.as_ptr(), threads) };
        if rc != 0 {
            return Err(format!("whisper_full returned {rc}"));
        }
        // SAFETY: valid ctx after a successful whisper_full.
        let n_seg = unsafe { whisper_full_n_segments(self.ctx.as_ptr()) };
        if n_seg < 0 || n_seg as u32 > max_segments {
            return Err(format!("segment count {n_seg} out of bounds"));
        }
        let mut segments = Vec::with_capacity(n_seg as usize);
        for i in 0..n_seg {
            // SAFETY: i in range; returned pointer is owned by ctx and valid until next call.
            let p = unsafe { whisper_full_get_segment_text(self.ctx.as_ptr(), i) };
            if !p.is_null() {
                // SAFETY: whisper returns NUL-terminated UTF-8 (possibly split mid-codepoint; lossy is fine).
                segments.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
            }
        }
        // SAFETY: as above.
        let id = unsafe { whisper_full_lang_id(self.ctx.as_ptr()) };
        let language = if id >= 0 {
            // SAFETY: id from whisper; returns static string or null.
            let p = unsafe { whisper_lang_str(id) };
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        } else {
            None
        };
        Ok(RawTranscript { segments, language })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: ctx came from whisper_init and is freed exactly once.
        unsafe { whisper_free(self.ctx.as_ptr()) }
    }
}

/// Reads one already-open file for whisper.cpp while hashing it.
pub struct HashingReader {
    file: File,
    hasher: sha2::Sha256,
    pub total: u64,
    eof: bool,
    pub failed: bool,
}

impl HashingReader {
    pub fn new(file: File) -> Self {
        use sha2::Digest;
        Self {
            file,
            hasher: sha2::Sha256::new(),
            total: 0,
            eof: false,
            failed: false,
        }
    }

    /// Hashes whatever whisper.cpp did not read, then returns the digest.
    pub fn finish(mut self) -> Option<String> {
        use sha2::Digest;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.hasher.update(&buf[..n]);
                    self.total += n as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
        if self.failed {
            return None;
        }
        Some(
            self.hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        )
    }
}

unsafe extern "C" fn read_cb(ctx: *mut c_void, out: *mut c_void, n: usize) -> usize {
    use sha2::Digest;
    // SAFETY: ctx is the HashingReader passed to load_from; out has n bytes.
    let r = unsafe { &mut *ctx.cast::<HashingReader>() };
    let buf = unsafe { std::slice::from_raw_parts_mut(out.cast::<u8>(), n) };
    let mut filled = 0;
    while filled < n {
        match r.file.read(&mut buf[filled..]) {
            Ok(0) => {
                r.eof = true;
                break;
            }
            Ok(k) => filled += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                r.failed = true;
                r.eof = true;
                break;
            }
        }
    }
    r.hasher.update(&buf[..filled]);
    r.total += filled as u64;
    filled
}

unsafe extern "C" fn eof_cb(ctx: *mut c_void) -> bool {
    // SAFETY: as in read_cb.
    unsafe { (*ctx.cast::<HashingReader>()).eof }
}

/// Strip control characters except tab/newline and trim outer whitespace.
/// Diacritics and all other text are preserved (docs/03 §5).
pub fn sanitize(segments: &[String]) -> String {
    let joined: String = segments.concat();
    let cleaned: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\t' || *c == '\n')
        .collect();
    cleaned.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitize_keeps_unicode_and_strips_controls() {
        let s = sanitize(&[
            " Grüß\u{0007} Gott,".into(),
            " selamat pagi 👋\u{001b}[0m\n".into(),
        ]);
        assert_eq!(s, "Grüß Gott, selamat pagi 👋[0m");
    }
}
