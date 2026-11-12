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
    CaptureFinished {
        id: SessionId,
        generation: Generation,
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

    fn apply(&mut self, input: Input, fx: &mut Vec<Effect>) {
        use Phase::*;
        match input {
            Input::Start {
                id,
                now_ms,
                ready,
                auto_delivery,
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
                        self.live = Some(Live {
                            id,
                            generation,
                            auto_eligible: auto_delivery,
                            ledger: Ledger::NotAttempted,
                            mode: DeliveryMode::Automatic,
                            text: None,
                            draft_revision: None,
                            closing_outcome: None,
                        });
                        self.phase = Starting;
                        fx.push(Effect::OpenCapture { id, generation });
                    }
                    // Never discard a draft silently (docs/02 §4).
                    DraftReady => {
                        fx.push(Effect::Notify(Outcome::Rejected(AppError::DraftPending)))
                    }
                    _ => fx.push(Effect::Notify(Outcome::Rejected(AppError::Busy))),
                }
            }

            Input::CaptureOpened { id, generation } => {
                if self.phase == Starting && self.is_live(id, generation) {
                    self.phase = Listening;
                }
            }

            Input::CaptureFailed {
                id,
                generation,
                error,
            } => {
                if matches!(self.phase, Starting | Listening | Finalizing)
                    && self.is_live(id, generation)
                {
                    self.begin_recovery(Outcome::Failed(error), fx);
                }
            }

            Input::Stop { id } => {
                // First finish request wins; later ones are no-ops.
                if self.phase == Listening
                    && let Some(l) = self.live.as_ref().filter(|l| l.id == id)
                {
                    let generation = l.generation;
                    self.phase = Finalizing;
                    fx.push(Effect::FinishCapture { id, generation });
                }
            }

            Input::CaptureFinished {
                id,
                generation,
                outcome,
            } => {
                if !matches!(self.phase, Listening | Finalizing) || !self.is_live(id, generation) {
                    return;
                }
                match outcome {
                    CaptureOutcome::Speech => {
                        self.phase = Transcribing;
                        let job = JobKey {
                            session: id,
                            generation,
                            worker_epoch: self.worker_epoch,
                        };
                        fx.push(Effect::StartTranscription { job });
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
                fx.push(Effect::DropAudio { id: job.session });
                let text = text.trim().to_owned();
                if text.is_empty() {
                    self.finish_clean(Outcome::NoUsableText, fx);
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
                if let Some(l) = self.live.as_mut()
                    && matches!(self.phase, Starting | Listening | Finalizing | Transcribing)
                {
                    l.auto_eligible = false;
                    l.closing_outcome = Some(Outcome::DraftReady(DraftReason::ReviewOpened));
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
                self.live = Some(Live {
                    id,
                    generation,
                    auto_eligible: false,
                    ledger: Ledger::NotAttempted,
                    mode: DeliveryMode::Explicit,
                    text: Some(text.clone()),
                    draft_revision: Some(revision),
                    closing_outcome: None,
                });
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
