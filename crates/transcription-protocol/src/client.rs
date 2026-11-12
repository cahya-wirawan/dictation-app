//! Parent-side supervisor for one `dictation-worker` process.
//!
//! Cancellation is a hard kill (docs/04 §5): [`CancelHandle::kill`] closes the
//! process, which makes the blocked reader observe EOF and the in-flight
//! [`WorkerClient::transcribe`] call return [`ClientError::WorkerExited`].
//! The caller is responsible for fencing late results by [`JobId`]; this client
//! additionally discards any result whose job id does not match the request.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dictation_domain::WorkerEpoch;

use crate::framing::{has_non_finite, read_frame, write_frame, write_pcm};
use crate::{
    FrameError, FromWorker, JobId, Language, MAX_SAMPLES, PROTOCOL_VERSION, ToWorker,
    TranscriptionResult, WorkerErrorCode,
};

const STDERR_TAIL_LINES: usize = 32;
const STDERR_LINE_MAX: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("failed to spawn worker: {0}")]
    Spawn(std::io::Error),
    #[error("worker protocol error: {0}")]
    Frame(#[from] FrameError),
    #[error("worker speaks protocol {0}, expected {PROTOCOL_VERSION}")]
    VersionMismatch(u32),
    #[error("unexpected message from worker: {0}")]
    Unexpected(String),
    #[error("worker reported {code:?}: {detail}")]
    Worker {
        code: WorkerErrorCode,
        detail: String,
    },
    #[error("worker exited or closed its pipe")]
    WorkerExited,
    #[error("timed out after {0:?}; worker killed")]
    Timeout(Duration),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub engine: String,
    pub engine_revision: String,
    pub backends: Vec<String>,
}

/// Cloneable handle that can kill the worker from any thread.
#[derive(Clone)]
pub struct CancelHandle {
    child: Arc<Mutex<Child>>,
}

impl CancelHandle {
    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}

pub struct WorkerClient {
    child: Arc<Mutex<Child>>,
    stdin: Option<BufWriter<ChildStdin>>,
    rx: Receiver<Result<FromWorker, FrameError>>,
    epoch: WorkerEpoch,
    info: WorkerInfo,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    model_loaded: bool,
}

impl WorkerClient {
    /// Spawns the worker from an absolute, app-owned path (never PATH lookup)
    /// and completes the Hello handshake.
    pub fn spawn(
        exe: &Path,
        epoch: WorkerEpoch,
        handshake_timeout: Duration,
    ) -> Result<Self, ClientError> {
        if !exe.is_absolute() {
            return Err(ClientError::InvalidRequest(
                "worker path must be absolute".into(),
            ));
        }
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .spawn()
            .map_err(ClientError::Spawn)?;

        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClientError::Spawn(std::io::Error::other(
                "missing stdio pipe",
            )));
        };

        let (tx, rx) = mpsc::sync_channel(4);
        thread::Builder::new()
            .name(format!("worker-{epoch}-stdout"))
            .spawn(move || {
                let mut r = BufReader::new(stdout);
                loop {
                    let msg = match read_frame::<_, FromWorker>(&mut r) {
                        Ok(Some(m)) => Ok(m),
                        Ok(None) => break,
                        Err(e) => Err(e),
                    };
                    let fatal = msg.is_err();
                    if tx.send(msg).is_err() || fatal {
                        break;
                    }
                }
            })
            .map_err(ClientError::Spawn)?;

        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let tail = Arc::clone(&stderr_tail);
        thread::Builder::new()
            .name(format!("worker-{epoch}-stderr"))
            .spawn(move || {
                // Bounded: keep only the last N truncated lines in memory.
                for line in BufReader::new(stderr).lines() {
                    let Ok(mut line) = line else { break };
                    if line.len() > STDERR_LINE_MAX {
                        let mut cut = STDERR_LINE_MAX;
                        while !line.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        line.truncate(cut);
                    }
                    if let Ok(mut t) = tail.lock() {
                        if t.len() == STDERR_TAIL_LINES {
                            t.pop_front();
                        }
                        t.push_back(line);
                    }
                }
            })
            .map_err(ClientError::Spawn)?;

        let mut client = Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Some(BufWriter::new(stdin)),
            rx,
            epoch,
            info: WorkerInfo {
                engine: String::new(),
                engine_revision: String::new(),
                backends: vec![],
            },
            stderr_tail,
            model_loaded: false,
        };
        match client.recv(handshake_timeout)? {
            FromWorker::Hello {
                protocol,
                engine,
                engine_revision,
                backends,
            } => {
                if protocol != PROTOCOL_VERSION {
                    client.kill();
                    return Err(ClientError::VersionMismatch(protocol));
                }
                client.info = WorkerInfo {
                    engine,
                    engine_revision,
                    backends,
                };
                Ok(client)
            }
            other => {
                client.kill();
                Err(ClientError::Unexpected(format!("{other:?}")))
            }
        }
    }

    pub fn epoch(&self) -> WorkerEpoch {
        self.epoch
    }
    pub fn info(&self) -> &WorkerInfo {
        &self.info
    }
    pub fn model_loaded(&self) -> bool {
        self.model_loaded
    }
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            child: Arc::clone(&self.child),
        }
    }
    /// Last few redacted diagnostic lines (whisper.cpp logs are suppressed by the
    /// worker; transcript text is never written to stderr).
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the worker-reported load time in milliseconds.
    pub fn load_model(
        &mut self,
        path: &Path,
        sha256: &str,
        timeout: Duration,
    ) -> Result<u64, ClientError> {
        let msg = ToWorker::LoadModel {
            path: path.to_string_lossy().into_owned(),
            sha256: sha256.to_owned(),
        };
        msg.validate().map_err(ClientError::InvalidRequest)?;
        self.send(&msg)?;
        match self.recv(timeout)? {
            FromWorker::ModelReady {
                sha256: got,
                load_ms,
            } if got == sha256 => {
                self.model_loaded = true;
                Ok(load_ms)
            }
            FromWorker::Error { code, detail, .. } => Err(ClientError::Worker { code, detail }),
            other => Err(ClientError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Runs one job. On timeout the worker is killed (and must be respawned).
    pub fn transcribe(
        &mut self,
        job: JobId,
        language: Language,
        pcm: &[f32],
        timeout: Duration,
    ) -> Result<TranscriptionResult, ClientError> {
        if job.worker_epoch != self.epoch {
            return Err(ClientError::InvalidRequest(
                "job epoch does not match this worker".into(),
            ));
        }
        if pcm.is_empty() || pcm.len() > MAX_SAMPLES as usize {
            return Err(ClientError::InvalidRequest(
                "PCM length out of range".into(),
            ));
        }
        if has_non_finite(pcm) {
            return Err(ClientError::InvalidRequest(
                "PCM contains non-finite samples".into(),
            ));
        }
        let n = pcm.len() as u32; // bounded by MAX_SAMPLES above
        let msg = ToWorker::Transcribe {
            job,
            language,
            sample_count: n,
            pcm_bytes: n * 4,
        };
        msg.validate().map_err(ClientError::InvalidRequest)?;
        self.send(&msg)?;
        let stdin = self.stdin.as_mut().ok_or(ClientError::WorkerExited)?;
        write_pcm(stdin, pcm).map_err(|_| ClientError::WorkerExited)?;

        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left)? {
                FromWorker::Result(r) if r.job == job => {
                    r.validate().map_err(ClientError::Unexpected)?;
                    return Ok(r);
                }
                FromWorker::Error {
                    job: Some(j),
                    code,
                    detail,
                } if j == job => {
                    return Err(ClientError::Worker { code, detail });
                }
                FromWorker::Error {
                    job: None,
                    code,
                    detail,
                } => return Err(ClientError::Worker { code, detail }),
                // Stale result/error for an older job: discard (invariant 4).
                FromWorker::Result(_) | FromWorker::Error { .. } => continue,
                other => return Err(ClientError::Unexpected(format!("{other:?}"))),
            }
        }
    }

    /// Kill and reap. Idempotent.
    pub fn kill(&mut self) -> Option<ExitStatus> {
        self.stdin = None;
        let mut c = self.child.lock().ok()?;
        let _ = c.kill();
        c.wait().ok()
    }

    /// Graceful exit: close stdin (worker exits on EOF), wait `grace`, then kill.
    pub fn shutdown(mut self, grace: Duration) -> Option<ExitStatus> {
        if let Some(mut w) = self.stdin.take() {
            let _ = write_frame(&mut w, &ToWorker::Shutdown {});
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if let Ok(mut c) = self.child.lock()
                && let Ok(Some(status)) = c.try_wait()
            {
                return Some(status);
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.kill()
    }

    /// Process id, for orphan checks in tests and diagnostics.
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().ok().map(|c| c.id())
    }

    fn send(&mut self, msg: &ToWorker) -> Result<(), ClientError> {
        let w = self.stdin.as_mut().ok_or(ClientError::WorkerExited)?;
        write_frame(w, msg).map_err(|e| match e {
            FrameError::Io(_) => ClientError::WorkerExited,
            other => ClientError::Frame(other),
        })
    }

    fn recv(&mut self, timeout: Duration) -> Result<FromWorker, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(m)) => Ok(m),
            Ok(Err(e)) => {
                self.kill();
                Err(ClientError::Frame(e))
            }
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Err(ClientError::Timeout(timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.kill();
                Err(ClientError::WorkerExited)
            }
        }
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        // Never leave an orphan: kill + reap.
        self.kill();
    }
}
