//! `dictation-worker`: one process, one loaded model, one job at a time.
//! Speaks transcription-protocol v1 on stdin/stdout; exits on stdin EOF
//! (parent liveness). stdout carries protocol frames only. Transcript text is
//! never written to stderr.

mod engine;

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use transcription_protocol::*;

use engine::{Engine, HashingReader, sanitize};

/// Generous upper bound for any approved Whisper model file.
const MAX_MODEL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn main() -> ExitCode {
    engine::silence_logs();
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match run(
        BufReader::with_capacity(64 * 1024, stdin),
        BufWriter::new(stdout),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Diagnostic only; never contains transcript text or PCM.
            eprintln!("dictation-worker: fatal: {e}");
            ExitCode::from(2)
        }
    }
}

fn run<R: Read, W: Write>(mut r: R, mut w: W) -> Result<(), FrameError> {
    let mut backends = vec!["cpu".to_string()];
    if cfg!(all(feature = "metal", target_os = "macos")) {
        backends.push("metal".into());
    }
    write_frame(
        &mut w,
        &FromWorker::Hello {
            protocol: PROTOCOL_VERSION,
            engine: "whisper.cpp".into(),
            engine_revision: format!(
                "{} ({})",
                env!("WHISPER_CPP_TAG"),
                env!("WHISPER_CPP_REVISION")
            ),
            backends,
        },
    )?;

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let mut model: Option<(Engine, String)> = None;

    loop {
        let msg: ToWorker = match read_frame(&mut r) {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(()), // parent gone
            Err(e) => {
                // Stream is no longer synchronized; report once and exit.
                let _ = write_frame(
                    &mut w,
                    &FromWorker::Error {
                        job: None,
                        code: WorkerErrorCode::ProtocolViolation,
                        detail: e.to_string(),
                    },
                );
                return Err(e);
            }
        };
        if let Err(detail) = msg.validate() {
            let _ = write_frame(
                &mut w,
                &FromWorker::Error {
                    job: None,
                    code: WorkerErrorCode::ProtocolViolation,
                    detail: detail.clone(),
                },
            );
            return Err(FrameError::Invalid(detail));
        }
        match msg {
            ToWorker::Shutdown {} => return Ok(()),
            ToWorker::LoadModel { path, sha256 } => {
                let started = Instant::now();
                match load_verified(Path::new(&path), &sha256) {
                    Ok(engine) => {
                        model = Some((engine, sha256.clone())); // drops any previous context
                        let load_ms = started.elapsed().as_millis() as u64;
                        write_frame(&mut w, &FromWorker::ModelReady { sha256, load_ms })?;
                    }
                    Err((code, detail)) => write_frame(
                        &mut w,
                        &FromWorker::Error {
                            job: None,
                            code,
                            detail,
                        },
                    )?,
                }
            }
            ToWorker::Transcribe {
                job,
                language,
                sample_count,
                ..
            } => {
                // Always consume the full payload so the stream stays framed.
                let pcm = read_pcm(&mut r, sample_count)?;
                let reply = match model.as_mut() {
                    None => err(job, WorkerErrorCode::ModelNotLoaded, "no model loaded"),
                    Some(_) if !Engine::language_supported(language.as_str()) => {
                        err(job, WorkerErrorCode::InvalidAudio, "unsupported language")
                    }
                    Some((engine, _)) => {
                        let started = Instant::now();
                        match engine.transcribe(&pcm, language.as_str(), threads, MAX_SEGMENTS) {
                            Err(detail) => err(job, WorkerErrorCode::InferenceFailed, &detail),
                            Ok(raw) => {
                                let text = sanitize(&raw.segments);
                                if text.len() > MAX_TEXT_BYTES {
                                    err(
                                        job,
                                        WorkerErrorCode::OutputTooLarge,
                                        "transcript exceeds limit",
                                    )
                                } else {
                                    FromWorker::Result(TranscriptionResult {
                                        job,
                                        text,
                                        language: raw.language,
                                        audio_ms: u64::from(sample_count) * 1000
                                            / u64::from(SAMPLE_RATE_HZ),
                                        inference_ms: started.elapsed().as_millis() as u64,
                                        segments: raw.segments.len() as u32,
                                    })
                                }
                            }
                        }
                    }
                };
                drop(pcm); // no cache of audio after the job
                write_frame(&mut w, &reply)?;
            }
        }
    }
}

fn err(job: JobId, code: WorkerErrorCode, detail: &str) -> FromWorker {
    FromWorker::Error {
        job: Some(job),
        code,
        detail: detail.into(),
    }
}

/// Opens the model once (refusing symlinks), lets whisper.cpp load from that
/// descriptor while hashing every byte, and accepts the context only if the
/// size and SHA-256 match. No path is reopened, so there is no window to swap
/// the file between verification and load.
fn load_verified(path: &Path, expected: &str) -> Result<Engine, (WorkerErrorCode, String)> {
    let invalid = |d: &str| (WorkerErrorCode::ModelInvalid, d.to_string());
    let file = open_no_follow(path).map_err(|_| invalid("model not readable (or a symlink)"))?;
    let meta = file.metadata().map_err(|_| invalid("model not readable"))?;
    if !meta.file_type().is_file() {
        return Err(invalid("model path is not a regular file"));
    }
    if meta.len() == 0 || meta.len() > MAX_MODEL_BYTES {
        return Err(invalid("model size out of range"));
    }
    let mut reader = HashingReader::new(file);
    let engine = Engine::load_from(
        &mut reader,
        cfg!(all(feature = "metal", target_os = "macos")),
    );
    let digest = reader.finish();
    match (engine, digest) {
        (_, None) => Err(invalid("model read failed")),
        (_, Some(d)) if d != expected => Err(invalid("model hash mismatch")), // engine dropped here
        (None, Some(_)) => Err((
            WorkerErrorCode::ModelLoadFailed,
            "whisper.cpp could not load model".into(),
        )),
        (Some(e), Some(_)) => Ok(e),
    }
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    File::open(path)
}
