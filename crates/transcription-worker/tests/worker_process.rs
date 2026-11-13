//! Real-process tests for dictation-worker (work packages B1–B3, M0 T0.7).
//!
//! Model-dependent tests use `.models/ggml-base.bin` (or DICTATION_TEST_MODEL /
//! DICTATION_TEST_MODEL_SHA256) and are skipped with a message if absent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use dictation_domain::SessionId;
use transcription_protocol::client::{ClientError, WorkerClient};
use transcription_protocol::*;

const BASE_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const T: Duration = Duration::from_secs(10);
/// Per-job limit for real transcriptions. On the GitHub Windows runner the
/// first job of each debug-build worker process has taken 57–72 s (later
/// jobs ~1.3 s); see the M7 findings in docs/IMPLEMENTATION_STATUS.md.
const JOB: Duration = Duration::from_secs(150);

fn worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_dictation-worker"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn model() -> Option<(PathBuf, String)> {
    let path = std::env::var_os("DICTATION_TEST_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(".models/ggml-base.bin"));
    let sha = std::env::var("DICTATION_TEST_MODEL_SHA256").unwrap_or_else(|_| BASE_SHA256.into());
    if path.is_file() {
        Some((path, sha))
    } else {
        eprintln!(
            "SKIP: model not found at {} (download ggml-base.bin, see IMPLEMENTATION_STATUS.md)",
            path.display()
        );
        None
    }
}

/// JFK inaugural excerpt shipped with whisper.cpp (US government work, public domain).
fn jfk_pcm() -> Vec<f32> {
    let mut r =
        hound::WavReader::open(repo_root().join("vendor/whisper.cpp/samples/jfk.wav")).unwrap();
    let spec = r.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (16_000, 1, 16)
    );
    r.samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

fn job(epoch: u64, generation: u64) -> JobId {
    JobId {
        session_id: SessionId::random().unwrap(),
        generation,
        worker_epoch: epoch,
    }
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn process_alive(pid: u32) -> bool {
    // Linux/macOS: zombie-free check via /proc where available, else `kill -0`.
    if Path::new("/proc").is_dir() {
        return Path::new(&format!("/proc/{pid}")).exists();
    }
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn handshake_reports_pinned_engine() {
    let c = WorkerClient::spawn(&worker_exe(), 1, T).unwrap();
    assert_eq!(c.info().engine, "whisper.cpp");
    assert!(
        c.info().engine_revision.contains("v1.9.5"),
        "{}",
        c.info().engine_revision
    );
    assert!(c.info().backends.iter().any(|b| b == "cpu"));
    let pid = c.pid().unwrap();
    c.shutdown(Duration::from_secs(2)).unwrap();
    assert!(!process_alive(pid));
}

#[test]
fn exits_on_stdin_eof() {
    let mut child = Command::new(worker_exe())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let started = Instant::now();
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn malformed_length_is_rejected_and_worker_exits() {
    let mut child = Command::new(worker_exe())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let hello: FromWorker = read_frame(&mut stdout).unwrap().unwrap();
    assert!(matches!(hello, FromWorker::Hello { .. }));
    stdin.write_all(&u32::MAX.to_le_bytes()).unwrap();
    stdin.flush().unwrap();
    let reply: FromWorker = read_frame(&mut stdout).unwrap().unwrap();
    assert!(
        matches!(
            reply,
            FromWorker::Error {
                code: WorkerErrorCode::ProtocolViolation,
                ..
            }
        ),
        "{reply:?}"
    );
    assert_eq!(child.wait().unwrap().code(), Some(2));
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(err.len() < 1024);
}

#[test]
fn transcribe_without_model_is_typed_error() {
    let mut c = WorkerClient::spawn(&worker_exe(), 4, T).unwrap();
    let r = c.transcribe(job(4, 1), Language::auto(), &[0.0; 1600], T);
    assert!(
        matches!(
            r,
            Err(ClientError::Worker {
                code: WorkerErrorCode::ModelNotLoaded,
                ..
            })
        ),
        "{r:?}"
    );
    // Stream stays framed: a second request still gets a typed reply.
    let r = c.transcribe(job(4, 2), Language::auto(), &[0.0; 1600], T);
    assert!(matches!(
        r,
        Err(ClientError::Worker {
            code: WorkerErrorCode::ModelNotLoaded,
            ..
        })
    ));
}

#[test]
fn client_rejects_wrong_epoch_and_bad_pcm() {
    let mut c = WorkerClient::spawn(&worker_exe(), 5, T).unwrap();
    assert!(matches!(
        c.transcribe(job(6, 1), Language::auto(), &[0.0; 16], T),
        Err(ClientError::InvalidRequest(_))
    ));
    assert!(matches!(
        c.transcribe(job(5, 1), Language::auto(), &[f32::NAN], T),
        Err(ClientError::InvalidRequest(_))
    ));
    assert!(matches!(
        c.transcribe(job(5, 1), Language::auto(), &[], T),
        Err(ClientError::InvalidRequest(_))
    ));
}

#[test]
fn tampered_hash_never_loads() {
    let Some((path, _)) = model() else { return };
    let mut c = WorkerClient::spawn(&worker_exe(), 7, T).unwrap();
    let r = c.load_model(&path, &"0".repeat(64), Duration::from_secs(30));
    assert!(
        matches!(
            r,
            Err(ClientError::Worker {
                code: WorkerErrorCode::ModelInvalid,
                ..
            })
        ),
        "{r:?}"
    );
    assert!(!c.model_loaded());
}

#[test]
fn symlinked_model_is_rejected() {
    let Some((path, sha)) = model() else { return };
    let dir = std::env::temp_dir().join(format!("dw-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("model.bin");
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&path, &link).unwrap();
    // Needs admin or Developer Mode on Windows (CI runners have it).
    #[cfg(windows)]
    if let Err(e) = std::os::windows::fs::symlink_file(&path, &link) {
        eprintln!("SKIP: cannot create a symlink here: {e}");
        return;
    }
    let mut c = WorkerClient::spawn(&worker_exe(), 8, T).unwrap();
    let r = c.load_model(&link, &sha, Duration::from_secs(30));
    assert!(
        matches!(
            r,
            Err(ClientError::Worker {
                code: WorkerErrorCode::ModelInvalid,
                ..
            })
        ),
        "{r:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn jfk_fixture_transcribes_offline_on_cpu() {
    let Some((path, sha)) = model() else { return };
    let pcm = jfk_pcm();
    let mut c = WorkerClient::spawn(&worker_exe(), 10, T).unwrap();
    let load_ms = c.load_model(&path, &sha, Duration::from_secs(60)).unwrap();

    let mut timings = Vec::new();
    let mut texts = Vec::new();
    for g in 0..3 {
        let j = job(10, g);
        let started = Instant::now();
        let r = c
            .transcribe(j, Language::try_from("en".to_string()).unwrap(), &pcm, JOB)
            .unwrap();
        timings.push(started.elapsed());
        assert_eq!(r.job, j);
        assert_eq!(r.audio_ms, 11_000);
        texts.push(r.text);
    }
    let auto = c
        .transcribe(job(10, 9), Language::auto(), &pcm, JOB)
        .unwrap();

    let norm = normalize(&texts[0]);
    assert!(
        norm.contains("ask not what your country can do for you"),
        "{norm}"
    );
    assert!(
        norm.contains("ask what you can do for your country"),
        "{norm}"
    );
    assert!(
        texts.iter().all(|t| t == &texts[0]),
        "greedy decoding must be deterministic: {texts:?}"
    );
    assert_eq!(auto.language.as_deref(), Some("en"));
    eprintln!(
        "M0 T0.7: model load {load_ms} ms; warm 11 s fixture: {:?}; auto-language={:?}; text={:?}",
        timings, auto.language, texts[0]
    );
}

#[test]
fn hard_cancel_kills_reaps_and_restarts() {
    let Some((path, sha)) = model() else { return };
    // 30 s of audio so inference is still running when we cancel.
    let pcm: Vec<f32> = jfk_pcm().into_iter().cycle().take(16_000 * 30).collect();
    let mut c = WorkerClient::spawn(&worker_exe(), 20, T).unwrap();
    c.load_model(&path, &sha, Duration::from_secs(60)).unwrap();
    let pid = c.pid().unwrap();
    let cancel = c.cancel_handle();
    let killer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        cancel.kill();
    });
    let started = Instant::now();
    let r = c.transcribe(job(20, 1), Language::auto(), &pcm, Duration::from_secs(60));
    killer.join().unwrap();
    assert!(matches!(r, Err(ClientError::WorkerExited)), "{r:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        started.elapsed()
    );
    drop(c);
    assert!(!process_alive(pid), "worker {pid} survived cancel");

    // Restart with a new epoch and run a job.
    let mut c2 = WorkerClient::spawn(&worker_exe(), 21, T).unwrap();
    c2.load_model(&path, &sha, Duration::from_secs(60)).unwrap();
    let r = c2
        .transcribe(
            job(21, 1),
            Language::try_from("en".to_string()).unwrap(),
            &jfk_pcm(),
            JOB,
        )
        .unwrap();
    assert!(normalize(&r.text).contains("my fellow americans"));
}

#[test]
fn timeout_kills_worker() {
    let Some((path, sha)) = model() else { return };
    let pcm: Vec<f32> = jfk_pcm().into_iter().cycle().take(16_000 * 60).collect();
    let mut c = WorkerClient::spawn(&worker_exe(), 30, T).unwrap();
    c.load_model(&path, &sha, Duration::from_secs(60)).unwrap();
    let pid = c.pid().unwrap();
    let r = c.transcribe(
        job(30, 1),
        Language::auto(),
        &pcm,
        Duration::from_millis(200),
    );
    assert!(matches!(r, Err(ClientError::Timeout(_))), "{r:?}");
    assert!(
        !process_alive(pid),
        "worker should be killed and reaped on timeout"
    );
}
