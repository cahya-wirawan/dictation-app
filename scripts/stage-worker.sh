#!/usr/bin/env bash
# Builds dictation-worker for the host (or $TARGET, including
# universal-apple-darwin = arm64 + x86_64 via lipo) and stages it where Tauri's
# `bundle.externalBin` expects it: src-tauri/binaries/dictation-worker-<triple>.
# Release builds must pass DICTATION_GGML_NATIVE=OFF (portable CPU code).
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-release}"
TRIPLE="${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
build() {
  local args=(-p transcription-worker --bin dictation-worker --target "$1")
  [[ "$PROFILE" == release ]] && args+=(--release)
  [[ -n "${WORKER_FEATURES:-}" ]] && args+=(--features "$WORKER_FEATURES")
  cargo build "${args[@]}"
}
mkdir -p src-tauri/binaries
out="src-tauri/binaries/dictation-worker-$TRIPLE"
if [[ "$TRIPLE" == universal-apple-darwin ]]; then
  # `tauri build --target universal-apple-darwin` bundles one fat sidecar,
  # but its per-architecture app builds each check for their own thin one,
  # so all three are staged.
  # The x86_64 slice uses the x86-64-v3 baseline (every Intel Mac that runs
  # macOS 13 has AVX2/FMA); see ADR-013.
  slices=()
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null
    build "$t"
    slices+=("target/$t/$PROFILE/dictation-worker")
    cp "target/$t/$PROFILE/dictation-worker" "src-tauri/binaries/dictation-worker-$t"
  done
  lipo -create "${slices[@]}" -output "$out"
  lipo -info "$out"
else
  build "$TRIPLE"
  ext=""; [[ "$TRIPLE" == *windows* ]] && ext=".exe"
  out="$out$ext"
  cp "target/$TRIPLE/$PROFILE/dictation-worker$ext" "$out"
fi
echo "staged $out"
