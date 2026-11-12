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
        outcome: CaptureOutcome::Speech,
    });
    match fx
        .iter()
        .find(|e| matches!(e, Effect::StartTranscription { .. }))
    {
        Some(Effect::StartTranscription { job }) => *job,
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
        outcome: CaptureOutcome::Speech,
    });
    let d = m.step(Input::CaptureFinished {
        id,
        generation,
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
    });
    let (id, generation) = m.live_session().unwrap();
    m.step(Input::CaptureOpened { id, generation });
    m.step(Input::CaptureFinished {
        id,
        generation,
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
    for _run in 0..400 {
        let mut m = Machine::new(0);
        let mut epoch = 0u64;
        let mut now = 0u64;
        let mut dispatched: HashMap<(SessionId, u64), u32> = HashMap::new();
        let mut canceled_precommit: HashSet<(SessionId, u64)> = HashSet::new();
        let mut speech_finished: HashSet<(SessionId, u64)> = HashSet::new();
        for _ in 0..60 {
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
                    }),
                    (Phase::Starting, Some((id, generation))) => {
                        Some(Input::CaptureOpened { id, generation })
                    }
                    (Phase::Listening | Phase::Finalizing, Some((id, generation))) => {
                        Some(if next(3) == 0 {
                            Input::Stop { id }
                        } else {
                            Input::CaptureFinished {
                                id,
                                generation,
                                outcome: if next(5) == 0 {
                                    CaptureOutcome::NoSpeech
                                } else {
                                    CaptureOutcome::Speech
                                },
                            }
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
                match next(20) {
                    0 | 1 => Input::Start {
                        id: sid(next(4) as u8),
                        now_ms: now,
                        ready: if next(6) == 0 {
                            Err(AppError::NotReady)
                        } else {
                            Ok(())
                        },
                        auto_delivery: next(4) != 0,
                    },
                    2 => Input::CaptureOpened { id, generation },
                    3 => Input::CaptureFailed {
                        id,
                        generation,
                        error: AppError::MicDisconnected,
                    },
                    4 => Input::Stop { id },
                    5 => Input::CaptureFinished {
                        id,
                        generation,
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
                    _ => Input::Tick { now_ms: now },
                }
            };
            let phase_before = m.phase();
            let ledger_before = m.ledger();
            let live_before = m.live_session();
            let is_cancel = matches!(input, Input::Cancel | Input::Suspend);
            let speech = matches!(
                input,
                Input::CaptureFinished {
                    outcome: CaptureOutcome::Speech,
                    ..
                }
            );
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
                    Effect::StartTranscription { job } => {
                        assert!(speech, "inference without accepted speech");
                        speech_finished.insert((job.session, job.generation));
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
    eprintln!("fuzz: {total_dispatch} dispatches, {total_precommit_cancels} pre-commit cancels");
    assert!(
        total_dispatch > 500,
        "fuzz should exercise dispatch; got {total_dispatch}"
    );
    assert!(
        total_precommit_cancels > 20,
        "fuzz should exercise pre-commit cancel; got {total_precommit_cancels}"
    );
}
