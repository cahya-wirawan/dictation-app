#!/usr/bin/env bash
# Downloads the approved Whisper base multilingual model into .models/ and
# verifies size and SHA-256 against the pinned manifest values. Idempotent.
set -euo pipefail
cd "$(dirname "$0")/.."
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
SHA256="60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
BYTES=147951465
OUT=".models/ggml-base.bin"

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
size() { wc -c <"$1" | tr -d ' '; }

mkdir -p .models
if [[ -f "$OUT" && "$(size "$OUT")" == "$BYTES" && "$(sha "$OUT")" == "$SHA256" ]]; then
  echo "model already present and verified: $OUT"
  exit 0
fi
curl -fsSL --retry 3 -o "$OUT.partial" "$URL"
if [[ "$(size "$OUT.partial")" != "$BYTES" || "$(sha "$OUT.partial")" != "$SHA256" ]]; then
  rm -f "$OUT.partial"
  echo "model download failed verification" >&2
  exit 1
fi
mv "$OUT.partial" "$OUT"
echo "model downloaded and verified: $OUT"
