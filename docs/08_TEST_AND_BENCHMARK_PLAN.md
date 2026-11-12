# 08 — Verification and benchmarks

## 1. Test layers

Pure Rust unit tests cover state transitions, timing, framing and policy. Audio fixture tests cover conversion/resampling/endpointing. Worker integration tests run real local inference. UI tests cover forms, keyboard accessibility and event reconciliation. Native end-to-end tests verify actual receiving text in controlled target apps; permission/signing tests use clean machines and release artifacts.

Do not test only mocks of the exact implementation. A mocked successful SendInput cannot prove that the destination received text. A real dictation test combines capture or deterministic injected PCM, native focus, engine output, clipboard transaction and destination observation.

## 2. Functional matrix

| Test | Fixture/action | Expected result |
|---|---|---|
| Toggle repeat | Hold chord; repeat key-downs | One start only; release/press required for stop |
| Endpoint | 2 s speech + 900 ms pause + speech | One continuous phrase at default 1000 ms |
| Finish | Speech + ≥1000 ms silence | One finish, trailing pad retained |
| No speech | 8 s silence/low noise | No engine job, clipboard operation or paste |
| False onset | Isolated click/brief impulse | No accepted speech session |
| Noisy room | Fan/music/keyboard without speech | Measure false accepts; no automatic text in standard no-speech suite |
| Sample formats | Mono/stereo, i16/f32, 44.1/48 kHz | Valid mono 16 kHz output; duration within resampler tolerance |
| Boundary words | Speech onset near frame edge | No clipped first syllable from missing pre-roll |
| Short phrase | “Ja”, “Ya”, “Yes” | Recognized or explicit no-usable-text, never unrelated filler |
| Cap | 120 s continuous speech | Finalize once at cap; no resource overrun |
| Audio fault | Overflow/disconnect | Abort; no partial silent success |
| Stop race | Explicit stop and endpoint same tick | One inference job |
| Cancel race | Cancel before/after worker result | No dispatch if cancellation precedes commit |
| Late response | Old session/epoch result | Discard without UI/draft overwrite |
| Worker fault | Crash/hang/EOF/oversized length | Bounded error; cleanup/restart; no orphan |
| Target switch | Change app during inference | Draft; no automatic clipboard write or input |
| Field switch | Same app/window, new field | Draft if element changed; unknown target not auto-delivered |
| Secure target | Password field/lock/elevated app | Automatic delivery unavailable |
| Modifiers held | User holds trigger modifiers | Wait ≤1500 ms then draft; never release user's physical keys |
| Unicode | Umlauts, accents, emoji, CJK | Exact clipboard/destination roundtrip |
| Clipboard busy | Simulated lock/write failure | Bounded failure; no input send |
| Dispatch uncertainty | Partial/failed native key submission | Mark uncertain; no automatic retry |
| New copy during restore | Ownership/revision changed | Do not overwrite newer clipboard |
| Draft collision | New start with existing draft | Block start and explain recovery |
| Lock/sleep | Lock during each phase | Cancel/clear; no resume paste on unlock |
| Tampered model | Size/hash mismatch | Never load; retain previous valid model |
| Offline | Block network after install/import | Capture/transcription/delivery still work |
| HUD permissions | Invoke sensitive commands from HUD | Denied at IPC/backend boundary |
| Logging | Speak distinctive secret token | Token absent from logs and exports |

## 3. Speech dataset

Create a consented, license-cleared corpus with at least 180 utterances: 60 per language (English, German, Indonesian), with varied speakers/accents, short 1–3 s commands-as-text, normal 5–15 s sentences and 20–60 s paragraphs. Include names, numbers, mixed language, punctuation contexts and background conditions. Aim for at least 6 speakers per language; document representation limits. Keep a separate tuning set and held-out acceptance set so VAD/decoder thresholds are not adjusted on the release test set.

Add at least 60 no-speech recordings (silence, fan, typing, music, traffic). Add deterministic waveform fixtures for exact onset/endpoint boundaries. Real ASR text varies with model/backend; validate exact transport with fixed synthetic transcripts separately from ASR quality.

Ground truth must be checked by a fluent reviewer for each language. Define transcript normalization in the benchmark script: Unicode NFC, case-folding, explicit punctuation policy, number normalization policy fixed before evaluation. Report raw and normalized WER; CER complements WER for names and diacritics. Separate no-speech false acceptance from speech WER.

## 4. Proposed quality gates

- Held-out clean speech median normalized WER ≤15% and language-specific aggregate WER ≤20% for the selected release model; report speaker/language distributions. These are initial product targets, not literature claims or achieved results.
- No automated insertion on the standard no-speech acceptance suite. Report the sample size and uncertainty; zero in a finite suite does not prove zero real-world false accepts.
- On annotated endpoint fixtures, end-of-speech-to-close is configured silence plus ≤200 ms p95 processing overhead. Report premature stops on natural pauses separately.
- At least 99 successful controlled-target deliveries out of 100 per supported app/version under ordinary conditions; zero wrong-target or duplicate dispatches in adversarial tests. Include each failure rather than averaging across apps.
- If Base misses language accuracy goals, compare Small and choose based on accuracy/latency/memory tradeoff; do not silently relax goals or advertise unsupported quality.

## 5. Performance method

Reference hardware must be recorded before measurements: initial candidate MacBook Air M2 with 16 GB RAM, running an exact recorded supported macOS release; Windows candidate modern 4+ core x64 CPU with 16 GB RAM and exact Windows 11 build. These are test plans, not evidence that such machines have been benchmarked.

For each model/backend, report model SHA-256, engine revision, compiler flags, thread count, OS/CPU/GPU, power mode and input device. Run at least 30 warm trials per 10 s phrase profile plus 10 cold starts. Distinguish model cold-load time from warm session latency. Test 1, 10, 30 and 120 s recordings. Record median, p95 and maximum; do not quote a universal “real-time” speed.

Metrics:

- Trigger latency = first captured sample timestamp minus accepted shortcut timestamp.
- Endpoint delay = capture close time minus annotated last speech sample time.
- Inference latency = result received minus worker job accepted.
- End-to-end completion = input dispatch time minus annotated last speech sample time.
- Real-time factor = inference seconds / audio seconds; report model load separately.
- Total peak RSS = parent + worker, with UI closed/open; include model residency and DSP buffers.
- Idle CPU measured over 60 s after stabilization; record per-core normalization.

Targets from PRD apply to the chosen warm Base profile: trigger p95 ≤300 ms, 10 s stop-to-dispatch p95 ≤3 s, total RSS ≤1 GB and idle CPU mean <1% of one logical core. Failing a target triggers investigation, a documented profile restriction, or explicit product decision; it is not a reason to hide cold results.

## 6. Soak and release evidence

Run 500 mixed sessions including start/stop, short silence, cancellation, worker restart and device changes. Track host/worker memory for an unexplained monotonic trend; compare steady-state windows and investigate growth >10% after warm-up. Verify worker reaping and zero idle mic stream. Exercise a 24-hour tray residency test with no recording.

Release report includes exact build hashes, signed installer verification, supported-app matrix, permission denial/revocation evidence, offline run, benchmark tables, known issues and privacy checks. Human accessibility smoke tests use VoiceOver/Narrator and keyboard-only onboarding. A green web UI CI suite alone is insufficient for native desktop release.
