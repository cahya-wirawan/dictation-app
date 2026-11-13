# Release procedure

This is how a release is cut and what must be true before it is published. It applies docs/10 §1–§4 to this repository. Measured evidence goes in `docs/IMPLEMENTATION_STATUS.md`.

## 1. Cut the version

1. Merge the milestone work to `main` with CI green.
2. On a release branch, run `pnpm version:bump <minor|patch>`. This moves `## [Unreleased]` in CHANGELOG.md to the new version.
3. Open a PR, merge it, then tag the merge commit `vX.Y.Z` and push the tag.

## 2. Build

Pushing the tag starts `.github/workflows/release.yml`:

1. **Consistency checks.** The version agrees across Cargo, npm, Tauri and the changelog, and the tag matches it. `pnpm notices:check` passes: notices are current and every shipped component has an allowed license.
2. **Worker.** The release worker is built with `DICTATION_GGML_NATIVE=OFF`, which means portable CPU code plus Accelerate.
3. **App and DMG.** Universal (arm64 + x86_64, ADR-013): `TARGET=universal-apple-darwin ./scripts/stage-worker.sh`, then `tauri build --target universal-apple-darwin --bundles app,dmg`, with the hardened runtime and `src-tauri/Entitlements.plist`, which grants audio input only.
   - **With signing secrets:** the app is Developer ID signed, then notarized and stapled.
   - **Without them:** it is ad-hoc signed, and the job warns that the result is not for distribution.
4. **Bundle check.** `scripts/verify-macos-bundle.sh` checks:
   - the bundle layout and the nested worker;
   - `codesign --verify --deep --strict`;
   - the hardened runtime on both executables;
   - the exact entitlements.
   With signing it also requires a Developer ID authority, Gatekeeper acceptance and a stapled ticket.
5. **Release files.** It produces a CycloneDX SBOM (`sbom.cdx.json`), the notices, and `SHA256SUMS`.
6. **Draft release.** A **draft** GitHub release is created with notes taken from the changelog section. Publishing it is a manual step after §3.

Secrets go in the `release` environment:
- `APPLE_CERTIFICATE` (base64 .p12) and `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and `APPLE_TEAM_ID`

Pull requests never run this workflow and never receive these secrets.

## 3. Pre-release checklist (docs/10 §3)

Tick each item against the specific artifact in the draft release, not a local build.

- [ ] CI green on the tagged commit: unit, fixture, protocol, frontend, clippy and bundle jobs.
- [ ] `verify-macos-bundle.sh --distribution` passed in the release job.
- [ ] **Clean machine** (a fresh macOS user, never granted before). Install from the DMG and go through onboarding. Grant the microphone, then deny Accessibility and recover. Revoke the microphone while the app is running and confirm a clear error.
- [ ] Model import with networking off. Then a tampered file is refused, and a partial or cancelled download recovers.
- [ ] Supported-app matrix (docs/05 §8) filled in for this build: TextEdit, Notes, a browser textarea, Mail compose and VS Code. Each row needs plain text, Unicode, a field switch, a window switch and a secure field.
- [ ] Benchmarks recorded with `dictation-bench` for the released worker and model. Run `corpus` on the release corpus (docs/08 §3), `nospeech` on recorded no-speech material, and `soak --sessions 500`. Compare the results with the docs/08 §4 gates. Report a failure as a failure.
- [ ] `lsof -i -P -p <pid>` while idle and while dictating shows no listener and no traffic apart from a model download you started.
- [ ] A support report saved after dictating a distinctive token does not contain the token.
- [ ] Upgrade from the previous release keeps settings, models and permissions. If not, onboarding explains why a permission must be granted again.
- [ ] Release notes name the limitations, models, engine version (whisper.cpp v1.9.5) and minimum OS (macOS 13).

## 4. Rollback

Rollback means reinstalling the previous signed DMG from its GitHub release. Settings and models stay compatible within a config `schema_version`. A newer schema opened by an older app falls back to defaults without overwriting the file. The app never downloads engine binaries.

## 5. Open release decisions

- **Bundle identifier.** It is still `app.localdictation.m0`. Changing it before the first public release moves the data directory and resets the macOS permission grants. Changing it later does the same to every user. Decide before 1.0.
- **Signing credentials.** No Developer ID certificate is configured yet, so every build so far is ad-hoc signed.
