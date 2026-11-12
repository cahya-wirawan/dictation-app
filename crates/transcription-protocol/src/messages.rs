use dictation_domain::{Generation, SessionId, WorkerEpoch};
use serde::{Deserialize, Serialize};

use crate::{MAX_SAMPLES, MAX_TEXT_BYTES};

/// Identity of one inference job. Every result must echo it exactly so the
/// coordinator can fence stale results (docs/04 §2 invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobId {
    pub session_id: SessionId,
    pub generation: Generation,
    pub worker_epoch: WorkerEpoch,
}

/// `"auto"` or a lowercase ISO-639 code (2–3 letters) that whisper.cpp knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Language(String);

impl Language {
    pub fn auto() -> Self {
        Self("auto".into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Language {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let ok = s == "auto"
            || ((2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase()));
        if ok {
            Ok(Self(s))
        } else {
            Err("language must be \"auto\" or a 2-3 letter lowercase code".into())
        }
    }
}

impl From<Language> for String {
    fn from(l: Language) -> String {
        l.0
    }
}

/// Parent → worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ToWorker {
    /// Path is resolved by the parent alone; the worker re-verifies the hash.
    LoadModel {
        path: String,
        sha256: String,
    },
    /// Followed immediately by `pcm_bytes` bytes of LE f32 PCM.
    Transcribe {
        job: JobId,
        language: Language,
        sample_count: u32,
        pcm_bytes: u32,
    },
    Shutdown {},
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum FromWorker {
    Hello {
        protocol: u32,
        engine: String,
        engine_revision: String,
        backends: Vec<String>,
    },
    ModelReady {
        sha256: String,
        load_ms: u64,
    },
    Result(TranscriptionResult),
    Error {
        job: Option<JobId>,
        code: WorkerErrorCode,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptionResult {
    pub job: JobId,
    pub text: String,
    pub language: Option<String>,
    pub audio_ms: u64,
    pub inference_ms: u64,
    pub segments: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerErrorCode {
    ProtocolViolation,
    ModelNotLoaded,
    ModelInvalid,
    ModelLoadFailed,
    InvalidAudio,
    InferenceFailed,
    OutputTooLarge,
}

impl ToWorker {
    /// Semantic validation beyond JSON shape. Both sides call it.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ToWorker::LoadModel { path, sha256 } => {
                if !std::path::Path::new(path).is_absolute() {
                    return Err("model path must be absolute".into());
                }
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err("sha256 must be 64 lowercase hex chars".into());
                }
                Ok(())
            }
            ToWorker::Transcribe {
                sample_count,
                pcm_bytes,
                ..
            } => {
                if *sample_count == 0 || *sample_count > MAX_SAMPLES {
                    return Err(format!("sample_count must be 1..={MAX_SAMPLES}"));
                }
                if u64::from(*pcm_bytes) != u64::from(*sample_count) * 4 {
                    return Err("pcm_bytes must equal sample_count * 4".into());
                }
                Ok(())
            }
            ToWorker::Shutdown {} => Ok(()),
        }
    }
}

impl TranscriptionResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err("text exceeds MAX_TEXT_BYTES".into());
        }
        Ok(())
    }
}
