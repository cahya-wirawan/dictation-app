//! Coordinator actor (docs/03 §1, §3): the only owner allowed to advance a
//! session or authorize delivery. One thread consumes [`Msg`]s sequentially,
//! feeds them to the pure reducer and executes the resulting effects through
//! adapters. Adapters report back by sending messages; they never call each
//! other and never paste on their own.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dictation_domain::session::{
    AppError, CaptureOutcome, DeliveryMode, DispatchResult, Draft, Effect, Input, JobKey, Ledger,
    Machine, Outcome,
};
use dictation_domain::{Generation, Phase, SessionId, WorkerEpoch};

/// How often the actor ticks for draft expiry when idle.
const TICK: Duration = Duration::from_secs(1);

/// The model a worker loads: an app-owned absolute path plus the approved
/// hash from the bundled manifest (the worker re-verifies while loading).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub path: PathBuf,
    pub sha256: String,
}

/// Events adapters send back to the actor.
#[derive(Debug)]
pub enum AdapterEvent {
    CaptureLevel {
        id: SessionId,
        generation: Generation,
        level: f32,
    },
    CaptureOnset {
        id: SessionId,
        generation: Generation,
    },
    /// Microphone closed; `pcm` is present only for accepted speech.
    /// `segment` numbers the utterances of one capture, starting at 0.
    CaptureFinished {
        id: SessionId,
        generation: Generation,
        segment: u32,
        outcome: CaptureOutcome,
        pcm: Option<Vec<f32>>,
    },
    /// Continuous capture: one utterance ended, the microphone stays open.
    CaptureSegment {
        id: SessionId,
        generation: Generation,
        segment: u32,
        outcome: CaptureOutcome,
        pcm: Option<Vec<f32>>,
    },
    /// Microphone closed after a fault.
    CaptureFailed {
        id: SessionId,
        generation: Generation,
        error: AppError,
    },
    /// Microphone closed after an abort request.
    CaptureAborted {
        id: SessionId,
        generation: Generation,
    },
    TranscriptionDone {
        job: JobKey,
        text: String,
    },
    TranscriptionFailed {
        job: JobKey,
        error: AppError,
    },
    WorkerReady {
        epoch: WorkerEpoch,
    },
    DeliveryPrepared {
        id: SessionId,
        generation: Generation,
        result: Result<(), AppError>,
    },
    DispatchFinished {
        id: SessionId,
        generation: Generation,
        result: DispatchResult,
    },
    CopyFinished {
        id: SessionId,
        revision: u32,
        ok: bool,
    },
    /// A model switch finished: on success, the new worker's epoch.
    ModelSwitched {
        result: Result<WorkerEpoch, AppError>,
    },
}

/// Opens and closes the microphone. Exactly one terminal event
/// (`CaptureFinished`/`CaptureFailed`/`CaptureAborted`) per successful `open`.
/// With `continuous`, `CaptureSegment`s report utterances before it.
pub trait CaptureAdapter: Send {
    fn open(
        &mut self,
        id: SessionId,
        generation: Generation,
        continuous: bool,
        events: Sender<Msg>,
    ) -> Result<(), AppError>;
    fn finish(&mut self, id: SessionId);
    fn abort(&mut self, id: SessionId);
}

pub trait TranscriberAdapter: Send {
    /// Starts one job in the background; reports `TranscriptionDone/Failed`.
    fn start(&mut self, job: JobKey, pcm: Vec<f32>, events: Sender<Msg>);
    /// Hard cancel: kill the worker, discard the job, begin respawning.
    /// Returns the epoch the replacement worker will use.
    fn kill(&mut self) -> WorkerEpoch;
    /// Load `model` in a fresh worker, smoke-test it, then swap it in.
    /// Reports `ModelSwitched`. The previous model keeps serving on failure.
    fn switch_model(&mut self, model: ModelRef, events: Sender<Msg>) {
        let _ = model;
        let _ = events.send(Msg::Adapter(AdapterEvent::ModelSwitched {
            result: Ok(self.kill()),
        }));
    }
    /// Explicit unload: stop the worker and forget the model.
    fn unload(&mut self) -> WorkerEpoch {
        self.kill()
    }
}

pub trait DeliveryAdapter: Send {
    /// Captures the target token at the start press. `false` means automatic
    /// delivery is impossible for this session (copy-only).
    fn capture_target(&mut self, id: SessionId) -> bool;
    fn prepare(
        &mut self,
        id: SessionId,
        generation: Generation,
        mode: DeliveryMode,
        text: String,
        events: Sender<Msg>,
    );
    fn dispatch(&mut self, id: SessionId, generation: Generation, events: Sender<Msg>);
    fn release(&mut self, id: SessionId);
    fn copy(&mut self, id: SessionId, revision: u32, text: String, events: Sender<Msg>);
}

/// UI fan-out. Payloads never contain transcript text.
pub trait Notifier: Send {
    fn phase(&mut self, phase: Phase);
    fn outcome(&mut self, outcome: Outcome);
    fn level(&mut self, level: f32);
}

/// Commands from shortcuts, tray and IPC.
#[derive(Debug)]
pub enum Command {
    /// The toggle chord: start when idle, finish when listening.
    Toggle,
    Stop,
    Cancel,
    ReviewOpened,
    DraftEdit {
        id: SessionId,
        expected_revision: u32,
        text: String,
    },
    DraftCopy {
        id: SessionId,
        revision: u32,
    },
    DraftDiscard {
        id: SessionId,
    },
    DraftPaste {
        id: SessionId,
        revision: u32,
    },
    Suspend,
    /// Whether automatic delivery is allowed for the next session (config).
    SetAutoDelivery(bool),
    /// Continuous dictation for the next session (config, ADR-014).
    SetContinuous(bool),
    /// Readiness for new sessions (model installed, worker up, …).
    SetReady(Result<(), AppError>),
    /// Activate another model; only while no session is in flight.
    SwitchModel {
        model: ModelRef,
        reply: Sender<Result<(), AppError>>,
    },
    UnloadModel {
        reply: Sender<Result<(), AppError>>,
    },
    Snapshot(Sender<Snapshot>),
    /// Review window only: the draft including its text.
    GetDraft(Sender<Option<Draft>>),
    Shutdown,
}

#[derive(Debug)]
pub enum Msg {
    Command(Command),
    Adapter(AdapterEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftInfo {
    pub id: SessionId,
    pub revision: u32,
    pub attempted: bool,
    /// Time left before the memory-only draft is cleared (docs/01 §5).
    pub expires_in_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub session: Option<SessionId>,
    pub ledger: Option<Ledger>,
    pub draft: Option<DraftInfo>,
    pub worker_epoch: WorkerEpoch,
    /// True while the microphone is open (for "no idle capture" checks).
    pub capture_open: bool,
    pub switching_model: bool,
    /// A continuous dictation session is live.
    pub continuous: bool,
}

pub struct Adapters {
    pub capture: Box<dyn CaptureAdapter>,
    pub transcriber: Box<dyn TranscriberAdapter>,
    pub delivery: Box<dyn DeliveryAdapter>,
    pub notifier: Box<dyn Notifier>,
}

/// Handle used by the rest of the app. Cloning is cheap.
#[derive(Clone)]
pub struct CoordinatorHandle {
    tx: Sender<Msg>,
}

impl CoordinatorHandle {
    pub fn send(&self, c: Command) -> bool {
        self.tx.send(Msg::Command(c)).is_ok()
    }

    pub fn snapshot(&self, timeout: Duration) -> Option<Snapshot> {
        let (tx, rx) = mpsc::channel();
        self.send(Command::Snapshot(tx));
        rx.recv_timeout(timeout).ok()
    }

    pub fn draft(&self, timeout: Duration) -> Option<Draft> {
        let (tx, rx) = mpsc::channel();
        self.send(Command::GetDraft(tx));
        rx.recv_timeout(timeout).ok().flatten()
    }
}

pub struct Coordinator {
    handle: CoordinatorHandle,
    join: Option<JoinHandle<()>>,
}

impl Coordinator {
    pub fn spawn(adapters: Adapters, initial_epoch: WorkerEpoch) -> Self {
        let (tx, rx) = mpsc::channel();
        let self_tx = tx.clone();
        let join = thread::Builder::new()
            .name("coordinator".into())
            .spawn(move || Actor::new(adapters, initial_epoch, self_tx).run(&rx))
            .ok();
        Self {
            handle: CoordinatorHandle { tx },
            join,
        }
    }

    pub fn handle(&self) -> CoordinatorHandle {
        self.handle.clone()
    }

    /// Cancels any session, closes resources and stops the actor.
    pub fn shutdown(mut self) {
        self.handle.send(Command::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.handle.send(Command::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Actor {
    machine: Machine,
    a: Adapters,
    tx: Sender<Msg>,
    started: Instant,
    /// Session whose microphone is open (until its terminal capture event).
    capture_open: Option<(SessionId, Generation)>,
    /// Accepted utterances awaiting `StartTranscription`, by capture segment.
    pcm: HashMap<(SessionId, u32), Vec<f32>>,
    auto_delivery: bool,
    continuous: bool,
    ready: Result<(), AppError>,
    /// Follow-up inputs produced while executing effects. Processed only after
    /// every effect of the current input, so notifications stay in order.
    queue: VecDeque<Input>,
    stepping: bool,
    /// Pending model switch reply; new sessions are refused meanwhile.
    switching: Option<Sender<Result<(), AppError>>>,
}

impl Actor {
    fn new(a: Adapters, epoch: WorkerEpoch, tx: Sender<Msg>) -> Self {
        Self {
            machine: Machine::new(epoch),
            a,
            tx,
            started: Instant::now(),
            capture_open: None,
            pcm: HashMap::new(),
            auto_delivery: true,
            continuous: false,
            ready: Ok(()),
            queue: VecDeque::new(),
            stepping: false,
            switching: None,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn run(mut self, rx: &Receiver<Msg>) {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Msg::Command(Command::Shutdown)) | Err(RecvTimeoutError::Disconnected) => {
                    self.step(Input::Cancel);
                    self.step(Input::Suspend); // clears any draft
                    if let Some((id, _)) = self.capture_open.take() {
                        self.a.capture.abort(id);
                    }
                    return;
                }
                Ok(Msg::Command(c)) => self.command(c),
                Ok(Msg::Adapter(e)) => self.adapter(e),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = self.now_ms();
            self.step(Input::Tick { now_ms: now });
        }
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Toggle
                if self.switching.is_some()
                    && matches!(self.machine.phase(), Phase::Idle | Phase::DraftReady) =>
            {
                self.a.notifier.outcome(Outcome::Rejected(AppError::Busy));
            }
            Command::Toggle => match self.machine.phase() {
                Phase::Idle | Phase::DraftReady => self.start(),
                // Transcribing/Delivering: a continuous session stops
                // listening after the phrase in flight; otherwise Busy.
                Phase::Starting | Phase::Listening | Phase::Transcribing | Phase::Delivering => {
                    if let Some((id, _)) = self.machine.live_session() {
                        self.step(Input::Stop { id });
                    }
                }
                _ => self.a.notifier.outcome(Outcome::Rejected(AppError::Busy)),
            },
            Command::Stop => {
                if let Some((id, _)) = self.machine.live_session() {
                    self.step(Input::Stop { id });
                }
            }
            Command::Cancel => self.step(Input::Cancel),
            Command::ReviewOpened => self.step(Input::ReviewOpened),
            Command::DraftEdit {
                id,
                expected_revision,
                text,
            } => self.step(Input::DraftEdit {
                id,
                expected_revision,
                text,
            }),
            Command::DraftCopy { id, revision } => self.step(Input::DraftCopy { id, revision }),
            Command::DraftDiscard { id } => self.step(Input::DraftDiscard { id }),
            Command::DraftPaste { id, revision } => {
                // The target for explicit delivery is captured at this press.
                self.a.delivery.capture_target(id);
                self.step(Input::DraftPaste { id, revision });
            }
            Command::Suspend => self.step(Input::Suspend),
            Command::SetAutoDelivery(on) => self.auto_delivery = on,
            Command::SetContinuous(on) => self.continuous = on,
            Command::SetReady(r) => self.ready = r,
            Command::SwitchModel { model, reply } => {
                if self.switching.is_some()
                    || !matches!(self.machine.phase(), Phase::Idle | Phase::DraftReady)
                {
                    let _ = reply.send(Err(AppError::Busy));
                } else {
                    self.switching = Some(reply);
                    self.a.transcriber.switch_model(model, self.tx.clone());
                }
            }
            Command::UnloadModel { reply } => {
                if self.switching.is_some()
                    || !matches!(self.machine.phase(), Phase::Idle | Phase::DraftReady)
                {
                    let _ = reply.send(Err(AppError::Busy));
                } else {
                    let epoch = self.a.transcriber.unload();
                    self.step(Input::WorkerEpochChanged { epoch });
                    let _ = reply.send(Ok(()));
                }
            }
            Command::Snapshot(reply) => {
                let _ = reply.send(Snapshot {
                    phase: self.machine.phase(),
                    session: self.machine.live_session().map(|(id, _)| id),
                    ledger: self.machine.ledger(),
                    draft: self.machine.draft().map(|d| DraftInfo {
                        id: d.id,
                        revision: d.revision,
                        attempted: d.attempted,
                        expires_in_ms: dictation_domain::session::DRAFT_TTL_MS
                            .saturating_sub(self.now_ms().saturating_sub(d.created_ms)),
                    }),
                    worker_epoch: self.machine.worker_epoch(),
                    capture_open: self.capture_open.is_some(),
                    switching_model: self.switching.is_some(),
                    continuous: self.machine.continuous(),
                });
            }
            Command::GetDraft(reply) => {
                let _ = reply.send(self.machine.draft().cloned());
            }
            Command::Shutdown => {}
        }
    }

    fn start(&mut self) {
        let id = match SessionId::random() {
            Ok(id) => id,
            Err(_) => {
                self.a
                    .notifier
                    .outcome(Outcome::Rejected(AppError::NotReady));
                return;
            }
        };
        let now_ms = self.now_ms();
        // Capture the target before anything else touches focus (docs/05 §4).
        let auto = self.ready.is_ok()
            && self.machine.phase() == Phase::Idle
            && self.auto_delivery
            && self.a.delivery.capture_target(id);
        self.step(Input::Start {
            id,
            now_ms,
            ready: self.ready,
            auto_delivery: auto,
            continuous: self.continuous,
        });
        if self.machine.live_session().map(|(s, _)| s) != Some(id) {
            self.a.delivery.release(id); // rejected start: drop the token
        }
    }

    fn adapter(&mut self, e: AdapterEvent) {
        match e {
            AdapterEvent::CaptureLevel {
                id,
                generation,
                level,
            } => {
                if self.capture_open == Some((id, generation)) {
                    self.a.notifier.level(level);
                }
            }
            AdapterEvent::CaptureOnset { .. } => {}
            AdapterEvent::CaptureFinished {
                id,
                generation,
                segment,
                outcome,
                pcm,
            } => {
                self.hold_audio(id, generation, segment, outcome, pcm);
                self.capture_closed(id, generation);
                self.step(Input::CaptureFinished {
                    id,
                    generation,
                    segment,
                    outcome,
                });
                self.release_unqueued(id, segment);
            }
            AdapterEvent::CaptureSegment {
                id,
                generation,
                segment,
                outcome,
                pcm,
            } => {
                self.hold_audio(id, generation, segment, outcome, pcm);
                self.step(Input::CaptureSegment {
                    id,
                    generation,
                    segment,
                    outcome,
                });
                self.release_unqueued(id, segment);
            }
            AdapterEvent::CaptureFailed {
                id,
                generation,
                error,
            } => {
                self.capture_closed(id, generation);
                self.step(Input::CaptureFailed {
                    id,
                    generation,
                    error,
                });
            }
            AdapterEvent::CaptureAborted { id, generation } => {
                self.capture_closed(id, generation);
                self.maybe_resources_closed();
            }
            AdapterEvent::TranscriptionDone { job, text } => {
                self.step(Input::TranscriptionDone { job, text })
            }
            AdapterEvent::TranscriptionFailed { job, error } => {
                self.step(Input::TranscriptionFailed { job, error })
            }
            AdapterEvent::WorkerReady { epoch } => self.step(Input::WorkerEpochChanged { epoch }),
            AdapterEvent::DeliveryPrepared {
                id,
                generation,
                result,
            } => self.step(Input::DeliveryPrepared {
                id,
                generation,
                result,
            }),
            AdapterEvent::DispatchFinished {
                id,
                generation,
                result,
            } => self.step(Input::DispatchFinished {
                id,
                generation,
                result,
            }),
            AdapterEvent::CopyFinished { id, revision, ok } => {
                self.step(Input::DraftCopyFinished { id, revision, ok })
            }
            AdapterEvent::ModelSwitched { result } => {
                if let Ok(epoch) = result {
                    self.step(Input::WorkerEpochChanged { epoch });
                }
                if let Some(reply) = self.switching.take() {
                    let _ = reply.send(result.map(|_| ()));
                }
            }
        }
    }

    /// Keeps accepted speech of the live capture until the reducer decides.
    fn hold_audio(
        &mut self,
        id: SessionId,
        generation: Generation,
        segment: u32,
        outcome: CaptureOutcome,
        pcm: Option<Vec<f32>>,
    ) {
        if let Some(pcm) = pcm
            && outcome == CaptureOutcome::Speech
            && self.machine.capture_session() == Some((id, generation))
        {
            self.pcm.insert((id, segment), pcm);
        }
    }

    /// Audio the reducer neither transcribed nor queued is released at once.
    fn release_unqueued(&mut self, id: SessionId, segment: u32) {
        if !self.machine.is_queued(id, segment) {
            self.pcm.remove(&(id, segment));
        }
    }

    fn capture_closed(&mut self, id: SessionId, generation: Generation) {
        if self.capture_open == Some((id, generation)) {
            self.capture_open = None;
        }
    }

    fn maybe_resources_closed(&mut self) {
        if self.machine.phase() == Phase::Recovering
            && self.capture_open.is_none()
            && let Some((id, _)) = self.machine.live_session()
        {
            self.step(Input::ResourcesClosed { id });
        }
    }

    /// Runs `input` and any follow-ups to completion. Re-entrant calls (from
    /// effect execution) only enqueue.
    fn step(&mut self, input: Input) {
        self.queue.push_back(input);
        if self.stepping {
            return;
        }
        self.stepping = true;
        while let Some(next) = self.queue.pop_front() {
            for e in self.machine.step(next) {
                self.execute(e);
            }
            if self.queue.is_empty() {
                self.maybe_resources_closed();
            }
        }
        self.stepping = false;
    }

    fn execute(&mut self, e: Effect) {
        match e {
            Effect::OpenCapture {
                id,
                generation,
                continuous,
            } => {
                match self
                    .a
                    .capture
                    .open(id, generation, continuous, self.tx.clone())
                {
                    Ok(()) => {
                        self.capture_open = Some((id, generation));
                        self.step(Input::CaptureOpened { id, generation });
                    }
                    Err(error) => self.step(Input::CaptureFailed {
                        id,
                        generation,
                        error,
                    }),
                }
            }
            Effect::FinishCapture { id, .. } => self.a.capture.finish(id),
            Effect::AbortCapture { id } => {
                if self.capture_open.is_some_and(|(s, _)| s == id) {
                    self.a.capture.abort(id);
                }
            }
            Effect::StartTranscription { job, segment } => {
                match self.pcm.remove(&(job.session, segment)) {
                    Some(pcm) => self.a.transcriber.start(job, pcm, self.tx.clone()),
                    None => self.step(Input::TranscriptionFailed {
                        job,
                        error: AppError::WorkerCrash,
                    }),
                }
            }
            Effect::KillWorker => {
                let epoch = self.a.transcriber.kill();
                self.step(Input::WorkerEpochChanged { epoch });
            }
            Effect::DropAudio { id } => self.pcm.retain(|(s, _), _| *s != id),
            Effect::PrepareDelivery {
                id,
                generation,
                mode,
                text,
            } => {
                self.a
                    .delivery
                    .prepare(id, generation, mode, text, self.tx.clone());
            }
            Effect::DispatchPaste { id, generation } => {
                self.a.delivery.dispatch(id, generation, self.tx.clone())
            }
            Effect::ReleaseTarget { id } => self.a.delivery.release(id),
            Effect::CopyText { id, revision, text } => {
                self.a.delivery.copy(id, revision, text, self.tx.clone())
            }
            Effect::PhaseChanged(p) => self.a.notifier.phase(p),
            Effect::Notify(o) => self.a.notifier.outcome(o),
        }
    }
}
