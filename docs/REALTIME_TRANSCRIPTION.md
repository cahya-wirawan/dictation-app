# Realtime transcription: research notes

Status: **parked** (14 November 2026; streaming engines compared the same day). This is not part of the spec. Read it when realtime (streaming) transcription is wanted, and write an ADR in `docs/09_ADRS_AND_RISKS.md` before implementing anything here.

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

## Dedicated streaming engines

These three models are built to stream, unlike Whisper. The facts below come from each project's own pages as of November 2026: the GitHub READMEs, the Hugging Face cards, the Moonshine docs and paper (arXiv 2602.12241). Recheck them before deciding.

| | Moonshine v2 Streaming | Zipformer (k2-fsa, via sherpa-onnx) | NVIDIA Nemotron Speech Streaming 0.6B |
|---|---|---|---|
| Model | Encoder-decoder; sliding-window encoder with about 80 ms lookahead; partial results replaced by final ones | Streaming transducer (RNN-T), with chunks such as `chunk-16-left-128` | Cache-aware FastConformer with an RNN-T decoder; chunk 0.08 / 0.16 / 0.56 / 1.12 s, chosen at inference time |
| Runtime | C++ library with a C API on ONNX Runtime (8-bit `.ort` files), CPU by default. Bindings: Python, Swift, Android, WASM | C/C++ on ONNX Runtime, with bindings for 12 languages including Rust (`rust-api-examples`). Runs on macOS, Windows and Linux | NeMo/PyTorch, Transformers ≥ 5.13, or NVIDIA's [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) with a q8_0 GGUF |
| Size | Tiny 34M, Small 123M, Medium 245M | English 20M to about 70M (int8 encoders of 41–180 MB on disk) | 600M |
| Languages (streaming) | English; German and Spanish (Small and Tiny); Japanese; Mandarin, Arabic, Vietnamese and Tagalog (Tiny only) | English, Chinese, Chinese+English, Korean, French, Bengali. No German or Indonesian | English only. *Nemotron 3.5 ASR Streaming 0.6B* (June 2026) covers 40 language-locales, not reviewed here |
| Accuracy (average WER on the Open ASR leaderboard sets) | Tiny 12.0 %, Small 7.84 %, Medium 6.65 % | Not published on the model pages | 6.93 % at 1.12 s chunks, 7.67 % at 0.16 s, 8.43 % at 0.08 s |
| Speed (stated) | Apple M3 CPU, end of speech to text: Tiny 50 ms, Small 148 ms, Medium 258 ms, using 8–29 % of real time. Whisper Small takes 1940 ms on the same setup | Real-time factor 0.03–0.15 in maintainer logs, on an unnamed CPU with 1 thread | No CPU figures. Tested on V100, A100, A6000 and DGX Spark GPUs |
| Punctuation and casing | Not stated in the docs; **must be checked** | No. English output is uppercase without punctuation; a separate punctuation model is needed | Yes, built in |
| Partial and final results | `on_line_updated` / `on_line_text_changed` / `on_line_completed`, updated every 0.5 s by default | `is_final` flag; built-in endpointing (trailing-silence rules) | Streaming chunks with cached state |
| Extras | Built-in VAD, key terms and context biasing, word timings | Hotwords, token timestamps | — |
| License | Code MIT. **All streaming models MIT.** Some older non-streaming non-English models use the restrictive *Moonshine AI Community License*: free only under US$1M revenue, with registration and a "Powered by Moonshine AI" notice. Avoid those | Code Apache-2.0. Most model licenses are unstated on the pages, and some were trained on non-commercial corpora (WenetSpeech) or an "internal dataset" | NVIDIA Open Model License; the card says it is "ready for commercial/non-commercial use" |

### Assessment for this app

- **Moonshine v2 is the first candidate.** It targets realtime on a laptop CPU, the streaming models are MIT, and the C API fits behind a worker process.
  - Costs: it needs a second worker binary linking ONNX Runtime, adding a dependency and a notices entry. ONNX Runtime must also be built universal (arm64 + x86_64), and the Moonshine docs do not mention a macOS x86_64 build.
  - Open questions: punctuation and casing output, and speed on the 2019 Intel i7.
- **Zipformer is the fallback.** The engine is mature, permissively licensed and has Rust bindings. But the models date from 2023, cover the wrong languages for us, have unclear licenses and produce unpunctuated uppercase English. It is weak as dictation output.
- **Nemotron is a later option for high-end Macs.** It is the most accurate, punctuates and needs no Python with NeMo-Speech.cpp. But at 600M parameters, aimed at GPUs and with no CPU figures, it is unlikely to stream on the Intel MacBook. It is English only unless the multilingual 3.5 model is used.

### Recommended architecture: two engines

- A small streaming model (Moonshine Small or Medium) drives **only the HUD live preview**.
- **Whisper still produces the text that is pasted** at the end of each phrase, as today.

This keeps Whisper's accuracy and languages, leaves delivery untouched, and confines the new engine's risk to the preview. It is cheaper than LocalAgreement on Whisper: one streaming pass instead of re-transcribing the buffer every second. The preview and the pasted text can differ slightly, so the HUD should make clear the preview is provisional.

## Product constraints that do not change

- **Pasting stays one checked paste per phrase.** Delivery is validate → commit → one paste chord, with no retraction (docs/05). Tentative words get revised, so they can never be pasted. At most, confirmed text could be pasted at the end of a phrase, which is what continuous dictation already does.
- **Live text goes to a preview only.** The natural place is the HUD. The HUD currently receives no text (the `Notifier` payloads "never contain transcript text"), and changing that needs the ADR to cover:
  - the HUD window's capability scope;
  - no persistence and no logging;
  - clearing on cancel or suspend.
- **Fencing.** Partial results need session, generation and worker-epoch fencing like final results (docs/04). A late partial must never update a newer session's preview.
- **Cancel.** It still means killing the worker. Streaming does not change "no re-running canceled audio".

## Suggested plan when this is picked up

0. **Engine spike:** run Moonshine v2 Small and Medium (C API) on the Intel MacBook and on Apple Silicon. Record the latency, the CPU load and whether the output has punctuation and casing. If it passes, use the two-engine architecture: the steps below then apply to a separate `preview-worker`, and Whisper's path stays as it is. If it fails, continue with LocalAgreement on Whisper.
1. **ADR:** live preview text in the HUD (scope, privacy, fencing), and the engine and policy chosen in step 0.
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
- Moonshine: <https://github.com/moonshine-ai/moonshine>, docs <https://moonshine-voice.readthedocs.io/>, paper <https://arxiv.org/abs/2602.12241>
- sherpa-onnx (Zipformer streaming models): <https://github.com/k2-fsa/sherpa-onnx>, models <https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-transducer/index.html>
- Nemotron Speech Streaming: <https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b>, NeMo-Speech.cpp <https://github.com/NVIDIA/NeMo-Speech.cpp>
