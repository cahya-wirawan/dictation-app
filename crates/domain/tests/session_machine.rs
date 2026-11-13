//! Session state machine fixtures (work package A1, docs/04 §1–2, docs/08 §2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};

use dictation_domain::Phase;
use dictation_domain::SessionId;
use dictation_domain::session::*;

fn sid(n: u8) -> SessionId {
    SessionId::from_bytes([n; 16])
}

fn start(m: &mut Machine, n: u8) -> Vec<Effect> {
    m.step(Input::Start {
        id: sid(n),
        now_ms: 1_000,
        ready: Ok(()),
        auto_delivery: true,
        continuous: false,
    })
}

/// Drives a session to `Transcribing`; returns its job key.
fn to_transcribing(m: &mut Machine, n: u8) -> JobKey {
    start(m, n);
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    let fx = m.step(Input::CaptureFinished {
        id,
        generation,
        segment: 0,
        outcome: CaptureOutcome::Speech,
    });
    match fx
        .iter()
        .find(|e| matches!(e, Effect::StartTranscription { .. }))
    {
        Some(Effect::StartTranscription { job, .. }) => *job,
        _ => panic!("no StartTranscription in {fx:?}"),
    }
}

fn to_delivering(m: &mut Machine, n: u8) -> (SessionId, u64) {
    let job = to_transcribing(m, n);
    m.step(Input::TranscriptionDone {
        job,
        text: " Hallo Welt ".into(),
    });
    assert_eq!(m.phase(), Phase::Delivering);
    (job.session, job.generation)
}

fn has(fx: &[Effect], f: impl Fn(&Effect) -> bool) -> bool {
    fx.iter().any(f)
}

fn notified(fx: &[Effect]) -> Option<Outcome> {
    fx.iter().find_map(|e| {
        if let Effect::Notify(o) = e {
            Some(*o)
        } else {
            None
        }
    })
}

#[test]
fn happy_path_automatic_delivery() {
    let mut m = Machine::new(0);
    let fx = start(&mut m, 1);
    assert_eq!(m.phase(), Phase::Starting);
    assert!(has(&fx, |e| matches!(e, Effect::OpenCapture { .. })));
    assert!(has(&fx, |e| *e == Effect::PhaseChanged(Phase::Starting)));

    let (id, generation) = to_delivering(&mut m, 1);
    let fx = m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Ok(()),
    });
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::DispatchPaste { .. }))
            .count(),
        1
    );
    assert_eq!(m.ledger(), Some(Ledger::Committed));

    let fx = m.step(Input::DispatchFinished {
        id,
        generation,
        result: DispatchResult::Sent,
    });
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(notified(&fx), Some(Outcome::PasteSent));
    assert!(m.draft().is_none());
}

#[test]
fn delivery_text_is_trimmed_and_carried() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    let fx = m.step(Input::TranscriptionDone {
        job,
        text: "  Grüß Gott 👋\n".into(),
    });
    assert!(has(
        &fx,
        |e| matches!(e, Effect::PrepareDelivery { text, mode: DeliveryMode::Automatic, .. } if text == "Grüß Gott 👋")
    ));
}

#[test]
fn start_rejections_do_not_change_state() {
    let mut m = Machine::new(0);
    let fx = m.step(Input::Start {
        id: sid(1),
        now_ms: 0,
        ready: Err(AppError::ModelUnavailable),
        auto_delivery: true,
        continuous: false,
    });
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(
        notified(&fx),
        Some(Outcome::Rejected(AppError::ModelUnavailable))
    );
    assert!(!has(&fx, |e| matches!(e, Effect::OpenCapture { .. })));

    start(&mut m, 1);
    let fx = start(&mut m, 2);
    assert_eq!(notified(&fx), Some(Outcome::Rejected(AppError::Busy)));
    assert_eq!(m.live_session().unwrap().0, sid(1));
}

#[test]
fn start_with_pending_draft_is_refused_not_discarded() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    m.step(Input::ReviewOpened);
    m.step(Input::TranscriptionDone {
        job,
        text: "keep me".into(),
    });
    assert_eq!(m.phase(), Phase::DraftReady);
    let fx = start(&mut m, 2);
    assert_eq!(
        notified(&fx),
        Some(Outcome::Rejected(AppError::DraftPending))
    );
    assert_eq!(m.draft().unwrap().text, "keep me");
}

#[test]
fn stop_and_endpoint_race_yields_one_job() {
    let mut m = Machine::new(0);
    start(&mut m, 1);
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    let a = m.step(Input::Stop { id });
    let b = m.step(Input::Stop { id });
    assert_eq!(
        a.iter()
            .filter(|e| matches!(e, Effect::FinishCapture { .. }))
            .count(),
        1
    );
    assert!(!has(&b, |e| matches!(e, Effect::FinishCapture { .. })));
    let c = m.step(Input::CaptureFinished {
        id,
        generation,
        segment: 0,
        outcome: CaptureOutcome::Speech,
    });
    let d = m.step(Input::CaptureFinished {
        id,
        generation,
        segment: 0,
        outcome: CaptureOutcome::Speech,
    });
    let jobs = c
        .iter()
        .chain(&d)
        .filter(|e| matches!(e, Effect::StartTranscription { .. }))
        .count();
    assert_eq!(jobs, 1);
}

#[test]
fn no_speech_never_reaches_inference_clipboard_or_input() {
    for outcome in [CaptureOutcome::NoSpeech, CaptureOutcome::TooShort] {
        let mut m = Machine::new(0);
        start(&mut m, 1);
        let (id, generation) = m.live_session().unwrap();
        m.step(Input::CaptureOpened { id, generation });
        let fx = m.step(Input::CaptureFinished {
            id,
            generation,
            segment: 0,
            outcome,
        });
        assert!(!has(&fx, |e| matches!(
            e,
            Effect::StartTranscription { .. }
                | Effect::PrepareDelivery { .. }
                | Effect::CopyText { .. }
                | Effect::DispatchPaste { .. }
        )));
        assert_eq!(m.phase(), Phase::Idle);
        let expected = if outcome == CaptureOutcome::NoSpeech {
            Outcome::NoSpeech
        } else {
            Outcome::NoUsableText
        };
        assert_eq!(notified(&fx), Some(expected));
    }
}

#[test]
fn empty_transcript_is_no_usable_text() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    let fx = m.step(Input::TranscriptionDone {
        job,
        text: " \n\t ".into(),
    });
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(notified(&fx), Some(Outcome::NoUsableText));
    assert!(!has(&fx, |e| matches!(e, Effect::PrepareDelivery { .. })));
}

#[test]
fn stale_inputs_are_ignored() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    let snapshot = m.clone();
    // Wrong generation, wrong session, wrong epoch.
    for bad in [
        JobKey {
            generation: job.generation + 1,
            ..job
        },
        JobKey {
            session: sid(9),
            ..job
        },
        JobKey {
            worker_epoch: job.worker_epoch + 1,
            ..job
        },
    ] {
        assert!(
            m.step(Input::TranscriptionDone {
                job: bad,
                text: "x".into()
            })
            .is_empty()
        );
        assert_eq!(m, snapshot);
    }
    // After the worker is replaced, the old epoch's result is stale.
    m.step(Input::WorkerEpochChanged { epoch: 1 });
    assert!(
        m.step(Input::TranscriptionDone {
            job,
            text: "late".into()
        })
        .is_empty()
    );
    assert_eq!(m.phase(), Phase::Transcribing);
}

#[test]
fn cancel_while_transcribing_kills_worker_and_discards_late_result() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    let fx = m.step(Input::Cancel);
    assert!(has(&fx, |e| *e == Effect::KillWorker));
    assert!(has(&fx, |e| matches!(e, Effect::AbortCapture { .. })));
    assert_eq!(m.phase(), Phase::Recovering);
    assert!(
        m.step(Input::TranscriptionDone {
            job,
            text: "late".into()
        })
        .is_empty()
    );
    let fx = m.step(Input::ResourcesClosed { id: job.session });
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(notified(&fx), Some(Outcome::Canceled));
}

#[test]
fn cancel_before_commit_blocks_dispatch() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::Cancel);
    let fx = m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Ok(()),
    });
    assert!(!has(&fx, |e| matches!(e, Effect::DispatchPaste { .. })));
    assert_eq!(m.phase(), Phase::Recovering);
}

#[test]
fn cancel_after_commit_reports_honestly() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Ok(()),
    });
    let fx = m.step(Input::Cancel);
    assert!(fx.is_empty(), "cannot retract a committed send: {fx:?}");
    let fx = m.step(Input::DispatchFinished {
        id,
        generation,
        result: DispatchResult::Sent,
    });
    assert_eq!(notified(&fx), Some(Outcome::CancelTooLate));
    assert_eq!(m.phase(), Phase::Idle);
}

#[test]
fn blocked_delivery_keeps_draft_with_reason() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    let fx = m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::TargetChanged),
    });
    assert_eq!(m.phase(), Phase::DraftReady);
    assert_eq!(
        notified(&fx),
        Some(Outcome::DraftReady(DraftReason::Blocked(
            AppError::TargetChanged
        )))
    );
    assert!(has(&fx, |e| matches!(e, Effect::ReleaseTarget { .. })));
    let d = m.draft().unwrap();
    assert_eq!(
        (d.text.as_str(), d.revision, d.attempted),
        ("Hallo Welt", 0, false)
    );
}

#[test]
fn uncertain_dispatch_never_retries_and_marks_attempted() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Ok(()),
    });
    // A duplicate prepare after commit must not dispatch again.
    let dup = m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Ok(()),
    });
    assert!(!has(&dup, |e| matches!(e, Effect::DispatchPaste { .. })));
    let fx = m.step(Input::DispatchFinished {
        id,
        generation,
        result: DispatchResult::Uncertain,
    });
    assert!(!has(&fx, |e| matches!(
        e,
        Effect::DispatchPaste { .. } | Effect::PrepareDelivery { .. }
    )));
    assert_eq!(m.phase(), Phase::DraftReady);
    assert!(m.draft().unwrap().attempted);

    // Explicit paste shortcut is the only way to resend.
    let fx = m.step(Input::DraftPaste { id, revision: 0 });
    assert!(has(&fx, |e| matches!(
        e,
        Effect::PrepareDelivery {
            mode: DeliveryMode::Explicit,
            ..
        }
    )));
    let (id2, gen2) = m.live_session().unwrap();
    assert_eq!(id2, id);
    assert!(
        gen2 > generation,
        "explicit delivery gets a fresh generation"
    );
    // Old-generation dispatch results are stale.
    assert!(
        m.step(Input::DispatchFinished {
            id,
            generation,
            result: DispatchResult::Sent
        })
        .is_empty()
    );
    m.step(Input::DeliveryPrepared {
        id,
        generation: gen2,
        result: Ok(()),
    });
    let fx = m.step(Input::DispatchFinished {
        id,
        generation: gen2,
        result: DispatchResult::Sent,
    });
    assert_eq!(notified(&fx), Some(Outcome::PasteSent));
    assert!(m.draft().is_none());
}

#[test]
fn explicit_paste_cancel_before_commit_keeps_draft() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::TargetChanged),
    });
    m.step(Input::DraftPaste { id, revision: 0 });
    m.step(Input::Cancel);
    assert_eq!(m.phase(), Phase::DraftReady);
    assert_eq!(m.draft().unwrap().text, "Hallo Welt");
}

#[test]
fn copy_only_readiness_goes_to_draft() {
    let mut m = Machine::new(0);
    m.step(Input::Start {
        id: sid(1),
        now_ms: 0,
        ready: Ok(()),
        auto_delivery: false,
        continuous: false,
    });
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    m.step(Input::CaptureFinished {
        id,
        generation,
        segment: 0,
        outcome: CaptureOutcome::Speech,
    });
    let fx = m.step(Input::TranscriptionDone {
        job: JobKey {
            session: id,
            generation,
            worker_epoch: 0,
        },
        text: "t".into(),
    });
    assert!(!has(&fx, |e| matches!(e, Effect::PrepareDelivery { .. })));
    assert_eq!(
        notified(&fx),
        Some(Outcome::DraftReady(DraftReason::ReviewMode))
    );
}

#[test]
fn review_opened_revokes_automatic_delivery() {
    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 1);
    m.step(Input::ReviewOpened);
    let fx = m.step(Input::TranscriptionDone {
        job,
        text: "t".into(),
    });
    assert!(!has(&fx, |e| matches!(e, Effect::PrepareDelivery { .. })));
    assert_eq!(
        notified(&fx),
        Some(Outcome::DraftReady(DraftReason::ReviewOpened))
    );
}

#[test]
fn draft_edit_copy_and_discard() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::PasteUnsupported),
    });

    let fx = m.step(Input::DraftEdit {
        id,
        expected_revision: 5,
        text: "x".into(),
    });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Rejected(AppError::StaleRequest))
    );
    m.step(Input::DraftEdit {
        id,
        expected_revision: 0,
        text: "edited".into(),
    });
    assert_eq!(m.draft().unwrap().revision, 1);
    let too_big = "a".repeat(MAX_DRAFT_BYTES + 1);
    m.step(Input::DraftEdit {
        id,
        expected_revision: 1,
        text: too_big,
    });
    assert_eq!(m.draft().unwrap().text, "edited");

    // Stale revision copy/paste are rejected.
    let fx = m.step(Input::DraftCopy { id, revision: 0 });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Rejected(AppError::StaleRequest))
    );
    let fx = m.step(Input::DraftPaste { id, revision: 0 });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Rejected(AppError::StaleRequest))
    );

    let fx = m.step(Input::DraftCopy { id, revision: 1 });
    assert!(has(
        &fx,
        |e| matches!(e, Effect::CopyText { text, .. } if text == "edited")
    ));
    let fx = m.step(Input::DraftCopyFinished {
        id,
        revision: 1,
        ok: false,
    });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Failed(AppError::ClipboardBusy))
    );
    assert_eq!(m.phase(), Phase::DraftReady);
    let fx = m.step(Input::DraftCopyFinished {
        id,
        revision: 1,
        ok: true,
    });
    assert_eq!(notified(&fx), Some(Outcome::DraftCopied));
    assert_eq!(m.phase(), Phase::Idle);
    assert!(m.draft().is_none());

    // Discard path.
    let (id, generation) = to_delivering(&mut m, 2);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::TargetChanged),
    });
    let fx = m.step(Input::DraftDiscard { id });
    assert_eq!(notified(&fx), Some(Outcome::DraftDiscarded));
    assert!(
        !has(&fx, |e| matches!(e, Effect::CopyText { .. })),
        "discard never touches the clipboard"
    );
}

#[test]
fn draft_expires_after_ttl() {
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1); // Start at now_ms=1000
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::TargetChanged),
    });
    assert!(
        m.step(Input::Tick {
            now_ms: 1_000 + DRAFT_TTL_MS - 1
        })
        .is_empty()
    );
    let fx = m.step(Input::Tick {
        now_ms: 1_000 + DRAFT_TTL_MS,
    });
    assert_eq!(notified(&fx), Some(Outcome::DraftExpired));
    assert_eq!(m.phase(), Phase::Idle);
}

#[test]
fn suspend_cancels_every_phase_and_clears_draft() {
    // Listening
    let mut m = Machine::new(0);
    start(&mut m, 1);
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    let fx = m.step(Input::Suspend);
    assert!(has(&fx, |e| matches!(e, Effect::AbortCapture { .. })));
    assert_eq!(m.phase(), Phase::Recovering);
    // Transcribing
    let mut m = Machine::new(0);
    to_transcribing(&mut m, 1);
    assert!(has(&m.step(Input::Suspend), |e| *e == Effect::KillWorker));
    // DraftReady
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::DeliveryPrepared {
        id,
        generation,
        result: Err(AppError::TargetChanged),
    });
    let fx = m.step(Input::Suspend);
    assert_eq!(m.phase(), Phase::Idle);
    assert!(m.draft().is_none());
    assert_eq!(notified(&fx), Some(Outcome::DraftDiscarded));
    // Delivering before commit: no dispatch afterwards.
    let mut m = Machine::new(0);
    let (id, generation) = to_delivering(&mut m, 1);
    m.step(Input::Suspend);
    assert!(!has(
        &m.step(Input::DeliveryPrepared {
            id,
            generation,
            result: Ok(())
        }),
        |e| matches!(e, Effect::DispatchPaste { .. })
    ));
}

#[test]
fn capture_and_worker_failures_recover_to_idle() {
    let mut m = Machine::new(0);
    start(&mut m, 1);
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    m.step(Input::CaptureFailed {
        id,
        generation,
        error: AppError::MicDisconnected,
    });
    assert_eq!(m.phase(), Phase::Recovering);
    let fx = m.step(Input::ResourcesClosed { id });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Failed(AppError::MicDisconnected))
    );

    let job = to_transcribing(&mut m, 2);
    let fx = m.step(Input::TranscriptionFailed {
        job,
        error: AppError::InferenceTimeout,
    });
    assert!(has(&fx, |e| *e == Effect::KillWorker));
    let fx = m.step(Input::ResourcesClosed { id: job.session });
    assert_eq!(
        notified(&fx),
        Some(Outcome::Failed(AppError::InferenceTimeout))
    );
}

#[test]
fn generations_increase_across_sessions() {
    let mut m = Machine::new(0);
    let a = to_transcribing(&mut m, 1);
    m.step(Input::Cancel);
    m.step(Input::ResourcesClosed { id: a.session });
    let b = to_transcribing(&mut m, 1); // even with a reused id
    assert!(b.generation > a.generation);
    // The canceled session's result can't land in the new one.
    assert!(
        m.step(Input::TranscriptionDone {
            job: a,
            text: "old".into()
        })
        .is_empty()
    );
}

/// Seeded random input sequences; checks safety invariants after every step.
#[test]
fn fuzz_invariants_hold() {
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move |n: u64| {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 33) % n
    };
    let mut total_dispatch = 0;
    let mut total_precommit_cancels = 0;
    let mut chained_phrases = 0;
    for _run in 0..400 {
        let mut m = Machine::new(0);
        let mut epoch = 0u64;
        let mut now = 0u64;
        let mut dispatched: HashMap<(SessionId, u64), u32> = HashMap::new();
        let mut canceled_precommit: HashSet<(SessionId, u64)> = HashSet::new();
        let mut speech_finished: HashSet<(SessionId, u64)> = HashSet::new();
        let mut accepted: HashSet<(SessionId, u32)> = HashSet::new();
        let mut transcribed: HashSet<(SessionId, u32)> = HashSet::new();
        for seg in 0..60u32 {
            let live = m.live_session();
            // Mostly target the live session, sometimes a stale/foreign one.
            let (id, generation) = match (live, next(5)) {
                (Some(l), 0..=3) => l,
                _ => (sid(next(4) as u8), next(6)),
            };
            let job = JobKey {
                session: id,
                generation,
                worker_epoch: if next(4) == 0 { epoch + 1 } else { epoch },
            };
            // Capture events are fenced on the capture generation, which a
            // continuous session keeps while its phrases get new ones.
            let (cid, cgen) = if live == Some((id, generation)) {
                m.capture_session().unwrap_or((id, generation))
            } else {
                (id, generation)
            };
            let rev = m.draft().map_or(0, |d| d.revision);
            let draft_id = m.draft().map_or(id, |d| d.id);
            now += next(200_000);
            // ~60%: the input that legally advances the current phase;
            // otherwise anything (stale ids, races, cancels, draft actions).
            let advance = if next(10) < 6 {
                match (m.phase(), live) {
                    (Phase::Idle, _) => Some(Input::Start {
                        id: sid(next(4) as u8),
                        now_ms: now,
                        ready: Ok(()),
                        auto_delivery: next(4) != 0,
                        continuous: next(2) == 0,
                    }),
                    (Phase::Starting, Some(_)) => Some(Input::CaptureOpened {
                        id: cid,
                        generation: cgen,
                    }),
                    (Phase::Listening | Phase::Finalizing, Some((id, _))) => {
                        Some(if next(3) == 0 {
                            Input::Stop { id }
                        } else if m.continuous() && next(4) != 0 {
                            Input::CaptureSegment {
                                id: cid,
                                generation: cgen,
                                segment: seg,
                                outcome: if next(5) == 0 {
                                    CaptureOutcome::TooShort
                                } else {
                                    CaptureOutcome::Speech
                                },
                            }
                        } else {
                            Input::CaptureFinished {
                                id: cid,
                                generation: cgen,
                                segment: seg,
                                outcome: if next(5) == 0 {
                                    CaptureOutcome::NoSpeech
                                } else {
                                    CaptureOutcome::Speech
                                },
                            }
                        })
                    }
                    (Phase::Transcribing | Phase::Delivering, Some(_))
                        if m.capture_session().is_some() && next(3) == 0 =>
                    {
                        Some(Input::CaptureSegment {
                            id: cid,
                            generation: cgen,
                            segment: seg,
                            outcome: CaptureOutcome::Speech,
                        })
                    }
                    (Phase::Transcribing, Some((id, generation))) => {
                        Some(Input::TranscriptionDone {
                            job: JobKey {
                                session: id,
                                generation,
                                worker_epoch: m.worker_epoch(),
                            },
                            text: "text".into(),
                        })
                    }
                    (Phase::Delivering, Some((id, generation))) => Some(match m.ledger() {
                        Some(Ledger::NotAttempted) => Input::DeliveryPrepared {
                            id,
                            generation,
                            result: if next(4) == 0 {
                                Err(AppError::TargetChanged)
                            } else {
                                Ok(())
                            },
                        },
                        _ => Input::DispatchFinished {
                            id,
                            generation,
                            result: if next(4) == 0 {
                                DispatchResult::Uncertain
                            } else {
                                DispatchResult::Sent
                            },
                        },
                    }),
                    (Phase::Recovering, Some((id, _))) => Some(Input::ResourcesClosed { id }),
                    (Phase::DraftReady, _) => Some(match next(3) {
                        0 => Input::DraftPaste {
                            id: draft_id,
                            revision: rev,
                        },
                        1 => Input::DraftCopy {
                            id: draft_id,
                            revision: rev,
                        },
                        _ => Input::DraftDiscard { id: draft_id },
                    }),
                    _ => None,
                }
            } else {
                None
            };
            let input = if let Some(i) = advance {
                i
            } else {
                match next(22) {
                    0 | 1 => Input::Start {
                        id: sid(next(4) as u8),
                        now_ms: now,
                        ready: if next(6) == 0 {
                            Err(AppError::NotReady)
                        } else {
                            Ok(())
                        },
                        auto_delivery: next(4) != 0,
                        continuous: next(2) == 0,
                    },
                    2 => Input::CaptureOpened {
                        id: cid,
                        generation: cgen,
                    },
                    3 => Input::CaptureFailed {
                        id: cid,
                        generation: cgen,
                        error: AppError::MicDisconnected,
                    },
                    4 => Input::Stop { id },
                    5 => Input::CaptureFinished {
                        id: cid,
                        generation: cgen,
                        segment: seg,
                        outcome: [
                            CaptureOutcome::Speech,
                            CaptureOutcome::NoSpeech,
                            CaptureOutcome::TooShort,
                        ][next(3) as usize],
                    },
                    6 => Input::TranscriptionDone {
                        job,
                        text: if next(5) == 0 {
                            "  ".into()
                        } else {
                            "text".into()
                        },
                    },
                    7 => Input::TranscriptionFailed {
                        job,
                        error: AppError::WorkerCrash,
                    },
                    8 => {
                        epoch += 1;
                        Input::WorkerEpochChanged { epoch }
                    }
                    9 => Input::DeliveryPrepared {
                        id,
                        generation,
                        result: if next(3) == 0 {
                            Err(AppError::TargetChanged)
                        } else {
                            Ok(())
                        },
                    },
                    10 => Input::DispatchFinished {
                        id,
                        generation,
                        result: if next(3) == 0 {
                            DispatchResult::Uncertain
                        } else {
                            DispatchResult::Sent
                        },
                    },
                    11 | 12 => Input::Cancel,
                    13 => Input::ResourcesClosed { id },
                    14 => Input::ReviewOpened,
                    15 => Input::DraftCopy {
                        id: draft_id,
                        revision: rev,
                    },
                    16 => Input::DraftPaste {
                        id: draft_id,
                        revision: rev,
                    },
                    17 => Input::DraftDiscard { id: draft_id },
                    18 => Input::Suspend,
                    19 | 20 => Input::CaptureSegment {
                        id: cid,
                        generation: cgen,
                        segment: seg,
                        outcome: [CaptureOutcome::Speech, CaptureOutcome::TooShort]
                            [next(2) as usize],
                    },
                    _ => Input::Tick { now_ms: now },
                }
            };
            let phase_before = m.phase();
            let ledger_before = m.ledger();
            let live_before = m.live_session();
            let is_cancel = matches!(input, Input::Cancel | Input::Suspend);
            let chains = matches!(
                input,
                Input::TranscriptionDone { .. } | Input::DispatchFinished { .. }
            );
            if let Input::CaptureFinished {
                id,
                segment,
                outcome: CaptureOutcome::Speech,
                ..
            }
            | Input::CaptureSegment {
                id,
                segment,
                outcome: CaptureOutcome::Speech,
                ..
            } = input
            {
                accepted.insert((id, segment));
            }
            let fx = m.step(input);

            if is_cancel
                && phase_before == Phase::Delivering
                && ledger_before == Some(Ledger::NotAttempted)
                && let Some(l) = live_before
            {
                canceled_precommit.insert(l);
                total_precommit_cancels += 1;
            }
            for e in &fx {
                match e {
                    Effect::DispatchPaste { id, generation } => {
                        let n = dispatched.entry((*id, *generation)).or_default();
                        *n += 1;
                        total_dispatch += 1;
                        assert_eq!(*n, 1, "duplicate dispatch for one delivery transaction");
                        assert!(
                            !canceled_precommit.contains(&(*id, *generation)),
                            "dispatch after pre-commit cancel"
                        );
                        assert_eq!(phase_before, Phase::Delivering);
                    }
                    Effect::StartTranscription { job, segment } => {
                        assert!(
                            accepted.contains(&(job.session, *segment)),
                            "inference without accepted speech"
                        );
                        assert!(
                            transcribed.insert((job.session, *segment)),
                            "one utterance transcribed twice"
                        );
                        assert!(
                            speech_finished.insert((job.session, job.generation)),
                            "two phrases share a generation"
                        );
                        if chains {
                            chained_phrases += 1;
                        }
                    }
                    Effect::OpenCapture { .. } => {
                        assert_eq!(phase_before, Phase::Idle, "capture opened while busy")
                    }
                    Effect::PrepareDelivery {
                        mode: DeliveryMode::Automatic,
                        id,
                        generation,
                        ..
                    } => {
                        assert!(
                            speech_finished.contains(&(*id, *generation)),
                            "auto delivery without a transcription job"
                        );
                    }
                    _ => {}
                }
            }
            // Structural invariants.
            if matches!(m.phase(), Phase::Idle | Phase::DraftReady) {
                assert!(
                    m.live_session().is_none(),
                    "{:?} with a live session",
                    m.phase()
                );
            }
            if m.phase() == Phase::DraftReady {
                assert!(m.draft().is_some(), "DraftReady without draft");
            }
            if m.phase() == Phase::Idle {
                assert!(m.draft().is_none(), "Idle with a pending draft");
            }
            if matches!(
                m.phase(),
                Phase::Starting
                    | Phase::Listening
                    | Phase::Finalizing
                    | Phase::Transcribing
                    | Phase::Delivering
                    | Phase::Recovering
            ) {
                assert!(
                    m.live_session().is_some(),
                    "busy phase {:?} without live session",
                    m.phase()
                );
            }
        }
    }
    eprintln!(
        "fuzz: {total_dispatch} dispatches, {total_precommit_cancels} pre-commit cancels, \
         {chained_phrases} continuous follow-up phrases"
    );
    assert!(
        chained_phrases > 50,
        "fuzz should exercise continuous sessions; got {chained_phrases}"
    );
    assert!(
        total_dispatch > 500,
        "fuzz should exercise dispatch; got {total_dispatch}"
    );
    assert!(
        total_precommit_cancels > 20,
        "fuzz should exercise pre-commit cancel; got {total_precommit_cancels}"
    );
}

// ---- Continuous dictation (ADR-014) ----

fn start_continuous(m: &mut Machine, n: u8) -> (SessionId, u64) {
    let fx = m.step(Input::Start {
        id: sid(n),
        now_ms: 1_000,
        ready: Ok(()),
        auto_delivery: true,
        continuous: true,
    });
    assert!(has(&fx, |e| matches!(
        e,
        Effect::OpenCapture {
            continuous: true,
            ..
        }
    )));
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    assert_eq!(m.phase(), Phase::Listening);
    assert!(m.continuous());
    (id, generation)
}

fn segment(m: &mut Machine, (id, generation): (SessionId, u64), segment: u32) -> Vec<Effect> {
    m.step(Input::CaptureSegment {
        id,
        generation,
        segment,
        outcome: CaptureOutcome::Speech,
    })
}

fn job_of(fx: &[Effect]) -> Option<(JobKey, u32)> {
    fx.iter().find_map(|e| match e {
        Effect::StartTranscription { job, segment } => Some((*job, *segment)),
        _ => None,
    })
}

/// Transcribe → prepare → dispatch one phrase; returns the dispatch effects.
fn deliver(m: &mut Machine, job: JobKey) -> Vec<Effect> {
    let fx = m.step(Input::TranscriptionDone {
        job,
        text: "phrase".into(),
    });
    assert!(has(&fx, |e| matches!(e, Effect::PrepareDelivery { .. })));
    let fx = m.step(Input::DeliveryPrepared {
        id: job.session,
        generation: job.generation,
        result: Ok(()),
    });
    assert!(has(&fx, |e| matches!(e, Effect::DispatchPaste { .. })));
    m.step(Input::DispatchFinished {
        id: job.session,
        generation: job.generation,
        result: DispatchResult::Sent,
    })
}

#[test]
fn continuous_pastes_each_phrase_in_order_and_keeps_listening() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);

    let (job1, s1) = job_of(&segment(&mut m, cap, 0)).unwrap();
    assert_eq!(
        (s1, job1.generation),
        (0, cap.1),
        "first phrase uses the capture generation"
    );
    assert_eq!(m.phase(), Phase::Transcribing);
    assert_eq!(m.capture_session(), Some(cap), "microphone stays open");

    // The next utterance ends while the first is still transcribing: queued.
    let fx = segment(&mut m, cap, 1);
    assert!(job_of(&fx).is_none());
    assert!(m.is_queued(cap.0, 1));

    let fx = deliver(&mut m, job1);
    assert_eq!(notified(&fx), Some(Outcome::PasteSent));
    assert!(
        !has(&fx, |e| matches!(e, Effect::ReleaseTarget { .. })),
        "the target token is kept for the next phrase"
    );
    let (job2, s2) = job_of(&fx).expect("queued phrase starts right after the paste");
    assert_eq!(s2, 1);
    assert!(
        job2.generation > job1.generation,
        "every phrase is its own transaction"
    );
    assert_eq!(m.ledger(), Some(Ledger::NotAttempted));

    // A late duplicate for phrase 1 is stale.
    assert!(
        m.step(Input::DispatchFinished {
            id: job1.session,
            generation: job1.generation,
            result: DispatchResult::Sent,
        })
        .is_empty()
    );

    let fx = deliver(&mut m, job2);
    assert_eq!(notified(&fx), Some(Outcome::PasteSent));
    assert_eq!(m.phase(), Phase::Listening);

    // Toggle: finish the capture; the trailing silence ends the session.
    let fx = m.step(Input::Stop { id: cap.0 });
    assert!(has(&fx, |e| *e
        == Effect::FinishCapture {
            id: cap.0,
            generation: cap.1
        }));
    assert_eq!(m.phase(), Phase::Finalizing);
    let fx = m.step(Input::CaptureFinished {
        id: cap.0,
        generation: cap.1,
        segment: 2,
        outcome: CaptureOutcome::NoSpeech,
    });
    assert_eq!(m.phase(), Phase::Idle);
    assert!(has(&fx, |e| *e == Effect::ReleaseTarget { id: cap.0 }));
    assert_eq!(
        notified(&fx),
        None,
        "phrases were pasted: no 'no speech' banner"
    );
}

#[test]
fn continuous_no_speech_timeout_without_any_phrase_reports_no_speech() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let fx = m.step(Input::CaptureFinished {
        id: cap.0,
        generation: cap.1,
        segment: 0,
        outcome: CaptureOutcome::NoSpeech,
    });
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(notified(&fx), Some(Outcome::NoSpeech));
}

#[test]
fn continuous_stop_while_transcribing_still_pastes_the_final_utterance() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let (job1, _) = job_of(&segment(&mut m, cap, 0)).unwrap();

    let fx = m.step(Input::Stop { id: cap.0 });
    assert!(has(&fx, |e| matches!(e, Effect::FinishCapture { .. })));
    assert_eq!(
        m.phase(),
        Phase::Transcribing,
        "the phrase in flight continues"
    );
    assert!(
        m.step(Input::Stop { id: cap.0 }).is_empty(),
        "second stop is a no-op"
    );

    // The final utterance arrives with the terminal capture event.
    m.step(Input::CaptureFinished {
        id: cap.0,
        generation: cap.1,
        segment: 1,
        outcome: CaptureOutcome::Speech,
    });
    let fx = deliver(&mut m, job1);
    let (job2, s2) = job_of(&fx).unwrap();
    assert_eq!(s2, 1);
    let fx = deliver(&mut m, job2);
    assert_eq!(m.phase(), Phase::Idle);
    assert!(has(&fx, |e| *e == Effect::ReleaseTarget { id: cap.0 }));
}

#[test]
fn continuous_noise_is_skipped_and_listening_continues() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let tooshort = m.step(Input::CaptureSegment {
        id: cap.0,
        generation: cap.1,
        segment: 0,
        outcome: CaptureOutcome::TooShort,
    });
    assert!(job_of(&tooshort).is_none());
    assert_eq!(m.phase(), Phase::Listening);

    let (job, _) = job_of(&segment(&mut m, cap, 1)).unwrap();
    let fx = m.step(Input::TranscriptionDone {
        job,
        text: "   ".into(),
    });
    assert_eq!(m.phase(), Phase::Listening);
    assert_eq!(notified(&fx), None);
}

#[test]
fn continuous_target_change_ends_the_session_with_a_draft() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let (job, _) = job_of(&segment(&mut m, cap, 0)).unwrap();
    segment(&mut m, cap, 1); // queued, never transcribed
    m.step(Input::TranscriptionDone {
        job,
        text: "phrase".into(),
    });
    let fx = m.step(Input::DeliveryPrepared {
        id: cap.0,
        generation: job.generation,
        result: Err(AppError::TargetChanged),
    });
    assert_eq!(m.phase(), Phase::DraftReady);
    assert_eq!(m.draft().unwrap().text, "phrase");
    assert!(
        has(&fx, |e| *e == Effect::AbortCapture { id: cap.0 }),
        "microphone closes"
    );
    assert!(
        has(&fx, |e| *e == Effect::DropAudio { id: cap.0 }),
        "queued audio is dropped"
    );
    assert!(!has(&fx, |e| matches!(e, Effect::DispatchPaste { .. })));
    assert!(!m.is_queued(cap.0, 1));
}

#[test]
fn continuous_cancel_after_commit_stops_everything_after_that_phrase() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let (job, _) = job_of(&segment(&mut m, cap, 0)).unwrap();
    segment(&mut m, cap, 1);
    m.step(Input::TranscriptionDone {
        job,
        text: "phrase".into(),
    });
    m.step(Input::DeliveryPrepared {
        id: cap.0,
        generation: job.generation,
        result: Ok(()),
    });
    assert!(
        m.step(Input::Cancel).is_empty(),
        "the committed send cannot be retracted"
    );
    let fx = m.step(Input::DispatchFinished {
        id: cap.0,
        generation: job.generation,
        result: DispatchResult::Sent,
    });
    assert_eq!(m.phase(), Phase::Recovering);
    assert!(job_of(&fx).is_none(), "no phrase after a cancel");
    assert!(has(&fx, |e| *e == Effect::AbortCapture { id: cap.0 }));
    let fx = m.step(Input::ResourcesClosed { id: cap.0 });
    assert_eq!(notified(&fx), Some(Outcome::CancelTooLate));
}

#[test]
fn continuous_capture_fault_lets_the_phrase_finish_then_reports() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let (job, _) = job_of(&segment(&mut m, cap, 0)).unwrap();
    m.step(Input::CaptureFailed {
        id: cap.0,
        generation: cap.1,
        error: AppError::MicDisconnected,
    });
    assert_eq!(m.phase(), Phase::Transcribing);
    let fx = deliver(&mut m, job);
    assert!(has(&fx, |e| *e == Effect::Notify(Outcome::PasteSent)));
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(
        fx.iter()
            .filter_map(|e| match e {
                Effect::Notify(o) => Some(*o),
                _ => None,
            })
            .next_back(),
        Some(Outcome::Failed(AppError::MicDisconnected))
    );
}

#[test]
fn continuous_review_opened_turns_the_next_phrase_into_a_draft() {
    let mut m = Machine::new(0);
    let cap = start_continuous(&mut m, 1);
    let fx = m.step(Input::ReviewOpened);
    assert!(has(&fx, |e| matches!(e, Effect::FinishCapture { .. })));
    m.step(Input::CaptureFinished {
        id: cap.0,
        generation: cap.1,
        segment: 0,
        outcome: CaptureOutcome::Speech,
    });
    let job = m.live_session().map(|(session, generation)| JobKey {
        session,
        generation,
        worker_epoch: 0,
    });
    m.step(Input::TranscriptionDone {
        job: job.unwrap(),
        text: "phrase".into(),
    });
    assert_eq!(m.phase(), Phase::DraftReady);
    assert_eq!(m.draft().unwrap().reason, DraftReason::ReviewOpened);
}

#[test]
fn continuous_needs_automatic_delivery_and_stop_is_busy_otherwise() {
    let mut m = Machine::new(0);
    let fx = m.step(Input::Start {
        id: sid(1),
        now_ms: 0,
        ready: Ok(()),
        auto_delivery: false,
        continuous: true,
    });
    assert!(has(&fx, |e| matches!(
        e,
        Effect::OpenCapture {
            continuous: false,
            ..
        }
    )));
    assert!(!m.continuous());

    let mut m = Machine::new(0);
    let job = to_transcribing(&mut m, 2);
    let fx = m.step(Input::Stop { id: job.session });
    assert_eq!(notified(&fx), Some(Outcome::Rejected(AppError::Busy)));
}
