#!/usr/bin/env bash
# Builds dictation-worker for the host (or $TARGET) and stages it where Tauri's
# `bundle.externalBin` expects it: src-tauri/binaries/dictation-worker-<triple>.
# Release builds must pass DICTATION_GGML_NATIVE=OFF (portable CPU code).
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-release}"
TRIPLE="${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
args=(-p transcription-worker --bin dictation-worker --target "$TRIPLE")
[[ "$PROFILE" == release ]] && args+=(--release)
[[ -n "${WORKER_FEATURES:-}" ]] && args+=(--features "$WORKER_FEATURES")
cargo build "${args[@]}"
ext=""; [[ "$TRIPLE" == *windows* ]] && ext=".exe"
mkdir -p src-tauri/binaries
cp "target/$TRIPLE/$PROFILE/dictation-worker$ext" "src-tauri/binaries/dictation-worker-$TRIPLE$ext"
echo "staged src-tauri/binaries/dictation-worker-$TRIPLE$ext"
