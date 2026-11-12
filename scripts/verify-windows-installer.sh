#!/usr/bin/env bash
# Installs the NSIS installer silently as the current (non-admin) user,
# checks the installed layout, runs real inference through the *installed*
# worker, then uninstalls and checks that the program files are gone
# (docs/10 §1, §3). Prints a Markdown summary. Windows (Git Bash) only.
set -euo pipefail
setup="${1:?usage: verify-windows-installer.sh <setup.exe>}"
fail() { echo "FAIL: $*"; exit 1; }
cd "$(dirname "$0")/.."

"$setup" /S
dir="$(cygpath -u "$LOCALAPPDATA")/Local Dictation"
for _ in $(seq 1 60); do [[ -f "$dir/uninstall.exe" ]] && break; sleep 1; done
[[ -d "$dir" ]] || fail "not installed under %LOCALAPPDATA%\\Local Dictation"
worker="$dir/dictation-worker.exe"
[[ -f "$worker" ]] || fail "worker missing: $(ls "$dir")"
app=$(find "$dir" -maxdepth 1 -name '*.exe' ! -name 'dictation-worker.exe' ! -name 'uninstall.exe' | head -1)
[[ -n "$app" ]] || fail "main executable missing"
[[ -f "$dir/THIRD_PARTY_NOTICES.md" ]] || fail "third-party notices missing"

cargo build --release -p dictation-bench >/dev/null 2>&1
bench_out=$(./target/release/dictation-bench corpus --worker "$(cygpath -w "$worker")" \
  --model .models/ggml-base.bin --sha 60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe \
  --manifest crates/bench/corpus/jfk.tsv --trials 5 --cold 3)
echo "$bench_out" | grep -q '| jfk.wav | en |' || fail "installed worker did not transcribe: $bench_out"

echo "| Check | Result |"
echo "|---|---|"
echo "| Installer | $(basename "$setup") ($(du -h "$setup" | cut -f1)) |"
echo "| Install scope | per-user, no elevation: $dir |"
echo "| Installed files | $(ls "$dir" | tr '\n' ' ') |"
echo
echo "Inference through the installed worker:"
echo
echo "$bench_out" | sed -n '/^| file/,/^$/p'
echo "$bench_out" | grep -E '^cold start|^worker peak'

"$dir/uninstall.exe" /S
for _ in $(seq 1 60); do [[ ! -f "$worker" ]] && break; sleep 1; done
[[ ! -f "$worker" ]] || fail "uninstall left the worker behind"
echo
echo "| Uninstall | program files removed |"
