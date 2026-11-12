//! Golden framing and bounds tests (work package B1).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Cursor;

use dictation_domain::SessionId;
use transcription_protocol::*;

fn job() -> JobId {
    JobId {
        session_id: SessionId::from_bytes([0x11; 16]),
        generation: 2,
        worker_epoch: 3,
    }
}

fn frame_bytes<T: serde::Serialize>(m: &T) -> Vec<u8> {
    let mut v = Vec::new();
    write_frame(&mut v, m).unwrap();
    v
}

#[test]
fn hello_golden_bytes() {
    let hello = FromWorker::Hello {
        protocol: 1,
        engine: "whisper.cpp".into(),
        engine_revision: "v1.9.5".into(),
        backends: vec!["cpu".into()],
    };
    let json = br#"{"type":"Hello","protocol":1,"engine":"whisper.cpp","engine_revision":"v1.9.5","backends":["cpu"]}"#;
    let mut expected = (json.len() as u32).to_le_bytes().to_vec();
    expected.extend_from_slice(json);
    assert_eq!(frame_bytes(&hello), expected);
}

#[test]
fn transcribe_golden_json() {
    let msg = ToWorker::Transcribe {
        job: job(),
        language: Language::auto(),
        sample_count: 2,
        pcm_bytes: 8,
    };
    let bytes = frame_bytes(&msg);
    let json = std::str::from_utf8(&bytes[4..]).unwrap();
    assert_eq!(
        json,
        r#"{"type":"Transcribe","job":{"session_id":"11111111111111111111111111111111","generation":2,"worker_epoch":3},"language":"auto","sample_count":2,"pcm_bytes":8}"#
    );
}

#[test]
fn roundtrip_frame_then_pcm() {
    let pcm = [0.0f32, -0.5, 0.25, 1.0];
    let mut buf = frame_bytes(&ToWorker::Transcribe {
        job: job(),
        language: Language::try_from("de".to_string()).unwrap(),
        sample_count: 4,
        pcm_bytes: 16,
    });
    write_pcm(&mut buf, &pcm).unwrap();
    let mut r = Cursor::new(buf);
    let msg: ToWorker = read_frame(&mut r).unwrap().unwrap();
    msg.validate().unwrap();
    let ToWorker::Transcribe { sample_count, .. } = msg else {
        panic!()
    };
    assert_eq!(read_pcm(&mut r, sample_count).unwrap(), pcm);
    // Clean EOF between frames.
    assert!(read_frame::<_, ToWorker>(&mut r).unwrap().is_none());
}

#[test]
fn oversized_length_rejected_before_allocation() {
    let mut r = Cursor::new(u32::MAX.to_le_bytes().to_vec());
    assert!(matches!(
        read_frame::<_, ToWorker>(&mut r),
        Err(FrameError::BadLength(u32::MAX))
    ));
    let mut r = Cursor::new((MAX_ENVELOPE_BYTES + 1).to_le_bytes().to_vec());
    assert!(matches!(
        read_frame::<_, ToWorker>(&mut r),
        Err(FrameError::BadLength(_))
    ));
}

#[test]
fn zero_length_rejected() {
    let mut r = Cursor::new(0u32.to_le_bytes().to_vec());
    assert!(matches!(
        read_frame::<_, ToWorker>(&mut r),
        Err(FrameError::BadLength(0))
    ));
}

#[test]
fn truncated_header_and_body_rejected() {
    let mut r = Cursor::new(vec![5u8, 0]);
    assert!(matches!(
        read_frame::<_, ToWorker>(&mut r),
        Err(FrameError::Truncated)
    ));
    let mut bytes = 10u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{\"ty");
    assert!(matches!(
        read_frame::<_, ToWorker>(&mut Cursor::new(bytes)),
        Err(FrameError::Truncated)
    ));
}

#[test]
fn unknown_fields_and_types_rejected() {
    for json in [
        r#"{"type":"Shutdown","x":1}"#,
        r#"{"type":"RunShell","cmd":"ls"}"#,
        r#"[1,2]"#,
    ] {
        let mut bytes = (json.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        assert!(
            matches!(
                read_frame::<_, ToWorker>(&mut Cursor::new(bytes)),
                Err(FrameError::Json(_))
            ),
            "{json}"
        );
    }
}

#[test]
fn pcm_non_finite_and_short_rejected() {
    let mut bytes = Vec::new();
    for s in [0.1f32, f32::NAN] {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    assert!(matches!(
        read_pcm(&mut Cursor::new(bytes.clone()), 2),
        Err(FrameError::NonFinitePcm)
    ));
    assert!(matches!(
        read_pcm(&mut Cursor::new(bytes[..6].to_vec()), 2),
        Err(FrameError::Truncated)
    ));
    assert!(read_pcm(&mut Cursor::new(Vec::new()), MAX_SAMPLES + 1).is_err());
}

#[test]
fn semantic_validation() {
    let bad_len = ToWorker::Transcribe {
        job: job(),
        language: Language::auto(),
        sample_count: 4,
        pcm_bytes: 15,
    };
    assert!(bad_len.validate().is_err());
    let too_many = ToWorker::Transcribe {
        job: job(),
        language: Language::auto(),
        sample_count: MAX_SAMPLES + 1,
        pcm_bytes: 0,
    };
    assert!(too_many.validate().is_err());
    let rel = ToWorker::LoadModel {
        path: "models/x.bin".into(),
        sha256: "a".repeat(64),
    };
    assert!(rel.validate().is_err());
    let bad_hash = ToWorker::LoadModel {
        path: "/m/x.bin".into(),
        sha256: "A".repeat(64),
    };
    assert!(bad_hash.validate().is_err());
    for l in ["", "EN", "english", "e1"] {
        assert!(Language::try_from(l.to_string()).is_err(), "{l}");
    }
}

#[test]
fn max_size_result_fits_envelope() {
    // Worst-case escaping: every char a control char → \u00XX (6 bytes each).
    // MAX_TEXT_BYTES of ASCII quotes escape to 2 bytes each, well inside 256 KiB;
    // control chars are stripped by the worker before serialization.
    let r = FromWorker::Result(TranscriptionResult {
        job: job(),
        text: "\"".repeat(MAX_TEXT_BYTES),
        language: Some("en".into()),
        audio_ms: 120_000,
        inference_ms: 1,
        segments: MAX_SEGMENTS,
    });
    assert!(frame_bytes(&r).len() <= MAX_ENVELOPE_BYTES as usize + 4);
}
