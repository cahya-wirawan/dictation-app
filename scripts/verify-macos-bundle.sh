#!/usr/bin/env bash
# Verifies a built Local Dictation.app (docs/10 §3): layout, nested worker,
# signature validity, hardened runtime, entitlements. With --distribution it
# also requires a Developer ID signature and passes Gatekeeper assessment
# (notarized + stapled). Prints a Markdown summary.
set -euo pipefail
app="${1:?usage: verify-macos-bundle.sh <path.app> [--distribution]}"
dist="${2:-}"
worker="$app/Contents/MacOS/dictation-worker"
main="$app/Contents/MacOS/dictation-app"
fail() { echo "FAIL: $*"; exit 1; }

test -x "$main" || fail "missing main executable"
test -x "$worker" || fail "missing nested worker"
test -f "$app/Contents/Resources/THIRD_PARTY_NOTICES.md" || fail "missing third-party notices"
/usr/libexec/PlistBuddy -c 'Print :NSMicrophoneUsageDescription' "$app/Contents/Info.plist" >/dev/null || fail "no NSMicrophoneUsageDescription"

codesign --verify --deep --strict --verbose=2 "$app" 2>&1 | sed 's/^/    /'
codesign --verify --strict "$worker" || fail "worker signature invalid"
flags=$(codesign -dv "$main" 2>&1 | sed -n 's/.*flags=\(0x[0-9a-f]*\)(\(.*\)).*/\2/p')
wflags=$(codesign -dv "$worker" 2>&1 | sed -n 's/.*flags=\(0x[0-9a-f]*\)(\(.*\)).*/\2/p')
[[ "$flags" == *runtime* ]] || fail "app is not signed with the hardened runtime ($flags)"
[[ "$wflags" == *runtime* ]] || fail "worker is not signed with the hardened runtime ($wflags)"
ents=$(codesign -d --entitlements - --xml "$main" 2>/dev/null || true)
[[ "$ents" == *com.apple.security.device.audio-input* ]] || fail "missing audio-input entitlement"
for bad in allow-jit allow-unsigned-executable-memory disable-library-validation; do
  [[ "$ents" != *"$bad"* ]] || fail "unexpected entitlement $bad"
done
# Expected slices: EXPECT_ARCHS (default "arm64"; universal builds pass
# "arm64 x86_64"). App and worker must both contain every one.
expect="${EXPECT_ARCHS:-arm64}"
for bin in "$main" "$worker"; do
  have=$(lipo -archs "$bin")
  for a in $expect; do
    [[ " $have " == *" $a "* ]] || fail "$(basename "$bin") lacks $a (has: $have)"
  done
done
archs="$(lipo -archs "$main") / $(lipo -archs "$worker")"
authority=$(codesign -dv "$main" 2>&1 | sed -n 's/^Authority=//p' | head -1)
sig=$(codesign -dv "$main" 2>&1 | sed -n 's/^Signature=//p')

echo "| Check | Result |"
echo "|---|---|"
echo "| Version | $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist") |"
echo "| Bundle id | $(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist") |"
echo "| Architectures (app / worker) | $archs |"
echo "| Signature | ${authority:-${sig:-unknown}} |"
echo "| Flags (app / worker) | $flags / $wflags |"
echo "| Entitlements | audio-input only |"

if [[ "$dist" == --distribution ]]; then
  [[ "$authority" == "Developer ID Application:"* ]] || fail "not Developer ID signed"
  spctl --assess --type execute --verbose=2 "$app" 2>&1 | sed 's/^/    /' || fail "Gatekeeper rejected the app"
  xcrun stapler validate "$app" || fail "notarization ticket not stapled"
  echo "| Gatekeeper | accepted, ticket stapled |"
else
  echo "| Gatekeeper | not assessed (ad-hoc build, not for distribution) |"
fi
