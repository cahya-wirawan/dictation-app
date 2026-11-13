//! Pure session state machine (docs/04 §1–2, work package A1).
//!
//! `Machine::step` consumes one [`Input`] and returns the [`Effect`]s the
//! coordinator must perform. No I/O, clocks or randomness happen here: ids and
//! monotonic timestamps arrive in inputs, so every transition is reproducible
//! in tests. Inputs that do not match the live session id, generation, worker
//! epoch and allowed phase are *stale* and ignored (invariant 4).
//!
//! Delivery is split at the commit point (docs/05 §5): `PrepareDelivery`
//! performs every abortable check and the clipboard write, and only a
//! `DeliveryPrepared` processed while the session is still eligible crosses
//! the ledger into `Committed` and emits the single `DispatchPaste`. Cancel is
//! serialized with that commit because both go through this reducer.
//!
//! Continuous dictation (ADR-014): a session started with `continuous` keeps
//! its microphone open and turns every finished utterance (a capture
//! *segment*) into its own phrase with a fresh generation. Phrases are
//! transcribed and pasted strictly in order, each through the same
//! validate → commit → single dispatch path, into the target captured at the
//! start press. The ledger crosses commit at most once per phrase. Any draft
//! (target changed, uncertain send, review opened) ends the session.

use std::collections::VecDeque;

use crate::{Generation, Phase, SessionId, WorkerEpoch};

/// Draft time-to-live (docs/01 §5): 10 minutes, memory only.
pub const DRAFT_TTL_MS: u64 = 10 * 60 * 1000;
/// Upper bound on a draft edit (bytes), mirrors the worker's text limit.
pub const MAX_DRAFT_BYTES: usize = 32 * 1024;

/// Error and outcome codes shared with the UI contract (docs/04 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppError {
    NotReady,
    Busy,
    DraftPending,
    ShortcutConflict,
    MicDenied,
    MicDisconnected,
    AudioOverflow,
    ModelInvalid,
    ModelUnavailable,
    WorkerCrash,
    InferenceTimeout,
    TargetChanged,
    /// Shortcut modifiers were still held after the release timeout.
    ModifiersHeld,
    PasteUnsupported,
    ClipboardBusy,
    InputUncertain,
    StaleRequest,
    SettingsConflict,
}

/// Why a draft exists instead of an automatic paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DraftReason {
    /// Copy-only readiness or the user chose review mode.
    ReviewMode,
    /// Review window opened while the session was in flight.
    ReviewOpened,
    /// Pre-commit validation failed (target changed, secure field, modifiers held, …).
    Blocked(AppError),
    /// The OS send was partial or failed after commit; resend needs explicit action.
    Uncertain,
}

/// User-visible outcomes. Never carry transcript text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    PasteSent,
    NoSpeech,
    NoUsableText,
    Canceled,
    /// Cancel arrived after dispatch commit: the text was already sent.
    CancelTooLate,
    DraftReady(DraftReason),
    DraftCopied,
    DraftDiscarded,
    DraftExpired,
    Failed(AppError),
    Rejected(AppError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ledger {
    NotAttempted,
    Committed,
    Sent,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// Accepted speech; the coordinator holds the PCM, not the reducer.
    Speech,
    NoSpeech,
    TooShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    Sent,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    /// Original target captured at shortcut start.
    Automatic,
    /// Draft paste: target captured at the deliberate paste-shortcut press.
    Explicit,
}

/// Identity of one in-flight job. Mirrors `transcription_protocol::JobId`
/// without depending on the protocol crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobKey {
    pub session: SessionId,
    pub generation: Generation,
    pub worker_epoch: WorkerEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: SessionId,
    pub revision: u32,
    pub text: String,
    pub reason: DraftReason,
    pub created_ms: u64,
    /// A send was attempted with an uncertain result.
    pub attempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Toggle pressed while idle (or Start command). `auto_delivery` is false
    /// when the platform/target cannot support automatic paste (copy-only).
    Start {
        id: SessionId,
        now_ms: u64,
        ready: Result<(), AppError>,
        auto_delivery: bool,
        /// Continuous dictation setting; only honored with `auto_delivery`.
        continuous: bool,
    },
    CaptureOpened {
        id: SessionId,
        generation: Generation,
    },
    CaptureFailed {
        id: SessionId,
        generation: Generation,
        error: AppError,
    },
    /// Explicit stop (second toggle press, Stop button).
    Stop {
        id: SessionId,
    },
    /// Endpointer/explicit-stop/cap result; the microphone is already closed.
    /// `segment` numbers the capture's utterances (audio key for the actor).
    CaptureFinished {
        id: SessionId,
        generation: Generation,
        segment: u32,
        outcome: CaptureOutcome,
    },
    /// Continuous capture only: one utterance ended, the microphone is still
    /// open for the next one.
    CaptureSegment {
        id: SessionId,
        generation: Generation,
        segment: u32,
        outcome: CaptureOutcome,
    },
    TranscriptionDone {
        job: JobKey,
        text: String,
    },
    TranscriptionFailed {
        job: JobKey,
        error: AppError,
    },
    /// A fresh worker process is ready; older epochs become stale.
    WorkerEpochChanged {
        epoch: WorkerEpoch,
    },
    /// Pre-commit delivery phase finished (target validated, modifiers up,
    /// clipboard written, target rechecked).
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
    /// Global cancel chord or tray Cancel. Targets the live session.
    Cancel,
    /// Capture, worker and delivery resources for `id` are closed.
    ResourcesClosed {
        id: SessionId,
    },
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
    DraftCopyFinished {
        id: SessionId,
        revision: u32,
        ok: bool,
    },
    DraftDiscard {
        id: SessionId,
    },
    /// Deliberate paste-draft shortcut press; target captured at that press.
    DraftPaste {
        id: SessionId,
        revision: u32,
    },
    /// Sleep, screen lock or logout.
    Suspend,
    Tick {
        now_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    OpenCapture {
        id: SessionId,
        generation: Generation,
        /// Keep listening after each utterance (ADR-014).
        continuous: bool,
    },
    /// Explicit stop: finalize with the audio captured so far.
    FinishCapture {
        id: SessionId,
        generation: Generation,
    },
    /// Abort: close the microphone and discard audio.
    AbortCapture {
        id: SessionId,
    },
    StartTranscription {
        job: JobKey,
        /// Which captured utterance to transcribe.
        segment: u32,
    },
    /// Hard-cancel the worker (kill + respawn later). Discards the job.
    KillWorker,
    /// Release any held PCM for this session.
    DropAudio {
        id: SessionId,
    },
    PrepareDelivery {
        id: SessionId,
        generation: Generation,
        mode: DeliveryMode,
        text: String,
    },
    DispatchPaste {
        id: SessionId,
        generation: Generation,
    },
    /// Forget the target token / delivery transaction for this session.
    ReleaseTarget {
        id: SessionId,
    },
    CopyText {
        id: SessionId,
        revision: u32,
        text: String,
    },
    PhaseChanged(Phase),
    Notify(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Live {
    id: SessionId,
    generation: Generation,
    auto_eligible: bool,
    ledger: Ledger,
    mode: DeliveryMode,
    /// Text being delivered (Delivering phase only).
    text: Option<String>,
    /// Revision of the draft being delivered explicitly.
    draft_revision: Option<u32>,
    /// Pending notification once resources close (Recovering).
    closing_outcome: Option<Outcome>,
    /// Continuous dictation session (ADR-014).
    continuous: bool,
    /// Generation of the capture; equals `generation` except in continuous
    /// sessions, where every phrase after the first gets a new generation.
    capture_generation: Generation,
    /// The microphone of this session has not reported a terminal event.
    capture_open: bool,
    /// Continuous: no new phrases after the capture closes (stop requested,
    /// no-speech timeout, capture fault or review opened).
    finishing: bool,
    /// Continuous: accepted utterances waiting for their turn, oldest first.
    queue: VecDeque<u32>,
    /// Continuous: phrases started / pasted so far.
    phrases: u32,
    pasted: u32,
    /// Continuous: why the session ended, reported if nothing was pasted
    /// (or always, for a fault).
    end_outcome: Option<Outcome>,
}

impl Live {
    fn new(id: SessionId, generation: Generation, mode: DeliveryMode) -> Self {
        Self {
            id,
            generation,
            auto_eligible: false,
            ledger: Ledger::NotAttempted,
            mode,
            text: None,
            draft_revision: None,
            closing_outcome: None,
            continuous: false,
            capture_generation: generation,
            capture_open: false,
            finishing: false,
            queue: VecDeque::new(),
            phrases: 0,
            pasted: 0,
            end_outcome: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    phase: Phase,
    live: Option<Live>,
    draft: Option<Draft>,
    next_generation: Generation,
    worker_epoch: WorkerEpoch,
    /// Latest monotonic time seen (from `Start`/`Tick`). Draft TTL is measured
    /// against it, so expiry precision equals the coordinator's tick period.
    now_ms: u64,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Machine {
    pub fn new(worker_epoch: WorkerEpoch) -> Self {
        Self {
            phase: Phase::Idle,
            live: None,
            draft: None,
            next_generation: 1,
            worker_epoch,
            now_ms: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }
    pub fn live_session(&self) -> Option<(SessionId, Generation)> {
        self.live.as_ref().map(|l| (l.id, l.generation))
    }
    pub fn ledger(&self) -> Option<Ledger> {
        self.live.as_ref().map(|l| l.ledger)
    }
    pub fn worker_epoch(&self) -> WorkerEpoch {
        self.worker_epoch
    }
    /// The live session's open capture, as its capture events are fenced.
    pub fn capture_session(&self) -> Option<(SessionId, Generation)> {
        self.live
            .as_ref()
            .filter(|l| l.capture_open)
            .map(|l| (l.id, l.capture_generation))
    }
    /// True while a continuous dictation session is live.
    pub fn continuous(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.continuous)
    }
    /// Whether captured utterance `segment` of session `id` still waits for
    /// transcription (the coordinator keeps its audio until then).
    pub fn is_queued(&self, id: SessionId, segment: u32) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.id == id && l.queue.contains(&segment))
    }

    pub fn step(&mut self, input: Input) -> Vec<Effect> {
        let mut fx = Vec::new();
        let before = self.phase;
        self.apply(input, &mut fx);
        if self.phase != before {
            fx.push(Effect::PhaseChanged(self.phase));
        }
        fx
    }

    fn is_live(&self, id: SessionId, generation: Generation) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.id == id && l.generation == generation)
    }

    /// Capture events are fenced on the capture's generation.
    fn is_capture(&self, id: SessionId, generation: Generation) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.id == id && l.capture_generation == generation && l.capture_open)
    }

    fn apply(&mut self, input: Input, fx: &mut Vec<Effect>) {
        use Phase::*;
        match input {
            Input::Start {
                id,
                now_ms,
                ready,
                auto_delivery,
                continuous,
            } => {
                self.now_ms = self.now_ms.max(now_ms);
                match self.phase {
                    Idle => {
                        if let Err(e) = ready {
                            fx.push(Effect::Notify(Outcome::Rejected(e)));
                            return;
                        }
                        let generation = self.next_generation;
                        self.next_generation += 1;
                        let mut live = Live::new(id, generation, DeliveryMode::Automatic);
                        live.auto_eligible = auto_delivery;
                        // Continuous phrases paste on their own; without
                        // automatic delivery there is nothing to continue.
                        live.continuous = continuous && auto_delivery;
                        live.capture_open = true;
                        self.live = Some(live);
                        self.phase = Starting;
                        fx.push(Effect::OpenCapture {
                            id,
                            generation,
                            continuous: continuous && auto_delivery,
                        });
                    }
                    // Never discard a draft silently (docs/02 §4).
                    DraftReady => {
                        fx.push(Effect::Notify(Outcome::Rejected(AppError::DraftPending)))
                    }
                    _ => fx.push(Effect::Notify(Outcome::Rejected(AppError::Busy))),
                }
            }

            Input::CaptureOpened { id, generation } => {
                if self.phase == Starting && self.is_capture(id, generation) {
                    self.phase = Listening;
                }
            }

            Input::CaptureFailed {
                id,
                generation,
                error,
            } => {
                if !self.is_capture(id, generation) {
                    return;
                }
                if matches!(self.phase, Starting | Listening | Finalizing) {
                    self.begin_recovery(Outcome::Failed(error), fx);
                } else if matches!(self.phase, Transcribing | Delivering)
                    && let Some(l) = self.live.as_mut().filter(|l| l.continuous)
                {
                    // The phrase in flight (and any queued) still completes;
                    // the fault is reported when the session ends.
                    l.capture_open = false;
                    l.finishing = true;
                    l.end_outcome = Some(Outcome::Failed(error));
                }
            }

            Input::Stop { id } => {
                let phase = self.phase;
                let Some(l) = self.live.as_mut().filter(|l| l.id == id) else {
                    return;
                };
                // First finish request wins; later ones are no-ops.
                if phase == Listening {
                    l.finishing = true;
                    let generation = l.capture_generation;
                    self.phase = Finalizing;
                    fx.push(Effect::FinishCapture { id, generation });
                } else if matches!(phase, Transcribing | Delivering) {
                    if !l.continuous {
                        fx.push(Effect::Notify(Outcome::Rejected(AppError::Busy)));
                    } else if l.capture_open && !l.finishing {
                        // Continuous: finish the capture; the phrase in
                        // flight and the final utterance still paste.
                        l.finishing = true;
                        fx.push(Effect::FinishCapture {
                            id,
                            generation: l.capture_generation,
                        });
                    }
                }
            }

            Input::CaptureSegment {
                id,
                generation,
                segment,
                outcome,
            } => {
                if !matches!(
                    self.phase,
                    Listening | Finalizing | Transcribing | Delivering
                ) || !self.is_capture(id, generation)
                {
                    return;
                }
                let Some(l) = self.live.as_mut().filter(|l| l.continuous) else {
                    return;
                };
                if outcome == CaptureOutcome::Speech {
                    l.queue.push_back(segment);
                    if matches!(self.phase, Listening | Finalizing) {
                        self.start_next(fx);
                    }
                }
            }

            Input::CaptureFinished {
                id,
                generation,
                segment,
                outcome,
            } => {
                if !self.is_capture(id, generation) {
                    return;
                }
                if self.continuous() {
                    if !matches!(
                        self.phase,
                        Listening | Finalizing | Transcribing | Delivering
                    ) {
                        return;
                    }
                    let Some(l) = self.live.as_mut() else { return };
                    l.capture_open = false;
                    l.finishing = true;
                    match outcome {
                        CaptureOutcome::Speech => l.queue.push_back(segment),
                        CaptureOutcome::NoSpeech => {
                            l.end_outcome.get_or_insert(Outcome::NoSpeech);
                        }
                        CaptureOutcome::TooShort => {
                            l.end_outcome.get_or_insert(Outcome::NoUsableText);
                        }
                    }
                    if matches!(self.phase, Listening | Finalizing) {
                        self.start_next(fx);
                    }
                    return;
                }
                if !matches!(self.phase, Listening | Finalizing) {
                    return;
                }
                if let Some(l) = self.live.as_mut() {
                    l.capture_open = false;
                }
                match outcome {
                    CaptureOutcome::Speech => {
                        self.phase = Transcribing;
                        let job = JobKey {
                            session: id,
                            generation,
                            worker_epoch: self.worker_epoch,
                        };
                        fx.push(Effect::StartTranscription { job, segment });
                    }
                    // Invariant 7: no speech → no inference, clipboard or input.
                    CaptureOutcome::NoSpeech => self.finish_clean(Outcome::NoSpeech, fx),
                    CaptureOutcome::TooShort => self.finish_clean(Outcome::NoUsableText, fx),
                }
            }

            Input::TranscriptionDone { job, text } => {
                if self.phase != Transcribing || !self.job_matches(job) {
                    return;
                }
                let continuous = self.continuous();
                if !continuous {
                    fx.push(Effect::DropAudio { id: job.session });
                }
                let text = text.trim().to_owned();
                if text.is_empty() {
                    if continuous {
                        self.start_next(fx); // a cough or noise: keep listening
                    } else {
                        self.finish_clean(Outcome::NoUsableText, fx);
                    }
                    return;
                }
                let Some(l) = self.live.as_mut() else { return };
                if l.auto_eligible {
                    l.text = Some(text.clone());
                    self.phase = Delivering;
                    fx.push(Effect::PrepareDelivery {
                        id: l.id,
                        generation: l.generation,
                        mode: DeliveryMode::Automatic,
                        text,
                    });
                } else {
                    let reason = if l.closing_outcome.is_some() {
                        DraftReason::ReviewOpened
                    } else {
                        DraftReason::ReviewMode
                    };
                    self.enter_draft(text, reason, false, fx);
                }
            }

            Input::TranscriptionFailed { job, error } => {
                if self.phase == Transcribing && self.job_matches(job) {
                    fx.push(Effect::KillWorker);
                    self.begin_recovery(Outcome::Failed(error), fx);
                }
            }

            Input::WorkerEpochChanged { epoch } => {
                if epoch > self.worker_epoch {
                    self.worker_epoch = epoch;
                }
            }

            Input::DeliveryPrepared {
                id,
                generation,
                result,
            } => {
                if self.phase != Delivering || !self.is_live(id, generation) {
                    return;
                }
                let Some(l) = self.live.as_mut() else { return };
                if l.ledger != Ledger::NotAttempted {
                    return; // the ledger crosses commit at most once
                }
                match result {
                    Ok(()) => {
                        l.ledger = Ledger::Committed;
                        fx.push(Effect::DispatchPaste { id, generation });
                    }
                    Err(e) => {
                        let text = l.text.take().unwrap_or_default();
                        let rev = l.draft_revision;
                        fx.push(Effect::ReleaseTarget { id });
                        self.enter_draft_with_revision(
                            text,
                            DraftReason::Blocked(e),
                            false,
                            rev,
                            fx,
                        );
                    }
                }
            }

            Input::DispatchFinished {
                id,
                generation,
                result,
            } => {
                if self.phase != Delivering || !self.is_live(id, generation) {
                    return;
                }
                let Some(l) = self.live.as_mut() else { return };
                if l.ledger != Ledger::Committed {
                    return;
                }
                if l.continuous {
                    match result {
                        DispatchResult::Sent => {
                            l.ledger = Ledger::Sent;
                            l.text = None;
                            if l.closing_outcome == Some(Outcome::CancelTooLate) {
                                // Canceled after commit: this phrase went out,
                                // nothing after it may.
                                self.begin_recovery(Outcome::CancelTooLate, fx);
                            } else {
                                l.pasted += 1;
                                fx.push(Effect::Notify(Outcome::PasteSent));
                                self.start_next(fx);
                            }
                        }
                        DispatchResult::Uncertain => {
                            l.ledger = Ledger::Uncertain;
                            let text = l.text.take().unwrap_or_default();
                            fx.push(Effect::ReleaseTarget { id });
                            self.enter_draft(text, DraftReason::Uncertain, true, fx);
                        }
                    }
                    return;
                }
                fx.push(Effect::ReleaseTarget { id });
                match result {
                    DispatchResult::Sent => {
                        l.ledger = Ledger::Sent;
                        let late_cancel = l.closing_outcome == Some(Outcome::CancelTooLate);
                        self.live = None;
                        self.draft = None; // an explicitly pasted draft is consumed
                        self.phase = Idle;
                        fx.push(Effect::Notify(if late_cancel {
                            Outcome::CancelTooLate
                        } else {
                            Outcome::PasteSent
                        }));
                    }
                    DispatchResult::Uncertain => {
                        l.ledger = Ledger::Uncertain;
                        let text = l.text.take().unwrap_or_default();
                        let rev = l.draft_revision;
                        self.enter_draft_with_revision(text, DraftReason::Uncertain, true, rev, fx);
                    }
                }
            }

            Input::Cancel => match self.phase {
                Starting | Listening | Finalizing => {
                    self.begin_recovery(Outcome::Canceled, fx);
                }
                Transcribing => {
                    fx.push(Effect::KillWorker);
                    self.begin_recovery(Outcome::Canceled, fx);
                }
                Delivering => {
                    let Some(l) = self.live.as_mut() else { return };
                    if l.ledger == Ledger::NotAttempted {
                        // Before commit: block the send. Explicit draft paste
                        // keeps the draft; an automatic session is abandoned.
                        let id = l.id;
                        fx.push(Effect::ReleaseTarget { id });
                        if l.mode == DeliveryMode::Explicit {
                            self.live = None;
                            self.phase = DraftReady;
                            fx.push(Effect::Notify(Outcome::Canceled));
                        } else {
                            self.begin_recovery(Outcome::Canceled, fx);
                        }
                    } else {
                        // After commit the send cannot be retracted: say so.
                        l.closing_outcome = Some(Outcome::CancelTooLate);
                    }
                }
                Idle | DraftReady | Recovering => {}
            },

            Input::ResourcesClosed { id } => {
                if self.phase == Recovering && self.live.as_ref().is_some_and(|l| l.id == id) {
                    let outcome = self
                        .live
                        .take()
                        .and_then(|l| l.closing_outcome)
                        .unwrap_or(Outcome::Canceled);
                    self.phase = Idle;
                    fx.push(Effect::Notify(outcome));
                }
            }

            Input::ReviewOpened => {
                // Invariant 9: opening Review revokes automatic delivery.
                let phases_ok = if self.continuous() {
                    matches!(
                        self.phase,
                        Starting | Listening | Finalizing | Transcribing | Delivering
                    )
                } else {
                    matches!(self.phase, Starting | Listening | Finalizing | Transcribing)
                };
                if let Some(l) = self.live.as_mut()
                    && phases_ok
                {
                    l.auto_eligible = false;
                    if l.closing_outcome.is_none() {
                        l.closing_outcome = Some(Outcome::DraftReady(DraftReason::ReviewOpened));
                    }
                    // Continuous: the next phrase becomes the draft and ends
                    // the session, so stop listening after it.
                    if l.continuous && l.capture_open && !l.finishing {
                        l.finishing = true;
                        fx.push(Effect::FinishCapture {
                            id: l.id,
                            generation: l.capture_generation,
                        });
                    }
                }
            }

            Input::DraftEdit {
                id,
                expected_revision,
                text,
            } => {
                if self.phase != DraftReady {
                    return;
                }
                match self.draft.as_mut() {
                    Some(d)
                        if d.id == id
                            && d.revision == expected_revision
                            && text.len() <= MAX_DRAFT_BYTES =>
                    {
                        d.text = text;
                        d.revision += 1;
                    }
                    Some(_) => fx.push(Effect::Notify(Outcome::Rejected(AppError::StaleRequest))),
                    None => {}
                }
            }

            Input::DraftCopy { id, revision } => {
                if self.phase == DraftReady
                    && let Some(d) = self.draft.as_ref()
                {
                    if d.id == id && d.revision == revision {
                        fx.push(Effect::CopyText {
                            id,
                            revision,
                            text: d.text.clone(),
                        });
                    } else {
                        fx.push(Effect::Notify(Outcome::Rejected(AppError::StaleRequest)));
                    }
                }
            }

            Input::DraftCopyFinished { id, revision, ok } => {
                if self.phase == DraftReady
                    && self
                        .draft
                        .as_ref()
                        .is_some_and(|d| d.id == id && d.revision == revision)
                {
                    if ok {
                        self.draft = None;
                        self.phase = Idle;
                        fx.push(Effect::Notify(Outcome::DraftCopied));
                    } else {
                        fx.push(Effect::Notify(Outcome::Failed(AppError::ClipboardBusy)));
                    }
                }
            }

            Input::DraftDiscard { id } => {
                if self.phase == DraftReady && self.draft.as_ref().is_some_and(|d| d.id == id) {
                    self.draft = None;
                    self.phase = Idle;
                    fx.push(Effect::Notify(Outcome::DraftDiscarded));
                }
            }

            Input::DraftPaste { id, revision } => {
                if self.phase != DraftReady {
                    return;
                }
                let Some(d) = self.draft.as_ref() else { return };
                if d.id != id || d.revision != revision {
                    fx.push(Effect::Notify(Outcome::Rejected(AppError::StaleRequest)));
                    return;
                }
                let generation = self.next_generation;
                self.next_generation += 1;
                let text = d.text.clone();
                let mut live = Live::new(id, generation, DeliveryMode::Explicit);
                live.text = Some(text.clone());
                live.draft_revision = Some(revision);
                self.live = Some(live);
                self.phase = Delivering;
                fx.push(Effect::PrepareDelivery {
                    id,
                    generation,
                    mode: DeliveryMode::Explicit,
                    text,
                });
            }

            Input::Suspend => {
                // Invariant 10: cancel everything and clear the draft; never resume.
                let had_draft = self.draft.take().is_some();
                match self.phase {
                    Starting | Listening | Finalizing => self.begin_recovery(Outcome::Canceled, fx),
                    Transcribing => {
                        fx.push(Effect::KillWorker);
                        self.begin_recovery(Outcome::Canceled, fx);
                    }
                    Delivering => {
                        let committed = self
                            .live
                            .as_ref()
                            .is_some_and(|l| l.ledger != Ledger::NotAttempted);
                        if committed {
                            if let Some(l) = self.live.as_mut() {
                                l.closing_outcome = Some(Outcome::CancelTooLate);
                            }
                        } else {
                            self.begin_recovery(Outcome::Canceled, fx);
                        }
                    }
                    DraftReady => {
                        self.phase = Idle;
                        fx.push(Effect::Notify(Outcome::DraftDiscarded));
                    }
                    Idle | Recovering => {
                        if had_draft {
                            fx.push(Effect::Notify(Outcome::DraftDiscarded));
                        }
                    }
                }
            }

            Input::Tick { now_ms } => {
                if self.phase == DraftReady
                    && self
                        .draft
                        .as_ref()
                        .is_some_and(|d| now_ms.saturating_sub(d.created_ms) >= DRAFT_TTL_MS)
                {
                    self.draft = None;
                    self.phase = Idle;
                    fx.push(Effect::Notify(Outcome::DraftExpired));
                }
                self.now_ms = self.now_ms.max(now_ms);
            }
        }
    }

    fn job_matches(&self, job: JobKey) -> bool {
        job.worker_epoch == self.worker_epoch && self.is_live(job.session, job.generation)
    }

    /// Continuous: start the next queued phrase, keep listening, or end the
    /// session once the capture is closed and nothing is left.
    fn start_next(&mut self, fx: &mut Vec<Effect>) {
        let Some(l) = self.live.as_mut() else { return };
        if let Some(segment) = l.queue.pop_front() {
            if l.phrases > 0 {
                l.generation = self.next_generation;
                self.next_generation += 1;
            }
            l.phrases += 1;
            l.ledger = Ledger::NotAttempted;
            l.text = None;
            let job = JobKey {
                session: l.id,
                generation: l.generation,
                worker_epoch: self.worker_epoch,
            };
            self.phase = Phase::Transcribing;
            fx.push(Effect::StartTranscription { job, segment });
        } else if l.capture_open {
            self.phase = if l.finishing {
                Phase::Finalizing
            } else {
                Phase::Listening
            };
        } else {
            let outcome = match l.end_outcome {
                Some(Outcome::Failed(e)) => Some(Outcome::Failed(e)),
                _ if l.pasted > 0 => None,
                other => Some(other.unwrap_or(Outcome::NoUsableText)),
            };
            let id = l.id;
            self.live = None;
            self.phase = Phase::Idle;
            fx.push(Effect::DropAudio { id });
            fx.push(Effect::ReleaseTarget { id });
            if let Some(o) = outcome {
                fx.push(Effect::Notify(o));
            }
        }
    }

    /// Clean terminal outcome with nothing left to close.
    fn finish_clean(&mut self, outcome: Outcome, fx: &mut Vec<Effect>) {
        if let Some(l) = self.live.take() {
            fx.push(Effect::DropAudio { id: l.id });
            fx.push(Effect::ReleaseTarget { id: l.id });
        }
        self.phase = Phase::Idle;
        fx.push(Effect::Notify(outcome));
    }

    /// Invalidate delivery eligibility first, then ask for resources to close.
    fn begin_recovery(&mut self, outcome: Outcome, fx: &mut Vec<Effect>) {
        let Some(l) = self.live.as_mut() else { return };
        l.auto_eligible = false;
        l.text = None;
        l.queue.clear();
        l.closing_outcome = Some(outcome);
        let id = l.id;
        self.phase = Phase::Recovering;
        fx.push(Effect::AbortCapture { id });
        fx.push(Effect::DropAudio { id });
        fx.push(Effect::ReleaseTarget { id });
    }

    fn enter_draft(
        &mut self,
        text: String,
        reason: DraftReason,
        attempted: bool,
        fx: &mut Vec<Effect>,
    ) {
        self.enter_draft_with_revision(text, reason, attempted, None, fx);
    }

    fn enter_draft_with_revision(
        &mut self,
        text: String,
        reason: DraftReason,
        attempted: bool,
        revision: Option<u32>,
        fx: &mut Vec<Effect>,
    ) {
        let Some(l) = self.live.take() else { return };
        if l.continuous {
            // A draft ends a continuous session: stop listening and forget
            // utterances that were not transcribed yet.
            if l.capture_open {
                fx.push(Effect::AbortCapture { id: l.id });
            }
            fx.push(Effect::DropAudio { id: l.id });
        }
        let created_ms = self.draft.as_ref().map_or(self.now_ms, |d| d.created_ms);
        let prior_attempted = self.draft.as_ref().is_some_and(|d| d.attempted);
        self.draft = Some(Draft {
            id: l.id,
            revision: revision.unwrap_or(0),
            text,
            reason,
            created_ms,
            attempted: attempted || prior_attempted,
        });
        self.phase = Phase::DraftReady;
        fx.push(Effect::Notify(Outcome::DraftReady(reason)));
    }
}
