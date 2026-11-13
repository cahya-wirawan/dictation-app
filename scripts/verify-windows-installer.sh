#!/usr/bin/env bash
# Installs the NSIS installer silently as the current (non-admin) user,
# checks the installed layout, runs real inference through the *installed*
# worker, then uninstalls and checks that the program files are gone
# (docs/10 §1, §3). Prints a Markdown summary. Windows (Git Bash) only.
set -euo pipefail
# Git Bash rewrites POSIX-looking arguments for native programs, so "/S"
# would reach NSIS as a drive path and the installer would wait in its GUI.
export MSYS_NO_PATHCONV=1
setup="${1:?usage: verify-windows-installer.sh <setup.exe>}"
fail() { echo "FAIL: $*"; exit 1; }
# Progress goes to stderr (the job log); stdout is the Markdown summary.
log() { echo "[$(date -u +%H:%M:%S)] $*" >&2; }
# Every phase is bounded; on a hang, show what is still running.
# Windows with a title (e.g. a dialog) are listed before the process is killed.
bounded() {
  local secs=$1; shift
  "$@" &
  local pid=$! t=0
  while kill -0 "$pid" 2>/dev/null; do
    if ((t >= secs)); then
      log "TIMEOUT after ${secs}s: $*"
      tasklist /v /fo csv 2>/dev/null | grep -v '"N/A"$' >&2 || true
      kill "$pid" 2>/dev/null || true
      fail "timed out: $*"
    fi
    sleep 1
    t=$((t + 1))
  done
  wait "$pid"
}
cd "$(dirname "$0")/.."

wv2=none
for key in 'HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' \
           'HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'; do
  v=$(reg query "$key" /v pv 2>/dev/null | awk '/pv/ {print $3}') || true
  [[ -n "${v:-}" ]] && wv2="$v" && break
done
log "WebView2 runtime before install: $wv2"

log "installing $setup (silent)"
bounded 600 "$setup" /S
log "installer exited"
dir="$(cygpath -u "$LOCALAPPDATA")/Local Dictation"
for _ in $(seq 1 60); do [[ -f "$dir/uninstall.exe" ]] && break; sleep 1; done
[[ -d "$dir" ]] || fail "not installed under %LOCALAPPDATA%\\Local Dictation"
worker="$dir/dictation-worker.exe"
[[ -f "$worker" ]] || fail "worker missing: $(ls "$dir")"
app=$(find "$dir" -maxdepth 1 -name '*.exe' ! -name 'dictation-worker.exe' ! -name 'uninstall.exe' | head -1)
[[ -n "$app" ]] || fail "main executable missing"
[[ -f "$dir/THIRD_PARTY_NOTICES.md" ]] || fail "third-party notices missing"

log "building dictation-bench"
bounded 1200 cargo build --release -p dictation-bench >&2
log "transcribing through the installed worker"
bench_out=$(bounded 600 ./target/release/dictation-bench corpus --worker "$(cygpath -w "$worker")" \
  --model .models/ggml-base.bin --sha 60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe \
  --manifest crates/bench/corpus/jfk.tsv --trials 5 --cold 3)
echo "$bench_out" | grep -q '| jfk.wav | en |' || fail "installed worker did not transcribe: $bench_out"

echo "| Check | Result |"
echo "|---|---|"
echo "| Installer | $(basename "$setup") ($(du -h "$setup" | cut -f1)) |"
echo "| WebView2 runtime before install | $wv2 |"
echo "| Install scope | per-user, no elevation: $dir |"
echo "| Installed files | $(ls "$dir" | tr '\n' ' ') |"
echo
echo "Inference through the installed worker:"
echo
echo "$bench_out" | sed -n '/^| file/,/^$/p'
echo "$bench_out" | grep -E '^cold start|^worker peak'

log "uninstalling"
bounded 300 "$dir/uninstall.exe" /S
for _ in $(seq 1 60); do [[ ! -f "$worker" ]] && break; sleep 1; done
[[ ! -f "$worker" ]] || fail "uninstall left the worker behind"
echo
echo "| Uninstall | program files removed |"
