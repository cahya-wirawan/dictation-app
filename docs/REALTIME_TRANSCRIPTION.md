# Realtime transcription: research notes

Status: **parked** (14 November 2026). This is not part of the spec. Read it when realtime (streaming) transcription is wanted, and write an ADR in `docs/09_ADRS_AND_RISKS.md` before implementing anything here.

"Realtime" here means text appears while you are still speaking. Continuous dictation (ADR-014) is not this: it pastes each phrase only after you pause.

## SimulStreaming as a reference

[SimulStreaming](https://github.com/ufal/SimulStreaming) comes from Charles University (ÚFAL). It is the successor to WhisperStreaming, and the README says it is the best-performing system in the IWSLT 2025 simultaneous speech translation shared task. The facts below come from its README and `LICENCE.txt`, read in November 2026:

| Aspect | SimulStreaming |
|---|---|
| Runtime | Python, with OpenAI's Whisper PyTorch code copied into the repo |
| Model | Whisper large-v3 (1.5B parameters) recommended; an optional EuroLLM-9B translation stage |
| Hardware | GPU with at least 10 GB VRAM recommended. The README says CPU "would be too slow for real-time" |
| Interface | CLI and a TCP server that reads raw 16 kHz S16_LE PCM; output is JSONL with `is_final` and word timings |
| Streaming policy | AlignAtt (see below); the translation stage uses LocalAgreement |
| License | MIT. The README asks commercial users to fill in a feedback form about "licensing models", which may change later |
| Latency figures | None in the README beyond "about 5 times faster than WhisperStreaming" |

### Why it cannot be used as is

- **Python runtime.** The spec forbids it, and PyTorch would add hundreds of MB to an app that is about 13 MB.
- **Localhost TCP server.** The spec forbids a localhost service. The worker talks over inherited pipes (docs/03).
- **GPU-sized model.** Target machines run Whisper Base or Small on the CPU, Accelerate or Metal. On the 2019 Intel MacBook it would not run in real time.
- **Tied to PyTorch Whisper internals.** It reads cross-attention weights and forces the decoder to continue from fixed text. It is not a black-box API.

The useful part is the **policy**, which can be rebuilt inside the existing whisper.cpp worker.

## Two streaming policies

### LocalAgreement (WhisperStreaming)

- About every second, re-transcribe the growing audio buffer.
- Words that two runs in a row agree on (the shared prefix) become **confirmed**; the rest is **tentative**.
- Once text is confirmed, trim the buffer at a sentence or segment boundary and pass the confirmed text as the prompt.

Assessment:

- No changes to whisper.cpp: it works with `whisper_full` and the current shim, and needs only protocol additions.
- Cost: repeated transcription of the same audio. It should be fine with Base or Small on Apple Silicon. It is borderline on the 2019 Intel i7 and must be measured (docs/08: latency, CPU, RSS).
- Words are confirmed about one to two seconds after they are spoken.

### AlignAtt (Simul-Whisper and SimulStreaming)

- Decode token by token. For each new token, take the alignment heads' cross-attention and find the audio frame it points at most strongly.
- Stop when that frame is within `frame_threshold` of the buffer's end (the "dangerous zone"), because that audio may still change.
- On the next audio chunk, force-decode the confirmed tokens again, then continue.
- An optional CIF model decides whether the last word is complete. None exists for large-v3. Without one, the last word is always held back.

Assessment:

- Faster than LocalAgreement, because each step decodes only new tokens instead of re-transcribing the whole buffer.
- Needs things whisper.cpp's public API does not expose:
  - A custom decoding loop with a forced prefix. This is possible with the low-level `whisper_encode` / `whisper_decode` / `whisper_get_logits` in `whisper.h`.
  - Cross-attention weights per decoded token. whisper.cpp computes them internally only for DTW token timestamps (`dtw_token_timestamps`, `dtw_aheads`), so reading them means patching the pinned `vendor/whisper.cpp`. The patch would have to be carried across upgrades.
- Higher effort and risk. Consider it only if LocalAgreement is too slow on target hardware.

## Product constraints that do not change

- **Pasting stays one checked paste per phrase.** Delivery is validate → commit → one paste chord, with no retraction (docs/05). Tentative words get revised, so they can never be pasted. At most, confirmed text could be pasted at the end of a phrase, which is what continuous dictation already does.
- **Live text goes to a preview only.** The natural place is the HUD. The HUD currently receives no text (the `Notifier` payloads "never contain transcript text"), and changing that needs the ADR to cover:
  - the HUD window's capability scope;
  - no persistence and no logging;
  - clearing on cancel or suspend.
- **Fencing.** Partial results need session, generation and worker-epoch fencing like final results (docs/04). A late partial must never update a newer session's preview.
- **Cancel.** It still means killing the worker. Streaming does not change "no re-running canceled audio".

## Suggested plan when this is picked up

1. **ADR:** live preview text in the HUD (scope, privacy, fencing), and LocalAgreement as the first policy.
2. **Protocol:** add an incremental request to the worker, such as `TranscribeChunk { job, pcm_offset }` with `Partial { confirmed, tentative }` replies. Bump `PROTOCOL_VERSION` and add golden tests.
3. **Worker:** implement LocalAgreement-2 over a rolling buffer, with a prompt from confirmed text and trimming at segment ends.
4. **Coordinator:** the DSP pipeline sends audio chunks while Listening. The reducer accepts `Partial` only for the live generation and notifies the HUD. The final text and the paste path stay as they are.
5. **Measure** on the Intel MacBook and on Apple Silicon, with Base and Small:
   - time to the first confirmed word;
   - confirmation delay;
   - CPU load while speaking;
   - battery impact.

   Turn the feature on only where those numbers are acceptable.
6. **Only if needed:** an AlignAtt spike with a patched whisper.cpp exposing the alignment heads' cross-attention.

## References

- SimulStreaming: <https://github.com/ufal/SimulStreaming>
- SimulStreaming at IWSLT 2025: <https://aclanthology.org/2025.iwslt-1.41/>
- WhisperStreaming (LocalAgreement): <https://github.com/ufal/whisper_streaming>
- Simul-Whisper (AlignAtt and CIF): <https://github.com/backspacetg/simul_whisper>
