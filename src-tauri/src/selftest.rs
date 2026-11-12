//! M0 T0.7 in-app worker self-test: spawns the bundled worker (sibling of the
//! app executable, never PATH), loads the approved model, transcribes the
//! licensed JFK fixture (public domain) and reports timings.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dictation_domain::SessionId;
use serde::Serialize;
use transcription_protocol::client::WorkerClient;
use transcription_protocol::{JobId, Language};

/// Approved model manifest entry (docs/03 §6). Values recorded in M0.
pub struct ApprovedModel {
    pub id: &'static str,
    pub file_name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub url: &'static str,
}

pub const BASE_MULTILINGUAL: ApprovedModel = ApprovedModel {
    id: "whisper-base-multilingual",
    file_name: "ggml-base.bin",
    bytes: 147_951_465,
    sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
};

const FIXTURE_WAV: &[u8] = include_bytes!("../../vendor/whisper.cpp/samples/jfk.wav");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelftestReport {
    pub worker_path: String,
    pub engine_revision: String,
    pub backends: Vec<String>,
    pub model_id: String,
    pub spawn_ms: u64,
    pub load_ms: u64,
    pub inference_ms: Vec<u64>,
    pub audio_ms: u64,
    pub language: Option<String>,
    /// Fixed licensed fixture text, not user speech.
    pub fixture_text: String,
    pub worker_reaped: bool,
}

pub fn worker_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "dictation-worker.exe"
    } else {
        "dictation-worker"
    };
    let p = exe.parent()?.join(name);
    p.is_file().then_some(p)
}

pub fn model_path(app_data_dir: &Path) -> PathBuf {
    // Development builds may point at a repo-local download.
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("DICTATION_MODEL_PATH") {
        return PathBuf::from(p);
    }
    app_data_dir
        .join("models")
        .join(BASE_MULTILINGUAL.file_name)
}

fn fixture_pcm() -> Result<Vec<f32>, String> {
    let r = hound::WavReader::new(std::io::Cursor::new(FIXTURE_WAV)).map_err(|e| e.to_string())?;
    r.into_samples::<i16>()
        .map(|s| s.map(|s| f32::from(s) / 32768.0).map_err(|e| e.to_string()))
        .collect()
}

pub fn run(model: &Path) -> Result<SelftestReport, String> {
    let exe =
        worker_path().ok_or("bundled dictation-worker not found next to the app executable")?;
    let meta = std::fs::metadata(model)
        .map_err(|_| format!("model not installed at {}", model.display()))?;
    if meta.len() != BASE_MULTILINGUAL.bytes {
        return Err("model size does not match the approved manifest".into());
    }
    let pcm = fixture_pcm()?;

    let t = Instant::now();
    let mut client =
        WorkerClient::spawn(&exe, 1, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let spawn_ms = t.elapsed().as_millis() as u64;
    let load_ms = client
        .load_model(model, BASE_MULTILINGUAL.sha256, Duration::from_secs(60))
        .map_err(|e| e.to_string())?;

    let mut inference_ms = vec![];
    let mut last = None;
    for generation in 0..3 {
        let job = JobId {
            session_id: SessionId::random().map_err(|e| e.to_string())?,
            generation,
            worker_epoch: 1,
        };
        let r = client
            .transcribe(job, Language::auto(), &pcm, Duration::from_secs(60))
            .map_err(|e| e.to_string())?;
        inference_ms.push(r.inference_ms);
        last = Some(r);
    }
    let last = last.ok_or("no result")?;
    let info = client.info().clone();
    let pid = client.pid();
    let status = client.shutdown(Duration::from_secs(2));
    Ok(SelftestReport {
        worker_path: exe.display().to_string(),
        engine_revision: info.engine_revision,
        backends: info.backends,
        model_id: BASE_MULTILINGUAL.id.into(),
        spawn_ms,
        load_ms,
        inference_ms,
        audio_ms: last.audio_ms,
        language: last.language,
        fixture_text: last.text,
        worker_reaped: status.is_some() && pid.is_some(),
    })
}
