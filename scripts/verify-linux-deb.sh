#!/usr/bin/env bash
# Installs the .deb with apt (dependency resolution included), checks the
# installed layout, runs real inference through the *installed* worker, then
# removes the package (docs/10 §1: experiments only). Prints Markdown.
set -euo pipefail
deb="${1:?usage: verify-linux-deb.sh <package.deb>}"
fail() { echo "FAIL: $*"; exit 1; }
cd "$(dirname "$0")/.."

pkg=$(dpkg-deb -f "$deb" Package)
sudo apt-get install -y "./$deb" >/dev/null
worker=/usr/bin/dictation-worker
[[ -x "$worker" ]] || fail "worker not installed at $worker: $(dpkg -L "$pkg" | tr '\n' ' ')"
app=$(dpkg -L "$pkg" | grep -E '^/usr/bin/' | grep -v dictation-worker | head -1)
[[ -n "$app" && -x "$app" ]] || fail "main executable missing"
notices=$(dpkg -L "$pkg" | grep THIRD_PARTY_NOTICES.md | head -1)
[[ -n "$notices" ]] || fail "third-party notices missing"

cargo build --release -p dictation-bench >/dev/null 2>&1
out=$(./target/release/dictation-bench corpus --worker "$worker" --model .models/ggml-base.bin \
  --sha 60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe \
  --manifest crates/bench/corpus/jfk.tsv --trials 5 --cold 3)
echo "$out" | grep -q '| jfk.wav | en |' || fail "installed worker did not transcribe: $out"

echo "| Check | Result |"
echo "|---|---|"
echo "| Package | $(basename "$deb") ($(du -h "$deb" | cut -f1)) |"
echo "| Depends | $(dpkg-deb -f "$deb" Depends) |"
echo "| Executables | $app, $worker |"
echo
echo "Inference through the installed worker:"
echo
echo "$out" | sed -n '/^| file/,/^$/p'
echo "$out" | grep -E '^cold start|^worker peak'

sudo apt-get remove -y "$pkg" >/dev/null
[[ ! -e "$worker" ]] || fail "removal left the worker behind"
echo
echo "| Removal | files removed |"
