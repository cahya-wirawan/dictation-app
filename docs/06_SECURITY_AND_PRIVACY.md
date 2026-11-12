# 06 — Security and privacy design

## 1. Trust boundaries

Trusted components: signed Rust host, signed bundled worker and pinned model manifest. Inputs remain untrusted: microphone samples, decoded text, frontend requests, native target metadata, model files on disk, installer/update artifacts and download responses. A local model is a large parser input, not trusted executable logic simply because inference is offline.

Tauri capabilities constrain frontend IPC; they do not sandbox mistakes in Rust [S4]. Configure custom app command permissions explicitly using the current Tauri AppManifest/permission mechanism, verify them in negative tests, and check window labels and request state in Rust. Registered custom commands must not be assumed restricted merely because plugin permissions look narrow [S4].

| Actor | Allowed access |
|---|---|
| Settings webview | Read status/config, validated setting changes, explicit model/permission operations |
| Review webview | Authorized draft ID/revision, bounded edit/copy/discard |
| HUD webview | Status and meter only; no draft, clipboard, device/model path or process APIs |
| Rust host | Native capture, policy, target validation, clipboard/input, worker lifecycle |
| Worker | Verified model read and session PCM over inherited pipes; no intended network use |

Process separation contains worker crashes and enables hard cancellation. It is not a security sandbox unless separately implemented. Do not claim the worker is OS-sandboxed simply because it is a sidecar.

## 2. Threats and controls

| Threat | Control | Remaining limitation |
|---|---|---|
| Speech reaches remote service | No cloud provider or inference endpoint; offline tests and egress inspection | OS services or other installed software are outside app control |
| Frontend compromise sends arbitrary input | No generic send-keys/exec APIs; narrow domain commands; permission and state checks | Rust validation mistakes remain possible |
| Transcript delivered to wrong app | Target token, editable-element recheck, no forced refocus, safe draft fallback | Final focus race and caret movement cannot be eliminated globally |
| Duplicate paste after retry | Per-session dispatch ledger and no automatic post-commit retries | User can explicitly paste twice |
| Clipboard exfiltration/history | Clear disclosure, no idle clipboard monitoring, copy-only choice | Clipboard managers/sync/other local apps can observe clipboard data |
| Corrupt/hostile model | Bundled approved hashes, size limits, atomic activation, worker isolation | Model parser vulnerabilities need upstream fixes |
| Worker pipe memory exhaustion | Frame/sample/text bounds before allocation, timeouts, process cleanup | Native engine allocations need actual RSS testing |
| Long/invalid capture | Fixed cap, bounded rings, finite checks, fail on overflow | Bad drivers can still fail the session |
| Shortcut captures ordinary typing | Register chosen chords only, no continuous key content logging | OS-level registration behavior varies |
| Supply-chain compromise | Lockfiles, pinned engine source, SBOM, signed app/worker, provenance | Signing keys/build systems need protection |

## 3. Data lifecycle

| Data | Location/lifetime | Persistence |
|---|---|---|
| Raw native audio | Bounded host ring while recording | Never intentionally written to disk |
| Normalized utterance | Host memory until sent/failed/canceled | Released after finalization/job handling |
| Worker PCM/tensors | Worker memory during job | Cleared/released on completion; worker killed on cancellation |
| Recognized text | Coordinator and authorized review UI | One memory-only draft, max 10 min |
| Clipboard snapshot | Preservation mode only | Memory only, bounded; deleted after restore decision |
| Clipboard text | OS-managed after paste/Copy | May persist outside app and in OS history |
| Settings/model inventory | OS app config/data directories | Atomic user-owned files |
| Model binaries | Private app model directory | Persist until user removes |
| Diagnostics | Local redacted rolling logs | Default 7 days or 5 MiB, whichever reached first |

On quit/lock/cancel clear sensitive references and overwrite reusable audio buffers where practical. Rust deallocation, native allocators, OS swap and crash dumps prevent a blanket secure-erasure guarantee. Avoid automatic crash dumps containing process memory; never upload dumps automatically. Text copied to the OS clipboard is not recalled on draft expiry.

## 4. Frontend hardening

Bundle all UI assets and fonts. No remote scripts/CDNs, remote navigation or arbitrary embedded websites. Strict CSP with the minimum Tauri IPC/asset origins required by the actual release build; validate exact origins for each OS instead of pasting an untested policy. No `unsafe-eval` in release. Avoid `dangerouslySetInnerHTML`; render transcripts as text. Treat URLs and native paths as untrusted.

Explicitly list enabled capability identifiers. Do not give HUD `core:default`, shell, filesystem or clipboard-wide grants merely for convenience. Rust performs clipboard/model operations; no frontend shell plugin needed. Deny remote webview access and unnecessary dynamic window creation. Validate the generated capability schema in CI.

## 5. Networking policy

After initial model setup/import, capture, inference and delivery require no network. v1 network operations are user-initiated model download and opening the documented update page. No analytics, transcript crash reporting or silent update polling. Downloader enforces HTTPS, approved hosts/redirects and byte limits, and does not include transcript/device/target identifiers in requests.

A later automatic updater requires signed update manifests/artifacts, explicit release policy and separate review. Model checksums bundled with the signed app establish integrity against that manifest; checksums fetched from the same untrusted server as a file are insufficient by themselves.

## 6. Diagnostics

Log codes and timings only: phase transitions, duration buckets, engine revision, model ID, backend, OS/build and permission state. Never log text, PCM, clipboard contents, arbitrary paths, window titles, target field contents or microphone device serials. Process/app identities can be sensitive: redact or omit them from standard exports. Export shows a preview and requires explicit user action.

## 7. Required security evidence

Before release demonstrate: no capture in Idle; no transcript network egress; unauthorized HUD commands rejected; stale/canceled result cannot paste; transcript treated as inert text; modified model rejected; worker path cannot be replaced by a frontend argument; no orphan worker after host death; no secret content in logs/diagnostics; secure-field/excluded-target handling; clipboard failure does not trigger repeated input sends.
